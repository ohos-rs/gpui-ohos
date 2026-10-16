use std::cell::Cell;

/// Result of a platform presentation attempt. A transient acquisition failure
/// retains demand until the next frame; an unavailable surface waits for input
/// or lifecycle events instead of spinning on a hidden or invalid surface.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presentation {
    Presented,
    Retry,
    Unavailable,
}

pub(crate) struct PresentationDemand {
    pending: Cell<bool>,
}

impl Default for PresentationDemand {
    fn default() -> Self {
        Self {
            pending: Cell::new(true),
        }
    }
}

impl PresentationDemand {
    pub(crate) fn request(&self) {
        self.pending.set(true);
    }

    pub(crate) fn needs_presentation(&self) -> bool {
        self.pending.get()
    }

    /// Returns whether the next frame should be requested.
    pub(crate) fn record(&self, presentation: Presentation) -> bool {
        let retry = presentation == Presentation::Retry;
        self.pending.set(retry);
        retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_frame_needs_presentation() {
        let demand = PresentationDemand::default();
        assert!(demand.needs_presentation());
    }

    #[test]
    fn presented_clears_demand() {
        let demand = PresentationDemand::default();
        assert!(!demand.record(Presentation::Presented));
        assert!(!demand.needs_presentation());
    }

    #[test]
    fn retry_keeps_demand_and_requests_next_frame() {
        let demand = PresentationDemand::default();
        for _ in 0..3 {
            assert!(demand.record(Presentation::Retry));
            assert!(demand.needs_presentation());
        }
    }

    #[test]
    fn unavailable_does_not_keep_idle_frames_presenting() {
        let demand = PresentationDemand::default();
        assert!(!demand.record(Presentation::Unavailable));
        for _ in 0..120 {
            assert!(!demand.needs_presentation());
        }
    }

    #[test]
    fn lifecycle_event_restores_demand_after_unavailable() {
        let demand = PresentationDemand::default();
        assert!(!demand.record(Presentation::Unavailable));
        demand.request();
        demand.request();
        assert!(demand.needs_presentation());
        assert!(demand.needs_presentation());
        assert!(!demand.record(Presentation::Presented));
        assert!(!demand.needs_presentation());
    }

    #[test]
    fn retry_restores_demand_after_a_new_draw() {
        let demand = PresentationDemand::default();
        for outcome in [Presentation::Presented, Presentation::Unavailable] {
            assert!(!demand.record(outcome));
            assert!(!demand.needs_presentation());
            assert!(demand.record(Presentation::Retry));
            assert!(demand.needs_presentation());
        }
    }

    #[test]
    fn terminal_outcomes_stop_retries() {
        let demand = PresentationDemand::default();
        for outcome in [Presentation::Presented, Presentation::Unavailable] {
            assert!(demand.record(Presentation::Retry));
            assert!(!demand.record(outcome));
            assert!(!demand.needs_presentation());
        }
    }
}
