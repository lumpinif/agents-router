use std::process::Stdio;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{Duration, Instant as TokioInstant, sleep_until};
use tracing::{debug, info, warn};

use crate::agent_controller::{
    AgentControllerAdapter, AgentControllerError, AgentControllerErrorKind, AgentControllerFuture,
    AgentControllerRequest, AgentControllerSubmitObserver, AgentControllerSuccess,
    AgentSessionStartFuture, AgentSessionStartObserver, AgentSessionStartRequest,
    AgentSessionStartSuccess,
};
use crate::agent_integration_catalog::AgentControllerKind;

#[cfg(not(test))]
const TURN_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(test)]
const TURN_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub struct CodexAppServerController {
    command: CodexAppServerCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexAppServerCommand {
    program: String,
    args: Vec<String>,
}

impl Default for CodexAppServerCommand {
    fn default() -> Self {
        Self {
            program: "codex".to_string(),
            args: vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string(),
            ],
        }
    }
}

impl CodexAppServerController {
    pub fn new() -> Self {
        Self {
            command: CodexAppServerCommand::default(),
        }
    }
}

impl Default for CodexAppServerController {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentControllerAdapter for CodexAppServerController {
    fn controller_kind(&self) -> AgentControllerKind {
        AgentControllerKind::CodexAppServer
    }

    fn continue_session<'a>(
        &'a self,
        request: AgentControllerRequest,
    ) -> AgentControllerFuture<'a> {
        Box::pin(async move {
            let submit_observer = NoopSubmitObserver;
            self.continue_session_with_submit_observer(request, &submit_observer)
                .await
        })
    }

    fn continue_session_with_submit_observer<'a>(
        &'a self,
        request: AgentControllerRequest,
        submit_observer: &'a dyn AgentControllerSubmitObserver,
    ) -> AgentControllerFuture<'a> {
        Box::pin(async move {
            let mut connection = ProcessAppServerConnection::spawn(&self.command)
                .await
                .map_err(|message| {
                    AgentControllerError::failed_before_submit(
                        &request,
                        AgentControllerErrorKind::ControllerUnavailable,
                        message,
                    )
                })?;

            let result =
                continue_session_with_connection(&mut connection, &request, submit_observer).await;
            connection.shutdown().await;
            result
        })
    }

    fn start_session<'a>(
        &'a self,
        request: AgentSessionStartRequest,
    ) -> AgentSessionStartFuture<'a> {
        Box::pin(async move {
            let observer = NoopStartObserver;
            self.start_session_with_observer(request, &observer).await
        })
    }

    fn start_session_with_observer<'a>(
        &'a self,
        request: AgentSessionStartRequest,
        observer: &'a dyn AgentSessionStartObserver,
    ) -> AgentSessionStartFuture<'a> {
        Box::pin(async move {
            let mut connection = ProcessAppServerConnection::spawn(&self.command)
                .await
                .map_err(|message| {
                    AgentControllerError::failed_before_submit(
                        &request.as_placeholder_controller_request(),
                        AgentControllerErrorKind::ControllerUnavailable,
                        message,
                    )
                })?;

            let result = start_session_with_connection(&mut connection, &request, observer).await;
            connection.shutdown().await;
            result
        })
    }
}

struct NoopSubmitObserver;

impl AgentControllerSubmitObserver for NoopSubmitObserver {
    fn submitted_possible<'a>(
        &'a self,
        _source_turn_id: &'a str,
    ) -> crate::agent_controller::AgentControllerSubmitFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

struct NoopStartObserver;

impl AgentSessionStartObserver for NoopStartObserver {
    fn thread_started<'a>(
        &'a self,
        _source_session_id: &'a str,
    ) -> crate::agent_controller::AgentControllerSubmitFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn submitted_possible<'a>(
        &'a self,
        _source_session_id: &'a str,
        _source_turn_id: &'a str,
    ) -> crate::agent_controller::AgentControllerSubmitFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

trait AppServerConnection {
    async fn send_line(&mut self, line: String) -> std::io::Result<()>;
    async fn read_line(&mut self) -> std::io::Result<Option<String>>;
}

struct ProcessAppServerConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl ProcessAppServerConnection {
    async fn spawn(command: &CodexAppServerCommand) -> Result<Self, String> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start Codex App Server controller: {error}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open Codex App Server controller stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open Codex App Server controller stdout".to_string())?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
        })
    }

    async fn shutdown(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

impl Drop for ProcessAppServerConnection {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl AppServerConnection for ProcessAppServerConnection {
    async fn send_line(&mut self, line: String) -> std::io::Result<()> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await
    }

    async fn read_line(&mut self) -> std::io::Result<Option<String>> {
        self.stdout.next_line().await
    }
}

async fn continue_session_with_connection(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    submit_observer: &dyn AgentControllerSubmitObserver,
) -> Result<AgentControllerSuccess, AgentControllerError> {
    let controller_started_at = Instant::now();
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        controller.kind = ?request.controller_kind,
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.controller.started",
    );
    let mut next_request_id = 1;
    send_request(
        connection,
        request,
        &mut next_request_id,
        "initialize",
        json!({
            "clientInfo": {
                "name": "agents-router",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "experimentalApi": true,
            },
        }),
        SubmitBoundary::BeforeSubmit,
        None,
    )
    .await?;
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        elapsed.ms = controller_started_at.elapsed().as_millis(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.initialize.succeeded",
    );
    send_notification(
        connection,
        request,
        "initialized",
        json!({}),
        SubmitBoundary::BeforeSubmit,
    )
    .await?;

    let thread_read_started_at = Instant::now();
    let thread_read_result = send_request(
        connection,
        request,
        &mut next_request_id,
        "thread/read",
        json!({
            "threadId": request.source_session_id,
        }),
        SubmitBoundary::BeforeSubmit,
        None,
    )
    .await?;
    let thread_status = ensure_read_thread_matches_request(request, &thread_read_result)?;
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        thread.status = %thread_status.as_log_value(),
        elapsed.ms = thread_read_started_at.elapsed().as_millis(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.thread_read.succeeded",
    );
    match &thread_status {
        CodexThreadStatus::Idle | CodexThreadStatus::Loadable(_) => {}
        CodexThreadStatus::Active(status) => {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                thread.status = %status,
                thread.stage = "thread_read",
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.thread_active.fail_closed",
            );
            return Err(AgentControllerError::failed_before_submit(
                request,
                AgentControllerErrorKind::SessionActive,
                "original Codex session is currently active",
            ));
        }
        CodexThreadStatus::Other(status) => {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                thread.status = %status,
                thread.stage = "thread_read",
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.thread_not_continuable.fail_closed",
            );
            return Err(AgentControllerError::failed_before_submit(
                request,
                AgentControllerErrorKind::SessionNotContinuable,
                format!("Codex App Server thread status `{status}` is not continuable"),
            ));
        }
        CodexThreadStatus::Missing => {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                thread.status = "missing",
                thread.stage = "thread_read",
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.thread_not_continuable.fail_closed",
            );
            return Err(AgentControllerError::failed_before_submit(
                request,
                AgentControllerErrorKind::SessionNotContinuable,
                "Codex App Server thread/read response did not include thread status",
            ));
        }
    }

    let thread_resume_started_at = Instant::now();
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        thread.read.status = %thread_status.as_log_value(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.thread_resume.started",
    );
    let resume_result = send_request(
        connection,
        request,
        &mut next_request_id,
        "thread/resume",
        json!({
            "threadId": request.source_session_id,
            "excludeTurns": true,
        }),
        SubmitBoundary::BeforeSubmit,
        None,
    )
    .await?;
    let post_resume_status = ensure_resumed_thread_matches_request(request, &resume_result)?;
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        thread.post_resume.status = %post_resume_status.as_log_value(),
        elapsed.ms = thread_resume_started_at.elapsed().as_millis(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.thread_resume.succeeded",
    );
    match &post_resume_status {
        CodexThreadStatus::Idle => {}
        CodexThreadStatus::Missing => {
            info!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                thread.post_resume.status = "missing",
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.thread_resume.status_missing_allowed",
            );
        }
        CodexThreadStatus::Active(status) => {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                thread.status = %status,
                thread.stage = "thread_resume",
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.thread_active.fail_closed",
            );
            return Err(AgentControllerError::failed_before_submit(
                request,
                AgentControllerErrorKind::SessionActive,
                "original Codex session is currently active",
            ));
        }
        CodexThreadStatus::Loadable(status) | CodexThreadStatus::Other(status) => {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                thread.status = %status,
                thread.stage = "thread_resume",
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.thread_not_continuable.fail_closed",
            );
            return Err(AgentControllerError::failed_before_submit(
                request,
                AgentControllerErrorKind::SessionNotContinuable,
                format!("Codex App Server post-resume thread status `{status}` is not continuable"),
            ));
        }
    }

    let mut turn_state = TurnStartState::new(request.source_session_id.clone());
    let turn_start_started_at = Instant::now();
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.turn_start.sent",
    );
    let turn_start_result = send_request(
        connection,
        request,
        &mut next_request_id,
        "turn/start",
        json!({
            "threadId": request.source_session_id,
            "input": [
                {
                    "type": "text",
                    "text": request.reply_text,
                }
            ],
        }),
        SubmitBoundary::AfterPossibleSubmit,
        Some(&mut turn_state),
    )
    .await?;
    turn_state.bind_turn_from_start_response(request, &turn_start_result)?;
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        turn.id = turn_state.turn_id.as_deref(),
        elapsed.ms = turn_start_started_at.elapsed().as_millis(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.turn_start.accepted",
    );
    let source_turn_id = turn_state.turn_id.as_deref().ok_or_else(|| {
        error_after_possible_submit(
            request,
            AgentControllerErrorKind::Internal,
            "Codex App Server accepted turn/start without turn id",
        )
    })?;
    submit_observer
        .submitted_possible(source_turn_id)
        .await
        .map_err(|error| {
            error_after_possible_submit(
                request,
                AgentControllerErrorKind::Internal,
                format!("failed to record submitted continuation boundary: {error}"),
            )
        })?;

    wait_for_final_answer(
        connection,
        request,
        &mut next_request_id,
        &mut turn_state,
        controller_started_at,
    )
    .await
}

async fn start_session_with_connection(
    connection: &mut impl AppServerConnection,
    request: &AgentSessionStartRequest,
    observer: &dyn AgentSessionStartObserver,
) -> Result<AgentSessionStartSuccess, AgentControllerError> {
    let controller_started_at = Instant::now();
    let placeholder = request.as_placeholder_controller_request();
    info!(
        surface.id = %request.surface_id,
        source.id = %request.source_id,
        source.type = %request.source_type.as_str(),
        project.path = %request.project_path,
        controller.kind = ?request.controller_kind,
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.session_start.controller.started",
    );
    let mut next_request_id = 1;
    send_request(
        connection,
        &placeholder,
        &mut next_request_id,
        "initialize",
        json!({
            "clientInfo": {
                "name": "agents-router",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "experimentalApi": true,
            },
        }),
        SubmitBoundary::BeforeSubmit,
        None,
    )
    .await?;
    send_notification(
        connection,
        &placeholder,
        "initialized",
        json!({}),
        SubmitBoundary::BeforeSubmit,
    )
    .await?;

    let thread_start_result = send_request(
        connection,
        &placeholder,
        &mut next_request_id,
        "thread/start",
        json!({
            "cwd": &request.project_path,
            "threadSource": "user",
        }),
        SubmitBoundary::BeforeSubmit,
        None,
    )
    .await?;
    let source_session_id =
        thread_id_from_result(&placeholder, &thread_start_result, "thread/start")?.to_string();
    info!(
        surface.id = %request.surface_id,
        source.session.id = %source_session_id,
        project.path = %request.project_path,
        elapsed.ms = controller_started_at.elapsed().as_millis(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.thread_start.succeeded",
    );
    observer
        .thread_started(&source_session_id)
        .await
        .map_err(|error| {
            AgentControllerError::failed_before_submit(
                &AgentControllerRequest {
                    source_session_id: source_session_id.clone(),
                    ..placeholder.clone()
                },
                AgentControllerErrorKind::Internal,
                format!("failed to record new session binding: {error}"),
            )
        })?;

    let turn_request = AgentControllerRequest {
        source_session_id: source_session_id.clone(),
        reply_text: request.prompt.clone(),
        ..placeholder
    };
    let mut turn_state = TurnStartState::new(source_session_id.clone());
    let turn_start_result = send_request(
        connection,
        &turn_request,
        &mut next_request_id,
        "turn/start",
        json!({
            "threadId": source_session_id,
            "input": [
                {
                    "type": "text",
                    "text": &request.prompt,
                }
            ],
        }),
        SubmitBoundary::AfterPossibleSubmit,
        Some(&mut turn_state),
    )
    .await?;
    turn_state.bind_turn_from_start_response(&turn_request, &turn_start_result)?;
    let source_turn_id = turn_state.turn_id.as_deref().ok_or_else(|| {
        error_after_possible_submit(
            &turn_request,
            AgentControllerErrorKind::Internal,
            "Codex App Server accepted turn/start without turn id",
        )
    })?;
    observer
        .submitted_possible(&source_session_id, source_turn_id)
        .await
        .map_err(|error| {
            error_after_possible_submit(
                &turn_request,
                AgentControllerErrorKind::Internal,
                format!("failed to record submitted new session boundary: {error}"),
            )
        })?;

    let result = wait_for_final_answer(
        connection,
        &turn_request,
        &mut next_request_id,
        &mut turn_state,
        controller_started_at,
    )
    .await?;

    Ok(AgentSessionStartSuccess {
        source_session_id,
        result,
    })
}

async fn wait_for_final_answer(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    next_request_id: &mut u64,
    turn_state: &mut TurnStartState,
    controller_started_at: Instant,
) -> Result<AgentControllerSuccess, AgentControllerError> {
    let mut next_poll_at = TokioInstant::now() + TURN_STATUS_POLL_INTERVAL;

    loop {
        if let Some(message) = turn_state.error_message.take() {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                turn.id = turn_state.turn_id.as_deref(),
                elapsed.ms = controller_started_at.elapsed().as_millis(),
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.turn_result.unconfirmed",
            );
            return Err(error_after_possible_submit(
                request,
                AgentControllerErrorKind::ControllerRejected,
                format!(
                    "Codex App Server reported an error after turn/start was submitted: {message}"
                ),
            ));
        }
        if let Some(final_answer) = turn_state.final_answer.take() {
            info!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                turn.id = turn_state.turn_id.as_deref(),
                elapsed.ms = controller_started_at.elapsed().as_millis(),
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.final_answer.observed",
            );
            return AgentControllerSuccess::from_result_text(final_answer).ok_or_else(|| {
                error_after_possible_submit(
                    request,
                    AgentControllerErrorKind::Internal,
                    "Codex App Server completed without result text",
                )
            });
        }
        if turn_state.turn_completed {
            warn!(
                surface.id = %request.surface_id,
                source.session.id = %request.source_session_id,
                turn.id = turn_state.turn_id.as_deref(),
                elapsed.ms = controller_started_at.elapsed().as_millis(),
                event.hash = %request.provider_event_id_hash,
                event = "codex_app_server.turn_completed_without_final_answer",
            );
            return Err(error_after_possible_submit(
                request,
                AgentControllerErrorKind::Internal,
                "Codex App Server turn completed before final answer",
            ));
        }

        // This is not an agent execution timeout. The router does not decide
        // that a long Codex turn failed because time passed. The timer only
        // gives the controller a regular chance to read official thread
        // status even when the app-server stream stays busy with other events.
        tokio::select! {
            biased;

            _ = sleep_until(next_poll_at) => {
                poll_turn_status_from_thread_snapshot(
                    connection,
                    request,
                    next_request_id,
                    turn_state,
                    controller_started_at,
                )
                .await?;
                next_poll_at = TokioInstant::now() + TURN_STATUS_POLL_INTERVAL;
            }
            message = read_message(connection, request, SubmitBoundary::AfterPossibleSubmit) => {
                let message = message?;
                turn_state.observe(&message);
            }
        }
    }
}

async fn poll_turn_status_from_thread_snapshot(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    next_request_id: &mut u64,
    turn_state: &mut TurnStartState,
    controller_started_at: Instant,
) -> Result<(), AgentControllerError> {
    let poll_started_at = Instant::now();
    let snapshot = send_request(
        connection,
        request,
        next_request_id,
        "thread/resume",
        json!({
            "threadId": request.source_session_id,
        }),
        SubmitBoundary::AfterPossibleSubmit,
        Some(turn_state),
    )
    .await?;
    let post_resume_status = ensure_resumed_thread_matches_request(request, &snapshot)?;
    turn_state.observe_thread_snapshot(&snapshot);
    info!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        thread.post_resume.status = %post_resume_status.as_log_value(),
        turn.id = turn_state.turn_id.as_deref(),
        turn.status = turn_state.last_observed_turn_status.as_deref(),
        final_answer.observed = turn_state.final_answer.is_some(),
        turn.terminal = turn_state.turn_completed,
        elapsed.ms = controller_started_at.elapsed().as_millis(),
        poll.elapsed.ms = poll_started_at.elapsed().as_millis(),
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.turn_status.polled",
    );
    Ok(())
}

async fn send_request(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    next_request_id: &mut u64,
    method: &str,
    params: Value,
    boundary: SubmitBoundary,
    mut turn_state: Option<&mut TurnStartState>,
) -> Result<Value, AgentControllerError> {
    let request_id = *next_request_id;
    *next_request_id += 1;
    let request_line = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    }))
    .map_err(|error| {
        controller_error(
            request,
            AgentControllerErrorKind::Internal,
            boundary,
            format!("failed to serialize Codex App Server request: {error}"),
        )
    })?;

    connection.send_line(request_line).await.map_err(|error| {
        controller_error(
            request,
            io_error_kind(method, boundary),
            boundary,
            format!("failed to send Codex App Server {method} request: {error}"),
        )
    })?;
    debug!(
        surface.id = %request.surface_id,
        source.session.id = %request.source_session_id,
        method = method,
        request.id = request_id,
        submit.boundary = ?boundary,
        event.hash = %request.provider_event_id_hash,
        event = "codex_app_server.request.sent",
    );

    loop {
        let message = read_message(connection, request, boundary).await?;
        if let Some(state) = turn_state.as_mut() {
            state.observe(&message);
            if let Some(message) = state.error_message.take() {
                return Err(error_after_possible_submit(
                    request,
                    AgentControllerErrorKind::ControllerRejected,
                    format!(
                        "Codex App Server reported an error after turn/start was submitted: {message}"
                    ),
                ));
            }
            if state.turn_completed && state.final_answer.is_none() {
                return Err(error_after_possible_submit(
                    request,
                    AgentControllerErrorKind::Internal,
                    "Codex App Server turn completed before final answer",
                ));
            }
        }

        if message.id == Some(json!(request_id)) {
            if let Some(error) = message.error {
                return Err(controller_error(
                    request,
                    rpc_error_kind(method, &error.message),
                    boundary,
                    format!("Codex App Server {method} failed: {}", error.message),
                ));
            }

            return Ok(message.result.unwrap_or(Value::Null));
        }
    }
}

async fn send_notification(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    method: &str,
    params: Value,
    boundary: SubmitBoundary,
) -> Result<(), AgentControllerError> {
    let request_line = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    }))
    .map_err(|error| {
        controller_error(
            request,
            AgentControllerErrorKind::Internal,
            boundary,
            format!("failed to serialize Codex App Server {method} notification: {error}"),
        )
    })?;

    connection.send_line(request_line).await.map_err(|error| {
        controller_error(
            request,
            io_error_kind(method, boundary),
            boundary,
            format!("failed to send Codex App Server {method} notification: {error}"),
        )
    })
}

async fn read_message(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    boundary: SubmitBoundary,
) -> Result<AppServerMessage, AgentControllerError> {
    let line = connection.read_line().await.map_err(|error| {
        controller_error(
            request,
            io_error_kind("read", boundary),
            boundary,
            format!("failed to read Codex App Server response: {error}"),
        )
    })?;

    let Some(line) = line else {
        return Err(controller_error(
            request,
            AgentControllerErrorKind::Internal,
            boundary,
            "Codex App Server stream ended before completion",
        ));
    };

    serde_json::from_str(&line).map_err(|error| {
        controller_error(
            request,
            AgentControllerErrorKind::Internal,
            boundary,
            format!("Codex App Server returned invalid JSON: {error}"),
        )
    })
}

fn ensure_resumed_thread_matches_request(
    request: &AgentControllerRequest,
    resume_result: &Value,
) -> Result<CodexThreadStatus, AgentControllerError> {
    let thread = thread_from_result(request, resume_result, "thread/resume")?;
    ensure_thread_identity_matches_request(request, thread, "resume")?;
    Ok(codex_thread_status(thread))
}

fn ensure_read_thread_matches_request(
    request: &AgentControllerRequest,
    read_result: &Value,
) -> Result<CodexThreadStatus, AgentControllerError> {
    let thread = thread_from_result(request, read_result, "thread/read")?;
    ensure_thread_identity_matches_request(request, thread, "read")?;
    Ok(codex_thread_status(thread))
}

fn thread_from_result<'a>(
    request: &AgentControllerRequest,
    result: &'a Value,
    method: &str,
) -> Result<&'a Value, AgentControllerError> {
    result.get("thread").ok_or_else(|| {
        AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            format!("Codex App Server {method} response did not include thread facts"),
        )
    })
}

fn thread_id_from_result<'a>(
    request: &AgentControllerRequest,
    result: &'a Value,
    method: &str,
) -> Result<&'a str, AgentControllerError> {
    let thread = thread_from_result(request, result, method)?;
    thread.get("id").and_then(Value::as_str).ok_or_else(|| {
        AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            format!("Codex App Server {method} response did not include thread.id"),
        )
    })
}

fn ensure_thread_identity_matches_request(
    request: &AgentControllerRequest,
    thread: &Value,
    action: &str,
) -> Result<(), AgentControllerError> {
    let thread_id = thread.get("id").and_then(Value::as_str).ok_or_else(|| {
        AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            format!("Codex App Server thread/{action} response did not include thread.id"),
        )
    })?;

    if thread_id != request.source_session_id {
        return Err(AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            format!(
                "Codex App Server {action} thread `{thread_id}` instead of requested source session"
            ),
        ));
    }

    if let Some(session_id) = thread.get("sessionId").and_then(Value::as_str)
        && session_id != request.source_session_id
    {
        return Err(AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            format!(
                "Codex App Server {action} session `{session_id}` instead of requested source session"
            ),
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexThreadStatus {
    Idle,
    Loadable(String),
    Active(String),
    Missing,
    Other(String),
}

impl CodexThreadStatus {
    fn as_log_value(&self) -> &str {
        match self {
            Self::Idle => "idle",
            Self::Missing => "missing",
            Self::Loadable(status) | Self::Active(status) | Self::Other(status) => status,
        }
    }
}

fn codex_thread_status(thread: &Value) -> CodexThreadStatus {
    let Some(status) = thread.get("status").and_then(|status| {
        status
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| status.as_str())
    }) else {
        return CodexThreadStatus::Missing;
    };

    match status {
        "idle" => CodexThreadStatus::Idle,
        "notLoaded" => CodexThreadStatus::Loadable(status.to_string()),
        "active" | "busy" | "generating" | "running" | "working" => {
            CodexThreadStatus::Active(status.to_string())
        }
        other => CodexThreadStatus::Other(other.to_string()),
    }
}

fn controller_error(
    request: &AgentControllerRequest,
    kind: AgentControllerErrorKind,
    boundary: SubmitBoundary,
    message: impl Into<String>,
) -> AgentControllerError {
    match boundary {
        SubmitBoundary::BeforeSubmit => {
            AgentControllerError::failed_before_submit(request, kind, message)
        }
        SubmitBoundary::AfterPossibleSubmit => {
            AgentControllerError::failed_after_possible_submit(request, kind, message)
        }
    }
}

fn error_after_possible_submit(
    request: &AgentControllerRequest,
    kind: AgentControllerErrorKind,
    message: impl Into<String>,
) -> AgentControllerError {
    AgentControllerError::failed_after_possible_submit(request, kind, message)
}

fn rpc_error_kind(method: &str, message: &str) -> AgentControllerErrorKind {
    let normalized = message.to_ascii_lowercase();
    if method == "thread/resume" && normalized.contains("not found") {
        return AgentControllerErrorKind::SessionNotFound;
    }
    if method == "thread/resume" {
        return AgentControllerErrorKind::SessionNotContinuable;
    }
    if method == "initialize" {
        return AgentControllerErrorKind::ControllerUnavailable;
    }
    AgentControllerErrorKind::ControllerRejected
}

fn io_error_kind(method: &str, boundary: SubmitBoundary) -> AgentControllerErrorKind {
    if boundary == SubmitBoundary::BeforeSubmit && method == "initialize" {
        return AgentControllerErrorKind::ControllerUnavailable;
    }
    AgentControllerErrorKind::Internal
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitBoundary {
    BeforeSubmit,
    AfterPossibleSubmit,
}

#[derive(Debug, Deserialize)]
struct AppServerMessage {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<AppServerRpcError>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct AppServerRpcError {
    message: String,
}

#[derive(Default)]
struct TurnStartState {
    thread_id: String,
    turn_id: Option<String>,
    final_answer: Option<String>,
    error_message: Option<String>,
    turn_completed: bool,
    last_observed_turn_status: Option<String>,
}

impl TurnStartState {
    fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            turn_id: None,
            final_answer: None,
            error_message: None,
            turn_completed: false,
            last_observed_turn_status: None,
        }
    }

    fn bind_turn_from_start_response(
        &mut self,
        request: &AgentControllerRequest,
        response: &Value,
    ) -> Result<(), AgentControllerError> {
        let Some(turn_id) = turn_id_from_value(response) else {
            if self.turn_id.is_some() {
                return Ok(());
            }
            return Err(error_after_possible_submit(
                request,
                AgentControllerErrorKind::Internal,
                "Codex App Server turn/start response did not include turn.id",
            ));
        };
        self.bind_turn_id(request, turn_id)
    }

    fn bind_turn_id(
        &mut self,
        request: &AgentControllerRequest,
        turn_id: &str,
    ) -> Result<(), AgentControllerError> {
        if let Some(current) = &self.turn_id
            && current != turn_id
        {
            return Err(error_after_possible_submit(
                request,
                AgentControllerErrorKind::Internal,
                format!("Codex App Server reported mismatched turn id `{turn_id}`"),
            ));
        }
        self.turn_id = Some(turn_id.to_string());
        Ok(())
    }

    fn observe(&mut self, message: &AppServerMessage) {
        match message.method.as_deref() {
            Some("turn/started") => {
                let Some(params) = message.params.as_ref() else {
                    return;
                };
                if !self.matches_thread(params) {
                    return;
                }
                if let Some(turn_id) = notification_turn_id(params)
                    && self.turn_id.is_none()
                {
                    self.turn_id = Some(turn_id.to_string());
                }
            }
            Some("item/completed") => {
                let Some(params) = message.params.as_ref() else {
                    return;
                };
                if !self.matches_current_turn(params) {
                    return;
                }
                let Some(item) = params.get("item") else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) == Some("agentMessage")
                    && item.get("phase").and_then(Value::as_str) == Some("final_answer")
                    && let Some(text) = item.get("text").and_then(Value::as_str)
                {
                    self.final_answer = Some(text.to_string());
                }
            }
            Some("error") => {
                let Some(params) = message.params.as_ref() else {
                    self.error_message = Some(extract_notification_error(message));
                    return;
                };
                if self.matches_current_turn_or_unscoped_error(params) {
                    self.error_message = Some(extract_notification_error(message));
                }
            }
            Some("turn/completed") => {
                let Some(params) = message.params.as_ref() else {
                    return;
                };
                if self.matches_current_turn(params) {
                    self.turn_completed = true;
                }
            }
            _ => {}
        }
    }

    fn observe_thread_snapshot(&mut self, snapshot: &Value) {
        let Some(turn_id) = self.turn_id.as_deref() else {
            return;
        };
        let Some(turn) = turns_from_snapshot(snapshot)
            .into_iter()
            .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        else {
            return;
        };

        self.last_observed_turn_status = turn_status_from_value(turn).map(str::to_string);
        if let Some(final_answer) = final_answer_from_items(turn.get("items")) {
            self.final_answer = Some(final_answer.to_string());
            self.turn_completed = true;
            return;
        }

        if self
            .last_observed_turn_status
            .as_deref()
            .is_some_and(is_terminal_turn_status)
        {
            self.turn_completed = true;
        }
    }

    fn matches_thread(&self, params: &Value) -> bool {
        notification_thread_id(params) == Some(self.thread_id.as_str())
    }

    fn matches_current_turn(&self, params: &Value) -> bool {
        self.matches_thread(params)
            && self
                .turn_id
                .as_deref()
                .is_some_and(|turn_id| notification_turn_id(params) == Some(turn_id))
    }

    fn matches_current_turn_or_unscoped_error(&self, params: &Value) -> bool {
        if let Some(thread_id) = notification_thread_id(params)
            && thread_id != self.thread_id
        {
            return false;
        }
        let Some(event_turn_id) = notification_turn_id(params) else {
            return true;
        };
        self.turn_id
            .as_deref()
            .is_none_or(|turn_id| turn_id == event_turn_id)
    }
}

fn extract_notification_error(message: &AppServerMessage) -> String {
    message
        .params
        .as_ref()
        .and_then(|params| params.get("error"))
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "unknown Codex App Server error".to_string())
}

fn turn_id_from_value(value: &Value) -> Option<&str> {
    value
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .or_else(|| value.get("turnId").and_then(Value::as_str))
        .or_else(|| value.get("id").and_then(Value::as_str))
}

fn notification_thread_id(params: &Value) -> Option<&str> {
    params
        .get("threadId")
        .and_then(Value::as_str)
        .or_else(|| params.get("thread_id").and_then(Value::as_str))
        .or_else(|| {
            params
                .get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
        })
}

fn notification_turn_id(params: &Value) -> Option<&str> {
    params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.get("turn_id").and_then(Value::as_str))
        .or_else(|| {
            params
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
        })
}

fn turns_from_snapshot(snapshot: &Value) -> Vec<&Value> {
    snapshot
        .get("thread")
        .and_then(|thread| thread.get("turns"))
        .and_then(Value::as_array)
        .map(|turns| turns.iter().collect())
        .unwrap_or_default()
}

fn turn_status_from_value(turn: &Value) -> Option<&str> {
    let status = turn.get("status")?;
    status
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| status.as_str())
}

fn final_answer_from_items(items: Option<&Value>) -> Option<&str> {
    items?
        .as_array()?
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
        .filter(|item| item.get("phase").and_then(Value::as_str) == Some("final_answer"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .next_back()
}

fn is_terminal_turn_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "interrupted" | "errored" | "failed" | "cancelled" | "canceled" | "aborted"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;

    use super::*;
    use crate::config::SourceType;

    async fn continue_session_with_connection(
        connection: &mut impl AppServerConnection,
        request: &AgentControllerRequest,
    ) -> Result<AgentControllerSuccess, AgentControllerError> {
        let submit_observer = NoopSubmitObserver;
        super::continue_session_with_connection(connection, request, &submit_observer).await
    }

    #[tokio::test]
    async fn success_path_returns_final_answer_text_without_rewriting() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(4, json!({"turn": {"id": "turn-2"}})),
            notification(
                "item/completed",
                json!({
                    "threadId": "session-1",
                    "turnId": "turn-2",
                    "item": {
                        "type": "agentMessage",
                        "id": "item-2",
                        "phase": "final_answer",
                        "text": "  exact agent result\nwith spacing  "
                    },
                    "completedAtMs": 1779528751000i64
                }),
            ),
        ]);
        let request = controller_request("<@B123> keep this reply exactly");

        let result = continue_session_with_connection(&mut connection, &request)
            .await
            .expect("controller should return final answer");

        assert_eq!(result.result_text(), "  exact agent result\nwith spacing  ");
        assert_eq!(connection.sent_json(1)["method"], "initialized");
        let turn_start = connection.sent_json(4);
        assert_eq!(turn_start["method"], "turn/start");
        assert_eq!(
            turn_start["params"]["input"][0]["text"],
            "<@B123> keep this reply exactly"
        );
        assert!(turn_start["params"].get("effort").is_none());
    }

    #[tokio::test]
    async fn thread_resume_failure_is_before_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            rpc_error(3, "thread not found"),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("thread/resume should fail");

        assert_eq!(error.kind, AgentControllerErrorKind::SessionNotFound);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedBeforeSubmit
        );
        assert_eq!(connection.sent_len(), 4);
    }

    #[tokio::test]
    async fn active_thread_fails_before_submit_without_turn_start() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "running"),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("active thread should fail before turn/start");

        assert_eq!(error.kind, AgentControllerErrorKind::SessionActive);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedBeforeSubmit
        );
        assert_eq!(connection.sent_len(), 3);
        assert_eq!(connection.sent_json(2)["method"], "thread/read");
    }

    #[tokio::test]
    async fn not_loaded_thread_resumes_to_idle_before_turn_start() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "notLoaded"),
            thread_resume_response(3, Some("idle")),
            response(4, json!({"turn": {"id": "turn-2"}})),
            notification(
                "item/completed",
                json!({
                    "threadId": "session-1",
                    "turnId": "turn-2",
                    "item": {
                        "type": "agentMessage",
                        "id": "item-2",
                        "phase": "final_answer",
                        "text": "loaded result"
                    }
                }),
            ),
        ]);
        let request = controller_request("continue");

        let result = continue_session_with_connection(&mut connection, &request)
            .await
            .expect("notLoaded thread should be resumed before turn/start");

        assert_eq!(result.result_text(), "loaded result");
        assert_eq!(connection.sent_json(2)["method"], "thread/read");
        assert_eq!(connection.sent_json(3)["method"], "thread/resume");
        assert_eq!(connection.sent_json(4)["method"], "turn/start");
    }

    #[tokio::test]
    async fn not_loaded_thread_resumes_with_missing_post_resume_status_before_turn_start() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "notLoaded"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(4, json!({"turn": {"id": "turn-2"}})),
            notification(
                "item/completed",
                json!({
                    "threadId": "session-1",
                    "turnId": "turn-2",
                    "item": {
                        "type": "agentMessage",
                        "id": "item-2",
                        "phase": "final_answer",
                        "text": "loaded result"
                    }
                }),
            ),
        ]);
        let request = controller_request("continue");

        let result = continue_session_with_connection(&mut connection, &request)
            .await
            .expect("notLoaded thread should be resumed before turn/start");

        assert_eq!(result.result_text(), "loaded result");
        assert_eq!(connection.sent_json(2)["method"], "thread/read");
        assert_eq!(connection.sent_json(3)["method"], "thread/resume");
        assert_eq!(connection.sent_json(4)["method"], "turn/start");
    }

    #[tokio::test]
    async fn not_loaded_thread_resume_active_fails_before_turn_start() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "notLoaded"),
            thread_resume_response(3, Some("active")),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("post-resume active thread should fail before turn/start");

        assert_eq!(error.kind, AgentControllerErrorKind::SessionActive);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedBeforeSubmit
        );
        assert_eq!(connection.sent_len(), 4);
        assert_eq!(connection.sent_json(3)["method"], "thread/resume");
    }

    #[tokio::test]
    async fn not_loaded_thread_resume_not_ready_status_fails_before_turn_start() {
        for status in ["notLoaded", "systemError", "unknown"] {
            let mut connection = FakeAppServerConnection::new(vec![
                response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
                thread_read_response(2, "notLoaded"),
                thread_resume_response(3, Some(status)),
            ]);
            let request = controller_request("continue");

            let error = continue_session_with_connection(&mut connection, &request)
                .await
                .expect_err("post-resume not-ready status should fail before turn/start");

            assert_eq!(error.kind, AgentControllerErrorKind::SessionNotContinuable);
            assert_eq!(
                error.submit_boundary,
                crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedBeforeSubmit
            );
            assert_eq!(connection.sent_len(), 4, "status {status}");
            assert_eq!(connection.sent_json(3)["method"], "thread/resume");
        }
    }

    #[tokio::test]
    async fn turn_start_rpc_error_is_after_possible_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            rpc_error(4, "active turn is already running"),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("turn/start should fail closed");

        assert_eq!(error.kind, AgentControllerErrorKind::ControllerRejected);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit
        );
        assert_eq!(connection.sent_len(), 5);
    }

    #[tokio::test]
    async fn post_submit_notification_error_reports_unconfirmed_result() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            notification(
                "error",
                json!({
                    "turnId": "turn-2",
                    "error": {
                        "message": "active turn is already running"
                    }
                }),
            ),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("turn/start notification error should fail closed");

        assert_eq!(error.kind, AgentControllerErrorKind::ControllerRejected);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit
        );
        assert!(
            error
                .message
                .contains("Codex App Server reported an error after turn/start was submitted"),
            "unexpected error message: {}",
            error.message
        );
        assert_eq!(connection.sent_len(), 5);
    }

    #[tokio::test]
    async fn ignores_other_thread_or_turn_final_answer_until_current_turn_final_answer() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(4, json!({"turn": {"id": "turn-current"}})),
            notification(
                "item/completed",
                json!({
                    "threadId": "other-session",
                    "turnId": "turn-current",
                    "item": {
                        "type": "agentMessage",
                        "id": "other-thread-final",
                        "phase": "final_answer",
                        "text": "wrong other thread final"
                    },
                    "completedAtMs": 1779528751000i64
                }),
            ),
            notification(
                "item/completed",
                json!({
                    "threadId": "session-1",
                    "turnId": "other-turn",
                    "item": {
                        "type": "agentMessage",
                        "id": "other-turn-final",
                        "phase": "final_answer",
                        "text": "wrong other turn final"
                    },
                    "completedAtMs": 1779528751001i64
                }),
            ),
            notification(
                "item/completed",
                json!({
                    "threadId": "session-1",
                    "turnId": "turn-current",
                    "item": {
                        "type": "agentMessage",
                        "id": "current-final",
                        "phase": "final_answer",
                        "text": "current turn final"
                    },
                    "completedAtMs": 1779528751002i64
                }),
            ),
        ]);
        let request = controller_request("continue");

        let result = continue_session_with_connection(&mut connection, &request)
            .await
            .expect("current turn final answer should win");

        assert_eq!(result.result_text(), "current turn final");
    }

    #[tokio::test]
    async fn current_turn_completed_without_current_final_answer_fails_after_possible_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(4, json!({"turn": {"id": "turn-current"}})),
            notification(
                "item/completed",
                json!({
                    "threadId": "session-1",
                    "turnId": "other-turn",
                    "item": {
                        "type": "agentMessage",
                        "id": "other-turn-final",
                        "phase": "final_answer",
                        "text": "wrong other turn final"
                    },
                    "completedAtMs": 1779528751001i64
                }),
            ),
            notification(
                "turn/completed",
                json!({
                    "threadId": "session-1",
                    "turn": {
                        "id": "turn-current"
                    }
                }),
            ),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("current turn completion without final answer should fail");

        assert_eq!(error.kind, AgentControllerErrorKind::Internal);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit
        );
        assert_eq!(
            error.message.as_ref(),
            "Codex App Server turn completed before final answer"
        );
    }

    #[tokio::test]
    async fn snake_case_notification_ids_match_current_turn() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(4, json!({"turn": {"id": "turn-current"}})),
            notification(
                "item/completed",
                json!({
                    "thread_id": "session-1",
                    "turn_id": "turn-current",
                    "item": {
                        "type": "agentMessage",
                        "id": "current-final",
                        "phase": "final_answer",
                        "text": "snake case final"
                    },
                    "completedAtMs": 1779528751002i64
                }),
            ),
        ]);
        let request = controller_request("continue");

        let result = continue_session_with_connection(&mut connection, &request)
            .await
            .expect("snake_case ids should match current turn");

        assert_eq!(result.result_text(), "snake case final");
    }

    #[tokio::test]
    async fn busy_stream_does_not_starve_thread_status_poll() {
        let mut connection = BusyStreamConnection::new();
        let request = controller_request("continue");

        let result = continue_session_with_connection(&mut connection, &request)
            .await
            .expect("thread snapshot poll should recover the final answer");

        assert_eq!(result.result_text(), "snapshot final");
        assert!(
            connection
                .sent_methods()
                .iter()
                .filter(|method| method.as_str() == "thread/resume")
                .count()
                >= 2,
            "controller should issue a status poll even while stream messages keep arriving"
        );
    }

    #[test]
    fn thread_snapshot_final_answer_marks_current_turn_done() {
        let mut state = TurnStartState::new("session-1".to_string());
        state.turn_id = Some("turn-current".to_string());

        state.observe_thread_snapshot(&json!({
            "thread": {
                "id": "session-1",
                "sessionId": "session-1",
                "status": "idle",
                "turns": [
                    {
                        "id": "turn-other",
                        "status": "completed",
                        "items": [
                            {
                                "type": "agentMessage",
                                "phase": "final_answer",
                                "text": "wrong answer"
                            }
                        ]
                    },
                    {
                        "id": "turn-current",
                        "status": "completed",
                        "items": [
                            {
                                "type": "agentMessage",
                                "phase": "commentary",
                                "text": "progress"
                            },
                            {
                                "type": "agentMessage",
                                "phase": "final_answer",
                                "text": "snapshot final"
                            }
                        ]
                    }
                ]
            }
        }));

        assert_eq!(state.final_answer.as_deref(), Some("snapshot final"));
        assert!(state.turn_completed);
        assert_eq!(
            state.last_observed_turn_status.as_deref(),
            Some("completed")
        );
    }

    #[test]
    fn thread_snapshot_terminal_without_final_marks_current_turn_done() {
        let mut state = TurnStartState::new("session-1".to_string());
        state.turn_id = Some("turn-current".to_string());

        state.observe_thread_snapshot(&json!({
            "thread": {
                "id": "session-1",
                "sessionId": "session-1",
                "status": "idle",
                "turns": [
                    {
                        "id": "turn-current",
                        "status": "interrupted",
                        "items": [
                            {
                                "type": "agentMessage",
                                "phase": "commentary",
                                "text": "partial"
                            }
                        ]
                    }
                ]
            }
        }));

        assert_eq!(state.final_answer, None);
        assert!(state.turn_completed);
        assert_eq!(
            state.last_observed_turn_status.as_deref(),
            Some("interrupted")
        );
    }

    #[tokio::test]
    async fn stream_end_after_turn_start_is_after_possible_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(4, json!({"turn": {"id": "turn-2"}})),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("missing final answer should fail closed");

        assert_eq!(error.kind, AgentControllerErrorKind::Internal);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit
        );
        assert_eq!(connection.sent_len(), 5);
    }

    #[tokio::test]
    async fn mismatched_resumed_thread_is_before_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            thread_read_response(2, "idle"),
            response(
                3,
                json!({"thread": {"id": "other-session", "sessionId": "other-session"}}),
            ),
        ]);
        let request = controller_request("continue");

        let error = continue_session_with_connection(&mut connection, &request)
            .await
            .expect_err("wrong thread binding should fail before submit");

        assert_eq!(error.kind, AgentControllerErrorKind::SessionNotContinuable);
        assert_eq!(
            error.submit_boundary,
            crate::agent_controller::AgentControllerFailureSubmitBoundary::FailedBeforeSubmit
        );
        assert_eq!(connection.sent_len(), 4);
    }

    #[tokio::test]
    async fn start_session_creates_thread_then_starts_first_turn() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.134.0"})),
            response(
                2,
                json!({
                    "thread": {
                        "id": "new-session-1",
                        "sessionId": "new-session-1",
                        "status": {"type": "idle"}
                    }
                }),
            ),
            response(3, json!({"turn": {"id": "turn-new"}})),
            notification(
                "item/completed",
                json!({
                    "threadId": "new-session-1",
                    "turnId": "turn-new",
                    "item": {
                        "type": "agentMessage",
                        "id": "item-final",
                        "phase": "final_answer",
                        "text": "new session result"
                    }
                }),
            ),
        ]);
        let request = session_start_request("Reply OK.");
        let observer = NoopStartObserver;

        let result = start_session_with_connection(&mut connection, &request, &observer)
            .await
            .expect("new session should complete");

        assert_eq!(result.source_session_id, "new-session-1");
        assert_eq!(result.result.result_text(), "new session result");
        assert_eq!(connection.sent_json(2)["method"], "thread/start");
        assert_eq!(
            connection.sent_json(2)["params"]["cwd"],
            "/Users/tester/projects/agents-router"
        );
        assert_eq!(connection.sent_json(3)["method"], "turn/start");
        assert_eq!(
            connection.sent_json(3)["params"]["threadId"],
            "new-session-1"
        );
        assert_eq!(
            connection.sent_json(3)["params"]["input"][0]["text"],
            "Reply OK."
        );
    }

    #[derive(Debug)]
    struct FakeAppServerConnection {
        sent: Vec<String>,
        responses: VecDeque<String>,
    }

    impl FakeAppServerConnection {
        fn new(responses: Vec<String>) -> Self {
            Self {
                sent: Vec::new(),
                responses: responses.into(),
            }
        }

        fn sent_len(&self) -> usize {
            self.sent.len()
        }

        fn sent_json(&self, index: usize) -> Value {
            serde_json::from_str(&self.sent[index]).expect("sent request should be valid JSON")
        }
    }

    impl AppServerConnection for FakeAppServerConnection {
        async fn send_line(&mut self, line: String) -> io::Result<()> {
            self.sent.push(line);
            Ok(())
        }

        async fn read_line(&mut self) -> io::Result<Option<String>> {
            Ok(self.responses.pop_front())
        }
    }

    #[derive(Debug)]
    struct BusyStreamConnection {
        sent: Vec<String>,
        handshake: VecDeque<String>,
        noise_sent: usize,
        poll_response_sent: bool,
    }

    impl BusyStreamConnection {
        fn new() -> Self {
            Self {
                sent: Vec::new(),
                handshake: vec![
                    response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
                    thread_read_response(2, "idle"),
                    response(
                        3,
                        json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
                    ),
                    response(4, json!({"turn": {"id": "turn-current"}})),
                ]
                .into(),
                noise_sent: 0,
                poll_response_sent: false,
            }
        }

        fn sent_methods(&self) -> Vec<String> {
            self.sent
                .iter()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter_map(|value| {
                    value
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        }

        fn last_sent_request(&self) -> Option<Value> {
            self.sent
                .last()
                .and_then(|line| serde_json::from_str::<Value>(line).ok())
        }
    }

    impl AppServerConnection for BusyStreamConnection {
        async fn send_line(&mut self, line: String) -> io::Result<()> {
            self.sent.push(line);
            Ok(())
        }

        async fn read_line(&mut self) -> io::Result<Option<String>> {
            if let Some(response) = self.handshake.pop_front() {
                return Ok(Some(response));
            }

            if !self.poll_response_sent
                && let Some(request) = self.last_sent_request()
                && request.get("method").and_then(Value::as_str) == Some("thread/resume")
                && request
                    .get("id")
                    .and_then(Value::as_u64)
                    .is_some_and(|id| id > 3)
            {
                self.poll_response_sent = true;
                let id = request
                    .get("id")
                    .and_then(Value::as_u64)
                    .expect("poll request id should be present");
                return Ok(Some(response(
                    id,
                    json!({
                        "thread": {
                            "id": "session-1",
                            "sessionId": "session-1",
                            "status": {"type": "idle"},
                            "turns": [
                                {
                                    "id": "turn-current",
                                    "status": {"type": "completed"},
                                    "items": [
                                        {
                                            "type": "agentMessage",
                                            "phase": "final_answer",
                                            "text": "snapshot final"
                                        }
                                    ]
                                }
                            ]
                        }
                    }),
                )));
            }

            if self.noise_sent < 4 {
                self.noise_sent += 1;
                tokio::time::sleep(Duration::from_millis(15)).await;
                return Ok(Some(notification(
                    "turn/started",
                    json!({
                        "threadId": "other-session",
                        "turnId": format!("other-turn-{}", self.noise_sent)
                    }),
                )));
            }

            Ok(None)
        }
    }

    fn response(id: u64, result: Value) -> String {
        json!({
            "id": id,
            "result": result,
        })
        .to_string()
    }

    fn thread_read_response(id: u64, status: &str) -> String {
        thread_response(id, Some(status))
    }

    fn thread_resume_response(id: u64, status: Option<&str>) -> String {
        thread_response(id, status)
    }

    fn thread_response(id: u64, status: Option<&str>) -> String {
        let mut thread = json!({
            "id": "session-1",
            "sessionId": "session-1",
        });
        if let Some(status) = status {
            thread["status"] = json!({
                "type": status
            });
        }
        response(
            id,
            json!({
                "thread": thread
            }),
        )
    }

    fn rpc_error(id: u64, message: &str) -> String {
        json!({
            "id": id,
            "error": {
                "code": -32000,
                "message": message,
            },
        })
        .to_string()
    }

    fn notification(method: &str, params: Value) -> String {
        json!({
            "method": method,
            "params": params,
        })
        .to_string()
    }

    fn controller_request(reply_text: &str) -> AgentControllerRequest {
        AgentControllerRequest {
            controller_kind: AgentControllerKind::CodexAppServer,
            surface_id: "surface-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: SourceType::CodexDesktop,
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            reply_text: reply_text.to_string(),
            provider_event_id_hash: "event-hash".to_string(),
        }
    }

    fn session_start_request(prompt: &str) -> AgentSessionStartRequest {
        AgentSessionStartRequest {
            controller_kind: AgentControllerKind::CodexAppServer,
            surface_id: "surface-new".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: SourceType::CodexDesktop,
            project_path: "/Users/tester/projects/agents-router".to_string(),
            prompt: prompt.to_string(),
            provider_event_id_hash: "event-hash".to_string(),
        }
    }
}
