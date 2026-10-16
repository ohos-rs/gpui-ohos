use super::task_queue::TaskQueue;
use crate::{PlatformDispatcher, Priority, PriorityQueueSender, RunnableVariant};
use openharmony_ability::OpenHarmonyWaker;
use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    thread,
    time::{Duration, Instant},
};

struct TimerAfter {
    when: Instant,
    sequence: u64,
    runnable: RunnableVariant,
}
impl Ord for TimerAfter {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .when
            .cmp(&self.when)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}
impl PartialOrd for TimerAfter {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for TimerAfter {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when && self.sequence == other.sequence
    }
}
impl Eq for TimerAfter {}
#[derive(Default)]
struct TimerState {
    heap: BinaryHeap<TimerAfter>,
    next_sequence: u64,
    closed: bool,
}
#[derive(Default)]
struct MainWake {
    requested: AtomicBool,
    waker: Mutex<Option<OpenHarmonyWaker>>,
    stopped: AtomicBool,
    enqueue_gate: Mutex<()>,
}
impl MainWake {
    fn notify(&self) {
        if self.stopped.load(AtomicOrdering::Acquire)
            || self.requested.swap(true, AtomicOrdering::AcqRel)
        {
            return;
        }
        let waker = self.waker.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(waker) = waker {
            waker.wake();
        } else {
            self.requested.store(false, AtomicOrdering::Release);
        }
    }
}

pub(crate) struct OhosDispatcher {
    main_thread_id: thread::ThreadId,
    main_sender: PriorityQueueSender<RunnableVariant>,
    background: Arc<TaskQueue<RunnableVariant>>,
    timers: Arc<(Mutex<TimerState>, Condvar)>,
    wake: Arc<MainWake>,
}
impl OhosDispatcher {
    pub(crate) fn new(main_sender: PriorityQueueSender<RunnableVariant>) -> Self {
        let background = Arc::new(TaskQueue::<RunnableVariant>::default());
        super::workers::Workers::start(background.clone());
        let timers = Arc::new((Mutex::new(TimerState::default()), Condvar::new()));
        let wake = Arc::new(MainWake::default());
        let timer_state = timers.clone();
        thread::Builder::new()
            .name("OhosTimer".into())
            .spawn(move || {
                let (lock, changed) = &*timer_state;
                loop {
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        if state.closed {
                            return;
                        }
                        match state.heap.peek() {
                            Some(next) if next.when <= Instant::now() => break,
                            Some(next) => {
                                let timeout = next.when.saturating_duration_since(Instant::now());
                                state = changed
                                    .wait_timeout(state, timeout)
                                    .unwrap_or_else(|e| e.into_inner())
                                    .0;
                            }
                            None => state = changed.wait(state).unwrap_or_else(|e| e.into_inner()),
                        }
                    }
                    let runnable = state.heap.pop().unwrap().runnable;
                    drop(state);
                    runnable.run();
                }
            })
            .expect("Failed to start OHOS timer");
        Self {
            main_thread_id: thread::current().id(),
            main_sender,
            background,
            timers,
            wake,
        }
    }
    pub(crate) fn set_waker(&self, waker: OpenHarmonyWaker) {
        *self.wake.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(waker);
        self.wake.notify();
    }
    pub(crate) fn begin_main_turn(&self) {
        self.wake.requested.store(false, AtomicOrdering::Release);
    }
    pub(crate) fn wake_main_thread(&self) {
        self.wake.notify();
    }
    pub(crate) fn execute_runnable(runnable: RunnableVariant) {
        runnable.run();
    }
    pub(crate) fn shutdown(&self) {
        {
            let _gate = self
                .wake
                .enqueue_gate
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if self.wake.stopped.swap(true, AtomicOrdering::AcqRel) {
                return;
            }
        }
        drop(self.background.close());
        let (lock, changed) = &*self.timers;
        let pending = {
            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            let pending: Vec<_> = state.heap.drain().map(|entry| entry.runnable).collect();
            changed.notify_all();
            pending
        };
        drop(pending);
        self.wake
            .waker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}
impl Drop for OhosDispatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}
impl PlatformDispatcher for OhosDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }
    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        if let Err(runnable) = self.background.push(priority, runnable) {
            drop(runnable);
        }
    }
    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        let gate = self
            .wake
            .enqueue_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.wake.stopped.load(AtomicOrdering::Acquire) {
            drop(gate);
            if self.is_main_thread() {
                drop(runnable);
            } else {
                std::mem::forget(runnable);
            }
            return;
        }
        let result = self.main_sender.send(priority, runnable);
        drop(gate);
        match result {
            Ok(()) => self.wake.notify(),
            Err(runnable) => std::mem::forget(runnable),
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        let (lock, changed) = &*self.timers;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            drop(state);
            drop(runnable);
            return;
        }
        let sequence = state.next_sequence;
        state.next_sequence = sequence.wrapping_add(1);
        state.heap.push(TimerAfter {
            when: Instant::now() + duration,
            sequence,
            runnable,
        });
        changed.notify_one();
    }
    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        thread::spawn(f);
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PriorityQueueReceiver;
    use gpui::{BackgroundExecutor, ForegroundExecutor};
    use std::{
        future::Future,
        sync::{Weak, mpsc},
        task::{Context, Wake, Waker},
    };

    struct TimerWake {
        index: usize,
        dispatcher: Weak<OhosDispatcher>,
        events: mpsc::Sender<(usize, String, bool)>,
    }

    impl Wake for TimerWake {
        fn wake(self: Arc<Self>) {
            let dispatcher = self.dispatcher.upgrade().unwrap();
            let unlocked = dispatcher.timers.0.try_lock().is_ok();
            self.events
                .send((
                    self.index,
                    thread::current().name().unwrap_or_default().to_owned(),
                    unlocked,
                ))
                .unwrap();
        }
    }

    #[test]
    fn timers_run_unlocked_on_timer_thread_in_deadline_and_fifo_order() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(OhosDispatcher::new(sender));
        let background = BackgroundExecutor::new(dispatcher.clone()).scheduler_executor();
        let (tx, rx) = mpsc::channel();
        let mut timers = Vec::new();
        for index in 0..3 {
            let mut timer = Box::pin(background.timer(Duration::from_secs(3600)));
            let waker = Waker::from(Arc::new(TimerWake {
                index,
                dispatcher: Arc::downgrade(&dispatcher),
                events: tx.clone(),
            }));
            assert!(
                timer
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            timers.push(timer);
        }
        {
            let mut state = dispatcher.timers.0.lock().unwrap();
            let now = Instant::now();
            let entries: Vec<_> = state.heap.drain().collect();
            for mut entry in entries {
                entry.when = if entry.sequence == 0 {
                    now
                } else {
                    now - Duration::from_millis(1)
                };
                state.heap.push(entry);
            }
        }
        dispatcher.timers.1.notify_one();
        for index in [1, 2, 0] {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                (index, "OhosTimer".into(), true)
            );
        }
        dispatcher.shutdown();
    }

    #[test]
    fn shutdown_and_late_dispatch_release_timers_off_the_ui_thread() {
        let (sender, _) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(OhosDispatcher::new(sender));
        let background = BackgroundExecutor::new(dispatcher.clone()).scheduler_executor();
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let timer = background.timer(Duration::from_secs(3600));
        let other = dispatcher.clone();
        let late_timer = thread::spawn(move || {
            other.shutdown();
            background.timer(Duration::from_secs(3600))
        })
        .join()
        .unwrap();
        assert!(
            foreground
                .block_with_timeout(Duration::from_secs(1), timer)
                .is_ok()
        );
        assert!(
            foreground
                .block_with_timeout(Duration::from_secs(1), late_timer)
                .is_ok()
        );
    }
}
