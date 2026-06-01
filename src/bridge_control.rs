use std::path::Path;

use chrono::{DateTime, Utc};

use crate::bridge_binding_ledger::{
    BridgeBindingLedger, OwnerDirectChatInput, ProjectRoomProposalInput, RoomBindingQuery,
    RoomProjectBindingInput, RoomProjectBindingRecord,
};
use crate::config::is_clean_absolute_project_path;
use crate::provider_inbound::{
    NormalizedProviderControlCommand, NormalizedProviderRoomEvent, NormalizedProviderSurfaceReply,
    ProviderControlCommand, ProviderRoomEvent,
};
use crate::response_surface_ledger::provider_event_id_hash;

const NEW_WITH_PROJECT_COMMAND_HINT: &str = "`/new /path/to/project what you want Codex to do`";
const DIRECT_BIND_COMMAND_HINT: &str = "`/bind /path/to/project`";
const ROOM_BIND_COMMAND_HINT: &str = "`@your-bot /bind /path/to/project`";
const ROOM_BIND_COMMAND_EXAMPLE: &str = "`@Agents Router /bind /Users/alex/Projects/my-app`";
const ROOM_NEW_COMMAND_HINT: &str = "`@your-bot /new what you want Codex to do`";
const ROOM_NEW_WITH_PROJECT_COMMAND_HINT: &str =
    "`@your-bot /new /path/to/project what you want Codex to do`";
const ROOM_UNBIND_PROJECT_COMMAND_HINT: &str = "`@your-bot /unbind /path/to/project`";
const ROOM_MENTION_NOTE: &str =
    "Use Lark's @ menu to select me. Do not type the @ name as plain text.";

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
pub(crate) struct BridgeRoomMessage {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_event_id_hash: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeProjectRoomPrompt {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub provider_event_id_hash: String,
    pub proposal_id: String,
    pub project_path: String,
    pub project_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BridgeControlOutcome {
    Reply(BridgeControlReply),
    RoomMessage(BridgeRoomMessage),
    ProjectRoomPrompt(BridgeProjectRoomPrompt),
    NewSession(BridgeNewSessionCommand),
}

pub(crate) fn handle_provider_room_event(
    event: NormalizedProviderRoomEvent,
) -> anyhow::Result<BridgeControlOutcome> {
    let provider_event_id_hash = provider_event_id_hash(
        &event.provider_type,
        &event.provider_id,
        &event.provider_account_id,
        &event.provider_conversation_id,
        &event.provider_event_id,
    );
    let text = match event.event {
        ProviderRoomEvent::BotAddedToChat => room_welcome_text(),
    };

    Ok(BridgeControlOutcome::RoomMessage(BridgeRoomMessage {
        provider_id: event.provider_id,
        provider_type: event.provider_type,
        provider_account_id: event.provider_account_id,
        provider_conversation_id: event.provider_conversation_id,
        provider_event_id_hash,
        text,
    }))
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

    if is_direct_chat_command(&command.command) {
        ledger.remember_owner_direct_chat_at(
            OwnerDirectChatInput {
                provider_id: command.provider_id.clone(),
                provider_type: command.provider_type.clone(),
                provider_account_id: command.provider_account_id.clone(),
                provider_conversation_id: command.provider_conversation_id.clone(),
            },
            now,
        )?;
    }

    let outcome = match &command.command {
        ProviderControlCommand::DirectProjectRoomPrompt { project_path } => {
            direct_project_room_prompt(
                ledger,
                &command,
                &provider_event_id_hash,
                project_path,
                now,
            )?
        }
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
            &command,
            &provider_event_id_hash,
            project_path.as_deref(),
            prompt,
        )?,
        ProviderControlCommand::DirectStatus => reply_outcome(
            &command,
            provider_event_id_hash,
            direct_chat_status(ledger, &command)?,
        ),
        ProviderControlCommand::DirectUnbindProjectGuidance => reply_outcome(
            &command,
            provider_event_id_hash,
            direct_unbind_project_guidance_text(),
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
        ProviderControlCommand::RoomGuidance => reply_outcome(
            &command,
            provider_event_id_hash,
            room_guidance_text(ledger, &command),
        ),
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

fn is_direct_chat_command(command: &ProviderControlCommand) -> bool {
    matches!(
        command,
        ProviderControlCommand::DirectProjectRoomPrompt { .. }
            | ProviderControlCommand::DirectChatGuidance
            | ProviderControlCommand::DirectHelp
            | ProviderControlCommand::DirectNewSession { .. }
            | ProviderControlCommand::DirectStatus
            | ProviderControlCommand::DirectUnbindProjectGuidance
    )
}

pub(crate) fn disconnected_lark_thread_reply(
    reply: NormalizedProviderSurfaceReply,
) -> BridgeControlReply {
    let provider_event_id_hash = provider_event_id_hash(
        &reply.provider_type,
        &reply.provider_id,
        &reply.provider_account_id,
        &reply.provider_conversation_id,
        &reply.provider_event_id,
    );

    BridgeControlReply {
        provider_id: reply.provider_id,
        provider_type: reply.provider_type,
        provider_account_id: reply.provider_account_id,
        provider_conversation_id: reply.provider_conversation_id,
        provider_thread_id: reply.provider_thread_id,
        provider_event_id_hash,
        text: disconnected_lark_thread_text(),
    }
}

fn bind_project(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    project_path: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    if !is_clean_absolute_project_path(project_path) {
        return Ok(format!(
            "Use the full path to a folder on your Mac.\n\nSend the full message like this:\n{ROOM_BIND_COMMAND_HINT}\n\nExample:\n{ROOM_BIND_COMMAND_EXAMPLE}\n\n{ROOM_MENTION_NOTE}"
        ));
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

    Ok(format!(
        "Done. This room is connected to:\n{project_path}\n\nNew Codex updates for this folder will appear here."
    ))
}

fn direct_project_room_prompt(
    ledger: &mut BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
    provider_event_id_hash: &str,
    project_path: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<BridgeControlOutcome> {
    if !is_clean_absolute_project_path(project_path) {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            format!(
                "`/bind` connects a project to a group room.\n\nUse the full folder path on this Mac:\n`/bind /path/to/project`\n\nTo start a new Codex session instead, use:\n{NEW_WITH_PROJECT_COMMAND_HINT}"
            ),
        ));
    }
    if !Path::new(project_path).is_dir() {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            format!("I can't find that folder on this Mac:\n{project_path}"),
        ));
    }

    let record = ledger.begin_manual_project_room_proposal_prompt_at(
        ProjectRoomProposalInput {
            provider_id: command.provider_id.clone(),
            provider_type: command.provider_type.clone(),
            project_path: project_path.to_string(),
        },
        now,
    )?;

    Ok(BridgeControlOutcome::ProjectRoomPrompt(
        BridgeProjectRoomPrompt {
            provider_id: command.provider_id.clone(),
            provider_type: command.provider_type.clone(),
            provider_account_id: command.provider_account_id.clone(),
            provider_conversation_id: command.provider_conversation_id.clone(),
            provider_thread_id: command.provider_thread_id.clone(),
            provider_event_id_hash: provider_event_id_hash.to_string(),
            proposal_id: record.proposal_id,
            project_path: record.project_path,
            project_name: record.project_name,
        },
    ))
}

fn direct_unbind_project_guidance_text() -> String {
    [
        "Direct chat does not bind a project folder.",
        "",
        "To start a new Codex session from direct chat, include the folder in `/new`:",
        NEW_WITH_PROJECT_COMMAND_HINT,
        "",
        "To disconnect a group room, use `/unbind` in that room.",
    ]
    .join("\n")
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
        return Ok(format!(
            "Use {ROOM_UNBIND_PROJECT_COMMAND_HINT} to disconnect one folder, or `@your-bot /unbind` to disconnect everything from this room.\n\n{ROOM_MENTION_NOTE}"
        ));
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
            format!(
                "Tell Codex what to do.\n\nExample:\n{ROOM_NEW_COMMAND_HINT}\n\n{ROOM_MENTION_NOTE}"
            ),
        ));
    }

    let connected = ledger.connected_projects_for_room(&room_query(command));
    let project_path = match explicit_project_path {
        Some(project_path) => {
            if !is_clean_absolute_project_path(project_path) {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    format!(
                        "Use the full path to a connected folder.\n\nSend the full message like this:\n{ROOM_NEW_WITH_PROJECT_COMMAND_HINT}\n\n{ROOM_MENTION_NOTE}"
                    ),
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
                        "This room is not connected to:\n{project_path}\n\nTo connect it, go back to the room itself and send:\n`@your-bot /bind {project_path}`\n\n{ROOM_MENTION_NOTE}"
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
                    format!(
                        "This room is not connected to a project folder yet.\n\nSend the full message like this:\n{ROOM_BIND_COMMAND_HINT}\n\nExample:\n{ROOM_BIND_COMMAND_EXAMPLE}\n\n{ROOM_MENTION_NOTE}"
                    ),
                ));
            }
            [project] => project.project_path.clone(),
            projects => {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    format!(
                        "This room is connected to multiple project folders.\n\nChoose one when you start a new Codex task:\n{ROOM_NEW_WITH_PROJECT_COMMAND_HINT}\n\n{ROOM_MENTION_NOTE}\n\nConnected folders:\n{}",
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
            format!("Tell Codex what to do.\n\nExample:\n{NEW_WITH_PROJECT_COMMAND_HINT}"),
        ));
    }

    let project_path = match explicit_project_path {
        Some(project_path) => {
            if !is_clean_absolute_project_path(project_path) {
                return Ok(reply_outcome(
                    command,
                    provider_event_id_hash.to_string(),
                    format!(
                        "Use the full path to a folder on your Mac.\n\nFormat:\n{NEW_WITH_PROJECT_COMMAND_HINT}"
                    ),
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
        None => None,
    };

    let Some(project_path) = project_path else {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            direct_chat_needs_project_text(),
        ));
    };

    if !Path::new(&project_path).is_dir() {
        return Ok(reply_outcome(
            command,
            provider_event_id_hash.to_string(),
            format!(
                "I can't find that folder on this Mac:\n{project_path}\n\nStart again with:\n{NEW_WITH_PROJECT_COMMAND_HINT}"
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

fn direct_chat_needs_project_text() -> String {
    [
        "Tell me which folder Codex should use.",
        "",
        "Send:",
        NEW_WITH_PROJECT_COMMAND_HINT,
        "",
        "Example:",
        "`/new /Users/alex/Projects/my-app fix the failing test`",
    ]
    .join("\n")
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
        return format!(
            "{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n\n{}",
            "This room is not connected yet.",
            "To bring Codex updates here, send the full message like this:",
            ROOM_BIND_COMMAND_HINT,
            "Example:",
            ROOM_BIND_COMMAND_EXAMPLE,
            ROOM_MENTION_NOTE,
            "After that, updates from that folder will appear in this room.",
        );
    }

    let project_count = connected.len();
    let new_session_hint = if project_count == 1 {
        ROOM_NEW_COMMAND_HINT
    } else {
        ROOM_NEW_WITH_PROJECT_COMMAND_HINT
    };
    let connected_label = if project_count == 1 {
        "This room is connected to 1 project folder:"
    } else {
        "This room is connected to these project folders:"
    };
    let start_label = if project_count == 1 {
        "Start a new Codex task from this room:"
    } else {
        "Start a new Codex task from one folder:"
    };

    [
        connected_label.to_string(),
        format_project_list(&connected),
        "".to_string(),
        start_label.to_string(),
        new_session_hint.to_string(),
        "".to_string(),
        ROOM_MENTION_NOTE.to_string(),
        "".to_string(),
        "When a Codex update appears here, reply in its Lark thread and use Lark's @ menu to mention me if you want Codex to continue that task.".to_string(),
    ]
    .join("\n")
}

fn direct_chat_status(
    ledger: &BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
) -> anyhow::Result<String> {
    let connected = ledger.connected_room_project_bindings_for_provider_account(
        &command.provider_id,
        &command.provider_type,
        &command.provider_account_id,
    )?;
    if connected.is_empty() {
        return Ok([
            "Direct chat is ready.",
            "",
            "Start a new Codex session by choosing a folder in `/new`:",
            NEW_WITH_PROJECT_COMMAND_HINT,
            "",
            "Example:",
            "`/new /Users/alex/Projects/my-app fix the failing test`",
            "",
            "To connect a project to a Lark room, open the project room menu:",
            DIRECT_BIND_COMMAND_HINT,
        ]
        .join("\n"));
    }

    let mut lines = vec!["Direct chat is ready".to_string(), "".to_string()];
    lines.push("Start a new Codex session by choosing a folder in `/new`:".to_string());
    lines.push(NEW_WITH_PROJECT_COMMAND_HINT.to_string());
    lines.push("".to_string());
    lines.push(format!("Folders connected in rooms: {}", connected.len()));
    lines.push(format_project_list(&connected));
    lines.push("".to_string());
    lines.push("Those room bindings do not become a direct chat default.".to_string());
    lines.push("".to_string());
    lines.push("To connect another project to a room, use:".to_string());
    lines.push(DIRECT_BIND_COMMAND_HINT.to_string());
    Ok(lines.join("\n"))
}

fn help_text() -> String {
    [
        "I help this room work with Codex on your Mac.",
        "",
        "First, connect a project folder. Send the full message like this:",
        ROOM_BIND_COMMAND_HINT,
        "",
        "Example:",
        ROOM_BIND_COMMAND_EXAMPLE,
        "",
        ROOM_MENTION_NOTE,
        "",
        "After that, Codex updates for that folder will appear here.",
        "",
        "Common commands",
        ROOM_NEW_COMMAND_HINT,
        "Start a new Codex task from this room.",
        "",
        "`@your-bot /status`",
        "Show which folders are connected.",
        "",
        "`@your-bot /unbind`",
        "Disconnect all folders from this room.",
        "",
        "More commands",
        ROOM_NEW_WITH_PROJECT_COMMAND_HINT,
        "Use this when the room has more than one connected folder.",
        "",
        ROOM_UNBIND_PROJECT_COMMAND_HINT,
        "Disconnect one folder from this room.",
        "",
        "In Lark threads, use Lark's @ menu to mention me only when you want Codex to continue that task.",
        "Use `/bind`, `/new`, and `/unbind` in the room itself, not inside a Lark thread.",
    ]
    .join("\n")
}

fn direct_help_text() -> String {
    [
        "Chat with Codex on your Mac.",
        "",
        "No mention is needed in direct chat.",
        "",
        "Start a new Codex session by choosing a folder in `/new`:",
        NEW_WITH_PROJECT_COMMAND_HINT,
        "",
        "Example:",
        "`/new /Users/alex/Projects/my-app fix the failing test`",
        "",
        "Common commands",
        "`/help`",
        "Show this help.",
        "",
        "`/status`",
        "Show direct chat guidance and connected project rooms.",
        "",
        DIRECT_BIND_COMMAND_HINT,
        "Open the project room menu for a folder.",
        "",
        "You can also add me to a group and connect that room directly:",
        ROOM_BIND_COMMAND_HINT,
        "",
        ROOM_MENTION_NOTE,
    ]
    .join("\n")
}

fn direct_chat_guidance_text() -> String {
    [
        "Direct chat is ready.",
        "",
        "Start a new Codex session by choosing a folder in `/new`:",
        NEW_WITH_PROJECT_COMMAND_HINT,
        "",
        "Example:",
        "`/new /Users/alex/Projects/my-app fix the failing test`",
        "",
        "To connect a project to a Lark room, open the project room menu:",
        DIRECT_BIND_COMMAND_HINT,
    ]
    .join("\n")
}

fn room_welcome_text() -> String {
    [
        "Hi, I can bring Codex updates into this room.",
        "",
        "To get started, connect this room to a project folder on your Mac. Send the full message like this:",
        ROOM_BIND_COMMAND_HINT,
        "",
        "Example:",
        ROOM_BIND_COMMAND_EXAMPLE,
        "",
        ROOM_MENTION_NOTE,
        "",
        "After that, updates from that folder will appear here.",
        "",
        "To start a Codex task from this room later, send:",
        ROOM_NEW_COMMAND_HINT,
    ]
    .join("\n")
}

fn room_guidance_text(
    ledger: &BridgeBindingLedger,
    command: &NormalizedProviderControlCommand,
) -> String {
    let connected = ledger.connected_projects_for_room(&room_query(command));
    if connected.is_empty() {
        return [
            "I can help once this room is connected to a project folder.",
            "",
            "Send the full message like this:",
            ROOM_BIND_COMMAND_HINT,
            "",
            "Example:",
            ROOM_BIND_COMMAND_EXAMPLE,
            "",
            ROOM_MENTION_NOTE,
        ]
        .join("\n");
    }

    let new_session_hint = if connected.len() == 1 {
        ROOM_NEW_COMMAND_HINT
    } else {
        ROOM_NEW_WITH_PROJECT_COMMAND_HINT
    };
    [
        "This room is ready for Codex.",
        "",
        "To start a new Codex task, send the full message like this:",
        new_session_hint,
        "",
        ROOM_MENTION_NOTE,
        "",
        "When a Codex update appears here, reply in its Lark thread and use Lark's @ menu to mention me if you want Codex to continue that task.",
    ]
    .join("\n")
}

fn disconnected_lark_thread_text() -> String {
    [
        "This Lark thread is no longer connected to Codex.",
        "",
        "Go back to the room and start a new task instead:",
        ROOM_NEW_COMMAND_HINT,
        "",
        ROOM_MENTION_NOTE,
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

        assert_eq!(
            reply,
            format!(
                "Done. This room is connected to:\n{path}\n\nNew Codex updates for this folder will appear here."
            )
        );
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
                "This room is connected to 1 project folder:\n- {path}\n\nStart a new Codex task from this room:\n`@your-bot /new what you want Codex to do`\n\nUse Lark's @ menu to select me. Do not type the @ name as plain text.\n\nWhen a Codex update appears here, reply in its Lark thread and use Lark's @ menu to mention me if you want Codex to continue that task."
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
            "This room is not connected yet.\n\nTo bring Codex updates here, send the full message like this:\n`@your-bot /bind /path/to/project`\n\nExample:\n`@Agents Router /bind /Users/alex/Projects/my-app`\n\nUse Lark's @ menu to select me. Do not type the @ name as plain text.\n\nAfter that, updates from that folder will appear in this room."
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

        assert!(reply.contains("connected to these project folders"));
        assert!(reply.contains(&first_path));
        assert!(reply.contains(&second_path));
        assert!(reply.contains("`@your-bot /new /path/to/project what you want Codex to do`"));
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

        assert!(reply.contains("`@your-bot /bind /path/to/project`"));
        assert!(reply.contains("`@your-bot /new what you want Codex to do`"));
        assert!(reply.contains("`@your-bot /status`"));
        assert!(reply.contains("`@your-bot /unbind`"));
        assert!(reply.contains("Use Lark's @ menu to select me"));
        assert!(reply.contains("room itself, not inside a Lark thread"));
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

        assert!(reply.contains("Chat with Codex on your Mac."));
        assert!(reply.contains("`/new /path/to/project what you want Codex to do`"));
        assert!(reply.contains("Start a new Codex session"));
        assert!(reply.contains("Open the project room menu for a folder."));
        assert!(reply.contains("You can also add me to a group"));
        assert!(reply.contains("`@your-bot /bind /path/to/project`"));
        assert!(reply.contains("Use Lark's @ menu to select me"));
    }

    #[test]
    fn direct_bind_project_opens_project_room_prompt() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        let outcome = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectProjectRoomPrompt {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("direct bind should handle");

        let BridgeControlOutcome::ProjectRoomPrompt(prompt) = outcome else {
            panic!("direct bind should dispatch project room prompt");
        };
        assert_eq!(prompt.project_path, path);
        assert_eq!(prompt.provider_conversation_id, "room-1");
        assert_eq!(prompt.provider_thread_id, "message-1");
        assert_eq!(
            prompt.project_name,
            dir.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        assert!(
            ledger
                .lookup_project_room_proposal(&prompt.proposal_id)
                .expect("proposal lookup should load")
                .is_some()
        );
        let owner = ledger
            .owner_direct_chat_for_provider("lark-personal-agent", "feishu_lark")
            .expect("owner direct chat lookup should load")
            .expect("direct command should remember the owner direct chat");
        assert_eq!(owner.provider_conversation_id, "room-1");
    }

    #[test]
    fn direct_bind_project_reopens_ignored_project_room_prompt() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().to_string_lossy().to_string();
        let mut ledger = BridgeBindingLedger::in_memory();
        let first = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectProjectRoomPrompt {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("direct bind should handle");
        let BridgeControlOutcome::ProjectRoomPrompt(first_prompt) = first else {
            panic!("first bind should dispatch project room prompt");
        };
        ledger
            .ignore_project_room_proposal_at(&first_prompt.proposal_id, test_time())
            .expect("proposal should ignore");

        let reopened = handle_provider_control_command(
            &mut ledger,
            command(ProviderControlCommand::DirectProjectRoomPrompt {
                project_path: path.clone(),
            }),
            test_time(),
        )
        .expect("direct bind should reopen");
        let BridgeControlOutcome::ProjectRoomPrompt(reopened_prompt) = reopened else {
            panic!("manual bind should reopen project room prompt");
        };

        assert_eq!(reopened_prompt.proposal_id, first_prompt.proposal_id);
        assert_eq!(
            ledger
                .lookup_project_room_proposal(&reopened_prompt.proposal_id)
                .expect("proposal lookup should load")
                .expect("proposal should exist")
                .status,
            crate::bridge_binding_ledger::ProjectRoomProposalStatus::Pending
        );
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

        assert!(reply.contains("Direct chat is ready"));
        assert!(reply.contains("Start a new Codex session by choosing a folder in `/new`"));
        assert!(reply.contains("Folders connected in rooms: 1"));
        assert!(reply.contains(&path));
    }

    #[test]
    fn direct_status_without_room_bindings_points_to_new_with_project() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::DirectStatus),
                test_time(),
            )
            .expect("direct status should handle"),
        );

        assert!(reply.contains("Direct chat is ready"));
        assert!(reply.contains("`/new /path/to/project what you want Codex to do`"));
        assert!(reply.contains("To connect a project to a Lark room"));
        assert!(reply.contains("`/bind /path/to/project`"));
    }

    #[test]
    fn direct_new_session_without_project_asks_for_path() {
        let mut ledger = BridgeBindingLedger::in_memory();
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

        assert!(reply.contains("Tell me which folder Codex should use."));
        assert!(reply.contains("`/new /path/to/project what you want Codex to do`"));
    }

    #[test]
    fn direct_new_session_still_requires_project_when_project_rooms_exist() {
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

        assert!(reply.contains("Tell me which folder Codex should use."));
        assert!(reply.contains("`/new /path/to/project what you want Codex to do`"));
        assert!(!reply.contains(&first_path));
        assert!(!reply.contains(&second_path));
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

        assert!(reply.contains("Start a new Codex session by choosing a folder in `/new`"));
        assert!(reply.contains("`/new /path/to/project what you want Codex to do`"));
        assert!(reply.contains("To connect a project to a Lark room"));
        assert!(reply.contains("`/bind /path/to/project`"));
    }

    #[test]
    fn room_guidance_explains_bind_before_project_is_connected() {
        let mut ledger = BridgeBindingLedger::in_memory();
        let reply = reply_text(
            handle_provider_control_command(
                &mut ledger,
                command(ProviderControlCommand::RoomGuidance),
                test_time(),
            )
            .expect("room guidance should handle"),
        );

        assert!(reply.contains("connected to a project folder"));
        assert!(reply.contains("Send the full message like this"));
        assert!(reply.contains("`@your-bot /bind /path/to/project`"));
        assert!(reply.contains("Use Lark's @ menu to select me"));
    }

    #[test]
    fn room_event_welcomes_new_project_room() {
        let outcome = handle_provider_room_event(room_event(ProviderRoomEvent::BotAddedToChat))
            .expect("room event should handle");

        let BridgeControlOutcome::RoomMessage(message) = outcome else {
            panic!("room event should send a room message");
        };
        assert_eq!(message.provider_conversation_id, "room-1");
        assert!(message.text.contains("Hi, I can bring Codex updates"));
        assert!(message.text.contains("`@your-bot /bind /path/to/project`"));
        assert!(message.text.contains("Use Lark's @ menu to select me"));
        assert!(
            message
                .text
                .contains("`@your-bot /new what you want Codex to do`")
        );
    }

    #[test]
    fn disconnected_lark_thread_reply_explains_how_to_start_over() {
        let reply = disconnected_lark_thread_reply(surface_reply());

        assert_eq!(reply.provider_thread_id, "message-1");
        assert!(reply.text.contains("no longer connected to Codex"));
        assert!(
            reply
                .text
                .contains("Go back to the room and start a new task")
        );
        assert!(
            reply
                .text
                .contains("`@your-bot /new what you want Codex to do`")
        );
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

        assert!(reply.contains("This room is connected to multiple project folders."));
        assert!(reply.contains(&first_path));
        assert!(reply.contains(&second_path));
    }

    fn reply_text(outcome: BridgeControlOutcome) -> String {
        match outcome {
            BridgeControlOutcome::Reply(reply) => reply.text,
            BridgeControlOutcome::RoomMessage(_) => panic!("expected reply outcome"),
            BridgeControlOutcome::ProjectRoomPrompt(_) => panic!("expected reply outcome"),
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

    fn room_event(event: ProviderRoomEvent) -> NormalizedProviderRoomEvent {
        NormalizedProviderRoomEvent {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: "room-1".to_string(),
            provider_event_id: "event-1".to_string(),
            event,
        }
    }

    fn surface_reply() -> NormalizedProviderSurfaceReply {
        NormalizedProviderSurfaceReply {
            provider_id: "lark-personal-agent".to_string(),
            provider_type: "feishu_lark".to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: "tenant-1".to_string(),
            provider_conversation_id: "room-1".to_string(),
            provider_thread_id: "message-1".to_string(),
            provider_event_id: "event-1".to_string(),
            provider_reply_message_id: Some("event-1".to_string()),
            reply_text: "continue".to_string(),
        }
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 31, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }
}
