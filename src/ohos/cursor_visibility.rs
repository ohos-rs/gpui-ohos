pub(super) struct CursorVisibility {
    visible: bool,
    applied: Option<bool>,
    pending: bool,
    supported: bool,
}

impl Default for CursorVisibility {
    fn default() -> Self {
        Self {
            visible: true,
            applied: Some(true),
            pending: false,
            supported: true,
        }
    }
}

impl CursorVisibility {
    pub(super) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn request(&mut self, visible: bool) -> Option<bool> {
        if !self.supported {
            return None;
        }
        self.visible = visible;
        if self.pending || self.applied == Some(visible) {
            return None;
        }
        self.pending = true;
        Some(visible)
    }

    pub(super) fn complete(&mut self, visible: bool, succeeded: bool) -> Option<bool> {
        self.pending = false;
        self.applied = succeeded.then_some(visible);
        // A failed native request must remain retryable on the next input, but
        // must not create an unbounded retry loop while the service is failing.
        if !succeeded && self.visible == visible {
            return None;
        }
        self.request(self.visible)
    }

    pub(super) fn mark_unsupported(&mut self) {
        self.supported = false;
        self.pending = false;
        self.visible = true;
        self.applied = None;
    }
}

#[cfg(test)]
mod tests {
    use super::CursorVisibility;

    #[test]
    fn repeated_typing_sends_one_hide_request() {
        let mut state = CursorVisibility::default();
        assert!(state.is_visible());
        assert_eq!(state.request(false), Some(false));
        for _ in 0..20 {
            assert_eq!(state.request(false), None);
        }
        assert!(!state.is_visible());
        assert_eq!(state.complete(false, true), None);
        assert_eq!(state.request(false), None);
    }

    #[test]
    fn movement_during_hide_restores_after_completion() {
        let mut state = CursorVisibility::default();
        assert_eq!(state.request(false), Some(false));
        assert_eq!(state.request(true), None);
        assert!(state.is_visible());
        assert_eq!(state.complete(false, true), Some(true));
        assert_eq!(state.complete(true, true), None);
        assert_eq!(state.request(true), None);
    }

    #[test]
    fn typing_after_movement_supersedes_a_queued_restore() {
        let mut state = CursorVisibility::default();
        assert_eq!(state.request(false), Some(false));
        assert_eq!(state.request(true), None);
        assert_eq!(state.request(false), None);
        assert_eq!(state.complete(false, true), None);
        assert!(!state.is_visible());
    }

    #[test]
    fn typing_during_restore_hides_after_completion() {
        let mut state = CursorVisibility::default();
        assert_eq!(state.request(false), Some(false));
        assert_eq!(state.complete(false, true), None);
        assert_eq!(state.request(true), Some(true));
        assert_eq!(state.request(false), None);
        assert_eq!(state.complete(true, true), Some(false));
        assert_eq!(state.complete(false, true), None);
        assert!(!state.is_visible());
    }

    #[test]
    fn failed_requests_retry_only_on_later_input() {
        let mut state = CursorVisibility::default();
        assert_eq!(state.request(false), Some(false));
        assert_eq!(state.complete(false, false), None);
        assert_eq!(state.request(false), Some(false));
        assert_eq!(state.complete(false, true), None);
        assert_eq!(state.request(true), Some(true));
        assert_eq!(state.complete(true, false), None);
        assert_eq!(state.request(true), Some(true));
        assert_eq!(state.complete(true, true), None);
    }

    #[test]
    fn failed_old_request_still_applies_new_visibility() {
        let mut state = CursorVisibility::default();
        assert_eq!(state.request(false), Some(false));
        assert_eq!(state.request(true), None);
        assert_eq!(state.complete(false, false), Some(true));
        assert_eq!(state.complete(true, true), None);
        assert_eq!(state.request(true), None);
    }

    #[test]
    fn unsupported_devices_keep_the_system_cursor_without_repeated_requests() {
        let mut state = CursorVisibility::default();
        assert_eq!(state.request(false), Some(false));
        state.mark_unsupported();
        for visible in [false, false, true, false, true] {
            assert_eq!(state.request(visible), None);
            assert!(state.is_visible());
        }
    }
}
