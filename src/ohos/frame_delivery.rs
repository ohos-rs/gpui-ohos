use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Instant,
};

use gpui::ForegroundExecutor;
use ohos_vsync_binding::Vsync;
use openharmony_ability::{FrameInputDelivery, IntervalInfo, OpenHarmonyApp};

use super::{
    dispatcher::MainWake,
    frame_clock::FrameClock,
    frame_demand::{DeliveryChange, FrameDemand},
    frame_request::FrameRequest,
};

#[cfg(feature = "frame-metrics")]
type Metrics = Arc<std::sync::Mutex<super::frame_metrics::FrameMetrics>>;

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

struct FrameScheduler {
    vsync: Vsync,
    clock: Option<FrameClock>,
    state: Arc<FrameRequest>,
    waker: Arc<MainWake>,
    #[cfg(feature = "frame-metrics")]
    metrics: Metrics,
}

impl FrameScheduler {
    fn new(
        window_id: i64,
        waker: Arc<MainWake>,
        #[cfg(feature = "frame-metrics")] metrics: Metrics,
    ) -> Option<Self> {
        Some(Self {
            vsync: Vsync::try_new(format!("gpui-ohos-{window_id}"))?,
            clock: FrameClock::new(),
            state: Arc::new(FrameRequest::default()),
            waker,
            #[cfg(feature = "frame-metrics")]
            metrics,
        })
    }

    fn request_frame(&self) {
        #[cfg(feature = "frame-metrics")]
        if !self.state.failed() {
            let mut metrics = self.metrics.lock().unwrap();
            metrics.set_mode("vsync");
            metrics.demand();
        }
        let Some(ticket) = self.state.request() else {
            return;
        };
        let state = self.state.clone();
        let waker = self.waker.clone();
        let clock = self.clock;
        #[cfg(feature = "frame-metrics")]
        let (metrics, requested_at) = {
            self.metrics.lock().unwrap().vsync_requested(ticket);
            (self.metrics.clone(), Instant::now())
        };
        let result = self.vsync.request_frame_once(move |timestamp_ns| {
            #[cfg(feature = "frame-metrics")]
            let callback_at = Instant::now();
            let frame_time = clock
                .and_then(|clock| clock.instant_for_timestamp(timestamp_ns))
                .unwrap_or_else(Instant::now);
            #[cfg(feature = "frame-metrics")]
            let mut metrics = metrics.lock().unwrap();
            let accepted = state.complete(ticket, frame_time);
            #[cfg(feature = "frame-metrics")]
            metrics.vsync_callback(ticket, requested_at, callback_at, accepted);
            if accepted {
                waker.notify();
            }
        });
        if result != 0 {
            self.state.fail(ticket);
            #[cfg(feature = "frame-metrics")]
            self.metrics.lock().unwrap().vsync_failed();
            self.waker.notify();
            log::warn!("Failed to request OHOS VSync frame: {result}");
        }
    }

    fn set_active(&self, active: bool) {
        self.state.set_active(active);
        if active {
            self.request_frame();
        }
    }
}

pub(crate) struct WindowFrames {
    app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    window_id: i64,
    scheduler: Option<FrameScheduler>,
    demand: RefCell<FrameDemand>,
    active: Cell<bool>,
    surface_available: Cell<bool>,
    xcomponent_active: Cell<bool>,
    foreground: ForegroundExecutor,
    sync_queued: Cell<bool>,
    callback_active: Rc<Cell<bool>>,
    register_warned: Cell<bool>,
    unregister_warned: Cell<bool>,
    #[cfg(feature = "frame-metrics")]
    pub(crate) metrics: Metrics,
}

impl WindowFrames {
    pub(crate) fn new(
        window_id: i64,
        app: Rc<RefCell<Option<OpenHarmonyApp>>>,
        foreground: ForegroundExecutor,
        waker: Arc<MainWake>,
        callback_active: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        #[cfg(feature = "frame-metrics")]
        let metrics = Arc::new(std::sync::Mutex::new(
            super::frame_metrics::FrameMetrics::new(window_id, "vsync"),
        ));
        let scheduler = if cfg!(feature = "xcomponent-frames") {
            None
        } else {
            let scheduler = FrameScheduler::new(
                window_id,
                waker,
                #[cfg(feature = "frame-metrics")]
                metrics.clone(),
            );
            if scheduler.is_none() {
                log::warn!("OHOS VSync is unavailable for window {window_id}");
            }
            scheduler
        };
        Rc::new(Self {
            app,
            window_id,
            scheduler,
            demand: RefCell::new(FrameDemand::default()),
            active: Cell::new(false),
            surface_available: Cell::new(false),
            xcomponent_active: Cell::new(false),
            foreground,
            sync_queued: Cell::new(false),
            callback_active,
            register_warned: Cell::new(false),
            unregister_warned: Cell::new(false),
            #[cfg(feature = "frame-metrics")]
            metrics,
        })
    }

    pub(crate) fn uses_xcomponent(&self) -> bool {
        self.scheduler
            .as_ref()
            .is_none_or(|scheduler| scheduler.state.failed())
    }

    pub(crate) fn request(self: &Rc<Self>) {
        if !self.active.get() {
            return;
        }
        if let Some(scheduler) = &self.scheduler {
            scheduler.request_frame();
            if !scheduler.state.failed() {
                return;
            }
        }
        self.update_mode();
        #[cfg(feature = "frame-metrics")]
        self.metrics.lock().unwrap().demand();
        self.demand.borrow_mut().request();
        self.sync_frame_delivery();
    }

    pub(crate) fn set_active(self: &Rc<Self>, visible: bool, surface_available: bool) {
        #[cfg(feature = "frame-metrics")]
        let extra_callbacks = self.demand.borrow().idle_callbacks;
        let active = visible && surface_available;
        self.active.set(active);
        if self.surface_available.replace(surface_available) && !surface_available {
            self.demand.borrow_mut().surface_destroyed();
            self.xcomponent_active.set(false);
        }
        if let Some(scheduler) = &self.scheduler {
            scheduler.set_active(active);
        }
        self.sync_frame_delivery();
        #[cfg(feature = "frame-metrics")]
        if !active {
            self.metrics.lock().unwrap().deactivate(extra_callbacks);
        }
    }

    fn update_mode(&self) {
        let xcomponent = self.uses_xcomponent();
        #[cfg(feature = "frame-metrics")]
        if self.active.get() {
            self.metrics
                .lock()
                .unwrap()
                .set_mode(if xcomponent { "xcomponent" } else { "vsync" });
        }
        let active = self.active.get() && xcomponent;
        if self.xcomponent_active.replace(active) != active {
            self.demand.borrow_mut().set_active(active);
            #[cfg(feature = "frame-metrics")]
            if active {
                self.metrics.lock().unwrap().demand();
            }
        }
    }

    pub(crate) fn sync_frame_delivery(self: &Rc<Self>) {
        self.update_mode();
        let Some(change) = self.demand.borrow().pending() else {
            return;
        };
        if self.callback_active.get() {
            self.defer_sync();
            return;
        }
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        let register = change == DeliveryChange::Register;
        let delivery = if register {
            FrameInputDelivery::Continuous
        } else {
            FrameInputDelivery::OnDemand
        };
        #[cfg(feature = "frame-metrics")]
        let started = Instant::now();
        let result = app.set_frame_input_delivery_for(self.window_id, delivery);
        #[cfg(feature = "frame-metrics")]
        {
            let mut metrics = self.metrics.lock().unwrap();
            if self.surface_available.get() {
                metrics.delivery_call(register, started, result.is_ok());
            }
            if self.uses_xcomponent() && !register && result.is_ok() {
                metrics.enter_idle(self.demand.borrow().idle_callbacks);
            }
            if register && result.is_err() {
                metrics.enter_idle(0);
            }
        }
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
        }
    }

    fn defer_sync(self: &Rc<Self>) {
        if self.sync_queued.replace(true) {
            return;
        }
        let frames = self.clone();
        // Redraw events never drain the foreground queue. Recheck demand in a later UI turn.
        self.foreground
            .spawn(async move {
                frames.sync_queued.set(false);
                frames.sync_frame_delivery();
            })
            .detach();
    }

    pub(crate) fn on_callback(self: &Rc<Self>, _interval: &IntervalInfo) {
        self.update_mode();
        #[cfg(feature = "frame-metrics")]
        if self.uses_xcomponent() {
            self.metrics
                .lock()
                .unwrap()
                .xcomponent_callback(_interval.time_stamp, _interval.target_time_stamp);
        }
        if self.demand.borrow_mut().on_callback().is_some() {
            self.defer_sync();
        }
    }

    pub(crate) fn take_pending(self: &Rc<Self>) -> Option<Instant> {
        self.sync_frame_delivery();
        if !self.active.get() || self.uses_xcomponent() {
            return None;
        }
        self.scheduler.as_ref()?.state.take_pending()
    }
}
