use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use crate::{
    ClipboardEntry, ClipboardItem, ClipboardReadError, ClipboardString, ExternalPaths,
    ForegroundExecutor, Image, ImageFormat, Task,
};
use anyhow::Result;
use futures::{
    FutureExt,
    channel::oneshot,
    future::{LocalBoxFuture, Shared},
};
use openharmony_ability::OpenHarmonyApp;
use openharmony_ability_plugin_clipboard::{
    ClipboardBridgePlugin, ClipboardClient, ClipboardRecord,
};

#[derive(Clone)]
pub(crate) struct OhosClipboard {
    app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    foreground_executor: ForegroundExecutor,
    cache: Rc<RefCell<Option<(u64, ClipboardItem)>>>,
    operation: Rc<Cell<u64>>,
    refresh_pending: Rc<Cell<bool>>,
    write_tail: Rc<RefCell<Option<Shared<LocalBoxFuture<'static, ()>>>>>,
}

impl OhosClipboard {
    pub(crate) fn new(
        app: Rc<RefCell<Option<OpenHarmonyApp>>>,
        foreground_executor: ForegroundExecutor,
    ) -> Self {
        Self {
            app,
            foreground_executor,
            cache: Rc::new(RefCell::new(None)),
            operation: Rc::new(Cell::new(0)),
            refresh_pending: Rc::new(Cell::new(false)),
            write_tail: Rc::new(RefCell::new(None)),
        }
    }

    fn revision(app: &OpenHarmonyApp) -> Option<u64> {
        app.registered_plugin::<ClipboardBridgePlugin>()
            .ok()
            .flatten()
            .map(|plugin| plugin.revision())
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
        // A synchronous GPUI read must never expose the old contents after a native update.
        // The SDK read is asynchronous and permission-gated; callers can await read().
        if self.app.borrow().is_some() && !self.refresh_pending.replace(true) {
            let clipboard = self.clone();
            self.foreground_executor
                .spawn(async move {
                    let _ = clipboard.read().await;
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
        let clipboard = self.clone();
        let operation = self.next_operation();
        let pending_write = self.write_tail.borrow().clone();
        self.foreground_executor.spawn(async move {
            if let Some(pending) = pending_write {
                pending.await;
            }
            let revision = Self::revision(&app);
            let client = ClipboardClient::new(&app)
                .map_err(|error| ClipboardReadError::Denied(error.to_string()))?;
            let records = client
                .read_records()
                .await
                .map_err(|error| ClipboardReadError::Denied(error.to_string()))?;
            let mut entries = Vec::with_capacity(records.len());
            for record in records {
                if let Some(text) = record.text {
                    entries.push(ClipboardEntry::String(ClipboardString {
                        text,
                        metadata: record.metadata,
                    }));
                } else if let Some(bytes) = record.encoded_image {
                    entries.push(ClipboardEntry::Image(Image::from_bytes(
                        ImageFormat::Png,
                        bytes,
                    )));
                } else if let Some(uri) = record.uri {
                    let path = ohos_fileuri_binding::get_path_from_uri(&uri)
                        .map_err(|_| ClipboardReadError::UnsupportedContent)?;
                    entries.push(ClipboardEntry::ExternalPaths(ExternalPaths(
                        smallvec::smallvec![PathBuf::from(path)],
                    )));
                }
            }
            let item = (!entries.is_empty()).then_some(ClipboardItem { entries });
            if clipboard.operation.get() == operation && revision == Self::revision(&app) {
                *clipboard.cache.borrow_mut() = revision.zip(item.clone());
            }
            Ok(item)
        })
    }

    pub(crate) fn write(&self, item: ClipboardItem) -> Task<Result<()>> {
        let Some(app) = self.app.borrow().clone() else {
            return Task::ready(Err(anyhow::anyhow!("Clipboard is unavailable")));
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
        self.foreground_executor.spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = async {
                let client = ClipboardClient::new(&app)?;
                if records.is_empty() {
                    client.clear().await?;
                } else {
                    client.write_records(records).await?;
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if result.is_ok() && clipboard.operation.get() == operation {
                *clipboard.cache.borrow_mut() = Self::revision(&app)
                    .filter(|_| !item.entries.is_empty())
                    .map(|revision| (revision, item));
            }
            let _ = complete.send(());
            result
        })
    }
}
