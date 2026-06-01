use tracing::{info, warn};

use crate::agent_controller::{ProviderThreadReplyAdapter, ProviderThreadReplyRequest};
use crate::bridge_binding_ledger::{ProjectRoomProposalStatus, RoomProjectBindingInput};
use crate::bridge_control::{BridgeControlReply, BridgeProjectRoomPrompt, BridgeRoomMessage};
use crate::provider_inbound::{NormalizedProviderProjectRoomAction, ProviderProjectRoomAction};
use crate::providers::feishu_lark::{
    FeishuLarkConversationTextRequest, FeishuLarkCreateProjectRoomRequest, FeishuLarkProvider,
};
use crate::response_surface_ledger::provider_event_id_hash;
use crate::router::{ProjectRoomPromptRequest, Provider};
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

pub(crate) fn dispatch_feishu_lark_project_room_prompt(
    runtime_state: RuntimeState,
    prompt: BridgeProjectRoomPrompt,
) {
    info!(
        provider.id = %prompt.provider_id,
        provider.type = %prompt.provider_type,
        provider.room.id = %prompt.provider_conversation_id,
        provider.thread.id = %prompt.provider_thread_id,
        proposal.id = %prompt.proposal_id,
        project.path = %prompt.project_path,
        event.hash = %prompt.provider_event_id_hash,
        event = "provider_control.project_room_prompt.dispatched",
    );
    tokio::spawn(async move {
        if let Err(error) = send_feishu_lark_project_room_prompt(runtime_state, prompt).await {
            warn!(
                error = %error,
                event = "provider_control.project_room_prompt.failed",
            );
        }
    });
}

pub(crate) fn dispatch_feishu_lark_project_room_action(
    runtime_state: RuntimeState,
    action: NormalizedProviderProjectRoomAction,
) {
    info!(
        provider.id = %action.provider_id,
        provider.type = %action.provider_type,
        provider.chat.id = %action.provider_conversation_id,
        proposal.id = %action.proposal_id,
        action.kind = ?action.action,
        event.hash = %provider_event_id_hash(
            &action.provider_type,
            &action.provider_id,
            &action.provider_account_id,
            &action.provider_conversation_id,
            &action.provider_event_id,
        ),
        event = "provider_control.project_room_action.dispatched",
    );
    tokio::spawn(async move {
        if let Err(error) = handle_feishu_lark_project_room_action(runtime_state, action).await {
            warn!(
                error = %error,
                event = "provider_control.project_room_action.failed",
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

async fn send_feishu_lark_project_room_prompt(
    runtime_state: RuntimeState,
    prompt: BridgeProjectRoomPrompt,
) -> anyhow::Result<()> {
    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&prompt.provider_id) else {
        anyhow::bail!(
            "provider `{}` missing before sending project room prompt",
            prompt.provider_id
        );
    };
    let provider = FeishuLarkProvider::from_config(current_provider)?;
    drop(snapshot);

    let request = ProjectRoomPromptRequest {
        proposal_id: prompt.proposal_id.clone(),
        project_path: prompt.project_path.clone(),
        project_name: prompt.project_name.clone(),
        prompt_conversation_id: Some(prompt.provider_conversation_id.clone()),
    };
    let Some(send_prompt) = provider.send_project_room_prompt(request) else {
        runtime_state
            .bridge_binding_ledger()
            .update(|ledger| {
                ledger.mark_project_room_proposal_prompt_failed_at(
                    &prompt.proposal_id,
                    chrono::Utc::now(),
                )
            })
            .await?;
        send_project_room_prompt_failure_reply(
            &provider,
            &prompt,
            "I could not show the project room menu because this Feishu/Lark setup does not support project-room prompts.".to_string(),
        )
        .await?;
        return Ok(());
    };

    match send_prompt.await {
        Ok(result) => {
            runtime_state
                .bridge_binding_ledger()
                .update(|ledger| {
                    ledger.record_project_room_proposal_prompt_sent_at(
                        &prompt.proposal_id,
                        result.provider_account_id.clone(),
                        result.provider_conversation_id.clone(),
                        result.provider_message_id.clone(),
                        chrono::Utc::now(),
                    )
                })
                .await?;
            info!(
                provider.id = %prompt.provider_id,
                provider.type = %prompt.provider_type,
                proposal.id = %prompt.proposal_id,
                provider.room.id = result.provider_conversation_id.as_deref(),
                provider.message.id = result.provider_message_id.as_deref(),
                event.hash = %prompt.provider_event_id_hash,
                event = "provider_control.project_room_prompt.succeeded",
            );
        }
        Err(error) => {
            runtime_state
                .bridge_binding_ledger()
                .update(|ledger| {
                    ledger.mark_project_room_proposal_prompt_failed_at(
                        &prompt.proposal_id,
                        chrono::Utc::now(),
                    )
                })
                .await?;
            warn!(
                provider.id = %prompt.provider_id,
                provider.type = %prompt.provider_type,
                proposal.id = %prompt.proposal_id,
                error.kind = %error.kind.as_str(),
                error.retriable = error.retriable,
                http.status = error.http_status,
                provider.code = error.provider_code.as_deref(),
                error = %error.message,
                event.hash = %prompt.provider_event_id_hash,
                event = "provider_control.project_room_prompt.failed",
            );
            send_project_room_prompt_failure_reply(
                &provider,
                &prompt,
                "I could not show the project room menu. Check the Feishu/Lark Personal Agent setup, then try `/bind /path/to/project` again.".to_string(),
            )
            .await?;
        }
    }

    Ok(())
}

async fn handle_feishu_lark_project_room_action(
    runtime_state: RuntimeState,
    action: NormalizedProviderProjectRoomAction,
) -> anyhow::Result<()> {
    let snapshot = runtime_state.current()?;
    let Some(current_provider) = snapshot.config.provider(&action.provider_id) else {
        anyhow::bail!(
            "provider `{}` missing before handling project room action",
            action.provider_id
        );
    };
    let provider = FeishuLarkProvider::from_config(current_provider)?;
    drop(snapshot);

    let event_hash = provider_event_id_hash(
        &action.provider_type,
        &action.provider_id,
        &action.provider_account_id,
        &action.provider_conversation_id,
        &action.provider_event_id,
    );

    match action.action {
        ProviderProjectRoomAction::IgnoreProject => {
            let proposal = runtime_state
                .bridge_binding_ledger()
                .update(|ledger| {
                    ledger.ignore_project_room_proposal_at(&action.proposal_id, chrono::Utc::now())
                })
                .await?;
            let text = match proposal {
                Some(proposal) => format!(
                    "Got it. I will not ask about this project again:\n{}",
                    proposal.project_path
                ),
                None => "I could not find that project request anymore.".to_string(),
            };
            send_project_room_action_direct_message(&provider, &action, &event_hash, text).await?;
        }
        ProviderProjectRoomAction::UseExistingRoom => {
            let proposal = runtime_state
                .bridge_binding_ledger()
                .update(|ledger| ledger.lookup_project_room_proposal(&action.proposal_id))
                .await?;
            let text = match proposal {
                Some(proposal) => [
                    "No problem. To use an existing room for this project:".to_string(),
                    "".to_string(),
                    "1. Add me to that room.".to_string(),
                    "2. In the room, use Feishu/Lark's @ menu to select me.".to_string(),
                    "3. Send:".to_string(),
                    format!("`@your-bot /bind {}`", proposal.project_path),
                    "".to_string(),
                    "Tip: choose me from the @ menu so Feishu/Lark sends a real mention."
                        .to_string(),
                ]
                .join("\n"),
                None => "I could not find that project request anymore.".to_string(),
            };
            send_project_room_action_direct_message(&provider, &action, &event_hash, text).await?;
        }
        ProviderProjectRoomAction::CreateProjectRoom => {
            create_project_room_from_action(runtime_state, provider, action, event_hash).await?;
        }
    }

    Ok(())
}

async fn create_project_room_from_action(
    runtime_state: RuntimeState,
    provider: FeishuLarkProvider,
    action: NormalizedProviderProjectRoomAction,
    event_hash: String,
) -> anyhow::Result<()> {
    let Some(operator_open_id) = action
        .operator_open_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
    else {
        send_project_room_action_direct_message(
            &provider,
            &action,
            &event_hash,
            "I could not create a room because Feishu/Lark did not include your user ID in the button action.".to_string(),
        )
        .await?;
        return Ok(());
    };

    let proposal = runtime_state
        .bridge_binding_ledger()
        .update(|ledger| ledger.lookup_project_room_proposal(&action.proposal_id))
        .await?;
    let Some(proposal) = proposal else {
        send_project_room_action_direct_message(
            &provider,
            &action,
            &event_hash,
            "I could not find that project request anymore.".to_string(),
        )
        .await?;
        return Ok(());
    };
    if proposal.status == ProjectRoomProposalStatus::RoomCreated {
        let room = proposal
            .created_room_id
            .as_deref()
            .unwrap_or("the project room");
        send_project_room_action_direct_message(
            &provider,
            &action,
            &event_hash,
            format!("This project is already connected to {room}."),
        )
        .await?;
        return Ok(());
    }

    let created = provider
        .create_project_room(FeishuLarkCreateProjectRoomRequest {
            provider_id: action.provider_id.clone(),
            provider_type: action.provider_type.clone(),
            provider_account_id: action.provider_account_id.clone(),
            operator_open_id,
            project_name: proposal.project_name.clone(),
            project_path: proposal.project_path.clone(),
            proposal_id: proposal.proposal_id.clone(),
            proposal_created_at: proposal.created_at,
        })
        .await?;
    runtime_state
        .bridge_binding_ledger()
        .update(|ledger| {
            ledger.connect_room_project_at(
                RoomProjectBindingInput {
                    provider_id: action.provider_id.clone(),
                    provider_type: action.provider_type.clone(),
                    provider_account_id: action.provider_account_id.clone(),
                    provider_conversation_id: created.provider_conversation_id.clone(),
                    project_path: proposal.project_path.clone(),
                },
                chrono::Utc::now(),
            )?;
            ledger.complete_project_room_proposal_with_created_room_at(
                &proposal.proposal_id,
                action.provider_account_id.clone(),
                created.provider_conversation_id.clone(),
                chrono::Utc::now(),
            )
        })
        .await?;

    provider
        .send_text_to_conversation(FeishuLarkConversationTextRequest {
            provider_id: action.provider_id.clone(),
            provider_type: action.provider_type.clone(),
            provider_account_id: action.provider_account_id.clone(),
            provider_conversation_id: created.provider_conversation_id.clone(),
            provider_event_id_hash: event_hash.clone(),
            text: format!(
                "This room is now connected to:\n{}\n\nFuture Codex updates from this project will appear here.",
                proposal.project_path
            ),
        })
        .await?;
    send_project_room_action_direct_message(
        &provider,
        &action,
        &event_hash,
        format!(
            "Done. I created a new room for:\n{}\n\nFuture Codex updates from this project will appear there.",
            proposal.project_path
        ),
    )
    .await?;
    Ok(())
}

async fn send_project_room_action_direct_message(
    provider: &FeishuLarkProvider,
    action: &NormalizedProviderProjectRoomAction,
    event_hash: &str,
    text: String,
) -> anyhow::Result<()> {
    provider
        .send_text_to_conversation(FeishuLarkConversationTextRequest {
            provider_id: action.provider_id.clone(),
            provider_type: action.provider_type.clone(),
            provider_account_id: action.provider_account_id.clone(),
            provider_conversation_id: action.provider_conversation_id.clone(),
            provider_event_id_hash: event_hash.to_string(),
            text,
        })
        .await?;
    Ok(())
}

async fn send_project_room_prompt_failure_reply(
    provider: &FeishuLarkProvider,
    prompt: &BridgeProjectRoomPrompt,
    text: String,
) -> anyhow::Result<()> {
    send_control_thread_reply(
        provider,
        BridgeControlReply {
            provider_id: prompt.provider_id.clone(),
            provider_type: prompt.provider_type.clone(),
            provider_account_id: prompt.provider_account_id.clone(),
            provider_conversation_id: prompt.provider_conversation_id.clone(),
            provider_thread_id: prompt.provider_thread_id.clone(),
            provider_event_id_hash: prompt.provider_event_id_hash.clone(),
            text,
        },
    )
    .await
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
