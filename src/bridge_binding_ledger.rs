use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::config::is_clean_absolute_project_path;
use crate::paths::bridge_binding_ledger_path;

const BRIDGE_BINDING_LEDGER_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct BridgeBindingLedger {
    state_path: Option<PathBuf>,
    state: BridgeBindingLedgerFile,
}

#[derive(Debug, Clone)]
pub struct BridgeBindingLedgerStore {
    state_path: PathBuf,
    lock: Arc<AsyncMutex<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BridgeBindingLedgerFile {
    schema_version: u32,
    room_project_bindings: Vec<RoomProjectBindingRecord>,
    thread_session_bindings: Vec<ThreadSessionBindingRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomProjectBindingRecord {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub project_path: String,
    pub status: RoomProjectBindingStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoomProjectBindingStatus {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomProjectBindingInput {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomBindingQuery {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadSessionBindingRecord {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub project_path: String,
    pub source_id: String,
    pub source_type: String,
    pub source_session_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSessionBindingInput {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub project_path: String,
    pub source_id: String,
    pub source_type: String,
    pub source_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadBindingQuery {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
}

impl BridgeBindingLedger {
    pub fn in_memory() -> Self {
        Self {
            state_path: None,
            state: BridgeBindingLedgerFile::empty(),
        }
    }

    pub fn load(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let state = match fs::read_to_string(&path) {
            Ok(raw) => {
                let state: BridgeBindingLedgerFile =
                    serde_json::from_str(&raw).with_context(|| {
                        format!(
                            "failed to parse bridge binding ledger at {}",
                            path.display()
                        )
                    })?;
                ensure!(
                    state.schema_version == BRIDGE_BINDING_LEDGER_SCHEMA_VERSION,
                    "unsupported bridge binding ledger schema_version {}; expected {}",
                    state.schema_version,
                    BRIDGE_BINDING_LEDGER_SCHEMA_VERSION
                );
                state
            }
            Err(error) if error.kind() == ErrorKind::NotFound => BridgeBindingLedgerFile::empty(),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read bridge binding ledger at {}", path.display())
                });
            }
        };

        Ok(Self {
            state_path: Some(path),
            state,
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = self.state_path.as_ref() else {
            return Ok(());
        };
        save_state(path, &self.state)
    }

    pub fn connect_room_project_at(
        &mut self,
        input: RoomProjectBindingInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<RoomProjectBindingRecord> {
        validate_room_project_binding_input(&input)?;
        if let Some(record) = self
            .state
            .room_project_bindings
            .iter_mut()
            .find(|record| room_project_binding_matches(record, &input))
        {
            record.status = RoomProjectBindingStatus::Connected;
            record.updated_at = now;
            let record = record.clone();
            self.save()?;
            return Ok(record);
        }

        let record = RoomProjectBindingRecord {
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            provider_account_id: input.provider_account_id,
            provider_conversation_id: input.provider_conversation_id,
            project_path: input.project_path,
            status: RoomProjectBindingStatus::Connected,
            created_at: now,
            updated_at: now,
        };
        self.state.room_project_bindings.push(record.clone());
        self.save()?;
        Ok(record)
    }

    pub fn disconnect_room_project_at(
        &mut self,
        input: RoomProjectBindingInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<RoomProjectBindingRecord>> {
        validate_room_project_binding_input(&input)?;
        let Some(record) = self
            .state
            .room_project_bindings
            .iter_mut()
            .find(|record| room_project_binding_matches(record, &input))
        else {
            return Ok(None);
        };
        record.status = RoomProjectBindingStatus::Disconnected;
        record.updated_at = now;
        let record = record.clone();
        self.save()?;
        Ok(Some(record))
    }

    pub fn connected_projects_for_room(
        &self,
        query: &RoomBindingQuery,
    ) -> Vec<RoomProjectBindingRecord> {
        self.state
            .room_project_bindings
            .iter()
            .filter(|record| {
                room_binding_matches(record, query)
                    && record.status == RoomProjectBindingStatus::Connected
            })
            .cloned()
            .collect()
    }

    pub fn connected_rooms_for_project(&self, project_path: &str) -> Vec<RoomProjectBindingRecord> {
        self.state
            .room_project_bindings
            .iter()
            .filter(|record| {
                record.project_path == project_path
                    && record.status == RoomProjectBindingStatus::Connected
            })
            .cloned()
            .collect()
    }

    pub fn connected_rooms_for_project_tree(
        &self,
        project_path: &str,
    ) -> Vec<RoomProjectBindingRecord> {
        self.state
            .room_project_bindings
            .iter()
            .filter(|record| {
                record.status == RoomProjectBindingStatus::Connected
                    && Path::new(project_path).starts_with(Path::new(&record.project_path))
            })
            .cloned()
            .collect()
    }

    pub fn bind_thread_session_at(
        &mut self,
        input: ThreadSessionBindingInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<ThreadSessionBindingRecord> {
        validate_thread_session_binding_input(&input)?;
        if let Some(record) = self
            .state
            .thread_session_bindings
            .iter()
            .find(|record| thread_binding_matches(record, &ThreadBindingQuery::from(&input)))
        {
            ensure!(
                thread_session_binding_matches(record, &input),
                "provider thread is already bound to a different source session"
            );
            return Ok(record.clone());
        }

        let record = ThreadSessionBindingRecord {
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            provider_account_id: input.provider_account_id,
            provider_conversation_id: input.provider_conversation_id,
            provider_thread_id: input.provider_thread_id,
            project_path: input.project_path,
            source_id: input.source_id,
            source_type: input.source_type,
            source_session_id: input.source_session_id,
            created_at: now,
        };
        self.state.thread_session_bindings.push(record.clone());
        self.save()?;
        Ok(record)
    }

    pub fn lookup_thread_session(
        &self,
        query: &ThreadBindingQuery,
    ) -> Option<ThreadSessionBindingRecord> {
        self.state
            .thread_session_bindings
            .iter()
            .find(|record| thread_binding_matches(record, query))
            .cloned()
    }
}

impl BridgeBindingLedgerStore {
    pub fn load_default() -> anyhow::Result<Self> {
        Self::new(bridge_binding_ledger_path()?)
    }

    pub fn new(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create bridge binding ledger directory {}",
                    parent.display()
                )
            })?;
        }
        Ok(Self {
            state_path: path,
            lock: Arc::new(AsyncMutex::new(())),
        })
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub async fn update<T>(
        &self,
        operation: impl FnOnce(&mut BridgeBindingLedger) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let _guard = self.lock.lock().await;
        let mut ledger = BridgeBindingLedger::load(self.state_path.clone())?;
        operation(&mut ledger)
    }
}

impl BridgeBindingLedgerFile {
    fn empty() -> Self {
        Self {
            schema_version: BRIDGE_BINDING_LEDGER_SCHEMA_VERSION,
            room_project_bindings: Vec::new(),
            thread_session_bindings: Vec::new(),
        }
    }
}

impl From<&ThreadSessionBindingInput> for ThreadBindingQuery {
    fn from(input: &ThreadSessionBindingInput) -> Self {
        Self {
            provider_id: input.provider_id.clone(),
            provider_type: input.provider_type.clone(),
            provider_account_id: input.provider_account_id.clone(),
            provider_conversation_id: input.provider_conversation_id.clone(),
            provider_thread_id: input.provider_thread_id.clone(),
        }
    }
}

fn validate_room_project_binding_input(input: &RoomProjectBindingInput) -> anyhow::Result<()> {
    validate_provider_binding_parts(
        &input.provider_id,
        &input.provider_type,
        &input.provider_account_id,
        &input.provider_conversation_id,
    )?;
    validate_project_path(&input.project_path)
}

fn validate_thread_session_binding_input(input: &ThreadSessionBindingInput) -> anyhow::Result<()> {
    validate_provider_binding_parts(
        &input.provider_id,
        &input.provider_type,
        &input.provider_account_id,
        &input.provider_conversation_id,
    )?;
    validate_present("provider_thread_id", &input.provider_thread_id)?;
    validate_project_path(&input.project_path)?;
    validate_present("source_id", &input.source_id)?;
    validate_present("source_type", &input.source_type)?;
    validate_present("source_session_id", &input.source_session_id)
}

fn validate_provider_binding_parts(
    provider_id: &str,
    provider_type: &str,
    provider_account_id: &str,
    provider_conversation_id: &str,
) -> anyhow::Result<()> {
    validate_present("provider_id", provider_id)?;
    validate_present("provider_type", provider_type)?;
    validate_present("provider_account_id", provider_account_id)?;
    validate_present("provider_conversation_id", provider_conversation_id)
}

fn validate_project_path(project_path: &str) -> anyhow::Result<()> {
    ensure!(
        is_clean_absolute_project_path(project_path),
        "project_path must be a clean absolute path"
    );
    Ok(())
}

fn validate_present(field: &str, value: &str) -> anyhow::Result<()> {
    ensure!(!value.trim().is_empty(), "{field} must not be empty");
    ensure!(
        value.trim() == value,
        "{field} must not have surrounding whitespace"
    );
    Ok(())
}

fn room_project_binding_matches(
    record: &RoomProjectBindingRecord,
    input: &RoomProjectBindingInput,
) -> bool {
    record.provider_id == input.provider_id
        && record.provider_type == input.provider_type
        && record.provider_account_id == input.provider_account_id
        && record.provider_conversation_id == input.provider_conversation_id
        && record.project_path == input.project_path
}

fn room_binding_matches(record: &RoomProjectBindingRecord, query: &RoomBindingQuery) -> bool {
    record.provider_id == query.provider_id
        && record.provider_type == query.provider_type
        && record.provider_account_id == query.provider_account_id
        && record.provider_conversation_id == query.provider_conversation_id
}

fn thread_binding_matches(record: &ThreadSessionBindingRecord, query: &ThreadBindingQuery) -> bool {
    record.provider_id == query.provider_id
        && record.provider_type == query.provider_type
        && record.provider_account_id == query.provider_account_id
        && record.provider_conversation_id == query.provider_conversation_id
        && record.provider_thread_id == query.provider_thread_id
}

fn thread_session_binding_matches(
    record: &ThreadSessionBindingRecord,
    input: &ThreadSessionBindingInput,
) -> bool {
    record.project_path == input.project_path
        && record.source_id == input.source_id
        && record.source_type == input.source_type
        && record.source_session_id == input.source_session_id
}

fn save_state(path: &Path, state: &BridgeBindingLedgerFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create bridge binding ledger directory {}",
                parent.display()
            )
        })?;
    }
    let raw =
        serde_json::to_string_pretty(state).context("failed to serialize bridge binding ledger")?;
    fs::write(path, raw).with_context(|| {
        format!(
            "failed to write bridge binding ledger at {}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn room_can_bind_multiple_projects_and_project_can_bind_multiple_rooms() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        ledger
            .connect_room_project_at(room_project("room-1", "/repo/agents-router"), now)
            .expect("room should bind first project");
        ledger
            .connect_room_project_at(room_project("room-1", "/repo/agent-transport-system"), now)
            .expect("room should bind second project");
        ledger
            .connect_room_project_at(room_project("room-2", "/repo/agents-router"), now)
            .expect("second room should bind same project");

        let room_projects = ledger.connected_projects_for_room(&room_query("room-1"));
        assert_eq!(room_projects.len(), 2);
        assert_eq!(
            ledger
                .connected_rooms_for_project("/repo/agents-router")
                .len(),
            2
        );
        assert_eq!(
            ledger
                .connected_rooms_for_project_tree("/repo/agents-router/crate")
                .len(),
            2
        );
    }

    #[test]
    fn thread_binding_is_immutable_after_creation() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        let first = thread_session("thread-1", "/repo/agents-router", "session-1");
        ledger
            .bind_thread_session_at(first.clone(), now)
            .expect("thread should bind source session");
        ledger
            .bind_thread_session_at(first, now)
            .expect("same thread binding should be idempotent");

        let error = ledger
            .bind_thread_session_at(
                thread_session("thread-1", "/repo/agent-transport-system", "session-2"),
                now,
            )
            .expect_err("thread should not silently switch source sessions");

        assert!(
            error
                .to_string()
                .contains("provider thread is already bound to a different source session")
        );
        let binding = ledger
            .lookup_thread_session(&thread_query("thread-1"))
            .expect("thread binding should still exist");
        assert_eq!(binding.project_path, "/repo/agents-router");
        assert_eq!(binding.source_session_id, "session-1");
    }

    #[tokio::test]
    async fn store_persists_room_project_binding() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let store = BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
            .expect("store should build");
        store
            .update(|ledger| {
                ledger.connect_room_project_at(
                    room_project("room-1", "/repo/agents-router"),
                    test_time(),
                )?;
                Ok(())
            })
            .await
            .expect("binding should persist");

        let reloaded = BridgeBindingLedger::load(store.state_path()).expect("ledger should reload");
        assert_eq!(
            reloaded
                .connected_projects_for_room(&room_query("room-1"))
                .len(),
            1
        );
    }

    fn room_project(provider_conversation_id: &str, project_path: &str) -> RoomProjectBindingInput {
        RoomProjectBindingInput {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: provider_conversation_id.to_string(),
            project_path: project_path.to_string(),
        }
    }

    fn room_query(provider_conversation_id: &str) -> RoomBindingQuery {
        RoomBindingQuery {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: provider_conversation_id.to_string(),
        }
    }

    fn thread_session(
        provider_thread_id: &str,
        project_path: &str,
        source_session_id: &str,
    ) -> ThreadSessionBindingInput {
        ThreadSessionBindingInput {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: "room-1".to_string(),
            provider_thread_id: provider_thread_id.to_string(),
            project_path: project_path.to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: source_session_id.to_string(),
        }
    }

    fn thread_query(provider_thread_id: &str) -> ThreadBindingQuery {
        ThreadBindingQuery {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: "room-1".to_string(),
            provider_thread_id: provider_thread_id.to_string(),
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 31, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
