use std::future::Future;
use std::pin::Pin;

use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::agent_integration_catalog::{
    AgentControllerKind, AgentIntegrationDescriptor, agent_integration_for_source,
};
use crate::config::{RouteConfig, SourceType, ValidatedConfig};
use crate::provider_catalog::{ProviderModeCapability, provider_config_mode_capability};
use crate::provider_inbound::ProviderInboundReady;
use crate::response_surface_ledger::{
    InboundEventDedupInput, ProcessedInboundEventDecision, ResponseSurfaceLedger,
};
use crate::response_surface_policy::{
    ResponseSurfaceDeliveryReceipt, ResponseSurfacePolicyDecision, ResponseSurfacePolicyInput,
    ResponseSurfacePolicySkipReason, ResponseSurfacePolicySourceFacts,
    evaluate_response_surface_policy,
};

pub mod codex_app_server;

pub type AgentControllerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AgentControllerSuccess, AgentControllerError>> + Send + 'a>>;

pub type ProviderThreadReplyFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<ProviderThreadReplySuccess, ProviderThreadReplyError>>
            + Send
            + 'a,
    >,
>;

pub trait AgentControllerAdapter: Send + Sync {
    fn controller_kind(&self) -> AgentControllerKind;
    fn continue_session<'a>(&'a self, request: AgentControllerRequest)
    -> AgentControllerFuture<'a>;
}

pub trait ProviderThreadReplyAdapter: Send + Sync {
    fn provider_id(&self) -> &str;
    fn provider_type(&self) -> &str;
    fn send_thread_reply<'a>(
        &'a self,
        request: ProviderThreadReplyRequest,
    ) -> ProviderThreadReplyFuture<'a>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentControllerRequest {
    pub controller_kind: AgentControllerKind,
    pub surface_id: String,
    pub source_id: String,
    pub source_type: SourceType,
    pub source_session_id: String,
    pub source_turn_id: Option<String>,
    pub reply_text: String,
    pub provider_event_id_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentControllerSuccess {
    result_text: String,
}

impl AgentControllerSuccess {
    pub fn from_result_text(result_text: impl Into<String>) -> Option<Self> {
        let result_text = result_text.into();
        if result_text.trim().is_empty() {
            return None;
        }

        Some(Self { result_text })
    }

    pub fn result_text(&self) -> &str {
        &self.result_text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderThreadReplyRequest {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub surface_id: String,
    pub provider_event_id_hash: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderThreadReplySuccess {
    pub provider_reply_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderThreadReplyError {
    pub provider_id: String,
    pub provider_type: String,
    pub surface_id: String,
    pub provider_event_id_hash: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentControllerError {
    pub kind: AgentControllerErrorKind,
    pub submit_boundary: AgentControllerFailureSubmitBoundary,
    pub controller_kind: AgentControllerKind,
    pub surface_id: String,
    pub source_id: String,
    pub source_type: SourceType,
    pub source_session_id: String,
    pub provider_event_id_hash: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentControllerFailureSubmitBoundary {
    FailedBeforeSubmit,
    FailedAfterPossibleSubmit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentControllerErrorKind {
    ControllerUnavailable,
    SessionNotFound,
    SessionNotContinuable,
    ControllerRejected,
    Timeout,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentControllerRuntimeDecision {
    Executed(AgentControllerRuntimeExecution),
    Failed(AgentControllerError),
    Skipped(AgentControllerRuntimeSkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentControllerClosedLoopDecision {
    Completed(AgentControllerClosedLoopCompletion),
    ControllerFailed(AgentControllerError),
    ProviderThreadReplyFailed(AgentControllerProviderThreadReplyFailure),
    Skipped(AgentControllerRuntimeSkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentControllerClosedLoopCompletion {
    pub outcome: AgentControllerClosedLoopOutcome,
    pub provider_reply_message_id: Option<String>,
    pub processed: ProcessedInboundEventDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentControllerProviderThreadReplyFailure {
    pub outcome: AgentControllerClosedLoopOutcome,
    pub error: ProviderThreadReplyError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentControllerClosedLoopOutcome {
    ControllerSucceeded(AgentControllerRuntimeExecution),
    ControllerFailed(AgentControllerError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentControllerRuntimeExecution {
    pub controller_kind: AgentControllerKind,
    pub surface_id: String,
    pub source_session_id: String,
    pub result: AgentControllerSuccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentControllerRuntimeSkipReason {
    InvalidSurfaceSourceType,
    CurrentSourceMissing,
    CurrentSourceTypeMismatch,
    CurrentProviderMissing,
    CurrentProviderTypeMismatch,
    CurrentProviderModeMismatch,
    NoMatchingRoute,
    RouteRepliesDisabled,
    RouteFiltersCannotBeRevalidated,
    Policy(ResponseSurfacePolicySkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InboundRouteResolution<'a> {
    Matched {
        route: &'a RouteConfig,
        provider: &'static ProviderModeCapability,
    },
    Skipped(AgentControllerRuntimeSkipReason),
}

enum PreparedInboundContinuation<'a> {
    Ready {
        request: AgentControllerRequest,
        adapter: &'a dyn AgentControllerAdapter,
    },
    Failed(AgentControllerError),
    Skipped(AgentControllerRuntimeSkipReason),
}

pub struct AgentControllerRuntime<'a> {
    adapters: Vec<&'a dyn AgentControllerAdapter>,
}

impl<'a> AgentControllerRuntime<'a> {
    pub fn new(adapters: Vec<&'a dyn AgentControllerAdapter>) -> Self {
        Self { adapters }
    }

    pub async fn run_inbound_continuation(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
    ) -> anyhow::Result<AgentControllerRuntimeDecision> {
        self.run_inbound_continuation_inner(config, ledger, ready, None, None)
            .await
    }

    #[cfg(test)]
    async fn run_inbound_continuation_with_test_policy_facts(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
        agent_integration_override: Option<AgentIntegrationDescriptor>,
        provider_capability_override: Option<&'static ProviderModeCapability>,
    ) -> anyhow::Result<AgentControllerRuntimeDecision> {
        self.run_inbound_continuation_inner(
            config,
            ledger,
            ready,
            agent_integration_override,
            provider_capability_override,
        )
        .await
    }

    async fn run_inbound_continuation_inner(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
        agent_integration_override: Option<AgentIntegrationDescriptor>,
        provider_capability_override: Option<&'static ProviderModeCapability>,
    ) -> anyhow::Result<AgentControllerRuntimeDecision> {
        let prepared = self.prepare_inbound_continuation(
            config,
            &ready,
            agent_integration_override,
            provider_capability_override,
        );

        let (request, adapter) = match prepared {
            PreparedInboundContinuation::Ready { request, adapter } => (request, adapter),
            PreparedInboundContinuation::Failed(error) => {
                release_inbound_claim(ledger, &ready)?;
                return Ok(AgentControllerRuntimeDecision::Failed(error));
            }
            PreparedInboundContinuation::Skipped(reason) => {
                release_inbound_claim(ledger, &ready)?;
                return Ok(AgentControllerRuntimeDecision::Skipped(reason));
            }
        };

        let result = adapter.continue_session(request.clone()).await;

        match result {
            // Once the controller has accepted work, the claim stays processing
            // until the closed loop records provider result reply success.
            Ok(result) => match controller_execution_from_success(&request, &ready, result) {
                Ok(execution) => Ok(AgentControllerRuntimeDecision::Executed(execution)),
                Err(error) => Ok(AgentControllerRuntimeDecision::Failed(error)),
            },
            Err(error) => {
                release_claim_if_controller_failed_before_submit(ledger, &ready, &error)?;
                Ok(AgentControllerRuntimeDecision::Failed(error))
            }
        }
    }

    pub async fn run_inbound_continuation_closed_loop(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
        provider_reply: &dyn ProviderThreadReplyAdapter,
        now: DateTime<Utc>,
    ) -> anyhow::Result<AgentControllerClosedLoopDecision> {
        self.run_inbound_continuation_closed_loop_inner(
            config,
            ledger,
            ready,
            provider_reply,
            now,
            None,
            None,
        )
        .await
    }

    #[cfg(test)]
    async fn run_inbound_continuation_closed_loop_with_test_policy_facts(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
        provider_reply: &dyn ProviderThreadReplyAdapter,
        now: DateTime<Utc>,
        agent_integration_override: Option<AgentIntegrationDescriptor>,
        provider_capability_override: Option<&'static ProviderModeCapability>,
    ) -> anyhow::Result<AgentControllerClosedLoopDecision> {
        self.run_inbound_continuation_closed_loop_inner(
            config,
            ledger,
            ready,
            provider_reply,
            now,
            agent_integration_override,
            provider_capability_override,
        )
        .await
    }

    async fn run_inbound_continuation_closed_loop_inner(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
        provider_reply: &dyn ProviderThreadReplyAdapter,
        now: DateTime<Utc>,
        agent_integration_override: Option<AgentIntegrationDescriptor>,
        provider_capability_override: Option<&'static ProviderModeCapability>,
    ) -> anyhow::Result<AgentControllerClosedLoopDecision> {
        let prepared = self.prepare_inbound_continuation(
            config,
            &ready,
            agent_integration_override,
            provider_capability_override,
        );

        let (request, adapter) = match prepared {
            PreparedInboundContinuation::Ready { request, adapter } => (request, adapter),
            PreparedInboundContinuation::Failed(error) => {
                release_inbound_claim(ledger, &ready)?;
                return Ok(AgentControllerClosedLoopDecision::ControllerFailed(error));
            }
            PreparedInboundContinuation::Skipped(reason) => {
                release_inbound_claim(ledger, &ready)?;
                return Ok(AgentControllerClosedLoopDecision::Skipped(reason));
            }
        };

        let outcome = match adapter.continue_session(request.clone()).await {
            Ok(result) => match controller_execution_from_success(&request, &ready, result) {
                Ok(execution) => AgentControllerClosedLoopOutcome::ControllerSucceeded(execution),
                Err(error) => {
                    release_claim_if_controller_failed_before_submit(ledger, &ready, &error)?;
                    return Ok(AgentControllerClosedLoopDecision::ControllerFailed(error));
                }
            },
            Err(error) => {
                release_claim_if_controller_failed_before_submit(ledger, &ready, &error)?;
                return Ok(AgentControllerClosedLoopDecision::ControllerFailed(error));
            }
        };

        send_result_reply_and_record_processed(ledger, &ready, provider_reply, outcome, now).await
    }

    fn prepare_inbound_continuation(
        &'a self,
        config: &ValidatedConfig,
        ready: &ProviderInboundReady,
        agent_integration_override: Option<AgentIntegrationDescriptor>,
        provider_capability_override: Option<&'static ProviderModeCapability>,
    ) -> PreparedInboundContinuation<'a> {
        let source_type = match SourceType::from_signal_value(&ready.surface.source_type) {
            Some(source_type) => source_type,
            None => {
                return PreparedInboundContinuation::Skipped(
                    AgentControllerRuntimeSkipReason::InvalidSurfaceSourceType,
                );
            }
        };

        let (route, provider) =
            match resolve_inbound_route(config, ready, source_type, provider_capability_override) {
                InboundRouteResolution::Matched { route, provider } => (route, provider),
                InboundRouteResolution::Skipped(reason) => {
                    return PreparedInboundContinuation::Skipped(reason);
                }
            };

        let receipt = delivery_receipt_from_ready(ready);
        let source_facts = ResponseSurfacePolicySourceFacts {
            source_id: &ready.surface.source_id,
            source_type,
            source_session_id: Some(&ready.surface.source_session_id),
        };
        let catalog_integration = agent_integration_for_source(source_facts.source_id, source_type);
        let override_integration = agent_integration_override.as_ref();
        let policy_input = ResponseSurfacePolicyInput {
            check: crate::response_surface_policy::ResponseSurfacePolicyCheck::InboundContinuation,
            provider,
            agent_integration: override_integration.or(catalog_integration),
            route,
            source: crate::response_surface_policy::ResponseSurfacePolicySource::InboundSurface(
                source_facts,
            ),
            delivery_receipt: &receipt,
        };

        let allow = match evaluate_response_surface_policy(policy_input) {
            ResponseSurfacePolicyDecision::Allow(allow) => allow,
            ResponseSurfacePolicyDecision::Skip(reason) => {
                return PreparedInboundContinuation::Skipped(
                    AgentControllerRuntimeSkipReason::Policy(reason),
                );
            }
        };

        let request = AgentControllerRequest {
            controller_kind: allow.controller_kind,
            surface_id: ready.surface.surface_id.clone(),
            source_id: ready.surface.source_id.clone(),
            source_type,
            source_session_id: allow.source_session_id.clone(),
            source_turn_id: ready.surface.source_turn_id.clone(),
            reply_text: ready.reply.reply_text.clone(),
            provider_event_id_hash: ready.provider_event_id_hash.clone(),
        };

        let Some(adapter) = self
            .adapters
            .iter()
            .copied()
            .find(|adapter| adapter.controller_kind() == allow.controller_kind)
        else {
            return PreparedInboundContinuation::Failed(
                AgentControllerError::failed_before_submit(
                    &request,
                    AgentControllerErrorKind::ControllerUnavailable,
                    "agent controller adapter is not registered",
                ),
            );
        };

        PreparedInboundContinuation::Ready { request, adapter }
    }
}

impl AgentControllerError {
    pub fn failed_before_submit(
        request: &AgentControllerRequest,
        kind: AgentControllerErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self::from_request(
            request,
            kind,
            AgentControllerFailureSubmitBoundary::FailedBeforeSubmit,
            message,
        )
    }

    pub fn failed_after_possible_submit(
        request: &AgentControllerRequest,
        kind: AgentControllerErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self::from_request(
            request,
            kind,
            AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit,
            message,
        )
    }

    fn from_request(
        request: &AgentControllerRequest,
        kind: AgentControllerErrorKind,
        submit_boundary: AgentControllerFailureSubmitBoundary,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            submit_boundary,
            controller_kind: request.controller_kind,
            surface_id: request.surface_id.clone(),
            source_id: request.source_id.clone(),
            source_type: request.source_type,
            source_session_id: request.source_session_id.clone(),
            provider_event_id_hash: request.provider_event_id_hash.clone(),
            message: message.into(),
        }
    }
}

impl ProviderThreadReplyError {
    pub fn from_ready(ready: &ProviderInboundReady, message: impl Into<String>) -> Self {
        Self {
            provider_id: ready.reply.provider_id.clone(),
            provider_type: ready.reply.provider_type.clone(),
            surface_id: ready.surface.surface_id.clone(),
            provider_event_id_hash: ready.provider_event_id_hash.clone(),
            message: message.into(),
        }
    }
}

fn controller_execution_from_success(
    request: &AgentControllerRequest,
    ready: &ProviderInboundReady,
    result: AgentControllerSuccess,
) -> Result<AgentControllerRuntimeExecution, AgentControllerError> {
    if result.result_text().trim().is_empty() {
        return Err(AgentControllerError::failed_after_possible_submit(
            request,
            AgentControllerErrorKind::Internal,
            "agent controller completed without result text",
        ));
    }

    Ok(AgentControllerRuntimeExecution {
        controller_kind: request.controller_kind,
        surface_id: ready.surface.surface_id.clone(),
        source_session_id: request.source_session_id.clone(),
        result,
    })
}

async fn send_result_reply_and_record_processed(
    ledger: &mut ResponseSurfaceLedger,
    ready: &ProviderInboundReady,
    provider_reply: &dyn ProviderThreadReplyAdapter,
    outcome: AgentControllerClosedLoopOutcome,
    now: DateTime<Utc>,
) -> anyhow::Result<AgentControllerClosedLoopDecision> {
    if provider_reply.provider_id() != ready.reply.provider_id
        || provider_reply.provider_type() != ready.reply.provider_type
    {
        return Ok(
            AgentControllerClosedLoopDecision::ProviderThreadReplyFailed(
                AgentControllerProviderThreadReplyFailure {
                    outcome,
                    error: ProviderThreadReplyError::from_ready(
                        ready,
                        "provider thread reply adapter does not match inbound provider",
                    ),
                },
            ),
        );
    }

    let request = provider_thread_reply_request(ready, provider_thread_result_text(&outcome));
    let provider_reply_result = provider_reply.send_thread_reply(request).await;
    let provider_reply_success = match provider_reply_result {
        Ok(success) => success,
        Err(error) => {
            return Ok(
                AgentControllerClosedLoopDecision::ProviderThreadReplyFailed(
                    AgentControllerProviderThreadReplyFailure { outcome, error },
                ),
            );
        }
    };

    // agents-router does not set an agent execution hard timeout. The event is
    // processed only after the controller outcome is replied back to the thread.
    let processed = ledger
        .record_processed_inbound_event_at(inbound_event_input(ready), now)
        .context("failed to record processed inbound event")?;

    Ok(AgentControllerClosedLoopDecision::Completed(
        AgentControllerClosedLoopCompletion {
            outcome,
            provider_reply_message_id: provider_reply_success.provider_reply_message_id,
            processed,
        },
    ))
}

fn provider_thread_reply_request(
    ready: &ProviderInboundReady,
    text: impl Into<String>,
) -> ProviderThreadReplyRequest {
    ProviderThreadReplyRequest {
        provider_id: ready.reply.provider_id.clone(),
        provider_type: ready.reply.provider_type.clone(),
        provider_account_id: ready.reply.provider_account_id.clone(),
        provider_conversation_id: ready.reply.provider_conversation_id.clone(),
        provider_thread_id: ready.reply.provider_thread_id.clone(),
        surface_id: ready.surface.surface_id.clone(),
        provider_event_id_hash: ready.provider_event_id_hash.clone(),
        text: text.into(),
    }
}

fn provider_thread_result_text(outcome: &AgentControllerClosedLoopOutcome) -> &str {
    match outcome {
        AgentControllerClosedLoopOutcome::ControllerSucceeded(execution) => {
            execution.result.result_text()
        }
        AgentControllerClosedLoopOutcome::ControllerFailed(error) => {
            controller_failure_notice_text(error.kind)
        }
    }
}

fn controller_failure_notice_text(kind: AgentControllerErrorKind) -> &'static str {
    match kind {
        AgentControllerErrorKind::ControllerUnavailable => {
            "Replies are not available for this agent session right now."
        }
        AgentControllerErrorKind::SessionNotFound => "The original agent session was not found.",
        AgentControllerErrorKind::SessionNotContinuable => {
            "The original agent session cannot continue from this reply."
        }
        AgentControllerErrorKind::ControllerRejected
        | AgentControllerErrorKind::Timeout
        | AgentControllerErrorKind::Internal => {
            "Could not forward this reply to the original agent session."
        }
    }
}

fn resolve_inbound_route<'a>(
    config: &'a ValidatedConfig,
    ready: &ProviderInboundReady,
    source_type: SourceType,
    provider_capability_override: Option<&'static ProviderModeCapability>,
) -> InboundRouteResolution<'a> {
    let Some(source) = config.source(&ready.surface.source_id) else {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::CurrentSourceMissing,
        );
    };
    if source.source_type.as_str() != ready.surface.source_type || source.source_type != source_type
    {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::CurrentSourceTypeMismatch,
        );
    }

    let Some(provider) = config.provider(&ready.reply.provider_id) else {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::CurrentProviderMissing,
        );
    };
    if provider.provider_type().as_str() != ready.reply.provider_type {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::CurrentProviderTypeMismatch,
        );
    }
    let provider_capability =
        provider_capability_override.unwrap_or_else(|| provider_config_mode_capability(provider));
    if provider_capability.provider_type.as_str() != ready.reply.provider_type {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::CurrentProviderTypeMismatch,
        );
    }
    if provider_capability.mode != ready.reply.provider_mode {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::CurrentProviderModeMismatch,
        );
    }

    let mut saw_matching_route = false;
    let mut saw_enabled_route_with_unverifiable_filters = false;

    for route in &config.routes {
        if !route
            .sources
            .iter()
            .any(|source| source == &ready.surface.source_id)
            || !route
                .providers
                .iter()
                .any(|provider| provider == &ready.reply.provider_id)
        {
            continue;
        }

        saw_matching_route = true;
        if route.response_surface.is_disabled() {
            continue;
        }
        if route_has_unverifiable_inbound_filters(route) {
            saw_enabled_route_with_unverifiable_filters = true;
            continue;
        }
        return InboundRouteResolution::Matched {
            route,
            provider: provider_capability,
        };
    }

    if saw_enabled_route_with_unverifiable_filters {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::RouteFiltersCannotBeRevalidated,
        );
    }
    if saw_matching_route {
        return InboundRouteResolution::Skipped(
            AgentControllerRuntimeSkipReason::RouteRepliesDisabled,
        );
    }
    InboundRouteResolution::Skipped(AgentControllerRuntimeSkipReason::NoMatchingRoute)
}

fn route_has_unverifiable_inbound_filters(route: &RouteConfig) -> bool {
    route.minimum_task_duration_minutes.is_some()
        || !route.only_forward_from_project_paths.is_empty()
}

fn delivery_receipt_from_ready(ready: &ProviderInboundReady) -> ResponseSurfaceDeliveryReceipt {
    ResponseSurfaceDeliveryReceipt {
        provider_account_id: Some(ready.surface.provider_account_id.clone()),
        provider_conversation_id: Some(ready.surface.provider_conversation_id.clone()),
        provider_message_id: Some(ready.surface.provider_message_id.clone()),
        provider_thread_id: Some(ready.surface.provider_thread_id.clone()),
    }
}

fn release_inbound_claim(
    ledger: &mut ResponseSurfaceLedger,
    ready: &ProviderInboundReady,
) -> anyhow::Result<()> {
    ledger
        .release_inbound_event_claim_at(inbound_event_input(ready))
        .context("failed to release inbound event claim")?;
    Ok(())
}

fn release_claim_if_controller_failed_before_submit(
    ledger: &mut ResponseSurfaceLedger,
    ready: &ProviderInboundReady,
    error: &AgentControllerError,
) -> anyhow::Result<()> {
    if error.submit_boundary == AgentControllerFailureSubmitBoundary::FailedBeforeSubmit {
        release_inbound_claim(ledger, ready)?;
    }
    Ok(())
}

fn inbound_event_input(ready: &ProviderInboundReady) -> InboundEventDedupInput {
    InboundEventDedupInput {
        provider_id: ready.reply.provider_id.clone(),
        provider_type: ready.reply.provider_type.clone(),
        provider_account_id: ready.reply.provider_account_id.clone(),
        provider_conversation_id: ready.reply.provider_conversation_id.clone(),
        provider_event_id: ready.reply.provider_event_id.clone(),
        surface_id: ready.surface.surface_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::{Duration, TimeZone, Utc};

    use super::*;
    use crate::agent_integration_catalog::{
        AgentIntegrationId, ContinuationCapability, agent_integration_descriptor,
    };
    use crate::config::{
        CONFIG_SCHEMA_VERSION, CliConfig, LogConfig, NotificationConfig, ProviderConfig,
        ProviderConfigDetail, RouteConfig, SlackProviderConfig, SourceConfig, UrlSource,
    };
    use crate::provider_catalog::{ProviderMode, provider_mode_capability};
    use crate::provider_inbound::{
        NormalizedProviderSurfaceReply, ProviderInboundDecision,
        lookup_and_claim_provider_surface_reply,
    };
    use crate::response_surface_ledger::{
        InboundEventClaimDecision, NewResponseSurface, ResponseSurfaceLookupQuery,
        ResponseSurfaceLookupResult,
    };

    #[tokio::test]
    async fn planned_catalog_integration_does_not_call_controller() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                None,
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert_eq!(
            decision,
            AgentControllerRuntimeDecision::Skipped(AgentControllerRuntimeSkipReason::Policy(
                ResponseSurfacePolicySkipReason::AgentContinuationPlanned
            ))
        );
        assert!(adapter.requests().is_empty());
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn current_webhook_provider_mode_does_not_accept_app_inbound_event() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation(&enabled_config(), &mut ledger, ready.clone())
            .await
            .expect("runtime should not fail");

        assert_eq!(
            decision,
            AgentControllerRuntimeDecision::Skipped(
                AgentControllerRuntimeSkipReason::CurrentProviderModeMismatch
            )
        );
        assert!(adapter.requests().is_empty());
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn available_test_integration_calls_fake_adapter_with_raw_reply_text() {
        let (mut ledger, mut ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);
        ready.reply.reply_text = "<@B123> keep this text exactly".to_string();

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert!(matches!(
            decision,
            AgentControllerRuntimeDecision::Executed(AgentControllerRuntimeExecution {
                controller_kind: AgentControllerKind::CodexAppServer,
                ..
            })
        ));
        let requests = adapter.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].reply_text, "<@B123> keep this text exactly");
        assert_eq!(requests[0].source_session_id, "session-1");
        assert_eq!(
            requests[0].controller_kind,
            AgentControllerKind::CodexAppServer
        );
        assert_event_is_still_processing(&mut ledger, ready);
    }

    #[tokio::test]
    async fn no_matching_current_route_skips_and_releases_claim() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);
        let mut config = enabled_config();
        config.routes[0].providers = vec!["other-provider".to_string()];

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &config,
                &mut ledger,
                ready.clone(),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert_eq!(
            decision,
            AgentControllerRuntimeDecision::Skipped(
                AgentControllerRuntimeSkipReason::NoMatchingRoute
            )
        );
        assert!(adapter.requests().is_empty());
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn route_replies_disabled_skips_and_releases_claim() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);
        let mut config = enabled_config();
        config.routes[0].response_surface.enabled = false;

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &config,
                &mut ledger,
                ready.clone(),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert_eq!(
            decision,
            AgentControllerRuntimeDecision::Skipped(
                AgentControllerRuntimeSkipReason::RouteRepliesDisabled
            )
        );
        assert!(adapter.requests().is_empty());
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn route_filters_are_not_defaulted_to_pass_without_signal_facts() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);
        let mut config = enabled_config();
        config.routes[0].only_forward_from_project_paths =
            vec!["/Users/felix/work/project".to_string()];

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &config,
                &mut ledger,
                ready.clone(),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert_eq!(
            decision,
            AgentControllerRuntimeDecision::Skipped(
                AgentControllerRuntimeSkipReason::RouteFiltersCannotBeRevalidated
            )
        );
        assert!(adapter.requests().is_empty());
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn controller_failed_before_submit_releases_claim() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::with_error_before_submit(
            AgentControllerErrorKind::ControllerRejected,
        );
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert!(matches!(
            decision,
            AgentControllerRuntimeDecision::Failed(AgentControllerError {
                kind: AgentControllerErrorKind::ControllerRejected,
                ..
            })
        ));
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn controller_failed_after_possible_submit_keeps_processing() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::with_error_after_possible_submit(
            AgentControllerErrorKind::ControllerRejected,
        );
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("runtime should not fail");

        assert!(matches!(
            decision,
            AgentControllerRuntimeDecision::Failed(AgentControllerError {
                kind: AgentControllerErrorKind::ControllerRejected,
                submit_boundary: AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit,
                ..
            })
        ));
        assert_event_is_still_processing(&mut ledger, ready);
    }

    #[tokio::test]
    async fn duplicate_provider_event_does_not_call_controller_twice_after_possible_submit() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::with_error_after_possible_submit(
            AgentControllerErrorKind::ControllerRejected,
        );
        let provider_reply = RecordingProviderThreadReplyAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_closed_loop_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                &provider_reply,
                test_time() + Duration::seconds(2),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("closed loop should not fail");

        assert!(matches!(
            decision,
            AgentControllerClosedLoopDecision::ControllerFailed(AgentControllerError {
                submit_boundary: AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit,
                ..
            })
        ));
        let duplicate = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            slack_app_capability(),
            ready.reply.clone(),
            test_time() + Duration::seconds(3),
        )
        .expect("duplicate lookup should not fail");

        assert!(matches!(
            duplicate,
            ProviderInboundDecision::Skip(
                crate::provider_inbound::ProviderInboundSkipReason::EventAlreadyProcessing { .. }
            )
        ));
        assert_eq!(adapter.requests().len(), 1);
        assert!(provider_reply.requests().is_empty());
    }

    #[tokio::test]
    async fn closed_loop_sends_agent_result_text_before_processed() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::with_success_message("agent final result");
        let provider_reply = RecordingProviderThreadReplyAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_closed_loop_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                &provider_reply,
                test_time() + Duration::seconds(2),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("closed loop should not fail");

        assert!(matches!(
            decision,
            AgentControllerClosedLoopDecision::Completed(AgentControllerClosedLoopCompletion {
                outcome: AgentControllerClosedLoopOutcome::ControllerSucceeded(_),
                ..
            })
        ));
        let provider_requests = provider_reply.requests();
        assert_eq!(provider_requests.len(), 1);
        assert_eq!(provider_requests[0].text, "agent final result");
        assert_event_is_duplicate_processed(&mut ledger, ready);
    }

    #[tokio::test]
    async fn closed_loop_provider_result_reply_failure_does_not_mark_processed_or_release_claim() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = RecordingAdapter::default();
        let provider_reply = RecordingProviderThreadReplyAdapter::with_error("result reply failed");
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_closed_loop_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                &provider_reply,
                test_time() + Duration::seconds(2),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("closed loop should not fail");

        assert!(matches!(
            decision,
            AgentControllerClosedLoopDecision::ProviderThreadReplyFailed(
                AgentControllerProviderThreadReplyFailure {
                    outcome: AgentControllerClosedLoopOutcome::ControllerSucceeded(_),
                    ..
                }
            )
        ));
        assert_eq!(adapter.requests().len(), 1);
        assert_eq!(provider_reply.requests().len(), 1);
        assert_event_is_still_processing(&mut ledger, ready);
    }

    #[tokio::test]
    async fn closed_loop_controller_failed_before_submit_releases_claim() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter =
            RecordingAdapter::with_error_before_submit(AgentControllerErrorKind::SessionNotFound);
        let provider_reply = RecordingProviderThreadReplyAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_closed_loop_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                &provider_reply,
                test_time() + Duration::seconds(2),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("closed loop should not fail");

        assert!(matches!(
            decision,
            AgentControllerClosedLoopDecision::ControllerFailed(AgentControllerError {
                kind: AgentControllerErrorKind::SessionNotFound,
                submit_boundary: AgentControllerFailureSubmitBoundary::FailedBeforeSubmit,
                ..
            })
        ));
        assert!(provider_reply.requests().is_empty());
        assert_claim_can_be_taken_again(&mut ledger, ready);
    }

    #[tokio::test]
    async fn closed_loop_controller_success_without_result_text_is_treated_as_failure() {
        let (mut ledger, ready) = ledger_and_ready_with_claim();
        let adapter = EmptyResultAdapter::default();
        let provider_reply = RecordingProviderThreadReplyAdapter::default();
        let runtime = AgentControllerRuntime::new(vec![&adapter]);

        let decision = runtime
            .run_inbound_continuation_closed_loop_with_test_policy_facts(
                &enabled_config(),
                &mut ledger,
                ready.clone(),
                &provider_reply,
                test_time() + Duration::seconds(2),
                Some(available_codex_desktop()),
                Some(slack_app_capability()),
            )
            .await
            .expect("closed loop should not fail");

        assert!(matches!(
            decision,
            AgentControllerClosedLoopDecision::ControllerFailed(AgentControllerError {
                kind: AgentControllerErrorKind::Internal,
                submit_boundary: AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit,
                ..
            })
        ));
        assert!(provider_reply.requests().is_empty());
        assert_event_is_still_processing(&mut ledger, ready);
    }

    #[tokio::test]
    async fn codex_app_server_placeholder_returns_controller_unavailable() {
        let adapter = codex_app_server::CodexAppServerController::new();
        let request = controller_request();

        let result = adapter.continue_session(request.clone()).await;

        assert_eq!(
            result.expect_err("placeholder should not execute"),
            AgentControllerError::failed_before_submit(
                &request,
                AgentControllerErrorKind::ControllerUnavailable,
                "Codex App Server continuation is not available in this build",
            )
        );
    }

    #[derive(Default)]
    struct RecordingAdapter {
        requests: Arc<Mutex<Vec<AgentControllerRequest>>>,
        error: Option<(
            AgentControllerErrorKind,
            AgentControllerFailureSubmitBoundary,
        )>,
        success_message: Option<String>,
    }

    impl RecordingAdapter {
        fn with_error_before_submit(error: AgentControllerErrorKind) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                error: Some((
                    error,
                    AgentControllerFailureSubmitBoundary::FailedBeforeSubmit,
                )),
                success_message: None,
            }
        }

        fn with_error_after_possible_submit(error: AgentControllerErrorKind) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                error: Some((
                    error,
                    AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit,
                )),
                success_message: None,
            }
        }

        fn with_success_message(message: &str) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                error: None,
                success_message: Some(message.to_string()),
            }
        }

        fn requests(&self) -> Vec<AgentControllerRequest> {
            self.requests
                .lock()
                .expect("requests mutex should not be poisoned")
                .clone()
        }
    }

    impl AgentControllerAdapter for RecordingAdapter {
        fn controller_kind(&self) -> AgentControllerKind {
            AgentControllerKind::CodexAppServer
        }

        fn continue_session<'a>(
            &'a self,
            request: AgentControllerRequest,
        ) -> AgentControllerFuture<'a> {
            Box::pin(async move {
                self.requests
                    .lock()
                    .expect("requests mutex should not be poisoned")
                    .push(request.clone());
                if let Some((error, submit_boundary)) = self.error {
                    return Err(match submit_boundary {
                        AgentControllerFailureSubmitBoundary::FailedBeforeSubmit => {
                            AgentControllerError::failed_before_submit(
                                &request,
                                error,
                                "fake controller error",
                            )
                        }
                        AgentControllerFailureSubmitBoundary::FailedAfterPossibleSubmit => {
                            AgentControllerError::failed_after_possible_submit(
                                &request,
                                error,
                                "fake controller error",
                            )
                        }
                    });
                }
                let result_text = self
                    .success_message
                    .clone()
                    .unwrap_or_else(|| "accepted".to_string());
                Ok(AgentControllerSuccess::from_result_text(result_text)
                    .expect("test controller success should include result text"))
            })
        }
    }

    #[derive(Default)]
    struct EmptyResultAdapter {
        requests: Arc<Mutex<Vec<AgentControllerRequest>>>,
    }

    impl AgentControllerAdapter for EmptyResultAdapter {
        fn controller_kind(&self) -> AgentControllerKind {
            AgentControllerKind::CodexAppServer
        }

        fn continue_session<'a>(
            &'a self,
            request: AgentControllerRequest,
        ) -> AgentControllerFuture<'a> {
            Box::pin(async move {
                self.requests
                    .lock()
                    .expect("requests mutex should not be poisoned")
                    .push(request);
                Ok(AgentControllerSuccess {
                    result_text: String::new(),
                })
            })
        }
    }

    #[derive(Default)]
    struct RecordingProviderThreadReplyAdapter {
        requests: Arc<Mutex<Vec<ProviderThreadReplyRequest>>>,
        error: Option<String>,
    }

    impl RecordingProviderThreadReplyAdapter {
        fn with_error(message: &str) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                error: Some(message.to_string()),
            }
        }

        fn requests(&self) -> Vec<ProviderThreadReplyRequest> {
            self.requests
                .lock()
                .expect("requests mutex should not be poisoned")
                .clone()
        }
    }

    impl ProviderThreadReplyAdapter for RecordingProviderThreadReplyAdapter {
        fn provider_id(&self) -> &str {
            "slack"
        }

        fn provider_type(&self) -> &str {
            "slack"
        }

        fn send_thread_reply<'a>(
            &'a self,
            request: ProviderThreadReplyRequest,
        ) -> ProviderThreadReplyFuture<'a> {
            Box::pin(async move {
                self.requests
                    .lock()
                    .expect("requests mutex should not be poisoned")
                    .push(request.clone());
                if let Some(error) = &self.error {
                    return Err(ProviderThreadReplyError {
                        provider_id: request.provider_id,
                        provider_type: request.provider_type,
                        surface_id: request.surface_id,
                        provider_event_id_hash: request.provider_event_id_hash,
                        message: error.clone(),
                    });
                }
                Ok(ProviderThreadReplySuccess {
                    provider_reply_message_id: Some("result-reply-message-1".to_string()),
                })
            })
        }
    }

    fn enabled_config() -> ValidatedConfig {
        let mut route =
            RouteConfig::new(vec!["codex_desktop".to_string()], vec!["slack".to_string()]);
        route.response_surface.enabled = true;
        ValidatedConfig {
            schema_version: CONFIG_SCHEMA_VERSION,
            cli: CliConfig::default(),
            log: LogConfig::default(),
            notification: NotificationConfig::default(),
            sources: vec![SourceConfig {
                id: "codex_desktop".to_string(),
                source_type: SourceType::CodexDesktop,
            }],
            providers: vec![ProviderConfig {
                id: "slack".to_string(),
                detail: ProviderConfigDetail::Slack(SlackProviderConfig {
                    url: UrlSource::Inline(
                        "https://hooks.slack.com/services/T000/B000/token".to_string(),
                    ),
                }),
            }],
            routes: vec![route],
        }
    }

    fn available_codex_desktop() -> AgentIntegrationDescriptor {
        let descriptor = *agent_integration_descriptor(AgentIntegrationId::CodexDesktop);
        let target = descriptor
            .continuation_capability
            .target()
            .expect("Codex Desktop planned continuation target should be cataloged");
        AgentIntegrationDescriptor {
            continuation_capability: ContinuationCapability::Available(target),
            ..descriptor
        }
    }

    fn slack_app_capability() -> &'static ProviderModeCapability {
        provider_mode_capability(ProviderMode::SlackApp)
    }

    fn ledger_and_ready_with_claim() -> (ResponseSurfaceLedger, ProviderInboundReady) {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let now = test_time();
        ledger
            .create_surface_at(new_surface(now), now)
            .expect("surface should be created");
        let surface = match ledger
            .lookup_surface_at(lookup_query(), now + Duration::seconds(1))
            .expect("surface lookup should succeed")
        {
            ResponseSurfaceLookupResult::Hit(surface) => surface,
            other => panic!("surface should be hit, got {other:?}"),
        };
        let ready = ready_for_surface(surface);
        assert!(matches!(
            ledger
                .claim_inbound_event_at(
                    InboundEventDedupInput {
                        provider_id: ready.reply.provider_id.clone(),
                        provider_type: ready.reply.provider_type.clone(),
                        provider_account_id: ready.reply.provider_account_id.clone(),
                        provider_conversation_id: ready.reply.provider_conversation_id.clone(),
                        provider_event_id: ready.reply.provider_event_id.clone(),
                        surface_id: ready.surface.surface_id.clone(),
                    },
                    now + Duration::seconds(1),
                )
                .expect("claim should be created"),
            InboundEventClaimDecision::Claimed { .. }
        ));
        (ledger, ready)
    }

    fn assert_claim_can_be_taken_again(
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
    ) {
        assert!(matches!(
            ledger
                .claim_inbound_event_at(
                    InboundEventDedupInput {
                        provider_id: ready.reply.provider_id,
                        provider_type: ready.reply.provider_type,
                        provider_account_id: ready.reply.provider_account_id,
                        provider_conversation_id: ready.reply.provider_conversation_id,
                        provider_event_id: ready.reply.provider_event_id,
                        surface_id: ready.surface.surface_id,
                    },
                    test_time() + Duration::seconds(2),
                )
                .expect("released claim should be claimable"),
            InboundEventClaimDecision::Claimed { .. }
        ));
    }

    fn assert_event_is_duplicate_processed(
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
    ) {
        assert!(matches!(
            ledger
                .claim_inbound_event_at(
                    inbound_event_input(&ready),
                    test_time() + Duration::seconds(3),
                )
                .expect("processed event should be duplicate"),
            InboundEventClaimDecision::DuplicateProcessed { .. }
        ));
    }

    fn assert_event_is_still_processing(
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
    ) {
        assert!(matches!(
            ledger
                .claim_inbound_event_at(
                    inbound_event_input(&ready),
                    test_time() + Duration::seconds(3),
                )
                .expect("incomplete event should still be processing"),
            InboundEventClaimDecision::AlreadyProcessing { .. }
        ));
    }

    fn ready_for_surface(
        surface: crate::response_surface_ledger::ResponseSurfaceLookupRecord,
    ) -> ProviderInboundReady {
        ProviderInboundReady {
            reply: NormalizedProviderSurfaceReply {
                provider_id: "slack".to_string(),
                provider_type: "slack".to_string(),
                provider_mode: ProviderMode::SlackApp,
                provider_account_id: "T123ABC456".to_string(),
                provider_conversation_id: "C123ABC456".to_string(),
                provider_thread_id: "1716200000.000100".to_string(),
                provider_event_id: "Ev123ABC456".to_string(),
                provider_reply_message_id: Some("1716200011.000200".to_string()),
                reply_text: "continue exactly".to_string(),
            },
            surface,
            provider_event_id_hash: "event-hash".to_string(),
        }
    }

    fn controller_request() -> AgentControllerRequest {
        AgentControllerRequest {
            controller_kind: AgentControllerKind::CodexAppServer,
            surface_id: "surface-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: SourceType::CodexDesktop,
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            reply_text: "continue exactly".to_string(),
            provider_event_id_hash: "event-hash".to_string(),
        }
    }

    fn new_surface(_now: chrono::DateTime<Utc>) -> NewResponseSurface {
        NewResponseSurface {
            signal_id: "signal-1".to_string(),
            delivery_id: "delivery-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            provider_id: "slack".to_string(),
            provider_type: "slack".to_string(),
            provider_mode: ProviderMode::SlackApp,
            provider_account_id: "T123ABC456".to_string(),
            provider_conversation_id: "C123ABC456".to_string(),
            provider_message_id: "1716200000.000100".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
        }
    }

    fn lookup_query() -> ResponseSurfaceLookupQuery {
        ResponseSurfaceLookupQuery {
            provider_id: "slack".to_string(),
            provider_account_id: "T123ABC456".to_string(),
            provider_conversation_id: "C123ABC456".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
        }
    }

    fn test_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
