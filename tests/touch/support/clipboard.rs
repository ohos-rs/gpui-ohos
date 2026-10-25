//! Native clipboard shims; the adapter, GPUI tasks and worker queues are production code.
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread::{self, ThreadId},
    time::Duration,
};

use crate::OpenHarmonyApp;

#[derive(Clone, Debug)]
pub enum ClipboardError {
    InvalidContent(String),
    InvalidText { mime: String },
    EmptyBinaryUnsupported { mime: String },
    AuthorizationRequired,
    ApiUnavailable { required: i32 },
    RevisionUnavailable,
    Native(String),
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClipboardError {}

#[derive(Clone, Default)]
pub struct ClipboardRecord {
    pub text: Option<String>,
    pub metadata: Option<String>,
    pub encoded_image: Option<Vec<u8>>,
    pub uri: Option<String>,
}

type NativeResult<T> = std::result::Result<T, ClipboardError>;
type WriteGate = Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>;

pub(crate) struct NativeClipboard {
    records: Mutex<Vec<ClipboardRecord>>,
    revision: AtomicU32,
    denied: AtomicBool,
    calls: Mutex<Vec<(&'static str, ThreadId)>>,
    write_gate: Mutex<WriteGate>,
    read_gate: Mutex<WriteGate>,
}

impl Default for NativeClipboard {
    fn default() -> Self {
        Self {
            records: Mutex::default(),
            revision: AtomicU32::new(1),
            denied: AtomicBool::new(false),
            calls: Mutex::default(),
            write_gate: Mutex::default(),
            read_gate: Mutex::default(),
        }
    }
}

pub struct ClipboardBridgePlugin(pub(crate) Arc<NativeClipboard>);
impl ClipboardBridgePlugin {
    pub fn revision(&self) -> NativeResult<u32> {
        match self.0.revision.load(Ordering::SeqCst) {
            0 => Err(ClipboardError::RevisionUnavailable),
            revision => Ok(revision),
        }
    }
}

pub struct ClipboardClient {
    native: Arc<NativeClipboard>,
}
impl ClipboardClient {
    pub fn new(app: &OpenHarmonyApp) -> NativeResult<Self> {
        Ok(Self {
            native: app.0.clipboard.clone(),
        })
    }

    pub fn with_metadata_mime_type(self, mime: &str) -> NativeResult<Self> {
        assert_eq!(mime, "application/x-gpui-ohos-text-metadata-v1");
        Ok(self)
    }

    pub async fn read_records(&self) -> NativeResult<Vec<ClipboardRecord>> {
        self.native
            .calls
            .lock()
            .unwrap()
            .push(("read", thread::current().id()));
        let gate = self.native.read_gate.lock().unwrap().take();
        if let Some((started, resume)) = gate {
            started.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        if self.native.denied.load(Ordering::SeqCst) {
            return Err(ClipboardError::AuthorizationRequired);
        }
        Ok(self.native.records.lock().unwrap().clone())
    }

    pub async fn write_records(&self, records: Vec<ClipboardRecord>) -> NativeResult<()> {
        self.native
            .calls
            .lock()
            .unwrap()
            .push(("write", thread::current().id()));
        let gate = self.native.write_gate.lock().unwrap().take();
        if let Some((started, resume)) = gate {
            started.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        for record in &records {
            if record.text.as_ref().is_some_and(|text| text.contains('\0')) {
                return Err(ClipboardError::InvalidText {
                    mime: "text/plain".into(),
                });
            }
            if record.metadata.as_deref() == Some("") {
                return Err(ClipboardError::EmptyBinaryUnsupported {
                    mime: "application/x-gpui-ohos-text-metadata-v1".into(),
                });
            }
        }
        *self.native.records.lock().unwrap() = records;
        self.native.revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub async fn clear(&self) -> NativeResult<()> {
        self.write_records(Vec::new()).await
    }
}

pub fn get_path_from_uri(uri: &str) -> anyhow::Result<String> {
    uri.strip_prefix("file://")
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("unsupported URI"))
}

pub fn get_uri_from_path(path: &str) -> anyhow::Result<String> {
    Ok(format!("file://{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BackgroundExecutor, ClipboardEntry, ClipboardItem, ClipboardReadError, ClipboardString,
        ForegroundExecutor, Image, ImageFormat, PriorityQueueReceiver, Task,
        clipboard::OhosClipboard, dispatcher::OhosDispatcher,
    };
    use std::{
        cell::RefCell,
        future::Future,
        rc::Rc,
        task::{Context, Poll},
        time::Instant,
    };

    struct Harness {
        clipboard: OhosClipboard,
        app: OpenHarmonyApp,
        queue: PriorityQueueReceiver<crate::RunnableVariant>,
        dispatcher: Arc<OhosDispatcher>,
    }
    impl Harness {
        fn new() -> Self {
            let app = OpenHarmonyApp::default();
            let (sender, queue) = PriorityQueueReceiver::new();
            let dispatcher = Arc::new(OhosDispatcher::new(sender));
            let clipboard = OhosClipboard::new(
                Rc::new(RefCell::new(Some(app.clone()))),
                ForegroundExecutor::new(dispatcher.clone()),
                BackgroundExecutor::new(dispatcher.clone()),
            );
            Self {
                clipboard,
                app,
                queue,
                dispatcher,
            }
        }

        fn pump(&self) {
            if let Some(runnable) = self.queue.0.pop_timeout(Duration::from_millis(1)) {
                self.dispatcher.begin_main_turn();
                OhosDispatcher::execute_runnable(runnable);
            }
        }

        fn run<T: 'static>(&self, task: Task<T>) -> T {
            let mut task = std::pin::pin!(task);
            let mut cx = Context::from_waker(futures::task::noop_waker_ref());
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Poll::Ready(value) = task.as_mut().poll(&mut cx) {
                    return value;
                }
                assert!(Instant::now() < deadline, "clipboard task timed out");
                self.pump();
            }
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.dispatcher.shutdown();
        }
    }

    #[test]
    fn native_records_and_image_bytes_round_trip_on_workers() {
        let h = Harness::new();
        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::String(ClipboardString {
                    text: "".into(),
                    metadata: Some("{}".into()),
                }),
                ClipboardEntry::Image(Image::from_bytes(
                    ImageFormat::Png,
                    include_bytes!("../../../example/src/fixtures/clipboard.png").to_vec(),
                )),
            ],
        };
        h.run(h.clipboard.write(item.clone())).unwrap();
        assert_eq!(h.run(h.clipboard.read()).unwrap(), Some(item));
        assert!(
            h.app
                .0
                .clipboard
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(_, thread)| *thread != thread::current().id())
        );
    }

    #[test]
    fn permission_denial_and_unsupported_values_do_not_return_cached_success() {
        let h = Harness::new();
        h.run(h.clipboard.write(ClipboardItem::new_string("old".into())))
            .unwrap();
        h.run(h.clipboard.read()).unwrap();
        h.app.0.clipboard.denied.store(true, Ordering::SeqCst);
        assert!(matches!(
            h.run(h.clipboard.read()),
            Err(ClipboardReadError::Denied(_))
        ));
        assert!(h.clipboard.read_cached().is_none());
        let item = ClipboardItem {
            entries: vec![ClipboardEntry::String(ClipboardString {
                text: "text".into(),
                metadata: Some("".into()),
            })],
        };
        assert!(h.run(h.clipboard.write(item)).is_err());
        assert!(
            h.run(h.clipboard.write(ClipboardItem::new_string("a\0b".into())))
                .is_err()
        );
    }

    #[test]
    fn zero_and_decreased_revisions_invalidate_cached_snapshots() {
        let h = Harness::new();
        h.run(h.clipboard.write(ClipboardItem::new_string("first".into())))
            .unwrap();
        h.run(h.clipboard.read()).unwrap();
        assert_eq!(
            h.clipboard.read_cached().unwrap().text().as_deref(),
            Some("first")
        );
        *h.app.0.clipboard.records.lock().unwrap() = vec![ClipboardRecord {
            text: Some("after restart".into()),
            ..Default::default()
        }];
        h.app.0.clipboard.revision.store(1, Ordering::SeqCst);
        assert!(h.clipboard.read_cached().is_none());
        assert_eq!(
            h.run(h.clipboard.read())
                .unwrap()
                .unwrap()
                .text()
                .as_deref(),
            Some("after restart")
        );
        h.app.0.clipboard.revision.store(0, Ordering::SeqCst);
        assert!(h.clipboard.read_cached().is_none());
        assert_eq!(
            h.run(h.clipboard.read())
                .unwrap()
                .unwrap()
                .text()
                .as_deref(),
            Some("after restart")
        );
        assert!(h.clipboard.read_cached().is_none());
        *h.app.0.clipboard.records.lock().unwrap() = vec![ClipboardRecord {
            text: Some("after restart".into()),
            ..Default::default()
        }];
        h.app.0.clipboard.revision.store(1, Ordering::SeqCst);
        assert_eq!(
            h.run(h.clipboard.read())
                .unwrap()
                .unwrap()
                .text()
                .as_deref(),
            Some("after restart")
        );
    }

    #[test]
    fn canceled_write_waiter_does_not_release_native_serialization_early() {
        let h = Harness::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        *h.app.0.clipboard.write_gate.lock().unwrap() = Some((started_tx, resume_rx));
        let first = h.clipboard.write(ClipboardItem::new_string("first".into()));
        let deadline = Instant::now() + Duration::from_secs(3);
        while started_rx.try_recv().is_err() {
            assert!(Instant::now() < deadline);
            h.pump();
        }
        drop(first);
        let second = h
            .clipboard
            .write(ClipboardItem::new_string("second".into()));
        let read = h.clipboard.read();
        for _ in 0..10 {
            h.pump();
        }
        assert_eq!(h.app.0.clipboard.calls.lock().unwrap().len(), 1);
        resume_tx.send(()).unwrap();
        h.run(second).unwrap();
        assert_eq!(
            h.run(read).unwrap().unwrap().text().as_deref(),
            Some("second")
        );
        assert_eq!(
            h.app
                .0
                .clipboard
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(call, _)| *call)
                .collect::<Vec<_>>(),
            ["write", "write", "read"]
        );
    }

    #[test]
    fn canceled_read_waiter_does_not_release_native_serialization_early() {
        let h = Harness::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        *h.app.0.clipboard.read_gate.lock().unwrap() = Some((started_tx, resume_rx));
        let first = h.clipboard.read();
        let deadline = Instant::now() + Duration::from_secs(3);
        while started_rx.try_recv().is_err() {
            assert!(Instant::now() < deadline);
            h.pump();
        }
        drop(first);
        let second = h.clipboard.read();
        for _ in 0..10 {
            h.pump();
        }
        assert_eq!(h.app.0.clipboard.calls.lock().unwrap().len(), 1);
        resume_tx.send(()).unwrap();
        assert_eq!(h.run(second).unwrap(), None);
        assert_eq!(h.app.0.clipboard.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn write_and_focus_refresh_warm_synchronous_cache() {
        let h = Harness::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        *h.app.0.clipboard.write_gate.lock().unwrap() = Some((started_tx, resume_rx));
        let write = h.clipboard.write(ClipboardItem::new_string("in-app".into()));
        assert_eq!(
            h.clipboard.read_cached().unwrap().text().as_deref(),
            Some("in-app")
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while started_rx.try_recv().is_err() {
            assert!(Instant::now() < deadline);
            h.pump();
        }
        resume_tx.send(()).unwrap();
        h.run(write).unwrap();
        assert_eq!(
            h.clipboard.read_cached().unwrap().text().as_deref(),
            Some("in-app")
        );

        *h.app.0.clipboard.records.lock().unwrap() = vec![ClipboardRecord {
            text: Some("external".into()),
            ..Default::default()
        }];
        h.app.0.clipboard.revision.fetch_add(1, Ordering::SeqCst);
        h.clipboard.refresh_if_needed();
        let deadline = Instant::now() + Duration::from_secs(3);
        while h.clipboard.read_cached().is_none() {
            assert!(Instant::now() < deadline);
            h.pump();
        }
        assert_eq!(
            h.clipboard.read_cached().unwrap().text().as_deref(),
            Some("external")
        );
    }
}
