use chrono::Utc;
use tracing::{debug, info, warn};

use crate::agent_controller::{
    AgentControllerClosedLoopDecision, AgentControllerPolicyOverrides, AgentControllerRuntime,
    ProviderThreadReplyAdapter, ProviderThreadReplyRequest,
    codex_app_server::CodexAppServerController, send_provider_thread_reply_with_retry,
};
use crate::continuation_dispatcher::{ClaimedContinuationWork, ContinuationDispatcher};
use crate::execution_scope_guard::{ExecutionScopeKey, ExecutionScopeLease};
use crate::provider_inbound::ProviderInboundReady;
use crate::providers::feishu_lark::FeishuLarkProvider;
use crate::response_surface_ledger::{InboundEventDedupInput, ResponseSurfaceLedgerStore};
use crate::runtime::RuntimeState;

const SOURCE_SESSION_BUSY_NOTICE_TEXT: &str =
    "This agent thread is still running. Reply again after the current turn finishes.";

#[derive(Clone)]
pub(crate) struct FeishuLarkContinuationDispatcher {
    runtime_state: RuntimeState,
}

impl FeishuLarkContinuationDispatcher {
    pub(crate) fn new(runtime_state: RuntimeState) -> Self {
        Self { runtime_state }
    }
}

impl ContinuationDispatcher for FeishuLarkContinuationDispatcher {
    fn dispatch(&self, work: ClaimedContinuationWork) -> anyhow::Result<()> {
        let runtime_state = self.runtime_state.clone();
        let scope = ExecutionScopeKey::from_ready(&work.ready);
        let Some(scope_lease) = runtime_state
            .execution_scope_guard()
            .try_acquire(scope.clone())?
        else {
            info!(
                source.type = %scope.source_type(),
                source.id = %scope.source_id(),
                source.session.id = %scope.source_session_id(),
                surface.id = %work.ready.surface.surface_id,
                event.hash = %work.ready.provider_event_id_hash,
                event = "response_surface.continuation.scope_busy",
            );
            tokio::spawn(async move {
                let surface_id = work.ready.surface.surface_id.clone();
                let event_hash = work.ready.provider_event_id_hash.clone();
                if let Err(error) = send_lark_source_session_busy_notice(runtime_state, work).await
                {
                    warn!(
                        surface.id = %surface_id,
                        event.hash = %event_hash,
                        error = %error,
                        event = "response_surface.continuation.scope_busy_notice.failed",
                    );
                }
            });
            return Ok(());
        };

        info!(
            surface.id = %work.ready.surface.surface_id,
            event.hash = %work.ready.provider_event_id_hash,
            event = "response_surface.continuation.dispatched",
        );
        tokio::spawn(async move {
            let surface_id = work.ready.surface.surface_id.clone();
            let event_hash = work.ready.provider_event_id_hash.clone();
            if let Err(error) =
                run_claimed_lark_continuation_worker(runtime_state, work, scope_lease).await
            {
                warn!(
                    surface.id = %surface_id,
                    event.hash = %event_hash,
                    error = %error,
                    event = "response_surface.continuation.worker.failed",
                );
            }
        });
        Ok(())
    }
}

async fn run_claimed_lark_continuation_worker(
    runtime_state: RuntimeState,
    work: ClaimedContinuationWork,
    _scope_lease: ExecutionScopeLease,
) -> anyhow::Result<()> {
    let ready = work.ready;
    let dispatch_latency_ms = (Utc::now() - work.dispatched_at).num_milliseconds();
    info!(
        surface.id = %ready.surface.surface_id,
        event.hash = %ready.provider_event_id_hash,
        dispatch.latency_ms = dispatch_latency_ms,
        event = "response_surface.continuation.worker.started",
    );

    let ledger_store = runtime_state.response_surface_ledger();
    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&ready.reply.provider_id) else {
        release_claimed_lark_event(&ledger_store, &ready).await?;
        warn!(
            provider.id = %ready.reply.provider_id,
            surface.id = %ready.surface.surface_id,
            event.hash = %ready.provider_event_id_hash,
            event = "feishu_lark.long_connection.live.event.skipped",
            reason = "current_provider_missing",
        );
        return Ok(());
    };

    let provider_reply = match FeishuLarkProvider::from_config(current_provider) {
        Ok(provider_reply) => provider_reply,
        Err(error) => {
            release_claimed_lark_event(&ledger_store, &ready).await?;
            warn!(
                provider.id = %ready.reply.provider_id,
                surface.id = %ready.surface.surface_id,
                event.hash = %ready.provider_event_id_hash,
                error = %error,
                event = "feishu_lark.long_connection.live.event.skipped",
                reason = "provider_reply_adapter_unavailable",
            );
            return Ok(());
        }
    };

    let controller = CodexAppServerController::new();
    let controller_runtime = AgentControllerRuntime::new(vec![&controller]);
    let closed_loop = controller_runtime
        .run_claimed_inbound_continuation_closed_loop_with_store(
            &snapshot.config,
            &ledger_store,
            ready.clone(),
            &provider_reply,
            Utc::now(),
            AgentControllerPolicyOverrides::default(),
        )
        .await?;

    log_live_closed_loop_decision(&ready.surface.surface_id, &closed_loop);
    Ok(())
}

async fn send_lark_source_session_busy_notice(
    runtime_state: RuntimeState,
    work: ClaimedContinuationWork,
) -> anyhow::Result<()> {
    let ready = work.ready;
    let ledger_store = runtime_state.response_surface_ledger();
    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&ready.reply.provider_id) else {
        release_claimed_lark_event(&ledger_store, &ready).await?;
        warn!(
            provider.id = %ready.reply.provider_id,
            surface.id = %ready.surface.surface_id,
            event.hash = %ready.provider_event_id_hash,
            event = "feishu_lark.long_connection.live.event.skipped",
            reason = "current_provider_missing",
        );
        return Ok(());
    };

    let provider_reply = match FeishuLarkProvider::from_config(current_provider) {
        Ok(provider_reply) => provider_reply,
        Err(error) => {
            release_claimed_lark_event(&ledger_store, &ready).await?;
            warn!(
                provider.id = %ready.reply.provider_id,
                surface.id = %ready.surface.surface_id,
                event.hash = %ready.provider_event_id_hash,
                error = %error,
                event = "feishu_lark.long_connection.live.event.skipped",
                reason = "provider_reply_adapter_unavailable",
            );
            return Ok(());
        }
    };

    if let Err(error) =
        send_lark_thread_reply_text(&provider_reply, &ready, SOURCE_SESSION_BUSY_NOTICE_TEXT).await
    {
        release_claimed_lark_event(&ledger_store, &ready).await?;
        return Err(error);
    }
    ledger_store
        .update(|ledger| {
            ledger.record_failed_notified_inbound_event_at(
                inbound_event_dedup_input(&ready),
                Utc::now(),
            )?;
            Ok(())
        })
        .await?;
    info!(
        surface.id = %ready.surface.surface_id,
        event.hash = %ready.provider_event_id_hash,
        event = "response_surface.continuation.scope_busy_notice.sent",
    );
    Ok(())
}

async fn send_lark_thread_reply_text(
    provider_reply: &dyn ProviderThreadReplyAdapter,
    ready: &ProviderInboundReady,
    text: &str,
) -> anyhow::Result<()> {
    if provider_reply.provider_id() != ready.reply.provider_id
        || provider_reply.provider_type() != ready.reply.provider_type
    {
        anyhow::bail!("provider thread reply adapter does not match inbound provider");
    }

    info!(
        surface.id = %ready.surface.surface_id,
        provider.id = %ready.reply.provider_id,
        provider.type = %ready.reply.provider_type,
        provider.thread.id = %ready.reply.provider_thread_id,
        event.hash = %ready.provider_event_id_hash,
        event = "provider_thread_result_reply.started",
    );
    let reply_result = send_provider_thread_reply_with_retry(
        provider_reply,
        ProviderThreadReplyRequest {
            provider_id: ready.reply.provider_id.clone(),
            provider_type: ready.reply.provider_type.clone(),
            provider_account_id: ready.reply.provider_account_id.clone(),
            provider_conversation_id: ready.reply.provider_conversation_id.clone(),
            provider_thread_id: ready.reply.provider_thread_id.clone(),
            surface_id: ready.surface.surface_id.clone(),
            provider_event_id_hash: ready.provider_event_id_hash.clone(),
            text: text.to_string(),
        },
    )
    .await;
    let success = match reply_result {
        Ok(success) => success,
        Err(error) => {
            warn!(
                surface.id = %ready.surface.surface_id,
                provider.id = %ready.reply.provider_id,
                provider.type = %ready.reply.provider_type,
                provider.thread.id = %ready.reply.provider_thread_id,
                event.hash = %ready.provider_event_id_hash,
                error = %error.message,
                event = "provider_thread_result_reply.failed",
            );
            anyhow::bail!("{}", error.message);
        }
    };
    info!(
        surface.id = %ready.surface.surface_id,
        provider.id = %ready.reply.provider_id,
        provider.type = %ready.reply.provider_type,
        provider.thread.id = %ready.reply.provider_thread_id,
        provider.reply.message_id = success.provider_reply_message_id.as_deref(),
        event.hash = %ready.provider_event_id_hash,
        event = "provider_thread_result_reply.succeeded",
    );
    Ok(())
}

async fn release_claimed_lark_event(
    ledger_store: &ResponseSurfaceLedgerStore,
    ready: &ProviderInboundReady,
) -> anyhow::Result<()> {
    ledger_store
        .update(|ledger| {
            ledger.release_inbound_event_claim_at(inbound_event_dedup_input(ready))?;
            Ok(())
        })
        .await
}

fn inbound_event_dedup_input(ready: &ProviderInboundReady) -> InboundEventDedupInput {
    InboundEventDedupInput {
        provider_id: ready.reply.provider_id.clone(),
        provider_type: ready.reply.provider_type.clone(),
        provider_account_id: ready.reply.provider_account_id.clone(),
        provider_conversation_id: ready.reply.provider_conversation_id.clone(),
        provider_event_id: ready.reply.provider_event_id.clone(),
        surface_id: ready.surface.surface_id.clone(),
    }
}

fn log_live_closed_loop_decision(surface_id: &str, decision: &AgentControllerClosedLoopDecision) {
    match decision {
        AgentControllerClosedLoopDecision::Completed(completion) => {
            info!(
                surface.id = %surface_id,
                provider.reply.message_id = completion.provider_reply_message_id.as_deref(),
                event = "feishu_lark.long_connection.live.closed_loop.completed",
            );
        }
        AgentControllerClosedLoopDecision::ControllerFailed(error) => {
            warn!(
                surface.id = %surface_id,
                controller.kind = ?error.kind,
                submit.boundary = ?error.submit_boundary,
                error = %error.message,
                event = "feishu_lark.long_connection.live.closed_loop.controller_failed",
            );
        }
        AgentControllerClosedLoopDecision::ProviderThreadReplyFailed(failure) => {
            warn!(
                surface.id = %surface_id,
                error = %failure.error.message,
                event = "feishu_lark.long_connection.live.closed_loop.provider_reply_failed",
            );
        }
        AgentControllerClosedLoopDecision::Skipped(reason) => {
            debug!(
                surface.id = %surface_id,
                reason = ?reason,
                event = "feishu_lark.long_connection.live.closed_loop.skipped",
            );
        }
    }
}
