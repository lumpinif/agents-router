use std::path::Path;

use chrono::{DateTime, Utc};

use crate::bridge_binding_ledger::{
    BridgeBindingLedger, RoomBindingQuery, RoomProjectBindingInput, RoomProjectBindingRecord,
};
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
        ProviderControlCommand::Help => help_text(),
        ProviderControlCommand::Status => room_status(ledger, &command),
        ProviderControlCommand::UnbindProject { project_path } => {
            unbind_project(ledger, &command, project_path.as_deref(), now)?
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

fn unbind_project(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    project_path: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    let Some(project_path) = project_path else {
        let connected = ledger.connected_projects_for_room(&room_query(command));
        if connected.is_empty() {
            return Ok("This room is not connected to any projects.".to_string());
        }

        let disconnected = disconnect_room_projects(ledger, command, &connected, now)?;
        return Ok(format!(
            "Disconnected this room from:\n{}",
            format_project_list(&disconnected)
        ));
    };

    if !is_clean_absolute_project_path(project_path) {
        return Ok("Use `/unbind /absolute/project/path` or `/unbind`.".to_string());
    }

    let disconnected = ledger.disconnect_room_project_at(
        RoomProjectBindingInput {
            provider_id: command.provider_id.clone(),
            provider_type: command.provider_type.clone(),
            provider_account_id: command.provider_account_id.clone(),
            provider_conversation_id: command.provider_conversation_id.clone(),
            project_path: project_path.to_string(),
        },
        now,
    )?;

    if disconnected.is_some() {
        Ok(format!("Disconnected this room from:\n{project_path}"))
    } else {
        Ok(format!("This room is not connected to:\n{project_path}"))
    }
}

fn room_status(ledger: &BridgeBindingLedger, command: &NormalizedProviderControlCommand) -> String {
    let connected = ledger.connected_projects_for_room(&room_query(command));
    if connected.is_empty() {
        return "This room is not connected to any projects.\nUse `/bind /absolute/project/path` in this room.".to_string();
    }

    format!(
        "This room is connected to:\n{}",
        format_project_list(&connected)
    )
}

fn help_text() -> String {
    [
        "Available commands:",
        "`/bind /absolute/project/path` - connect this room to a local project.",
        "`/status` - show connected projects for this room.",
        "`/unbind /absolute/project/path` - disconnect one project.",
        "`/unbind` - disconnect all projects from this room.",
        "",
        "In groups and threads, mention me before the command.",
    ]
    .join("\n")
}

fn disconnect_room_projects(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    records: &[RoomProjectBindingRecord],
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<RoomProjectBindingRecord>> {
    let mut disconnected = Vec::new();
    for record in records {
        if let Some(record) = ledger.disconnect_room_project_at(
            RoomProjectBindingInput {
                provider_id: command.provider_id.clone(),
                provider_type: command.provider_type.clone(),
                provider_account_id: command.provider_account_id.clone(),
                provider_conversation_id: command.provider_conversation_id.clone(),
                project_path: record.project_path.clone(),
            },
            now,
        )? {
            disconnected.push(record);
        }
    }
    Ok(disconnected)
}

fn room_query(command: &NormalizedProviderControlCommand) -> RoomBindingQuery {
    RoomBindingQuery {
        provider_id: command.provider_id.clone(),
        provider_type: command.provider_type.clone(),
        provider_account_id: command.provider_account_id.clone(),
        provider_conversation_id: command.provider_conversation_id.clone(),
    }
}

fn format_project_list(records: &[RoomProjectBindingRecord]) -> String {
    records
        .iter()
        .map(|record| format!("- {}", record.project_path))
        .collect::<Vec<_>>()
        .join("\n")
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

    #[test]
    fn status_reports_room_projects() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::BindProject {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("bind should handle");

        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::Status),
            test_time(),
        )
        .expect("status should handle");

        assert_eq!(reply.text, format!("This room is connected to:\n- {path}"));
    }

    #[test]
    fn status_reports_empty_room() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::Status),
            test_time(),
        )
        .expect("status should handle");

        assert_eq!(
            reply.text,
            "This room is not connected to any projects.\nUse `/bind /absolute/project/path` in this room."
        );
    }

    #[test]
    fn unbind_project_disconnects_existing_binding() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::BindProject {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("bind should handle");

        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::UnbindProject {
                project_path: Some(path.clone()),
            }),
            test_time(),
        )
        .expect("unbind should handle");

        assert_eq!(reply.text, format!("Disconnected this room from:\n{path}"));
        assert!(ledger.connected_rooms_for_project(&path).is_empty());
    }

    #[test]
    fn unbind_without_path_disconnects_all_room_projects() {
        let first = tempfile::tempdir().expect("temp dir should exist");
        let second = tempfile::tempdir().expect("temp dir should exist");
        let first_path = first.path().to_string_lossy().to_string();
        let second_path = second.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        for path in [&first_path, &second_path] {
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::BindProject {
                    project_path: path.clone(),
                }),
                test_time(),
            )
            .expect("bind should handle");
        }

        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::UnbindProject { project_path: None }),
            test_time(),
        )
        .expect("unbind should handle");

        assert_eq!(
            reply.text,
            format!("Disconnected this room from:\n- {first_path}\n- {second_path}")
        );
        assert!(ledger.connected_rooms_for_project(&first_path).is_empty());
        assert!(ledger.connected_rooms_for_project(&second_path).is_empty());
    }

    #[test]
    fn help_lists_minimal_control_commands() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::Help),
            test_time(),
        )
        .expect("help should handle");

        assert!(reply.text.contains("`/bind /absolute/project/path`"));
        assert!(reply.text.contains("`/status`"));
        assert!(reply.text.contains("`/unbind`"));
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
