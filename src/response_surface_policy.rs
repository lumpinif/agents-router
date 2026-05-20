use crate::agent_integration_catalog::{
    AgentControllerKind, AgentIntegrationDescriptor, AgentIntegrationId, ContinuationCapability,
    ContinuationSessionBinding, agent_integration_for_source,
};
use crate::config::{RouteConfig, SourceType};
use crate::provider_catalog::{
    DeliveryReceiptField, InboundReplyMode, ProviderMode, ProviderModeCapability,
    RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS, RESPONSE_SURFACE_REPLY_SURFACES,
    StableEventIdCapability,
};
use crate::signal::Signal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseSurfacePolicyCheck {
    SurfaceCreation,
    InboundContinuation,
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseSurfacePolicyInput<'a> {
    pub check: ResponseSurfacePolicyCheck,
    pub provider: &'a ProviderModeCapability,
    pub agent_integration: Option<&'a AgentIntegrationDescriptor>,
    pub route: &'a RouteConfig,
    pub signal: &'a Signal,
    pub delivery_receipt: &'a ResponseSurfaceDeliveryReceipt,
}

impl<'a> ResponseSurfacePolicyInput<'a> {
    pub fn from_catalog(
        check: ResponseSurfacePolicyCheck,
        provider: &'a ProviderModeCapability,
        route: &'a RouteConfig,
        signal: &'a Signal,
        delivery_receipt: &'a ResponseSurfaceDeliveryReceipt,
    ) -> Self {
        Self {
            check,
            provider,
            agent_integration: agent_integration_for_signal(signal),
            route,
            signal,
            delivery_receipt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResponseSurfaceDeliveryReceipt {
    pub provider_account_id: Option<String>,
    pub provider_conversation_id: Option<String>,
    pub provider_message_id: Option<String>,
    pub provider_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseSurfacePolicyDecision {
    Allow(ResponseSurfacePolicyAllow),
    Skip(ResponseSurfacePolicySkipReason),
}

impl ResponseSurfacePolicyDecision {
    pub fn skip_reason(&self) -> Option<&ResponseSurfacePolicySkipReason> {
        match self {
            Self::Allow(_) => None,
            Self::Skip(reason) => Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseSurfacePolicyAllow {
    pub check: ResponseSurfacePolicyCheck,
    pub provider_mode: ProviderMode,
    pub agent_integration_id: AgentIntegrationId,
    pub controller_kind: AgentControllerKind,
    pub source_session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseSurfacePolicySkipReason {
    RouteRepliesDisabled,
    ProviderInboundReplyNotLocal,
    ProviderRequiresPublicEndpoint,
    ProviderStableEventIdUnavailable,
    ProviderReplySurfaceUnsupported,
    ProviderReceiptFieldUnsupported(DeliveryReceiptField),
    AgentIntegrationNotFound,
    AgentIntegrationSourceMismatch,
    AgentContinuationUnsupported,
    AgentContinuationPlanned,
    SignalSessionIdMissing,
    DeliveryReceiptFieldMissing(DeliveryReceiptField),
}

pub fn evaluate_response_surface_policy(
    input: ResponseSurfacePolicyInput<'_>,
) -> ResponseSurfacePolicyDecision {
    if !input.route.response_surface.enabled {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::RouteRepliesDisabled,
        );
    }

    if input.provider.inbound_reply.mode != InboundReplyMode::LocalConnection {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::ProviderInboundReplyNotLocal,
        );
    }

    if input.provider.inbound_reply.requires_public_endpoint {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::ProviderRequiresPublicEndpoint,
        );
    }

    if input.provider.inbound_reply.stable_event_id != StableEventIdCapability::Available {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::ProviderStableEventIdUnavailable,
        );
    }

    if !input
        .provider
        .inbound_reply
        .reply_surfaces
        .iter()
        .any(|surface| RESPONSE_SURFACE_REPLY_SURFACES.contains(surface))
    {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::ProviderReplySurfaceUnsupported,
        );
    }

    for field in RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS {
        if !input.provider.delivery_receipt.has_field(*field) {
            return ResponseSurfacePolicyDecision::Skip(
                ResponseSurfacePolicySkipReason::ProviderReceiptFieldUnsupported(*field),
            );
        }
    }

    let Some(agent_integration) = input.agent_integration else {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::AgentIntegrationNotFound,
        );
    };

    if !agent_integration_matches_signal(agent_integration, input.signal) {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::AgentIntegrationSourceMismatch,
        );
    }

    let target = match agent_integration.continuation_capability {
        ContinuationCapability::Unsupported => {
            return ResponseSurfacePolicyDecision::Skip(
                ResponseSurfacePolicySkipReason::AgentContinuationUnsupported,
            );
        }
        ContinuationCapability::Planned(_) => {
            return ResponseSurfacePolicyDecision::Skip(
                ResponseSurfacePolicySkipReason::AgentContinuationPlanned,
            );
        }
        ContinuationCapability::Available(target) => target,
    };

    let Some(source_session_id) =
        source_session_id_for_binding(input.signal, target.session_binding)
    else {
        return ResponseSurfacePolicyDecision::Skip(
            ResponseSurfacePolicySkipReason::SignalSessionIdMissing,
        );
    };

    for field in RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS {
        if !input.delivery_receipt.has_field(*field) {
            return ResponseSurfacePolicyDecision::Skip(
                ResponseSurfacePolicySkipReason::DeliveryReceiptFieldMissing(*field),
            );
        }
    }

    ResponseSurfacePolicyDecision::Allow(ResponseSurfacePolicyAllow {
        check: input.check,
        provider_mode: input.provider.mode,
        agent_integration_id: agent_integration.id,
        controller_kind: target.controller_kind,
        source_session_id,
    })
}

pub fn agent_integration_for_signal(
    signal: &Signal,
) -> Option<&'static AgentIntegrationDescriptor> {
    let source_type = SourceType::from_signal_value(signal.source_type())?;
    agent_integration_for_source(signal.source_id(), source_type)
}

fn agent_integration_matches_signal(
    agent_integration: &AgentIntegrationDescriptor,
    signal: &Signal,
) -> bool {
    let source = agent_integration.source_capability;
    signal.source_id() == source.canonical_source_id
        && signal.source_type() == source.source_type.as_str()
}

fn source_session_id_for_binding(
    signal: &Signal,
    binding: ContinuationSessionBinding,
) -> Option<String> {
    match binding {
        ContinuationSessionBinding::SignalConversationSessionId => signal
            .conversation
            .as_ref()
            .and_then(|conversation| present(conversation.session_id.as_deref()))
            .map(str::to_string),
    }
}

impl ResponseSurfaceDeliveryReceipt {
    fn has_field(&self, field: DeliveryReceiptField) -> bool {
        match field {
            DeliveryReceiptField::ProviderAccountId => present(self.provider_account_id.as_deref()),
            DeliveryReceiptField::ProviderConversationId => {
                present(self.provider_conversation_id.as_deref())
            }
            DeliveryReceiptField::ProviderMessageId => present(self.provider_message_id.as_deref()),
            DeliveryReceiptField::ProviderThreadId => present(self.provider_thread_id.as_deref()),
        }
        .is_some()
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::agent_integration_catalog::{
        AgentIntegrationId, ContinuationCapability, agent_integration_descriptor,
    };
    use crate::provider_catalog::{
        DeliveryReceiptCapability, ProviderModeCapability, ProviderReplySurface,
        provider_mode_capability,
    };
    use crate::signal::{
        Signal, SignalConversation, SignalDisplay, SignalEvent, SignalEventKind, SignalSource,
    };

    #[test]
    fn allows_surface_only_when_all_policy_inputs_are_available() {
        let fixture = PolicyFixture::allowable();

        let decision = evaluate_response_surface_policy(fixture.input());

        assert_eq!(
            decision,
            ResponseSurfacePolicyDecision::Allow(ResponseSurfacePolicyAllow {
                check: ResponseSurfacePolicyCheck::SurfaceCreation,
                provider_mode: ProviderMode::SlackApp,
                agent_integration_id: AgentIntegrationId::CodexDesktop,
                controller_kind: AgentControllerKind::CodexAppServer,
                source_session_id: "session-1".to_string(),
            })
        );
    }

    #[test]
    fn inbound_continuation_uses_the_same_policy_gate() {
        let mut fixture = PolicyFixture::allowable();
        fixture.check = ResponseSurfacePolicyCheck::InboundContinuation;

        let decision = evaluate_response_surface_policy(fixture.input());

        assert_eq!(
            decision,
            ResponseSurfacePolicyDecision::Allow(ResponseSurfacePolicyAllow {
                check: ResponseSurfacePolicyCheck::InboundContinuation,
                provider_mode: ProviderMode::SlackApp,
                agent_integration_id: AgentIntegrationId::CodexDesktop,
                controller_kind: AgentControllerKind::CodexAppServer,
                source_session_id: "session-1".to_string(),
            })
        );
    }

    #[test]
    fn skips_when_route_replies_are_disabled_by_default() {
        let mut fixture = PolicyFixture::allowable();
        fixture.route.response_surface.enabled = false;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::RouteRepliesDisabled,
        );
    }

    #[test]
    fn skips_current_webhook_only_provider_modes() {
        for mode in [
            ProviderMode::SlackIncomingWebhook,
            ProviderMode::FeishuLarkCustomBot,
            ProviderMode::Webhook,
        ] {
            let mut fixture = PolicyFixture::allowable();
            fixture.provider = provider_mode_capability(mode);

            assert_skip(
                fixture.input(),
                ResponseSurfacePolicySkipReason::ProviderInboundReplyNotLocal,
            );
        }
    }

    #[test]
    fn skips_provider_modes_that_require_public_endpoints() {
        let mut provider = *provider_mode_capability(ProviderMode::SlackApp);
        provider.inbound_reply.requires_public_endpoint = true;
        let mut fixture = PolicyFixture::allowable();
        fixture.provider = &provider;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::ProviderRequiresPublicEndpoint,
        );
    }

    #[test]
    fn skips_provider_modes_without_stable_event_ids() {
        let mut provider = *provider_mode_capability(ProviderMode::SlackApp);
        provider.inbound_reply.stable_event_id = StableEventIdCapability::Unavailable;
        let mut fixture = PolicyFixture::allowable();
        fixture.provider = &provider;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::ProviderStableEventIdUnavailable,
        );
    }

    #[test]
    fn skips_provider_modes_without_supported_reply_surface() {
        let mut provider = *provider_mode_capability(ProviderMode::SlackApp);
        provider.inbound_reply.reply_surfaces = &[];
        let mut fixture = PolicyFixture::allowable();
        fixture.provider = &provider;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::ProviderReplySurfaceUnsupported,
        );
    }

    #[test]
    fn allows_message_reply_surface_when_other_policy_inputs_are_available() {
        let mut provider = *provider_mode_capability(ProviderMode::SlackApp);
        provider.inbound_reply.reply_surfaces = &[ProviderReplySurface::MessageReply];
        let mut fixture = PolicyFixture::allowable();
        fixture.provider = &provider;

        assert!(matches!(
            evaluate_response_surface_policy(fixture.input()),
            ResponseSurfacePolicyDecision::Allow(_)
        ));
    }

    #[test]
    fn skips_provider_modes_without_required_receipt_capability() {
        let mut provider = *provider_mode_capability(ProviderMode::SlackApp);
        provider.delivery_receipt = DeliveryReceiptCapability {
            fields: &[
                DeliveryReceiptField::ProviderAccountId,
                DeliveryReceiptField::ProviderConversationId,
                DeliveryReceiptField::ProviderMessageId,
            ],
        };
        let mut fixture = PolicyFixture::allowable();
        fixture.provider = &provider;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::ProviderReceiptFieldUnsupported(
                DeliveryReceiptField::ProviderThreadId,
            ),
        );
    }

    #[test]
    fn skips_when_no_cataloged_agent_integration_is_supplied() {
        let mut fixture = PolicyFixture::allowable();
        fixture.agent_integration = None;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::AgentIntegrationNotFound,
        );
    }

    #[test]
    fn skips_when_supplied_agent_integration_does_not_match_signal_source() {
        let mut fixture = PolicyFixture::allowable();
        fixture.signal.source.id = "my_desktop".to_string();

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::AgentIntegrationSourceMismatch,
        );
    }

    #[test]
    fn skips_unsupported_agent_continuation() {
        let mut fixture = PolicyFixture::allowable();
        fixture.agent_integration =
            Some(*agent_integration_descriptor(AgentIntegrationId::CodexCli));
        fixture.signal.source.id = "codex_cli".to_string();
        fixture.signal.source.source_type = "codex_cli".to_string();

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::AgentContinuationUnsupported,
        );
    }

    #[test]
    fn skips_planned_agent_continuation_from_production_catalog() {
        let mut fixture = PolicyFixture::allowable();
        fixture.agent_integration = Some(*agent_integration_descriptor(
            AgentIntegrationId::CodexDesktop,
        ));

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::AgentContinuationPlanned,
        );
    }

    #[test]
    fn skips_when_signal_lacks_required_session_id() {
        let mut fixture = PolicyFixture::allowable();
        fixture
            .signal
            .conversation
            .as_mut()
            .expect("fixture signal has conversation")
            .session_id = None;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::SignalSessionIdMissing,
        );
    }

    #[test]
    fn skips_when_delivery_receipt_lacks_required_field() {
        let mut fixture = PolicyFixture::allowable();
        fixture.delivery_receipt.provider_thread_id = None;

        assert_skip(
            fixture.input(),
            ResponseSurfacePolicySkipReason::DeliveryReceiptFieldMissing(
                DeliveryReceiptField::ProviderThreadId,
            ),
        );
    }

    #[test]
    fn catalog_signal_lookup_requires_canonical_source_id_and_type() {
        let mut signal = codex_desktop_signal();
        signal.source.id = "my_desktop".to_string();
        let fixture = PolicyFixture {
            signal,
            ..PolicyFixture::allowable()
        };

        let input = ResponseSurfacePolicyInput::from_catalog(
            ResponseSurfacePolicyCheck::SurfaceCreation,
            fixture.provider,
            &fixture.route,
            &fixture.signal,
            &fixture.delivery_receipt,
        );

        assert_skip(
            input,
            ResponseSurfacePolicySkipReason::AgentIntegrationNotFound,
        );
    }

    fn assert_skip(
        input: ResponseSurfacePolicyInput<'_>,
        expected: ResponseSurfacePolicySkipReason,
    ) {
        assert_eq!(
            evaluate_response_surface_policy(input),
            ResponseSurfacePolicyDecision::Skip(expected)
        );
    }

    struct PolicyFixture<'a> {
        check: ResponseSurfacePolicyCheck,
        provider: &'a ProviderModeCapability,
        agent_integration: Option<AgentIntegrationDescriptor>,
        route: RouteConfig,
        signal: Signal,
        delivery_receipt: ResponseSurfaceDeliveryReceipt,
    }

    impl<'a> PolicyFixture<'a> {
        fn allowable() -> Self {
            Self {
                check: ResponseSurfacePolicyCheck::SurfaceCreation,
                provider: provider_mode_capability(ProviderMode::SlackApp),
                agent_integration: Some(available_codex_desktop()),
                route: replies_enabled_route(),
                signal: codex_desktop_signal(),
                delivery_receipt: complete_receipt(),
            }
        }

        fn input(&'a self) -> ResponseSurfacePolicyInput<'a> {
            ResponseSurfacePolicyInput {
                check: self.check,
                provider: self.provider,
                agent_integration: self.agent_integration.as_ref(),
                route: &self.route,
                signal: &self.signal,
                delivery_receipt: &self.delivery_receipt,
            }
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

    fn replies_enabled_route() -> RouteConfig {
        let mut route =
            RouteConfig::new(vec!["codex_desktop".to_string()], vec!["slack".to_string()]);
        route.response_surface.enabled = true;
        route
    }

    fn codex_desktop_signal() -> Signal {
        let mut signal = Signal::new_structured_with_timestamp(
            "signal-1",
            SignalSource {
                id: "codex_desktop".to_string(),
                source_type: "codex_desktop".to_string(),
            },
            SignalEvent {
                kind: SignalEventKind::TurnCompleted,
                raw_name: None,
            },
            SignalDisplay {
                title: "Codex Desktop".to_string(),
                summary: "Task complete.".to_string(),
            },
            Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
                .single()
                .expect("test timestamp should be valid"),
            Default::default(),
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

    fn complete_receipt() -> ResponseSurfaceDeliveryReceipt {
        ResponseSurfaceDeliveryReceipt {
            provider_account_id: Some("T123".to_string()),
            provider_conversation_id: Some("C123".to_string()),
            provider_message_id: Some("1716200000.000100".to_string()),
            provider_thread_id: Some("1716200000.000100".to_string()),
        }
    }
}
