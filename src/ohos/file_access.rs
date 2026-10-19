use anyhow::{Context as _, Result};
use ohos_fileshare_binding::PolicyInfo;
use parking_lot::Mutex;
use std::{collections::BTreeMap, io::Write as _, path::PathBuf};

const READ: u32 = 1;
const READ_WRITE: u32 = 3;

pub(crate) struct FileAccess {
    path: Option<PathBuf>,
    state: Mutex<FileAccessState>,
}

#[derive(Default)]
struct FileAccessState {
    policies: Option<BTreeMap<String, u32>>,
    restored: bool,
    dirty: bool,
}

trait FileShare {
    fn persist(&self, policy: &PolicyInfo) -> Result<()>;
    fn is_persistent(&self, policy: &PolicyInfo) -> Result<bool>;
    fn activate(&self, policy: &PolicyInfo) -> Result<()>;
}

struct NativeFileShare;

impl FileShare for NativeFileShare {
    fn persist(&self, policy: &PolicyInfo) -> Result<()> {
        let failed = ohos_fileshare_binding::persist_permission(std::slice::from_ref(policy))?;
        anyhow::ensure!(
            failed.is_empty(),
            "File permission persistence failed: {failed:?}"
        );
        Ok(())
    }

    fn is_persistent(&self, policy: &PolicyInfo) -> Result<bool> {
        let result =
            ohos_fileshare_binding::check_persistent_permission(std::slice::from_ref(policy))?;
        anyhow::ensure!(
            result.len() == 1,
            "File permission query returned an invalid result"
        );
        result
            .first()
            .copied()
            .context("File permission query returned no result")
    }

    fn activate(&self, policy: &PolicyInfo) -> Result<()> {
        let failed = ohos_fileshare_binding::activate_permission(std::slice::from_ref(policy))?;
        anyhow::ensure!(
            failed.is_empty(),
            "File permission activation failed: {failed:?}"
        );
        Ok(())
    }
}

impl FileAccess {
    pub(crate) fn new(base_path: Option<String>) -> Self {
        Self {
            path: base_path
                .filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join("gpui-ohos-file-access.json")),
            state: Mutex::new(FileAccessState::default()),
        }
    }

    pub(crate) fn restore(&self) -> Result<()> {
        self.restore_with(&NativeFileShare)
    }

    pub(crate) fn remember(&self, uris: &[String]) -> Result<()> {
        self.remember_with(uris, &NativeFileShare)
    }

    fn restore_with(&self, backend: &impl FileShare) -> Result<()> {
        self.restore_state(&mut self.state.lock(), backend)
    }

    fn read_history(&self) -> Result<BTreeMap<String, u32>> {
        let Some(path) = &self.path else {
            return Ok(BTreeMap::new());
        };
        let policies: BTreeMap<String, u32> = match std::fs::read(path) {
            Ok(contents) => serde_json::from_slice(&contents)
                .with_context(|| format!("Read file access history {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error).context("Read file access history"),
        };
        anyhow::ensure!(
            policies.iter().all(|(uri, mode)| uri.starts_with("file://")
                && matches!(*mode, READ | READ_WRITE)),
            "Invalid file access history"
        );
        Ok(policies)
    }

    fn restore_state(&self, state: &mut FileAccessState, backend: &impl FileShare) -> Result<()> {
        if state.restored {
            return self.save_state(state);
        }
        if state.policies.is_none() {
            state.policies = Some(self.read_history()?);
        }
        let policies = state
            .policies
            .as_mut()
            .context("File access history is unavailable")?;
        let mut updates = Vec::new();
        let mut restored = 0;
        let mut complete = true;
        for (uri, mode) in policies.iter() {
            let mut policy = PolicyInfo {
                uri: uri.clone(),
                operation_mode: *mode,
            };
            match backend.is_persistent(&policy) {
                Ok(true) => match backend.activate(&policy) {
                    Ok(()) => restored += 1,
                    Err(error) => {
                        complete = false;
                        log::warn!("Cannot restore file access for {uri}: {error:#}");
                    }
                },
                Ok(false) if *mode == READ_WRITE => {
                    policy.operation_mode = READ;
                    match backend.is_persistent(&policy) {
                        Ok(true) => {
                            updates.push((uri.clone(), Some(READ)));
                            match backend.activate(&policy) {
                                Ok(()) => restored += 1,
                                Err(error) => {
                                    complete = false;
                                    log::warn!(
                                        "Cannot restore read-only file access for {uri}: {error:#}"
                                    );
                                }
                            }
                        }
                        Ok(false) => updates.push((uri.clone(), None)),
                        Err(error) => {
                            complete = false;
                            log::warn!("Cannot check read-only file access for {uri}: {error:#}");
                        }
                    }
                }
                Ok(false) => updates.push((uri.clone(), None)),
                Err(error) => {
                    complete = false;
                    log::warn!("Cannot check saved file access for {uri}: {error:#}");
                }
            }
        }
        for (uri, mode) in &updates {
            if let Some(mode) = mode {
                policies.insert(uri.clone(), *mode);
            } else {
                policies.remove(uri);
            }
        }
        state.dirty |= !updates.is_empty();
        self.save_state(state)?;
        state.restored = complete;
        if restored > 0 {
            log::info!("Restored OHOS file access for {restored} selected paths");
        }
        Ok(())
    }

    fn remember_with(&self, uris: &[String], backend: &impl FileShare) -> Result<()> {
        let mut state = self.state.lock();
        self.restore_state(&mut state, backend)?;
        let policies = state
            .policies
            .as_mut()
            .context("File access history could not be loaded")?;
        let mut remembered = false;
        let mut complete = true;
        let mut failures = Vec::new();
        for uri in uris {
            let mut policy = PolicyInfo {
                uri: uri.clone(),
                operation_mode: READ_WRITE,
            };
            if let Err(error) = backend.persist(&policy) {
                // Providers can grant read-only access. Keep that access without claiming a
                // write grant that the native permission service rejected.
                log::debug!("Cannot retain writable file access for {uri}: {error:#}");
                policy.operation_mode = READ;
                if let Err(error) = backend.persist(&policy) {
                    failures.push(format!("Cannot retain file access for {uri}: {error:#}"));
                    complete = false;
                    continue;
                }
            }
            if let Err(error) = backend.activate(&policy) {
                complete = false;
                failures.push(format!(
                    "Cannot activate retained file access for {uri}: {error:#}"
                ));
            }
            policies.insert(uri.clone(), policy.operation_mode);
            remembered = true;
        }
        state.dirty |= remembered;
        state.restored &= complete;
        if let Err(error) = self.save_state(&mut state) {
            failures.push(format!("Cannot save file access history: {error:#}"));
        }
        anyhow::ensure!(failures.is_empty(), "{}", failures.join("\n"));
        Ok(())
    }

    fn save_state(&self, state: &mut FileAccessState) -> Result<()> {
        if state.dirty {
            self.save(
                state
                    .policies
                    .as_ref()
                    .context("File access history is unavailable")?,
            )?;
            state.dirty = false;
        }
        Ok(())
    }

    fn save(&self, policies: &BTreeMap<String, u32>) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .context("No private directory for file access history")?;
        let parent = path
            .parent()
            .context("File access history has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let contents = serde_json::to_vec(policies)?;
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const URI: &str = "file://docs/storage/Users/currentUser/Documents/project%20%23%25";

    #[derive(Default)]
    struct Backend {
        temporary: Cell<u32>,
        persistent: Cell<u32>,
        active: Cell<u32>,
        fail_checks: Cell<bool>,
        fail_activations: Cell<bool>,
        rejected_uri: Option<String>,
        calls: RefCell<Vec<(&'static str, u32)>>,
    }

    impl FileShare for Backend {
        fn persist(&self, policy: &PolicyInfo) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(("persist", policy.operation_mode));
            anyhow::ensure!(
                self.rejected_uri.as_ref() != Some(&policy.uri),
                "Rejected URI"
            );
            anyhow::ensure!(
                self.temporary.get() & policy.operation_mode == policy.operation_mode,
                "Not granted"
            );
            self.persistent
                .set(self.persistent.get() | policy.operation_mode);
            Ok(())
        }

        fn is_persistent(&self, policy: &PolicyInfo) -> Result<bool> {
            self.calls
                .borrow_mut()
                .push(("check", policy.operation_mode));
            anyhow::ensure!(!self.fail_checks.get(), "Service unavailable");
            Ok(self.persistent.get() & policy.operation_mode == policy.operation_mode)
        }

        fn activate(&self, policy: &PolicyInfo) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(("activate", policy.operation_mode));
            anyhow::ensure!(!self.fail_activations.get(), "Service unavailable");
            anyhow::ensure!(
                self.persistent.get() & policy.operation_mode == policy.operation_mode,
                "Not persistent"
            );
            self.active.set(self.active.get() | policy.operation_mode);
            Ok(())
        }
    }

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Result<Self> {
            let path =
                std::env::temp_dir().join(format!("gpui-file-access-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn access(&self) -> Result<FileAccess> {
            Ok(FileAccess::new(Some(
                self.0.to_str().context("Test path is not UTF-8")?.into(),
            )))
        }

        fn history(&self) -> PathBuf {
            self.0.join("gpui-ohos-file-access.json")
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                eprintln!(
                    "Could not remove test directory {}: {error}",
                    self.0.display()
                );
            }
        }
    }

    fn remember(directory: &Directory, backend: &Backend) -> Result<()> {
        let access = directory.access()?;
        access.restore_with(backend)?;
        access.remember_with(&[URI.into()], backend)
    }

    #[test]
    fn restores_selected_read_write_grants_after_restart_without_granting_again() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        remember(&directory, &backend)?;
        assert_eq!(backend.persistent.get(), READ_WRITE);
        backend.active.set(0);
        backend.temporary.set(0);
        backend.calls.borrow_mut().clear();
        directory.access()?.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ_WRITE);
        assert_eq!(
            *backend.calls.borrow(),
            [("check", READ_WRITE), ("activate", READ_WRITE)]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(directory.history())?.permissions().mode() & 0o777,
                0o600
            );
        }
        Ok(())
    }

    #[test]
    fn retains_read_only_grants_without_claiming_write_access() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ);
        remember(&directory, &backend)?;
        assert_eq!(backend.persistent.get(), READ);
        backend.active.set(0);
        directory.access()?.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ);
        Ok(())
    }

    #[test]
    fn removes_revoked_grants_without_persisting_them_again() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        remember(&directory, &backend)?;
        backend.persistent.set(0);
        backend.active.set(0);
        backend.calls.borrow_mut().clear();
        directory.access()?.restore_with(&backend)?;
        assert_eq!(backend.active.get(), 0);
        assert!(
            !backend
                .calls
                .borrow()
                .iter()
                .any(|(operation, _)| *operation == "persist")
        );
        assert_eq!(std::fs::read_to_string(directory.history())?, "{}");
        Ok(())
    }

    #[test]
    fn restores_remaining_read_grant_when_write_was_revoked() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        remember(&directory, &backend)?;
        backend.persistent.set(READ);
        backend.active.set(0);
        directory.access()?.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ);
        let policies: BTreeMap<String, u32> =
            serde_json::from_slice(&std::fs::read(directory.history())?)?;
        assert_eq!(policies.get(URI), Some(&READ));
        Ok(())
    }

    #[test]
    fn transient_query_errors_do_not_erase_saved_grants() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        remember(&directory, &backend)?;
        let original = std::fs::read(directory.history())?;
        backend.active.set(0);
        backend.fail_checks.set(true);
        let access = directory.access()?;
        access.restore_with(&backend)?;
        assert_eq!(backend.active.get(), 0);
        assert_eq!(std::fs::read(directory.history())?, original);
        backend.fail_checks.set(false);
        access.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ_WRITE);
        Ok(())
    }

    #[test]
    fn failed_activation_can_be_retried_without_persisting_again() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        remember(&directory, &backend)?;
        backend.active.set(0);
        backend.temporary.set(0);
        backend.calls.borrow_mut().clear();
        backend.fail_activations.set(true);
        let access = directory.access()?;
        access.restore_with(&backend)?;
        assert_eq!(backend.active.get(), 0);
        backend.fail_activations.set(false);
        access.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ_WRITE);
        assert!(
            !backend
                .calls
                .borrow()
                .iter()
                .any(|(operation, _)| *operation == "persist")
        );
        Ok(())
    }

    #[test]
    fn failed_history_read_can_be_retried_on_the_same_access() -> Result<()> {
        let directory = Directory::new()?;
        std::fs::create_dir(directory.history())?;
        let backend = Backend::default();
        let access = directory.access()?;
        assert!(access.restore_with(&backend).is_err());
        std::fs::remove_dir(directory.history())?;
        std::fs::write(
            directory.history(),
            serde_json::to_vec(&BTreeMap::from([(URI, READ_WRITE)]))?,
        )?;
        backend.persistent.set(READ_WRITE);
        access.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ_WRITE);
        Ok(())
    }

    #[test]
    fn newly_remembered_grants_retry_failed_activation_without_persisting_again() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        backend.fail_activations.set(true);
        let access = directory.access()?;
        assert!(access.remember_with(&[URI.into()], &backend).is_err());
        assert_eq!(backend.active.get(), 0);
        backend.fail_activations.set(false);
        backend.temporary.set(0);
        backend.calls.borrow_mut().clear();
        access.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ_WRITE);
        assert!(
            !backend
                .calls
                .borrow()
                .iter()
                .any(|(operation, _)| *operation == "persist")
        );
        Ok(())
    }

    #[test]
    fn failed_revocation_write_can_be_retried_without_restoring_removed_grants() -> Result<()> {
        let directory = Directory::new()?;
        std::fs::write(
            directory.history(),
            serde_json::to_vec(&BTreeMap::from([(URI, READ_WRITE)]))?,
        )?;
        let temporary = directory.history().with_extension("json.tmp");
        std::fs::create_dir(&temporary)?;
        let backend = Backend::default();
        let access = directory.access()?;
        assert!(access.restore_with(&backend).is_err());
        std::fs::remove_dir(temporary)?;
        access.restore_with(&backend)?;
        assert_eq!(std::fs::read_to_string(directory.history())?, "{}");
        assert_eq!(backend.active.get(), 0);
        assert!(
            !backend
                .calls
                .borrow()
                .iter()
                .any(|(operation, _)| *operation == "persist")
        );
        Ok(())
    }

    #[test]
    fn malformed_history_is_reported_and_preserved() -> Result<()> {
        let directory = Directory::new()?;
        std::fs::write(directory.history(), "truncated{")?;
        let backend = Backend::default();
        let access = directory.access()?;
        assert!(access.restore_with(&backend).is_err());
        assert!(access.remember_with(&[URI.into()], &backend).is_err());
        assert_eq!(std::fs::read_to_string(directory.history())?, "truncated{");
        assert!(backend.calls.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn a_failed_history_write_can_be_retried_for_the_same_uri() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        let access = directory.access()?;
        access.restore_with(&backend)?;
        let temporary = directory.history().with_extension("json.tmp");
        std::fs::create_dir(&temporary)?;
        assert!(access.remember_with(&[URI.into()], &backend).is_err());
        assert!(!directory.history().exists());
        std::fs::remove_dir(&temporary)?;
        access.remember_with(&[URI.into()], &backend)?;
        backend.active.set(0);
        directory.access()?.restore_with(&backend)?;
        assert_eq!(backend.active.get(), READ_WRITE);
        Ok(())
    }

    #[test]
    fn rejected_grants_are_not_saved() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        assert!(remember(&directory, &backend).is_err());
        assert!(!directory.history().exists());
        assert_eq!(backend.active.get(), 0);
        Ok(())
    }

    #[test]
    fn partial_persistence_reports_failure_and_saves_successful_grants() -> Result<()> {
        let directory = Directory::new()?;
        let rejected = format!("{URI}/rejected");
        let backend = Backend {
            rejected_uri: Some(rejected.clone()),
            ..Default::default()
        };
        backend.temporary.set(READ_WRITE);
        let access = directory.access()?;
        let error = access
            .remember_with(&[URI.into(), rejected.clone()], &backend)
            .expect_err("partial persistence must be reported");
        assert!(error.to_string().contains(&rejected));
        let policies: BTreeMap<String, u32> =
            serde_json::from_slice(&std::fs::read(directory.history())?)?;
        assert_eq!(policies.get(URI), Some(&READ_WRITE));
        assert!(!policies.contains_key(&rejected));
        Ok(())
    }

    #[test]
    fn incoming_grants_can_arrive_before_startup_restoration() -> Result<()> {
        let directory = Directory::new()?;
        let backend = Backend::default();
        backend.temporary.set(READ_WRITE);
        let access = directory.access()?;
        access.remember_with(&[URI.into()], &backend)?;
        backend.calls.borrow_mut().clear();
        access.restore_with(&backend)?;
        assert!(backend.calls.borrow().is_empty());
        let policies: BTreeMap<String, u32> =
            serde_json::from_slice(&std::fs::read(directory.history())?)?;
        assert_eq!(policies.get(URI), Some(&READ_WRITE));
        Ok(())
    }

    #[test]
    fn concurrent_initialization_restores_saved_grants_once() -> Result<()> {
        use std::sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Default)]
        struct CountingBackend(AtomicUsize);

        impl FileShare for CountingBackend {
            fn persist(&self, _policy: &PolicyInfo) -> Result<()> {
                anyhow::bail!("Restoring a grant must not persist it again")
            }

            fn is_persistent(&self, _policy: &PolicyInfo) -> Result<bool> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            }

            fn activate(&self, _policy: &PolicyInfo) -> Result<()> {
                Ok(())
            }
        }

        let directory = Directory::new()?;
        std::fs::write(
            directory.history(),
            serde_json::to_vec(&BTreeMap::from([(URI, READ_WRITE)]))?,
        )?;
        let access = directory.access()?;
        let backend = CountingBackend::default();
        let barrier = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                access.restore_with(&backend)
            });
            barrier.wait();
            access.restore_with(&backend)?;
            first
                .join()
                .map_err(|_| anyhow::anyhow!("Restoration thread panicked"))?
        })?;
        assert_eq!(backend.0.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
