pub(crate) const IDLE_CALLBACK_THRESHOLD: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryChange {
    Register,
    Unregister,
}

pub(crate) struct FrameDemand {
    active: bool,
    registered: bool,
    demanded: bool,
    idle_callbacks: usize,
    pending: Option<DeliveryChange>,
    unregister_failed: bool,
}

impl Default for FrameDemand {
    fn default() -> Self {
        Self {
            active: false,
            registered: false,
            demanded: false,
            idle_callbacks: 0,
            pending: Some(DeliveryChange::Unregister),
            unregister_failed: false,
        }
    }
}

impl FrameDemand {
    pub(crate) fn set_active(&mut self, active: bool) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            self.request();
        } else {
            self.demanded = false;
            self.pending = (!self.unregister_failed).then_some(DeliveryChange::Unregister);
        }
    }

    pub(crate) fn surface_destroyed(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn request(&mut self) -> Option<DeliveryChange> {
        if !self.active {
            return None;
        }
        self.demanded = true;
        self.idle_callbacks = 0;
        self.pending = (!self.registered).then_some(DeliveryChange::Register);
        self.pending
    }

    pub(crate) fn on_callback(&mut self) -> Option<DeliveryChange> {
        // A callback is authoritative even when ability restored a retained registration.
        self.registered = true;
        if self.pending == Some(DeliveryChange::Register) {
            self.pending = None;
        }
        if std::mem::take(&mut self.demanded) {
            self.idle_callbacks = 0;
        } else {
            self.idle_callbacks = self.idle_callbacks.saturating_add(1);
        }
        if (!self.active || self.idle_callbacks >= IDLE_CALLBACK_THRESHOLD)
            && !self.unregister_failed
        {
            self.pending = Some(DeliveryChange::Unregister);
        }
        self.pending
    }

    pub(crate) fn pending(&self) -> Option<DeliveryChange> {
        self.pending
    }

    pub(crate) fn complete(&mut self, change: DeliveryChange, success: bool) {
        self.pending = None;
        match change {
            DeliveryChange::Register => {
                self.registered = success;
            }
            DeliveryChange::Unregister => {
                self.registered = !success;
                self.unregister_failed = !success;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeliveryChange::*, *};

    fn registered() -> FrameDemand {
        let mut demand = FrameDemand::default();
        demand.set_active(true);
        assert_eq!(demand.pending(), Some(Register));
        demand.complete(Register, true);
        demand
    }

    fn become_idle(demand: &mut FrameDemand) {
        assert_eq!(demand.on_callback(), None);
        for _ in 1..IDLE_CALLBACK_THRESHOLD {
            assert_eq!(demand.on_callback(), None);
        }
        assert_eq!(demand.on_callback(), Some(Unregister));
    }

    #[test]
    fn first_demand_registers_and_coalesces() {
        let mut demand = registered();
        assert_eq!(demand.request(), None);
        assert_eq!(demand.on_callback(), None);
    }

    #[test]
    fn sustained_demand_never_unregisters() {
        let mut demand = registered();
        for _ in 0..IDLE_CALLBACK_THRESHOLD * 10 {
            assert_eq!(demand.on_callback(), None);
            assert_eq!(demand.request(), None);
        }
        assert_eq!(demand.idle_callbacks, 0);
    }

    #[test]
    fn idle_callbacks_defer_unregister_until_completion() {
        let mut demand = registered();
        become_idle(&mut demand);
        assert!(demand.registered);
        assert_eq!(demand.idle_callbacks, IDLE_CALLBACK_THRESHOLD);
        demand.complete(demand.pending().unwrap(), true);
        assert!(!demand.registered);
        assert_eq!(demand.request(), Some(Register));
    }

    #[test]
    fn demand_cancels_deferred_unregister_even_in_threshold_callback() {
        let mut demand = registered();
        become_idle(&mut demand);
        assert_eq!(demand.request(), None);
        assert_eq!(demand.pending(), None);
        assert_eq!(demand.on_callback(), None);
    }

    #[test]
    fn short_bursts_reset_the_idle_threshold() {
        let mut demand = registered();
        for _ in 0..4 {
            demand.on_callback();
            for _ in 1..IDLE_CALLBACK_THRESHOLD {
                assert_eq!(demand.on_callback(), None);
            }
            assert_eq!(demand.request(), None);
        }
    }

    #[test]
    fn hide_and_restore_request_a_first_frame() {
        let mut demand = registered();
        demand.set_active(false);
        assert_eq!(demand.pending(), Some(Unregister));
        demand.complete(Unregister, true);
        assert_eq!(demand.request(), None);
        demand.set_active(true);
        assert_eq!(demand.pending(), Some(Register));
    }

    #[test]
    fn surface_recreation_discards_pending_unregistration() {
        let mut demand = registered();
        become_idle(&mut demand);
        demand.surface_destroyed();
        assert!(!demand.registered);
        demand.complete(Unregister, true);
        demand.set_active(true);
        assert_eq!(demand.pending(), Some(Register));
    }

    #[test]
    fn unexpected_callback_reconciles_registration_without_losing_demand() {
        let mut demand = FrameDemand::default();
        demand.set_active(true);
        assert_eq!(demand.on_callback(), None);
        assert!(demand.registered);
        assert_eq!(demand.request(), None);
        become_idle(&mut demand);
    }

    #[test]
    fn hidden_callback_requests_only_unregistration() {
        let mut demand = FrameDemand::default();
        demand.complete(Unregister, true);
        assert_eq!(demand.on_callback(), Some(Unregister));
        assert_eq!(demand.request(), None);
        assert_eq!(demand.pending(), Some(Unregister));
    }

    #[test]
    fn failed_registration_retries_only_on_new_demand() {
        let mut demand = FrameDemand::default();
        demand.set_active(true);
        demand.complete(Register, false);
        assert!(!demand.registered);
        demand.set_active(true);
        assert_eq!(demand.pending(), None);
        assert_eq!(demand.request(), Some(Register));
    }

    #[test]
    fn failed_unregister_stays_continuous_until_surface_destruction() {
        let mut demand = registered();
        become_idle(&mut demand);
        demand.complete(Unregister, false);
        assert!(demand.registered);
        for _ in 0..IDLE_CALLBACK_THRESHOLD * 10 {
            assert_eq!(demand.on_callback(), None);
        }
        demand.request();
        demand.set_active(false);
        assert_eq!(demand.pending(), None);
        demand.set_active(true);
        assert_eq!(demand.pending(), None);
        demand.surface_destroyed();
        demand.set_active(true);
        assert_eq!(demand.pending(), Some(Register));
    }

    #[test]
    fn windows_keep_independent_demand() {
        let mut main = registered();
        let mut sub = registered();
        become_idle(&mut main);
        sub.request();
        assert_eq!(sub.on_callback(), None);
        assert_eq!(main.pending(), Some(Unregister));
    }
}
