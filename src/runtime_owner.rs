use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::paths::{
    app_support_dir_path, log_file_path, response_surface_ledger_path, runtime_owner_lock_path,
};
use crate::process::{ProcessManager, ProcessState, SystemProcessManager};

#[derive(Debug)]
pub struct RuntimeOwnerLock {
    path: PathBuf,
    token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeOwnerInfo {
    pub pid: u32,
    pub config_path: String,
    pub log_path: String,
    pub state_path: String,
    pub ledger_path: String,
    pub started_at: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwnerLockError {
    pub lock_path: PathBuf,
    pub existing: RuntimeOwnerInfo,
}

impl fmt::Display for RuntimeOwnerLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "another agents-router watch runtime already owns this state"
        )?;
        writeln!(formatter, "existing pid: {}", self.existing.pid)?;
        writeln!(formatter, "config: {}", self.existing.config_path)?;
        writeln!(formatter, "log: {}", self.existing.log_path)?;
        writeln!(formatter, "state: {}", self.existing.state_path)?;
        writeln!(formatter, "ledger: {}", self.existing.ledger_path)?;
        writeln!(formatter, "lock: {}", self.lock_path.display())?;
        write!(
            formatter,
            "stop or restart the existing runtime explicitly before starting another one"
        )
    }
}

impl std::error::Error for RuntimeOwnerLockError {}

impl RuntimeOwnerLock {
    pub fn acquire_default(config_path: &Path) -> anyhow::Result<Self> {
        let info = RuntimeOwnerInfo::new(
            std::process::id(),
            normalize_path(config_path)?,
            log_file_path()?,
            app_support_dir_path()?,
            response_surface_ledger_path()?,
        );
        Self::acquire_with_manager(runtime_owner_lock_path()?, info, &SystemProcessManager)
    }

    pub fn acquire_with_manager(
        path: PathBuf,
        info: RuntimeOwnerInfo,
        manager: &dyn ProcessManager,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create runtime owner lock directory `{}`",
                    parent.display()
                )
            })?;
        }

        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let raw = serde_json::to_vec_pretty(&info)
                        .context("failed to serialize runtime owner lock")?;
                    file.write_all(&raw).with_context(|| {
                        format!("failed to write runtime owner lock `{}`", path.display())
                    })?;
                    info!(
                        runtime.pid = info.pid,
                        config.path = %info.config_path,
                        log.path = %info.log_path,
                        state.path = %info.state_path,
                        ledger.path = %info.ledger_path,
                        lock.path = %path.display(),
                        event = "runtime_owner_lock.acquired",
                    );
                    return Ok(Self {
                        path,
                        token: info.token,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let existing = read_owner_info(&path)?;
                    match manager.process_state(existing.pid)? {
                        ProcessState::Missing => {
                            warn!(
                                runtime.pid = existing.pid,
                                lock.path = %path.display(),
                                event = "runtime_owner_lock.stale_removed",
                            );
                            fs::remove_file(&path).with_context(|| {
                                format!(
                                    "failed to remove stale runtime owner lock `{}`",
                                    path.display()
                                )
                            })?;
                            continue;
                        }
                        ProcessState::AgentsRouter | ProcessState::Other => {
                            warn!(
                                runtime.pid = existing.pid,
                                config.path = %existing.config_path,
                                log.path = %existing.log_path,
                                state.path = %existing.state_path,
                                ledger.path = %existing.ledger_path,
                                lock.path = %path.display(),
                                event = "runtime_owner_lock.busy",
                            );
                            return Err(anyhow!(RuntimeOwnerLockError {
                                lock_path: path,
                                existing,
                            }));
                        }
                    }
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create runtime owner lock `{}`", path.display())
                    });
                }
            }
        }
    }
}

impl Drop for RuntimeOwnerLock {
    fn drop(&mut self) {
        let Ok(existing) = read_owner_info(&self.path) else {
            return;
        };
        if existing.pid == std::process::id()
            && existing.token == self.token
            && let Err(error) = fs::remove_file(&self.path)
        {
            warn!(
                lock.path = %self.path.display(),
                error = %error,
                event = "runtime_owner_lock.release_failed",
            );
        }
    }
}

impl RuntimeOwnerInfo {
    pub fn new(
        pid: u32,
        config_path: PathBuf,
        log_path: PathBuf,
        state_path: PathBuf,
        ledger_path: PathBuf,
    ) -> Self {
        Self {
            pid,
            config_path: config_path.display().to_string(),
            log_path: log_path.display().to_string(),
            state_path: state_path.display().to_string(),
            ledger_path: ledger_path.display().to_string(),
            started_at: Utc::now().to_rfc3339(),
            token: Uuid::new_v4().to_string(),
        }
    }
}

fn read_owner_info(path: &Path) -> anyhow::Result<RuntimeOwnerInfo> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read runtime owner lock `{}`", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse runtime owner lock `{}`", path.display()))
}

fn normalize_path(path: &Path) -> anyhow::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to detect current directory")?
            .join(path)
    };
    Ok(fs::canonicalize(&path).unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use tempfile::tempdir;

    use super::*;
    use crate::process::ProcessState;

    struct FakeProcessManager {
        states: RefCell<Vec<ProcessState>>,
    }

    impl FakeProcessManager {
        fn new(states: Vec<ProcessState>) -> Self {
            Self {
                states: RefCell::new(states),
            }
        }
    }

    impl ProcessManager for FakeProcessManager {
        fn process_state(&self, _pid: u32) -> anyhow::Result<ProcessState> {
            Ok(self.states.borrow_mut().remove(0))
        }

        fn terminate(&self, _pid: u32) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn second_watch_runtime_fails_fast_when_owner_lock_is_busy() {
        let dir = tempdir().expect("tempdir should be created");
        let lock_path = dir.path().join("watch-owner.lock");
        let existing = RuntimeOwnerInfo::new(
            123,
            PathBuf::from("/tmp/config.toml"),
            PathBuf::from("/tmp/agents-router.log"),
            PathBuf::from("/tmp/state"),
            PathBuf::from("/tmp/response-surface-ledger.json"),
        );
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&existing).expect("lock should serialize"),
        )
        .expect("lock should be written");
        let manager = FakeProcessManager::new(vec![ProcessState::AgentsRouter]);

        let error = RuntimeOwnerLock::acquire_with_manager(
            lock_path.clone(),
            RuntimeOwnerInfo::new(
                456,
                PathBuf::from("/tmp/config.toml"),
                PathBuf::from("/tmp/agents-router.log"),
                PathBuf::from("/tmp/state"),
                PathBuf::from("/tmp/response-surface-ledger.json"),
            ),
            &manager,
        )
        .expect_err("busy lock should fail fast");
        let message = error.to_string();

        assert!(message.contains("existing pid: 123"));
        assert!(message.contains("config: /tmp/config.toml"));
        assert!(message.contains("log: /tmp/agents-router.log"));
        assert!(message.contains("state: /tmp/state"));
        assert!(message.contains("ledger: /tmp/response-surface-ledger.json"));
        assert!(message.contains(&format!("lock: {}", lock_path.display())));
    }

    #[test]
    fn stale_owner_lock_is_removed_before_acquire() {
        let dir = tempdir().expect("tempdir should be created");
        let lock_path = dir.path().join("watch-owner.lock");
        let stale = RuntimeOwnerInfo::new(
            123,
            PathBuf::from("/tmp/old.toml"),
            PathBuf::from("/tmp/old.log"),
            PathBuf::from("/tmp/old-state"),
            PathBuf::from("/tmp/old-ledger.json"),
        );
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&stale).expect("lock should serialize"),
        )
        .expect("lock should be written");
        let manager = FakeProcessManager::new(vec![ProcessState::Missing]);

        let lock = RuntimeOwnerLock::acquire_with_manager(
            lock_path.clone(),
            RuntimeOwnerInfo::new(
                456,
                PathBuf::from("/tmp/new.toml"),
                PathBuf::from("/tmp/new.log"),
                PathBuf::from("/tmp/new-state"),
                PathBuf::from("/tmp/new-ledger.json"),
            ),
            &manager,
        )
        .expect("stale lock should be replaced");
        let current = read_owner_info(&lock_path).expect("new lock should be readable");

        assert_eq!(current.pid, 456);
        drop(lock);
    }
}
