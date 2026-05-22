// Hidden Phase 2 contract; live service wiring stays closed until exposure gates open.
#![allow(dead_code)]

use std::fmt;

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};

use crate::config::{
    FeishuLarkAppDomain, FeishuLarkProviderConfig, ProviderConfig, ProviderConfigDetail,
    ProviderType,
};
use crate::provider_catalog::{ProviderMode, provider_mode_capability};
use crate::provider_inbound::{
    ProviderInboundDecision, ProviderInboundNormalizeResult, ProviderInboundReady,
    ProviderInboundSkipReason, lookup_and_claim_provider_surface_reply,
    normalize_feishu_lark_long_connection_surface_reply,
};
use crate::response_surface_ledger::ResponseSurfaceLedger;

#[derive(Clone)]
pub(crate) struct FeishuLarkLongConnectionConfig {
    pub provider_id: String,
    pub domain: FeishuLarkAppDomain,
    pub app_id: String,
    app_secret: String,
}

impl fmt::Debug for FeishuLarkLongConnectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeishuLarkLongConnectionConfig")
            .field("provider_id", &self.provider_id)
            .field("domain", &self.domain)
            .field("app_id", &self.app_id)
            .field("app_secret", &"[redacted]")
            .finish()
    }
}

impl FeishuLarkLongConnectionConfig {
    pub fn from_provider_config(provider: &ProviderConfig) -> anyhow::Result<Self> {
        let ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(detail)) =
            &provider.detail
        else {
            anyhow::bail!("Feishu/Lark long connection runtime requires explicit App Bot mode");
        };

        let app_secret = detail
            .app_secret
            .resolve_runtime_value(
                &provider.id,
                ProviderType::FeishuLark.as_str(),
                "app_secret",
            )
            .context("failed to resolve Feishu/Lark App Bot app_secret")?;

        ensure!(
            !detail.app_id.trim().is_empty(),
            "Feishu/Lark long connection runtime requires app_id"
        );
        ensure!(
            !app_secret.trim().is_empty(),
            "Feishu/Lark long connection runtime requires app_secret"
        );

        Ok(Self {
            provider_id: provider.id.clone(),
            domain: detail.domain,
            app_id: detail.app_id.clone(),
            app_secret,
        })
    }

    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkLongConnectionEvent<'a> {
    pub raw_event: &'a [u8],
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeishuLarkPlatformAck {
    Acknowledge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeishuLarkLongConnectionDecision {
    AckReadyAfterLocalClaim(ProviderInboundReady),
    AckSkip(ProviderInboundSkipReason),
}

impl FeishuLarkLongConnectionDecision {
    pub fn platform_ack(&self) -> FeishuLarkPlatformAck {
        FeishuLarkPlatformAck::Acknowledge
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FeishuLarkLongConnectionRuntime {
    config: FeishuLarkLongConnectionConfig,
}

impl FeishuLarkLongConnectionRuntime {
    pub fn from_provider_config(provider: &ProviderConfig) -> anyhow::Result<Self> {
        Ok(Self {
            config: FeishuLarkLongConnectionConfig::from_provider_config(provider)?,
        })
    }

    pub fn connection_config(&self) -> &FeishuLarkLongConnectionConfig {
        &self.config
    }

    pub fn handle_event_before_platform_ack(
        &self,
        ledger: &mut ResponseSurfaceLedger,
        event: FeishuLarkLongConnectionEvent<'_>,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        let normalized = normalize_feishu_lark_long_connection_surface_reply(
            &self.config.provider_id,
            event.raw_event,
        )?;

        let reply = match normalized {
            ProviderInboundNormalizeResult::SurfaceReply(reply) => reply,
            ProviderInboundNormalizeResult::Skip(reason) => {
                return Ok(FeishuLarkLongConnectionDecision::AckSkip(reason));
            }
        };

        let decision = lookup_and_claim_provider_surface_reply(
            ledger,
            provider_mode_capability(ProviderMode::FeishuLarkAppBot),
            reply,
            event.received_at,
        )?;

        // Platform ack only means the local machine has safely skipped or
        // claimed the event. Agent execution can run longer and finishes later.
        Ok(match decision {
            ProviderInboundDecision::Ready(ready) => {
                FeishuLarkLongConnectionDecision::AckReadyAfterLocalClaim(ready)
            }
            ProviderInboundDecision::Skip(reason) => {
                FeishuLarkLongConnectionDecision::AckSkip(reason)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::config::{
        FeishuLarkAppBotProviderConfig, FeishuLarkCustomBotProviderConfig, SecretSource, UrlSource,
    };
    use crate::response_surface_ledger::NewResponseSurface;

    #[test]
    fn app_bot_runtime_uses_official_long_connection_credentials_only() {
        let runtime = FeishuLarkLongConnectionRuntime::from_provider_config(&app_bot_provider())
            .expect("App Bot should build hidden long connection runtime");

        assert_eq!(runtime.connection_config().provider_id, "lark-app");
        assert_eq!(
            runtime.connection_config().domain,
            FeishuLarkAppDomain::Lark
        );
        assert_eq!(runtime.connection_config().app_id, "cli_test_app");
        assert_eq!(runtime.connection_config().app_secret(), "test-secret");
        assert!(!format!("{:?}", runtime.connection_config()).contains("test-secret"));
    }

    #[test]
    fn custom_bot_runtime_is_rejected_instead_of_inferred_from_fields() {
        let error = FeishuLarkLongConnectionRuntime::from_provider_config(&ProviderConfig {
            id: "lark-custom".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::CustomBot(
                FeishuLarkCustomBotProviderConfig {
                    url: UrlSource::Inline(
                        "https://open.larksuite.com/open-apis/bot/v2/hook/test".to_string(),
                    ),
                    secret: None,
                },
            )),
        })
        .expect_err("Custom Bot must not enter long connection runtime");

        assert!(error.to_string().contains("explicit App Bot mode"));
    }

    #[test]
    fn surface_reply_is_claimed_before_platform_ack_ready() {
        let now = test_time();
        let runtime = FeishuLarkLongConnectionRuntime::from_provider_config(&app_bot_provider())
            .expect("App Bot should build hidden long connection runtime");
        let mut ledger = ledger_with_lark_surface(now);

        let decision = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(1)))
            .expect("event handling should succeed");

        assert_eq!(decision.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        let FeishuLarkLongConnectionDecision::AckReadyAfterLocalClaim(ready) = decision else {
            panic!("surface reply should be ready after local claim");
        };
        assert_eq!(ready.surface.source_session_id, "session-1");
        assert_eq!(ready.reply.provider_id, "lark-app");
        assert_eq!(ready.reply.provider_thread_id, "om_root_message_id");
        assert_eq!(ready.reply.reply_text, "@_user_1 continue with README");

        let duplicate = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(2)))
            .expect("duplicate event handling should succeed");
        assert_eq!(duplicate.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        assert!(matches!(
            duplicate,
            FeishuLarkLongConnectionDecision::AckSkip(
                ProviderInboundSkipReason::EventAlreadyProcessing { .. }
            )
        ));
    }

    #[test]
    fn lookup_miss_is_ack_skipped_without_claiming_event() {
        let now = test_time();
        let runtime = FeishuLarkLongConnectionRuntime::from_provider_config(&app_bot_provider())
            .expect("App Bot should build hidden long connection runtime");
        let mut ledger = ResponseSurfaceLedger::in_memory();

        let decision = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(1)))
            .expect("lookup miss should not fail");

        assert_eq!(decision.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        assert_eq!(
            decision,
            FeishuLarkLongConnectionDecision::AckSkip(ProviderInboundSkipReason::SurfaceLookupMiss)
        );

        ledger
            .create_surface_at(lark_surface(now), now + Duration::seconds(2))
            .expect("surface should be creatable after lookup miss");
        let later = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(3)))
            .expect("event should still be claimable after surface appears");
        assert!(matches!(
            later,
            FeishuLarkLongConnectionDecision::AckReadyAfterLocalClaim(_)
        ));
    }

    #[test]
    fn rootless_mention_is_ack_skipped_without_surface_trigger() {
        let now = test_time();
        let runtime = FeishuLarkLongConnectionRuntime::from_provider_config(&app_bot_provider())
            .expect("App Bot should build hidden long connection runtime");
        let mut ledger = ledger_with_lark_surface(now);

        let decision = runtime
            .handle_event_before_platform_ack(
                &mut ledger,
                FeishuLarkLongConnectionEvent {
                    raw_event: br#"{
                        "schema": "2.0",
                        "header": {
                            "event_id": "event-1",
                            "event_type": "im.message.receive_v1",
                            "tenant_key": "2ca1d211f64f6438"
                        },
                        "event": {
                            "sender": { "sender_type": "user" },
                            "message": {
                                "message_id": "om_rootless_reply",
                                "root_id": "",
                                "chat_id": "oc_5ce6d572455d361153b7xx51da133945",
                                "message_type": "text",
                                "content": "{\"text\":\"@_user_1 continue\"}"
                            }
                        }
                    }"#,
                    received_at: now + Duration::seconds(1),
                },
            )
            .expect("rootless mention should parse as skip");

        assert_eq!(decision.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        assert_eq!(
            decision,
            FeishuLarkLongConnectionDecision::AckSkip(ProviderInboundSkipReason::NotSurfaceReply)
        );
    }

    fn app_bot_provider() -> ProviderConfig {
        ProviderConfig {
            id: "lark-app".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(
                FeishuLarkAppBotProviderConfig {
                    domain: FeishuLarkAppDomain::Lark,
                    app_id: "cli_test_app".to_string(),
                    app_secret: SecretSource::Inline("test-secret".to_string()),
                    tenant_key: "2ca1d211f64f6438".to_string(),
                    chat_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                },
            )),
        }
    }

    fn ledger_with_lark_surface(now: DateTime<Utc>) -> ResponseSurfaceLedger {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        ledger
            .create_surface_at(lark_surface(now), now)
            .expect("surface should be created");
        ledger
    }

    fn lark_surface(_now: DateTime<Utc>) -> NewResponseSurface {
        NewResponseSurface {
            signal_id: "signal-1".to_string(),
            delivery_id: "delivery-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            provider_id: "lark-app".to_string(),
            provider_type: ProviderType::FeishuLark.as_str().to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: "2ca1d211f64f6438".to_string(),
            provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
            provider_message_id: "om_root_message_id".to_string(),
            provider_thread_id: "om_root_message_id".to_string(),
        }
    }

    fn event_at(received_at: DateTime<Utc>) -> FeishuLarkLongConnectionEvent<'static> {
        FeishuLarkLongConnectionEvent {
            raw_event: include_bytes!(
                "../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
            ),
            received_at,
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
