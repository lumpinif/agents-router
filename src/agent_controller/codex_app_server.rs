use std::process::Stdio;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::agent_controller::{
    AgentControllerAdapter, AgentControllerError, AgentControllerErrorKind, AgentControllerFuture,
    AgentControllerRequest, AgentControllerSuccess,
};
use crate::agent_integration_catalog::AgentControllerKind;

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
            let mut connection = ProcessAppServerConnection::spawn(&self.command)
                .await
                .map_err(|message| {
                    AgentControllerError::failed_before_submit(
                        &request,
                        AgentControllerErrorKind::ControllerUnavailable,
                        message,
                    )
                })?;

            let result = continue_session_with_connection(&mut connection, &request).await;
            connection.shutdown().await;
            result
        })
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
) -> Result<AgentControllerSuccess, AgentControllerError> {
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
    send_notification(
        connection,
        request,
        "initialized",
        json!({}),
        SubmitBoundary::BeforeSubmit,
    )
    .await?;

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
    ensure_resumed_thread_matches_request(request, &resume_result)?;

    let mut turn_state = TurnStartState::new(request.source_session_id.clone());
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

    wait_for_final_answer(connection, request, &mut turn_state).await
}

async fn wait_for_final_answer(
    connection: &mut impl AppServerConnection,
    request: &AgentControllerRequest,
    turn_state: &mut TurnStartState,
) -> Result<AgentControllerSuccess, AgentControllerError> {
    loop {
        if let Some(message) = turn_state.error_message.take() {
            return Err(error_after_possible_submit(
                request,
                AgentControllerErrorKind::ControllerRejected,
                format!("Codex App Server rejected turn/start: {message}"),
            ));
        }
        if let Some(final_answer) = turn_state.final_answer.take() {
            return AgentControllerSuccess::from_result_text(final_answer).ok_or_else(|| {
                error_after_possible_submit(
                    request,
                    AgentControllerErrorKind::Internal,
                    "Codex App Server completed without result text",
                )
            });
        }
        if turn_state.turn_completed {
            return Err(error_after_possible_submit(
                request,
                AgentControllerErrorKind::Internal,
                "Codex App Server turn completed before final answer",
            ));
        }

        let message =
            read_message(connection, request, SubmitBoundary::AfterPossibleSubmit).await?;
        turn_state.observe(&message);
    }
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

    loop {
        let message = read_message(connection, request, boundary).await?;
        if let Some(state) = turn_state.as_mut() {
            state.observe(&message);
            if let Some(message) = state.error_message.take() {
                return Err(error_after_possible_submit(
                    request,
                    AgentControllerErrorKind::ControllerRejected,
                    format!("Codex App Server rejected turn/start: {message}"),
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
) -> Result<(), AgentControllerError> {
    let thread = resume_result.get("thread").ok_or_else(|| {
        AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            "Codex App Server thread/resume response did not include thread facts",
        )
    })?;
    let thread_id = thread.get("id").and_then(Value::as_str).ok_or_else(|| {
        AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            "Codex App Server thread/resume response did not include thread.id",
        )
    })?;

    if thread_id != request.source_session_id {
        return Err(AgentControllerError::failed_before_submit(
            request,
            AgentControllerErrorKind::SessionNotContinuable,
            format!(
                "Codex App Server resumed thread `{thread_id}` instead of requested source session"
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
                "Codex App Server resumed session `{session_id}` instead of requested source session"
            ),
        ));
    }

    Ok(())
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
}

impl TurnStartState {
    fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            turn_id: None,
            final_answer: None,
            error_message: None,
            turn_completed: false,
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
    params.get("threadId").and_then(Value::as_str)
}

fn notification_turn_id(params: &Value) -> Option<&str> {
    params.get("turnId").and_then(Value::as_str).or_else(|| {
        params
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;

    use super::*;
    use crate::config::SourceType;

    #[tokio::test]
    async fn success_path_returns_final_answer_text_without_rewriting() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            response(
                2,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(3, json!({"turn": {"id": "turn-2"}})),
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
        let turn_start = connection.sent_json(3);
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
            rpc_error(2, "thread not found"),
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
        assert_eq!(connection.sent_len(), 3);
    }

    #[tokio::test]
    async fn turn_start_rpc_error_is_after_possible_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            response(
                2,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            rpc_error(3, "active turn is already running"),
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
        assert_eq!(connection.sent_len(), 4);
    }

    #[tokio::test]
    async fn turn_start_notification_error_is_after_possible_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            response(
                2,
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
        assert_eq!(connection.sent_len(), 4);
    }

    #[tokio::test]
    async fn ignores_other_thread_or_turn_final_answer_until_current_turn_final_answer() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            response(
                2,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(3, json!({"turn": {"id": "turn-current"}})),
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
            response(
                2,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(3, json!({"turn": {"id": "turn-current"}})),
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
    }

    #[tokio::test]
    async fn stream_end_after_turn_start_is_after_possible_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            response(
                2,
                json!({"thread": {"id": "session-1", "sessionId": "session-1"}}),
            ),
            response(3, json!({"turn": {"id": "turn-2"}})),
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
        assert_eq!(connection.sent_len(), 4);
    }

    #[tokio::test]
    async fn mismatched_resumed_thread_is_before_submit() {
        let mut connection = FakeAppServerConnection::new(vec![
            response(1, json!({"userAgent": "Codex Desktop/0.130.0"})),
            response(
                2,
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
        assert_eq!(connection.sent_len(), 3);
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

    fn response(id: u64, result: Value) -> String {
        json!({
            "id": id,
            "result": result,
        })
        .to_string()
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
}
