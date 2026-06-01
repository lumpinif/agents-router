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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeNewSessionCommand {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub provider_event_id_hash: String,
    pub project_path: String,
    pub prompt: String,
    pub source_id: String,
    pub source_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BridgeControlOutcome {
    Reply(BridgeControlReply),
    NewSession(BridgeNewSessionCommand),
}

pub(crate) fn handle_provider_control_command(
    ledger: &mut BridgeBindingLedger,
    command: NormalizedProviderControlCommand,
    now: DateTime<Utc>,
) -> anyhow::Result<BridgeControlOutcome> {
    let provider_event_id_hash = provider_event_id_hash(
        &command.provider_type,
        &command.provider_id,
        &command.provider_account_id,
        &command.provider_conversation_id,
        &command.provider_event_id,
    );

    let outcome = match &command.command {
        ProviderControlCommand::NewSession {
            project_path,
            prompt,
        } => resolve_new_session_command(
            ledger,
            &command,
            &provider_event_id_hash,
            project_path.as_deref(),
            prompt,
        )?,
        ProviderControlCommand::BindProject { project_path } => reply_outcome(
            &command,
            provider_event_id_hash,
            bind_project(ledger, &command, project_path, now)?,
        ),
        ProviderControlCommand::Help => {
            reply_outcome(&command, provider_event_id_hash, help_text())
        }
        ProviderControlCommand::Status => reply_outcome(
            &command,
            provider_event_id_hash,
            room_status(ledger, &command),
        ),
        ProviderControlCommand::UnbindProject { project_path } => reply_outcome(
            &command,
            provider_event_id_hash,
            unbind_project(ledger, &command, project_path.as_deref(), now)?,
        ),
        ProviderControlCommand::Invalid { message } => {
            reply_outcome(&command, provider_event_id_hash, message.clone())
        }
    };

    Ok(outcome)
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

fn resolve_new_session_command(
    ledger: &BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    provider_event_id_hash: &str,
    explicit_project_path: Option<&str>,
    prompt: &str,
) -> anyhow::Result<BridgeControlOutcome> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            "Use `/new what you want Codex to do`.".to_string(),
        ));
    }

    let connected = ledger.connected_projects_for_room(&room_query(command));
    let project_path = match explicit_project_path {
        Some(project_path) => {
            if !is_clean_absolute_project_path(project_path) {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    "Use `/new /absolute/project/path what you want Codex to do`.".to_string(),
                ));
            }
            if !Path::new(project_path).is_dir() {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    format!("I can't find that folder on this Mac:\n{project_path}"),
                ));
            }
            if !connected
                .iter()
                .any(|record| record.project_path == project_path)
            {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    format!(
                        "This room is not connected to:\n{project_path}\n\nUse `/bind {project_path}` first."
                    ),
                ));
            }
            project_path.to_string()
        }
        None => match connected.as_slice() {
            [] => {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    "Connect this room first with `/bind /absolute/project/path`.".to_string(),
                ));
            }
            [project] => project.project_path.clone(),
            projects => {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    format!(
                        "This room is connected to multiple projects.\nUse `/new /absolute/project/path what you want Codex to do`.\n\nConnected projects:\n{}",
                        format_project_list(projects)
                    ),
                ));
            }
        },
    };

    Ok(BridgeControlOutcome::NewSession(BridgeNewSessionCommand {
        provider_id: command.provider_id.clone(),
        provider_type: command.provider_type.clone(),
        provider_account_id: command.provider_account_id.clone(),
        provider_conversation_id: command.provider_conversation_id.clone(),
        provider_thread_id: command.provider_thread_id.clone(),
        provider_event_id_hash: provider_event_id_hash.to_string(),
        project_path,
        prompt: prompt.to_string(),
        source_id: "codex_desktop".to_string(),
        source_type: "codex_desktop".to_string(),
    }))
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
        "`/new what you want Codex to do` - start a new Codex thread in the connected project.",
        "`/new /absolute/project/path what you want Codex to do` - start in one connected project when this room has multiple projects.",
        "`/status` - show connected projects for this room.",
        "`/unbind /absolute/project/path` - disconnect one project.",
        "`/unbind` - disconnect all projects from this room.",
        "",
        "In groups and threads, mention me before the command.",
    ]
    .join("\n")
}

fn reply_outcome(
    command: &NormalizedProviderControlCommand,
    provider_event_id_hash: String,
    text: String,
) -> BridgeControlOutcome {
    BridgeControlOutcome::Reply(BridgeControlReply {
        provider_id: command.provider_id.clone(),
        provider_type: command.provider_type.clone(),
        provider_account_id: command.provider_account_id.clone(),
        provider_conversation_id: command.provider_conversation_id.clone(),
        provider_thread_id: command.provider_thread_id.clone(),
        provider_event_id_hash,
        text,
    })
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
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::BindProject {
                    project_path: path.clone(),
                }),
                test_time(),
            )
            .expect("bind should handle"),
        );

        assert_eq!(reply, format!("Connected this room to:\n{path}"));
        assert_eq!(ledger.connected_rooms_for_project(&path).len(), 1);
    }

    #[test]
    fn bind_project_reports_missing_folder_without_recording_binding() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::BindProject {
                    project_path: "/definitely/missing/agents-router".to_string(),
                }),
                test_time(),
            )
            .expect("bind should handle"),
        );

        assert_eq!(
            reply,
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

        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::Status),
                test_time(),
            )
            .expect("status should handle"),
        );

        assert_eq!(reply, format!("This room is connected to:\n- {path}"));
    }

    #[test]
    fn status_reports_empty_room() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::Status),
                test_time(),
            )
            .expect("status should handle"),
        );

        assert_eq!(
            reply,
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

        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::UnbindProject {
                    project_path: Some(path.clone()),
                }),
                test_time(),
            )
            .expect("unbind should handle"),
        );

        assert_eq!(reply, format!("Disconnected this room from:\n{path}"));
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

        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::UnbindProject { project_path: None }),
                test_time(),
            )
            .expect("unbind should handle"),
        );

        assert_eq!(
            reply,
            format!("Disconnected this room from:\n- {first_path}\n- {second_path}")
        );
        assert!(ledger.connected_rooms_for_project(&first_path).is_empty());
        assert!(ledger.connected_rooms_for_project(&second_path).is_empty());
    }

    #[test]
    fn help_lists_minimal_control_commands() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::Help),
                test_time(),
            )
            .expect("help should handle"),
        );

        assert!(reply.contains("`/bind /absolute/project/path`"));
        assert!(reply.contains("`/new what you want Codex to do`"));
        assert!(reply.contains("`/status`"));
        assert!(reply.contains("`/unbind`"));
    }

    #[test]
    fn new_session_uses_the_only_connected_project() {
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

        let outcome = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::NewSession {
                project_path: None,
                prompt: "Reply OK.".to_string(),
            }),
            test_time(),
        )
        .expect("new should handle");

        let BridgeControlOutcome::NewSession(command) = outcome else {
            panic!("new session should dispatch work");
        };
        assert_eq!(command.project_path, path);
        assert_eq!(command.prompt, "Reply OK.");
        assert_eq!(command.source_id, "codex_desktop");
        assert_eq!(command.source_type, "codex_desktop");
    }

    #[test]
    fn new_session_requires_explicit_project_when_room_has_multiple_projects() {
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

        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::NewSession {
                    project_path: None,
                    prompt: "Reply OK.".to_string(),
                }),
                test_time(),
            )
            .expect("new should handle"),
        );

        assert!(reply.contains("This room is connected to multiple projects."));
        assert!(reply.contains(&first_path));
        assert!(reply.contains(&second_path));
    }

    fn reply_text(outcome: BridgeControlOutcome) -> String {
        match outcome {
            BridgeControlOutcome::Reply(reply) => reply.text,
            BridgeControlOutcome::NewSession(_) => panic!("expected reply outcome"),
        }
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
