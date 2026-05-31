use std::path::Path;

use chrono::{DateTime, Utc};

use crate::bridge_binding_ledger::{BridgeBindingLedger, RoomProjectBindingInput};
use crate::config::is_clean_absolute_project_path;
use crate::provider_inbound::{NormalizedProviderControlCommand, ProviderControlCommand};
use crate::response_surface_ledger::provider_event_id_hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeControlReply {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub provider_event_id_hash: String,
    pub text: String,
}

pub(crate) fn handle_provider_control_command(
    ledger: &mut BridgeBindingLedger,
    command: NormalizedProviderControlCommand,
    now: DateTime<Utc>,
) -> anyhow::Result<BridgeControlReply> {
    let provider_event_id_hash = provider_event_id_hash(
        &command.provider_type,
        &command.provider_id,
        &command.provider_account_id,
        &command.provider_conversation_id,
        &command.provider_event_id,
    );
    let text = match &command.command {
        ProviderControlCommand::BindProject { project_path } => {
            bind_project(ledger, &command, project_path, now)?
        }
        ProviderControlCommand::Invalid { message } => message.clone(),
    };

    Ok(BridgeControlReply {
        provider_id: command.provider_id,
        provider_type: command.provider_type,
        provider_account_id: command.provider_account_id,
        provider_conversation_id: command.provider_conversation_id,
        provider_thread_id: command.provider_thread_id,
        provider_event_id_hash,
        text,
    })
}

fn bind_project(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    project_path: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    if !is_clean_absolute_project_path(project_path) {
        return Ok("Use `/bind /absolute/project/path`.".to_string());
    }
    if !Path::new(project_path).is_dir() {
        return Ok(format!(
            "I can't find that folder on this Mac:\n{project_path}"
        ));
    }

    ledger.connect_room_project_at(
        RoomProjectBindingInput {
            provider_id: command.provider_id.clone(),
            provider_type: command.provider_type.clone(),
            provider_account_id: command.provider_account_id.clone(),
            provider_conversation_id: command.provider_conversation_id.clone(),
            project_path: project_path.to_string(),
        },
        now,
    )?;

    Ok(format!("Connected this room to:\n{project_path}"))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::provider_catalog::ProviderMode;

    #[test]
    fn bind_project_connects_room_to_existing_folder() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::BindProject {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("bind should handle");

        assert_eq!(reply.text, format!("Connected this room to:\n{path}"));
        assert_eq!(ledger.connected_rooms_for_project(&path).len(), 1);
    }

    #[test]
    fn bind_project_reports_missing_folder_without_recording_binding() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::BindProject {
                project_path: "/definitely/missing/agents-router".to_string(),
            }),
            test_time(),
        )
        .expect("bind should handle");

        assert_eq!(
            reply.text,
            "I can't find that folder on this Mac:\n/definitely/missing/agents-router"
        );
        assert!(
            ledger
                .connected_rooms_for_project("/definitely/missing/agents-router")
                .is_empty()
        );
    }

    fn command(command: ProviderControlCommand) -> NormalizedProviderControlCommand {
        NormalizedProviderControlCommand {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: "room-1".to_string(),
            provider_thread_id: "message-1".to_string(),
            provider_event_id: "message-1".to_string(),
            command,
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 31, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
