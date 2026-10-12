use std::time::{Duration, Instant};

use super::{frame_request::FrameTicket, presentation::Presentation};

#[derive(Default)]
struct Samples(Vec<f64>);

impl Samples {
    fn push(&mut self, value: f64) {
        self.0.push(value);
    }

    fn stats(&mut self) -> Stats {
        self.0.sort_unstable_by(f64::total_cmp);
        if self.0.is_empty() {
            return Stats::default();
        }
        let count = self.0.len();
        let median = if count.is_multiple_of(2) {
            (self.0[count / 2 - 1] + self.0[count / 2]) / 2.0
        } else {
            self.0[count / 2]
        };
        Stats {
            median,
            p95: self.0[(count * 95).div_ceil(100) - 1],
            min: self.0[0],
            max: self.0[count - 1],
            late: self.0.iter().filter(|&&value| value > median * 1.5).count(),
        }
    }
}

#[derive(Default)]
struct Stats {
    median: f64,
    p95: f64,
    min: f64,
    max: f64,
    late: usize,
}

#[derive(Default)]
struct Summary {
    callbacks: usize,
    draws: usize,
    presented: usize,
    retry: usize,
    unavailable: usize,
    intervals: Samples,
    dispatch: Samples,
    request: Samples,
    first_callback: Samples,
    register_us: Vec<u128>,
    unregister_us: Vec<u128>,
    register_failed: usize,
    unregister_failed: usize,
    timestamp_delta: Samples,
    target_delta: Samples,
}

pub(crate) struct FrameMetrics {
    window_id: i64,
    mode: &'static str,
    summary: Summary,
    started: Instant,
    last_callback: Option<Instant>,
    last_demand: Option<Instant>,
    registered_at: Option<Instant>,
    requested: Option<FrameTicket>,
    pending: bool,
    idle: bool,
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn monotonic_ns() -> Option<i128> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes into a valid timespec.
    (unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } == 0)
        .then(|| i128::from(time.tv_sec) * 1_000_000_000 + i128::from(time.tv_nsec))
}

impl FrameMetrics {
    pub(crate) fn new(window_id: i64, mode: &'static str) -> Self {
        Self {
            window_id,
            mode,
            summary: Summary::default(),
            started: Instant::now(),
            last_callback: None,
            last_demand: None,
            registered_at: None,
            requested: None,
            pending: false,
            idle: true,
        }
    }

    pub(crate) fn set_mode(&mut self, mode: &'static str) {
        if self.mode != mode {
            self.enter_idle(0);
            self.mode = mode;
            self.requested = None;
            self.pending = false;
        }
    }

    pub(crate) fn demand(&mut self) {
        let now = Instant::now();
        if self.idle {
            self.started = now;
            self.idle = false;
        }
        self.last_demand = Some(now);
    }

    pub(crate) fn vsync_requested(&mut self, ticket: FrameTicket) {
        self.requested = Some(ticket);
    }

    pub(crate) fn vsync_failed(&mut self) {
        self.requested = None;
    }

    pub(crate) fn vsync_callback(
        &mut self,
        ticket: FrameTicket,
        requested_at: Instant,
        now: Instant,
        accepted: bool,
    ) {
        if self.mode != "vsync" {
            return;
        }
        self.callback(now);
        self.summary
            .request
            .push(milliseconds(now.saturating_duration_since(requested_at)));
        if self.requested == Some(ticket) {
            self.requested = None;
        }
        self.pending |= accepted;
    }

    fn callback(&mut self, now: Instant) {
        if self.idle {
            self.started = now;
            self.idle = false;
        }
        self.summary.callbacks += 1;
        if let Some(previous) = self.last_callback.replace(now) {
            self.summary
                .intervals
                .push(milliseconds(now.saturating_duration_since(previous)));
        }
    }

    pub(crate) fn xcomponent_callback(&mut self, time_stamp: i64, target_time_stamp: i64) {
        let now = Instant::now();
        let monotonic = monotonic_ns();
        self.set_mode("xcomponent");
        self.callback(now);
        if let Some(start) = self.registered_at.take() {
            self.summary
                .first_callback
                .push(milliseconds(now.saturating_duration_since(start)));
        }
        if let Some(monotonic) = monotonic {
            self.summary
                .timestamp_delta
                .push((i128::from(time_stamp) - monotonic) as f64 / 1_000_000.0);
            self.summary
                .target_delta
                .push((i128::from(target_time_stamp) - monotonic) as f64 / 1_000_000.0);
        }
    }

    pub(crate) fn delivery_call(&mut self, register: bool, start: Instant, success: bool) {
        let elapsed = start.elapsed().as_micros();
        if register {
            self.summary.register_us.push(elapsed);
            self.summary.register_failed += usize::from(!success);
            self.registered_at = success.then_some(start);
        } else {
            self.summary.unregister_us.push(elapsed);
            self.summary.unregister_failed += usize::from(!success);
            if success {
                self.registered_at = None;
            }
        }
    }

    pub(crate) fn begin_frame(&mut self, frame_time: Instant) {
        if self.mode == "vsync" {
            self.pending = false;
            self.summary.dispatch.push(milliseconds(
                Instant::now().saturating_duration_since(frame_time),
            ));
        }
    }

    pub(crate) fn draw(&mut self, result: Presentation) {
        self.summary.draws += 1;
        match result {
            Presentation::Presented => self.summary.presented += 1,
            Presentation::Retry => self.summary.retry += 1,
            Presentation::Unavailable => self.summary.unavailable += 1,
        }
    }

    pub(crate) fn end_frame(&mut self) {
        if self.mode == "vsync" && self.requested.is_none() && !self.pending {
            self.enter_idle(0);
        } else if self.started.elapsed() >= Duration::from_secs(1) {
            self.report("active", 0);
        }
    }

    pub(crate) fn deactivate(&mut self, extra_callbacks: usize) {
        self.requested = None;
        self.pending = false;
        self.registered_at = None;
        self.enter_idle(extra_callbacks);
    }

    pub(crate) fn enter_idle(&mut self, extra_callbacks: usize) {
        if !self.idle {
            self.report("idle", extra_callbacks);
            self.idle = true;
        }
        self.last_callback = None;
    }

    fn report(&mut self, event: &str, extra_callbacks: usize) {
        let now = Instant::now();
        let mut summary = std::mem::take(&mut self.summary);
        let interval = summary.intervals.stats();
        let dispatch = summary.dispatch.stats();
        let request = summary.request.stats();
        let first = summary.first_callback.stats();
        let timestamp = summary.timestamp_delta.stats();
        let target = summary.target_delta.stats();
        log::info!(
            "gpui-frame-metrics window={} mode={} event={} span_ms={:.3} callbacks={} draws={} presented={} retry={} unavailable={} interval_ms_p50={:.3} interval_ms_p95={:.3} interval_ms_max={:.3} late={} vsync_to_draw_ms_p50={:.3} vsync_to_draw_ms_max={:.3} request_to_callback_ms_p50={:.3} request_to_callback_ms_max={:.3} registers={} register_us={:?} register_failed={} unregisters={} unregister_us={:?} unregister_failed={} register_to_callback_ms_p50={:.3} register_to_callback_ms_max={:.3} timestamp_minus_monotonic_ms_p50={:.3} timestamp_minus_monotonic_ms_min={:.3} timestamp_minus_monotonic_ms_max={:.3} target_minus_monotonic_ms_p50={:.3} target_minus_monotonic_ms_min={:.3} target_minus_monotonic_ms_max={:.3} last_demand_ms={:.3} idle_extra_callbacks={}",
            self.window_id,
            self.mode,
            event,
            milliseconds(now.saturating_duration_since(self.started)),
            summary.callbacks,
            summary.draws,
            summary.presented,
            summary.retry,
            summary.unavailable,
            interval.median,
            interval.p95,
            interval.max,
            interval.late,
            dispatch.median,
            dispatch.max,
            request.median,
            request.max,
            summary.register_us.len(),
            summary.register_us,
            summary.register_failed,
            summary.unregister_us.len(),
            summary.unregister_us,
            summary.unregister_failed,
            first.median,
            first.max,
            timestamp.median,
            timestamp.min,
            timestamp.max,
            target.median,
            target.min,
            target.max,
            self.last_demand.map_or(0.0, |last| milliseconds(
                now.saturating_duration_since(last)
            )),
            extra_callbacks,
        );
        self.started = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_statistics_include_stalls_and_nearest_rank_p95() {
        let mut samples = Samples(vec![16.0; 19]);
        samples.push(50.0);
        let stats = samples.stats();
        assert_eq!(
            (stats.median, stats.p95, stats.max, stats.late),
            (16.0, 16.0, 50.0, 1)
        );
        assert_eq!(Samples::default().stats().median, 0.0);
        assert_eq!(Samples(vec![-4.0, 2.0]).stats().median, -1.0);
    }

    #[test]
    fn idle_gap_is_excluded_from_callback_intervals() {
        let mut metrics = FrameMetrics::new(3, "xcomponent");
        let now = Instant::now();
        metrics.demand();
        metrics.callback(now);
        metrics.callback(now + Duration::from_millis(16));
        metrics.enter_idle(12);
        metrics.demand();
        metrics.callback(now + Duration::from_secs(10));
        assert!(metrics.summary.intervals.0.is_empty());
        assert_eq!(metrics.summary.callbacks, 1);
    }

    #[test]
    fn native_idle_waits_for_both_requested_and_queued_ticks() {
        let state = super::super::frame_request::FrameRequest::default();
        state.set_active(true);
        let ticket = state.request().unwrap();
        let mut metrics = FrameMetrics::new(0, "vsync");
        metrics.demand();
        metrics.vsync_requested(ticket);
        metrics.end_frame();
        assert!(!metrics.idle);
        let now = Instant::now();
        metrics.vsync_callback(ticket, now, now, true);
        metrics.end_frame();
        assert!(!metrics.idle);
        metrics.begin_frame(now);
        metrics.end_frame();
        assert!(metrics.idle);
    }
}
