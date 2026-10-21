//! Native registration and receiver shims for the production frame delivery owner.
use std::{
    cell::{Cell, RefCell},
    fmt,
    rc::Rc,
};

#[derive(Debug)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("simulated native failure")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameInputDelivery {
    Continuous,
    OnDemand,
}

#[derive(Default)]
pub(crate) struct FrameState {
    #[cfg(feature = "audio")]
    pub(crate) audio: crate::audio_support::AudioState,
    pub(crate) delivery_window: Cell<i64>,
    pub(crate) xcomponent: Cell<bool>,
    pub(crate) callback_active: Rc<Cell<bool>>,
    pub(crate) fail_delivery: Cell<bool>,
    #[cfg(feature = "native-vsync")]
    pub(crate) native_count: Cell<usize>,
    #[cfg(feature = "native-vsync")]
    pub(crate) token: Cell<usize>,
    #[cfg(feature = "native-vsync")]
    pub(crate) requests: Cell<usize>,
    #[cfg(feature = "native-vsync")]
    pub(crate) creations: Cell<usize>,
    #[cfg(feature = "native-vsync")]
    pub(crate) fail_create: Cell<bool>,
    #[cfg(feature = "native-vsync")]
    pub(crate) fail_request: Cell<bool>,
}

#[derive(Clone, Default)]
pub struct OpenHarmonyApp(pub(crate) Rc<FrameState>);

impl OpenHarmonyApp {
    pub fn set_frame_input_delivery_for(
        &self,
        window_id: i64,
        delivery: FrameInputDelivery,
    ) -> crate::Result<()> {
        assert!(
            !self.0.callback_active.get(),
            "registration inside callback"
        );
        if self.0.fail_delivery.replace(false) {
            return Err(Error);
        }
        self.0.delivery_window.set(window_id);
        #[cfg(feature = "native-vsync")]
        assert_eq!(
            delivery,
            FrameInputDelivery::OnDemand,
            "XComponent enabled in NativeVSync build"
        );
        self.0
            .xcomponent
            .set(delivery == FrameInputDelivery::Continuous);
        Ok(())
    }

    #[cfg(feature = "native-vsync")]
    pub(crate) fn main_thread(&self) -> crate::Result<Rc<FrameState>> {
        Ok(self.0.clone())
    }

    #[cfg(feature = "native-vsync")]
    pub(crate) fn refresh_rate(&self) -> u32 {
        120
    }
}

pub(crate) mod window {
    use super::*;
    use std::time::Instant;

    #[derive(Default)]
    pub(crate) struct OhosWindow {
        pub(crate) draws: Cell<usize>,
        pub(crate) on_draw: RefCell<Option<Box<dyn FnMut()>>>,
    }

    impl OhosWindow {
        #[cfg(feature = "native-vsync")]
        pub(crate) fn draw_native_frame(&self, time: Instant) {
            self.draw_requested_frame(time);
        }

        pub(crate) fn draw_requested_frame(&self, _time: Instant) {
            self.draws.set(self.draws.get() + 1);
            if let Some(callback) = self.on_draw.borrow_mut().as_mut() {
                callback();
            }
        }
    }
}

#[cfg(feature = "native-vsync")]
pub(crate) mod native_frames {
    use super::*;
    use crate::frame_delivery::WindowFrames;

    pub(crate) fn register(_frames: &Rc<WindowFrames>) {}

    pub(crate) struct NativeFrames {
        state: Rc<FrameState>,
        token: usize,
    }

    impl NativeFrames {
        pub(crate) fn new(_window_id: i64, _hz: u32, state: Rc<FrameState>) -> crate::Result<Self> {
            assert!(!state.callback_active.get());
            assert!(!state.xcomponent.get(), "two frame sources");
            state.creations.set(state.creations.get() + 1);
            if state.fail_create.get() {
                return Err(Error);
            }
            let token = state.token.get() + 1;
            state.token.set(token);
            state.native_count.set(state.native_count.get() + 1);
            let mut native = Self { state, token };
            native.request()?;
            Ok(native)
        }

        pub(crate) fn token(&self) -> usize {
            self.token
        }

        pub(crate) fn request(&mut self) -> crate::Result<()> {
            if self.state.fail_request.replace(false) {
                return Err(Error);
            }
            self.state.requests.set(self.state.requests.get() + 1);
            Ok(())
        }
    }

    impl Drop for NativeFrames {
        fn drop(&mut self) {
            assert!(
                !self.state.callback_active.get(),
                "receiver destroyed inside callback"
            );
            self.state
                .native_count
                .set(self.state.native_count.get() - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PriorityQueueReceiver,
        dispatcher::OhosDispatcher,
        frame_delivery::{FrameCallbackScope, WindowFrames},
        frame_demand::IDLE_CALLBACK_THRESHOLD,
    };
    use gpui::ForegroundExecutor;
    use std::{sync::Arc, time::Duration};

    struct Driver {
        app: OpenHarmonyApp,
        frames: Rc<WindowFrames>,
        window: Rc<RefCell<window::OhosWindow>>,
        queue: PriorityQueueReceiver<crate::RunnableVariant>,
        dispatcher: Arc<OhosDispatcher>,
    }

    impl Driver {
        fn new() -> Self {
            Self::with_app(OpenHarmonyApp::default())
        }

        fn with_app(app: OpenHarmonyApp) -> Self {
            let (sender, queue) = PriorityQueueReceiver::new();
            let dispatcher = Arc::new(OhosDispatcher::new(sender));
            let frames = WindowFrames::new(
                0,
                Rc::new(RefCell::new(Some(app.clone()))),
                ForegroundExecutor::new(dispatcher.clone()),
                app.0.callback_active.clone(),
            );
            let window = Rc::new(RefCell::new(window::OhosWindow::default()));
            frames.set_window(Rc::downgrade(&window));
            frames.set_active(true, true);
            Self {
                app,
                frames,
                window,
                queue,
                dispatcher,
            }
        }

        fn flush(&self) {
            for _ in 0..100 {
                let Some(task) = self.queue.0.pop_timeout(Duration::ZERO) else {
                    return;
                };
                self.dispatcher.begin_main_turn();
                task.run();
            }
            panic!("frame delivery sync did not settle");
        }

        #[cfg(feature = "native-vsync")]
        fn tick(&self) {
            self.frames.on_native_callback(self.app.0.token.get());
        }

        #[cfg(not(feature = "native-vsync"))]
        fn tick(&self) {
            let _scope = FrameCallbackScope::enter(self.app.0.callback_active.clone());
            self.frames.on_xcomponent_callback();
            self.window
                .borrow()
                .draw_requested_frame(std::time::Instant::now());
        }
    }

    impl Drop for Driver {
        fn drop(&mut self) {
            self.frames.set_active(false, false);
            self.flush();
            self.dispatcher.shutdown();
        }
    }

    #[cfg(not(feature = "native-vsync"))]
    #[test]
    fn xcomponent_build_unregisters_after_idle_callbacks() {
        let driver = Driver::new();
        assert!(driver.app.0.xcomponent.get());
        for _ in 0..=IDLE_CALLBACK_THRESHOLD {
            driver.tick();
        }
        assert!(driver.app.0.xcomponent.get());
        driver.flush();
        assert!(!driver.app.0.xcomponent.get());
        driver.frames.request();
        assert!(driver.app.0.xcomponent.get());
    }

    #[cfg(not(feature = "native-vsync"))]
    #[test]
    fn failed_xcomponent_registration_waits_for_new_demand() {
        let app = OpenHarmonyApp::default();
        app.0.fail_delivery.set(true);
        let driver = Driver::with_app(app);
        driver.flush();
        assert!(!driver.app.0.xcomponent.get());
        assert_eq!(driver.window.borrow().draws.get(), 0);
        driver.frames.request();
        assert!(driver.app.0.xcomponent.get());
    }

    #[test]
    fn hide_inside_callback_defers_delivery_cleanup() {
        let driver = Driver::new();
        {
            let _scope = FrameCallbackScope::enter(driver.app.0.callback_active.clone());
            driver.frames.set_active(false, true);
            #[cfg(not(feature = "native-vsync"))]
            assert!(driver.app.0.xcomponent.get());
            #[cfg(feature = "native-vsync")]
            assert_eq!(driver.app.0.native_count.get(), 1);
        }
        driver.flush();
        assert!(!driver.app.0.xcomponent.get());
        #[cfg(feature = "native-vsync")]
        assert_eq!(driver.app.0.native_count.get(), 0);
    }

    #[test]
    fn retained_frame_waker_follows_rebound_surface() {
        let driver = Driver::new();
        let retained = driver.frames.clone();
        #[cfg(feature = "native-vsync")]
        let old_token = driver.app.0.token.get();
        {
            let _scope = FrameCallbackScope::enter(driver.app.0.callback_active.clone());
            driver.frames.rebind_window(42);
            driver.frames.set_active(true, true);
            assert_eq!(driver.app.0.delivery_window.get(), 0);
        }
        driver.flush();
        assert_eq!(driver.app.0.delivery_window.get(), 42);
        #[cfg(feature = "native-vsync")]
        {
            assert_eq!(driver.app.0.native_count.get(), 1);
            assert_ne!(driver.app.0.token.get(), old_token);
            driver.frames.on_native_callback(old_token);
            assert_eq!(driver.window.borrow().draws.get(), 0);
        }
        retained.request();
        driver.tick();
        assert_eq!(driver.window.borrow().draws.get(), 1);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn native_build_starts_without_runtime_configuration() {
        let driver = Driver::new();
        assert!(!driver.app.0.xcomponent.get());
        assert_eq!(driver.app.0.native_count.get(), 1);
        driver.tick();
        assert_eq!(driver.window.borrow().draws.get(), 1);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn failed_native_creation_does_not_start_xcomponent_or_retry_without_demand() {
        let app = OpenHarmonyApp::default();
        app.0.fail_create.set(true);
        let driver = Driver::with_app(app);
        driver.flush();
        assert!(!driver.app.0.xcomponent.get());
        assert_eq!(driver.app.0.native_count.get(), 0);
        assert_eq!(driver.app.0.creations.get(), 1);
        assert_eq!(driver.window.borrow().draws.get(), 0);
        driver.app.0.fail_create.set(false);
        driver.frames.request();
        assert_eq!(driver.app.0.native_count.get(), 1);
        assert_eq!(driver.app.0.creations.get(), 2);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn failed_xcomponent_deactivation_does_not_start_native_receiver() {
        let app = OpenHarmonyApp::default();
        app.0.fail_delivery.set(true);
        let driver = Driver::with_app(app);
        driver.flush();
        assert_eq!(driver.app.0.native_count.get(), 0);
        assert_eq!(driver.app.0.creations.get(), 0);
        driver.frames.request();
        assert_eq!(driver.app.0.native_count.get(), 1);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn failed_native_rearm_releases_receiver_without_switching_sources() {
        let driver = Driver::new();
        driver.app.0.fail_request.set(true);
        driver.tick();
        assert_eq!(driver.app.0.native_count.get(), 1);
        driver.flush();
        assert!(!driver.app.0.xcomponent.get());
        assert_eq!(driver.app.0.native_count.get(), 0);
        assert_eq!(driver.app.0.creations.get(), 1);
        driver.frames.request();
        assert_eq!(driver.app.0.native_count.get(), 1);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn demand_during_failed_native_frame_restarts_after_callback_cleanup() {
        let driver = Driver::new();
        let old_token = driver.app.0.token.get();
        let frames = Rc::downgrade(&driver.frames);
        *driver.window.borrow().on_draw.borrow_mut() = Some(Box::new(move || {
            frames.upgrade().unwrap().request();
        }));
        driver.app.0.fail_request.set(true);
        driver.tick();
        assert_eq!(driver.app.0.native_count.get(), 1);
        assert_eq!(driver.app.0.creations.get(), 1);
        driver.window.borrow().on_draw.borrow_mut().take();
        driver.flush();
        assert_eq!(driver.app.0.native_count.get(), 1);
        assert_eq!(driver.app.0.creations.get(), 2);
        assert!(!driver.app.0.xcomponent.get());
        driver.frames.on_native_callback(old_token);
        assert_eq!(driver.window.borrow().draws.get(), 1);
        driver.tick();
        assert_eq!(driver.window.borrow().draws.get(), 2);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn idle_threshold_draw_can_request_another_native_frame() {
        let driver = Driver::new();
        let frames = Rc::downgrade(&driver.frames);
        let state = driver.app.0.clone();
        let draws = Cell::new(0);
        *driver.window.borrow().on_draw.borrow_mut() = Some(Box::new(move || {
            draws.set(draws.get() + 1);
            assert_eq!(state.requests.get(), draws.get() + 1);
            if draws.get() == IDLE_CALLBACK_THRESHOLD + 1 {
                frames.upgrade().unwrap().request();
            }
        }));
        for _ in 0..=IDLE_CALLBACK_THRESHOLD {
            driver.tick();
        }
        driver.flush();
        assert_eq!(driver.app.0.native_count.get(), 1);
        driver.tick();
        driver.window.borrow().on_draw.borrow_mut().take();
        for _ in 0..IDLE_CALLBACK_THRESHOLD {
            driver.tick();
        }
        driver.flush();
        assert_eq!(driver.app.0.native_count.get(), 0);
        driver.frames.request();
        assert_eq!(driver.app.0.native_count.get(), 1);
    }

    #[cfg(feature = "native-vsync")]
    #[test]
    fn hiding_and_surface_recreation_reject_queued_native_callbacks() {
        let driver = Driver::new();
        let old_token = driver.app.0.token.get();
        driver.frames.set_active(false, false);
        assert_eq!(driver.app.0.native_count.get(), 0);
        driver.frames.on_native_callback(old_token);
        assert_eq!(driver.window.borrow().draws.get(), 0);
        driver.frames.set_active(true, true);
        assert_eq!(driver.app.0.native_count.get(), 1);
        driver.frames.on_native_callback(old_token);
        assert_eq!(driver.window.borrow().draws.get(), 0);
        driver.tick();
        assert_eq!(driver.window.borrow().draws.get(), 1);
    }
}
