use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::bridge_binding_ledger::{
    BridgeBindingLedgerStore, ProjectRoomProposalInput, ProjectRoomProposalPromptDecision,
    ProjectRoomProposalSkipReason, RoomProjectBindingRecord, SourceSessionThreadBindingQuery,
    ThreadSessionBindingInput,
};
use crate::config::{
    FeishuLarkProviderConfig, ProviderConfig, ProviderConfigDetail, RouteConfig, ValidatedConfig,
    is_clean_absolute_project_path,
};
use crate::delivery::{
    DeliveryError, DeliveryErrorContext, DeliveryErrorKind, ProviderDeliveryReceipt,
    ProviderDeliveryReceiptStatus, ProviderSendResult,
};
use crate::delivery_safety::{
    DeliveryAttempt, DeliverySafetyDecision, DeliverySafetyGuard, DeliverySuppression,
};
use crate::provider_catalog::provider_config_mode_capability;
use crate::response_surface_ledger::ResponseSurfaceLedgerStore;
use crate::response_surface_policy::ResponseSurfaceDeliveryReceipt;
use crate::response_surface_runtime::{
    ResponseSurfaceCreationDecision, ResponseSurfaceDeliveryFacts,
    create_response_surface_after_delivery,
};
use crate::signal::Signal;

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderSendResult, DeliveryError>> + Send + 'a>>;

pub type ProjectRoomPromptFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProjectRoomPromptResult, DeliveryError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoomPromptRequest {
    pub proposal_id: String,
    pub project_path: String,
    pub project_name: String,
    pub prompt_conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoomPromptResult {
    pub provider_account_id: Option<String>,
    pub provider_conversation_id: Option<String>,
    pub provider_message_id: Option<String>,
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn provider_type(&self) -> &str;
    fn send<'a>(&'a self, signal: &'a Signal) -> ProviderFuture<'a>;

    fn can_prompt_for_project_room(&self) -> bool {
        false
    }

    fn send_project_room_prompt<'a>(
        &'a self,
        _request: ProjectRoomPromptRequest,
    ) -> Option<ProjectRoomPromptFuture<'a>> {
        None
    }

    fn send_to_provider_conversation<'a>(
        &'a self,
        _signal: &'a Signal,
        _provider_conversation_id: &'a str,
    ) -> Option<ProviderFuture<'a>> {
        None
    }

    fn send_to_provider_thread<'a>(
        &'a self,
        _signal: &'a Signal,
        _provider_account_id: &'a str,
        _provider_conversation_id: &'a str,
        _provider_thread_id: &'a str,
    ) -> Option<ProviderFuture<'a>> {
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    pub matched_routes: usize,
    pub attempted: usize,
    pub succeeded: usize,
    pub suppressed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    pub signal_id: String,
    pub source_id: String,
    pub provider_id: String,
    pub provider_type: String,
    pub kind: DeliveryErrorKind,
    pub message: String,
    pub http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub retriable: bool,
}

#[derive(Debug)]
pub enum RouterError {
    ProviderFailures(Vec<ProviderFailure>),
}

impl fmt::Display for RouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderFailures(failures) => {
                write!(
                    f,
                    "{} provider failure{}",
                    failures.len(),
                    if failures.len() == 1 { "" } else { "s" }
                )?;
                for (index, failure) in failures.iter().take(3).enumerate() {
                    write!(
                        f,
                        "{}{} `{}` ({}) failed for signal `{}` from `{}`: {}",
                        if index == 0 { ": " } else { "; " },
                        failure.provider_type,
                        failure.provider_id,
                        failure.kind.as_str(),
                        failure.signal_id,
                        failure.source_id,
                        failure.message
                    )?;
                    if let Some(http_status) = failure.http_status {
                        write!(f, " http_status={http_status}")?;
                    }
                    if let Some(provider_code) = failure.provider_code.as_deref() {
                        write!(f, " provider_code={provider_code}")?;
                    }
                    if failure.retriable {
                        write!(f, " retriable=true")?;
                    }
                }
                if failures.len() > 3 {
                    write!(f, "; {} more provider failures omitted", failures.len() - 3)?;
                }
                Ok(())
            }
        }
    }
}

impl StdError for RouterError {}

pub struct Router<'a> {
    config: &'a ValidatedConfig,
}

impl<'a> Router<'a> {
    pub fn new(config: &'a ValidatedConfig) -> Self {
        Self { config }
    }

    pub async fn route(
        &self,
        signal: &Signal,
        providers: &[&dyn Provider],
    ) -> Result<DeliveryReport, RouterError> {
        self.route_with_safety(signal, providers, None).await
    }

    pub async fn route_with_safety(
        &self,
        signal: &Signal,
        providers: &[&dyn Provider],
        delivery_safety: Option<&DeliverySafetyGuard>,
    ) -> Result<DeliveryReport, RouterError> {
        self.route_with_safety_and_response_surfaces(signal, providers, delivery_safety, None, None)
            .await
    }

    pub async fn route_with_safety_and_response_surfaces(
        &self,
        signal: &Signal,
        providers: &[&dyn Provider],
        delivery_safety: Option<&DeliverySafetyGuard>,
        response_surface_ledger: Option<&ResponseSurfaceLedgerStore>,
        bridge_binding_ledger: Option<&BridgeBindingLedgerStore>,
    ) -> Result<DeliveryReport, RouterError> {
        let providers_by_id: HashMap<&str, &dyn Provider> = providers
            .iter()
            .map(|provider| (provider.id(), *provider))
            .collect();
        let candidate_routes: Vec<_> = self
            .config
            .routes
            .iter()
            .enumerate()
            .filter(|(_, route)| route.matches_source(signal.source_id()))
            .collect();
        let candidate_route_count = candidate_routes.len();
        let mut matching_routes = Vec::new();
        let mut filtered_routes = 0;

        for (route_index, route) in candidate_routes {
            if let Some(reason) = route_filter_miss(signal, route) {
                filtered_routes += 1;
                debug!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    route.index = route_index,
                    reason = %reason.as_str(),
                    event = "route.filtered",
                );
                continue;
            }

            matching_routes.push(route);
        }

        info!(
            signal.id = %signal.id,
            source.id = %signal.source_id(),
            candidate_routes = candidate_route_count,
            matched_routes = matching_routes.len(),
            filtered_routes,
            event = "route.matched",
        );

        let mut attempted = 0;
        let mut succeeded = 0;
        let mut suppressed = 0;
        let mut failures = Vec::new();

        for route in &matching_routes {
            for provider_id in &route.providers {
                let Some(provider) = providers_by_id.get(provider_id.as_str()) else {
                    failures.push(self.missing_provider_failure(signal, provider_id));
                    continue;
                };

                let delivery_targets = match self
                    .delivery_targets_for_provider(signal, *provider, bridge_binding_ledger)
                    .await
                {
                    Ok(delivery_targets) => delivery_targets,
                    Err(failure) => {
                        failures.push(failure);
                        continue;
                    }
                };

                for delivery_target in delivery_targets {
                    attempted += 1;

                    if let Some(delivery_safety) = delivery_safety {
                        let safety_result = if let Some(provider_target_id) =
                            delivery_target.provider_target_id()
                        {
                            delivery_safety.check_for_provider_target(
                                DeliveryAttempt {
                                    signal,
                                    provider_id: provider.id(),
                                },
                                provider_target_id,
                            )
                        } else {
                            delivery_safety.check(DeliveryAttempt {
                                signal,
                                provider_id: provider.id(),
                            })
                        };
                        match safety_result {
                            Ok(DeliverySafetyDecision::Allow { .. }) => {}
                            Ok(DeliverySafetyDecision::Suppress(suppression)) => {
                                suppressed += 1;
                                log_delivery_suppressed(
                                    signal,
                                    *provider,
                                    delivery_target.provider_target_id(),
                                    &suppression,
                                );
                                continue;
                            }
                            Err(error) => {
                                failures.push(ProviderFailure {
                                    signal_id: signal.id.clone(),
                                    source_id: signal.source_id().to_string(),
                                    provider_id: provider.id().to_string(),
                                    provider_type: provider.provider_type().to_string(),
                                    kind: DeliveryErrorKind::Internal,
                                    message: format!("delivery safety guard failed: {error}"),
                                    http_status: None,
                                    provider_code: None,
                                    retriable: false,
                                });
                                continue;
                            }
                        }
                    }

                    info!(
                        signal.id = %signal.id,
                        source.id = %signal.source_id(),
                        provider.id = %provider.id(),
                        provider.type = %provider.provider_type(),
                        provider.target_id = delivery_target.provider_target_id(),
                        event = "provider.send.started",
                    );

                    match send_to_target(*provider, signal, &delivery_target).await {
                        Ok(result) => {
                            succeeded += 1;
                            info!(
                                signal.id = %signal.id,
                                source.id = %signal.source_id(),
                                provider.id = %provider.id(),
                                provider.type = %provider.provider_type(),
                                provider.target_id = delivery_target.provider_target_id(),
                                provider.status = %result.status.as_str(),
                                http.status = result.http_status,
                                event = "provider.send.succeeded",
                            );
                            if let Some(response_surface_ledger) = response_surface_ledger {
                                self.create_response_surface_after_delivery(
                                    response_surface_ledger,
                                    bridge_binding_ledger,
                                    signal,
                                    route,
                                    *provider,
                                    &result,
                                )
                                .await;
                            }
                        }
                        Err(error) => {
                            warn!(
                                signal.id = %signal.id,
                                source.id = %signal.source_id(),
                                provider.id = %provider.id(),
                                provider.type = %provider.provider_type(),
                                provider.target_id = delivery_target.provider_target_id(),
                                error.kind = %error.kind.as_str(),
                                error.phase = %error.context.phase.as_str(),
                                error.retriable = error.retriable,
                                http.status = error.http_status,
                                provider.code = error.provider_code.as_deref(),
                                error = %error.message,
                                event = "provider.send.failed",
                            );
                            failures.push(ProviderFailure {
                                signal_id: error.context.signal_id,
                                source_id: error.context.source_id,
                                provider_id: provider.id().to_string(),
                                provider_type: provider.provider_type().to_string(),
                                kind: error.kind,
                                message: error.message,
                                http_status: error.http_status,
                                provider_code: error.provider_code,
                                retriable: error.retriable,
                            });
                        }
                    }
                }
            }
        }

        if failures.is_empty() {
            Ok(DeliveryReport {
                matched_routes: matching_routes.len(),
                attempted,
                succeeded,
                suppressed,
            })
        } else {
            Err(RouterError::ProviderFailures(failures))
        }
    }

    async fn delivery_targets_for_provider(
        &self,
        signal: &Signal,
        provider: &dyn Provider,
        bridge_binding_ledger: Option<&BridgeBindingLedgerStore>,
    ) -> Result<Vec<ProviderDeliveryTarget>, ProviderFailure> {
        let Some(bridge_binding_ledger) = bridge_binding_ledger else {
            return Ok(vec![ProviderDeliveryTarget::Static]);
        };
        let Some(project_path) = signal
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.project_path.as_deref())
            .filter(|path| is_clean_absolute_project_path(path))
        else {
            return Ok(vec![ProviderDeliveryTarget::Static]);
        };

        let rooms = bridge_binding_ledger
            .update(|ledger| Ok(ledger.connected_rooms_for_project_tree(project_path)))
            .await
            .map_err(|error| ProviderFailure {
                signal_id: signal.id.clone(),
                source_id: signal.source_id().to_string(),
                provider_id: provider.id().to_string(),
                provider_type: provider.provider_type().to_string(),
                kind: DeliveryErrorKind::Internal,
                message: format!("bridge binding ledger lookup failed: {error}"),
                http_status: None,
                provider_code: None,
                retriable: false,
            })?;

        let mut seen = HashSet::new();
        let mut targets = Vec::new();
        for room in rooms.into_iter().filter(|room| {
            room.provider_id == provider.id() && room.provider_type == provider.provider_type()
        }) {
            let target = self
                .delivery_target_for_bound_room(
                    signal,
                    provider,
                    project_path,
                    &room,
                    bridge_binding_ledger,
                )
                .await?;
            let key = target.bridge_delivery_key(&room);
            if seen.insert(key) {
                targets.push(target);
            }
        }

        if targets.is_empty() {
            if let Some(config_provider) = self.config.provider(provider.id())
                && !provider_has_static_delivery_target(config_provider)
            {
                self.prompt_project_room_for_unbound_project(
                    signal,
                    provider,
                    project_path,
                    bridge_binding_ledger,
                )
                .await?;
                info!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    event = "bridge_binding.delivery.skipped",
                    reason = "no_bound_room_and_no_default_room",
                );
                return Ok(Vec::new());
            }
            Ok(vec![ProviderDeliveryTarget::Static])
        } else {
            Ok(targets)
        }
    }

    async fn prompt_project_room_for_unbound_project(
        &self,
        signal: &Signal,
        provider: &dyn Provider,
        project_path: &str,
        bridge_binding_ledger: &BridgeBindingLedgerStore,
    ) -> Result<(), ProviderFailure> {
        if !provider.can_prompt_for_project_room() {
            debug!(
                signal.id = %signal.id,
                source.id = %signal.source_id(),
                provider.id = %provider.id(),
                provider.type = %provider.provider_type(),
                project.path = %project_path,
                event = "bridge_binding.project_room_proposal.skipped",
                reason = "provider_cannot_prompt",
            );
            return Ok(());
        }

        let now = Utc::now();
        let (decision, owner_direct_chat) = bridge_binding_ledger
            .update(|ledger| {
                let decision = ledger.begin_project_room_proposal_prompt_at(
                    ProjectRoomProposalInput {
                        provider_id: provider.id().to_string(),
                        provider_type: provider.provider_type().to_string(),
                        project_path: project_path.to_string(),
                    },
                    now,
                )?;
                let owner_direct_chat = ledger
                    .owner_direct_chat_for_provider(provider.id(), provider.provider_type())?;
                Ok((decision, owner_direct_chat))
            })
            .await
            .map_err(|error| ProviderFailure {
                signal_id: signal.id.clone(),
                source_id: signal.source_id().to_string(),
                provider_id: provider.id().to_string(),
                provider_type: provider.provider_type().to_string(),
                kind: DeliveryErrorKind::Internal,
                message: format!("bridge binding project room proposal lookup failed: {error}"),
                http_status: None,
                provider_code: None,
                retriable: false,
            })?;

        let record = match decision {
            ProjectRoomProposalPromptDecision::Prompt(record) => record,
            ProjectRoomProposalPromptDecision::Skip(reason) => {
                debug!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    project.path = %project_path,
                    proposal.skip_reason = %project_room_proposal_skip_reason(reason),
                    event = "bridge_binding.project_room_proposal.skipped",
                );
                return Ok(());
            }
        };

        info!(
            signal.id = %signal.id,
            source.id = %signal.source_id(),
            provider.id = %provider.id(),
            provider.type = %provider.provider_type(),
            project.path = %record.project_path,
            project.name = %record.project_name,
            proposal.id = %record.proposal_id,
            event = "bridge_binding.project_room_proposal.prompt.started",
        );

        let Some(prompt) = provider.send_project_room_prompt(ProjectRoomPromptRequest {
            proposal_id: record.proposal_id.clone(),
            project_path: record.project_path.clone(),
            project_name: record.project_name.clone(),
            prompt_conversation_id: owner_direct_chat.map(|chat| chat.provider_conversation_id),
        }) else {
            let _ = bridge_binding_ledger
                .update(|ledger| {
                    ledger.mark_project_room_proposal_prompt_failed_at(
                        &record.proposal_id,
                        Utc::now(),
                    )
                })
                .await;
            warn!(
                signal.id = %signal.id,
                source.id = %signal.source_id(),
                provider.id = %provider.id(),
                provider.type = %provider.provider_type(),
                project.path = %record.project_path,
                proposal.id = %record.proposal_id,
                event = "bridge_binding.project_room_proposal.prompt.failed",
                error = "provider returned no prompt future",
            );
            return Ok(());
        };

        match prompt.await {
            Ok(result) => {
                bridge_binding_ledger
                    .update(|ledger| {
                        ledger.record_project_room_proposal_prompt_sent_at(
                            &record.proposal_id,
                            result.provider_account_id.clone(),
                            result.provider_conversation_id.clone(),
                            result.provider_message_id.clone(),
                            Utc::now(),
                        )
                    })
                    .await
                    .map_err(|error| ProviderFailure {
                        signal_id: signal.id.clone(),
                        source_id: signal.source_id().to_string(),
                        provider_id: provider.id().to_string(),
                        provider_type: provider.provider_type().to_string(),
                        kind: DeliveryErrorKind::Internal,
                        message: format!(
                            "bridge binding project room proposal update failed: {error}"
                        ),
                        http_status: None,
                        provider_code: None,
                        retriable: false,
                    })?;
                info!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    project.path = %record.project_path,
                    proposal.id = %record.proposal_id,
                    provider.conversation.id = result.provider_conversation_id.as_deref(),
                    provider.message.id = result.provider_message_id.as_deref(),
                    event = "bridge_binding.project_room_proposal.prompt.succeeded",
                );
            }
            Err(error) => {
                let _ = bridge_binding_ledger
                    .update(|ledger| {
                        ledger.mark_project_room_proposal_prompt_failed_at(
                            &record.proposal_id,
                            Utc::now(),
                        )
                    })
                    .await;
                warn!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    project.path = %record.project_path,
                    proposal.id = %record.proposal_id,
                    error.kind = %error.kind.as_str(),
                    error.retriable = error.retriable,
                    http.status = error.http_status,
                    provider.code = error.provider_code.as_deref(),
                    error = %error.message,
                    event = "bridge_binding.project_room_proposal.prompt.failed",
                );
            }
        }

        Ok(())
    }

    async fn delivery_target_for_bound_room(
        &self,
        signal: &Signal,
        provider: &dyn Provider,
        project_path: &str,
        room: &RoomProjectBindingRecord,
        bridge_binding_ledger: &BridgeBindingLedgerStore,
    ) -> Result<ProviderDeliveryTarget, ProviderFailure> {
        let Some(source_session_id) = signal
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.session_id.as_deref())
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(ProviderDeliveryTarget::ProviderConversation {
                provider_conversation_id: room.provider_conversation_id.clone(),
            });
        };

        let query = SourceSessionThreadBindingQuery {
            provider_id: provider.id().to_string(),
            provider_type: provider.provider_type().to_string(),
            provider_account_id: room.provider_account_id.clone(),
            provider_conversation_id: room.provider_conversation_id.clone(),
            project_path: project_path.to_string(),
            source_id: signal.source_id().to_string(),
            source_type: signal.source_type().to_string(),
            source_session_id: source_session_id.to_string(),
        };
        let binding = bridge_binding_ledger
            .update(|ledger| ledger.lookup_thread_session_for_source_session(&query))
            .await
            .map_err(|error| ProviderFailure {
                signal_id: signal.id.clone(),
                source_id: signal.source_id().to_string(),
                provider_id: provider.id().to_string(),
                provider_type: provider.provider_type().to_string(),
                kind: DeliveryErrorKind::Internal,
                message: format!("bridge binding thread lookup failed: {error}"),
                http_status: None,
                provider_code: None,
                retriable: false,
            })?;

        Ok(match binding {
            Some(binding) => ProviderDeliveryTarget::ProviderThread {
                provider_account_id: binding.provider_account_id,
                provider_conversation_id: binding.provider_conversation_id,
                provider_thread_id: binding.provider_thread_id,
            },
            None => ProviderDeliveryTarget::ProviderConversation {
                provider_conversation_id: room.provider_conversation_id.clone(),
            },
        })
    }

    async fn create_response_surface_after_delivery(
        &self,
        response_surface_ledger: &ResponseSurfaceLedgerStore,
        bridge_binding_ledger: Option<&BridgeBindingLedgerStore>,
        signal: &Signal,
        route: &RouteConfig,
        provider: &dyn Provider,
        result: &ProviderSendResult,
    ) {
        let Some(receipt) = result.delivery_receipt.as_ref() else {
            return;
        };
        if receipt.status != ProviderDeliveryReceiptStatus::SurfaceReady {
            debug!(
                signal.id = %signal.id,
                source.id = %signal.source_id(),
                provider.id = %provider.id(),
                provider.type = %provider.provider_type(),
                receipt.status = ?receipt.status,
                event = "response_surface.creation.skipped",
                reason = "delivery_receipt_not_surface_ready",
            );
            return;
        }
        let Some(config_provider) = self.config.provider(provider.id()) else {
            warn!(
                signal.id = %signal.id,
                source.id = %signal.source_id(),
                provider.id = %provider.id(),
                provider.type = %provider.provider_type(),
                event = "response_surface.creation.failed",
                error = "provider config missing after delivery",
            );
            return;
        };

        let provider_capability = provider_config_mode_capability(config_provider);
        let delivery_id = match receipt.provider_conversation_id.as_deref() {
            Some(provider_conversation_id) => {
                format!(
                    "{}:{}:{}",
                    signal.id,
                    provider.id(),
                    provider_conversation_id
                )
            }
            None => format!("{}:{}", signal.id, provider.id()),
        };
        let delivery = ResponseSurfaceDeliveryFacts {
            delivery_id,
            provider_id: provider.id().to_string(),
            receipt: response_surface_delivery_receipt(receipt),
        };
        let outcome = response_surface_ledger
            .update(|ledger| {
                create_response_surface_after_delivery(
                    ledger,
                    signal,
                    route,
                    provider_capability,
                    delivery,
                    Utc::now(),
                )
            })
            .await;

        match outcome {
            Ok(ResponseSurfaceCreationDecision::Created(record)) => {
                info!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    surface.id = %record.surface_id,
                    ledger.path = %response_surface_ledger.state_path().display(),
                    event = "response_surface.created",
                );
                if let Some(bridge_binding_ledger) = bridge_binding_ledger {
                    self.bind_response_surface_thread_session(
                        bridge_binding_ledger,
                        signal,
                        &record,
                    )
                    .await;
                }
            }
            Ok(ResponseSurfaceCreationDecision::Skipped(reason)) => {
                debug!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    reason = ?reason,
                    event = "response_surface.creation.skipped",
                );
            }
            Err(error) => {
                warn!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    provider.id = %provider.id(),
                    provider.type = %provider.provider_type(),
                    error = %error,
                    event = "response_surface.creation.failed",
                );
            }
        }
    }

    async fn bind_response_surface_thread_session(
        &self,
        bridge_binding_ledger: &BridgeBindingLedgerStore,
        signal: &Signal,
        record: &crate::response_surface_ledger::ResponseSurfaceRecord,
    ) {
        let Some(project_path) = signal
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.project_path.as_deref())
            .filter(|path| is_clean_absolute_project_path(path))
        else {
            debug!(
                signal.id = %signal.id,
                source.id = %signal.source_id(),
                surface.id = %record.surface_id,
                event = "bridge_binding.thread_session.skipped",
                reason = "missing_clean_project_path",
            );
            return;
        };

        let input = ThreadSessionBindingInput {
            provider_id: record.provider_id.clone(),
            provider_type: record.provider_type.clone(),
            provider_account_id: record.provider_account_id.clone(),
            provider_conversation_id: record.provider_conversation_id.clone(),
            provider_thread_id: record.provider_thread_id.clone(),
            project_path: project_path.to_string(),
            source_id: record.source_id.clone(),
            source_type: record.source_type.clone(),
            source_session_id: record.source_session_id.clone(),
        };
        let result = bridge_binding_ledger
            .update(|ledger| ledger.bind_thread_session_at(input, Utc::now()))
            .await;

        match result {
            Ok(binding) => {
                info!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    source.session.id = %binding.source_session_id,
                    provider.id = %binding.provider_id,
                    provider.type = %binding.provider_type,
                    provider.conversation.id = %binding.provider_conversation_id,
                    provider.thread.id = %binding.provider_thread_id,
                    project.path = %binding.project_path,
                    ledger.path = %bridge_binding_ledger.state_path().display(),
                    event = "bridge_binding.thread_session.bound",
                );
            }
            Err(error) => {
                warn!(
                    signal.id = %signal.id,
                    source.id = %signal.source_id(),
                    surface.id = %record.surface_id,
                    error = %error,
                    event = "bridge_binding.thread_session.failed",
                );
            }
        }
    }

    fn missing_provider_failure(&self, signal: &Signal, provider_id: &str) -> ProviderFailure {
        let provider_type = self
            .config
            .provider(provider_id)
            .map(|provider| provider.provider_type().as_str())
            .unwrap_or("unknown");

        ProviderFailure {
            signal_id: signal.id.clone(),
            source_id: signal.source_id().to_string(),
            provider_id: provider_id.to_string(),
            provider_type: provider_type.to_string(),
            kind: DeliveryErrorKind::Config,
            message: "provider adapter is not registered".to_string(),
            http_status: None,
            provider_code: None,
            retriable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderDeliveryTarget {
    Static,
    ProviderConversation {
        provider_conversation_id: String,
    },
    ProviderThread {
        provider_account_id: String,
        provider_conversation_id: String,
        provider_thread_id: String,
    },
}

impl ProviderDeliveryTarget {
    fn provider_target_id(&self) -> Option<&str> {
        match self {
            Self::Static => None,
            Self::ProviderConversation {
                provider_conversation_id,
            } => Some(provider_conversation_id),
            Self::ProviderThread {
                provider_thread_id, ..
            } => Some(provider_thread_id),
        }
    }

    fn bridge_delivery_key(&self, room: &RoomProjectBindingRecord) -> String {
        match self {
            Self::Static => "static".to_string(),
            Self::ProviderConversation {
                provider_conversation_id,
            } => bridge_room_delivery_key(room, provider_conversation_id, None),
            Self::ProviderThread {
                provider_conversation_id,
                provider_thread_id,
                ..
            } => bridge_room_delivery_key(room, provider_conversation_id, Some(provider_thread_id)),
        }
    }
}

async fn send_to_target(
    provider: &dyn Provider,
    signal: &Signal,
    target: &ProviderDeliveryTarget,
) -> Result<ProviderSendResult, DeliveryError> {
    match target {
        ProviderDeliveryTarget::Static => provider.send(signal).await,
        ProviderDeliveryTarget::ProviderConversation {
            provider_conversation_id,
        } => {
            let Some(send) =
                provider.send_to_provider_conversation(signal, provider_conversation_id)
            else {
                return Err(DeliveryError::new(
                    DeliveryErrorKind::Internal,
                    DeliveryErrorContext::provider_send(
                        signal,
                        provider.id(),
                        provider.provider_type(),
                    ),
                    format!(
                        "provider `{}` does not support project-bound room delivery",
                        provider.id()
                    ),
                ));
            };
            send.await
        }
        ProviderDeliveryTarget::ProviderThread {
            provider_account_id,
            provider_conversation_id,
            provider_thread_id,
        } => {
            let Some(send) = provider.send_to_provider_thread(
                signal,
                provider_account_id,
                provider_conversation_id,
                provider_thread_id,
            ) else {
                return Err(DeliveryError::new(
                    DeliveryErrorKind::Internal,
                    DeliveryErrorContext::provider_send(
                        signal,
                        provider.id(),
                        provider.provider_type(),
                    ),
                    format!(
                        "provider `{}` does not support project-bound thread delivery",
                        provider.id()
                    ),
                ));
            };
            send.await
        }
    }
}

fn bridge_room_delivery_key(
    room: &RoomProjectBindingRecord,
    provider_conversation_id: &str,
    provider_thread_id: Option<&str>,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        room.provider_id,
        room.provider_type,
        room.provider_account_id,
        provider_conversation_id,
        provider_thread_id.unwrap_or("")
    )
}

fn provider_has_static_delivery_target(provider: &ProviderConfig) -> bool {
    !matches!(
        &provider.detail,
        ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(detail))
            if detail.chat_id.is_none()
    )
}

fn log_delivery_suppressed(
    signal: &Signal,
    provider: &dyn Provider,
    provider_target_id: Option<&str>,
    suppression: &DeliverySuppression,
) {
    warn!(
        signal.id = %signal.id,
        source.id = %signal.source_id(),
        provider.id = %provider.id(),
        provider.type = %provider.provider_type(),
        provider.target_id = provider_target_id,
        reason = %suppression.reason.as_str(),
        message.fingerprint = %suppression.message_fingerprint_hash,
        delivery.key = %suppression.delivery_key_hash,
        window.seconds = suppression.window_seconds,
        count = suppression.count,
        global.paused = suppression.global_paused,
        event = "delivery.suppressed",
    );

    if let Some(pause_started) = &suppression.pause_started {
        warn!(
            signal.id = %signal.id,
            source.id = %signal.source_id(),
            provider.id = %provider.id(),
            provider.type = %provider.provider_type(),
            provider.target_id = provider_target_id,
            message.fingerprint = %suppression.message_fingerprint_hash,
            message.fingerprint_count = pause_started.message_fingerprint_count,
            window.seconds = pause_started.window_seconds,
            event = "delivery.pause.started",
        );
    }
}

fn response_surface_delivery_receipt(
    receipt: &ProviderDeliveryReceipt,
) -> ResponseSurfaceDeliveryReceipt {
    ResponseSurfaceDeliveryReceipt {
        provider_account_id: receipt.provider_account_id.clone(),
        provider_conversation_id: receipt.provider_conversation_id.clone(),
        provider_message_id: receipt.provider_message_id.clone(),
        provider_thread_id: receipt.provider_thread_id.clone(),
    }
}

fn project_room_proposal_skip_reason(reason: ProjectRoomProposalSkipReason) -> &'static str {
    match reason {
        ProjectRoomProposalSkipReason::AlreadyPending => "already_pending",
        ProjectRoomProposalSkipReason::Ignored => "ignored",
        ProjectRoomProposalSkipReason::RoomCreated => "room_created",
        ProjectRoomProposalSkipReason::BoundToExisting => "bound_to_existing",
    }
}

impl RouteConfig {
    fn matches_source(&self, source_id: &str) -> bool {
        self.sources.iter().any(|source| source == source_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteFilterMissReason {
    DurationTooShort,
    MissingProjectPath,
    ProjectPathNotAllowed,
}

impl RouteFilterMissReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::DurationTooShort => "duration_too_short",
            Self::MissingProjectPath => "missing_project_path",
            Self::ProjectPathNotAllowed => "project_path_not_allowed",
        }
    }
}

fn route_filter_miss(signal: &Signal, route: &RouteConfig) -> Option<RouteFilterMissReason> {
    if let Some(minimum_duration_ms) = route.minimum_task_duration_ms()
        && let Some(duration_ms) = signal
            .lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.duration_ms)
        && duration_ms < minimum_duration_ms
    {
        return Some(RouteFilterMissReason::DurationTooShort);
    }

    if !route.only_forward_from_project_paths.is_empty() {
        let Some(project_path) = signal
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.project_path.as_deref())
            .filter(|path| is_clean_absolute_project_path(path))
        else {
            return Some(RouteFilterMissReason::MissingProjectPath);
        };

        let allowed = route
            .only_forward_from_project_paths
            .iter()
            .any(|allowed_path| Path::new(project_path).starts_with(Path::new(allowed_path)));

        if !allowed {
            return Some(RouteFilterMissReason::ProjectPathNotAllowed);
        }
    }

    None
}

#[cfg(test)]
mod tests;
