use crate::agent_integration_catalog::{AgentIntegrationDescriptor, ContinuationSupportStatus};
use crate::config::RouteConfig;
use crate::provider_catalog::{
    DeliveryReceiptField, InboundReplyMode, ProviderModeCapability,
    RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS, RESPONSE_SURFACE_REPLY_SURFACES,
    StableEventIdCapability,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseSurfaceExposureDecision {
    Eligible,
    Ineligible(ResponseSurfaceExposureSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseSurfaceExposureSkipReason {
    AgentContinuationUnsupported,
    AgentContinuationPlanned,
    ProviderDoesNotSupportLocalInbound,
    ProviderRequiresPublicEndpoint,
    ProviderStableEventIdUnavailable,
    ProviderReplySurfaceUnsupported,
    ProviderReceiptFieldUnsupported(DeliveryReceiptField),
    RouteRepliesDisabled,
}

pub fn evaluate_response_surface_exposure(
    agent: &AgentIntegrationDescriptor,
    provider: &ProviderModeCapability,
    route: &RouteConfig,
) -> ResponseSurfaceExposureDecision {
    match agent.continuation_status() {
        ContinuationSupportStatus::Unsupported => {
            return ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::AgentContinuationUnsupported,
            );
        }
        ContinuationSupportStatus::Planned => {
            return ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::AgentContinuationPlanned,
            );
        }
        ContinuationSupportStatus::Available => {}
    }

    if provider.inbound_reply.mode != InboundReplyMode::LocalConnection {
        return ResponseSurfaceExposureDecision::Ineligible(
            ResponseSurfaceExposureSkipReason::ProviderDoesNotSupportLocalInbound,
        );
    }
    if provider.inbound_reply.requires_public_endpoint {
        return ResponseSurfaceExposureDecision::Ineligible(
            ResponseSurfaceExposureSkipReason::ProviderRequiresPublicEndpoint,
        );
    }
    if provider.inbound_reply.stable_event_id != StableEventIdCapability::Available {
        return ResponseSurfaceExposureDecision::Ineligible(
            ResponseSurfaceExposureSkipReason::ProviderStableEventIdUnavailable,
        );
    }
    if !provider
        .inbound_reply
        .reply_surfaces
        .iter()
        .any(|surface| RESPONSE_SURFACE_REPLY_SURFACES.contains(surface))
    {
        return ResponseSurfaceExposureDecision::Ineligible(
            ResponseSurfaceExposureSkipReason::ProviderReplySurfaceUnsupported,
        );
    }
    for field in RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS {
        if !provider.delivery_receipt.has_field(*field) {
            return ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::ProviderReceiptFieldUnsupported(*field),
            );
        }
    }
    if route.response_surface.is_disabled() {
        return ResponseSurfaceExposureDecision::Ineligible(
            ResponseSurfaceExposureSkipReason::RouteRepliesDisabled,
        );
    }

    ResponseSurfaceExposureDecision::Eligible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_integration_catalog::{
        AgentIntegrationDescriptor, AgentIntegrationId, ContinuationCapability,
        agent_integration_descriptor,
    };
    use crate::provider_catalog::{
        DeliveryReceiptCapability, DeliveryReceiptField, ProviderMode, provider_mode_capability,
    };

    const NO_REPLY_SURFACES: &[crate::provider_catalog::ProviderReplySurface] = &[];
    const INCOMPLETE_RECEIPT_FIELDS: &[DeliveryReceiptField] = &[
        DeliveryReceiptField::ProviderAccountId,
        DeliveryReceiptField::ProviderConversationId,
        DeliveryReceiptField::ProviderThreadId,
    ];

    #[test]
    fn planned_agent_integration_is_not_user_visible_reply_capability() {
        let agent = planned_codex_desktop();
        let mut route = route_with_replies_enabled();

        let decision = evaluate_response_surface_exposure(
            &agent,
            provider_mode_capability(ProviderMode::SlackApp),
            &route,
        );

        assert_eq!(
            decision,
            ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::AgentContinuationPlanned
            )
        );

        route.response_surface.enabled = false;
        assert_eq!(
            evaluate_response_surface_exposure(
                &available_codex_desktop(),
                provider_mode_capability(ProviderMode::SlackApp),
                &route,
            ),
            ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::RouteRepliesDisabled
            )
        );
    }

    #[test]
    fn current_agent_and_provider_mode_must_both_be_eligible() {
        let route = route_with_replies_enabled();

        assert_eq!(
            evaluate_response_surface_exposure(
                &available_codex_desktop(),
                provider_mode_capability(ProviderMode::SlackApp),
                &route,
            ),
            ResponseSurfaceExposureDecision::Eligible
        );
        assert_eq!(
            evaluate_response_surface_exposure(
                agent_integration_descriptor(AgentIntegrationId::ClaudeCode),
                provider_mode_capability(ProviderMode::SlackApp),
                &route,
            ),
            ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::AgentContinuationUnsupported
            )
        );
        assert_eq!(
            evaluate_response_surface_exposure(
                &available_codex_desktop(),
                provider_mode_capability(ProviderMode::SlackIncomingWebhook),
                &route,
            ),
            ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::ProviderDoesNotSupportLocalInbound
            )
        );
    }

    #[test]
    fn exposure_eligibility_matches_surface_and_receipt_capability() {
        let route = route_with_replies_enabled();
        let mut unsupported_surface = *provider_mode_capability(ProviderMode::SlackApp);
        unsupported_surface.inbound_reply.reply_surfaces = NO_REPLY_SURFACES;
        let mut incomplete_receipt = *provider_mode_capability(ProviderMode::SlackApp);
        incomplete_receipt.delivery_receipt = DeliveryReceiptCapability {
            fields: INCOMPLETE_RECEIPT_FIELDS,
        };

        assert_eq!(
            evaluate_response_surface_exposure(
                &available_codex_desktop(),
                &unsupported_surface,
                &route,
            ),
            ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::ProviderReplySurfaceUnsupported
            )
        );
        assert_eq!(
            evaluate_response_surface_exposure(
                &available_codex_desktop(),
                &incomplete_receipt,
                &route,
            ),
            ResponseSurfaceExposureDecision::Ineligible(
                ResponseSurfaceExposureSkipReason::ProviderReceiptFieldUnsupported(
                    DeliveryReceiptField::ProviderMessageId
                )
            )
        );
        assert_eq!(
            evaluate_response_surface_exposure(
                &available_codex_desktop(),
                provider_mode_capability(ProviderMode::FeishuLarkAppBot),
                &route,
            ),
            ResponseSurfaceExposureDecision::Eligible
        );
    }

    fn route_with_replies_enabled() -> RouteConfig {
        let mut route =
            RouteConfig::new(vec!["codex_desktop".to_string()], vec!["slack".to_string()]);
        route.response_surface.enabled = true;
        route
    }

    fn available_codex_desktop() -> AgentIntegrationDescriptor {
        let descriptor = *agent_integration_descriptor(AgentIntegrationId::CodexDesktop);
        let target = descriptor
            .continuation_capability
            .target()
            .expect("Codex Desktop continuation target should be cataloged");
        AgentIntegrationDescriptor {
            continuation_capability: ContinuationCapability::available_experimental(target),
            ..descriptor
        }
    }

    fn planned_codex_desktop() -> AgentIntegrationDescriptor {
        let descriptor = *agent_integration_descriptor(AgentIntegrationId::CodexDesktop);
        let target = descriptor
            .continuation_capability
            .target()
            .expect("Codex Desktop continuation target should be cataloged");
        AgentIntegrationDescriptor {
            continuation_capability: ContinuationCapability::Planned(target),
            ..descriptor
        }
    }
}
