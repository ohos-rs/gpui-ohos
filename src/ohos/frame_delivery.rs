use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use gpui::{BackgroundExecutor, ForegroundExecutor, Task};
use openharmony_ability::{FrameInputDelivery, OpenHarmonyApp};

use super::{
    frame_demand::{DeliveryChange, FrameDemand},
    window::OhosWindow,
};
#[cfg(feature = "native-vsync")]
use super::{native_frames::NativeFrames, native_refresh::NativeRefresh};

const FALLBACK_FRAME_DELAY: Duration = Duration::from_millis(16);

pub(crate) struct FrameCallbackScope {
    active: Rc<Cell<bool>>,
    previous: bool,
}

impl FrameCallbackScope {
    pub(crate) fn enter(active: Rc<Cell<bool>>) -> Self {
        let previous = active.replace(true);
        Self { active, previous }
    }
}

impl Drop for FrameCallbackScope {
    fn drop(&mut self) {
        self.active.set(self.previous);
    }
}

pub(crate) struct WindowFrames {
    app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    window_id: i64,
    window: RefCell<Weak<RefCell<OhosWindow>>>,
    demand: RefCell<FrameDemand>,
    surface_available: Cell<bool>,
    foreground: ForegroundExecutor,
    background: BackgroundExecutor,
    sync_queued: Cell<bool>,
    callback_active: Rc<Cell<bool>>,
    fallback_task: RefCell<Option<Task<()>>>,
    register_warned: Cell<bool>,
    unregister_warned: Cell<bool>,
    #[cfg(feature = "native-vsync")]
    active: Cell<bool>,
    #[cfg(feature = "native-vsync")]
    native_refresh: Cell<Option<NativeRefresh>>,
    #[cfg(feature = "native-vsync")]
    native: RefCell<Option<NativeFrames>>,
    #[cfg(feature = "native-vsync")]
    source_changed: Cell<bool>,
    #[cfg(feature = "native-vsync")]
    native_failed: Cell<bool>,
}

impl WindowFrames {
    pub(crate) fn new(
        window_id: i64,
        app: Rc<RefCell<Option<OpenHarmonyApp>>>,
        foreground: ForegroundExecutor,
        background: BackgroundExecutor,
        callback_active: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let frames = Rc::new(Self {
            app,
            window_id,
            window: RefCell::new(Weak::new()),
            demand: RefCell::new(FrameDemand::default()),
            surface_available: Cell::new(false),
            foreground,
            background,
            sync_queued: Cell::new(false),
            callback_active,
            fallback_task: RefCell::new(None),
            register_warned: Cell::new(false),
            unregister_warned: Cell::new(false),
            #[cfg(feature = "native-vsync")]
            active: Cell::new(false),
            #[cfg(feature = "native-vsync")]
            native_refresh: Cell::new(None),
            #[cfg(feature = "native-vsync")]
            native: RefCell::new(None),
            #[cfg(feature = "native-vsync")]
            source_changed: Cell::new(false),
            #[cfg(feature = "native-vsync")]
            native_failed: Cell::new(false),
        });
        #[cfg(feature = "native-vsync")]
        super::native_frames::register(&frames);
        frames
    }

    pub(crate) fn set_window(&self, window: Weak<RefCell<OhosWindow>>) {
        *self.window.borrow_mut() = window;
    }

    pub(crate) fn request(self: &Rc<Self>) {
        self.demand.borrow_mut().request();
        self.sync_frame_delivery();
    }

    pub(crate) fn set_active(self: &Rc<Self>, visible: bool, surface_available: bool) {
        #[cfg(feature = "native-vsync")]
        self.active.set(visible && surface_available);
        if self.surface_available.replace(surface_available) && !surface_available {
            self.demand.borrow_mut().surface_destroyed();
        }
        self.demand
            .borrow_mut()
            .set_active(visible && surface_available);
        if !visible || !surface_available {
            self.fallback_task.borrow_mut().take();
        }
        self.sync_frame_delivery();
    }

    fn sync_frame_delivery(self: &Rc<Self>) {
        let pending = self.demand.borrow().pending().is_some();
        #[cfg(feature = "native-vsync")]
        let pending = pending || self.source_changed.get();
        if !pending {
            return;
        }
        if self.callback_active.get() {
            self.defer_sync();
            return;
        }
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        #[cfg(feature = "native-vsync")]
        if self.source_changed.replace(false) {
            // Switch sources outside every native callback, with at most one
            // source registered. A fresh demand retries the latest scene.
            if let Err(error) =
                app.set_frame_input_delivery_for(self.window_id, FrameInputDelivery::OnDemand)
            {
                log::warn!(
                    "Cannot switch frame source for window {}: {error}",
                    self.window_id
                );
                self.source_changed.set(true);
                return;
            }
            self.native.borrow_mut().take();
            self.demand.borrow_mut().surface_destroyed();
            self.demand.borrow_mut().set_active(self.active.get());
        }
        let Some(change) = self.demand.borrow().pending() else {
            return;
        };
        let register = change == DeliveryChange::Register;
        let delivery = if register {
            FrameInputDelivery::Continuous
        } else {
            FrameInputDelivery::OnDemand
        };
        let result = if !register {
            #[cfg(feature = "native-vsync")]
            self.native.borrow_mut().take();
            app.set_frame_input_delivery_for(self.window_id, delivery)
        } else {
            self.register_delivery(&app, delivery)
        };
        self.demand.borrow_mut().complete(change, result.is_ok());
        if let Err(error) = result {
            let warned = if register {
                &self.register_warned
            } else {
                &self.unregister_warned
            };
            if !warned.replace(true) {
                let operation = if register { "register" } else { "unregister" };
                log::warn!(
                    "Cannot {operation} OHOS frame callback for window {}: {error}",
                    self.window_id
                );
            }
            if register {
                self.schedule_fallback();
            }
        } else if register {
            self.fallback_task.borrow_mut().take();
        }
    }

    fn register_delivery(
        &self,
        app: &OpenHarmonyApp,
        delivery: FrameInputDelivery,
    ) -> napi_ohos::Result<()> {
        #[cfg(feature = "native-vsync")]
        if let Some(target) = self
            .native_refresh
            .get()
            .filter(|_| !self.native_failed.get())
        {
            let result = app
                .main_thread()
                .and_then(|main| NativeFrames::new(target, self.window_id, main));
            match result {
                Ok(native) => {
                    *self.native.borrow_mut() = Some(native);
                    log::info!(
                        "native-frame-source window={} display_window={} target_hz={}",
                        self.window_id,
                        target.window_id,
                        target.hz
                    );
                    return Ok(());
                }
                Err(error) => {
                    self.native_failed.set(true);
                    log::warn!(
                        "NativeVSync unavailable for window {}; using XComponent: {error}",
                        self.window_id
                    );
                }
            }
        }
        app.set_frame_input_delivery_for(self.window_id, delivery)
    }

    fn schedule_fallback(self: &Rc<Self>) {
        if !self.demand.borrow_mut().schedule_fallback() {
            return;
        }
        let frames = Rc::downgrade(self);
        let background = self.background.clone();
        let task = self.foreground.spawn(async move {
            loop {
                background.timer(FALLBACK_FRAME_DELAY).await;
                let Some(frames) = frames.upgrade() else {
                    return;
                };
                if frames.callback_active.get() {
                    continue;
                }
                // A draw may request another frame and replace this task.
                if let Some(task) = frames.fallback_task.borrow_mut().take() {
                    task.detach();
                }
                let draw = frames.demand.borrow_mut().begin_fallback();
                if draw && let Some(window) = frames.window.borrow().upgrade() {
                    window.borrow().draw_requested_frame(Instant::now());
                }
                return;
            }
        });
        *self.fallback_task.borrow_mut() = Some(task);
    }

    fn defer_sync(self: &Rc<Self>) {
        if self.sync_queued.replace(true) {
            return;
        }
        let frames = self.clone();
        // Keep cleanup alive through window drop; redraw events never drain this queue.
        self.foreground
            .spawn(async move {
                frames.sync_queued.set(false);
                frames.sync_frame_delivery();
            })
            .detach();
    }

    fn on_callback(self: &Rc<Self>) {
        self.fallback_task.borrow_mut().take();
        if self.demand.borrow_mut().on_callback().is_some() {
            self.defer_sync();
        }
    }

    pub(crate) fn on_xcomponent_callback(self: &Rc<Self>) -> bool {
        if self.native_delivering() {
            return false;
        }
        self.on_callback();
        true
    }

    #[cfg(feature = "native-vsync")]
    pub(crate) fn window_id(&self) -> i64 {
        self.window_id
    }

    pub(crate) fn native_delivering(&self) -> bool {
        #[cfg(feature = "native-vsync")]
        {
            self.native.borrow().is_some()
        }
        #[cfg(not(feature = "native-vsync"))]
        {
            false
        }
    }

    #[cfg(feature = "native-vsync")]
    pub(crate) fn set_native_refresh(self: &Rc<Self>, target: Option<NativeRefresh>) {
        if self.native_refresh.replace(target) != target {
            self.native_failed.set(false);
            self.source_changed.set(true);
            self.defer_sync();
        }
    }

    #[cfg(feature = "native-vsync")]
    pub(crate) fn on_native_callback(self: &Rc<Self>, token: usize) {
        if !self
            .native
            .borrow()
            .as_ref()
            .is_some_and(|native| native.token() == token)
        {
            return;
        }
        {
            let _scope = FrameCallbackScope::enter(self.callback_active.clone());
            self.on_callback();
            // Arm the next VSync before drawing. Draw/present may use most of
            // the refresh period; requesting afterwards can miss the deadline
            // even when rendering itself fits within that period.
            // Even the idle-threshold draw can create fresh demand. Arm now;
            // deferred idle cleanup drops the receiver if it stays idle.
            if self.active.get() {
                let result = self.native.borrow_mut().as_mut().map(NativeFrames::request);
                if let Some(Err(error)) = result {
                    log::warn!("NativeVSync stopped for window {}: {error}", self.window_id);
                    self.native_failed.set(true);
                    self.source_changed.set(true);
                    self.defer_sync();
                }
            }
            if let Some(window) = self.window.borrow().upgrade() {
                window.borrow().draw_requested_frame(Instant::now());
            }
        }
    }
}
