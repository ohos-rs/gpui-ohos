use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use crate::{
    BackgroundExecutor, ClipboardEntry, ClipboardItem, ClipboardReadError, ClipboardString,
    ExternalPaths, ForegroundExecutor, Image, ImageFormat, Task,
};
use anyhow::Result;
use futures::{
    FutureExt,
    channel::oneshot,
    future::{LocalBoxFuture, Shared},
    lock::Mutex,
};
use openharmony_ability::OpenHarmonyApp;
use openharmony_ability_plugin_clipboard::{
    ClipboardBridgePlugin, ClipboardClient, ClipboardError, ClipboardRecord,
};

const METADATA_MIME: &str = "application/x-gpui-ohos-text-metadata-v1";

#[derive(Clone)]
pub(crate) struct OhosClipboard {
    app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    foreground_executor: ForegroundExecutor,
    background_executor: BackgroundExecutor,
    cache: Rc<RefCell<Option<(u32, ClipboardItem)>>>,
    operation: Rc<Cell<u64>>,
    refresh_pending: Rc<Cell<bool>>,
    write_tail: Rc<RefCell<Option<Shared<LocalBoxFuture<'static, ()>>>>>,
    read_lock: Rc<Mutex<()>>,
}

impl OhosClipboard {
    pub(crate) fn new(
        app: Rc<RefCell<Option<OpenHarmonyApp>>>,
        foreground_executor: ForegroundExecutor,
        background_executor: BackgroundExecutor,
    ) -> Self {
        Self {
            app,
            foreground_executor,
            background_executor,
            cache: Rc::new(RefCell::new(None)),
            operation: Rc::new(Cell::new(0)),
            refresh_pending: Rc::new(Cell::new(false)),
            write_tail: Rc::new(RefCell::new(None)),
            read_lock: Rc::new(Mutex::new(())),
        }
    }

    fn revision(app: &OpenHarmonyApp) -> Option<u32> {
        match app.registered_plugin::<ClipboardBridgePlugin>() {
            Ok(Some(plugin)) => query_revision(&plugin),
            Ok(None) => {
                log::warn!("OHOS clipboard cache unavailable: plugin is not registered");
                None
            }
            Err(error) => {
                log::warn!("Failed to query OHOS clipboard plugin: {error}");
                None
            }
        }
    }

    fn next_operation(&self) -> u64 {
        let next = self.operation.get().wrapping_add(1);
        self.operation.set(next);
        next
    }

    pub(crate) fn read_cached(&self) -> Option<ClipboardItem> {
        let revision = self.app.borrow().as_ref().and_then(Self::revision);
        if let Some((cached_revision, item)) = self.cache.borrow().as_ref()
            && Some(*cached_revision) == revision
        {
            return Some(item.clone());
        }
        *self.cache.borrow_mut() = None;
        if self.app.borrow().is_some() && !self.refresh_pending.replace(true) {
            let clipboard = self.clone();
            self.foreground_executor
                .spawn(async move {
                    if let Err(error) = clipboard.read().await {
                        log::warn!("OHOS clipboard cache refresh failed: {error}");
                    }
                    clipboard.refresh_pending.set(false);
                })
                .detach();
        }
        None
    }

    pub(crate) fn read(
        &self,
    ) -> Task<std::result::Result<Option<ClipboardItem>, ClipboardReadError>> {
        let Some(app) = self.app.borrow().clone() else {
            return Task::ready(Err(ClipboardReadError::Unavailable));
        };
        let client = match ClipboardClient::new(&app)
            .and_then(|client| client.with_metadata_mime_type(METADATA_MIME))
        {
            Ok(client) => client,
            Err(error) => return Task::ready(Err(read_error(error))),
        };
        let plugin = match app.registered_plugin::<ClipboardBridgePlugin>() {
            Ok(plugin) => plugin,
            Err(error) => {
                log::warn!("Failed to query OHOS clipboard plugin: {error}");
                return Task::ready(Err(ClipboardReadError::Unavailable));
            }
        };
        let pending_write = self.write_tail.borrow().clone();
        let clipboard = self.clone();
        let operation = self.next_operation();
        let (sender, receiver) = oneshot::channel();
        self.foreground_executor
            .spawn(async move {
                if let Some(pending) = pending_write {
                    pending.await;
                }
                let _read_guard = clipboard.read_lock.lock().await;
                if sender.is_canceled() {
                    return;
                }
                // The native facade performs synchronous IPC when polled. Keep
                // the detached guard alive through completion, even if canceled.
                let result = clipboard
                    .background_executor
                    .spawn(async move {
                        let before = plugin.as_deref().and_then(query_revision);
                        let records = client.read_records().await.map_err(read_error)?;
                        let after = plugin.as_deref().and_then(query_revision);
                        Ok((records, before.filter(|before| Some(*before) == after)))
                    })
                    .await;
                let result = result.and_then(|(records, revision)| {
                    let mut entries = Vec::with_capacity(records.len());
                    for record in records {
                        if let Some(text) = record.text {
                            entries.push(ClipboardEntry::String(ClipboardString {
                                text,
                                metadata: record.metadata,
                            }));
                        } else if let Some(bytes) = record.encoded_image {
                            let format = image::guess_format(&bytes)
                                .ok()
                                .and_then(|format| {
                                    ImageFormat::from_mime_type(format.to_mime_type())
                                })
                                .ok_or_else(|| {
                                    log::warn!("Unsupported encoded OHOS clipboard image format");
                                    ClipboardReadError::UnsupportedContent
                                })?;
                            entries.push(ClipboardEntry::Image(Image::from_bytes(format, bytes)));
                        } else if let Some(uri) = record.uri {
                            let path =
                                ohos_fileuri_binding::get_path_from_uri(&uri).map_err(|error| {
                                    log::warn!("Unsupported OHOS clipboard URI {uri}: {error}");
                                    ClipboardReadError::UnsupportedContent
                                })?;
                            entries.push(ClipboardEntry::ExternalPaths(ExternalPaths(
                                smallvec::smallvec![PathBuf::from(path)],
                            )));
                        }
                    }
                    let item = (!entries.is_empty()).then_some(ClipboardItem { entries });
                    if clipboard.operation.get() == operation {
                        *clipboard.cache.borrow_mut() = revision.zip(item.clone());
                    }
                    Ok(item)
                });
                if result.is_err() && clipboard.operation.get() == operation {
                    *clipboard.cache.borrow_mut() = None;
                }
                if let Err(Err(error)) = sender.send(result) {
                    log::warn!("Abandoned OHOS clipboard read failed: {error}");
                }
            })
            .detach();
        self.foreground_executor.spawn(async move {
            receiver
                .await
                .unwrap_or(Err(ClipboardReadError::Unavailable))
        })
    }

    pub(crate) fn write(&self, item: ClipboardItem) -> Task<Result<()>> {
        let Some(app) = self.app.borrow().clone() else {
            return Task::ready(Err(anyhow::anyhow!("Clipboard is unavailable")));
        };
        let client = match ClipboardClient::new(&app)
            .and_then(|client| client.with_metadata_mime_type(METADATA_MIME))
        {
            Ok(client) => client,
            Err(error) => return Task::ready(Err(error.into())),
        };
        let mut records = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::String(text) => records.push(ClipboardRecord {
                    text: Some(text.text.clone()),
                    metadata: text.metadata.clone(),
                    ..Default::default()
                }),
                ClipboardEntry::Image(image) => records.push(ClipboardRecord {
                    encoded_image: Some(image.bytes.clone()),
                    ..Default::default()
                }),
                ClipboardEntry::ExternalPaths(paths) => {
                    for path in paths.paths() {
                        let Some(path) = path.to_str() else {
                            return Task::ready(Err(anyhow::anyhow!(
                                "Clipboard path is not UTF-8"
                            )));
                        };
                        let uri = match ohos_fileuri_binding::get_uri_from_path(path) {
                            Ok(uri) => uri,
                            Err(error) => return Task::ready(Err(error.into())),
                        };
                        records.push(ClipboardRecord {
                            uri: Some(uri),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        let clipboard = self.clone();
        let operation = self.next_operation();
        let previous = self.write_tail.borrow().clone();
        let (complete, completion) = oneshot::channel();
        *self.write_tail.borrow_mut() = Some(
            completion
                .map(|_: Result<(), oneshot::Canceled>| ())
                .boxed_local()
                .shared(),
        );
        let (sender, receiver) = oneshot::channel();
        self.foreground_executor
            .spawn(async move {
                if let Some(previous) = previous {
                    previous.await;
                }
                let result = clipboard
                    .background_executor
                    .spawn(async move {
                        if records.is_empty() {
                            client.clear().await?;
                        } else {
                            client.write_records(records).await?;
                        }
                        Ok::<_, ClipboardError>(())
                    })
                    .await;
                let result = result
                    .map(|()| {
                        if clipboard.operation.get() == operation {
                            // A different application can write between SetData and a
                            // counter query; only native read snapshots seed the cache.
                            *clipboard.cache.borrow_mut() = None;
                        }
                    })
                    .map_err(anyhow::Error::from);
                let _ = complete.send(());
                if let Err(Err(error)) = sender.send(result) {
                    log::warn!("Abandoned OHOS clipboard write failed: {error}");
                }
            })
            .detach();
        self.foreground_executor
            .spawn(async move { receiver.await.map_err(anyhow::Error::from)? })
    }
}

fn query_revision(plugin: &ClipboardBridgePlugin) -> Option<u32> {
    match plugin.revision() {
        Ok(revision) => Some(revision),
        Err(error) => {
            log::warn!("OHOS clipboard cache disabled for this query: {error}");
            None
        }
    }
}

fn read_error(error: ClipboardError) -> ClipboardReadError {
    log::warn!("Failed to read OHOS clipboard: {error}");
    match error {
        ClipboardError::AuthorizationRequired => ClipboardReadError::Denied(error.to_string()),
        ClipboardError::InvalidContent(_)
        | ClipboardError::InvalidText { .. }
        | ClipboardError::EmptyBinaryUnsupported { .. } => ClipboardReadError::UnsupportedContent,
        ClipboardError::ApiUnavailable { .. }
        | ClipboardError::RevisionUnavailable
        | ClipboardError::Native(_) => ClipboardReadError::Unavailable,
    }
}
