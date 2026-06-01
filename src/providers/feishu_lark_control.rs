use tracing::{info, warn};

use crate::agent_controller::{ProviderThreadReplyAdapter, ProviderThreadReplyRequest};
use crate::bridge_control::{BridgeControlReply, BridgeRoomMessage};
use crate::providers::feishu_lark::{FeishuLarkConversationTextRequest, FeishuLarkProvider};
use crate::runtime::RuntimeState;

pub(crate) fn dispatch_feishu_lark_control_reply(
    runtime_state: RuntimeState,
    reply: BridgeControlReply,
) {
    info!(
        provider.id = %reply.provider_id,
        provider.type = %reply.provider_type,
        provider.room.id = %reply.provider_conversation_id,
        provider.thread.id = %reply.provider_thread_id,
        event.hash = %reply.provider_event_id_hash,
        event = "provider_control.reply.dispatched",
    );
    tokio::spawn(async move {
        if let Err(error) = send_feishu_lark_control_reply(runtime_state, reply).await {
            warn!(
                error = %error,
                event = "provider_control.reply.failed",
            );
        }
    });
}

pub(crate) fn dispatch_feishu_lark_room_message(
    runtime_state: RuntimeState,
    message: BridgeRoomMessage,
) {
    info!(
        provider.id = %message.provider_id,
        provider.type = %message.provider_type,
        provider.room.id = %message.provider_conversation_id,
        event.hash = %message.provider_event_id_hash,
        event = "provider_control.room_message.dispatched",
    );
    tokio::spawn(async move {
        if let Err(error) = send_feishu_lark_room_message(runtime_state, message).await {
            warn!(
                error = %error,
                event = "provider_control.room_message.failed",
            );
        }
    });
}

async fn send_feishu_lark_control_reply(
    runtime_state: RuntimeState,
    reply: BridgeControlReply,
) -> anyhow::Result<()> {
    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&reply.provider_id) else {
        anyhow::bail!(
            "provider `{}` missing before sending control reply",
            reply.provider_id
        );
    };
    let provider_reply = FeishuLarkProvider::from_config(current_provider)?;
    send_control_thread_reply(&provider_reply, reply).await
}

async fn send_feishu_lark_room_message(
    runtime_state: RuntimeState,
    message: BridgeRoomMessage,
) -> anyhow::Result<()> {
    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&message.provider_id) else {
        anyhow::bail!(
            "provider `{}` missing before sending room message",
            message.provider_id
        );
    };
    let provider = FeishuLarkProvider::from_config(current_provider)?;
    let success = provider
        .send_text_to_conversation(FeishuLarkConversationTextRequest {
            provider_id: message.provider_id.clone(),
            provider_type: message.provider_type.clone(),
            provider_account_id: message.provider_account_id.clone(),
            provider_conversation_id: message.provider_conversation_id.clone(),
            provider_event_id_hash: message.provider_event_id_hash.clone(),
            text: message.text.clone(),
        })
        .await?;

    info!(
        provider.id = %message.provider_id,
        provider.type = %message.provider_type,
        provider.room.id = %message.provider_conversation_id,
        provider.reply.message_id = success.provider_message_id.as_deref(),
        event.hash = %message.provider_event_id_hash,
        event = "provider_control.room_message.succeeded",
    );
    Ok(())
}

async fn send_control_thread_reply(
    provider_reply: &dyn ProviderThreadReplyAdapter,
    reply: BridgeControlReply,
) -> anyhow::Result<()> {
    if provider_reply.provider_id() != reply.provider_id
        || provider_reply.provider_type() != reply.provider_type
    {
        anyhow::bail!("provider thread reply adapter does not match control reply provider");
    }

    info!(
        provider.id = %reply.provider_id,
        provider.type = %reply.provider_type,
        provider.room.id = %reply.provider_conversation_id,
        provider.thread.id = %reply.provider_thread_id,
        event.hash = %reply.provider_event_id_hash,
        event = "provider_control.reply.started",
    );
    let result = provider_reply
        .send_thread_reply(ProviderThreadReplyRequest {
            provider_id: reply.provider_id.clone(),
            provider_type: reply.provider_type.clone(),
            provider_account_id: reply.provider_account_id.clone(),
            provider_conversation_id: reply.provider_conversation_id.clone(),
            provider_thread_id: reply.provider_thread_id.clone(),
            surface_id: "bridge-control".to_string(),
            provider_event_id_hash: reply.provider_event_id_hash.clone(),
            text: reply.text.clone(),
        })
        .await;

    match result {
        Ok(success) => {
            info!(
                provider.id = %reply.provider_id,
                provider.type = %reply.provider_type,
                provider.room.id = %reply.provider_conversation_id,
                provider.thread.id = %reply.provider_thread_id,
                provider.reply.message_id = success.provider_reply_message_id.as_deref(),
                event.hash = %reply.provider_event_id_hash,
                event = "provider_control.reply.succeeded",
            );
            Ok(())
        }
        Err(error) => {
            anyhow::bail!("{}", error.message);
        }
    }
}
