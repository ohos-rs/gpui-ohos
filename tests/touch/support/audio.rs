//! Bridge shim for the production audio service; acknowledgements are controlled by each test.
use crate::{Error, OpenHarmonyApp};
use futures::channel::oneshot;
use std::{
    any::TypeId,
    cell::{Cell, RefCell},
    collections::VecDeque,
    os::fd::{AsRawFd, BorrowedFd},
};

pub struct AudioBridgePlugin;

#[derive(Debug)]
pub struct AudioUpdateRequest {
    pub player: String,
    pub source_fd: Option<i32>,
    pub replace: bool,
    pub release: bool,
    pub play: bool,
    pub volume: f64,
    pub report_errors: bool,
    pub label: String,
    pub diagnostics: bool,
}

#[derive(Default)]
pub(crate) struct AudioState {
    registered: Cell<bool>,
    requests: RefCell<Vec<AudioUpdateRequest>>,
    acknowledgements: RefCell<VecDeque<oneshot::Sender<()>>>,
}

impl OpenHarmonyApp {
    pub fn registered_plugin<P: 'static>(&self) -> crate::Result<Option<()>> {
        assert_eq!(TypeId::of::<P>(), TypeId::of::<AudioBridgePlugin>());
        Ok(self.0.audio.registered.get().then_some(()))
    }
}

pub struct AudioClient(OpenHarmonyApp);

impl AudioClient {
    pub fn new(app: &OpenHarmonyApp) -> crate::Result<Self> {
        Ok(Self(app.clone()))
    }

    pub async fn update(
        &self,
        mut request: AudioUpdateRequest,
        source: Option<BorrowedFd<'_>>,
    ) -> crate::Result<()> {
        request.source_fd = source.map(|fd| fd.as_raw_fd());
        self.0.0.audio.requests.borrow_mut().push(request);
        let (ack, pending) = oneshot::channel();
        self.0.0.audio.acknowledgements.borrow_mut().push_back(ack);
        pending.await.map_err(|_| Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ForegroundExecutor, PriorityQueueReceiver, RunnableVariant,
        audio::{NativeAudioError, NativeAudioPlayerOptions, NativeAudioService},
        dispatcher::OhosDispatcher,
    };
    use std::{
        fs::File,
        io::{ErrorKind, Read},
        os::{fd::OwnedFd, unix::net::UnixStream},
        sync::Arc,
        time::Duration,
    };

    struct Driver {
        app: OpenHarmonyApp,
        queue: PriorityQueueReceiver<RunnableVariant>,
        dispatcher: Arc<OhosDispatcher>,
        executor: ForegroundExecutor,
    }

    impl Driver {
        fn new() -> Self {
            let (sender, queue) = PriorityQueueReceiver::new();
            let dispatcher = Arc::new(OhosDispatcher::new(sender));
            Self {
                app: OpenHarmonyApp::default(),
                executor: ForegroundExecutor::new(dispatcher.clone()),
                queue,
                dispatcher,
            }
        }

        fn service(&self) -> NativeAudioService {
            self.app.0.audio.registered.set(true);
            NativeAudioService::new(self.app.clone(), self.executor.clone()).unwrap()
        }

        fn flush(&self) {
            for _ in 0..100 {
                let Some(task) = self.queue.0.pop_timeout(Duration::ZERO) else {
                    return;
                };
                self.dispatcher.begin_main_turn();
                task.run();
            }
            panic!("audio queue did not settle");
        }

        fn acknowledge(&self) {
            self.app
                .0
                .audio
                .acknowledgements
                .borrow_mut()
                .pop_front()
                .unwrap()
                .send(())
                .unwrap();
            self.flush();
        }
    }

    impl Drop for Driver {
        fn drop(&mut self) {
            self.dispatcher.shutdown();
        }
    }

    fn owned_source() -> (File, UnixStream) {
        let (source, peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        (File::from(OwnedFd::from(source)), peer)
    }

    #[test]
    fn audio_requires_explicit_registration() {
        let driver = Driver::new();
        assert!(matches!(
            NativeAudioService::new(driver.app.clone(), driver.executor.clone()),
            Err(NativeAudioError::Unavailable)
        ));
        let service = driver.service();
        let first = service.player(NativeAudioPlayerOptions::default()).unwrap();
        let second = service.player(NativeAudioPlayerOptions::default()).unwrap();
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn audio_retains_owned_file_until_ack_and_orders_release_last() {
        let driver = Driver::new();
        let player = driver
            .service()
            .player(NativeAudioPlayerOptions { diagnostics: true })
            .unwrap();
        let (file, mut peer) = owned_source();
        player
            .replace_and_play(file, 0.5, true, "bell".into())
            .unwrap();
        player.replay(0.25).unwrap();
        player.clear().unwrap();
        drop(player);
        driver.flush();
        assert_eq!(driver.app.0.audio.requests.borrow().len(), 1);
        assert_eq!(
            peer.read(&mut [0]).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        {
            let requests = driver.app.0.audio.requests.borrow();
            let replace = &requests[0];
            assert!(replace.source_fd.is_some());
            assert!(
                replace.replace && replace.play && replace.report_errors && replace.diagnostics
            );
            assert_eq!(replace.label, "bell");
            assert_eq!(replace.volume, 0.5);
        }
        driver.acknowledge();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        {
            let requests = driver.app.0.audio.requests.borrow();
            assert_eq!(requests.len(), 2);
            let replay = &requests[1];
            assert!(replay.play && !replay.replace && !replay.release);
            assert_eq!(replay.volume, 0.25);
        }
        driver.acknowledge();
        {
            let requests = driver.app.0.audio.requests.borrow();
            assert_eq!(requests.len(), 3);
            assert!(requests[2].replace && !requests[2].play && !requests[2].release);
        }
        driver.acknowledge();
        {
            let requests = driver.app.0.audio.requests.borrow();
            assert_eq!(requests.len(), 4);
            assert!(requests[3].release && !requests[3].play && !requests[3].replace);
            assert!(
                requests
                    .iter()
                    .all(|request| request.player == requests[0].player)
            );
        }
        driver.acknowledge();
    }

    #[test]
    fn audio_rejected_ack_closes_file_and_continues_queue() {
        let driver = Driver::new();
        let player = driver
            .service()
            .player(NativeAudioPlayerOptions::default())
            .unwrap();
        let (file, mut peer) = owned_source();
        player
            .replace_and_play(file, 1.0, false, String::new())
            .unwrap();
        player.replay(1.0).unwrap();
        driver.flush();
        drop(
            driver
                .app
                .0
                .audio
                .acknowledgements
                .borrow_mut()
                .pop_front()
                .unwrap(),
        );
        driver.flush();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        assert_eq!(driver.app.0.audio.requests.borrow().len(), 2);
        driver.acknowledge();
        drop(player);
        driver.flush();
        driver.acknowledge();
    }

    #[test]
    fn audio_invalid_volume_never_reaches_bridge() {
        let driver = Driver::new();
        let player = driver
            .service()
            .player(NativeAudioPlayerOptions::default())
            .unwrap();
        for volume in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert_eq!(player.replay(volume), Err(NativeAudioError::InvalidVolume));
        }
        driver.flush();
        assert!(driver.app.0.audio.requests.borrow().is_empty());
        drop(player);
        driver.flush();
        driver.acknowledge();
    }
}
