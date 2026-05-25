use std::collections::BTreeMap;
#[cfg(unix)]
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::time::{Duration, timeout};
use tracing::{info, warn};

use crate::config::{SourceType, ValidatedConfig};
use crate::delivery_safety::DeliverySafetyGuard;
use crate::paths::{IngressEndpoint, codex_sessions_dir_path};
use crate::response_surface_ledger::ResponseSurfaceLedgerStore;
use crate::router::{DeliveryReport, Provider, Router};
use crate::runtime::RuntimeState;
use crate::signal::{
    SignalEvent, SignalEventKind, SignalLifecycle, SignalLifecycleStatus, SignalLink,
    SignalWorkspace,
};
use crate::signal_builder::{SignalBuilder, SignalConversationDraft, SignalDraft};
use crate::sources::codex_cli;

const READINESS_PING_TIMEOUT: Duration = Duration::from_millis(500);

/// Cross-process local ingress payload.
///
/// This type is the wire DTO accepted from `agents-router emit` / `ingest` over
/// the local socket or named pipe. It is not the internal source draft model.
/// Convert it to `SignalDraft` only at the local ingress boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSignalEvent {
    pub source_id: String,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "LocalSignalAction::is_route_signal")]
    pub action: LocalSignalAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<SignalEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<SignalWorkspace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<LocalSignalConversation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<SignalLifecycle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<SignalLink>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl LocalSignalEvent {
    pub fn new(
        source_id: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            title: title.into(),
            body: body.into(),
            action: LocalSignalAction::RouteSignal,
            event: None,
            workspace: None,
            conversation: None,
            lifecycle: None,
            links: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn into_signal_draft(self, expected_source_type: SourceType) -> SignalDraft {
        let mut draft =
            SignalDraft::new(self.source_id, expected_source_type, self.title, self.body);
        if let Some(event) = self.event {
            draft.event = event;
        }
        draft.workspace = self.workspace;
        draft.lifecycle = self.lifecycle;
        draft.links = self.links;
        draft.metadata.extend(self.metadata);
        if let Some(conversation) = self.conversation {
            draft.conversation = Some(SignalConversationDraft {
                session_id: conversation.session_id,
                session_title: conversation.session_title,
                turn_id: conversation.turn_id,
                prompt: conversation.prompt,
                answer: conversation.answer,
                model: conversation.model,
            });
        }
        draft
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalSignalAction {
    #[default]
    RouteSignal,
    StoreSessionContext,
    StoreTurnStart,
}

impl LocalSignalAction {
    fn is_route_signal(action: &Self) -> bool {
        *action == Self::RouteSignal
    }
}

/// Conversation fields carried by the local ingress wire DTO.
///
/// Builder-level privacy gates are intentionally not applied here. This type
/// only preserves what the caller submitted; `SignalBuilder` decides what can
/// enter the final `Signal`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSignalConversation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Default)]
pub struct LocalIngressState {
    session_context: Mutex<BTreeMap<SessionContextKey, LocalSessionContext>>,
}

impl LocalIngressState {
    fn record_session_model(
        &self,
        source_id: String,
        session_id: String,
        model: String,
    ) -> anyhow::Result<()> {
        let mut session_context = self
            .session_context
            .lock()
            .map_err(|_| anyhow::anyhow!("local ingress session context lock is poisoned"))?;
        let context = session_context
            .entry(SessionContextKey {
                source_id,
                session_id,
            })
            .or_default();
        context.model = Some(model);
        Ok(())
    }

    fn record_turn_started_at(
        &self,
        source_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let mut session_context = self
            .session_context
            .lock()
            .map_err(|_| anyhow::anyhow!("local ingress session context lock is poisoned"))?;
        let context = session_context
            .entry(SessionContextKey {
                source_id,
                session_id,
            })
            .or_default();
        context.turn_started_at = Some(started_at);
        Ok(())
    }

    fn session_model(&self, source_id: &str, session_id: &str) -> anyhow::Result<Option<String>> {
        let session_context = self
            .session_context
            .lock()
            .map_err(|_| anyhow::anyhow!("local ingress session context lock is poisoned"))?;
        Ok(session_context
            .get(&SessionContextKey {
                source_id: source_id.to_string(),
                session_id: session_id.to_string(),
            })
            .and_then(|context| context.model.clone()))
    }

    fn take_turn_started_at(
        &self,
        source_id: &str,
        session_id: &str,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let mut session_context = self
            .session_context
            .lock()
            .map_err(|_| anyhow::anyhow!("local ingress session context lock is poisoned"))?;
        let key = SessionContextKey {
            source_id: source_id.to_string(),
            session_id: session_id.to_string(),
        };
        let Some(context) = session_context.get_mut(&key) else {
            return Ok(None);
        };

        let started_at = context.turn_started_at.take();
        if context.model.is_none() && context.turn_started_at.is_none() {
            session_context.remove(&key);
        }
        Ok(started_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SessionContextKey {
    source_id: String,
    session_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LocalSessionContext {
    model: Option<String>,
    turn_started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalIngressResponse {
    ok: bool,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delivery: Option<LocalIngressDeliveryReport>,
}

impl LocalIngressResponse {
    fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            delivery: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalIngressDeliveryReport {
    pub attempted: usize,
    pub succeeded: usize,
    pub suppressed: usize,
}

impl From<DeliveryReport> for LocalIngressDeliveryReport {
    fn from(report: DeliveryReport) -> Self {
        Self {
            attempted: report.attempted,
            succeeded: report.succeeded,
            suppressed: report.suppressed,
        }
    }
}

pub async fn submit_event(
    endpoint: &IngressEndpoint,
    event: &LocalSignalEvent,
) -> anyhow::Result<()> {
    submit_event_report(endpoint, event).await.map(|_| ())
}

pub async fn submit_event_report(
    endpoint: &IngressEndpoint,
    event: &LocalSignalEvent,
) -> anyhow::Result<Option<LocalIngressDeliveryReport>> {
    let request = serde_json::to_vec(event).context("failed to serialize local ingress event")?;
    submit_raw_request(endpoint, &request).await
}

pub async fn reset_delivery_safety(endpoint: &IngressEndpoint) -> anyhow::Result<()> {
    let request = serde_json::to_vec(&serde_json::json!({
        "kind": "delivery_safety_reset"
    }))
    .context("failed to serialize delivery safety reset request")?;
    submit_raw_request(endpoint, &request).await.map(|_| ())
}

async fn submit_raw_request(
    endpoint: &IngressEndpoint,
    request: &[u8],
) -> anyhow::Result<Option<LocalIngressDeliveryReport>> {
    match endpoint {
        #[cfg(unix)]
        IngressEndpoint::UnixSocket(socket_path) => {
            submit_raw_request_to_unix_socket(socket_path, request).await
        }
        #[cfg(windows)]
        IngressEndpoint::WindowsNamedPipe(pipe_name) => {
            submit_raw_request_to_windows_named_pipe(pipe_name, request).await
        }
    }
}

#[cfg(unix)]
async fn submit_raw_request_to_unix_socket(
    socket_path: &Path,
    request: &[u8],
) -> anyhow::Result<Option<LocalIngressDeliveryReport>> {
    let mut stream = UnixStream::connect(socket_path).await.with_context(|| {
        format!(
            "agents-router service is not running at `{}`; run `agents-router start` first",
            socket_path.display()
        )
    })?;

    stream
        .write_all(request)
        .await
        .context("failed to submit local ingress event")?;
    stream
        .shutdown()
        .await
        .context("failed to finish local ingress request")?;

    let mut raw_response = Vec::new();
    stream
        .read_to_end(&mut raw_response)
        .await
        .context("failed to read local ingress response")?;
    let response: LocalIngressResponse =
        serde_json::from_slice(&raw_response).context("failed to parse local ingress response")?;

    if response.ok {
        Ok(response.delivery)
    } else {
        anyhow::bail!(
            "agents-router service rejected local ingress event: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        )
    }
}

#[cfg(windows)]
async fn submit_raw_request_to_windows_named_pipe(
    pipe_name: &str,
    request: &[u8],
) -> anyhow::Result<Option<LocalIngressDeliveryReport>> {
    let mut stream = ClientOptions::new().open(pipe_name).with_context(|| {
        format!(
            "agents-router service is not running at `{pipe_name}`; run `agents-router start` first"
        )
    })?;

    stream
        .write_all(request)
        .await
        .context("failed to submit local ingress event")?;
    stream
        .shutdown()
        .await
        .context("failed to finish local ingress request")?;

    let mut raw_response = Vec::new();
    stream
        .read_to_end(&mut raw_response)
        .await
        .context("failed to read local ingress response")?;
    let response: LocalIngressResponse =
        serde_json::from_slice(&raw_response).context("failed to parse local ingress response")?;

    if response.ok {
        Ok(response.delivery)
    } else {
        anyhow::bail!(
            "agents-router service rejected local ingress event: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        )
    }
}

pub async fn serve(runtime: RuntimeState, endpoint: IngressEndpoint) -> anyhow::Result<()> {
    match &endpoint {
        #[cfg(unix)]
        IngressEndpoint::UnixSocket(socket_path) => serve_unix_socket(runtime, socket_path).await,
        #[cfg(windows)]
        IngressEndpoint::WindowsNamedPipe(pipe_name) => {
            serve_windows_named_pipe(runtime, pipe_name).await
        }
    }
}

#[cfg(unix)]
async fn serve_unix_socket(runtime: RuntimeState, socket_path: &Path) -> anyhow::Result<()> {
    prepare_socket_path(socket_path).await?;
    let listener = UnixListener::bind(socket_path).with_context(|| {
        format!(
            "failed to bind local ingress socket `{}`",
            socket_path.display()
        )
    })?;
    let _guard = SocketFileGuard::new(socket_path.to_path_buf());

    info!(
        socket.path = %socket_path.display(),
        event = "ingress.started",
    );

    let state = LocalIngressState::default();
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .context("failed to accept local ingress connection")?;
        if let Err(error) = handle_unix_connection(stream, &runtime, &state).await {
            warn!(
                error = %error,
                event = "ingress.client.failed",
            );
        }
    }
}

#[cfg(windows)]
async fn serve_windows_named_pipe(runtime: RuntimeState, pipe_name: &str) -> anyhow::Result<()> {
    info!(
        pipe.name = %pipe_name,
        event = "ingress.started",
    );

    let state = LocalIngressState::default();
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_name)
            .with_context(|| format!("failed to create local ingress named pipe `{pipe_name}`"))?;
        server
            .connect()
            .await
            .with_context(|| format!("failed to accept local ingress named pipe `{pipe_name}`"))?;

        if let Err(error) = handle_windows_pipe(server, &runtime, &state).await {
            warn!(
                error = %error,
                event = "ingress.client.failed",
            );
        }
    }
}

pub async fn route_event(
    config: &ValidatedConfig,
    providers: &[&dyn Provider],
    event: LocalSignalEvent,
) -> anyhow::Result<()> {
    let state = LocalIngressState::default();
    route_event_with_state(config, providers, &state, event).await
}

pub async fn route_event_with_state(
    config: &ValidatedConfig,
    providers: &[&dyn Provider],
    state: &LocalIngressState,
    event: LocalSignalEvent,
) -> anyhow::Result<()> {
    let codex_sessions_dir = codex_sessions_dir_path().ok();
    route_event_with_state_and_codex_sessions_dir(
        config,
        providers,
        state,
        event,
        codex_sessions_dir.as_deref(),
    )
    .await
    .map(|_| ())
}

async fn route_event_with_state_and_codex_sessions_dir(
    config: &ValidatedConfig,
    providers: &[&dyn Provider],
    state: &LocalIngressState,
    event: LocalSignalEvent,
    codex_sessions_dir: Option<&Path>,
) -> anyhow::Result<DeliveryReport> {
    route_event_with_state_and_codex_sessions_dir_and_safety(
        config,
        providers,
        state,
        event,
        codex_sessions_dir,
        None,
    )
    .await
}

async fn route_event_with_state_and_codex_sessions_dir_and_safety(
    config: &ValidatedConfig,
    providers: &[&dyn Provider],
    state: &LocalIngressState,
    event: LocalSignalEvent,
    codex_sessions_dir: Option<&Path>,
    delivery_safety: Option<&DeliverySafetyGuard>,
) -> anyhow::Result<DeliveryReport> {
    route_event_with_state_and_codex_sessions_dir_and_safety_and_response_surfaces(
        config,
        providers,
        state,
        event,
        codex_sessions_dir,
        delivery_safety,
        None,
    )
    .await
}

async fn route_event_with_state_and_codex_sessions_dir_and_safety_and_response_surfaces(
    config: &ValidatedConfig,
    providers: &[&dyn Provider],
    state: &LocalIngressState,
    event: LocalSignalEvent,
    codex_sessions_dir: Option<&Path>,
    delivery_safety: Option<&DeliverySafetyGuard>,
    response_surface_ledger: Option<&ResponseSurfaceLedgerStore>,
) -> anyhow::Result<DeliveryReport> {
    info!(
        source.id = %event.source_id,
        action = ?event.action,
        event = "ingress.received",
    );

    match event.action {
        LocalSignalAction::StoreSessionContext => {
            store_session_context(config, state, event)?;
            return Ok(DeliveryReport::default());
        }
        LocalSignalAction::StoreTurnStart => {
            store_turn_start(config, state, event)?;
            return Ok(DeliveryReport::default());
        }
        LocalSignalAction::RouteSignal => {}
    }

    if codex_cli_stop_event_is_shadowed_by_codex_desktop(config, &event, codex_sessions_dir) {
        return Ok(DeliveryReport::default());
    }

    let mut draft = create_signal_draft(config, event)?;
    apply_session_context(state, &mut draft)?;
    apply_turn_start_context(state, &mut draft)?;
    let signal = SignalBuilder::new(config).build(draft)?;
    info!(
        signal.id = %signal.id,
        source.id = %signal.source_id(),
        source.type = %signal.source_type(),
        event = "signal.created",
    );

    Router::new(config)
        .route_with_safety_and_response_surfaces(
            &signal,
            providers,
            delivery_safety,
            response_surface_ledger,
        )
        .await
        .map_err(Into::into)
}

fn codex_cli_stop_event_is_shadowed_by_codex_desktop(
    config: &ValidatedConfig,
    event: &LocalSignalEvent,
    codex_sessions_dir: Option<&Path>,
) -> bool {
    let Some(codex_sessions_dir) = codex_sessions_dir else {
        return false;
    };

    match codex_cli::local_stop_event_is_shadowed_by_codex_desktop(
        config,
        event,
        codex_sessions_dir,
    ) {
        Ok(true) => {
            info!(
                source.id = %event.source_id,
                event = "ingress.codex_cli_stop.ignored",
                reason = "codex_desktop_source_enabled",
            );
            true
        }
        Ok(false) => false,
        Err(error) => {
            warn!(
                source.id = %event.source_id,
                error = %error,
                event = "ingress.codex_cli_stop.origin_unresolved",
            );
            false
        }
    }
}

pub async fn route_event_with_runtime(
    runtime: &RuntimeState,
    state: &LocalIngressState,
    event: LocalSignalEvent,
) -> anyhow::Result<DeliveryReport> {
    let snapshot = runtime.current()?;
    let providers = snapshot.provider_refs();
    let delivery_safety = runtime.delivery_safety();
    let response_surface_ledger = runtime.response_surface_ledger();
    let codex_sessions_dir = codex_sessions_dir_path().ok();
    route_event_with_state_and_codex_sessions_dir_and_safety_and_response_surfaces(
        &snapshot.config,
        &providers,
        state,
        event,
        codex_sessions_dir.as_deref(),
        Some(&delivery_safety),
        Some(&response_surface_ledger),
    )
    .await
}

fn store_session_context(
    config: &ValidatedConfig,
    state: &LocalIngressState,
    event: LocalSignalEvent,
) -> anyhow::Result<()> {
    let source_id = event.source_id;
    validate_local_ingress_source(config, &source_id)?;
    let conversation = event
        .conversation
        .context("local ingress session context event missing `conversation`")?;
    let session_id = conversation
        .session_id
        .and_then(present_owned)
        .context("local ingress session context event missing `conversation.session_id`")?;
    let model = conversation
        .model
        .and_then(present_owned)
        .context("local ingress session context event missing `conversation.model`")?;

    state.record_session_model(source_id.clone(), session_id.clone(), model.clone())?;
    info!(
        source.id = %source_id,
        conversation.session_id = %session_id,
        conversation.model = %model,
        event = "ingress.session_context.stored",
    );
    Ok(())
}

fn store_turn_start(
    config: &ValidatedConfig,
    state: &LocalIngressState,
    event: LocalSignalEvent,
) -> anyhow::Result<()> {
    let source_id = event.source_id;
    validate_local_ingress_source(config, &source_id)?;
    let conversation = event
        .conversation
        .context("local ingress turn start event missing `conversation`")?;
    let session_id = conversation
        .session_id
        .and_then(present_owned)
        .context("local ingress turn start event missing `conversation.session_id`")?;
    let lifecycle = event
        .lifecycle
        .context("local ingress turn start event missing `lifecycle`")?;
    let started_at = lifecycle
        .started_at
        .context("local ingress turn start event missing `lifecycle.started_at`")?;

    state.record_turn_started_at(source_id.clone(), session_id.clone(), started_at)?;
    info!(
        source.id = %source_id,
        conversation.session_id = %session_id,
        event = "ingress.turn_start.stored",
    );
    Ok(())
}

#[cfg(test)]
fn create_signal(
    config: &ValidatedConfig,
    event: LocalSignalEvent,
) -> anyhow::Result<crate::signal::Signal> {
    let draft = create_signal_draft(config, event)?;
    SignalBuilder::new(config).build(draft)
}

fn create_signal_draft(
    config: &ValidatedConfig,
    event: LocalSignalEvent,
) -> anyhow::Result<SignalDraft> {
    let source = validate_local_ingress_source(config, &event.source_id)?;
    Ok(event.into_signal_draft(source.source_type))
}

fn validate_local_ingress_source<'a>(
    config: &'a ValidatedConfig,
    source_id: &str,
) -> anyhow::Result<&'a crate::config::SourceConfig> {
    let source = config
        .source(source_id)
        .with_context(|| format!("source `{source_id}` is not configured"))?;

    if source.source_type == SourceType::CodexDesktop {
        anyhow::bail!(
            "source `{}` has type `codex_desktop`; local ingress only accepts `agents_router`, `agent_hook`, `codex_cli`, and `claude_code` events",
            source.id
        );
    }

    Ok(source)
}

fn apply_session_context(state: &LocalIngressState, draft: &mut SignalDraft) -> anyhow::Result<()> {
    let source_id = draft.source_id.clone();
    let Some(conversation) = draft.conversation.as_mut() else {
        return Ok(());
    };
    if conversation.model.is_some() {
        return Ok(());
    }
    let Some(session_id) = conversation.session_id.as_deref() else {
        return Ok(());
    };
    if let Some(model) = state.session_model(&source_id, session_id)? {
        conversation.model = Some(model);
    }
    Ok(())
}

fn apply_turn_start_context(
    state: &LocalIngressState,
    draft: &mut SignalDraft,
) -> anyhow::Result<()> {
    if draft.event.kind != SignalEventKind::TurnCompleted {
        return Ok(());
    }

    let source_id = draft.source_id.clone();
    let Some(conversation) = draft.conversation.as_ref() else {
        return Ok(());
    };
    let Some(session_id) = conversation.session_id.as_deref() else {
        return Ok(());
    };
    let Some(started_at) = state.take_turn_started_at(&source_id, session_id)? else {
        return Ok(());
    };

    let lifecycle = draft.lifecycle.get_or_insert(SignalLifecycle {
        status: Some(SignalLifecycleStatus::Completed),
        started_at: None,
        completed_at: None,
        duration_ms: None,
    });
    let effective_started_at = if let Some(started_at) = lifecycle.started_at {
        started_at
    } else {
        lifecycle.started_at = Some(started_at);
        started_at
    };
    if lifecycle.duration_ms.is_none()
        && let Some(completed_at) = lifecycle.completed_at
    {
        let duration_ms = completed_at
            .signed_duration_since(effective_started_at)
            .num_milliseconds();
        if let Ok(duration_ms) = u64::try_from(duration_ms) {
            lifecycle.duration_ms = Some(duration_ms);
        }
    }

    Ok(())
}

fn present_owned(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(unix)]
async fn handle_unix_connection(
    mut stream: UnixStream,
    runtime: &RuntimeState,
    state: &LocalIngressState,
) -> anyhow::Result<()> {
    let mut raw_request = Vec::new();
    stream
        .read_to_end(&mut raw_request)
        .await
        .context("failed to read local ingress request")?;

    let raw_response = handle_request_bytes(runtime, state, &raw_request).await?;

    stream
        .write_all(&raw_response)
        .await
        .context("failed to write local ingress response")?;

    Ok(())
}

#[cfg(windows)]
async fn handle_windows_pipe(
    mut pipe: tokio::net::windows::named_pipe::NamedPipeServer,
    runtime: &RuntimeState,
    state: &LocalIngressState,
) -> anyhow::Result<()> {
    let mut raw_request = Vec::new();
    pipe.read_to_end(&mut raw_request)
        .await
        .context("failed to read local ingress request")?;

    let raw_response = handle_request_bytes(runtime, state, &raw_request).await?;
    pipe.write_all(&raw_response)
        .await
        .context("failed to write local ingress response")?;

    Ok(())
}

async fn handle_request_bytes(
    runtime: &RuntimeState,
    state: &LocalIngressState,
    raw_request: &[u8],
) -> anyhow::Result<Vec<u8>> {
    if is_ping_request(raw_request) {
        return serde_json::to_vec(&LocalIngressResponse::ok())
            .context("failed to serialize local ingress response");
    }
    if is_delivery_safety_reset_request(raw_request) {
        let result = runtime.delivery_safety().reset();
        let response = match result {
            Ok(()) => {
                info!(event = "delivery.pause.reset");
                LocalIngressResponse::ok()
            }
            Err(error) => LocalIngressResponse {
                ok: false,
                error: Some(error.to_string()),
                delivery: None,
            },
        };
        return serde_json::to_vec(&response).context("failed to serialize local ingress response");
    }

    let result = async {
        let event: LocalSignalEvent = serde_json::from_slice(raw_request)
            .context("failed to parse local ingress event JSON")?;
        route_event_with_runtime(runtime, state, event).await
    }
    .await;

    let response = match result {
        Ok(report) => LocalIngressResponse {
            ok: true,
            error: None,
            delivery: Some(report.into()),
        },
        Err(error) => {
            warn!(
                error = %error,
                event = "ingress.client.failed",
            );
            LocalIngressResponse {
                ok: false,
                error: Some(error.to_string()),
                delivery: None,
            }
        }
    };

    serde_json::to_vec(&response).context("failed to serialize local ingress response")
}

fn is_delivery_safety_reset_request(raw_request: &[u8]) -> bool {
    request_kind(raw_request)
        .map(|kind| kind == "delivery_safety_reset")
        .unwrap_or(false)
}

fn is_ping_request(raw_request: &[u8]) -> bool {
    request_kind(raw_request)
        .map(|kind| kind == "ping")
        .unwrap_or(false)
}

fn request_kind(raw_request: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(raw_request)
        .ok()
        .and_then(|value| {
            value
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

#[cfg(unix)]
async fn prepare_socket_path(socket_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create local ingress socket directory `{}`",
                parent.display()
            )
        })?;
    }

    if socket_path.exists() {
        if UnixStream::connect(socket_path).await.is_ok() {
            anyhow::bail!(
                "local ingress socket `{}` is already accepting connections; agents-router may already be running",
                socket_path.display()
            );
        }
        fs::remove_file(socket_path).with_context(|| {
            format!(
                "failed to remove stale local ingress socket `{}`",
                socket_path.display()
            )
        })?;
    }

    Ok(())
}

pub async fn is_ready(endpoint: &IngressEndpoint) -> bool {
    match endpoint {
        #[cfg(unix)]
        IngressEndpoint::UnixSocket(socket_path) => {
            timeout(READINESS_PING_TIMEOUT, ping_unix_socket(socket_path))
                .await
                .unwrap_or(false)
        }
        #[cfg(windows)]
        IngressEndpoint::WindowsNamedPipe(pipe_name) => {
            timeout(READINESS_PING_TIMEOUT, ping_windows_named_pipe(pipe_name))
                .await
                .unwrap_or(false)
        }
    }
}

#[cfg(unix)]
async fn ping_unix_socket(socket_path: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path).await else {
        return false;
    };

    if stream.write_all(br#"{"kind":"ping"}"#).await.is_err() {
        return false;
    }
    if stream.shutdown().await.is_err() {
        return false;
    }

    let mut raw_response = Vec::new();
    if stream.read_to_end(&mut raw_response).await.is_err() {
        return false;
    }

    serde_json::from_slice::<LocalIngressResponse>(&raw_response)
        .map(|response| response.ok)
        .unwrap_or(false)
}

#[cfg(windows)]
async fn ping_windows_named_pipe(pipe_name: &str) -> bool {
    let Ok(mut stream) = ClientOptions::new().open(pipe_name) else {
        return false;
    };

    if stream.write_all(br#"{"kind":"ping"}"#).await.is_err() {
        return false;
    }
    if stream.shutdown().await.is_err() {
        return false;
    }

    let mut raw_response = Vec::new();
    if stream.read_to_end(&mut raw_response).await.is_err() {
        return false;
    }

    serde_json::from_slice::<LocalIngressResponse>(&raw_response)
        .map(|response| response.ok)
        .unwrap_or(false)
}

pub fn cleanup_endpoint(endpoint: &IngressEndpoint) -> anyhow::Result<()> {
    match endpoint {
        #[cfg(unix)]
        IngressEndpoint::UnixSocket(socket_file) => {
            if socket_file.exists() {
                fs::remove_file(socket_file).with_context(|| {
                    format!("failed to remove socket file `{}`", socket_file.display())
                })?;
            }
            Ok(())
        }
        #[cfg(windows)]
        IngressEndpoint::WindowsNamedPipe(_) => Ok(()),
    }
}

#[cfg(unix)]
struct SocketFileGuard {
    path: PathBuf,
}

#[cfg(unix)]
impl SocketFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[cfg(unix)]
impl Drop for SocketFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests;
