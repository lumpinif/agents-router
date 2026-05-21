use serde::{Deserialize, Serialize};

use crate::config::{FeishuLarkProviderConfig, ProviderConfig, ProviderConfigDetail, ProviderType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider_type: ProviderType,
    pub display_name: &'static str,
    pub setup_order: u16,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub message_constraints: &'static [ProviderMessageConstraint],
    pub modes: &'static [ProviderModeCapability],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderModeCapability {
    pub provider_type: ProviderType,
    pub mode: ProviderMode,
    pub display_name: &'static str,
    pub inbound_reply: InboundReplyCapability,
    pub delivery_receipt: DeliveryReceiptCapability,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    NtfyTopic,
    Webhook,
    FeishuLarkCustomBot,
    FeishuLarkAppBot,
    PushoverMessagesApi,
    SlackIncomingWebhook,
    SlackApp,
    DiscordWebhook,
    TelegramBotApi,
    WhatsappCloudApi,
    WechatIlink,
    MicrosoftTeamsWorkflowWebhook,
    EmailSmtp,
}

impl ProviderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NtfyTopic => "ntfy_topic",
            Self::Webhook => "webhook",
            Self::FeishuLarkCustomBot => "feishu_lark_custom_bot",
            Self::FeishuLarkAppBot => "feishu_lark_app_bot",
            Self::PushoverMessagesApi => "pushover_messages_api",
            Self::SlackIncomingWebhook => "slack_incoming_webhook",
            Self::SlackApp => "slack_app",
            Self::DiscordWebhook => "discord_webhook",
            Self::TelegramBotApi => "telegram_bot_api",
            Self::WhatsappCloudApi => "whatsapp_cloud_api",
            Self::WechatIlink => "wechat_ilink",
            Self::MicrosoftTeamsWorkflowWebhook => "microsoft_teams_workflow_webhook",
            Self::EmailSmtp => "email_smtp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundReplyCapability {
    pub mode: InboundReplyMode,
    pub local_connection_kind: Option<LocalConnectionKind>,
    pub requires_public_endpoint: bool,
    pub stable_event_id: StableEventIdCapability,
    pub reply_surfaces: &'static [ProviderReplySurface],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundReplyMode {
    None,
    LocalConnection,
    PublicWebhookOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalConnectionKind {
    SocketMode,
    Websocket,
    LongConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableEventIdCapability {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderReplySurface {
    ThreadRootReply,
    MessageReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryReceiptCapability {
    pub fields: &'static [DeliveryReceiptField],
}

impl DeliveryReceiptCapability {
    pub fn has_field(self, field: DeliveryReceiptField) -> bool {
        self.fields.contains(&field)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryReceiptField {
    ProviderAccountId,
    ProviderConversationId,
    ProviderMessageId,
    ProviderThreadId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderMessageConstraint {
    pub surface: MessageSurface,
    pub unit: MessageLimitUnit,
    pub limit: Option<usize>,
    pub source: MessageConstraintSource,
    pub enforcement: MessageConstraintEnforcement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSurface {
    Title,
    MessageBody,
    TextBody,
    WebhookContent,
    Payload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLimitUnit {
    Characters,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageConstraintSource {
    ProviderDocumented,
    ProviderDocumentedDefault,
    LocalDeliveryGuard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageConstraintEnforcement {
    PolicyOnly,
    LocalPreflight,
}

const NO_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[];
const NO_REPLY_SURFACES: &[ProviderReplySurface] = &[];
const THREAD_ROOT_REPLY_SURFACES: &[ProviderReplySurface] =
    &[ProviderReplySurface::ThreadRootReply];
pub const RESPONSE_SURFACE_REPLY_SURFACES: &[ProviderReplySurface] = &[
    ProviderReplySurface::ThreadRootReply,
    ProviderReplySurface::MessageReply,
];
const NO_DELIVERY_RECEIPT_FIELDS: &[DeliveryReceiptField] = &[];
const FEISHU_LARK_CANDIDATE_DELIVERY_RECEIPT_FIELDS: &[DeliveryReceiptField] = &[
    DeliveryReceiptField::ProviderAccountId,
    DeliveryReceiptField::ProviderConversationId,
    DeliveryReceiptField::ProviderMessageId,
];
pub const RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS: &[DeliveryReceiptField] = &[
    DeliveryReceiptField::ProviderAccountId,
    DeliveryReceiptField::ProviderConversationId,
    DeliveryReceiptField::ProviderMessageId,
    DeliveryReceiptField::ProviderThreadId,
];

const OUTBOUND_ONLY_INBOUND_REPLY: InboundReplyCapability = InboundReplyCapability {
    mode: InboundReplyMode::None,
    local_connection_kind: None,
    requires_public_endpoint: false,
    stable_event_id: StableEventIdCapability::Unavailable,
    reply_surfaces: NO_REPLY_SURFACES,
};

const SLACK_SOCKET_MODE_INBOUND_REPLY: InboundReplyCapability = InboundReplyCapability {
    mode: InboundReplyMode::LocalConnection,
    local_connection_kind: Some(LocalConnectionKind::SocketMode),
    requires_public_endpoint: false,
    stable_event_id: StableEventIdCapability::Available,
    reply_surfaces: THREAD_ROOT_REPLY_SURFACES,
};

const FEISHU_LARK_LONG_CONNECTION_INBOUND_REPLY: InboundReplyCapability = InboundReplyCapability {
    mode: InboundReplyMode::LocalConnection,
    local_connection_kind: Some(LocalConnectionKind::LongConnection),
    requires_public_endpoint: false,
    stable_event_id: StableEventIdCapability::Available,
    reply_surfaces: THREAD_ROOT_REPLY_SURFACES,
};

const NO_DELIVERY_RECEIPT_CAPABILITY: DeliveryReceiptCapability = DeliveryReceiptCapability {
    fields: NO_DELIVERY_RECEIPT_FIELDS,
};

const RESPONSE_SURFACE_DELIVERY_RECEIPT_CAPABILITY: DeliveryReceiptCapability =
    DeliveryReceiptCapability {
        fields: RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS,
    };
const FEISHU_LARK_CANDIDATE_DELIVERY_RECEIPT_CAPABILITY: DeliveryReceiptCapability =
    DeliveryReceiptCapability {
        fields: FEISHU_LARK_CANDIDATE_DELIVERY_RECEIPT_FIELDS,
    };

const NTFY_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Ntfy,
    mode: ProviderMode::NtfyTopic,
    display_name: "ntfy topic",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const WEBHOOK_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Webhook,
    mode: ProviderMode::Webhook,
    display_name: "Webhook",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const FEISHU_LARK_PROVIDER_MODES: &[ProviderModeCapability] = &[
    ProviderModeCapability {
        provider_type: ProviderType::FeishuLark,
        mode: ProviderMode::FeishuLarkCustomBot,
        display_name: "Feishu/Lark custom bot",
        inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
        delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
    },
    ProviderModeCapability {
        provider_type: ProviderType::FeishuLark,
        mode: ProviderMode::FeishuLarkAppBot,
        display_name: "Feishu/Lark app bot",
        inbound_reply: FEISHU_LARK_LONG_CONNECTION_INBOUND_REPLY,
        delivery_receipt: FEISHU_LARK_CANDIDATE_DELIVERY_RECEIPT_CAPABILITY,
    },
];

const PUSHOVER_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Pushover,
    mode: ProviderMode::PushoverMessagesApi,
    display_name: "Pushover Messages API",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const SLACK_PROVIDER_MODES: &[ProviderModeCapability] = &[
    ProviderModeCapability {
        provider_type: ProviderType::Slack,
        mode: ProviderMode::SlackIncomingWebhook,
        display_name: "Slack incoming webhook",
        inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
        delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
    },
    ProviderModeCapability {
        provider_type: ProviderType::Slack,
        mode: ProviderMode::SlackApp,
        display_name: "Slack app",
        inbound_reply: SLACK_SOCKET_MODE_INBOUND_REPLY,
        delivery_receipt: RESPONSE_SURFACE_DELIVERY_RECEIPT_CAPABILITY,
    },
];

const DISCORD_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Discord,
    mode: ProviderMode::DiscordWebhook,
    display_name: "Discord webhook",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const TELEGRAM_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Telegram,
    mode: ProviderMode::TelegramBotApi,
    display_name: "Telegram Bot API",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const WHATSAPP_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Whatsapp,
    mode: ProviderMode::WhatsappCloudApi,
    display_name: "WhatsApp Cloud API",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const WECHAT_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::Wechat,
    mode: ProviderMode::WechatIlink,
    display_name: "WeChat iLink",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const MICROSOFT_TEAMS_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::MicrosoftTeams,
    mode: ProviderMode::MicrosoftTeamsWorkflowWebhook,
    display_name: "Microsoft Teams workflow webhook",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const EMAIL_SMTP_PROVIDER_MODES: &[ProviderModeCapability] = &[ProviderModeCapability {
    provider_type: ProviderType::EmailSmtp,
    mode: ProviderMode::EmailSmtp,
    display_name: "Email SMTP",
    inbound_reply: OUTBOUND_ONLY_INBOUND_REPLY,
    delivery_receipt: NO_DELIVERY_RECEIPT_CAPABILITY,
}];

const NTFY_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[ProviderMessageConstraint {
    surface: MessageSurface::MessageBody,
    unit: MessageLimitUnit::Characters,
    limit: Some(4096),
    source: MessageConstraintSource::ProviderDocumentedDefault,
    enforcement: MessageConstraintEnforcement::PolicyOnly,
}];

const PUSHOVER_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[
    ProviderMessageConstraint {
        surface: MessageSurface::Title,
        unit: MessageLimitUnit::Characters,
        limit: Some(250),
        source: MessageConstraintSource::ProviderDocumented,
        enforcement: MessageConstraintEnforcement::LocalPreflight,
    },
    ProviderMessageConstraint {
        surface: MessageSurface::MessageBody,
        unit: MessageLimitUnit::Characters,
        limit: Some(1024),
        source: MessageConstraintSource::ProviderDocumented,
        enforcement: MessageConstraintEnforcement::LocalPreflight,
    },
];

const SLACK_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[ProviderMessageConstraint {
    surface: MessageSurface::TextBody,
    unit: MessageLimitUnit::Characters,
    limit: Some(4000),
    source: MessageConstraintSource::LocalDeliveryGuard,
    enforcement: MessageConstraintEnforcement::LocalPreflight,
}];

const DISCORD_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[ProviderMessageConstraint {
    surface: MessageSurface::WebhookContent,
    unit: MessageLimitUnit::Characters,
    limit: Some(2000),
    source: MessageConstraintSource::ProviderDocumented,
    enforcement: MessageConstraintEnforcement::LocalPreflight,
}];

const TELEGRAM_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[ProviderMessageConstraint {
    surface: MessageSurface::TextBody,
    unit: MessageLimitUnit::Characters,
    limit: Some(4096),
    source: MessageConstraintSource::ProviderDocumented,
    enforcement: MessageConstraintEnforcement::LocalPreflight,
}];

const WHATSAPP_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[ProviderMessageConstraint {
    surface: MessageSurface::TextBody,
    unit: MessageLimitUnit::Characters,
    limit: Some(4096),
    source: MessageConstraintSource::LocalDeliveryGuard,
    enforcement: MessageConstraintEnforcement::LocalPreflight,
}];

const WECHAT_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] = &[ProviderMessageConstraint {
    surface: MessageSurface::TextBody,
    unit: MessageLimitUnit::Characters,
    limit: Some(3800),
    source: MessageConstraintSource::LocalDeliveryGuard,
    enforcement: MessageConstraintEnforcement::LocalPreflight,
}];

const MICROSOFT_TEAMS_MESSAGE_CONSTRAINTS: &[ProviderMessageConstraint] =
    &[ProviderMessageConstraint {
        surface: MessageSurface::Payload,
        unit: MessageLimitUnit::Bytes,
        limit: Some(28 * 1024),
        source: MessageConstraintSource::ProviderDocumented,
        enforcement: MessageConstraintEnforcement::LocalPreflight,
    }];

const PROVIDER_DESCRIPTORS: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        provider_type: ProviderType::Slack,
        display_name: "Slack",
        setup_order: 10,
        capabilities: ProviderCapabilities {
            message_constraints: SLACK_MESSAGE_CONSTRAINTS,
            modes: SLACK_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Discord,
        display_name: "Discord",
        setup_order: 20,
        capabilities: ProviderCapabilities {
            message_constraints: DISCORD_MESSAGE_CONSTRAINTS,
            modes: DISCORD_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Telegram,
        display_name: "Telegram",
        setup_order: 30,
        capabilities: ProviderCapabilities {
            message_constraints: TELEGRAM_MESSAGE_CONSTRAINTS,
            modes: TELEGRAM_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::MicrosoftTeams,
        display_name: "Microsoft Teams",
        setup_order: 40,
        capabilities: ProviderCapabilities {
            message_constraints: MICROSOFT_TEAMS_MESSAGE_CONSTRAINTS,
            modes: MICROSOFT_TEAMS_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::EmailSmtp,
        display_name: "Email SMTP",
        setup_order: 50,
        capabilities: ProviderCapabilities {
            message_constraints: NO_MESSAGE_CONSTRAINTS,
            modes: EMAIL_SMTP_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Ntfy,
        display_name: "ntfy",
        setup_order: 60,
        capabilities: ProviderCapabilities {
            message_constraints: NTFY_MESSAGE_CONSTRAINTS,
            modes: NTFY_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Pushover,
        display_name: "Pushover",
        setup_order: 70,
        capabilities: ProviderCapabilities {
            message_constraints: PUSHOVER_MESSAGE_CONSTRAINTS,
            modes: PUSHOVER_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::FeishuLark,
        display_name: "Feishu/Lark custom bot",
        setup_order: 80,
        capabilities: ProviderCapabilities {
            message_constraints: NO_MESSAGE_CONSTRAINTS,
            modes: FEISHU_LARK_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Webhook,
        display_name: "Webhook",
        setup_order: 90,
        capabilities: ProviderCapabilities {
            message_constraints: NO_MESSAGE_CONSTRAINTS,
            modes: WEBHOOK_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Whatsapp,
        display_name: "WhatsApp",
        setup_order: 100,
        capabilities: ProviderCapabilities {
            message_constraints: WHATSAPP_MESSAGE_CONSTRAINTS,
            modes: WHATSAPP_PROVIDER_MODES,
        },
    },
    ProviderDescriptor {
        provider_type: ProviderType::Wechat,
        display_name: "WeChat",
        setup_order: 110,
        capabilities: ProviderCapabilities {
            message_constraints: WECHAT_MESSAGE_CONSTRAINTS,
            modes: WECHAT_PROVIDER_MODES,
        },
    },
];

pub fn all_provider_descriptors() -> &'static [ProviderDescriptor] {
    PROVIDER_DESCRIPTORS
}

pub fn provider_descriptor(provider_type: ProviderType) -> &'static ProviderDescriptor {
    PROVIDER_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.provider_type == provider_type)
        .expect("every ProviderType must have a ProviderDescriptor")
}

pub fn setup_provider_descriptors() -> impl Iterator<Item = &'static ProviderDescriptor> {
    let mut descriptors = PROVIDER_DESCRIPTORS.iter().collect::<Vec<_>>();
    descriptors.sort_by_key(|descriptor| descriptor.setup_order);
    descriptors.into_iter()
}

pub fn default_setup_provider_type() -> ProviderType {
    ProviderType::Slack
}

pub fn default_setup_provider_descriptor() -> &'static ProviderDescriptor {
    provider_descriptor(default_setup_provider_type())
}

pub fn provider_message_constraints(
    provider_type: ProviderType,
) -> &'static [ProviderMessageConstraint] {
    provider_descriptor(provider_type)
        .capabilities
        .message_constraints
}

pub fn provider_mode_capabilities(
    provider_type: ProviderType,
) -> &'static [ProviderModeCapability] {
    provider_descriptor(provider_type).capabilities.modes
}

pub fn provider_mode_capability(mode: ProviderMode) -> &'static ProviderModeCapability {
    PROVIDER_DESCRIPTORS
        .iter()
        .flat_map(|descriptor| descriptor.capabilities.modes.iter())
        .find(|capability| capability.mode == mode)
        .expect("every ProviderMode must have a ProviderModeCapability")
}

pub fn provider_config_mode(provider: &ProviderConfig) -> ProviderMode {
    match &provider.detail {
        ProviderConfigDetail::Ntfy(_) => ProviderMode::NtfyTopic,
        ProviderConfigDetail::Webhook(_) => ProviderMode::Webhook,
        ProviderConfigDetail::FeishuLark(detail) => match detail {
            FeishuLarkProviderConfig::CustomBot(_) => ProviderMode::FeishuLarkCustomBot,
            FeishuLarkProviderConfig::AppBot(_) => ProviderMode::FeishuLarkAppBot,
        },
        ProviderConfigDetail::Pushover(_) => ProviderMode::PushoverMessagesApi,
        ProviderConfigDetail::Slack(_) => ProviderMode::SlackIncomingWebhook,
        ProviderConfigDetail::Discord(_) => ProviderMode::DiscordWebhook,
        ProviderConfigDetail::Telegram(_) => ProviderMode::TelegramBotApi,
        ProviderConfigDetail::Whatsapp(_) => ProviderMode::WhatsappCloudApi,
        ProviderConfigDetail::Wechat(_) => ProviderMode::WechatIlink,
        ProviderConfigDetail::MicrosoftTeams(_) => ProviderMode::MicrosoftTeamsWorkflowWebhook,
        ProviderConfigDetail::EmailSmtp(_) => ProviderMode::EmailSmtp,
    }
}

pub fn provider_config_mode_capability(
    provider: &ProviderConfig,
) -> &'static ProviderModeCapability {
    provider_mode_capability(provider_config_mode(provider))
}

pub fn provider_message_constraint(
    provider_type: ProviderType,
    surface: MessageSurface,
) -> Option<&'static ProviderMessageConstraint> {
    provider_message_constraints(provider_type)
        .iter()
        .find(|constraint| constraint.surface == surface)
}

pub fn provider_local_preflight_message_constraint(
    provider_type: ProviderType,
    surface: MessageSurface,
) -> Option<&'static ProviderMessageConstraint> {
    provider_message_constraint(provider_type, surface)
        .filter(|constraint| constraint.enforcement == MessageConstraintEnforcement::LocalPreflight)
}

pub fn provider_local_preflight_message_limit(
    provider_type: ProviderType,
    surface: MessageSurface,
) -> usize {
    provider_local_preflight_message_constraint(provider_type, surface)
        .and_then(|constraint| constraint.limit)
        .expect("provider local preflight message limit must be cataloged for this surface")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        FeishuLarkAppBotProviderConfig, FeishuLarkAppDomain, FeishuLarkCustomBotProviderConfig,
        FeishuLarkProviderConfig, ProviderConfig, ProviderConfigDetail, SecretSource,
        SlackProviderConfig, UrlSource, WebhookProviderConfig,
    };

    #[test]
    fn every_provider_type_has_one_descriptor() {
        let provider_types = [
            ProviderType::Ntfy,
            ProviderType::Webhook,
            ProviderType::FeishuLark,
            ProviderType::Pushover,
            ProviderType::Slack,
            ProviderType::Discord,
            ProviderType::Telegram,
            ProviderType::Whatsapp,
            ProviderType::Wechat,
            ProviderType::MicrosoftTeams,
            ProviderType::EmailSmtp,
        ];

        for provider_type in provider_types {
            let matches = all_provider_descriptors()
                .iter()
                .filter(|descriptor| descriptor.provider_type == provider_type)
                .count();
            assert_eq!(
                matches,
                1,
                "{} should have one descriptor",
                provider_type.as_str()
            );
        }
        assert_eq!(all_provider_descriptors().len(), provider_types.len());
    }

    #[test]
    fn setup_provider_order_matches_existing_ui_order() {
        let provider_ids = setup_provider_descriptors()
            .map(|descriptor| descriptor.provider_type.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            provider_ids,
            vec![
                "slack",
                "discord",
                "telegram",
                "microsoft_teams",
                "email_smtp",
                "ntfy",
                "pushover",
                "feishu_lark",
                "webhook",
                "whatsapp",
                "wechat",
            ]
        );
    }

    #[test]
    fn setup_provider_order_is_driven_by_unique_setup_order() {
        let mut setup_orders = all_provider_descriptors()
            .iter()
            .map(|descriptor| descriptor.setup_order)
            .collect::<Vec<_>>();
        setup_orders.sort_unstable();
        setup_orders.dedup();
        assert_eq!(setup_orders.len(), all_provider_descriptors().len());

        let sorted_setup_orders = setup_provider_descriptors()
            .map(|descriptor| descriptor.setup_order)
            .collect::<Vec<_>>();
        assert!(sorted_setup_orders.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn default_setup_provider_is_cataloged() {
        assert_eq!(default_setup_provider_type(), ProviderType::Slack);
        assert_eq!(
            default_setup_provider_descriptor().provider_type,
            default_setup_provider_type()
        );
    }

    #[test]
    fn message_constraints_are_canonical() {
        assert_eq!(
            provider_message_constraints(ProviderType::Ntfy),
            &[ProviderMessageConstraint {
                surface: MessageSurface::MessageBody,
                unit: MessageLimitUnit::Characters,
                limit: Some(4096),
                source: MessageConstraintSource::ProviderDocumentedDefault,
                enforcement: MessageConstraintEnforcement::PolicyOnly,
            }]
        );
        assert_eq!(
            provider_message_constraints(ProviderType::Pushover),
            &[
                ProviderMessageConstraint {
                    surface: MessageSurface::Title,
                    unit: MessageLimitUnit::Characters,
                    limit: Some(250),
                    source: MessageConstraintSource::ProviderDocumented,
                    enforcement: MessageConstraintEnforcement::LocalPreflight,
                },
                ProviderMessageConstraint {
                    surface: MessageSurface::MessageBody,
                    unit: MessageLimitUnit::Characters,
                    limit: Some(1024),
                    source: MessageConstraintSource::ProviderDocumented,
                    enforcement: MessageConstraintEnforcement::LocalPreflight,
                },
            ]
        );
        let teams_payload_constraint =
            provider_message_constraint(ProviderType::MicrosoftTeams, MessageSurface::Payload)
                .expect("Teams payload constraint should be cataloged");
        assert_eq!(teams_payload_constraint.limit, Some(28 * 1024));
        assert_eq!(
            provider_local_preflight_message_limit(
                ProviderType::MicrosoftTeams,
                MessageSurface::Payload
            ),
            28 * 1024
        );
        assert!(
            provider_local_preflight_message_constraint(
                ProviderType::Ntfy,
                MessageSurface::MessageBody
            )
            .is_none()
        );
        assert!(provider_message_constraints(ProviderType::Webhook).is_empty());
        assert!(provider_message_constraints(ProviderType::FeishuLark).is_empty());
        assert!(provider_message_constraints(ProviderType::EmailSmtp).is_empty());
    }

    #[test]
    fn response_surface_provider_modes_are_cataloged() {
        assert_eq!(
            provider_mode_capabilities(ProviderType::Slack)
                .iter()
                .map(|capability| capability.mode)
                .collect::<Vec<_>>(),
            vec![ProviderMode::SlackIncomingWebhook, ProviderMode::SlackApp]
        );
        assert_eq!(
            provider_mode_capabilities(ProviderType::FeishuLark)
                .iter()
                .map(|capability| capability.mode)
                .collect::<Vec<_>>(),
            vec![
                ProviderMode::FeishuLarkCustomBot,
                ProviderMode::FeishuLarkAppBot
            ]
        );

        let slack_app = provider_mode_capability(ProviderMode::SlackApp);
        assert_eq!(slack_app.provider_type, ProviderType::Slack);
        assert_eq!(
            slack_app.inbound_reply.mode,
            InboundReplyMode::LocalConnection
        );
        assert_eq!(
            slack_app.inbound_reply.local_connection_kind,
            Some(LocalConnectionKind::SocketMode)
        );
        assert!(!slack_app.inbound_reply.requires_public_endpoint);
        assert_eq!(
            slack_app.inbound_reply.stable_event_id,
            StableEventIdCapability::Available
        );
        assert!(
            slack_app
                .inbound_reply
                .reply_surfaces
                .contains(&ProviderReplySurface::ThreadRootReply)
        );
        assert_eq!(
            slack_app.delivery_receipt.fields,
            RESPONSE_SURFACE_DELIVERY_RECEIPT_FIELDS
        );

        let feishu_lark_app = provider_mode_capability(ProviderMode::FeishuLarkAppBot);
        assert_eq!(feishu_lark_app.provider_type, ProviderType::FeishuLark);
        assert_eq!(
            feishu_lark_app.inbound_reply.mode,
            InboundReplyMode::LocalConnection
        );
        assert_eq!(
            feishu_lark_app.inbound_reply.local_connection_kind,
            Some(LocalConnectionKind::LongConnection)
        );
        assert!(!feishu_lark_app.inbound_reply.requires_public_endpoint);
        assert_eq!(
            feishu_lark_app.inbound_reply.stable_event_id,
            StableEventIdCapability::Available
        );
        assert!(
            feishu_lark_app
                .inbound_reply
                .reply_surfaces
                .contains(&ProviderReplySurface::ThreadRootReply)
        );
        assert_eq!(
            feishu_lark_app.delivery_receipt.fields,
            FEISHU_LARK_CANDIDATE_DELIVERY_RECEIPT_FIELDS
        );
        assert!(
            !feishu_lark_app
                .delivery_receipt
                .has_field(DeliveryReceiptField::ProviderThreadId)
        );
    }

    #[test]
    fn existing_provider_configs_map_to_outbound_only_modes() {
        let slack = ProviderConfig {
            id: "slack".to_string(),
            detail: ProviderConfigDetail::Slack(SlackProviderConfig {
                url: UrlSource::Inline("https://hooks.slack.com/services/T/B/C".to_string()),
            }),
        };
        let feishu_lark = ProviderConfig {
            id: "feishu_lark".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::CustomBot(
                FeishuLarkCustomBotProviderConfig {
                    url: UrlSource::Inline(
                        "https://open.feishu.cn/open-apis/bot/v2/hook/test".to_string(),
                    ),
                    secret: None,
                },
            )),
        };
        let webhook = ProviderConfig {
            id: "webhook".to_string(),
            detail: ProviderConfigDetail::Webhook(WebhookProviderConfig {
                url: UrlSource::Inline("https://example.com/hook".to_string()),
            }),
        };

        assert_eq!(
            provider_config_mode(&slack),
            ProviderMode::SlackIncomingWebhook
        );
        assert_eq!(
            provider_config_mode(&feishu_lark),
            ProviderMode::FeishuLarkCustomBot
        );
        assert_eq!(provider_config_mode(&webhook), ProviderMode::Webhook);

        for provider in [&slack, &feishu_lark, &webhook] {
            assert_eq!(
                provider_config_mode_capability(provider).inbound_reply.mode,
                InboundReplyMode::None
            );
        }
    }

    #[test]
    fn explicit_feishu_lark_app_bot_config_maps_to_app_bot_mode() {
        let provider = ProviderConfig {
            id: "lark_app".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(
                FeishuLarkAppBotProviderConfig {
                    domain: FeishuLarkAppDomain::Lark,
                    app_id: "cli_9f5343c580712544".to_string(),
                    app_secret: SecretSource::Env("AGENTS_ROUTER_LARK_APP_SECRET".to_string()),
                    tenant_key: "2ca1d211f64f6438".to_string(),
                    chat_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                },
            )),
        };

        assert_eq!(
            provider_config_mode(&provider),
            ProviderMode::FeishuLarkAppBot
        );
        assert_eq!(
            provider_config_mode_capability(&provider)
                .inbound_reply
                .mode,
            InboundReplyMode::LocalConnection
        );
    }

    #[test]
    fn webhook_only_modes_do_not_expose_reply_capability() {
        for mode in [
            ProviderMode::SlackIncomingWebhook,
            ProviderMode::FeishuLarkCustomBot,
            ProviderMode::Webhook,
        ] {
            let capability = provider_mode_capability(mode);
            assert_eq!(capability.inbound_reply.mode, InboundReplyMode::None);
            assert_eq!(capability.inbound_reply.local_connection_kind, None);
            assert_eq!(
                capability.inbound_reply.stable_event_id,
                StableEventIdCapability::Unavailable
            );
            assert!(capability.inbound_reply.reply_surfaces.is_empty());
            assert!(capability.delivery_receipt.fields.is_empty());
        }
    }

    #[test]
    fn user_docs_include_cataloged_message_constraint_limits() {
        for descriptor in all_provider_descriptors() {
            let provider_docs = provider_user_docs(descriptor.provider_type);
            for constraint in descriptor.capabilities.message_constraints {
                let Some(limit) = constraint.limit else {
                    continue;
                };
                let expected = docs_limit_text(*constraint, limit);
                assert!(
                    provider_docs.contains(&expected),
                    "provider docs should include cataloged limit `{expected}` for `{}`",
                    descriptor.provider_type.as_str()
                );
            }
        }
    }

    #[test]
    fn setup_docs_include_policy_limit_for_each_restricted_provider() {
        let setup_docs = include_str!("../docs/setup.md");

        for (provider_type, surface) in [
            (ProviderType::Ntfy, MessageSurface::MessageBody),
            (ProviderType::Pushover, MessageSurface::MessageBody),
            (ProviderType::Slack, MessageSurface::TextBody),
            (ProviderType::Discord, MessageSurface::WebhookContent),
            (ProviderType::Telegram, MessageSurface::TextBody),
            (ProviderType::Whatsapp, MessageSurface::TextBody),
            (ProviderType::Wechat, MessageSurface::TextBody),
            (ProviderType::MicrosoftTeams, MessageSurface::Payload),
        ] {
            let descriptor = provider_descriptor(provider_type);
            let constraint = provider_message_constraint(provider_type, surface)
                .expect("restricted provider policy limit must be cataloged");
            let limit = constraint
                .limit
                .expect("restricted provider policy limit must have a numeric value");
            let expected = docs_limit_text(*constraint, limit);
            let provider_line = setup_docs
                .lines()
                .find(|line| line.starts_with(&format!("- {},", descriptor.display_name)))
                .unwrap_or_else(|| {
                    panic!(
                        "setup docs should include a restricted provider line for `{}`",
                        descriptor.provider_type.as_str()
                    )
                });

            assert!(
                provider_line.contains(&expected),
                "setup docs line `{provider_line}` should include cataloged limit `{expected}`"
            );
        }
    }

    fn provider_user_docs(provider_type: ProviderType) -> String {
        match provider_type {
            ProviderType::Ntfy => [
                include_str!("../docs/providers/ntfy.md"),
                include_str!("../docs/providers/ntfy.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Pushover => [
                include_str!("../docs/providers/pushover.md"),
                include_str!("../docs/providers/pushover.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Slack => [
                include_str!("../docs/providers/slack.md"),
                include_str!("../docs/providers/slack.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Discord => [
                include_str!("../docs/providers/discord.md"),
                include_str!("../docs/providers/discord.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Telegram => [
                include_str!("../docs/providers/telegram.md"),
                include_str!("../docs/providers/telegram.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Whatsapp => [
                include_str!("../docs/providers/whatsapp.md"),
                include_str!("../docs/providers/whatsapp.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Wechat => [
                include_str!("../docs/providers/wechat.md"),
                include_str!("../docs/providers/wechat.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::MicrosoftTeams => [
                include_str!("../docs/providers/microsoft-teams.md"),
                include_str!("../docs/providers/microsoft-teams.zh-CN.md"),
            ]
            .join("\n"),
            ProviderType::Webhook | ProviderType::FeishuLark | ProviderType::EmailSmtp => {
                String::new()
            }
        }
    }

    fn docs_limit_text(constraint: ProviderMessageConstraint, limit: usize) -> String {
        if constraint.unit == MessageLimitUnit::Bytes && limit.is_multiple_of(1024) {
            return format!("{} KB", limit / 1024);
        }

        limit.to_string()
    }
}
