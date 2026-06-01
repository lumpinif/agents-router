use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::info;

use crate::bridge_binding_ledger::{ThreadBindingQuery, ThreadSessionBindingRecord};
use crate::provider_catalog::{
    InboundReplyMode, ProviderMode, ProviderModeCapability, RESPONSE_SURFACE_REPLY_SURFACES,
    StableEventIdCapability,
};
use crate::response_surface_ledger::{
    InboundEventClaimDecision, InboundEventDedupInput, ResponseSurfaceLedger,
    ResponseSurfaceLookupQuery, ResponseSurfaceLookupRecord, ResponseSurfaceLookupResult,
    ResponseSurfaceStatus,
};

const SLACK_SOCKET_MODE_EVENTS_API_TYPE: &str = "events_api";
const SLACK_EVENT_CALLBACK_TYPE: &str = "event_callback";
const SLACK_MESSAGE_EVENT_TYPE: &str = "message";
const FEISHU_LARK_EVENT_SCHEMA: &str = "2.0";
const FEISHU_LARK_MESSAGE_RECEIVE_EVENT_TYPE: &str = "im.message.receive_v1";
const FEISHU_LARK_BOT_ADDED_EVENT_TYPE: &str = "im.chat.member.bot.added_v1";
const FEISHU_LARK_TEXT_MESSAGE_TYPE: &str = "text";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedProviderSurfaceReply {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_mode: ProviderMode,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub provider_event_id: String,
    pub provider_reply_message_id: Option<String>,
    pub reply_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderInboundNormalizeResult {
    SurfaceReply(NormalizedProviderSurfaceReply),
    Skip(ProviderInboundSkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedProviderControlCommand {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_mode: ProviderMode,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub provider_event_id: String,
    pub command: ProviderControlCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedProviderRoomEvent {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_mode: ProviderMode,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_event_id: String,
    pub event: ProviderRoomEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderControlCommand {
    BindProject {
        project_path: String,
    },
    DirectBindProject {
        project_path: String,
    },
    DirectChatGuidance,
    DirectHelp,
    DirectNewSession {
        project_path: Option<String>,
        prompt: String,
    },
    DirectStatus,
    DirectUnbindProject,
    Help,
    NewSession {
        project_path: Option<String>,
        prompt: String,
    },
    RoomGuidance,
    Status,
    UnbindProject {
        project_path: Option<String>,
    },
    Invalid {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRoomEvent {
    BotAddedToChat,
}

impl ProviderControlCommand {
    fn requires_project_room(&self) -> bool {
        matches!(
            self,
            Self::BindProject { .. }
                | Self::NewSession { .. }
                | Self::Status
                | Self::UnbindProject { .. }
        )
    }

    fn requires_room_root(&self) -> bool {
        matches!(
            self,
            Self::BindProject { .. }
                | Self::DirectBindProject { .. }
                | Self::DirectNewSession { .. }
                | Self::NewSession { .. }
                | Self::DirectUnbindProject
                | Self::UnbindProject { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderControlNormalizeResult {
    ControlCommand(Box<NormalizedProviderControlCommand>),
    RoomEvent(Box<NormalizedProviderRoomEvent>),
    Skip(ProviderControlSkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderControlSkipReason {
    UnsupportedEnvelopeType,
    UnsupportedEventType,
    NonUserMessage,
    NotAddressedToBot,
    UnsupportedMessageType,
    EmptyMessageText,
    NotControlCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInboundReady {
    pub reply: NormalizedProviderSurfaceReply,
    pub surface: ResponseSurfaceLookupRecord,
    pub provider_event_id_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderInboundDecision {
    Ready(Box<ProviderInboundReady>),
    Skip(ProviderInboundSkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderInboundSkipReason {
    UnsupportedProviderMode(ProviderMode),
    UnsupportedEnvelopeType,
    UnsupportedEventType,
    IgnoredMessageSubtype,
    NonUserMessage,
    NotAddressedToBot,
    NotSurfaceReply,
    UnsupportedMessageType,
    EmptyReplyText,
    SurfaceLookupMiss,
    SurfaceClosed { surface_id: String },
    DuplicateEvent { surface_id: String },
    EventAlreadyProcessing { surface_id: String },
}

pub fn normalize_slack_socket_mode_surface_reply(
    provider_id: &str,
    raw_event: &[u8],
) -> anyhow::Result<ProviderInboundNormalizeResult> {
    ensure_present("provider_id", provider_id)?;
    let envelope: SlackSocketModeEnvelope =
        serde_json::from_slice(raw_event).context("failed to parse Slack Socket Mode event")?;

    if optional_field(envelope.kind.as_deref()) != Some(SLACK_SOCKET_MODE_EVENTS_API_TYPE) {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::UnsupportedEnvelopeType,
        ));
    }

    required_trimmed("slack envelope_id", envelope.envelope_id.as_deref())?;
    let payload = envelope
        .payload
        .context("Slack Socket Mode event is missing payload")?;
    if optional_field(payload.kind.as_deref()) != Some(SLACK_EVENT_CALLBACK_TYPE) {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::UnsupportedEventType,
        ));
    }

    let event_id = required_owned("slack payload.event_id", payload.event_id.as_deref())?;
    let team_id = required_owned("slack payload.team_id", payload.team_id.as_deref())?;
    let event = payload
        .event
        .context("Slack event callback payload is missing event")?;

    if optional_field(event.kind.as_deref()) != Some(SLACK_MESSAGE_EVENT_TYPE) {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::UnsupportedEventType,
        ));
    }

    if event
        .subtype
        .as_deref()
        .and_then(optional_trimmed)
        .is_some()
    {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::IgnoredMessageSubtype,
        ));
    }

    if event.bot_id.as_deref().and_then(optional_trimmed).is_some()
        || event.user.as_deref().and_then(optional_trimmed).is_none()
    {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::NonUserMessage,
        ));
    }

    let reply_message_id = required_owned("slack event.ts", event.ts.as_deref())?;
    let thread_id = match optional_field(event.thread_ts.as_deref()) {
        Some(thread_id) if thread_id != reply_message_id => thread_id.to_string(),
        _ => {
            return Ok(ProviderInboundNormalizeResult::Skip(
                ProviderInboundSkipReason::NotSurfaceReply,
            ));
        }
    };
    let Some(reply_text) = normalized_reply_text(event.text.as_deref()) else {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::EmptyReplyText,
        ));
    };
    let channel_id = required_owned("slack event.channel", event.channel.as_deref())?;

    Ok(ProviderInboundNormalizeResult::SurfaceReply(
        NormalizedProviderSurfaceReply {
            provider_id: provider_id.to_string(),
            provider_type: "slack".to_string(),
            provider_mode: ProviderMode::SlackApp,
            provider_account_id: team_id,
            provider_conversation_id: channel_id,
            provider_thread_id: thread_id,
            provider_event_id: event_id,
            provider_reply_message_id: Some(reply_message_id),
            reply_text,
        },
    ))
}

pub fn normalize_feishu_lark_long_connection_surface_reply(
    provider_id: &str,
    bot_open_id: &str,
    raw_event: &[u8],
) -> anyhow::Result<ProviderInboundNormalizeResult> {
    ensure_present("provider_id", provider_id)?;
    ensure_present("bot_open_id", bot_open_id)?;
    let envelope: FeishuLarkEventEnvelope =
        serde_json::from_slice(raw_event).context("failed to parse Feishu/Lark event")?;

    if optional_field(envelope.schema.as_deref()) != Some(FEISHU_LARK_EVENT_SCHEMA) {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::UnsupportedEnvelopeType,
        ));
    }

    let header = envelope
        .header
        .context("Feishu/Lark event is missing header")?;
    if optional_field(header.event_type.as_deref()) != Some(FEISHU_LARK_MESSAGE_RECEIVE_EVENT_TYPE)
    {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::UnsupportedEventType,
        ));
    }

    required_owned("feishu_lark header.event_id", header.event_id.as_deref())?;
    let tenant_key = required_owned(
        "feishu_lark header.tenant_key",
        header.tenant_key.as_deref(),
    )?;
    let event = envelope
        .event
        .context("Feishu/Lark message event is missing event")?;
    let sender = event
        .sender
        .context("Feishu/Lark message event is missing sender")?;
    if optional_field(sender.sender_type.as_deref()) != Some("user") {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::NonUserMessage,
        ));
    }

    let message = event
        .message
        .context("Feishu/Lark message event is missing message")?;
    if optional_field(message.message_type.as_deref()) != Some(FEISHU_LARK_TEXT_MESSAGE_TYPE) {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::UnsupportedMessageType,
        ));
    }

    let reply_message_id = required_owned(
        "feishu_lark message.message_id",
        message.message_id.as_deref(),
    )?;
    let root_id = match optional_field(message.root_id.as_deref()) {
        Some(root_id) if root_id != reply_message_id => root_id.to_string(),
        _ => {
            return Ok(ProviderInboundNormalizeResult::Skip(
                ProviderInboundSkipReason::NotSurfaceReply,
            ));
        }
    };
    let chat_id = required_owned("feishu_lark message.chat_id", message.chat_id.as_deref())?;
    let content = required_trimmed("feishu_lark message.content", message.content.as_deref())?;
    let text_content: FeishuLarkTextContent = serde_json::from_str(content)
        .context("failed to parse Feishu/Lark text message content")?;
    let Some(raw_reply_text) = normalized_reply_text(text_content.text.as_deref()) else {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::EmptyReplyText,
        ));
    };
    let Some(reply_text) = addressed_feishu_lark_text(&message, &raw_reply_text, bot_open_id)
    else {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::NotAddressedToBot,
        ));
    };
    if reply_text.is_empty() {
        return Ok(ProviderInboundNormalizeResult::Skip(
            ProviderInboundSkipReason::EmptyReplyText,
        ));
    }

    Ok(ProviderInboundNormalizeResult::SurfaceReply(
        NormalizedProviderSurfaceReply {
            provider_id: provider_id.to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: tenant_key,
            provider_conversation_id: chat_id,
            provider_thread_id: root_id,
            provider_event_id: reply_message_id.clone(),
            provider_reply_message_id: Some(reply_message_id),
            reply_text,
        },
    ))
}

pub fn normalize_feishu_lark_long_connection_control_command(
    provider_id: &str,
    bot_open_id: &str,
    raw_event: &[u8],
) -> anyhow::Result<ProviderControlNormalizeResult> {
    ensure_present("provider_id", provider_id)?;
    ensure_present("bot_open_id", bot_open_id)?;
    let envelope: FeishuLarkEventEnvelope =
        serde_json::from_slice(raw_event).context("failed to parse Feishu/Lark event")?;

    if optional_field(envelope.schema.as_deref()) != Some(FEISHU_LARK_EVENT_SCHEMA) {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::UnsupportedEnvelopeType,
        ));
    }

    let header = envelope
        .header
        .context("Feishu/Lark event is missing header")?;
    let event_type = optional_field(header.event_type.as_deref());
    if event_type == Some(FEISHU_LARK_BOT_ADDED_EVENT_TYPE) {
        let tenant_key = required_owned(
            "feishu_lark header.tenant_key",
            header.tenant_key.as_deref(),
        )?;
        let event_id = required_owned("feishu_lark header.event_id", header.event_id.as_deref())?;
        let event = envelope
            .event
            .context("Feishu/Lark bot added event is missing event")?;
        let chat_id = required_owned("feishu_lark event.chat_id", event.chat_id.as_deref())?;
        return Ok(ProviderControlNormalizeResult::RoomEvent(Box::new(
            NormalizedProviderRoomEvent {
                provider_id: provider_id.to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: tenant_key,
                provider_conversation_id: chat_id,
                provider_event_id: event_id,
                event: ProviderRoomEvent::BotAddedToChat,
            },
        )));
    }
    if event_type != Some(FEISHU_LARK_MESSAGE_RECEIVE_EVENT_TYPE) {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::UnsupportedEventType,
        ));
    }

    let tenant_key = required_owned(
        "feishu_lark header.tenant_key",
        header.tenant_key.as_deref(),
    )?;
    let event = envelope
        .event
        .context("Feishu/Lark message event is missing event")?;
    let sender = event
        .sender
        .context("Feishu/Lark message event is missing sender")?;
    if optional_field(sender.sender_type.as_deref()) != Some("user") {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::NonUserMessage,
        ));
    }

    let message = event
        .message
        .context("Feishu/Lark message event is missing message")?;
    if optional_field(message.message_type.as_deref()) != Some(FEISHU_LARK_TEXT_MESSAGE_TYPE) {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::UnsupportedMessageType,
        ));
    }

    let message_id = required_owned(
        "feishu_lark message.message_id",
        message.message_id.as_deref(),
    )?;
    let chat_id = required_owned("feishu_lark message.chat_id", message.chat_id.as_deref())?;
    let is_direct_chat = feishu_lark_message_is_direct_chat(&message);
    let is_thread_reply = optional_field(message.root_id.as_deref())
        .is_some_and(|root_id| root_id != message_id.as_str());
    let content = required_trimmed("feishu_lark message.content", message.content.as_deref())?;
    let text_content: FeishuLarkTextContent = serde_json::from_str(content)
        .context("failed to parse Feishu/Lark text message content")?;
    let Some(raw_text) = normalized_reply_text(text_content.text.as_deref()) else {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::EmptyMessageText,
        ));
    };
    let Some(text) = addressed_feishu_lark_text(&message, &raw_text, bot_open_id) else {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::NotAddressedToBot,
        ));
    };
    if text.is_empty() {
        return Ok(ProviderControlNormalizeResult::Skip(
            ProviderControlSkipReason::EmptyMessageText,
        ));
    }
    let command = match parse_provider_control_command(&text) {
        Some(command) => command,
        None if is_direct_chat && !is_thread_reply => ProviderControlCommand::DirectNewSession {
            project_path: None,
            prompt: text,
        },
        None if !is_direct_chat && !is_thread_reply => ProviderControlCommand::RoomGuidance,
        None => {
            return Ok(ProviderControlNormalizeResult::Skip(
                ProviderControlSkipReason::NotControlCommand,
            ));
        }
    };

    let root_id = optional_field(message.root_id.as_deref());
    let provider_thread_id = root_id.unwrap_or(message_id.as_str()).to_string();
    let command = if root_id.is_some() && command.requires_room_root() {
        ProviderControlCommand::Invalid {
            message: "That command only works in the room itself, not inside a Lark thread.\n\nGo back to the room, mention me, and send the command there. Thread replies are only for continuing the Codex task that posted the update.".to_string(),
        }
    } else if is_direct_chat {
        match command {
            ProviderControlCommand::BindProject { project_path } => {
                ProviderControlCommand::DirectBindProject { project_path }
            }
            ProviderControlCommand::Help => ProviderControlCommand::DirectHelp,
            ProviderControlCommand::Status => ProviderControlCommand::DirectStatus,
            ProviderControlCommand::NewSession {
                project_path,
                prompt,
            } => ProviderControlCommand::DirectNewSession {
                project_path,
                prompt,
            },
            ProviderControlCommand::UnbindProject { project_path: None } => {
                ProviderControlCommand::DirectUnbindProject
            }
            ProviderControlCommand::UnbindProject {
                project_path: Some(_),
            } => ProviderControlCommand::Invalid {
                message: "Use `/unbind` in direct chat to clear the default project.".to_string(),
            },
            command if command.requires_project_room() => ProviderControlCommand::Invalid {
                message: "That command is for group rooms. In direct chat, use `/bind /path/to/project`, `/new ...`, `/status`, or `/help`.".to_string(),
            },
            command => command,
        }
    } else {
        command
    };

    Ok(ProviderControlNormalizeResult::ControlCommand(Box::new(
        NormalizedProviderControlCommand {
            provider_id: provider_id.to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: tenant_key,
            provider_conversation_id: chat_id,
            provider_thread_id,
            provider_event_id: message_id,
            command,
        },
    )))
}

pub fn lookup_and_claim_provider_surface_reply(
    ledger: &mut ResponseSurfaceLedger,
    provider: &ProviderModeCapability,
    reply: NormalizedProviderSurfaceReply,
    now: DateTime<Utc>,
) -> anyhow::Result<ProviderInboundDecision> {
    ensure!(
        provider.mode == reply.provider_mode,
        "provider inbound mode `{}` does not match normalized reply mode `{}`",
        provider.mode.as_str(),
        reply.provider_mode.as_str()
    );
    ensure!(
        provider.provider_type.as_str() == reply.provider_type,
        "provider inbound type `{}` does not match normalized reply type `{}`",
        provider.provider_type.as_str(),
        reply.provider_type
    );

    if !provider_mode_accepts_local_surface_replies(provider) {
        return Ok(ProviderInboundDecision::Skip(
            ProviderInboundSkipReason::UnsupportedProviderMode(provider.mode),
        ));
    }

    let lookup = ledger.lookup_surface_at(
        ResponseSurfaceLookupQuery {
            provider_id: reply.provider_id.clone(),
            provider_account_id: reply.provider_account_id.clone(),
            provider_conversation_id: reply.provider_conversation_id.clone(),
            provider_thread_id: reply.provider_thread_id.clone(),
        },
        now,
    )?;

    let surface = match lookup {
        ResponseSurfaceLookupResult::Hit(surface) => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                provider.conversation.id = %reply.provider_conversation_id,
                provider.thread.id = %reply.provider_thread_id,
                surface.id = %surface.surface_id,
                event = "provider_inbound.surface_lookup.hit",
            );
            surface
        }
        ResponseSurfaceLookupResult::Miss => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                provider.conversation.id = %reply.provider_conversation_id,
                provider.thread.id = %reply.provider_thread_id,
                event = "provider_inbound.surface_lookup.miss",
            );
            return Ok(ProviderInboundDecision::Skip(
                ProviderInboundSkipReason::SurfaceLookupMiss,
            ));
        }
        ResponseSurfaceLookupResult::Closed { surface_id } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                provider.conversation.id = %reply.provider_conversation_id,
                provider.thread.id = %reply.provider_thread_id,
                surface.id = %surface_id,
                event = "provider_inbound.surface_lookup.closed",
            );
            return Ok(ProviderInboundDecision::Skip(
                ProviderInboundSkipReason::SurfaceClosed { surface_id },
            ));
        }
    };

    // Dedup is based only on provider identity plus the provider's stable
    // event id. Reply text is user content and must not decide duplicates.
    let claim = ledger.claim_inbound_event_at(
        InboundEventDedupInput {
            provider_id: reply.provider_id.clone(),
            provider_type: reply.provider_type.clone(),
            provider_account_id: reply.provider_account_id.clone(),
            provider_conversation_id: reply.provider_conversation_id.clone(),
            provider_event_id: reply.provider_event_id.clone(),
            surface_id: surface.surface_id.clone(),
        },
        now,
    )?;

    match claim {
        InboundEventClaimDecision::Claimed {
            provider_event_id_hash,
        } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                surface.id = %surface.surface_id,
                event.hash = %provider_event_id_hash,
                event = "provider_inbound.event_claim.succeeded",
            );
            Ok(ProviderInboundDecision::Ready(Box::new(
                ProviderInboundReady {
                    reply,
                    surface: *surface,
                    provider_event_id_hash,
                },
            )))
        }
        InboundEventClaimDecision::AlreadyProcessing {
            surface_id,
            provider_event_id_hash,
            status,
        } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                surface.id = %surface_id,
                event.hash = %provider_event_id_hash,
                inbound.status = %status.as_str(),
                event = "provider_inbound.event_claim.already_processing",
            );
            Ok(ProviderInboundDecision::Skip(
                ProviderInboundSkipReason::EventAlreadyProcessing { surface_id },
            ))
        }
        InboundEventClaimDecision::DuplicateProcessed {
            surface_id,
            provider_event_id_hash,
            status,
        } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                surface.id = %surface_id,
                event.hash = %provider_event_id_hash,
                inbound.status = %status.as_str(),
                event = "provider_inbound.event_claim.duplicate_processed",
            );
            Ok(ProviderInboundDecision::Skip(
                ProviderInboundSkipReason::DuplicateEvent { surface_id },
            ))
        }
    }
}

pub fn lookup_and_claim_provider_thread_session_reply(
    ledger: &mut ResponseSurfaceLedger,
    provider: &ProviderModeCapability,
    binding: ThreadSessionBindingRecord,
    reply: NormalizedProviderSurfaceReply,
    now: DateTime<Utc>,
) -> anyhow::Result<ProviderInboundDecision> {
    ensure!(
        provider.mode == reply.provider_mode,
        "provider inbound mode `{}` does not match normalized reply mode `{}`",
        provider.mode.as_str(),
        reply.provider_mode.as_str()
    );
    ensure!(
        provider.provider_type.as_str() == reply.provider_type,
        "provider inbound type `{}` does not match normalized reply type `{}`",
        provider.provider_type.as_str(),
        reply.provider_type
    );
    ensure!(
        binding.provider_id == reply.provider_id
            && binding.provider_type == reply.provider_type
            && binding.provider_account_id == reply.provider_account_id
            && binding.provider_conversation_id == reply.provider_conversation_id
            && binding.provider_thread_id == reply.provider_thread_id,
        "provider thread binding does not match normalized reply"
    );

    if !provider_mode_accepts_local_surface_replies(provider) {
        return Ok(ProviderInboundDecision::Skip(
            ProviderInboundSkipReason::UnsupportedProviderMode(provider.mode),
        ));
    }

    let surface_id = bridge_thread_surface_id(&binding);
    let claim = ledger.claim_inbound_event_at(
        InboundEventDedupInput {
            provider_id: reply.provider_id.clone(),
            provider_type: reply.provider_type.clone(),
            provider_account_id: reply.provider_account_id.clone(),
            provider_conversation_id: reply.provider_conversation_id.clone(),
            provider_event_id: reply.provider_event_id.clone(),
            surface_id: surface_id.clone(),
        },
        now,
    )?;

    match claim {
        InboundEventClaimDecision::Claimed {
            provider_event_id_hash,
        } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                provider.conversation.id = %reply.provider_conversation_id,
                provider.thread.id = %reply.provider_thread_id,
                surface.id = %surface_id,
                source.id = %binding.source_id,
                source.type = %binding.source_type,
                source.session.id = %binding.source_session_id,
                event.hash = %provider_event_id_hash,
                event = "provider_inbound.thread_binding.claim.succeeded",
            );
            Ok(ProviderInboundDecision::Ready(Box::new(
                ProviderInboundReady {
                    reply,
                    surface: ResponseSurfaceLookupRecord {
                        surface_id,
                        signal_id: "bridge-thread-binding".to_string(),
                        delivery_id: "bridge-thread-binding".to_string(),
                        source_id: binding.source_id,
                        source_type: binding.source_type,
                        source_session_id: binding.source_session_id,
                        source_turn_id: None,
                        provider_id: binding.provider_id,
                        provider_type: binding.provider_type,
                        provider_mode: provider.mode,
                        provider_account_id: binding.provider_account_id,
                        provider_conversation_id: binding.provider_conversation_id,
                        provider_message_id: binding.provider_thread_id.clone(),
                        provider_thread_id: binding.provider_thread_id,
                        route_binding_hash: None,
                        status: ResponseSurfaceStatus::Open,
                    },
                    provider_event_id_hash,
                },
            )))
        }
        InboundEventClaimDecision::AlreadyProcessing {
            surface_id,
            provider_event_id_hash,
            status,
        } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                surface.id = %surface_id,
                event.hash = %provider_event_id_hash,
                inbound.status = %status.as_str(),
                event = "provider_inbound.thread_binding.claim.already_processing",
            );
            Ok(ProviderInboundDecision::Skip(
                ProviderInboundSkipReason::EventAlreadyProcessing { surface_id },
            ))
        }
        InboundEventClaimDecision::DuplicateProcessed {
            surface_id,
            provider_event_id_hash,
            status,
        } => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.mode = %reply.provider_mode.as_str(),
                surface.id = %surface_id,
                event.hash = %provider_event_id_hash,
                inbound.status = %status.as_str(),
                event = "provider_inbound.thread_binding.claim.duplicate_processed",
            );
            Ok(ProviderInboundDecision::Skip(
                ProviderInboundSkipReason::DuplicateEvent { surface_id },
            ))
        }
    }
}

fn provider_mode_accepts_local_surface_replies(provider: &ProviderModeCapability) -> bool {
    provider.inbound_reply.mode == InboundReplyMode::LocalConnection
        && !provider.inbound_reply.requires_public_endpoint
        && provider.inbound_reply.stable_event_id == StableEventIdCapability::Available
        && provider
            .inbound_reply
            .reply_surfaces
            .iter()
            .any(|surface| RESPONSE_SURFACE_REPLY_SURFACES.contains(surface))
}

pub fn thread_binding_query_for_reply(
    reply: &NormalizedProviderSurfaceReply,
) -> ThreadBindingQuery {
    ThreadBindingQuery {
        provider_id: reply.provider_id.clone(),
        provider_type: reply.provider_type.clone(),
        provider_account_id: reply.provider_account_id.clone(),
        provider_conversation_id: reply.provider_conversation_id.clone(),
        provider_thread_id: reply.provider_thread_id.clone(),
    }
}

pub(crate) fn provider_event_id_stable_hash(provider_event_id: &str) -> String {
    stable_hash(provider_event_id)
}

fn bridge_thread_surface_id(binding: &ThreadSessionBindingRecord) -> String {
    format!(
        "bridge-thread-{}",
        stable_hash(&format!(
            "v=1\nprovider.id={}\nprovider.type={}\nprovider.account.id={}\nprovider.conversation.id={}\nprovider.thread.id={}\nsource.id={}\nsource.type={}\nsource.session.id={}\n",
            binding.provider_id,
            binding.provider_type,
            binding.provider_account_id,
            binding.provider_conversation_id,
            binding.provider_thread_id,
            binding.source_id,
            binding.source_type,
            binding.source_session_id,
        ))
    )
}

fn stable_hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Deserialize)]
struct SlackSocketModeEnvelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    envelope_id: Option<String>,
    payload: Option<SlackEventCallbackPayload>,
}

#[derive(Debug, Deserialize)]
struct SlackEventCallbackPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    team_id: Option<String>,
    event_id: Option<String>,
    event: Option<SlackMessageEvent>,
}

#[derive(Debug, Deserialize)]
struct SlackMessageEvent {
    #[serde(rename = "type")]
    kind: Option<String>,
    subtype: Option<String>,
    channel: Option<String>,
    user: Option<String>,
    bot_id: Option<String>,
    text: Option<String>,
    ts: Option<String>,
    thread_ts: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventEnvelope {
    schema: Option<String>,
    header: Option<FeishuLarkEventHeader>,
    event: Option<FeishuLarkEventBody>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventHeader {
    event_id: Option<String>,
    event_type: Option<String>,
    tenant_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventBody {
    sender: Option<FeishuLarkEventSender>,
    message: Option<FeishuLarkEventMessage>,
    chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventSender {
    sender_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventMessage {
    message_id: Option<String>,
    root_id: Option<String>,
    chat_id: Option<String>,
    chat_type: Option<String>,
    message_type: Option<String>,
    content: Option<String>,
    mentions: Option<Vec<FeishuLarkEventMention>>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventMention {
    key: Option<String>,
    id: Option<FeishuLarkEventMentionId>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkEventMentionId {
    open_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkTextContent {
    text: Option<String>,
}

fn required_owned(field: &'static str, value: Option<&str>) -> anyhow::Result<String> {
    Ok(required_trimmed(field, value)?.to_string())
}

fn required_trimmed<'a>(field: &'static str, value: Option<&'a str>) -> anyhow::Result<&'a str> {
    let Some(value) = value.and_then(optional_trimmed) else {
        anyhow::bail!("provider inbound `{field}` must be present");
    };
    Ok(value)
}

fn ensure_present(field: &'static str, value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.trim().is_empty(),
        "provider inbound `{field}` must be present"
    );
    Ok(())
}

fn normalized_reply_text(value: Option<&str>) -> Option<String> {
    value
        .and_then(optional_trimmed)
        .map(std::string::ToString::to_string)
}

fn parse_provider_control_command(text: &str) -> Option<ProviderControlCommand> {
    let (command, rest) = split_command(text)?;
    match command {
        "/bind" => {
            let project_path = rest.trim();
            if project_path.is_empty() {
                return Some(ProviderControlCommand::Invalid {
                    message: "Use the full path to a folder on your Mac.\n\nDirect chat:\n`/bind /path/to/project`\n\nGroup room:\n`@your-bot /bind /path/to/project`\n\nUse Lark's @ menu to select me. Do not type the @ name as plain text."
                        .to_string(),
                });
            }
            Some(ProviderControlCommand::BindProject {
                project_path: project_path.to_string(),
            })
        }
        "/help" => {
            if !rest.trim().is_empty() {
                return Some(ProviderControlCommand::Invalid {
                    message: "Use `/help`.".to_string(),
                });
            }
            Some(ProviderControlCommand::Help)
        }
        "/new" => Some(parse_new_session_command(rest)),
        "/status" => {
            if !rest.trim().is_empty() {
                return Some(ProviderControlCommand::Invalid {
                    message: "Use `/status`.".to_string(),
                });
            }
            Some(ProviderControlCommand::Status)
        }
        "/unbind" => {
            let project_path = rest.trim();
            if project_path.is_empty() {
                return Some(ProviderControlCommand::UnbindProject { project_path: None });
            }
            Some(ProviderControlCommand::UnbindProject {
                project_path: Some(project_path.to_string()),
            })
        }
        _ => None,
    }
}

fn parse_new_session_command(rest: &str) -> ProviderControlCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ProviderControlCommand::Invalid {
            message: "Tell Codex what to do.\n\nDirect chat:\n`/new what you want Codex to do`\n\nGroup room:\n`@your-bot /new what you want Codex to do`\n\nUse Lark's @ menu to select me in group rooms.".to_string(),
        };
    }

    let Some((first, remaining)) = rest.split_once(char::is_whitespace) else {
        return ProviderControlCommand::NewSession {
            project_path: None,
            prompt: rest.to_string(),
        };
    };

    if first.starts_with('/') {
        let prompt = remaining.trim();
        if prompt.is_empty() {
            return ProviderControlCommand::Invalid {
                message: "Use the full path to a folder on your Mac.\n\nDirect chat:\n`/new /path/to/project what you want Codex to do`\n\nGroup room:\n`@your-bot /new /path/to/project what you want Codex to do`\n\nUse Lark's @ menu to select me in group rooms.".to_string(),
            };
        }

        return ProviderControlCommand::NewSession {
            project_path: Some(first.to_string()),
            prompt: prompt.to_string(),
        };
    }

    ProviderControlCommand::NewSession {
        project_path: None,
        prompt: rest.to_string(),
    }
}

fn addressed_feishu_lark_text(
    message: &FeishuLarkEventMessage,
    text: &str,
    bot_open_id: &str,
) -> Option<String> {
    if feishu_lark_message_requires_mention(message)
        && !has_structured_leading_lark_bot_mention(message, text, bot_open_id)
    {
        return None;
    }

    Some(trim_leading_lark_mentions(text).trim().to_string())
}

fn feishu_lark_message_requires_mention(message: &FeishuLarkEventMessage) -> bool {
    !feishu_lark_message_is_direct_chat(message)
}

fn feishu_lark_message_is_direct_chat(message: &FeishuLarkEventMessage) -> bool {
    optional_field(message.chat_type.as_deref()) == Some("p2p")
}

fn has_structured_leading_lark_bot_mention(
    message: &FeishuLarkEventMessage,
    text: &str,
    bot_open_id: &str,
) -> bool {
    let Some(token) = leading_lark_mention_token(text) else {
        return false;
    };

    message
        .mentions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|mention| {
            optional_field(mention.key.as_deref()) == Some(token)
                && mention
                    .id
                    .as_ref()
                    .and_then(|id| optional_field(id.open_id.as_deref()))
                    == Some(bot_open_id)
        })
}

fn leading_lark_mention_token(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    trimmed
        .strip_prefix('@')
        .and_then(|after_at| after_at.split_whitespace().next())
        .filter(|token| !token.is_empty())
        .map(|token| &trimmed[..1 + token.len()])
}

fn trim_leading_lark_mentions(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim_start();
        let Some(after_at) = trimmed.strip_prefix('@') else {
            return trimmed;
        };
        let Some((_, rest)) = after_at.split_once(char::is_whitespace) else {
            return trimmed;
        };
        text = rest;
    }
}

fn split_command(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if !text.starts_with('/') {
        return None;
    }
    let command_end = text.find(char::is_whitespace).unwrap_or(text.len());
    Some((&text[..command_end], &text[command_end..]))
}

fn optional_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn optional_field(value: Option<&str>) -> Option<&str> {
    value.and_then(optional_trimmed)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::provider_catalog::{ProviderMode, provider_mode_capability};
    use crate::response_surface_ledger::{InboundEventRecordDecision, NewResponseSurface};

    const TEST_LARK_BOT_OPEN_ID: &str = "ou_test_bot";

    fn control_result(command: NormalizedProviderControlCommand) -> ProviderControlNormalizeResult {
        ProviderControlNormalizeResult::ControlCommand(Box::new(command))
    }

    #[test]
    fn normalizes_slack_socket_mode_surface_reply_using_thread_ts_as_lookup_key() {
        let normalized = normalize_slack_socket_mode_surface_reply(
            "slack-app",
            include_bytes!("../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"),
        )
        .expect("Slack event should parse");

        assert_eq!(
            normalized,
            ProviderInboundNormalizeResult::SurfaceReply(NormalizedProviderSurfaceReply {
                provider_id: "slack-app".to_string(),
                provider_type: "slack".to_string(),
                provider_mode: ProviderMode::SlackApp,
                provider_account_id: "T123ABC456".to_string(),
                provider_conversation_id: "C123ABC456".to_string(),
                provider_thread_id: "1716200000.000100".to_string(),
                provider_event_id: "Ev123ABC456".to_string(),
                provider_reply_message_id: Some("1716200011.000200".to_string()),
                reply_text: "Run the tests and fix the failing one.".to_string(),
            })
        );
    }

    #[test]
    fn slack_socket_mode_root_message_without_thread_ts_is_not_a_surface_reply() {
        let raw = br#"{
            "type": "events_api",
            "envelope_id": "envelope-1",
            "payload": {
                "type": "event_callback",
                "team_id": "T123ABC456",
                "event_id": "Ev123ABC456",
                "event": {
                    "type": "message",
                    "channel": "C123ABC456",
                    "user": "U123ABC456",
                    "text": "ordinary channel message",
                    "ts": "1716200011.000200"
                }
            }
        }"#;

        assert_eq!(
            normalize_slack_socket_mode_surface_reply("slack-app", raw)
                .expect("Slack event should parse"),
            ProviderInboundNormalizeResult::Skip(ProviderInboundSkipReason::NotSurfaceReply)
        );
    }

    #[test]
    fn slack_socket_mode_mention_without_surface_lookup_key_is_not_a_trigger() {
        let raw = br#"{
            "type": "events_api",
            "envelope_id": "envelope-1",
            "payload": {
                "type": "event_callback",
                "team_id": "T123ABC456",
                "event_id": "Ev123ABC456",
                "event": {
                    "type": "message",
                    "channel": "C123ABC456",
                    "user": "U123ABC456",
                    "text": "<@B123ABC456> continue this task",
                    "ts": "1716200011.000200"
                }
            }
        }"#;

        assert_eq!(
            normalize_slack_socket_mode_surface_reply("slack-app", raw)
                .expect("Slack event should parse"),
            ProviderInboundNormalizeResult::Skip(ProviderInboundSkipReason::NotSurfaceReply)
        );
    }

    #[test]
    fn slack_socket_mode_bot_message_is_not_a_user_surface_reply() {
        let raw = br#"{
            "type": "events_api",
            "envelope_id": "envelope-1",
            "payload": {
                "type": "event_callback",
                "team_id": "T123ABC456",
                "event_id": "Ev123ABC456",
                "event": {
                    "type": "message",
                    "subtype": "bot_message",
                    "channel": "C123ABC456",
                    "bot_id": "B123ABC456",
                    "text": "bot message",
                    "ts": "1716200011.000200",
                    "thread_ts": "1716200000.000100"
                }
            }
        }"#;

        assert_eq!(
            normalize_slack_socket_mode_surface_reply("slack-app", raw)
                .expect("Slack event should parse"),
            ProviderInboundNormalizeResult::Skip(ProviderInboundSkipReason::IgnoredMessageSubtype)
        );
    }

    #[test]
    fn slack_socket_mode_uses_payload_event_id_not_envelope_id_for_dedup() {
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };

        assert_eq!(reply.provider_event_id, "Ev123ABC456");
        assert_ne!(reply.provider_event_id, "envelope-123");
    }

    #[test]
    fn normalizes_feishu_lark_surface_reply_using_root_id_as_lookup_key() {
        let normalized = normalize_feishu_lark_long_connection_surface_reply(
            "lark-app",
            TEST_LARK_BOT_OPEN_ID,
            include_bytes!("../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"),
        )
        .expect("Feishu/Lark event should parse");

        assert_eq!(
            normalized,
            ProviderInboundNormalizeResult::SurfaceReply(NormalizedProviderSurfaceReply {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                provider_thread_id: "om_root_message_id".to_string(),
                provider_event_id: "om_reply_message_id".to_string(),
                provider_reply_message_id: Some("om_reply_message_id".to_string()),
                reply_text: "continue with README".to_string(),
            })
        );
    }

    #[test]
    fn feishu_lark_uses_reply_message_id_not_header_event_id_for_dedup() {
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_feishu_lark_long_connection_surface_reply(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                include_bytes!("../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"),
            )
            .expect("Feishu/Lark event should parse")
        else {
            panic!("event should be a surface reply");
        };

        assert_eq!(reply.provider_event_id, "om_reply_message_id");
        assert_ne!(reply.provider_event_id, "5e3702a84e847582be8db7fb73283c02");
    }

    #[test]
    fn normalizes_feishu_lark_root_bind_command_after_bot_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_bind_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 /bind /Users/felix/Desktop/felix-projects/agents-router\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_test_bot" },
                            "name": "Agents Router",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        let normalized = normalize_feishu_lark_long_connection_control_command(
            "lark-app",
            TEST_LARK_BOT_OPEN_ID,
            raw,
        )
        .expect("Feishu/Lark event should parse");

        assert_eq!(
            normalized,
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_thread_id: "om_bind_message_id".to_string(),
                provider_event_id: "om_bind_message_id".to_string(),
                command: ProviderControlCommand::BindProject {
                    project_path: "/Users/felix/Desktop/felix-projects/agents-router".to_string(),
                },
            })
        );
    }

    #[test]
    fn normalizes_feishu_lark_root_new_command_after_bot_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_new_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 /new Reply exactly OK.\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_test_bot" },
                            "name": "Agents Router",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        let normalized = normalize_feishu_lark_long_connection_control_command(
            "lark-app",
            TEST_LARK_BOT_OPEN_ID,
            raw,
        )
        .expect("Feishu/Lark event should parse");

        assert_eq!(
            normalized,
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_thread_id: "om_new_message_id".to_string(),
                provider_event_id: "om_new_message_id".to_string(),
                command: ProviderControlCommand::NewSession {
                    project_path: None,
                    prompt: "Reply exactly OK.".to_string(),
                },
            })
        );
    }

    #[test]
    fn feishu_lark_mention_without_root_id_is_not_a_trigger_even_with_parent_id() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "5e3702a84e847582be8db7fb73283c02",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_reply_message_id",
                    "root_id": "",
                    "parent_id": "om_parent_message_id",
                    "chat_id": "oc_5ce6d572455d361153b7xx51da133945",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 continue\"}"
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_surface_reply(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            ProviderInboundNormalizeResult::Skip(ProviderInboundSkipReason::NotSurfaceReply)
        );
    }

    #[test]
    fn feishu_lark_shared_thread_reply_without_mention_is_ignored() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_reply_message_id",
                    "root_id": "om_root_message_id",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"continue without waking the agent\"}"
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_surface_reply(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            ProviderInboundNormalizeResult::Skip(ProviderInboundSkipReason::NotAddressedToBot)
        );
    }

    #[test]
    fn feishu_lark_shared_thread_reply_mentioning_another_user_is_ignored() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_reply_message_id",
                    "root_id": "om_root_message_id",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 continue without waking the agent\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_other_user" },
                            "name": "Another User",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_surface_reply(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            ProviderInboundNormalizeResult::Skip(ProviderInboundSkipReason::NotAddressedToBot)
        );
    }

    #[test]
    fn feishu_lark_dm_reply_does_not_require_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_reply_message_id",
                    "root_id": "om_root_message_id",
                    "chat_id": "oc_dm",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"continue without a mention\"}"
                }
            }
        }"#;

        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_feishu_lark_long_connection_surface_reply(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse")
        else {
            panic!("p2p reply should normalize without a mention");
        };

        assert_eq!(reply.reply_text, "continue without a mention");
    }

    #[test]
    fn feishu_lark_shared_root_bind_without_mention_is_ignored() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_bind_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"/bind /Users/felix/Desktop/felix-projects/agents-router\"}"
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            ProviderControlNormalizeResult::Skip(ProviderControlSkipReason::NotAddressedToBot)
        );
    }

    #[test]
    fn feishu_lark_shared_root_bind_plain_at_without_structured_mention_is_ignored() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_bind_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@AgentsRouter /bind /Users/felix/Desktop/felix-projects/agents-router\"}"
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            ProviderControlNormalizeResult::Skip(ProviderControlSkipReason::NotAddressedToBot)
        );
    }

    #[test]
    fn feishu_lark_bot_added_event_normalizes_as_room_event() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-bot-added",
                "event_type": "im.chat.member.bot.added_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "chat_id": "oc_project_room"
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            ProviderControlNormalizeResult::RoomEvent(Box::new(NormalizedProviderRoomEvent {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_event_id: "event-bot-added".to_string(),
                event: ProviderRoomEvent::BotAddedToChat,
            }))
        );
    }

    #[test]
    fn feishu_lark_shared_root_plain_mention_returns_room_guidance() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_plain_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 hello\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_test_bot" },
                            "name": "Agents Router",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_thread_id: "om_plain_message_id".to_string(),
                provider_event_id: "om_plain_message_id".to_string(),
                command: ProviderControlCommand::RoomGuidance,
            })
        );
    }

    #[test]
    fn feishu_lark_direct_chat_bind_sets_direct_project() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_bind_message_id",
                    "root_id": "",
                    "chat_id": "oc_direct_chat",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"/bind /Users/felix/Desktop/felix-projects/agents-router\"}"
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_direct_chat".to_string(),
                provider_thread_id: "om_bind_message_id".to_string(),
                provider_event_id: "om_bind_message_id".to_string(),
                command: ProviderControlCommand::DirectBindProject {
                    project_path: "/Users/felix/Desktop/felix-projects/agents-router".to_string(),
                },
            })
        );
    }

    #[test]
    fn feishu_lark_direct_chat_plain_text_starts_new_session() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_direct_message_id",
                    "root_id": "",
                    "chat_id": "oc_direct_chat",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"what is the time\"}"
                }
            }
        }"#;

        assert_eq!(
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse"),
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_direct_chat".to_string(),
                provider_thread_id: "om_direct_message_id".to_string(),
                provider_event_id: "om_direct_message_id".to_string(),
                command: ProviderControlCommand::DirectNewSession {
                    project_path: None,
                    prompt: "what is the time".to_string(),
                },
            })
        );
    }

    #[test]
    fn feishu_lark_direct_chat_help_does_not_require_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_help_message_id",
                    "root_id": "",
                    "chat_id": "oc_direct_chat",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"/help\"}"
                }
            }
        }"#;

        let ProviderControlNormalizeResult::ControlCommand(command) =
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse")
        else {
            panic!("direct chat help should normalize");
        };

        assert_eq!(command.command, ProviderControlCommand::DirectHelp);
    }

    #[test]
    fn feishu_lark_direct_chat_status_does_not_require_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_status_message_id",
                    "root_id": "",
                    "chat_id": "oc_direct_chat",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"/status\"}"
                }
            }
        }"#;

        let ProviderControlNormalizeResult::ControlCommand(command) =
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse")
        else {
            panic!("direct chat status should normalize");
        };

        assert_eq!(command.command, ProviderControlCommand::DirectStatus);
    }

    #[test]
    fn feishu_lark_direct_chat_new_with_explicit_project_does_not_require_room() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_new_message_id",
                    "root_id": "",
                    "chat_id": "oc_direct_chat",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"/new /Users/felix/Desktop/felix-projects/agents-router Reply OK.\"}"
                }
            }
        }"#;

        let ProviderControlNormalizeResult::ControlCommand(command) =
            normalize_feishu_lark_long_connection_control_command(
                "lark-app",
                TEST_LARK_BOT_OPEN_ID,
                raw,
            )
            .expect("Feishu/Lark event should parse")
        else {
            panic!("direct chat new should normalize");
        };

        assert_eq!(
            command.command,
            ProviderControlCommand::DirectNewSession {
                project_path: Some("/Users/felix/Desktop/felix-projects/agents-router".to_string()),
                prompt: "Reply OK.".to_string(),
            }
        );
    }

    #[test]
    fn normalizes_feishu_lark_help_command_after_bot_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_help_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 /help\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_test_bot" },
                            "name": "Agents Router",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        let normalized = normalize_feishu_lark_long_connection_control_command(
            "lark-app",
            TEST_LARK_BOT_OPEN_ID,
            raw,
        )
        .expect("Feishu/Lark event should parse");

        assert_eq!(
            normalized,
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_thread_id: "om_help_message_id".to_string(),
                provider_event_id: "om_help_message_id".to_string(),
                command: ProviderControlCommand::Help,
            })
        );
    }

    #[test]
    fn normalizes_feishu_lark_status_command_after_bot_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_status_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 /status\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_test_bot" },
                            "name": "Agents Router",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        let normalized = normalize_feishu_lark_long_connection_control_command(
            "lark-app",
            TEST_LARK_BOT_OPEN_ID,
            raw,
        )
        .expect("Feishu/Lark event should parse");

        assert_eq!(
            normalized,
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_thread_id: "om_status_message_id".to_string(),
                provider_event_id: "om_status_message_id".to_string(),
                command: ProviderControlCommand::Status,
            })
        );
    }

    #[test]
    fn normalizes_feishu_lark_unbind_command_after_bot_mention() {
        let raw = br#"{
            "schema": "2.0",
            "header": {
                "event_id": "event-1",
                "event_type": "im.message.receive_v1",
                "tenant_key": "2ca1d211f64f6438"
            },
            "event": {
                "sender": { "sender_type": "user" },
                "message": {
                    "message_id": "om_unbind_message_id",
                    "root_id": "",
                    "chat_id": "oc_project_room",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 /unbind /Users/felix/Desktop/felix-projects/agents-router\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "id": { "open_id": "ou_test_bot" },
                            "name": "Agents Router",
                            "tenant_key": "2ca1d211f64f6438"
                        }
                    ]
                }
            }
        }"#;

        let normalized = normalize_feishu_lark_long_connection_control_command(
            "lark-app",
            TEST_LARK_BOT_OPEN_ID,
            raw,
        )
        .expect("Feishu/Lark event should parse");

        assert_eq!(
            normalized,
            control_result(NormalizedProviderControlCommand {
                provider_id: "lark-app".to_string(),
                provider_type: "feishu_lark".to_string(),
                provider_mode: ProviderMode::FeishuLarkAppBot,
                provider_account_id: "2ca1d211f64f6438".to_string(),
                provider_conversation_id: "oc_project_room".to_string(),
                provider_thread_id: "om_unbind_message_id".to_string(),
                provider_event_id: "om_unbind_message_id".to_string(),
                command: ProviderControlCommand::UnbindProject {
                    project_path: Some(
                        "/Users/felix/Desktop/felix-projects/agents-router".to_string()
                    ),
                },
            })
        );
    }

    #[test]
    fn lookup_and_claim_returns_surface_bound_work_without_agent_decision() {
        let now = test_time();
        let mut ledger = ledger_with_slack_surface(now);
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };

        let decision = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply,
            now + Duration::seconds(1),
        )
        .expect("lookup should succeed");

        let ProviderInboundDecision::Ready(ready) = decision else {
            panic!("reply should be ready for the next boundary");
        };
        assert_eq!(ready.surface.source_id, "codex_desktop");
        assert_eq!(ready.surface.source_session_id, "session-1");
        assert_eq!(
            ready.reply.reply_text,
            "Run the tests and fix the failing one."
        );
        assert!(!ready.provider_event_id_hash.contains("Ev123ABC456"));
    }

    #[test]
    fn lookup_miss_does_not_claim_inbound_event() {
        let now = test_time();
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };

        let decision = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply.clone(),
            now,
        )
        .expect("lookup should not fail");

        assert_eq!(
            decision,
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::SurfaceLookupMiss)
        );

        let surface = ledger
            .create_surface_at(slack_surface(now), now + Duration::seconds(1))
            .expect("surface should still be claimable later");
        let claim = ledger
            .claim_inbound_event_at(
                InboundEventDedupInput {
                    provider_id: reply.provider_id,
                    provider_type: reply.provider_type,
                    provider_account_id: reply.provider_account_id,
                    provider_conversation_id: reply.provider_conversation_id,
                    provider_event_id: reply.provider_event_id,
                    surface_id: surface.surface_id,
                },
                now + Duration::seconds(2),
            )
            .expect("event should not have been claimed on miss");

        assert!(matches!(claim, InboundEventClaimDecision::Claimed { .. }));
    }

    #[test]
    fn thread_session_binding_can_create_ready_work_after_surface_miss() {
        let now = test_time();
        let mut ledger = ResponseSurfaceLedger::in_memory();
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };

        let decision = lookup_and_claim_provider_thread_session_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            slack_thread_session_binding(),
            reply,
            now,
        )
        .expect("thread binding should claim");

        let ProviderInboundDecision::Ready(ready) = decision else {
            panic!("thread binding should create ready work");
        };
        assert!(ready.surface.surface_id.starts_with("bridge-thread-"));
        assert_eq!(ready.surface.source_id, "codex_desktop");
        assert_eq!(ready.surface.source_session_id, "session-1");
        assert_eq!(ready.surface.provider_thread_id, "1716200000.000100");
        assert_eq!(
            ready.reply.reply_text,
            "Run the tests and fix the failing one."
        );
    }

    #[test]
    fn duplicate_inbound_event_does_not_create_second_ready_work_item() {
        let now = test_time();
        let mut ledger = ledger_with_slack_surface(now);
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };

        let first = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply.clone(),
            now + Duration::seconds(1),
        )
        .expect("first lookup should succeed");
        let second = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply,
            now + Duration::seconds(2),
        )
        .expect("duplicate lookup should succeed");

        assert!(matches!(first, ProviderInboundDecision::Ready(_)));
        assert!(matches!(
            second,
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::EventAlreadyProcessing { .. })
        ));
    }

    #[test]
    fn different_provider_event_id_with_same_text_creates_separate_ready_work_items() {
        let now = test_time();
        let mut ledger = ledger_with_slack_surface(now);
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };
        let mut second_reply = reply.clone();
        second_reply.provider_event_id = "Ev456DIFFERENT".to_string();
        second_reply.reply_text = reply.reply_text.clone();

        let first = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply,
            now + Duration::seconds(1),
        )
        .expect("first lookup should succeed");
        let second = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            second_reply,
            now + Duration::seconds(2),
        )
        .expect("second lookup should succeed");

        assert!(matches!(first, ProviderInboundDecision::Ready(_)));
        assert!(matches!(second, ProviderInboundDecision::Ready(_)));
    }

    #[test]
    fn processed_inbound_event_is_skipped_as_duplicate() {
        let now = test_time();
        let mut ledger = ledger_with_slack_surface(now);
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };
        let surface_id = match ledger
            .lookup_surface_at(
                crate::response_surface_ledger::ResponseSurfaceLookupQuery {
                    provider_id: reply.provider_id.clone(),
                    provider_account_id: reply.provider_account_id.clone(),
                    provider_conversation_id: reply.provider_conversation_id.clone(),
                    provider_thread_id: reply.provider_thread_id.clone(),
                },
                now,
            )
            .expect("lookup should succeed")
        {
            crate::response_surface_ledger::ResponseSurfaceLookupResult::Hit(surface) => {
                surface.surface_id
            }
            _ => panic!("surface should exist"),
        };
        assert!(matches!(
            ledger
                .record_processed_inbound_event_at(
                    InboundEventDedupInput {
                        provider_id: reply.provider_id.clone(),
                        provider_type: reply.provider_type.clone(),
                        provider_account_id: reply.provider_account_id.clone(),
                        provider_conversation_id: reply.provider_conversation_id.clone(),
                        provider_event_id: reply.provider_event_id.clone(),
                        surface_id,
                    },
                    now + Duration::seconds(1),
                )
                .expect("processed event should record"),
            InboundEventRecordDecision::Recorded { .. }
        ));

        let decision = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply,
            now + Duration::seconds(2),
        )
        .expect("duplicate lookup should succeed");

        assert!(matches!(
            decision,
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::DuplicateEvent { .. })
        ));
    }

    #[test]
    fn unsupported_provider_mode_does_not_enter_inbound_path() {
        let now = test_time();
        let mut ledger = ledger_with_slack_surface(now);
        let ProviderInboundNormalizeResult::SurfaceReply(mut reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };
        reply.provider_mode = ProviderMode::SlackIncomingWebhook;

        let decision = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackIncomingWebhook),
            reply,
            now,
        )
        .expect("unsupported provider should skip");

        assert_eq!(
            decision,
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::UnsupportedProviderMode(
                ProviderMode::SlackIncomingWebhook
            ))
        );
    }

    #[test]
    fn closed_surface_does_not_create_ready_work_item() {
        let now = test_time();
        let mut ledger = ledger_with_slack_surface(now);
        let surface = match ledger
            .lookup_surface_at(slack_lookup_query(), now)
            .expect("surface should lookup")
        {
            crate::response_surface_ledger::ResponseSurfaceLookupResult::Hit(surface) => surface,
            _ => panic!("surface should exist"),
        };
        assert!(
            ledger
                .close_surface(&surface.surface_id)
                .expect("surface should close")
        );
        let ProviderInboundNormalizeResult::SurfaceReply(reply) =
            normalize_slack_socket_mode_surface_reply(
                "slack-app",
                include_bytes!(
                    "../tests/fixtures/provider_inbound/slack_socket_surface_reply.json"
                ),
            )
            .expect("Slack event should parse")
        else {
            panic!("event should be a surface reply");
        };

        let decision = lookup_and_claim_provider_surface_reply(
            &mut ledger,
            provider_mode_capability(ProviderMode::SlackApp),
            reply,
            now + Duration::seconds(1),
        )
        .expect("closed surface should skip");

        assert_eq!(
            decision,
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::SurfaceClosed {
                surface_id: surface.surface_id,
            })
        );
    }

    fn ledger_with_slack_surface(now: DateTime<Utc>) -> ResponseSurfaceLedger {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        ledger
            .create_surface_at(slack_surface(now), now)
            .expect("surface should be created");
        ledger
    }

    fn slack_surface(_now: DateTime<Utc>) -> NewResponseSurface {
        NewResponseSurface {
            signal_id: "signal-1".to_string(),
            delivery_id: "delivery-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            provider_id: "slack-app".to_string(),
            provider_type: "slack".to_string(),
            provider_mode: ProviderMode::SlackApp,
            provider_account_id: "T123ABC456".to_string(),
            provider_conversation_id: "C123ABC456".to_string(),
            provider_message_id: "1716200000.000100".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
            route_binding_hash: None,
        }
    }

    fn slack_lookup_query() -> ResponseSurfaceLookupQuery {
        ResponseSurfaceLookupQuery {
            provider_id: "slack-app".to_string(),
            provider_account_id: "T123ABC456".to_string(),
            provider_conversation_id: "C123ABC456".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
        }
    }

    fn slack_thread_session_binding() -> ThreadSessionBindingRecord {
        ThreadSessionBindingRecord {
            provider_id: "slack-app".to_string(),
            provider_type: "slack".to_string(),
            provider_account_id: "T123ABC456".to_string(),
            provider_conversation_id: "C123ABC456".to_string(),
            provider_thread_id: "1716200000.000100".to_string(),
            project_path: "/Users/tester/projects/agents-router".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
            created_at: test_time(),
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
