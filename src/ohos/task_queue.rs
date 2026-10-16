//! Priority work queue with an explicit shutdown boundary.
use crate::Priority;
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

struct State<T> {
    queues: [VecDeque<T>; 3],
    cursor: usize,
    closed: bool,
    started: u64,
    wake_on_progress: bool,
    #[cfg(test)]
    progress_notifications: usize,
}

pub(crate) struct TaskQueue<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
    changed: Condvar,
}

pub(crate) struct StallBackoff {
    interval: Duration,
    started: u64,
}

impl Default for StallBackoff {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(10),
            started: 0,
        }
    }
}

impl StallBackoff {
    pub(crate) fn reset(&mut self) {
        self.interval = Duration::from_millis(10);
    }

    pub(crate) fn back_off(&mut self) {
        self.interval = (self.interval * 2).min(Duration::from_millis(100));
    }
}

impl<T> Default for TaskQueue<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                queues: std::array::from_fn(|_| VecDeque::new()),
                cursor: 0,
                closed: false,
                started: 0,
                wake_on_progress: false,
                #[cfg(test)]
                progress_notifications: 0,
            }),
            ready: Condvar::new(),
            changed: Condvar::new(),
        }
    }
}

impl<T> TaskQueue<T> {
    pub(crate) fn push(&self, priority: Priority, task: T) -> Result<(), T> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(task);
        }
        let index = match priority {
            Priority::High => 0,
            Priority::Medium => 1,
            Priority::Low => 2,
            Priority::RealtimeAudio => unreachable!("realtime work has a dedicated thread"),
        };
        let was_empty = state.queues.iter().all(VecDeque::is_empty);
        state.queues[index].push_back(task);
        self.ready.notify_one();
        if was_empty {
            self.changed.notify_one();
        }
        Ok(())
    }

    fn take(state: &mut State<T>) -> Option<T> {
        // GPUI's priority weights are 60/30/10. FIFO service with the same
        // proportions also guarantees that low-priority work makes progress.
        const ORDER: [usize; 10] = [0, 0, 0, 0, 0, 0, 1, 1, 1, 2];
        for _ in 0..ORDER.len() {
            let index = ORDER[state.cursor];
            state.cursor = (state.cursor + 1) % ORDER.len();
            if let Some(task) = state.queues[index].pop_front() {
                state.started = state.started.wrapping_add(1);
                return Some(task);
            }
        }
        None
    }

    pub(crate) fn pop(&self) -> Option<T> {
        self.pop_until(None)
    }

    pub(crate) fn pop_timeout(&self, timeout: Duration) -> Option<T> {
        self.pop_until(Some(Instant::now() + timeout))
    }

    fn pop_until(&self, deadline: Option<Instant>) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(task) = Self::take(&mut state) {
                if state.wake_on_progress {
                    state.wake_on_progress = false;
                    #[cfg(test)]
                    {
                        state.progress_notifications += 1;
                    }
                    self.changed.notify_one();
                }
                return Some(task);
            }
            if state.closed {
                return None;
            }
            state = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return None;
                    }
                    self.ready
                        .wait_timeout(state, remaining)
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                }
                None => self.ready.wait(state).unwrap_or_else(|e| e.into_inner()),
            };
        }
    }

    // The monitor sleeps when there is no backlog. Stalling requests an OS-state
    // observation; it does not by itself authorize another worker.
    pub(crate) fn wait_for_stalled_work(&self, backoff: &mut StallBackoff) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            while !state.closed && state.queues.iter().all(VecDeque::is_empty) {
                backoff.reset();
                state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.closed {
                return false;
            }
            if backoff.started != state.started {
                backoff.reset();
                backoff.started = state.started;
            }
            state.wake_on_progress = backoff.interval > Duration::from_millis(10);
            let (next, timeout) = self
                .changed
                .wait_timeout(state, backoff.interval)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
            state.wake_on_progress = false;
            if state.closed {
                return false;
            }
            if timeout.timed_out()
                && backoff.started == state.started
                && state.queues.iter().any(|queue| !queue.is_empty())
            {
                return true;
            }
        }
    }

    pub(crate) fn close(&self) -> Vec<T> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        let remaining = state
            .queues
            .iter_mut()
            .flat_map(|queue| queue.drain(..))
            .collect();
        self.ready.notify_all();
        self.changed.notify_all();
        remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn priorities_are_fifo_and_low_work_is_serviced() {
        let q = TaskQueue::default();
        for i in 0..6 {
            q.push(Priority::High, i).unwrap();
        }
        q.push(Priority::Medium, 10).unwrap();
        q.push(Priority::Medium, 11).unwrap();
        q.push(Priority::Low, 20).unwrap();
        let result: Vec<_> = (0..9).map(|_| q.pop().unwrap()).collect();
        assert_eq!(result, [0, 1, 2, 3, 4, 5, 10, 11, 20]);
    }
    #[test]
    fn closing_wakes_waiters_and_rejects_new_work() {
        let q = std::sync::Arc::new(TaskQueue::<u32>::default());
        let other = q.clone();
        let worker = std::thread::spawn(move || other.pop());
        assert!(q.close().is_empty());
        assert_eq!(worker.join().unwrap(), None);
        assert_eq!(q.push(Priority::Low, 7), Err(7));
    }
    #[test]
    fn an_idle_elastic_worker_can_retire_without_closing_the_queue() {
        let q = TaskQueue::default();
        assert_eq!(q.pop_timeout(Duration::from_millis(5)), None);
        q.push(Priority::Medium, 7).unwrap();
        assert_eq!(q.pop(), Some(7));
    }
    #[test]
    fn dequeues_notify_only_once_per_backed_off_wait() {
        let q = TaskQueue::default();
        for task in 0..6 {
            q.push(Priority::Medium, task).unwrap();
        }
        assert_eq!(q.pop(), Some(0));
        assert_eq!(q.state.lock().unwrap().progress_notifications, 0);

        q.state.lock().unwrap().wake_on_progress = true;
        for task in 1..4 {
            assert_eq!(q.pop(), Some(task));
        }
        {
            let state = q.state.lock().unwrap();
            assert!(!state.wake_on_progress);
            assert_eq!(state.progress_notifications, 1);
        }

        q.state.lock().unwrap().wake_on_progress = true;
        assert_eq!(q.pop(), Some(4));
        assert_eq!(q.pop(), Some(5));
        assert_eq!(q.state.lock().unwrap().progress_notifications, 2);
    }
    #[test]
    fn stalled_observations_back_off_until_work_progresses() {
        let q = TaskQueue::default();
        let mut backoff = StallBackoff::default();
        q.push(Priority::Medium, 1).unwrap();
        q.push(Priority::Medium, 2).unwrap();
        for millis in [10, 20, 40, 80, 100, 100] {
            let start = Instant::now();
            assert!(q.wait_for_stalled_work(&mut backoff));
            assert_eq!(backoff.interval, Duration::from_millis(millis));
            assert!(start.elapsed() >= backoff.interval);
            backoff.back_off();
        }
        assert_eq!(q.pop(), Some(1));
        assert!(q.wait_for_stalled_work(&mut backoff));
        assert_eq!(backoff.interval, Duration::from_millis(10));
    }
    #[test]
    fn progress_wakes_a_monitor_in_backoff() {
        let q = std::sync::Arc::new(TaskQueue::default());
        q.push(Priority::Medium, 1).unwrap();
        q.push(Priority::Medium, 2).unwrap();
        let other = q.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let monitor = std::thread::spawn(move || {
            let mut backoff = StallBackoff {
                interval: Duration::from_secs(60),
                started: 0,
            };
            assert!(other.wait_for_stalled_work(&mut backoff));
            tx.send(backoff.interval).unwrap();
        });
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(q.pop(), Some(1));
        let result = rx.recv_timeout(Duration::from_secs(1));
        q.close();
        monitor.join().unwrap();
        assert_eq!(result.unwrap(), Duration::from_millis(10));
    }
    #[test]
    fn empty_queue_sleeps_and_resets_observation_backoff() {
        let q = std::sync::Arc::new(TaskQueue::default());
        let other = q.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let monitor = std::thread::spawn(move || {
            let mut backoff = StallBackoff {
                interval: Duration::from_millis(100),
                started: 0,
            };
            assert!(other.wait_for_stalled_work(&mut backoff));
            tx.send(backoff.interval).unwrap();
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(150)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        q.push(Priority::Medium, 1).unwrap();
        let result = rx.recv_timeout(Duration::from_secs(1));
        q.close();
        monitor.join().unwrap();
        assert_eq!(result.unwrap(), Duration::from_millis(10));
    }
    #[test]
    fn shutdown_wakes_the_idle_growth_monitor() {
        let q = std::sync::Arc::new(TaskQueue::<u32>::default());
        let other = q.clone();
        let monitor =
            std::thread::spawn(move || other.wait_for_stalled_work(&mut StallBackoff::default()));
        assert!(q.close().is_empty());
        assert!(!monitor.join().unwrap());
    }
}
