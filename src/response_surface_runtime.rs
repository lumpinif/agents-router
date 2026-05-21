use chrono::{DateTime, Utc};

use crate::agent_integration_catalog::AgentIntegrationDescriptor;
use crate::config::RouteConfig;
use crate::provider_catalog::ProviderModeCapability;
use crate::response_surface_ledger::{
    NewResponseSurface, ResponseSurfaceLedger, ResponseSurfaceRecord,
};
use crate::response_surface_policy::{
    ResponseSurfaceDeliveryReceipt, ResponseSurfacePolicyCheck, ResponseSurfacePolicyDecision,
    ResponseSurfacePolicyInput, ResponseSurfacePolicySkipReason, ResponseSurfacePolicySource,
    agent_integration_for_signal, evaluate_response_surface_policy,
};
use crate::signal::Signal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseSurfaceDeliveryFacts {
    pub delivery_id: String,
    pub provider_id: String,
    pub receipt: ResponseSurfaceDeliveryReceipt,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseSurfaceCreationDecision {
    Created(ResponseSurfaceRecord),
    Skipped(ResponseSurfacePolicySkipReason),
}

pub fn create_response_surface_after_delivery(
    ledger: &mut ResponseSurfaceLedger,
    signal: &Signal,
    route: &RouteConfig,
    provider: &ProviderModeCapability,
    delivery: ResponseSurfaceDeliveryFacts,
    now: DateTime<Utc>,
) -> anyhow::Result<ResponseSurfaceCreationDecision> {
    create_response_surface_after_delivery_inner(
        ledger, signal, route, provider, delivery, now, None,
    )
}

#[cfg(test)]
fn create_response_surface_after_delivery_with_agent_integration(
    ledger: &mut ResponseSurfaceLedger,
    signal: &Signal,
    route: &RouteConfig,
    provider: &ProviderModeCapability,
    delivery: ResponseSurfaceDeliveryFacts,
    now: DateTime<Utc>,
    agent_integration: AgentIntegrationDescriptor,
) -> anyhow::Result<ResponseSurfaceCreationDecision> {
    create_response_surface_after_delivery_inner(
        ledger,
        signal,
        route,
        provider,
        delivery,
        now,
        Some(agent_integration),
    )
}

fn create_response_surface_after_delivery_inner(
    ledger: &mut ResponseSurfaceLedger,
    signal: &Signal,
    route: &RouteConfig,
    provider: &ProviderModeCapability,
    delivery: ResponseSurfaceDeliveryFacts,
    now: DateTime<Utc>,
    agent_integration_override: Option<AgentIntegrationDescriptor>,
) -> anyhow::Result<ResponseSurfaceCreationDecision> {
    let override_integration = agent_integration_override.as_ref();
    let policy_input = ResponseSurfacePolicyInput {
        check: ResponseSurfacePolicyCheck::SurfaceCreation,
        provider,
        agent_integration: override_integration.or_else(|| agent_integration_for_signal(signal)),
        route,
        source: ResponseSurfacePolicySource::Signal(signal),
        delivery_receipt: &delivery.receipt,
    };

    let allow = match evaluate_response_surface_policy(policy_input) {
        ResponseSurfacePolicyDecision::Allow(allow) => allow,
        ResponseSurfacePolicyDecision::Skip(reason) => {
            return Ok(ResponseSurfaceCreationDecision::Skipped(reason));
        }
    };

    let record = ledger.create_surface_at(
        NewResponseSurface {
            signal_id: signal.id.clone(),
            delivery_id: delivery.delivery_id,
            source_id: signal.source_id().to_string(),
            source_type: signal.source_type().to_string(),
            source_session_id: allow.source_session_id,
            source_turn_id: signal
                .conversation
                .as_ref()
                .and_then(|conversation| conversation.turn_id.clone()),
            provider_id: delivery.provider_id,
            provider_type: provider.provider_type.as_str().to_string(),
            provider_mode: allow.provider_mode,
            provider_account_id: delivery
                .receipt
                .provider_account_id
                .expect("policy allow requires provider_account_id receipt"),
            provider_conversation_id: delivery
                .receipt
                .provider_conversation_id
                .expect("policy allow requires provider_conversation_id receipt"),
            provider_message_id: delivery
                .receipt
                .provider_message_id
                .expect("policy allow requires provider_message_id receipt"),
            provider_thread_id: delivery
                .receipt
                .provider_thread_id
                .expect("policy allow requires provider_thread_id receipt"),
            expires_at: delivery.expires_at,
        },
        now,
    )?;

    Ok(ResponseSurfaceCreationDecision::Created(record))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::agent_integration_catalog::{
        AgentIntegrationDescriptor, AgentIntegrationId, ContinuationCapability,
        agent_integration_descriptor,
    };
    use crate::config::RouteConfig;
    use crate::provider_catalog::{DeliveryReceiptField, ProviderMode, provider_mode_capability};
    use crate::response_surface_ledger::{ResponseSurfaceLookupQuery, ResponseSurfaceLookupResult};
    use crate::signal::{Signal, SignalConversation};

    #[test]
    fn planned_agent_integration_does_not_create_surface() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let decision = create_response_surface_after_delivery(
            &mut ledger,
            &codex_desktop_signal(),
            &route_with_replies_enabled(),
            provider_mode_capability(ProviderMode::SlackApp),
            delivery_facts(full_receipt()),
            test_time(),
        )
        .expect("creation should not fail");

        assert_eq!(
            decision,
            ResponseSurfaceCreationDecision::Skipped(
                ResponseSurfacePolicySkipReason::AgentContinuationPlanned
            )
        );
    }

    #[test]
    fn available_policy_path_creates_surface_only_after_full_delivery_receipt() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let signal = codex_desktop_signal();
        let route = route_with_replies_enabled();
        let provider = provider_mode_capability(ProviderMode::SlackApp);
        let decision = create_response_surface_after_delivery_with_agent_integration(
            &mut ledger,
            &signal,
            &route,
            provider,
            delivery_facts(full_receipt()),
            test_time(),
            available_codex_desktop(),
        )
        .expect("test available path should not fail");

        let ResponseSurfaceCreationDecision::Created(record) = decision else {
            panic!("available path should create surface, got {decision:?}");
        };

        assert_eq!(record.source_session_id, "session-1");
        assert!(matches!(
            ledger
                .lookup_surface_at(
                    ResponseSurfaceLookupQuery {
                        provider_id: "slack".to_string(),
                        provider_account_id: "T123ABC456".to_string(),
                        provider_conversation_id: "C123ABC456".to_string(),
                        provider_thread_id: "1716200000.000100".to_string(),
                    },
                    test_time() + Duration::seconds(1),
                )
                .expect("lookup should succeed"),
            ResponseSurfaceLookupResult::Hit(_)
        ));
    }

    #[test]
    fn missing_delivery_receipt_does_not_create_surface() {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let signal = codex_desktop_signal();
        let route = route_with_replies_enabled();
        let decision = create_response_surface_after_delivery_with_agent_integration(
            &mut ledger,
            &signal,
            &route,
            provider_mode_capability(ProviderMode::SlackApp),
            delivery_facts(ResponseSurfaceDeliveryReceipt {
                provider_account_id: Some("T123ABC456".to_string()),
                provider_conversation_id: Some("C123ABC456".to_string()),
                provider_message_id: None,
                provider_thread_id: Some("1716200000.000100".to_string()),
            }),
            test_time(),
            available_codex_desktop(),
        )
        .expect("creation should not fail");

        assert_eq!(
            decision,
            ResponseSurfaceCreationDecision::Skipped(
                ResponseSurfacePolicySkipReason::DeliveryReceiptFieldMissing(
                    DeliveryReceiptField::ProviderMessageId
                )
            )
        );
    }

    fn codex_desktop_signal() -> Signal {
        let mut signal = Signal::new_with_timestamp(
            "signal-1",
            "codex_desktop",
            "codex_desktop",
            "Codex",
            "Ready for review.",
            test_time(),
            BTreeMap::new(),
        );
        signal.conversation = Some(SignalConversation {
            session_id: Some("session-1".to_string()),
            session_title: None,
            turn_id: Some("turn-1".to_string()),
            prompt: None,
            answer: None,
            model: None,
        });
        signal
    }

    fn route_with_replies_enabled() -> RouteConfig {
        let mut route =
            RouteConfig::new(vec!["codex_desktop".to_string()], vec!["slack".to_string()]);
        route.response_surface.enabled = true;
        route
    }

    fn delivery_facts(receipt: ResponseSurfaceDeliveryReceipt) -> ResponseSurfaceDeliveryFacts {
        ResponseSurfaceDeliveryFacts {
            delivery_id: "delivery-1".to_string(),
            provider_id: "slack".to_string(),
            receipt,
            expires_at: test_time() + Duration::hours(24),
        }
    }

    fn full_receipt() -> ResponseSurfaceDeliveryReceipt {
        ResponseSurfaceDeliveryReceipt {
            provider_account_id: Some("T123ABC456".to_string()),
            provider_conversation_id: Some("C123ABC456".to_string()),
            provider_message_id: Some("1716200000.000100".to_string()),
            provider_thread_id: Some("1716200000.000100".to_string()),
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

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
