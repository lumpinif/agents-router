use chrono::Utc;
use tracing::{info, warn};

use crate::agent_controller::{
    AgentControllerAdapter, AgentControllerError, AgentControllerErrorKind,
    AgentControllerFailureSubmitBoundary, AgentControllerSubmitFuture, AgentSessionStartObserver,
    AgentSessionStartRequest, ProviderThreadReplyAdapter, ProviderThreadReplyRequest,
    codex_app_server::CodexAppServerController, send_provider_thread_reply_with_retry,
};
use crate::agent_integration_catalog::AgentControllerKind;
use crate::bridge_binding_ledger::{BridgeBindingLedgerStore, ThreadSessionBindingInput};
use crate::bridge_control::{BridgeControlReply, BridgeNewSessionCommand};
use crate::config::{ProviderType, SourceType, ValidatedConfig};
use crate::execution_scope_guard::{ExecutionScopeKey, ExecutionScopeLease};
use crate::new_session_dispatcher::{NewSessionDispatcher, NewSessionWork};
use crate::providers::feishu_lark::FeishuLarkProvider;
use crate::providers::feishu_lark::{
    FeishuLarkLazyStreamingThreadReplyParts, FeishuLarkStreamingThreadReplyRequest,
};
use crate::providers::feishu_lark_control::dispatch_feishu_lark_control_reply;
use crate::response_surface_runtime::{
    response_surface_route_binding_hash, route_allows_response_surface_project,
};
use crate::runtime::RuntimeState;

const NEW_SESSION_BUSY_NOTICE_TEXT: &str =
    "This project is already starting a new Codex thread. Try again after it finishes.";
const NEW_SESSION_SUBMITTED_UNKNOWN_NOTICE_TEXT: &str = "Codex received the new task, but Agents Router could not find a final answer to send back. To avoid running the same task twice, it will not retry automatically. Please check Codex Desktop.";

#[derive(Clone)]
pub(crate) struct FeishuLarkNewSessionDispatcher {
    runtime_state: RuntimeState,
}

impl FeishuLarkNewSessionDispatcher {
    pub(crate) fn new(runtime_state: RuntimeState) -> Self {
        Self { runtime_state }
    }
}

impl NewSessionDispatcher for FeishuLarkNewSessionDispatcher {
    fn dispatch(&self, work: NewSessionWork) -> anyhow::Result<()> {
        let runtime_state = self.runtime_state.clone();
        let scope = ExecutionScopeKey::from_parts(
            work.command.source_type.clone(),
            work.command.source_id.clone(),
            format!("new-session:{}", work.command.project_path),
        );
        let Some(scope_lease) = runtime_state
            .execution_scope_guard()
            .try_acquire(scope.clone())?
        else {
            info!(
                source.type = %scope.source_type(),
                source.id = %scope.source_id(),
                source.session.id = %scope.source_session_id(),
                event.hash = %work.command.provider_event_id_hash,
                event = "provider_control.new_session.scope_busy",
            );
            dispatch_feishu_lark_control_reply(
                runtime_state,
                control_reply_from_command(&work.command, NEW_SESSION_BUSY_NOTICE_TEXT),
            );
            return Ok(());
        };

        info!(
            provider.id = %work.command.provider_id,
            provider.thread.id = %work.command.provider_thread_id,
            project.path = %work.command.project_path,
            event.hash = %work.command.provider_event_id_hash,
            event = "provider_control.new_session.dispatched",
        );
        tokio::spawn(async move {
            let event_hash = work.command.provider_event_id_hash.clone();
            if let Err(error) = run_lark_new_session_worker(runtime_state, work, scope_lease).await
            {
                warn!(
                    event.hash = %event_hash,
                    error = %error,
                    event = "provider_control.new_session.worker.failed",
                );
            }
        });
        Ok(())
    }
}

async fn run_lark_new_session_worker(
    runtime_state: RuntimeState,
    work: NewSessionWork,
    _scope_lease: ExecutionScopeLease,
) -> anyhow::Result<()> {
    let command = work.command;
    let dispatch_latency_ms = (Utc::now() - work.dispatched_at).num_milliseconds();
    info!(
        provider.id = %command.provider_id,
        provider.thread.id = %command.provider_thread_id,
        project.path = %command.project_path,
        event.hash = %command.provider_event_id_hash,
        dispatch.latency_ms = dispatch_latency_ms,
        event = "provider_control.new_session.worker.started",
    );

    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&command.provider_id) else {
        warn!(
            provider.id = %command.provider_id,
            event.hash = %command.provider_event_id_hash,
            event = "provider_control.new_session.skipped",
            reason = "current_provider_missing",
        );
        return Ok(());
    };
    if current_provider.provider_type() != ProviderType::FeishuLark {
        warn!(
            provider.id = %command.provider_id,
            current.provider.type = %current_provider.provider_type().as_str(),
            event.hash = %command.provider_event_id_hash,
            event = "provider_control.new_session.skipped",
            reason = "current_provider_type_mismatch",
        );
        return Ok(());
    }

    let provider_reply = FeishuLarkProvider::from_config(current_provider)?;
    let Some(source_type) = SourceType::from_signal_value(&command.source_type) else {
        send_new_session_reply(
            &provider_reply,
            &command,
            "This source cannot start new Codex threads.",
        )
        .await?;
        return Ok(());
    };

    let streaming_reply = FeishuLarkLazyStreamingThreadReplyParts::new(
        provider_reply,
        streaming_request_from_command(&command),
    );
    if let Err(error) = streaming_reply.start().await {
        warn!(
            provider.id = %error.provider_id,
            provider.type = %error.provider_type,
            provider.thread.id = %command.provider_thread_id,
            event.hash = %command.provider_event_id_hash,
            http.status = error.http_status,
            error.retriable = error.retriable,
            error = %error.message,
            event = "provider_control.new_session.streaming_start.failed",
        );
    }
    let progress_observer = streaming_reply.progress_observer();
    let controller = CodexAppServerController::new();
    let observer = LarkNewSessionStartObserver {
        bridge_binding_ledger_store: runtime_state.bridge_binding_ledger(),
        command: command.clone(),
        route_binding_hash: route_binding_hash_for_new_session(&snapshot.config, &command),
    };
    let result = controller
        .start_session_with_observers(
            AgentSessionStartRequest {
                controller_kind: AgentControllerKind::CodexAppServer,
                surface_id: bridge_new_session_surface_id(&command),
                source_id: command.source_id.clone(),
                source_type,
                project_path: command.project_path.clone(),
                prompt: command.prompt.clone(),
                provider_event_id_hash: command.provider_event_id_hash.clone(),
            },
            &observer,
            &progress_observer,
        )
        .await;

    match result {
        Ok(success) => {
            info!(
                provider.id = %command.provider_id,
                provider.thread.id = %command.provider_thread_id,
                source.session.id = %success.source_session_id,
                event.hash = %command.provider_event_id_hash,
                event = "provider_control.new_session.controller_succeeded",
            );
            send_new_session_reply(&streaming_reply, &command, success.result.result_text())
                .await?;
        }
        Err(error) => {
            warn!(
                provider.id = %command.provider_id,
                provider.thread.id = %command.provider_thread_id,
                controller.error.kind = ?error.kind,
                submit.boundary = ?error.submit_boundary,
                event.hash = %command.provider_event_id_hash,
                error = %error.message,
                event = "provider_control.new_session.controller_failed",
            );
            send_new_session_reply(&streaming_reply, &command, new_session_failure_text(&error))
                .await?;
        }
    }

    Ok(())
}

fn streaming_request_from_command(
    command: &BridgeNewSessionCommand,
) -> FeishuLarkStreamingThreadReplyRequest {
    FeishuLarkStreamingThreadReplyRequest {
        provider_id: command.provider_id.clone(),
        provider_type: command.provider_type.clone(),
        provider_account_id: command.provider_account_id.clone(),
        provider_conversation_id: command.provider_conversation_id.clone(),
        provider_thread_id: command.provider_thread_id.clone(),
        surface_id: bridge_new_session_surface_id(command),
        provider_event_id_hash: command.provider_event_id_hash.clone(),
    }
}

struct LarkNewSessionStartObserver {
    bridge_binding_ledger_store: BridgeBindingLedgerStore,
    command: BridgeNewSessionCommand,
    route_binding_hash: Option<String>,
}

impl AgentSessionStartObserver for LarkNewSessionStartObserver {
    fn thread_started<'a>(&'a self, source_session_id: &'a str) -> AgentControllerSubmitFuture<'a> {
        Box::pin(async move {
            self.bridge_binding_ledger_store
                .update(|ledger| {
                    ledger.bind_thread_session_at(
                        ThreadSessionBindingInput {
                            provider_id: self.command.provider_id.clone(),
                            provider_type: self.command.provider_type.clone(),
                            provider_account_id: self.command.provider_account_id.clone(),
                            provider_conversation_id: self.command.provider_conversation_id.clone(),
                            provider_thread_id: self.command.provider_thread_id.clone(),
                            project_path: self.command.project_path.clone(),
                            source_id: self.command.source_id.clone(),
                            source_type: self.command.source_type.clone(),
                            source_session_id: source_session_id.to_string(),
                            route_binding_hash: self.route_binding_hash.clone(),
                        },
                        Utc::now(),
                    )?;
                    Ok(())
                })
                .await
        })
    }

    fn submitted_possible<'a>(
        &'a self,
        source_session_id: &'a str,
        source_turn_id: &'a str,
    ) -> AgentControllerSubmitFuture<'a> {
        Box::pin(async move {
            info!(
                provider.id = %self.command.provider_id,
                provider.thread.id = %self.command.provider_thread_id,
                source.session.id = %source_session_id,
                source.turn.id = %source_turn_id,
                event.hash = %self.command.provider_event_id_hash,
                event = "provider_control.new_session.submitted_possible",
            );
            Ok(())
        })
    }
}

fn route_binding_hash_for_new_session(
    config: &ValidatedConfig,
    command: &BridgeNewSessionCommand,
) -> Option<String> {
    config
        .routes
        .iter()
        .find(|route| {
            route_allows_response_surface_project(
                route,
                &command.source_id,
                &command.provider_id,
                &command.project_path,
            )
        })
        .map(response_surface_route_binding_hash)
}

async fn send_new_session_reply(
    provider_reply: &dyn ProviderThreadReplyAdapter,
    command: &BridgeNewSessionCommand,
    text: &str,
) -> anyhow::Result<()> {
    if provider_reply.provider_id() != command.provider_id
        || provider_reply.provider_type() != command.provider_type
    {
        anyhow::bail!("provider thread reply adapter does not match new session provider");
    }

    info!(
        provider.id = %command.provider_id,
        provider.type = %command.provider_type,
        provider.thread.id = %command.provider_thread_id,
        event.hash = %command.provider_event_id_hash,
        event = "provider_control.new_session.reply.started",
    );
    let success = send_provider_thread_reply_with_retry(
        provider_reply,
        ProviderThreadReplyRequest {
            provider_id: command.provider_id.clone(),
            provider_type: command.provider_type.clone(),
            provider_account_id: command.provider_account_id.clone(),
            provider_conversation_id: command.provider_conversation_id.clone(),
            provider_thread_id: command.provider_thread_id.clone(),
            surface_id: bridge_new_session_surface_id(command),
            provider_event_id_hash: command.provider_event_id_hash.clone(),
            text: text.to_string(),
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!("{}", error.message))?;
    info!(
        provider.id = %command.provider_id,
        provider.type = %command.provider_type,
        provider.thread.id = %command.provider_thread_id,
        provider.reply.message_id = success.provider_reply_message_id.as_deref(),
        event.hash = %command.provider_event_id_hash,
        event = "provider_control.new_session.reply.succeeded",
    );
    Ok(())
}

fn new_session_failure_text(error: &AgentControllerError) -> &'static str {
    if error.submit_boundary == AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit {
        return NEW_SESSION_SUBMITTED_UNKNOWN_NOTICE_TEXT;
    }

    match error.kind {
        AgentControllerErrorKind::ControllerUnavailable => {
            "Codex Desktop is not available right now. Open Codex Desktop and try again."
        }
        AgentControllerErrorKind::SessionActive => {
            "Codex is still working on this project. Try again after it finishes."
        }
        AgentControllerErrorKind::SessionNotFound
        | AgentControllerErrorKind::SessionNotContinuable
        | AgentControllerErrorKind::ControllerRejected
        | AgentControllerErrorKind::Timeout
        | AgentControllerErrorKind::Internal => "I couldn't start a new Codex thread.",
    }
}

fn control_reply_from_command(command: &BridgeNewSessionCommand, text: &str) -> BridgeControlReply {
    BridgeControlReply {
        provider_id: command.provider_id.clone(),
        provider_type: command.provider_type.clone(),
        provider_account_id: command.provider_account_id.clone(),
        provider_conversation_id: command.provider_conversation_id.clone(),
        provider_thread_id: command.provider_thread_id.clone(),
        provider_event_id_hash: command.provider_event_id_hash.clone(),
        text: text.to_string(),
    }
}

fn bridge_new_session_surface_id(command: &BridgeNewSessionCommand) -> String {
    format!(
        "bridge-new-session:{}:{}",
        command.provider_id, command.provider_event_id_hash
    )
}
