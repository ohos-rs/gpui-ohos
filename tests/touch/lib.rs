// Native wake and the main queue are host shims; GPUI futures and production queues are real.
#[cfg(test)]
extern crate self as napi_ohos;
extern crate self as openharmony_ability;
#[cfg(test)]
#[path = "support/frame_delivery.rs"]
mod frame_delivery_support;
#[cfg(test)]
pub use frame_delivery_support::{Error, FrameInputDelivery, OpenHarmonyApp};
#[cfg(test)]
pub type Result<T, E = Error> = std::result::Result<T, E>;
#[cfg(all(test, feature = "native-vsync"))]
use frame_delivery_support::native_frames;
#[cfg(test)]
use frame_delivery_support::window;
#[allow(dead_code)]
#[path = "../../src/ohos/cursor_visibility.rs"]
mod cursor_visibility;
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../src/ohos/frame_delivery.rs"]
mod frame_delivery;
#[allow(dead_code)]
#[path = "../../src/ohos/launch.rs"]
mod launch;
pub use gpui::{Pixels, PlatformDispatcher, Priority, RunnableVariant, Size};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Clone, Default)]
pub struct OpenHarmonyWaker {
    calls: Arc<AtomicUsize>,
}
impl OpenHarmonyWaker {
    pub fn wake(&self) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}
#[allow(dead_code)]
#[path = "../../src/ohos/capture_pixels.rs"]
mod capture_pixels;
#[allow(dead_code)]
#[path = "../../src/ohos/dispatcher.rs"]
mod dispatcher;
#[allow(dead_code)]
#[path = "../../src/ohos/frame_demand.rs"]
mod frame_demand;
#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../../src/ohos/worker_state.rs"]
mod native_worker_state;
#[allow(dead_code)]
#[path = "../../src/ohos/presentation.rs"]
mod presentation;
#[allow(dead_code)]
#[path = "../../src/ohos/quad_pipeline.rs"]
mod quad_pipeline;
#[allow(dead_code)]
#[path = "../../src/ohos/render_cache.rs"]
mod render_cache;
#[path = "../../src/ohos/task_queue.rs"]
mod task_queue;
#[allow(dead_code)]
#[path = "../../src/ohos/touch_scroll.rs"]
mod touch_scroll;
#[allow(dead_code)]
#[path = "../../src/ohos/viewport.rs"]
mod viewport;
#[cfg(target_os = "macos")]
#[path = "support/macos_worker_state.rs"]
mod worker_state;
#[cfg(not(target_os = "macos"))]
#[path = "../../src/ohos/worker_state.rs"]
mod worker_state;
#[path = "../../src/ohos/workers.rs"]
mod workers;

pub struct PriorityQueueSender<T>(Arc<task_queue::TaskQueue<T>>);
#[cfg(test)]
pub struct PriorityQueueReceiver<T>(Arc<task_queue::TaskQueue<T>>);
impl<T> PriorityQueueSender<T> {
    fn send(&self, priority: Priority, value: T) -> Result<(), T> {
        self.0.push(priority, value)
    }
}
#[cfg(test)]
impl<T> PriorityQueueReceiver<T> {
    fn new() -> (PriorityQueueSender<T>, Self) {
        let state = Arc::new(task_queue::TaskQueue::default());
        (PriorityQueueSender(state.clone()), Self(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{BackgroundExecutor, ForegroundExecutor};
    use std::{
        collections::HashSet,
        future::Future,
        pin::Pin,
        sync::Mutex,
        task::{Context, Poll},
        time::{Duration, Instant},
    };
    struct YieldOnce(bool);
    impl Future for YieldOnce {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
    #[test]
    fn cooperative_background_rescheduling_reuses_workers() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let executor = BackgroundExecutor::new(dispatcher.clone());
        let ids = Arc::new(Mutex::new(HashSet::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..16 {
            let ids = ids.clone();
            let tx = tx.clone();
            executor
                .spawn(async move {
                    for _ in 0..64 {
                        ids.lock().unwrap().insert(std::thread::current().id());
                        YieldOnce(false).await;
                    }
                    tx.send(()).unwrap();
                })
                .detach();
        }
        for _ in 0..16 {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(ids.lock().unwrap().len() <= 8);
        dispatcher.shutdown();
    }
    #[test]
    fn cpu_bound_polls_do_not_expand_the_worker_pool() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let executor = BackgroundExecutor::new(dispatcher.clone());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..32 {
            let (active, peak, tx) = (active.clone(), peak.clone(), tx.clone());
            executor
                .spawn(async move {
                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    let end = Instant::now() + Duration::from_millis(100);
                    let mut x = 1u64;
                    while Instant::now() < end {
                        for _ in 0..1000 {
                            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                        }
                        std::hint::black_box(x);
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    tx.send(()).unwrap();
                })
                .detach();
        }
        for _ in 0..32 {
            rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        let base = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 8));
        assert!(peak.load(Ordering::SeqCst) <= base);
        dispatcher.shutdown();
    }
    #[test]
    fn blocking_background_parents_can_wait_for_queued_children() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let executor = BackgroundExecutor::new(dispatcher.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..16 {
            let executor = executor.clone();
            let tx = tx.clone();
            executor
                .clone()
                .spawn(async move {
                    let (child_tx, child_rx) = std::sync::mpsc::channel();
                    executor
                        .spawn(async move {
                            child_tx.send(()).unwrap();
                        })
                        .detach();
                    child_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    tx.send(()).unwrap();
                })
                .detach();
        }
        for _ in 0..16 {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        dispatcher.shutdown();
    }
    #[test]
    fn external_work_progresses_while_all_base_workers_are_blocked() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let executor = BackgroundExecutor::new(dispatcher.clone());
        let count = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 8));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let mut releases = Vec::new();
        for _ in 0..count {
            let (tx, rx) = std::sync::mpsc::channel();
            releases.push(tx);
            let started = started_tx.clone();
            executor
                .spawn(async move {
                    started.send(()).unwrap();
                    rx.recv_timeout(Duration::from_secs(3)).unwrap();
                })
                .detach();
        }
        for _ in 0..count {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        executor
            .spawn(async move {
                done_tx.send(()).unwrap();
            })
            .detach();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        for release in releases {
            release.send(()).unwrap();
        }
        dispatcher.shutdown();
    }
    #[test]
    fn blocked_workers_stop_growing_at_the_limit_and_resume_queued_work() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let executor = BackgroundExecutor::new(dispatcher.clone());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut releases = Vec::new();
        for _ in 0..65 {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            releases.push(tx);
            let started = started_tx.clone();
            let done = done_tx.clone();
            executor
                .spawn(async move {
                    started.send(()).unwrap();
                    let _ = rx.recv();
                    let _ = done.send(());
                })
                .detach();
        }
        for _ in 0..64 {
            started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        assert_eq!(
            started_rx.recv_timeout(Duration::from_millis(250)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        drop(releases);
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        for _ in 0..65 {
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        dispatcher.shutdown();
    }
    #[test]
    fn main_queue_wakes_once_per_burst_and_rearms_after_pumping() {
        let (sender, queue) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let wake = OpenHarmonyWaker::default();
        dispatcher.set_waker(wake.clone());
        dispatcher.begin_main_turn();
        let before = wake.calls.load(Ordering::Relaxed);
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let count = Arc::new(AtomicUsize::new(0));
        for _ in 0..100 {
            let count = count.clone();
            foreground
                .spawn(async move {
                    count.fetch_add(1, Ordering::Relaxed);
                })
                .detach();
        }
        assert_eq!(wake.calls.load(Ordering::Relaxed) - before, 1);
        dispatcher.begin_main_turn();
        for _ in 0..100 {
            queue.0.pop().unwrap().run();
        }
        assert_eq!(count.load(Ordering::Relaxed), 100);
        let before = wake.calls.load(Ordering::Relaxed);
        foreground.spawn(async {}).detach();
        assert_eq!(wake.calls.load(Ordering::Relaxed) - before, 1);
        queue.0.pop().unwrap().run();
        dispatcher.shutdown();
    }
    #[test]
    fn foreground_future_resumes_after_timer_fires_without_ui_timer_pumping() {
        let (sender, queue) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let wake = OpenHarmonyWaker::default();
        dispatcher.set_waker(wake.clone());
        dispatcher.begin_main_turn();
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let background = BackgroundExecutor::new(dispatcher.clone());
        let done = std::rc::Rc::new(std::cell::Cell::new(false));
        let observed = done.clone();
        let ui_thread = std::thread::current().id();
        foreground
            .spawn(async move {
                background.timer(Duration::from_millis(10)).await;
                assert_eq!(std::thread::current().id(), ui_thread);
                observed.set(true);
            })
            .detach();
        queue.0.pop().unwrap().run();
        dispatcher.begin_main_turn();
        let resumed = queue.0.pop_timeout(Duration::from_secs(1)).unwrap();
        assert!(!done.get());
        resumed.run();
        assert!(done.get());
        dispatcher.shutdown();
    }
    #[test]
    fn background_timer_completes_while_ui_thread_runs_no_tasks() {
        let (sender, queue) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let wake = OpenHarmonyWaker::default();
        dispatcher.set_waker(wake.clone());
        dispatcher.begin_main_turn();
        let before = wake.calls.load(Ordering::Relaxed);
        let background = BackgroundExecutor::new(dispatcher.clone());
        let executor = background.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        background
            .spawn(async move {
                executor.timer(Duration::from_millis(10)).await;
                tx.send(()).unwrap();
            })
            .detach();
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(wake.calls.load(Ordering::Relaxed), before);
        assert!(queue.0.pop_timeout(Duration::ZERO).is_none());
        dispatcher.shutdown();
    }
    #[test]
    fn ui_thread_can_block_with_timeout_on_a_background_timer() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let background = BackgroundExecutor::new(dispatcher.clone());
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        assert!(
            foreground
                .block_with_timeout(
                    Duration::from_secs(1),
                    background.timer(Duration::from_millis(10)),
                )
                .is_ok()
        );
        dispatcher.shutdown();
    }
    #[test]
    fn shutdown_releases_a_waiting_timer_without_waiting_for_its_deadline() {
        let (sender, queue) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(dispatcher::OhosDispatcher::new(sender));
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let background = BackgroundExecutor::new(dispatcher.clone());
        let task = foreground.spawn(async move {
            background.timer(Duration::from_secs(3600)).await;
        });
        queue.0.pop().unwrap().run();
        let start = Instant::now();
        dispatcher.shutdown();
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(task);
    }
}

#[cfg(test)]
extern crate self as ohos_fileshare_binding;
#[cfg(test)]
pub struct PolicyInfo {
    pub uri: String,
    pub operation_mode: u32,
}
#[cfg(test)]
pub fn persist_permission(_: &[PolicyInfo]) -> anyhow::Result<Vec<String>> {
    anyhow::bail!("Unexpected native file grant")
}
#[cfg(test)]
pub fn activate_permission(_: &[PolicyInfo]) -> anyhow::Result<Vec<String>> {
    anyhow::bail!("Unexpected native file activation")
}
#[cfg(test)]
pub fn check_persistent_permission(_: &[PolicyInfo]) -> anyhow::Result<Vec<bool>> {
    anyhow::bail!("Unexpected native file query")
}
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../src/ohos/file_access.rs"]
mod file_access;

#[allow(dead_code)]
#[path = "../../src/ohos/mouse_buttons.rs"]
mod mouse_buttons;
#[allow(dead_code)]
#[path = "../../src/ohos/pointer_position.rs"]
#[cfg(feature = "native-input")]
mod pointer_position;
#[allow(dead_code)]
#[path = "../../src/ohos/scroll_input.rs"]
#[cfg(feature = "native-input")]
mod scroll_input;
#[allow(dead_code)]
#[path = "../../src/ohos/scroll_packet.rs"]
mod scroll_packet;
