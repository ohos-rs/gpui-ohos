use anyhow::{Context as _, Result};
use ohos_fileshare_binding::PolicyInfo;
use parking_lot::Mutex;
use serde::de::{MapAccess, Visitor};
use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use super::presentation::Presentation;

const READ: u32 = 1;
const READ_WRITE: u32 = 3;
const MAX_GRANTS: usize = 256;
const BATCH_SIZE: usize = 64;
const MAX_URI_BYTES: usize = 4096;
const MAX_HISTORY_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) struct FileAccess {
    enabled: bool,
    path: Option<PathBuf>,
    presented: AtomicBool,
    state: Mutex<FileAccessState>,
}

#[derive(Default)]
struct FileAccessState {
    policies: Option<BTreeMap<String, u32>>,
    pending: BTreeMap<String, u32>,
    restored: bool,
    dirty: bool,
}

struct PolicyFailure {
    uri: String,
    message: String,
}

trait FileShare {
    fn persist(&self, policies: &[PolicyInfo]) -> Result<Vec<PolicyFailure>>;
    fn check(&self, policies: &[PolicyInfo]) -> Result<Vec<bool>>;
    fn activate(&self, policies: &[PolicyInfo]) -> Result<Vec<PolicyFailure>>;
}

struct NativeFileShare;

impl FileShare for NativeFileShare {
    fn persist(&self, policies: &[PolicyInfo]) -> Result<Vec<PolicyFailure>> {
        Ok(ohos_fileshare_binding::persist_permission(policies)?
            .into_iter()
            .map(|error| PolicyFailure {
                message: format!("{error:?}"),
                uri: error.uri,
            })
            .collect())
    }

    fn check(&self, policies: &[PolicyInfo]) -> Result<Vec<bool>> {
        Ok(ohos_fileshare_binding::check_persistent_permission(
            policies,
        )?)
    }

    fn activate(&self, policies: &[PolicyInfo]) -> Result<Vec<PolicyFailure>> {
        Ok(ohos_fileshare_binding::activate_permission(policies)?
            .into_iter()
            .map(|error| PolicyFailure {
                message: format!("{error:?}"),
                uri: error.uri,
            })
            .collect())
    }
}

impl FileAccess {
    pub(crate) fn new(base_path: Option<String>, enabled: bool) -> Self {
        Self {
            enabled,
            path: base_path
                .filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join("gpui-ohos-file-access.json")),
            presented: AtomicBool::new(false),
            state: Mutex::new(FileAccessState::default()),
        }
    }

    /// Only the first successful presentation schedules work; never lock on the render thread.
    pub(crate) fn on_presentation(&self, presentation: Presentation) -> bool {
        self.enabled
            && presentation == Presentation::Presented
            && !self.presented.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn restore(&self) -> Result<()> {
        self.restore_with(&NativeFileShare)
    }

    pub(crate) fn remember(&self, uris: &[String], writable: bool) -> Result<()> {
        self.remember_with(uris, writable, &NativeFileShare)
    }

    fn restore_with(&self, backend: &impl FileShare) -> Result<()> {
        if !self.enabled || !self.presented.load(Ordering::Acquire) {
            return Ok(());
        }
        self.process(&mut self.state.lock(), backend, Vec::new())
    }

    fn remember_with(
        &self,
        uris: &[String],
        writable: bool,
        backend: &impl FileShare,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut state = self.state.lock();
        let mut failures = Vec::new();
        for uri in uris {
            if !valid_uri(uri) {
                failures.push(format!("Invalid or oversized file grant URI: {uri}"));
                continue;
            }
            if !state.pending.contains_key(uri) && state.pending.len() == MAX_GRANTS {
                failures.push(format!(
                    "File grant queue is full ({MAX_GRANTS}); remaining selections are not retained"
                ));
                break;
            }
            let mode = if writable { READ_WRITE } else { READ };
            state
                .pending
                .entry(uri.clone())
                .and_modify(|previous| *previous |= mode)
                .or_insert(mode);
        }
        if self.presented.load(Ordering::Acquire) {
            self.process(&mut state, backend, failures)
        } else {
            report(failures)
        }
    }

    fn process(
        &self,
        state: &mut FileAccessState,
        backend: &impl FileShare,
        mut failures: Vec<String>,
    ) -> Result<()> {
        // A read failure preserves both the file and the bounded pending selection.
        if state.policies.is_none() {
            state.policies = Some(self.read_history()?);
        }
        self.restore_state(state, backend, &mut failures);
        let pending = std::mem::take(&mut state.pending);
        let policies = state.policies.as_mut().expect("history loaded above");
        let mut requests = Vec::new();
        let mut count = policies.len();
        for (uri, operation_mode) in pending {
            if !policies.contains_key(&uri) {
                if count == MAX_GRANTS {
                    failures.push(format!("File grant history is full ({MAX_GRANTS}): {uri}"));
                    continue;
                }
                count += 1;
            }
            requests.push(PolicyInfo {
                uri,
                operation_mode,
            });
        }
        let mut retained = apply_batches("persist", &requests, &mut failures, |batch| {
            backend.persist(batch)
        });
        // Save dialogs may return providers that grant only reading.
        let read_fallbacks = requests
            .iter()
            .filter(|policy| {
                policy.operation_mode == READ_WRITE
                    && !retained.iter().any(|saved| saved.uri == policy.uri)
            })
            .map(|policy| PolicyInfo {
                uri: policy.uri.clone(),
                operation_mode: READ,
            })
            .collect::<Vec<_>>();
        retained.extend(apply_batches(
            "persist read-only",
            &read_fallbacks,
            &mut failures,
            |batch| backend.persist(batch),
        ));
        for policy in &retained {
            let mode = policies.get(&policy.uri).copied().unwrap_or(0) | policy.operation_mode;
            let previous = policies.insert(policy.uri.clone(), mode);
            state.dirty |= previous != Some(mode);
        }
        let activated = apply_batches("activate", &retained, &mut failures, |batch| {
            backend.activate(batch)
        });
        state.restored &= activated.len() == retained.len();
        if let Err(error) = self.save_state(state) {
            failures.push(format!("Cannot save file access history: {error:#}"));
        }
        report(failures)
    }

    fn restore_state(
        &self,
        state: &mut FileAccessState,
        backend: &impl FileShare,
        failures: &mut Vec<String>,
    ) {
        if state.restored {
            return;
        }
        let policies = state.policies.as_mut().expect("history loaded above");
        let mut to_check = policies
            .iter()
            .map(|(uri, mode)| PolicyInfo {
                uri: uri.clone(),
                operation_mode: *mode,
            })
            .collect::<Vec<_>>();
        let initial_failures = failures.len();
        let mut active = Vec::new();
        // The second pass checks READ only for revoked writable grants.
        for _ in 0..2 {
            let mut fallback = Vec::new();
            for batch in to_check.chunks(BATCH_SIZE) {
                match backend.check(batch) {
                    Ok(results) if results.len() == batch.len() => {
                        for (policy, granted) in batch.iter().zip(results) {
                            if granted {
                                state.dirty |= policies
                                    .insert(policy.uri.clone(), policy.operation_mode)
                                    != Some(policy.operation_mode);
                                active.push(policy.clone());
                            } else if policy.operation_mode == READ_WRITE {
                                fallback.push(PolicyInfo {
                                    uri: policy.uri.clone(),
                                    operation_mode: READ,
                                });
                            } else {
                                state.dirty |= policies.remove(&policy.uri).is_some();
                            }
                        }
                    }
                    Ok(results) => failures.push(format!(
                        "File permission query returned {} results for {} policies",
                        results.len(),
                        batch.len()
                    )),
                    Err(error) => failures.push(format!("Cannot check file grants: {error:#}")),
                }
            }
            to_check = fallback;
        }
        apply_batches("restore", &active, failures, |batch| {
            backend.activate(batch)
        });
        state.restored = failures.len() == initial_failures;
    }

    fn read_history(&self) -> Result<BTreeMap<String, u32>> {
        let path = self
            .path
            .as_ref()
            .context("No private file access directory")?;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(error).context("Read file access history"),
        };
        let mut contents = Vec::new();
        file.take(MAX_HISTORY_BYTES + 1)
            .read_to_end(&mut contents)?;
        anyhow::ensure!(
            contents.len() as u64 <= MAX_HISTORY_BYTES,
            "File access history exceeds {MAX_HISTORY_BYTES} bytes"
        );
        struct History;
        impl<'de> Visitor<'de> for History {
            type Value = BTreeMap<String, u32>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "at most {MAX_GRANTS} file grants")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut policies = BTreeMap::new();
                let mut count = 0;
                while let Some((uri, mode)) = map.next_entry::<String, u32>()? {
                    count += 1;
                    if count > MAX_GRANTS || !valid_uri(&uri) || !matches!(mode, READ | READ_WRITE)
                    {
                        return Err(serde::de::Error::custom("Invalid or excessive file grants"));
                    }
                    policies.insert(uri, mode);
                }
                Ok(policies)
            }
        }
        let mut deserializer = serde_json::Deserializer::from_slice(&contents);
        let policies = serde::de::Deserializer::deserialize_map(&mut deserializer, History)
            .with_context(|| format!("Read file access history {}", path.display()))?;
        deserializer.end()?;
        Ok(policies)
    }

    fn save_state(&self, state: &mut FileAccessState) -> Result<()> {
        if !state.dirty {
            return Ok(());
        }
        let policies = state
            .policies
            .as_ref()
            .context("File history unavailable")?;
        anyhow::ensure!(policies.len() <= MAX_GRANTS, "File grant history is full");
        let contents = serde_json::to_vec(policies)?;
        anyhow::ensure!(
            contents.len() as u64 <= MAX_HISTORY_BYTES,
            "File access history exceeds {MAX_HISTORY_BYTES} bytes"
        );
        let path = self
            .path
            .as_ref()
            .context("No private file access directory")?;
        std::fs::create_dir_all(path.parent().context("File history has no parent")?)?;
        let temporary_path = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary_path)
            .context("Write file access history")?;
        file.write_all(&contents)?;
        file.sync_all()?;
        std::fs::rename(temporary_path, path).context("Replace file access history")?;
        state.dirty = false;
        Ok(())
    }
}

fn valid_uri(uri: &str) -> bool {
    uri.starts_with("file://") && uri.len() <= MAX_URI_BYTES && !uri.contains('\0')
}

fn report(failures: Vec<String>) -> Result<()> {
    anyhow::ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

fn apply_batches(
    operation: &str,
    policies: &[PolicyInfo],
    failures: &mut Vec<String>,
    call: impl Fn(&[PolicyInfo]) -> Result<Vec<PolicyFailure>>,
) -> Vec<PolicyInfo> {
    let mut successful = Vec::new();
    for batch in policies.chunks(BATCH_SIZE) {
        match call(batch) {
            Ok(errors) => {
                if errors
                    .iter()
                    .any(|error| !batch.iter().any(|policy| policy.uri == error.uri))
                {
                    failures.push(format!(
                        "Invalid file permission {operation} failure response"
                    ));
                    continue;
                }
                successful.extend(
                    batch
                        .iter()
                        .filter(|policy| !errors.iter().any(|error| error.uri == policy.uri))
                        .cloned(),
                );
                for error in errors {
                    failures.push(format!(
                        "Cannot {operation} {}: {}",
                        error.uri, error.message
                    ));
                }
            }
            Err(error) => failures.push(format!("Cannot {operation} file grants: {error:#}")),
        }
    }
    successful
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::Arc,
    };

    #[derive(Default)]
    struct Backend {
        persistent: RefCell<BTreeMap<String, u32>>,
        calls: RefCell<Vec<(&'static str, Vec<PolicyInfo>)>>,
        reject_uri: RefCell<Option<String>>,
        read_only: Cell<bool>,
        fail_check: Cell<bool>,
        short_check: Cell<bool>,
        fail_activate: Cell<bool>,
        invalid_failure: Cell<bool>,
        fail_first_persist: Cell<bool>,
    }

    impl Backend {
        fn record(&self, name: &'static str, policies: &[PolicyInfo]) {
            assert!(!policies.is_empty());
            assert!(policies.len() <= BATCH_SIZE);
            self.calls.borrow_mut().push((name, policies.to_vec()));
        }

        fn sizes(&self, name: &str) -> Vec<usize> {
            self.calls
                .borrow()
                .iter()
                .filter(|(operation, _)| *operation == name)
                .map(|(_, policies)| policies.len())
                .collect()
        }
    }

    impl FileShare for Backend {
        fn persist(&self, policies: &[PolicyInfo]) -> Result<Vec<PolicyFailure>> {
            self.record("persist", policies);
            anyhow::ensure!(
                !self.fail_first_persist.replace(false),
                "service unavailable"
            );
            if self.invalid_failure.get() {
                return Ok(vec![PolicyFailure {
                    uri: "file://not-requested".into(),
                    message: "invalid service response".into(),
                }]);
            }
            let mut failed = Vec::new();
            for policy in policies {
                if self.reject_uri.borrow().as_ref() == Some(&policy.uri)
                    || (self.read_only.get() && policy.operation_mode == READ_WRITE)
                {
                    failed.push(PolicyFailure {
                        uri: policy.uri.clone(),
                        message: "provider denied requested mode".into(),
                    });
                } else {
                    self.persistent
                        .borrow_mut()
                        .entry(policy.uri.clone())
                        .and_modify(|mode| *mode |= policy.operation_mode)
                        .or_insert(policy.operation_mode);
                }
            }
            Ok(failed)
        }

        fn check(&self, policies: &[PolicyInfo]) -> Result<Vec<bool>> {
            self.record("check", policies);
            anyhow::ensure!(!self.fail_check.get(), "service unavailable");
            if self.short_check.get() {
                return Ok(Vec::new());
            }
            Ok(policies
                .iter()
                .map(|policy| {
                    self.persistent
                        .borrow()
                        .get(&policy.uri)
                        .copied()
                        .unwrap_or(0)
                        & policy.operation_mode
                        == policy.operation_mode
                })
                .collect())
        }

        fn activate(&self, policies: &[PolicyInfo]) -> Result<Vec<PolicyFailure>> {
            self.record("activate", policies);
            Ok(if self.fail_activate.get() {
                policies
                    .iter()
                    .map(|policy| PolicyFailure {
                        uri: policy.uri.clone(),
                        message: "activation unavailable".into(),
                    })
                    .collect()
            } else {
                Vec::new()
            })
        }
    }

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("gpui-file-access-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn access(&self, enabled: bool) -> FileAccess {
            FileAccess::new(Some(self.0.to_str().unwrap().into()), enabled)
        }

        fn history(&self) -> PathBuf {
            self.0.join("gpui-ohos-file-access.json")
        }

        fn write(&self, grants: &BTreeMap<String, u32>) {
            std::fs::write(self.history(), serde_json::to_vec(grants).unwrap()).unwrap();
        }

        fn read(&self) -> BTreeMap<String, u32> {
            serde_json::from_slice(&std::fs::read(self.history()).unwrap()).unwrap()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn uris(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("file://docs/selection-{index:04}"))
            .collect()
    }

    fn grants(count: usize, mode: u32) -> BTreeMap<String, u32> {
        uris(count).into_iter().map(|uri| (uri, mode)).collect()
    }

    #[test]
    fn default_options_never_read_write_or_request_persistence() {
        let directory = Directory::new();
        directory.write(&grants(1, READ));
        let original = std::fs::read(directory.history()).unwrap();
        let access = directory
            .access(crate::platform_options::PlatformOptions::default().persist_file_access);
        assert!(!access.on_presentation(Presentation::Presented));
        access.restore().unwrap();
        access.remember(&uris(1), true).unwrap();
        assert_eq!(std::fs::read(directory.history()).unwrap(), original);
        assert!(access.state.lock().policies.is_none());
        assert!(access.state.lock().pending.is_empty());
        assert!(crate::platform_options::PlatformOptions::default().native_menus);
    }

    #[test]
    fn nothing_runs_before_first_successfully_presented_frame() {
        let directory = Directory::new();
        directory.write(&grants(1, READ));
        let backend = Backend::default();
        backend.persistent.replace(grants(1, READ));
        let access = directory.access(true);
        access
            .remember_with(&uris(2)[1..], false, &backend)
            .unwrap();
        for outcome in [Presentation::Retry, Presentation::Unavailable] {
            assert!(!access.on_presentation(outcome));
            access.restore_with(&backend).unwrap();
        }
        assert!(backend.calls.borrow().is_empty());
        assert!(access.state.lock().policies.is_none());
        assert!(access.on_presentation(Presentation::Presented));
        access.restore_with(&backend).unwrap();
        assert_eq!(backend.sizes("check"), [1]);
        assert_eq!(backend.sizes("persist"), [1]);
        assert_eq!(directory.read().len(), 2);
        assert!(!access.on_presentation(Presentation::Presented));
        backend.calls.borrow_mut().clear();
        access.restore_with(&backend).unwrap();
        assert!(backend.calls.borrow().is_empty());
    }

    #[test]
    fn first_frame_schedules_once_across_windows_and_threads() {
        let access = Arc::new(FileAccess::new(None, true));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let access = access.clone();
            workers.push(std::thread::spawn(move || {
                access.on_presentation(Presentation::Presented)
            }));
        }
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|thread| thread.join().unwrap().then_some(()))
                .count(),
            1
        );
    }

    #[test]
    fn persist_check_and_activate_use_bounded_batches() {
        let directory = Directory::new();
        let backend = Backend::default();
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        access.remember_with(&uris(135), false, &backend).unwrap();
        assert_eq!(backend.sizes("persist"), [64, 64, 7]);
        assert_eq!(backend.sizes("activate"), [64, 64, 7]);
        assert!(
            backend
                .calls
                .borrow()
                .iter()
                .flat_map(|(_, policies)| policies)
                .all(|policy| policy.operation_mode == READ)
        );
        backend.calls.borrow_mut().clear();
        let restarted = directory.access(true);
        restarted.on_presentation(Presentation::Presented);
        restarted.restore_with(&backend).unwrap();
        assert_eq!(backend.sizes("check"), [64, 64, 7]);
        assert_eq!(backend.sizes("activate"), [64, 64, 7]);
        assert!(backend.sizes("persist").is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(directory.history())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn cap_bounds_pre_frame_queue_and_stored_grants() {
        let directory = Directory::new();
        let backend = Backend::default();
        let access = directory.access(true);
        assert!(
            access
                .remember_with(&uris(MAX_GRANTS + 20), false, &backend)
                .is_err()
        );
        assert_eq!(access.state.lock().pending.len(), MAX_GRANTS);
        assert!(backend.calls.borrow().is_empty());
        access.on_presentation(Presentation::Presented);
        access.restore_with(&backend).unwrap();
        assert_eq!(directory.read().len(), MAX_GRANTS);
        assert_eq!(backend.sizes("persist"), [64, 64, 64, 64]);
        assert!(
            access
                .remember_with(&["file://docs/overflow".into()], false, &backend)
                .is_err()
        );
        assert_eq!(directory.read().len(), MAX_GRANTS);
    }

    #[test]
    fn full_history_allows_existing_entries_without_evicting_successes() {
        let directory = Directory::new();
        let backend = Backend::default();
        backend.persistent.replace(grants(MAX_GRANTS, READ));
        directory.write(&grants(MAX_GRANTS, READ));
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        access.remember_with(&uris(3), true, &backend).unwrap();
        assert_eq!(directory.read().len(), MAX_GRANTS);
        assert!(
            uris(3)
                .iter()
                .all(|uri| directory.read()[uri] == READ_WRITE)
        );
    }

    #[test]
    fn oversized_history_count_is_rejected_without_ipc_or_overwriting() {
        let directory = Directory::new();
        directory.write(&grants(MAX_GRANTS + 1, READ));
        let original = std::fs::read(directory.history()).unwrap();
        let access = directory.access(true);
        let backend = Backend::default();
        access.on_presentation(Presentation::Presented);
        assert!(access.restore_with(&backend).is_err());
        assert!(access.state.lock().policies.is_none());
        assert!(backend.calls.borrow().is_empty());
        assert_eq!(std::fs::read(directory.history()).unwrap(), original);
    }

    #[test]
    fn oversized_history_bytes_are_rejected_without_ipc() {
        let directory = Directory::new();
        std::fs::write(
            directory.history(),
            vec![b' '; MAX_HISTORY_BYTES as usize + 1],
        )
        .unwrap();
        let access = directory.access(true);
        let backend = Backend::default();
        access.on_presentation(Presentation::Presented);
        assert!(
            access
                .restore_with(&backend)
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
        assert!(backend.calls.borrow().is_empty());
    }

    #[test]
    fn malformed_history_preserves_file_and_pending_selection_for_retry() {
        let directory = Directory::new();
        std::fs::write(directory.history(), "truncated{").unwrap();
        let access = directory.access(true);
        let backend = Backend::default();
        access.on_presentation(Presentation::Presented);
        assert!(access.remember_with(&uris(1), false, &backend).is_err());
        assert_eq!(
            std::fs::read_to_string(directory.history()).unwrap(),
            "truncated{"
        );
        assert_eq!(access.state.lock().pending.len(), 1);
        assert!(backend.calls.borrow().is_empty());
        std::fs::remove_file(directory.history()).unwrap();
        access.restore_with(&backend).unwrap();
        assert_eq!(directory.read(), grants(1, READ));
    }

    #[test]
    fn invalid_history_modes_uris_and_trailing_data_are_reported() {
        for contents in [
            r#"{"file://docs/one":2}"#,
            r#"{"https://docs/one":1}"#,
            r#"{"file://docs/one":1} {}"#,
        ] {
            let directory = Directory::new();
            std::fs::write(directory.history(), contents).unwrap();
            let access = directory.access(true);
            let backend = Backend::default();
            access.on_presentation(Presentation::Presented);
            assert!(access.restore_with(&backend).is_err());
            assert!(backend.calls.borrow().is_empty());
            assert_eq!(
                std::fs::read_to_string(directory.history()).unwrap(),
                contents
            );
        }
    }

    #[test]
    fn invalid_selection_does_not_discard_other_successful_grants() {
        let directory = Directory::new();
        let backend = Backend::default();
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        assert!(
            access
                .remember_with(
                    &[uris(1)[0].clone(), "https://invalid".into()],
                    false,
                    &backend
                )
                .is_err()
        );
        assert_eq!(directory.read(), grants(1, READ));
    }

    #[test]
    fn partial_persistence_reports_provider_error_and_keeps_successes() {
        let directory = Directory::new();
        let backend = Backend::default();
        backend.reject_uri.replace(Some(uris(2)[1].clone()));
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        let error = access.remember_with(&uris(3), false, &backend).unwrap_err();
        assert!(error.to_string().contains("provider denied requested mode"));
        assert!(error.to_string().contains(&uris(2)[1]));
        assert_eq!(directory.read().len(), 2);
        assert!(!directory.read().contains_key(&uris(2)[1]));
        assert_eq!(backend.sizes("persist"), [3]);
        assert_eq!(backend.sizes("activate"), [2]);
    }

    #[test]
    fn failed_batch_does_not_discard_successful_later_batches() {
        let directory = Directory::new();
        let backend = Backend::default();
        backend.fail_first_persist.set(true);
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        assert!(
            access
                .remember_with(&uris(BATCH_SIZE + 5), false, &backend)
                .is_err()
        );
        assert_eq!(backend.sizes("persist"), [BATCH_SIZE, 5]);
        assert_eq!(backend.sizes("activate"), [5]);
        assert_eq!(directory.read().len(), 5);
    }

    #[test]
    fn read_selection_never_downgrades_an_existing_writable_grant() {
        let directory = Directory::new();
        directory.write(&grants(1, READ_WRITE));
        let backend = Backend::default();
        backend.persistent.replace(grants(1, READ_WRITE));
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        access.remember_with(&uris(1), false, &backend).unwrap();
        assert_eq!(directory.read(), grants(1, READ_WRITE));
        assert_eq!(
            backend
                .calls
                .borrow()
                .iter()
                .find(|(name, _)| *name == "persist")
                .unwrap()
                .1[0]
                .operation_mode,
            READ
        );
    }

    #[test]
    fn missing_private_directory_is_an_error_only_when_opted_in_after_frame() {
        let backend = Backend::default();
        let access = FileAccess::new(None, true);
        access.restore_with(&backend).unwrap();
        access.on_presentation(Presentation::Presented);
        assert!(access.restore_with(&backend).is_err());
        assert!(backend.calls.borrow().is_empty());
        let disabled = FileAccess::new(None, false);
        disabled.restore_with(&backend).unwrap();
        disabled.remember_with(&uris(1), false, &backend).unwrap();
    }

    #[test]
    fn activation_failure_saves_persisted_grants_and_can_retry_without_persist() {
        let directory = Directory::new();
        let backend = Backend::default();
        backend.fail_activate.set(true);
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        assert!(access.remember_with(&uris(3), false, &backend).is_err());
        assert_eq!(directory.read(), grants(3, READ));
        backend.fail_activate.set(false);
        backend.calls.borrow_mut().clear();
        access.restore_with(&backend).unwrap();
        assert_eq!(backend.sizes("check"), [3]);
        assert_eq!(backend.sizes("activate"), [3]);
        assert!(backend.sizes("persist").is_empty());
    }

    #[test]
    fn explicit_save_uses_write_but_reports_read_only_fallback() {
        let directory = Directory::new();
        let backend = Backend::default();
        backend.read_only.set(true);
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        assert!(access.remember_with(&uris(3), true, &backend).is_err());
        assert_eq!(directory.read(), grants(3, READ));
        let calls = backend.calls.borrow();
        assert_eq!(calls[0].1[0].operation_mode, READ_WRITE);
        assert_eq!(calls[1].1[0].operation_mode, READ);
    }

    #[test]
    fn revoked_write_restores_remaining_read_access_in_batches() {
        let directory = Directory::new();
        directory.write(&grants(70, READ_WRITE));
        let backend = Backend::default();
        backend.persistent.replace(grants(69, READ));
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        access.restore_with(&backend).unwrap();
        assert_eq!(directory.read(), grants(69, READ));
        assert_eq!(backend.sizes("check"), [64, 6, 64, 6]);
        assert_eq!(backend.sizes("activate"), [64, 5]);
        assert!(backend.sizes("persist").is_empty());
    }

    #[test]
    fn transient_query_and_invalid_cardinality_do_not_erase_grants() {
        for invalid_count in [false, true] {
            let directory = Directory::new();
            directory.write(&grants(3, READ));
            let backend = Backend::default();
            backend.persistent.replace(grants(3, READ));
            backend.fail_check.set(!invalid_count);
            backend.short_check.set(invalid_count);
            let access = directory.access(true);
            access.on_presentation(Presentation::Presented);
            assert!(access.restore_with(&backend).is_err());
            assert_eq!(directory.read(), grants(3, READ));
            backend.fail_check.set(false);
            backend.short_check.set(false);
            access.restore_with(&backend).unwrap();
            assert_eq!(backend.sizes("activate"), [3]);
        }
    }

    #[test]
    fn invalid_failure_response_is_not_treated_as_success() {
        let directory = Directory::new();
        let backend = Backend::default();
        backend.invalid_failure.set(true);
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        assert!(access.remember_with(&uris(2), false, &backend).is_err());
        assert!(!directory.history().exists());
        assert!(backend.sizes("activate").is_empty());
    }

    #[test]
    fn failed_atomic_write_is_reported_and_retried_without_repersisting() {
        let directory = Directory::new();
        let temporary = directory.history().with_extension("json.tmp");
        std::fs::create_dir(&temporary).unwrap();
        let backend = Backend::default();
        let access = directory.access(true);
        access.on_presentation(Presentation::Presented);
        assert!(access.remember_with(&uris(3), false, &backend).is_err());
        assert!(!directory.history().exists());
        std::fs::remove_dir(temporary).unwrap();
        backend.calls.borrow_mut().clear();
        access.restore_with(&backend).unwrap();
        assert_eq!(directory.read(), grants(3, READ));
        assert!(backend.calls.borrow().is_empty());
    }
}
