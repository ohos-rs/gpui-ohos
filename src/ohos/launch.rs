pub(crate) struct LaunchState {
    surface_ready: bool,
    callback: Option<Box<dyn FnOnce()>>,
}

impl LaunchState {
    pub(crate) fn new(callback: Box<dyn FnOnce()>) -> Self {
        Self {
            surface_ready: false,
            callback: Some(callback),
        }
    }

    pub(crate) fn set_surface_ready(&mut self, ready: bool) {
        self.surface_ready = ready;
    }

    pub(crate) fn cancel(&mut self) {
        self.callback.take();
    }

    pub(crate) fn take_callback(&mut self, gpu_ready: bool) -> Option<Box<dyn FnOnce()>> {
        if self.surface_ready && gpu_ready {
            self.callback.take()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn starts_once_regardless_of_which_dependency_finishes_first() {
        for gpu_ready_first in [false, true] {
            let calls = Rc::new(Cell::new(0));
            let mut launch = LaunchState::new(Box::new({
                let calls = calls.clone();
                move || calls.set(calls.get() + 1)
            }));
            assert!(launch.take_callback(gpu_ready_first).is_none());
            launch.set_surface_ready(true);
            if !gpu_ready_first {
                assert!(launch.take_callback(false).is_none());
            }
            if let Some(callback) = launch.take_callback(true) {
                callback();
            }
            assert_eq!(calls.get(), 1);
            launch.set_surface_ready(false);
            launch.set_surface_ready(true);
            assert!(launch.take_callback(true).is_none());
        }
    }

    #[test]
    fn gpu_completion_waits_for_a_replacement_surface() {
        let mut launch = LaunchState::new(Box::new(|| {}));
        launch.set_surface_ready(true);
        assert!(launch.take_callback(false).is_none());
        launch.set_surface_ready(false);
        assert!(launch.take_callback(true).is_none());
        launch.set_surface_ready(true);
        assert!(launch.take_callback(true).is_some());
    }

    #[test]
    fn destruction_releases_callback_and_ignores_late_completion() {
        let retained = Rc::new(());
        let weak = Rc::downgrade(&retained);
        let mut launch = LaunchState::new(Box::new(move || drop(retained)));
        launch.set_surface_ready(true);
        launch.cancel();
        assert!(weak.upgrade().is_none());
        assert!(launch.take_callback(true).is_none());
        launch.set_surface_ready(false);
        launch.set_surface_ready(true);
        assert!(launch.take_callback(true).is_none());
    }
}
