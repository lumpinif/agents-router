use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    #[serde(default)]
    owner_direct_chats: Vec<OwnerDirectChatRecord>,
    #[serde(default)]
    direct_chat_project_bindings: Vec<LegacyDirectChatProjectBindingRecord>,
    #[serde(default)]
    project_room_proposals: Vec<ProjectRoomProposalRecord>,
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
pub struct OwnerDirectChatRecord {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerDirectChatInput {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LegacyDirectChatProjectBindingRecord {
    provider_id: String,
    provider_type: String,
    provider_account_id: String,
    provider_conversation_id: String,
    project_path: String,
    status: RoomProjectBindingStatus,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRoomProposalRecord {
    pub proposal_id: String,
    pub provider_id: String,
    pub provider_type: String,
    pub project_path: String,
    pub project_name: String,
    pub status: ProjectRoomProposalStatus,
    pub provider_account_id: Option<String>,
    pub prompt_conversation_id: Option<String>,
    pub prompt_message_id: Option<String>,
    pub created_room_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRoomProposalStatus {
    Pending,
    PromptFailed,
    Ignored,
    RoomCreated,
    BoundToExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoomProposalInput {
    pub provider_id: String,
    pub provider_type: String,
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectRoomProposalPromptDecision {
    Prompt(Box<ProjectRoomProposalRecord>),
    Skip(ProjectRoomProposalSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRoomProposalSkipReason {
    AlreadyPending,
    Ignored,
    RoomCreated,
    BoundToExisting,
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
    #[serde(default)]
    pub route_binding_hash: Option<String>,
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
    pub route_binding_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadBindingQuery {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSessionThreadBindingQuery {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub project_path: String,
    pub source_id: String,
    pub source_type: String,
    pub source_session_id: String,
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
            mark_project_room_proposal_bound_to_existing(
                &mut self.state,
                &record.provider_id,
                &record.provider_type,
                &record.provider_account_id,
                &record.provider_conversation_id,
                &record.project_path,
                now,
            );
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
        mark_project_room_proposal_bound_to_existing(
            &mut self.state,
            &record.provider_id,
            &record.provider_type,
            &record.provider_account_id,
            &record.provider_conversation_id,
            &record.project_path,
            now,
        );
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

    pub fn connected_room_project_bindings_for_provider(
        &self,
        provider_id: &str,
        provider_type: &str,
    ) -> anyhow::Result<Vec<RoomProjectBindingRecord>> {
        validate_present("provider_id", provider_id)?;
        validate_present("provider_type", provider_type)?;
        Ok(self
            .state
            .room_project_bindings
            .iter()
            .filter(|record| {
                record.provider_id == provider_id
                    && record.provider_type == provider_type
                    && record.status == RoomProjectBindingStatus::Connected
            })
            .cloned()
            .collect())
    }

    pub fn connected_room_project_bindings_for_provider_account(
        &self,
        provider_id: &str,
        provider_type: &str,
        provider_account_id: &str,
    ) -> anyhow::Result<Vec<RoomProjectBindingRecord>> {
        validate_present("provider_id", provider_id)?;
        validate_present("provider_type", provider_type)?;
        validate_present("provider_account_id", provider_account_id)?;
        Ok(self
            .state
            .room_project_bindings
            .iter()
            .filter(|record| {
                record.provider_id == provider_id
                    && record.provider_type == provider_type
                    && record.provider_account_id == provider_account_id
                    && record.status == RoomProjectBindingStatus::Connected
            })
            .cloned()
            .collect())
    }

    pub fn remember_owner_direct_chat_at(
        &mut self,
        input: OwnerDirectChatInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<OwnerDirectChatRecord> {
        validate_owner_direct_chat_input(&input)?;
        if let Some(record) = self
            .state
            .owner_direct_chats
            .iter_mut()
            .find(|record| owner_direct_chat_matches(record, &input))
        {
            record.updated_at = now;
            let record = record.clone();
            self.save()?;
            return Ok(record);
        }

        let record = OwnerDirectChatRecord {
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            provider_account_id: input.provider_account_id,
            provider_conversation_id: input.provider_conversation_id,
            created_at: now,
            updated_at: now,
        };
        self.state.owner_direct_chats.push(record.clone());
        self.save()?;
        Ok(record)
    }

    pub fn owner_direct_chat_for_provider(
        &self,
        provider_id: &str,
        provider_type: &str,
    ) -> anyhow::Result<Option<OwnerDirectChatRecord>> {
        validate_present("provider_id", provider_id)?;
        validate_present("provider_type", provider_type)?;

        let current = self
            .state
            .owner_direct_chats
            .iter()
            .filter(|record| {
                record.provider_id == provider_id && record.provider_type == provider_type
            })
            .max_by_key(|record| record.updated_at)
            .cloned();
        if current.is_some() {
            return Ok(current);
        }

        Ok(self
            .state
            .direct_chat_project_bindings
            .iter()
            .filter(|record| {
                record.provider_id == provider_id && record.provider_type == provider_type
            })
            .max_by_key(|record| record.updated_at)
            .map(|record| OwnerDirectChatRecord {
                provider_id: record.provider_id.clone(),
                provider_type: record.provider_type.clone(),
                provider_account_id: record.provider_account_id.clone(),
                provider_conversation_id: record.provider_conversation_id.clone(),
                created_at: record.created_at,
                updated_at: record.updated_at,
            }))
    }

    pub fn begin_project_room_proposal_prompt_at(
        &mut self,
        input: ProjectRoomProposalInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<ProjectRoomProposalPromptDecision> {
        validate_project_room_proposal_input(&input)?;
        let proposal_id = project_room_proposal_id(&input.provider_id, &input.project_path);
        let project_name = project_name_from_path(&input.project_path);

        if let Some(record) = self
            .state
            .project_room_proposals
            .iter_mut()
            .find(|record| record.proposal_id == proposal_id)
        {
            record.last_seen_at = now;
            record.updated_at = now;
            let decision = match record.status {
                ProjectRoomProposalStatus::Pending => ProjectRoomProposalPromptDecision::Skip(
                    ProjectRoomProposalSkipReason::AlreadyPending,
                ),
                ProjectRoomProposalStatus::Ignored => {
                    ProjectRoomProposalPromptDecision::Skip(ProjectRoomProposalSkipReason::Ignored)
                }
                ProjectRoomProposalStatus::RoomCreated => ProjectRoomProposalPromptDecision::Skip(
                    ProjectRoomProposalSkipReason::RoomCreated,
                ),
                ProjectRoomProposalStatus::BoundToExisting => {
                    ProjectRoomProposalPromptDecision::Skip(
                        ProjectRoomProposalSkipReason::BoundToExisting,
                    )
                }
                ProjectRoomProposalStatus::PromptFailed => {
                    record.status = ProjectRoomProposalStatus::Pending;
                    record.project_name = project_name;
                    record.provider_account_id = None;
                    record.prompt_conversation_id = None;
                    record.prompt_message_id = None;
                    ProjectRoomProposalPromptDecision::Prompt(Box::new(record.clone()))
                }
            };
            self.save()?;
            return Ok(decision);
        }

        let record = ProjectRoomProposalRecord {
            proposal_id,
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            project_path: input.project_path,
            project_name,
            status: ProjectRoomProposalStatus::Pending,
            provider_account_id: None,
            prompt_conversation_id: None,
            prompt_message_id: None,
            created_room_id: None,
            created_at: now,
            first_seen_at: now,
            last_seen_at: now,
            updated_at: now,
        };
        self.state.project_room_proposals.push(record.clone());
        self.save()?;
        Ok(ProjectRoomProposalPromptDecision::Prompt(Box::new(record)))
    }

    pub fn begin_manual_project_room_proposal_prompt_at(
        &mut self,
        input: ProjectRoomProposalInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<ProjectRoomProposalRecord> {
        validate_project_room_proposal_input(&input)?;
        let proposal_id = project_room_proposal_id(&input.provider_id, &input.project_path);
        let project_name = project_name_from_path(&input.project_path);

        if let Some(record) = self
            .state
            .project_room_proposals
            .iter_mut()
            .find(|record| record.proposal_id == proposal_id)
        {
            record.project_name = project_name;
            record.status = ProjectRoomProposalStatus::Pending;
            record.provider_account_id = None;
            record.prompt_conversation_id = None;
            record.prompt_message_id = None;
            record.created_room_id = None;
            record.last_seen_at = now;
            record.updated_at = now;
            let record = record.clone();
            self.save()?;
            return Ok(record);
        }

        let record = ProjectRoomProposalRecord {
            proposal_id,
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            project_path: input.project_path,
            project_name,
            status: ProjectRoomProposalStatus::Pending,
            provider_account_id: None,
            prompt_conversation_id: None,
            prompt_message_id: None,
            created_room_id: None,
            created_at: now,
            first_seen_at: now,
            last_seen_at: now,
            updated_at: now,
        };
        self.state.project_room_proposals.push(record.clone());
        self.save()?;
        Ok(record)
    }

    pub fn record_project_room_proposal_prompt_sent_at(
        &mut self,
        proposal_id: &str,
        provider_account_id: Option<String>,
        prompt_conversation_id: Option<String>,
        prompt_message_id: Option<String>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<ProjectRoomProposalRecord>> {
        validate_present("proposal_id", proposal_id)?;
        let Some(record) = self
            .state
            .project_room_proposals
            .iter_mut()
            .find(|record| record.proposal_id == proposal_id)
        else {
            return Ok(None);
        };
        record.provider_account_id = provider_account_id.and_then(|value| present_owned(&value));
        record.prompt_conversation_id =
            prompt_conversation_id.and_then(|value| present_owned(&value));
        record.prompt_message_id = prompt_message_id.and_then(|value| present_owned(&value));
        record.status = ProjectRoomProposalStatus::Pending;
        record.updated_at = now;
        let record = record.clone();
        self.save()?;
        Ok(Some(record))
    }

    pub fn mark_project_room_proposal_prompt_failed_at(
        &mut self,
        proposal_id: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<ProjectRoomProposalRecord>> {
        validate_present("proposal_id", proposal_id)?;
        let Some(record) = self
            .state
            .project_room_proposals
            .iter_mut()
            .find(|record| record.proposal_id == proposal_id)
        else {
            return Ok(None);
        };
        record.status = ProjectRoomProposalStatus::PromptFailed;
        record.updated_at = now;
        let record = record.clone();
        self.save()?;
        Ok(Some(record))
    }

    pub fn lookup_project_room_proposal(
        &self,
        proposal_id: &str,
    ) -> anyhow::Result<Option<ProjectRoomProposalRecord>> {
        validate_present("proposal_id", proposal_id)?;
        Ok(self
            .state
            .project_room_proposals
            .iter()
            .find(|record| record.proposal_id == proposal_id)
            .cloned())
    }

    pub fn ignore_project_room_proposal_at(
        &mut self,
        proposal_id: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<ProjectRoomProposalRecord>> {
        validate_present("proposal_id", proposal_id)?;
        let Some(record) = self
            .state
            .project_room_proposals
            .iter_mut()
            .find(|record| record.proposal_id == proposal_id)
        else {
            return Ok(None);
        };
        record.status = ProjectRoomProposalStatus::Ignored;
        record.updated_at = now;
        let record = record.clone();
        self.save()?;
        Ok(Some(record))
    }

    pub fn complete_project_room_proposal_with_created_room_at(
        &mut self,
        proposal_id: &str,
        provider_account_id: String,
        provider_conversation_id: String,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<ProjectRoomProposalRecord>> {
        validate_present("proposal_id", proposal_id)?;
        validate_present("provider_account_id", &provider_account_id)?;
        validate_present("provider_conversation_id", &provider_conversation_id)?;
        let Some(record) = self
            .state
            .project_room_proposals
            .iter_mut()
            .find(|record| record.proposal_id == proposal_id)
        else {
            return Ok(None);
        };
        record.provider_account_id = Some(provider_account_id);
        record.created_room_id = Some(provider_conversation_id);
        record.status = ProjectRoomProposalStatus::RoomCreated;
        record.updated_at = now;
        let record = record.clone();
        self.save()?;
        Ok(Some(record))
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
        if let Some(record_index) = self
            .state
            .thread_session_bindings
            .iter()
            .position(|record| thread_binding_matches(record, &ThreadBindingQuery::from(&input)))
        {
            let record = &self.state.thread_session_bindings[record_index];
            if thread_session_binding_matches(record, &input) {
                return Ok(record.clone());
            }
            if thread_session_binding_can_adopt_route_hash(record, &input) {
                self.state.thread_session_bindings[record_index].route_binding_hash =
                    input.route_binding_hash.clone();
                let record = self.state.thread_session_bindings[record_index].clone();
                self.save()?;
                return Ok(record);
            }
            bail!("provider thread is already bound to a different source session or route");
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
            route_binding_hash: input.route_binding_hash,
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

    pub fn lookup_thread_session_for_source_session(
        &self,
        query: &SourceSessionThreadBindingQuery,
    ) -> anyhow::Result<Option<ThreadSessionBindingRecord>> {
        validate_source_session_thread_binding_query(query)?;
        Ok(self
            .state
            .thread_session_bindings
            .iter()
            .rev()
            .find(|record| source_session_thread_binding_matches(record, query))
            .cloned())
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
            owner_direct_chats: Vec::new(),
            direct_chat_project_bindings: Vec::new(),
            project_room_proposals: Vec::new(),
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

fn validate_project_room_proposal_input(input: &ProjectRoomProposalInput) -> anyhow::Result<()> {
    validate_present("provider_id", &input.provider_id)?;
    validate_present("provider_type", &input.provider_type)?;
    validate_project_path(&input.project_path)
}

fn validate_owner_direct_chat_input(input: &OwnerDirectChatInput) -> anyhow::Result<()> {
    validate_provider_binding_parts(
        &input.provider_id,
        &input.provider_type,
        &input.provider_account_id,
        &input.provider_conversation_id,
    )
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
    validate_present("source_session_id", &input.source_session_id)?;
    if let Some(route_binding_hash) = input.route_binding_hash.as_deref() {
        validate_present("route_binding_hash", route_binding_hash)?;
    }
    Ok(())
}

fn validate_source_session_thread_binding_query(
    query: &SourceSessionThreadBindingQuery,
) -> anyhow::Result<()> {
    validate_provider_binding_parts(
        &query.provider_id,
        &query.provider_type,
        &query.provider_account_id,
        &query.provider_conversation_id,
    )?;
    validate_project_path(&query.project_path)?;
    validate_present("source_id", &query.source_id)?;
    validate_present("source_type", &query.source_type)?;
    validate_present("source_session_id", &query.source_session_id)
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

fn present_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn project_room_proposal_id(provider_id: &str, project_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(project_path.as_bytes());
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("prp_{}", &hash[..24])
}

fn project_name_from_path(project_path: &str) -> String {
    Path::new(project_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(project_path)
        .to_string()
}

fn mark_project_room_proposal_bound_to_existing(
    state: &mut BridgeBindingLedgerFile,
    provider_id: &str,
    provider_type: &str,
    provider_account_id: &str,
    provider_conversation_id: &str,
    project_path: &str,
    now: DateTime<Utc>,
) {
    for proposal in state.project_room_proposals.iter_mut().filter(|proposal| {
        proposal.provider_id == provider_id
            && proposal.provider_type == provider_type
            && proposal.project_path == project_path
            && proposal.status != ProjectRoomProposalStatus::RoomCreated
    }) {
        proposal.status = ProjectRoomProposalStatus::BoundToExisting;
        proposal.provider_account_id = Some(provider_account_id.to_string());
        proposal.created_room_id = Some(provider_conversation_id.to_string());
        proposal.updated_at = now;
    }
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

fn owner_direct_chat_matches(record: &OwnerDirectChatRecord, input: &OwnerDirectChatInput) -> bool {
    record.provider_id == input.provider_id
        && record.provider_type == input.provider_type
        && record.provider_account_id == input.provider_account_id
        && record.provider_conversation_id == input.provider_conversation_id
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
    project_paths_share_tree(&record.project_path, &input.project_path)
        && record.source_id == input.source_id
        && record.source_type == input.source_type
        && record.source_session_id == input.source_session_id
        && record.route_binding_hash == input.route_binding_hash
}

fn thread_session_binding_can_adopt_route_hash(
    record: &ThreadSessionBindingRecord,
    input: &ThreadSessionBindingInput,
) -> bool {
    project_paths_share_tree(&record.project_path, &input.project_path)
        && record.source_id == input.source_id
        && record.source_type == input.source_type
        && record.source_session_id == input.source_session_id
        && record.route_binding_hash.is_none()
        && input.route_binding_hash.is_some()
}

fn source_session_thread_binding_matches(
    record: &ThreadSessionBindingRecord,
    query: &SourceSessionThreadBindingQuery,
) -> bool {
    record.provider_id == query.provider_id
        && record.provider_type == query.provider_type
        && record.provider_account_id == query.provider_account_id
        && record.provider_conversation_id == query.provider_conversation_id
        && project_paths_share_tree(&record.project_path, &query.project_path)
        && record.source_id == query.source_id
        && record.source_type == query.source_type
        && record.source_session_id == query.source_session_id
}

fn project_paths_share_tree(left: &str, right: &str) -> bool {
    let left = Path::new(left);
    let right = Path::new(right);
    left.starts_with(right) || right.starts_with(left)
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
            error.to_string().contains(
                "provider thread is already bound to a different source session or route"
            )
        );
        let binding = ledger
            .lookup_thread_session(&thread_query("thread-1"))
            .expect("thread binding should still exist");
        assert_eq!(binding.project_path, "/repo/agents-router");
        assert_eq!(binding.source_session_id, "session-1");
        assert_eq!(binding.route_binding_hash.as_deref(), Some("route-hash-1"));
    }

    #[test]
    fn thread_binding_is_idempotent_for_same_session_inside_project_tree() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        ledger
            .bind_thread_session_at(
                thread_session("thread-1", "/repo/agents-router", "session-1"),
                now,
            )
            .expect("thread should bind source session");

        let rebound = ledger
            .bind_thread_session_at(
                thread_session("thread-1", "/repo/agents-router/crate", "session-1"),
                now,
            )
            .expect("same source session inside the project tree should be idempotent");

        assert_eq!(rebound.project_path, "/repo/agents-router");
        assert_eq!(rebound.source_session_id, "session-1");
        assert_eq!(rebound.route_binding_hash.as_deref(), Some("route-hash-1"));
        assert_eq!(ledger.state.thread_session_bindings.len(), 1);
    }

    #[test]
    fn legacy_thread_binding_adopts_route_hash_for_same_session() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        let mut legacy = thread_session("thread-1", "/repo/agents-router", "session-1");
        legacy.route_binding_hash = None;
        ledger
            .bind_thread_session_at(legacy, now)
            .expect("legacy thread should bind without route hash");

        let rebound = ledger
            .bind_thread_session_at(
                thread_session("thread-1", "/repo/agents-router/crate", "session-1"),
                now,
            )
            .expect("same legacy source session should adopt the route hash");

        assert_eq!(rebound.project_path, "/repo/agents-router");
        assert_eq!(rebound.source_session_id, "session-1");
        assert_eq!(rebound.route_binding_hash.as_deref(), Some("route-hash-1"));
        assert_eq!(ledger.state.thread_session_bindings.len(), 1);
    }

    #[test]
    fn provider_project_room_status_lists_only_connected_bindings() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        ledger
            .connect_room_project_at(room_project("room-1", "/repo/agents-router"), now)
            .expect("room should bind project");
        ledger
            .connect_room_project_at(room_project("room-2", "/repo/agent-transport-system"), now)
            .expect("second room should bind project");
        ledger
            .disconnect_room_project_at(room_project("room-2", "/repo/agent-transport-system"), now)
            .expect("second room should disconnect");

        let bindings = ledger
            .connected_room_project_bindings_for_provider("lark-personal-agent", "feishu_lark")
            .expect("provider binding status should load");

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].provider_conversation_id, "room-1");
        assert_eq!(bindings[0].project_path, "/repo/agents-router");
    }

    #[test]
    fn owner_direct_chat_is_remembered_for_provider() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();

        ledger
            .remember_owner_direct_chat_at(owner_direct_chat("oc_owner_direct"), now)
            .expect("owner direct chat should be remembered");

        let owner = ledger
            .owner_direct_chat_for_provider("lark-personal-agent", "feishu_lark")
            .expect("owner direct chat lookup should succeed")
            .expect("owner direct chat should exist");
        assert_eq!(owner.provider_conversation_id, "oc_owner_direct");
    }

    #[test]
    fn legacy_direct_chat_project_binding_can_seed_owner_direct_chat() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().join("bridge-bindings.json");
        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "room_project_bindings": [],
                "direct_chat_project_bindings": [
                    {
                        "provider_id": "lark-personal-agent",
                        "provider_type": "feishu_lark",
                        "provider_account_id": "tenant-1",
                        "provider_conversation_id": "oc_legacy_direct",
                        "project_path": "/tmp/agents-router-smoke-project",
                        "status": "disconnected",
                        "created_at": "2026-05-31T01:02:03Z",
                        "updated_at": "2026-05-31T01:02:03Z"
                    }
                ],
                "project_room_proposals": [],
                "thread_session_bindings": []
            }"#,
        )
        .expect("legacy ledger fixture should write");
        let ledger = BridgeBindingLedger::load(path).expect("legacy ledger should load");

        let owner = ledger
            .owner_direct_chat_for_provider("lark-personal-agent", "feishu_lark")
            .expect("owner direct chat lookup should succeed")
            .expect("legacy direct chat should seed owner direct chat");

        assert_eq!(owner.provider_conversation_id, "oc_legacy_direct");
    }

    #[test]
    fn project_room_proposal_prompts_once_until_user_decides() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        let input = project_room_proposal("/repo/agents-router");

        let first = ledger
            .begin_project_room_proposal_prompt_at(input.clone(), now)
            .expect("first proposal should start");
        let ProjectRoomProposalPromptDecision::Prompt(record) = first else {
            panic!("first proposal should ask the user");
        };
        assert_eq!(record.project_name, "agents-router");

        let second = ledger
            .begin_project_room_proposal_prompt_at(input, now)
            .expect("second proposal should load");
        assert_eq!(
            second,
            ProjectRoomProposalPromptDecision::Skip(ProjectRoomProposalSkipReason::AlreadyPending)
        );
    }

    #[test]
    fn ignored_project_room_proposal_does_not_prompt_again() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        let input = project_room_proposal("/repo/agents-router");
        let ProjectRoomProposalPromptDecision::Prompt(record) = ledger
            .begin_project_room_proposal_prompt_at(input.clone(), now)
            .expect("proposal should start")
        else {
            panic!("proposal should ask the user");
        };
        ledger
            .ignore_project_room_proposal_at(&record.proposal_id, now)
            .expect("proposal should be ignored");

        assert_eq!(
            ledger
                .begin_project_room_proposal_prompt_at(input, now)
                .expect("ignored proposal should load"),
            ProjectRoomProposalPromptDecision::Skip(ProjectRoomProposalSkipReason::Ignored)
        );
    }

    #[test]
    fn manual_project_room_proposal_reopens_ignored_project() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        let input = project_room_proposal("/repo/agents-router");
        let ProjectRoomProposalPromptDecision::Prompt(ignored) = ledger
            .begin_project_room_proposal_prompt_at(input.clone(), now)
            .expect("proposal should begin")
        else {
            panic!("proposal should ask the user");
        };
        ledger
            .ignore_project_room_proposal_at(&ignored.proposal_id, now)
            .expect("proposal should ignore");

        let reopened = ledger
            .begin_manual_project_room_proposal_prompt_at(input, now)
            .expect("manual proposal should reopen");

        assert_eq!(reopened.proposal_id, ignored.proposal_id);
        assert_eq!(reopened.status, ProjectRoomProposalStatus::Pending);
        assert_eq!(
            ledger
                .lookup_project_room_proposal(&ignored.proposal_id)
                .expect("proposal lookup should load")
                .expect("proposal should exist")
                .status,
            ProjectRoomProposalStatus::Pending
        );
    }

    #[test]
    fn binding_room_marks_matching_pending_project_room_proposal_bound_to_existing() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let now = test_time();
        let ProjectRoomProposalPromptDecision::Prompt(record) = ledger
            .begin_project_room_proposal_prompt_at(
                project_room_proposal("/repo/agents-router"),
                now,
            )
            .expect("proposal should start")
        else {
            panic!("proposal should ask the user");
        };

        ledger
            .connect_room_project_at(room_project("room-1", "/repo/agents-router"), now)
            .expect("room should bind project");
        let proposal = ledger
            .lookup_project_room_proposal(&record.proposal_id)
            .expect("proposal lookup should succeed")
            .expect("proposal should still exist");

        assert_eq!(proposal.status, ProjectRoomProposalStatus::BoundToExisting);
        assert_eq!(proposal.created_room_id.as_deref(), Some("room-1"));
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

    fn owner_direct_chat(provider_conversation_id: &str) -> OwnerDirectChatInput {
        OwnerDirectChatInput {
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
            route_binding_hash: Some("route-hash-1".to_string()),
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

    fn project_room_proposal(project_path: &str) -> ProjectRoomProposalInput {
        ProjectRoomProposalInput {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            project_path: project_path.to_string(),
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 31, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
