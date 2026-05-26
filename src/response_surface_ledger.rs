use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::paths::response_surface_ledger_path;
use crate::provider_catalog::ProviderMode;

const LEDGER_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ResponseSurfaceLedger {
    state_path: Option<PathBuf>,
    state: ResponseSurfaceLedgerFile,
}

#[derive(Debug, Clone)]
pub struct ResponseSurfaceLedgerStore {
    state_path: PathBuf,
    lock: Arc<AsyncMutex<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ResponseSurfaceLedgerFile {
    schema_version: u32,
    surfaces: Vec<ResponseSurfaceRecord>,
    inbound_events: Vec<InboundEventDedupRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    continuation_turns: Vec<ResponseSurfaceContinuationTurnRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseSurfaceRecord {
    pub surface_id: String,
    pub signal_id: String,
    pub delivery_id: String,
    pub source_id: String,
    pub source_type: String,
    pub source_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_turn_id: Option<String>,
    pub provider_id: String,
    pub provider_type: String,
    pub provider_mode: ProviderMode,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_message_id: String,
    pub provider_thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_binding_hash: Option<String>,
    pub status: ResponseSurfaceStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseSurfaceStatus {
    Open,
    Closed,
    // Legacy state from the earlier time-window model. It is treated as open
    // for lookup because response surfaces no longer expire by time.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewResponseSurface {
    pub signal_id: String,
    pub delivery_id: String,
    pub source_id: String,
    pub source_type: String,
    pub source_session_id: String,
    pub source_turn_id: Option<String>,
    pub provider_id: String,
    pub provider_type: String,
    pub provider_mode: ProviderMode,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_message_id: String,
    pub provider_thread_id: String,
    pub route_binding_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseSurfaceLookupQuery {
    pub provider_id: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseSurfaceLookupResult {
    Hit(ResponseSurfaceLookupRecord),
    Miss,
    Closed { surface_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseSurfaceLookupRecord {
    pub surface_id: String,
    pub signal_id: String,
    pub delivery_id: String,
    pub source_id: String,
    pub source_type: String,
    pub source_session_id: String,
    pub source_turn_id: Option<String>,
    pub provider_id: String,
    pub provider_type: String,
    pub provider_mode: ProviderMode,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_message_id: String,
    pub provider_thread_id: String,
    pub route_binding_hash: Option<String>,
    pub status: ResponseSurfaceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundEventDedupInput {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_event_id: String,
    pub surface_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundEventClaimDecision {
    Claimed {
        provider_event_id_hash: String,
    },
    AlreadyProcessing {
        provider_event_id_hash: String,
        surface_id: String,
        status: InboundEventDedupStatus,
    },
    DuplicateProcessed {
        provider_event_id_hash: String,
        surface_id: String,
        status: InboundEventDedupStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundEventRecordDecision {
    Recorded {
        provider_event_id_hash: String,
        status: InboundEventDedupStatus,
    },
    Duplicate {
        provider_event_id_hash: String,
        surface_id: String,
        status: InboundEventDedupStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundEventDiagnosticRecord {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_event_id_hash: String,
    pub surface_id: String,
    pub status: InboundEventDedupStatus,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InboundEventDedupRecord {
    provider_id: String,
    provider_type: String,
    provider_event_id_hash: String,
    surface_id: String,
    status: InboundEventDedupStatus,
    received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ResponseSurfaceContinuationTurnRecord {
    surface_id: String,
    source_session_id: String,
    source_turn_id: String,
    recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseSurfaceContinuationTurnInput {
    pub surface_id: String,
    pub source_session_id: String,
    pub source_turn_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseSurfaceContinuationTurnIndex {
    turns: BTreeSet<ResponseSurfaceContinuationTurnKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResponseSurfaceContinuationTurnKey {
    source_session_id: String,
    source_turn_id: String,
}

impl ResponseSurfaceContinuationTurnIndex {
    pub fn contains(&self, source_session_id: &str, source_turn_id: &str) -> bool {
        self.turns.contains(&ResponseSurfaceContinuationTurnKey {
            source_session_id: source_session_id.to_string(),
            source_turn_id: source_turn_id.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InboundEventDedupStatus {
    #[serde(alias = "processing")]
    ClaimedBeforeSubmit,
    SubmittedPossible,
    Processed,
    FailedNotified,
    SubmittedUnknownNotified,
}

impl InboundEventDedupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaimedBeforeSubmit => "claimed_before_submit",
            Self::SubmittedPossible => "submitted_possible",
            Self::Processed => "processed",
            Self::FailedNotified => "failed_notified",
            Self::SubmittedUnknownNotified => "submitted_unknown_notified",
        }
    }

    fn is_processing(self) -> bool {
        matches!(self, Self::ClaimedBeforeSubmit | Self::SubmittedPossible)
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Processed | Self::FailedNotified | Self::SubmittedUnknownNotified
        )
    }
}

impl Default for ResponseSurfaceLedgerFile {
    fn default() -> Self {
        Self {
            schema_version: LEDGER_SCHEMA_VERSION,
            surfaces: Vec::new(),
            inbound_events: Vec::new(),
            continuation_turns: Vec::new(),
        }
    }
}

impl ResponseSurfaceLedger {
    pub fn in_memory() -> Self {
        Self {
            state_path: None,
            state: ResponseSurfaceLedgerFile::default(),
        }
    }

    pub fn load_default() -> anyhow::Result<Self> {
        Self::load(response_surface_ledger_path()?)
    }

    pub fn load(state_path: PathBuf) -> anyhow::Result<Self> {
        let state = load_ledger_file(&state_path)?;
        Ok(Self {
            state_path: Some(state_path),
            state,
        })
    }

    pub fn create_surface_at(
        &mut self,
        input: NewResponseSurface,
        now: DateTime<Utc>,
    ) -> anyhow::Result<ResponseSurfaceRecord> {
        input.validate()?;

        // Response surfaces have no time window. An open surface stays
        // lookupable until it is explicitly closed.
        if let Some(existing) = self
            .state
            .surfaces
            .iter()
            .find(|record| active_surface_has_same_provider_thread(record, &input))
        {
            if surface_create_is_idempotent(existing, &input) {
                return Ok(existing.clone());
            }

            anyhow::bail!(
                "provider thread is already bound to response surface `{}`",
                existing.surface_id
            );
        }

        let record = ResponseSurfaceRecord {
            surface_id: Uuid::new_v4().to_string(),
            signal_id: input.signal_id,
            delivery_id: input.delivery_id,
            source_id: input.source_id,
            source_type: input.source_type,
            source_session_id: input.source_session_id,
            source_turn_id: input
                .source_turn_id
                .filter(|value| !value.trim().is_empty()),
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            provider_mode: input.provider_mode,
            provider_account_id: input.provider_account_id,
            provider_conversation_id: input.provider_conversation_id,
            provider_message_id: input.provider_message_id,
            provider_thread_id: input.provider_thread_id,
            route_binding_hash: input
                .route_binding_hash
                .filter(|value| !value.trim().is_empty()),
            status: ResponseSurfaceStatus::Open,
            created_at: now,
        };

        self.state.surfaces.push(record.clone());
        self.save()?;
        Ok(record)
    }

    pub fn lookup_surface_at(
        &mut self,
        query: ResponseSurfaceLookupQuery,
        _now: DateTime<Utc>,
    ) -> anyhow::Result<ResponseSurfaceLookupResult> {
        query.validate()?;

        let Some(record) = self
            .state
            .surfaces
            .iter()
            .rev()
            .find(|record| surface_matches_query(record, &query))
        else {
            return Ok(ResponseSurfaceLookupResult::Miss);
        };

        Ok(match record.status {
            ResponseSurfaceStatus::Open | ResponseSurfaceStatus::Expired => {
                ResponseSurfaceLookupResult::Hit(ResponseSurfaceLookupRecord::from_surface(record))
            }
            ResponseSurfaceStatus::Closed => ResponseSurfaceLookupResult::Closed {
                surface_id: record.surface_id.clone(),
            },
        })
    }

    pub fn close_surface(&mut self, surface_id: &str) -> anyhow::Result<bool> {
        ensure_present("surface_id", surface_id)?;
        let mut changed = false;
        if let Some(record) = self
            .state
            .surfaces
            .iter_mut()
            .find(|record| record.surface_id == surface_id)
            && record.status == ResponseSurfaceStatus::Open
        {
            record.status = ResponseSurfaceStatus::Closed;
            changed = true;
        }

        if changed {
            self.save()?;
        }
        Ok(changed)
    }

    pub fn claim_inbound_event_at(
        &mut self,
        input: InboundEventDedupInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<InboundEventClaimDecision> {
        input.validate()?;

        let provider_event_id_hash = provider_event_id_hash(
            &input.provider_type,
            &input.provider_id,
            &input.provider_account_id,
            &input.provider_conversation_id,
            &input.provider_event_id,
        );

        if let Some(existing) = self.state.inbound_events.iter().find(|record| {
            record.provider_id == input.provider_id
                && record.provider_type == input.provider_type
                && record.provider_event_id_hash == provider_event_id_hash
        }) {
            let decision = match existing.status {
                status if status.is_processing() => InboundEventClaimDecision::AlreadyProcessing {
                    provider_event_id_hash,
                    surface_id: existing.surface_id.clone(),
                    status,
                },
                status if status.is_terminal() => InboundEventClaimDecision::DuplicateProcessed {
                    provider_event_id_hash,
                    surface_id: existing.surface_id.clone(),
                    status,
                },
                status => unreachable!("unclassified inbound event status: {status:?}"),
            };

            return Ok(decision);
        }

        // A claim means the local machine has accepted the event and is still
        // processing it. It is not an agent execution timeout.
        self.state.inbound_events.push(InboundEventDedupRecord {
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            provider_event_id_hash: provider_event_id_hash.clone(),
            surface_id: input.surface_id,
            status: InboundEventDedupStatus::ClaimedBeforeSubmit,
            received_at: now,
        });
        self.save()?;

        Ok(InboundEventClaimDecision::Claimed {
            provider_event_id_hash,
        })
    }

    pub fn record_processed_inbound_event_at(
        &mut self,
        input: InboundEventDedupInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<InboundEventRecordDecision> {
        self.record_inbound_event_status_at(input, InboundEventDedupStatus::Processed, now)
    }

    pub fn record_submitted_possible_inbound_event_at(
        &mut self,
        input: InboundEventDedupInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<InboundEventRecordDecision> {
        self.record_inbound_event_status_at(input, InboundEventDedupStatus::SubmittedPossible, now)
    }

    pub fn record_failed_notified_inbound_event_at(
        &mut self,
        input: InboundEventDedupInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<InboundEventRecordDecision> {
        self.record_inbound_event_status_at(input, InboundEventDedupStatus::FailedNotified, now)
    }

    pub fn record_submitted_unknown_notified_inbound_event_at(
        &mut self,
        input: InboundEventDedupInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<InboundEventRecordDecision> {
        self.record_inbound_event_status_at(
            input,
            InboundEventDedupStatus::SubmittedUnknownNotified,
            now,
        )
    }

    pub fn release_inbound_event_claim_at(
        &mut self,
        input: InboundEventDedupInput,
    ) -> anyhow::Result<bool> {
        input.validate()?;
        let provider_event_id_hash = provider_event_id_hash(
            &input.provider_type,
            &input.provider_id,
            &input.provider_account_id,
            &input.provider_conversation_id,
            &input.provider_event_id,
        );

        let Some(position) = self.state.inbound_events.iter().position(|record| {
            record.provider_id == input.provider_id
                && record.provider_type == input.provider_type
                && record.provider_event_id_hash == provider_event_id_hash
                && record.surface_id == input.surface_id
                && record.status == InboundEventDedupStatus::ClaimedBeforeSubmit
        }) else {
            return Ok(false);
        };

        self.state.inbound_events.remove(position);
        self.save()?;
        Ok(true)
    }

    pub fn record_response_surface_continuation_turn_at(
        &mut self,
        input: ResponseSurfaceContinuationTurnInput,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        input.validate()?;

        if let Some(existing) = self
            .state
            .continuation_turns
            .iter()
            .find(|record| continuation_turn_matches(record, &input))
        {
            ensure!(
                existing.surface_id == input.surface_id,
                "response surface continuation turn is already associated with another surface"
            );
            return Ok(false);
        }

        self.state
            .continuation_turns
            .push(ResponseSurfaceContinuationTurnRecord {
                surface_id: input.surface_id,
                source_session_id: input.source_session_id,
                source_turn_id: input.source_turn_id,
                recorded_at: now,
            });
        self.save()?;
        Ok(true)
    }

    pub fn response_surface_continuation_turn_index(&self) -> ResponseSurfaceContinuationTurnIndex {
        ResponseSurfaceContinuationTurnIndex {
            turns: self
                .state
                .continuation_turns
                .iter()
                .map(|record| ResponseSurfaceContinuationTurnKey {
                    source_session_id: record.source_session_id.clone(),
                    source_turn_id: record.source_turn_id.clone(),
                })
                .collect(),
        }
    }

    pub fn unfinished_inbound_events(&self) -> Vec<InboundEventDiagnosticRecord> {
        self.state
            .inbound_events
            .iter()
            .filter(|record| record.status.is_processing())
            .map(|record| InboundEventDiagnosticRecord {
                provider_id: record.provider_id.clone(),
                provider_type: record.provider_type.clone(),
                provider_event_id_hash: record.provider_event_id_hash.clone(),
                surface_id: record.surface_id.clone(),
                status: record.status,
                received_at: record.received_at,
            })
            .collect()
    }

    fn record_inbound_event_status_at(
        &mut self,
        input: InboundEventDedupInput,
        status: InboundEventDedupStatus,
        now: DateTime<Utc>,
    ) -> anyhow::Result<InboundEventRecordDecision> {
        input.validate()?;
        let provider_event_id_hash = provider_event_id_hash(
            &input.provider_type,
            &input.provider_id,
            &input.provider_account_id,
            &input.provider_conversation_id,
            &input.provider_event_id,
        );

        if let Some(existing) = self.state.inbound_events.iter_mut().find(|record| {
            record.provider_id == input.provider_id
                && record.provider_type == input.provider_type
                && record.provider_event_id_hash == provider_event_id_hash
        }) {
            ensure!(
                existing.surface_id == input.surface_id,
                "inbound event is already associated with another response surface"
            );

            if existing.status == status || existing.status.is_terminal() {
                return Ok(InboundEventRecordDecision::Duplicate {
                    provider_event_id_hash,
                    surface_id: existing.surface_id.clone(),
                    status: existing.status,
                });
            }

            // Claim status is a durable execution boundary, not an agent
            // timeout. Once a submit may have reached the agent, the router
            // must not release the event and guess a retry.
            existing.status = status;
            self.save()?;

            return Ok(InboundEventRecordDecision::Recorded {
                provider_event_id_hash,
                status,
            });
        }

        ensure!(
            status == InboundEventDedupStatus::Processed,
            "inbound event must be claimed before recording `{}`",
            status.as_str()
        );

        // Processed events are retained indefinitely so the same provider event
        // cannot execute again after a later platform retry or local restart.
        self.state.inbound_events.push(InboundEventDedupRecord {
            provider_id: input.provider_id,
            provider_type: input.provider_type,
            provider_event_id_hash: provider_event_id_hash.clone(),
            surface_id: input.surface_id,
            status,
            received_at: now,
        });
        self.save()?;

        Ok(InboundEventRecordDecision::Recorded {
            provider_event_id_hash,
            status,
        })
    }

    fn save(&self) -> anyhow::Result<()> {
        let Some(state_path) = &self.state_path else {
            return Ok(());
        };
        save_ledger_file(state_path, &self.state)
    }
}

impl ResponseSurfaceLedgerStore {
    pub fn load_default() -> anyhow::Result<Self> {
        Self::new(response_surface_ledger_path()?)
    }

    pub fn new(state_path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create response surface ledger directory `{}`",
                    parent.display()
                )
            })?;
        }

        Ok(Self {
            state_path,
            lock: Arc::new(AsyncMutex::new(())),
        })
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub async fn update<R>(
        &self,
        operation: impl FnOnce(&mut ResponseSurfaceLedger) -> anyhow::Result<R>,
    ) -> anyhow::Result<R> {
        let _guard = self.lock.lock().await;
        let mut ledger = ResponseSurfaceLedger::load(self.state_path.clone())?;
        operation(&mut ledger)
    }

    pub async fn unfinished_inbound_events(
        &self,
    ) -> anyhow::Result<Vec<InboundEventDiagnosticRecord>> {
        self.update(|ledger| Ok(ledger.unfinished_inbound_events()))
            .await
    }

    pub async fn response_surface_continuation_turn_index(
        &self,
    ) -> anyhow::Result<ResponseSurfaceContinuationTurnIndex> {
        self.update(|ledger| Ok(ledger.response_surface_continuation_turn_index()))
            .await
    }
}

impl NewResponseSurface {
    fn validate(&self) -> anyhow::Result<()> {
        ensure_present("signal_id", &self.signal_id)?;
        ensure_present("delivery_id", &self.delivery_id)?;
        ensure_present("source_id", &self.source_id)?;
        ensure_present("source_type", &self.source_type)?;
        ensure_present("source_session_id", &self.source_session_id)?;
        ensure_present("provider_id", &self.provider_id)?;
        ensure_present("provider_type", &self.provider_type)?;
        ensure_present("provider_account_id", &self.provider_account_id)?;
        ensure_present("provider_conversation_id", &self.provider_conversation_id)?;
        ensure_present("provider_message_id", &self.provider_message_id)?;
        ensure_present("provider_thread_id", &self.provider_thread_id)?;
        Ok(())
    }
}

impl ResponseSurfaceLookupQuery {
    fn validate(&self) -> anyhow::Result<()> {
        ensure_present("provider_id", &self.provider_id)?;
        ensure_present("provider_account_id", &self.provider_account_id)?;
        ensure_present("provider_conversation_id", &self.provider_conversation_id)?;
        ensure_present("provider_thread_id", &self.provider_thread_id)?;
        Ok(())
    }
}

impl InboundEventDedupInput {
    fn validate(&self) -> anyhow::Result<()> {
        ensure_present("provider_id", &self.provider_id)?;
        ensure_present("provider_type", &self.provider_type)?;
        ensure_present("provider_account_id", &self.provider_account_id)?;
        ensure_present("provider_conversation_id", &self.provider_conversation_id)?;
        ensure_present("provider_event_id", &self.provider_event_id)?;
        ensure_present("surface_id", &self.surface_id)?;
        Ok(())
    }
}

impl ResponseSurfaceContinuationTurnInput {
    fn validate(&self) -> anyhow::Result<()> {
        ensure_present("surface_id", &self.surface_id)?;
        ensure_present("source_session_id", &self.source_session_id)?;
        ensure_present("source_turn_id", &self.source_turn_id)?;
        Ok(())
    }
}

impl ResponseSurfaceLookupRecord {
    fn from_surface(record: &ResponseSurfaceRecord) -> Self {
        Self {
            surface_id: record.surface_id.clone(),
            signal_id: record.signal_id.clone(),
            delivery_id: record.delivery_id.clone(),
            source_id: record.source_id.clone(),
            source_type: record.source_type.clone(),
            source_session_id: record.source_session_id.clone(),
            source_turn_id: record.source_turn_id.clone(),
            provider_id: record.provider_id.clone(),
            provider_type: record.provider_type.clone(),
            provider_mode: record.provider_mode,
            provider_account_id: record.provider_account_id.clone(),
            provider_conversation_id: record.provider_conversation_id.clone(),
            provider_message_id: record.provider_message_id.clone(),
            provider_thread_id: record.provider_thread_id.clone(),
            route_binding_hash: record.route_binding_hash.clone(),
            status: record.status,
        }
    }
}

fn surface_matches_query(
    record: &ResponseSurfaceRecord,
    query: &ResponseSurfaceLookupQuery,
) -> bool {
    record.provider_id == query.provider_id
        && record.provider_account_id == query.provider_account_id
        && record.provider_conversation_id == query.provider_conversation_id
        && record.provider_thread_id == query.provider_thread_id
}

fn active_surface_has_same_provider_thread(
    record: &ResponseSurfaceRecord,
    input: &NewResponseSurface,
) -> bool {
    record.status != ResponseSurfaceStatus::Closed
        && record.provider_id == input.provider_id
        && record.provider_account_id == input.provider_account_id
        && record.provider_conversation_id == input.provider_conversation_id
        && record.provider_thread_id == input.provider_thread_id
}

fn surface_create_is_idempotent(
    record: &ResponseSurfaceRecord,
    input: &NewResponseSurface,
) -> bool {
    record.signal_id == input.signal_id
        && record.source_id == input.source_id
        && record.source_type == input.source_type
        && record.source_session_id == input.source_session_id
        && record.source_turn_id.as_deref() == normalized_optional_value(&input.source_turn_id)
        && record.provider_type == input.provider_type
        && record.provider_mode == input.provider_mode
        && record.provider_message_id == input.provider_message_id
        && record.route_binding_hash.as_deref()
            == normalized_optional_value(&input.route_binding_hash)
}

fn normalized_optional_value(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.trim().is_empty())
}

fn continuation_turn_matches(
    record: &ResponseSurfaceContinuationTurnRecord,
    input: &ResponseSurfaceContinuationTurnInput,
) -> bool {
    record.source_session_id == input.source_session_id
        && record.source_turn_id == input.source_turn_id
}

fn ensure_present(field: &'static str, value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.trim().is_empty(),
        "response surface ledger `{field}` must be present"
    );
    Ok(())
}

fn provider_event_id_hash(
    provider_type: &str,
    provider_id: &str,
    provider_account_id: &str,
    provider_conversation_id: &str,
    provider_event_id: &str,
) -> String {
    let mut hasher = Sha256::new();
    // The event id must come from the provider's stable event identity. The
    // conversation scope prevents providers with per-conversation ids from
    // deduping unrelated replies together.
    for value in [
        provider_type,
        provider_id,
        provider_account_id,
        provider_conversation_id,
        provider_event_id,
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_ledger_file(path: &Path) -> anyhow::Result<ResponseSurfaceLedgerFile> {
    if !path.exists() {
        return Ok(ResponseSurfaceLedgerFile::default());
    }

    let raw = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read response surface ledger `{}`",
            path.display()
        )
    })?;
    let state: ResponseSurfaceLedgerFile = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse response surface ledger `{}`",
            path.display()
        )
    })?;
    if state.schema_version != LEDGER_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported response surface ledger schema version `{}` in `{}`",
            state.schema_version,
            path.display()
        );
    }
    Ok(state)
}

fn save_ledger_file(path: &Path, state: &ResponseSurfaceLedgerFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create response surface ledger directory `{}`",
                parent.display()
            )
        })?;
    }

    let raw =
        serde_json::to_vec_pretty(state).context("failed to serialize response surface ledger")?;
    let temp_path = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ));
    fs::write(&temp_path, raw).with_context(|| {
        format!(
            "failed to write response surface ledger `{}`",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace response surface ledger `{}`",
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use serde_json::Value;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn creates_and_looks_up_open_surface_by_provider_thread() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let surface = ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");

        let lookup = ledger
            .lookup_surface_at(test_lookup_query(), now + Duration::minutes(1))
            .expect("lookup should succeed");

        assert!(is_opaque_uuid(&surface.surface_id));
        assert_eq!(
            lookup,
            ResponseSurfaceLookupResult::Hit(ResponseSurfaceLookupRecord {
                surface_id: surface.surface_id,
                signal_id: "signal-1".to_string(),
                delivery_id: "delivery-1".to_string(),
                source_id: "codex_desktop".to_string(),
                source_type: "codex_desktop".to_string(),
                source_session_id: "session-1".to_string(),
                source_turn_id: Some("turn-1".to_string()),
                provider_id: "slack-work".to_string(),
                provider_type: "slack".to_string(),
                provider_mode: ProviderMode::SlackApp,
                provider_account_id: "T123".to_string(),
                provider_conversation_id: "C123".to_string(),
                provider_message_id: "1716200000.000100".to_string(),
                provider_thread_id: "1716200000.000100".to_string(),
                route_binding_hash: Some("route-hash-1".to_string()),
                status: ResponseSurfaceStatus::Open,
            })
        );
    }

    #[test]
    fn duplicate_delivery_for_same_provider_thread_returns_existing_surface() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let first = ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");

        let mut retry = test_surface(now);
        retry.delivery_id = "delivery-retry".to_string();
        let second = ledger
            .create_surface_at(retry, now + Duration::seconds(1))
            .expect("duplicate delivery should be idempotent");

        assert_eq!(second.surface_id, first.surface_id);
        assert_eq!(second.delivery_id, "delivery-1");
        assert_eq!(ledger.state.surfaces.len(), 1);
    }

    #[test]
    fn same_provider_thread_cannot_be_bound_to_another_session() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let first = ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");

        let mut conflicting = test_surface(now);
        conflicting.signal_id = "signal-2".to_string();
        conflicting.source_session_id = "session-2".to_string();
        let error = ledger
            .create_surface_at(conflicting, now + Duration::seconds(1))
            .expect_err("same provider thread cannot point at another session");

        assert!(error.to_string().contains("already bound"));
        assert_eq!(ledger.state.surfaces.len(), 1);
        let lookup = ledger
            .lookup_surface_at(test_lookup_query(), now + Duration::seconds(2))
            .expect("lookup should succeed");
        assert!(matches!(
            lookup,
            ResponseSurfaceLookupResult::Hit(record)
                if record.surface_id == first.surface_id && record.source_session_id == "session-1"
        ));
    }

    #[test]
    fn lookup_requires_normalized_provider_thread_id_and_does_not_guess_from_message_id() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");

        let lookup = ledger
            .lookup_surface_at(
                ResponseSurfaceLookupQuery {
                    provider_thread_id: "different-thread".to_string(),
                    ..test_lookup_query()
                },
                now + Duration::minutes(1),
            )
            .expect("lookup should not fail");

        assert_eq!(lookup, ResponseSurfaceLookupResult::Miss);
    }

    #[test]
    fn rejects_surface_without_provider_thread_id() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let mut input = test_surface(now);
        input.provider_thread_id = String::new();

        let error = ledger
            .create_surface_at(input, now)
            .expect_err("missing provider_thread_id should fail fast");

        assert!(error.to_string().contains("provider_thread_id"));
    }

    #[test]
    fn old_surface_still_returns_hit_long_after_creation() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let surface = ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");

        let lookup = ledger
            .lookup_surface_at(test_lookup_query(), now + Duration::weeks(8))
            .expect("lookup should succeed");

        assert!(matches!(
            lookup,
            ResponseSurfaceLookupResult::Hit(record) if record.surface_id == surface.surface_id
        ));
    }

    #[test]
    fn legacy_expired_surface_status_still_returns_hit() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let surface = ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");
        ledger.state.surfaces[0].status = ResponseSurfaceStatus::Expired;

        let lookup = ledger
            .lookup_surface_at(test_lookup_query(), now + Duration::weeks(8))
            .expect("legacy expired lookup should succeed");

        assert!(matches!(
            lookup,
            ResponseSurfaceLookupResult::Hit(record)
                if record.surface_id == surface.surface_id
                    && record.status == ResponseSurfaceStatus::Expired
        ));
    }

    #[test]
    fn closed_surface_does_not_return_hit() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let surface = ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");
        assert!(
            ledger
                .close_surface(&surface.surface_id)
                .expect("close should succeed")
        );

        let lookup = ledger
            .lookup_surface_at(test_lookup_query(), now + Duration::minutes(1))
            .expect("lookup should succeed");

        assert_eq!(
            lookup,
            ResponseSurfaceLookupResult::Closed {
                surface_id: surface.surface_id
            }
        );
    }

    #[test]
    fn restart_recovers_surface_without_time_window() {
        let dir = tempdir().expect("temp dir should be created");
        let path = dir.path().join("response-surface-ledger.json");
        let now = test_time();
        let surface = {
            let mut ledger =
                ResponseSurfaceLedger::load(path.clone()).expect("ledger should load empty state");
            ledger
                .create_surface_at(test_surface(now), now)
                .expect("surface should be created")
        };

        let mut reloaded = ResponseSurfaceLedger::load(path).expect("ledger should reload");
        let lookup = reloaded
            .lookup_surface_at(test_lookup_query(), now + Duration::weeks(8))
            .expect("lookup should succeed after reload");

        assert!(matches!(
            lookup,
            ResponseSurfaceLookupResult::Hit(record) if record.surface_id == surface.surface_id
        ));
    }

    #[test]
    fn inbound_event_claim_does_not_mark_event_processed_until_completion() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let input = test_inbound_event(now);

        let claim = ledger
            .claim_inbound_event_at(input.clone(), now)
            .expect("first event should be claimed");
        let retry_while_processing = ledger
            .claim_inbound_event_at(input.clone(), now + Duration::seconds(1))
            .expect("in-flight event should be recognized");

        let InboundEventClaimDecision::Claimed {
            provider_event_id_hash,
        } = claim
        else {
            panic!("first event should be claimed");
        };
        assert_eq!(
            retry_while_processing,
            InboundEventClaimDecision::AlreadyProcessing {
                provider_event_id_hash: provider_event_id_hash.clone(),
                surface_id: "surface-1".to_string(),
                status: InboundEventDedupStatus::ClaimedBeforeSubmit,
            }
        );
        assert!(
            ledger
                .release_inbound_event_claim_at(input.clone())
                .expect("claim should release after failed handling")
        );

        let retry_after_release = ledger
            .claim_inbound_event_at(input.clone(), now + Duration::seconds(2))
            .expect("released claim should be claimable again");
        assert_eq!(
            retry_after_release,
            InboundEventClaimDecision::Claimed {
                provider_event_id_hash: provider_event_id_hash.clone(),
            }
        );

        let processed = ledger
            .record_processed_inbound_event_at(input.clone(), now + Duration::seconds(3))
            .expect("processed event should record");
        assert_eq!(
            processed,
            InboundEventRecordDecision::Recorded {
                provider_event_id_hash: provider_event_id_hash.clone(),
                status: InboundEventDedupStatus::Processed,
            }
        );

        let duplicate_after_processing = ledger
            .claim_inbound_event_at(input, now + Duration::weeks(8))
            .expect("processed event should be duplicate");
        assert_eq!(
            duplicate_after_processing,
            InboundEventClaimDecision::DuplicateProcessed {
                provider_event_id_hash,
                surface_id: "surface-1".to_string(),
                status: InboundEventDedupStatus::Processed,
            }
        );
    }

    #[test]
    fn submitted_possible_blocks_duplicate_without_timeout_or_release() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let input = test_inbound_event(now);
        ledger
            .claim_inbound_event_at(input.clone(), now)
            .expect("event should be claimed");

        let submitted = ledger
            .record_submitted_possible_inbound_event_at(input.clone(), now + Duration::seconds(1))
            .expect("submitted possible should record");
        let duplicate = ledger
            .claim_inbound_event_at(input.clone(), now + Duration::weeks(8))
            .expect("submitted possible event should stay in-flight");
        let released = ledger
            .release_inbound_event_claim_at(input)
            .expect("submitted possible should not release");

        assert!(matches!(
            submitted,
            InboundEventRecordDecision::Recorded {
                status: InboundEventDedupStatus::SubmittedPossible,
                ..
            }
        ));
        assert!(matches!(
            duplicate,
            InboundEventClaimDecision::AlreadyProcessing {
                status: InboundEventDedupStatus::SubmittedPossible,
                ..
            }
        ));
        assert!(!released);
    }

    #[test]
    fn records_response_surface_continuation_turn_index_without_content() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();

        let recorded = ledger
            .record_response_surface_continuation_turn_at(
                ResponseSurfaceContinuationTurnInput {
                    surface_id: "surface-1".to_string(),
                    source_session_id: "session-1".to_string(),
                    source_turn_id: "continuation-turn-1".to_string(),
                },
                now,
            )
            .expect("continuation turn should record");
        let duplicate = ledger
            .record_response_surface_continuation_turn_at(
                ResponseSurfaceContinuationTurnInput {
                    surface_id: "surface-1".to_string(),
                    source_session_id: "session-1".to_string(),
                    source_turn_id: "continuation-turn-1".to_string(),
                },
                now + Duration::seconds(1),
            )
            .expect("same continuation turn should be idempotent");
        let index = ledger.response_surface_continuation_turn_index();

        assert!(recorded);
        assert!(!duplicate);
        assert!(index.contains("session-1", "continuation-turn-1"));
        assert!(!index.contains("session-1", "manual-turn-2"));
        assert!(!index.contains("session-2", "continuation-turn-1"));

        let raw = serde_json::to_string_pretty(&ledger.state).expect("state should serialize");
        assert!(raw.contains("continuation_turns"));
        for forbidden in [
            "prompt",
            "answer",
            "reply_text",
            "raw_payload",
            "provider_message_body",
            "message_body",
        ] {
            assert!(
                !raw.contains(forbidden),
                "continuation turn index must not contain `{forbidden}`"
            );
        }
    }

    #[test]
    fn continuation_turn_cannot_point_to_two_surfaces() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        ledger
            .record_response_surface_continuation_turn_at(
                ResponseSurfaceContinuationTurnInput {
                    surface_id: "surface-1".to_string(),
                    source_session_id: "session-1".to_string(),
                    source_turn_id: "continuation-turn-1".to_string(),
                },
                now,
            )
            .expect("first continuation turn should record");

        let error = ledger
            .record_response_surface_continuation_turn_at(
                ResponseSurfaceContinuationTurnInput {
                    surface_id: "surface-2".to_string(),
                    source_session_id: "session-1".to_string(),
                    source_turn_id: "continuation-turn-1".to_string(),
                },
                now + Duration::seconds(1),
            )
            .expect_err("same continuation turn cannot point to another surface");

        assert!(error.to_string().contains("another surface"));
    }

    #[test]
    fn notified_terminal_statuses_dedup_forever() {
        for status in [
            InboundEventDedupStatus::FailedNotified,
            InboundEventDedupStatus::SubmittedUnknownNotified,
            InboundEventDedupStatus::Processed,
        ] {
            let mut ledger = ResponseSurfaceLedger::in_memory();
            let now = test_time();
            let input = test_inbound_event(now);
            ledger
                .claim_inbound_event_at(input.clone(), now)
                .expect("event should be claimed");

            match status {
                InboundEventDedupStatus::FailedNotified => {
                    ledger
                        .record_failed_notified_inbound_event_at(
                            input.clone(),
                            now + Duration::seconds(1),
                        )
                        .expect("failed notified should record");
                }
                InboundEventDedupStatus::SubmittedUnknownNotified => {
                    ledger
                        .record_submitted_possible_inbound_event_at(
                            input.clone(),
                            now + Duration::seconds(1),
                        )
                        .expect("submitted possible should record");
                    ledger
                        .record_submitted_unknown_notified_inbound_event_at(
                            input.clone(),
                            now + Duration::seconds(2),
                        )
                        .expect("submitted unknown notified should record");
                }
                InboundEventDedupStatus::Processed => {
                    ledger
                        .record_processed_inbound_event_at(
                            input.clone(),
                            now + Duration::seconds(1),
                        )
                        .expect("processed should record");
                }
                InboundEventDedupStatus::ClaimedBeforeSubmit
                | InboundEventDedupStatus::SubmittedPossible => unreachable!(),
            }

            let duplicate = ledger
                .claim_inbound_event_at(input, now + Duration::weeks(8))
                .expect("terminal event should dedup forever");
            assert!(matches!(
                duplicate,
                InboundEventClaimDecision::DuplicateProcessed {
                    status: duplicate_status,
                    ..
                } if duplicate_status == status
            ));
        }
    }

    #[test]
    fn inbound_event_processing_claim_does_not_expire_automatically() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let input = test_inbound_event(now);

        let first = ledger
            .claim_inbound_event_at(input.clone(), now)
            .expect("event should be claimed");
        let second = ledger
            .claim_inbound_event_at(input, now + Duration::weeks(8))
            .expect("processing claim should remain active until release or processed");

        assert!(matches!(first, InboundEventClaimDecision::Claimed { .. }));
        assert!(matches!(
            second,
            InboundEventClaimDecision::AlreadyProcessing { .. }
        ));
        assert_eq!(ledger.state.inbound_events.len(), 1);
    }

    #[test]
    fn inbound_event_dedup_hash_is_scoped_by_provider_account_without_persisting_raw_event_id() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let mut other_account_event = test_inbound_event(now);
        other_account_event.provider_account_id = "T999".to_string();

        let first = ledger
            .record_processed_inbound_event_at(test_inbound_event(now), now)
            .expect("first account event should record");
        let second = ledger
            .record_processed_inbound_event_at(other_account_event, now + Duration::seconds(1))
            .expect("other account event should record");

        assert!(matches!(first, InboundEventRecordDecision::Recorded { .. }));
        assert!(matches!(
            second,
            InboundEventRecordDecision::Recorded { .. }
        ));

        let raw = serde_json::to_string(&ledger.state).expect("state should serialize");
        assert!(!raw.contains("Ev123"));
        assert!(!raw.contains("provider_event_id\""));
        assert!(raw.contains("provider_event_id_hash"));
    }

    #[test]
    fn inbound_event_dedup_hash_is_scoped_by_provider_conversation() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        let mut other_conversation_event = test_inbound_event(now);
        other_conversation_event.provider_conversation_id = "C999".to_string();
        other_conversation_event.surface_id = "surface-2".to_string();

        let first = ledger
            .claim_inbound_event_at(test_inbound_event(now), now)
            .expect("first conversation event should claim");
        let second = ledger
            .claim_inbound_event_at(other_conversation_event, now + Duration::seconds(1))
            .expect("other conversation event should claim separately");

        let InboundEventClaimDecision::Claimed {
            provider_event_id_hash: first_hash,
        } = first
        else {
            panic!("first event should be claimed");
        };
        let InboundEventClaimDecision::Claimed {
            provider_event_id_hash: second_hash,
        } = second
        else {
            panic!("second conversation event should be claimed");
        };
        assert_ne!(first_hash, second_hash);
    }

    #[test]
    fn ledger_schema_does_not_store_content_or_product_support_facts() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        ledger
            .create_surface_at(test_surface(now), now)
            .expect("surface should be created");
        ledger
            .record_processed_inbound_event_at(test_inbound_event(now), now)
            .expect("event should record");

        let raw = serde_json::to_string_pretty(&ledger.state).expect("state should serialize");
        let value: Value = serde_json::from_str(&raw).expect("state should be JSON");

        assert_eq!(value["schema_version"], 1);
        for forbidden in [
            "prompt",
            "answer",
            "reply_text",
            "expires_at",
            "claim_expires_at",
            "processed_expires_at",
            "raw_inbound_payload",
            "raw_payload",
            "provider_message_body",
            "message_body",
            "rendered_payload",
            "agent_transcript",
            "controller_transcript",
            "tool_output",
            "controller_supported",
            "controller_kind",
            "continuation_status",
        ] {
            assert!(
                !raw.contains(forbidden),
                "ledger schema must not contain `{forbidden}`"
            );
        }
    }

    fn test_surface(_now: DateTime<Utc>) -> NewResponseSurface {
        NewResponseSurface {
            signal_id: "signal-1".to_string(),
            delivery_id: "delivery-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            provider_id: "slack-work".to_string(),
            provider_type: "slack".to_string(),
            provider_mode: ProviderMode::SlackApp,
            provider_account_id: "T123".to_string(),
            provider_conversation_id: "C123".to_string(),
            provider_message_id: "1716200000.000100".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
            route_binding_hash: Some("route-hash-1".to_string()),
        }
    }

    fn test_lookup_query() -> ResponseSurfaceLookupQuery {
        ResponseSurfaceLookupQuery {
            provider_id: "slack-work".to_string(),
            provider_account_id: "T123".to_string(),
            provider_conversation_id: "C123".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
        }
    }

    fn test_inbound_event(_now: DateTime<Utc>) -> InboundEventDedupInput {
        InboundEventDedupInput {
            provider_id: "slack-work".to_string(),
            provider_type: "slack".to_string(),
            provider_account_id: "T123".to_string(),
            provider_conversation_id: "C123".to_string(),
            provider_event_id: "Ev123".to_string(),
            surface_id: "surface-1".to_string(),
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }

    fn is_opaque_uuid(value: &str) -> bool {
        Uuid::parse_str(value).is_ok()
    }
}
