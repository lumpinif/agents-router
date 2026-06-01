use std::path::Path;

use chrono::{DateTime, Utc};

use crate::bridge_binding_ledger::{
    BridgeBindingLedger, DirectChatBindingQuery, DirectChatProjectBindingInput, RoomBindingQuery,
    RoomProjectBindingInput, RoomProjectBindingRecord,
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
        ProviderControlCommand::DirectBindProject { project_path } => reply_outcome(
            &command,
            provider_event_id_hash,
            bind_direct_chat_project(ledger, &command, project_path, now)?,
        ),
        ProviderControlCommand::DirectChatGuidance => reply_outcome(
            &command,
            provider_event_id_hash,
            direct_chat_guidance_text(),
        ),
        ProviderControlCommand::DirectHelp => {
            reply_outcome(&command, provider_event_id_hash, direct_help_text())
        }
        ProviderControlCommand::DirectNewSession {
            project_path,
            prompt,
        } => resolve_direct_new_session_command(
            ledger,
            &command,
            &provider_event_id_hash,
            project_path.as_deref(),
            prompt,
            now,
        )?,
        ProviderControlCommand::DirectStatus => reply_outcome(
            &command,
            provider_event_id_hash,
            direct_chat_status(ledger, &command)?,
        ),
        ProviderControlCommand::DirectUnbindProject => reply_outcome(
            &command,
            provider_event_id_hash,
            unbind_direct_chat_project(ledger, &command, now)?,
        ),
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
        return Ok(
            "Use a clean absolute folder path:\n`/bind /absolute/project/path`.".to_string(),
        );
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

fn bind_direct_chat_project(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    project_path: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    if !is_clean_absolute_project_path(project_path) {
        return Ok(
            "Set this direct chat's default project with:\n`/bind /absolute/project/path`."
                .to_string(),
        );
    }
    if !Path::new(project_path).is_dir() {
        return Ok(format!(
            "I can't find that folder on this Mac:\n{project_path}"
        ));
    }

    ledger.connect_direct_chat_project_at(
        DirectChatProjectBindingInput {
            provider_id: command.provider_id.clone(),
            provider_type: command.provider_type.clone(),
            provider_account_id: command.provider_account_id.clone(),
            provider_conversation_id: command.provider_conversation_id.clone(),
            project_path: project_path.to_string(),
        },
        now,
    )?;

    Ok(format!("Direct chat will use:\n{project_path}"))
}

fn unbind_direct_chat_project(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    let disconnected =
        ledger.disconnect_direct_chat_project_at(&direct_chat_query(command), now)?;
    if let Some(disconnected) = disconnected {
        Ok(format!(
            "Direct chat no longer uses:\n{}",
            disconnected.project_path
        ))
    } else {
        Ok("Direct chat is not connected to a project.".to_string())
    }
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
            "Tell Codex what to do:\n`/new what you want Codex to do`.".to_string(),
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
                        "This room is not connected to:\n{project_path}\n\nIn the main room, mention me and send:\n`/bind {project_path}`"
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
                    "This room is not connected to a project yet.\n\nIn the main room, mention me and send:\n`/bind /absolute/project/path`.".to_string(),
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

    Ok(new_session_outcome(
        command,
        provider_event_id_hash,
        project_path,
        prompt,
    ))
}

fn resolve_direct_new_session_command(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    provider_event_id_hash: &str,
    explicit_project_path: Option<&str>,
    prompt: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<BridgeControlOutcome> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            "Tell Codex what to do:\n`/new what you want Codex to do`.".to_string(),
        ));
    }

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
            Some(project_path.to_string())
        }
        None => direct_chat_project_path_or_reply(ledger, command, now)?,
    };

    let Some(project_path) = project_path else {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            direct_chat_needs_project_text(ledger, command)?,
        ));
    };

    if !Path::new(&project_path).is_dir() {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            format!(
                "I can't find the direct chat project on this Mac:\n{project_path}\n\nSet a new project with `/bind /absolute/project/path`."
            ),
        ));
    }

    Ok(new_session_outcome(
        command,
        provider_event_id_hash,
        project_path,
        prompt,
    ))
}

fn direct_chat_project_path_or_reply(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<String>> {
    if let Some(record) = ledger.connected_project_for_direct_chat(&direct_chat_query(command))? {
        return Ok(Some(record.project_path));
    }

    let connected = unique_connected_projects_for_provider_account(ledger, command)?;
    if connected.len() == 1 {
        let project_path = connected[0].clone();
        ledger.connect_direct_chat_project_at(
            DirectChatProjectBindingInput {
                provider_id: command.provider_id.clone(),
                provider_type: command.provider_type.clone(),
                provider_account_id: command.provider_account_id.clone(),
                provider_conversation_id: command.provider_conversation_id.clone(),
                project_path: project_path.clone(),
            },
            now,
        )?;
        return Ok(Some(project_path));
    }

    Ok(None)
}

fn direct_chat_needs_project_text(
    ledger: &BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
) -> anyhow::Result<String> {
    let connected = unique_connected_projects_for_provider_account(ledger, command)?;
    if connected.is_empty() {
        return Ok([
            "Set a default project for this direct chat first:",
            "`/bind /absolute/project/path`",
            "",
            "After that, any plain message here starts a new Codex thread in that project.",
        ]
        .join("\n"));
    }

    let mut lines = vec![
        "I found more than one connected project room.".to_string(),
        "Set the default project for this direct chat:".to_string(),
        "`/bind /absolute/project/path`".to_string(),
        "".to_string(),
        "Connected project rooms:".to_string(),
    ];
    lines.extend(
        connected
            .iter()
            .map(|project_path| format!("- {project_path}")),
    );
    Ok(lines.join("\n"))
}

fn new_session_outcome(
    command: &NormalizedProviderControlCommand,
    provider_event_id_hash: &str,
    project_path: String,
    prompt: &str,
) -> BridgeControlOutcome {
    BridgeControlOutcome::NewSession(BridgeNewSessionCommand {
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
    })
}

fn room_status(ledger: &BridgeBindingLedger, command: &NormalizedProviderControlCommand) -> String {
    let connected = ledger.connected_projects_for_room(&room_query(command));
    if connected.is_empty() {
        return [
            "Room status",
            "",
            "No projects are connected yet.",
            "",
            "In the main room, mention me and send:",
            "`/bind /absolute/project/path`",
            "",
            "After that, new Codex Desktop updates from that project will appear here. `/new ...` can start a new Codex thread in that project.",
        ]
        .join("\n");
    }

    let project_count = connected.len();
    let new_session_hint = if project_count == 1 {
        "`/new what you want Codex to do`"
    } else {
        "`/new /absolute/project/path what you want Codex to do`"
    };

    [
        "Room status".to_string(),
        "".to_string(),
        format!("Connected projects: {project_count}"),
        format_project_list(&connected),
        "".to_string(),
        format!("Start a new Codex thread:\n{new_session_hint}"),
        "".to_string(),
        "When a Codex update appears here, open its Lark thread, mention me, and reply there to continue the same Codex thread.".to_string(),
    ]
    .join("\n")
}

fn direct_chat_status(
    ledger: &BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
) -> anyhow::Result<String> {
    let direct_project = ledger.connected_project_for_direct_chat(&direct_chat_query(command))?;
    let connected = ledger.connected_room_project_bindings_for_provider_account(
        &command.provider_id,
        &command.provider_type,
        &command.provider_account_id,
    )?;
    if direct_project.is_none() && connected.is_empty() {
        return Ok([
            "Direct chat status",
            "",
            "No default project is set.",
            "",
            "Set a default project for this direct chat:",
            "`/bind /absolute/project/path`",
            "",
            "After that, any plain message here starts a new Codex thread in that project.",
        ]
        .join("\n"));
    }

    let mut lines = vec!["Direct chat status".to_string(), "".to_string()];
    if let Some(project) = direct_project {
        lines.push("Default project:".to_string());
        lines.push(format!("- {}", project.project_path));
        lines.push("".to_string());
        lines.push("Any plain message here starts a new Codex thread in that project.".to_string());
    } else {
        lines.push("No default project is set.".to_string());
        lines.push("Set one with `/bind /absolute/project/path`.".to_string());
    }
    if !connected.is_empty() {
        lines.push("".to_string());
        lines.push(format!("Connected project rooms: {}", connected.len()));
        lines.push(format_project_list(&connected));
    }
    Ok(lines.join("\n"))
}

fn help_text() -> String {
    [
        "I connect this room to local Codex projects on your Mac.",
        "",
        "Commands",
        "`/bind /absolute/project/path`",
        "Connect this room to a local project.",
        "",
        "`/new what you want Codex to do`",
        "Start a new Codex thread in the connected project.",
        "",
        "`/new /absolute/project/path what you want Codex to do`",
        "Start in one connected project when this room has multiple projects.",
        "",
        "`/status`",
        "Show what this room is connected to.",
        "",
        "`/unbind /absolute/project/path`",
        "Disconnect one project from this room.",
        "",
        "`/unbind`",
        "Disconnect all projects from this room.",
        "",
        "For shared rooms: mention me before each command.",
        "For Lark threads: mention me only when you want to continue that Codex thread.",
        "Run `/bind`, `/new`, and `/unbind` in the main room, not inside a thread.",
    ]
    .join("\n")
}

fn direct_help_text() -> String {
    [
        "Direct chat with local Codex on your Mac.",
        "",
        "No mention is needed here.",
        "",
        "Direct chat",
        "`/help`",
        "Show this help.",
        "",
        "`/status`",
        "Show this direct chat's default project and connected project rooms.",
        "",
        "`/bind /absolute/project/path`",
        "Set the default project for this direct chat.",
        "",
        "Any plain message",
        "Start a new Codex thread in the default project.",
        "",
        "`/new what you want Codex to do`",
        "Start a new Codex thread in the default project.",
        "",
        "`/new /absolute/project/path what you want Codex to do`",
        "Start a new Codex thread in a one-off project.",
        "",
        "Project rooms",
        "Add me to a group and send:",
        "`/bind /absolute/project/path`",
        "",
        "After that, new Codex Desktop updates from that project will appear in the group, and `/new` can start a new Codex thread there.",
        "",
        "In shared rooms and Lark threads, mention me before commands or replies.",
    ]
    .join("\n")
}

fn direct_chat_guidance_text() -> String {
    [
        "Direct chat is ready.",
        "",
        "Set a default project first:",
        "`/bind /absolute/project/path`",
        "",
        "After that, any plain message here starts a new Codex thread in that project.",
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

fn direct_chat_query(command: &NormalizedProviderControlCommand) -> DirectChatBindingQuery {
    DirectChatBindingQuery {
        provider_id: command.provider_id.clone(),
        provider_type: command.provider_type.clone(),
        provider_account_id: command.provider_account_id.clone(),
        provider_conversation_id: command.provider_conversation_id.clone(),
    }
}

fn unique_connected_projects_for_provider_account(
    ledger: &BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
) -> anyhow::Result<Vec<String>> {
    let mut project_paths = ledger
        .connected_room_project_bindings_for_provider_account(
            &command.provider_id,
            &command.provider_type,
            &command.provider_account_id,
        )?
        .into_iter()
        .map(|record| record.project_path)
        .collect::<Vec<_>>();
    project_paths.sort();
    project_paths.dedup();
    Ok(project_paths)
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

        assert_eq!(
            reply,
            format!(
                "Room status\n\nConnected projects: 1\n- {path}\n\nStart a new Codex thread:\n`/new what you want Codex to do`\n\nWhen a Codex update appears here, open its Lark thread, mention me, and reply there to continue the same Codex thread."
            )
        );
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
            "Room status\n\nNo projects are connected yet.\n\nIn the main room, mention me and send:\n`/bind /absolute/project/path`\n\nAfter that, new Codex Desktop updates from that project will appear here. `/new ...` can start a new Codex thread in that project."
        );
    }

    #[test]
    fn status_reports_explicit_new_command_for_multiple_projects() {
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
                command(ProviderControlCommand::Status),
                test_time(),
            )
            .expect("status should handle"),
        );

        assert!(reply.contains("Connected projects: 2"));
        assert!(reply.contains(&first_path));
        assert!(reply.contains(&second_path));
        assert!(reply.contains("`/new /absolute/project/path what you want Codex to do`"));
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
        assert!(reply.contains("mention me before each command"));
        assert!(reply.contains("main room, not inside a thread"));
    }

    #[test]
    fn direct_help_explains_private_chat_and_project_rooms() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::DirectHelp),
                test_time(),
            )
            .expect("direct help should handle"),
        );

        assert!(reply.contains("Direct chat"));
        assert!(reply.contains("`/bind /absolute/project/path`"));
        assert!(reply.contains("Any plain message"));
        assert!(reply.contains("`/new what you want Codex to do`"));
        assert!(reply.contains("`/new /absolute/project/path what you want Codex to do`"));
        assert!(reply.contains("Project rooms"));
        assert!(reply.contains("mention me before commands or replies"));
    }

    #[test]
    fn direct_bind_project_sets_direct_chat_default_project() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::DirectBindProject {
                    project_path: path.clone(),
                }),
                test_time(),
            )
            .expect("direct bind should handle"),
        );

        assert_eq!(reply, format!("Direct chat will use:\n{path}"));
        let binding = ledger
            .connected_project_for_direct_chat(&direct_chat_query(&command(
                ProviderControlCommand::DirectStatus,
            )))
            .expect("direct chat binding should query")
            .expect("direct chat should have a default project");
        assert_eq!(binding.project_path, path);
    }

    #[test]
    fn direct_status_lists_connected_project_rooms() {
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
                command(ProviderControlCommand::DirectStatus),
                test_time(),
            )
            .expect("direct status should handle"),
        );

        assert!(reply.contains("Direct chat status"));
        assert!(reply.contains("No default project is set."));
        assert!(reply.contains("Connected project rooms: 1"));
        assert!(reply.contains(&path));
    }

    #[test]
    fn direct_status_reports_direct_chat_default_project() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectBindProject {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("direct bind should handle");

        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::DirectStatus),
                test_time(),
            )
            .expect("direct status should handle"),
        );

        assert!(reply.contains("Default project:"));
        assert!(reply.contains(&path));
        assert!(reply.contains("Any plain message here starts a new Codex thread"));
    }

    #[test]
    fn direct_new_session_uses_direct_chat_default_project() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectBindProject {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("direct bind should handle");

        let outcome = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectNewSession {
                project_path: None,
                prompt: "What is the time?".to_string(),
            }),
            test_time(),
        )
        .expect("direct new should handle");

        let BridgeControlOutcome::NewSession(new_session) = outcome else {
            panic!("direct new session should dispatch work");
        };
        assert_eq!(new_session.project_path, path);
        assert_eq!(new_session.prompt, "What is the time?");
    }

    #[test]
    fn direct_new_session_adopts_the_only_connected_project_room() {
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
        .expect("room bind should handle");

        let outcome = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectNewSession {
                project_path: None,
                prompt: "What is the time?".to_string(),
            }),
            test_time(),
        )
        .expect("direct new should handle");

        let BridgeControlOutcome::NewSession(new_session) = outcome else {
            panic!("direct new session should dispatch work");
        };
        assert_eq!(new_session.project_path, path);
        assert!(
            ledger
                .connected_project_for_direct_chat(&direct_chat_query(&command(
                    ProviderControlCommand::DirectStatus,
                )))
                .expect("direct chat binding should query")
                .is_some()
        );
    }

    #[test]
    fn direct_new_session_asks_for_project_when_multiple_project_rooms_exist() {
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
            .expect("room bind should handle");
        }

        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::DirectNewSession {
                    project_path: None,
                    prompt: "What is the time?".to_string(),
                }),
                test_time(),
            )
            .expect("direct new should handle"),
        );

        assert!(reply.contains("I found more than one connected project room"));
        assert!(reply.contains("Set the default project for this direct chat"));
        assert!(reply.contains(&first_path));
        assert!(reply.contains(&second_path));
    }

    #[test]
    fn direct_new_session_uses_explicit_project_without_room_binding() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();

        let outcome = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectNewSession {
                project_path: Some(path.clone()),
                prompt: "Reply OK.".to_string(),
            }),
            test_time(),
        )
        .expect("direct new should handle");

        let BridgeControlOutcome::NewSession(command) = outcome else {
            panic!("direct new session should dispatch work");
        };
        assert_eq!(command.project_path, path);
        assert_eq!(command.prompt, "Reply OK.");
        assert_eq!(command.source_id, "codex_desktop");
        assert_eq!(command.source_type, "codex_desktop");
    }

    #[test]
    fn direct_chat_guidance_points_to_new_and_bind() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::DirectChatGuidance),
                test_time(),
            )
            .expect("direct guidance should handle"),
        );

        assert!(reply.contains("Set a default project first"));
        assert!(reply.contains("`/bind /absolute/project/path`"));
        assert!(reply.contains("any plain message here"));
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
