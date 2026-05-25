use std::fs::File;
use std::io::Write;

use tempfile::tempdir;

use crate::config::{AnswerDetail, PromptDetail, SourceType, ValidatedConfig};
use crate::delivery::{DeliveryError, DeliveryErrorContext, DeliveryErrorKind};
use crate::router::{Provider, ProviderFuture};

use super::*;

#[test]
fn first_run_skips_existing_rollout_events_then_emits_new_events() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(
        &rollout_path,
        &[
            session_meta_line("session-1"),
            task_complete_line("turn-old", "2026-05-09T17:00:00.000Z", "old"),
        ],
    );
    write_lines(
            &index_path,
            &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
        );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    let first = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("first poll should pass");
    assert!(first.signals.is_empty());
    watcher.commit(first).expect("first batch should commit");

    append_line(&rollout_path, &turn_context_line("gpt-5.5"));
    append_line(
        &rollout_path,
        &task_complete_line("turn-new", "2026-05-09T17:01:32.000Z", "new result"),
    );
    let second = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("second poll should pass");

    assert_eq!(second.signals.len(), 1);
    assert_eq!(
        second.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.turn_id.as_deref()),
        Some("turn-new")
    );
    assert_eq!(
        second.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.model.as_deref()),
        Some("gpt-5.5")
    );
    assert_eq!(
        second.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.answer.as_ref())
            .map(|answer| answer.content.as_str()),
        Some("new result")
    );
}

#[test]
fn restart_skips_known_file_backlog_then_emits_runtime_append() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&rollout_path, &[session_meta_line("session-1")]);
    write_lines(
        &index_path,
        &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
    );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .and_then(|batch| watcher.commit(batch))
        .expect("initial poll should pass");

    append_line(
        &rollout_path,
        &task_complete_line("turn-downtime", "2026-05-09T17:01:32.000Z", "missed"),
    );

    let mut restarted = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher restart should baseline downtime backlog");
    let after_restart = restarted
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("post-restart poll should pass");
    assert!(
        after_restart.signals.is_empty(),
        "Codex Desktop source must not replay completions written while service was down"
    );
    restarted
        .commit(after_restart)
        .expect("post-restart baseline should commit");

    append_line(&rollout_path, &turn_context_line("gpt-5.5"));
    append_line(
        &rollout_path,
        &task_complete_line("turn-runtime", "2026-05-09T17:02:32.000Z", "new result"),
    );
    let runtime_batch = restarted
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("runtime poll should pass");

    assert_eq!(runtime_batch.signals.len(), 1);
    assert_eq!(
        runtime_batch.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.turn_id.as_deref()),
        Some("turn-runtime")
    );
    assert_eq!(
        runtime_batch.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.answer.as_ref())
            .map(|answer| answer.content.as_str()),
        Some("new result")
    );
}

#[test]
fn restart_skips_new_file_created_while_stopped_then_emits_runtime_append() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let first_rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let second_rollout_path = day_dir.join("rollout-2026-05-10T02-00-00-session-2.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&first_rollout_path, &[session_meta_line("session-1")]);
    write_lines(
        &index_path,
        &[
            r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string(),
            r#"{"id":"session-2","thread_name":"agents-router runtime report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string(),
        ],
    );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .and_then(|batch| watcher.commit(batch))
        .expect("initial poll should pass");

    write_lines(
        &second_rollout_path,
        &[
            session_meta_line("session-2"),
            task_complete_line("turn-downtime", "2026-05-09T17:01:32.000Z", "missed"),
        ],
    );

    let mut restarted = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher restart should baseline downtime file");
    let after_restart = restarted
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("post-restart poll should pass");
    assert!(
        after_restart.signals.is_empty(),
        "Codex Desktop source must not replay session files created while service was down"
    );
    restarted
        .commit(after_restart)
        .expect("post-restart baseline should commit");

    append_line(
        &second_rollout_path,
        &task_complete_line("turn-runtime", "2026-05-09T17:02:32.000Z", "new result"),
    );
    let runtime_batch = restarted
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("runtime poll should pass");

    assert_eq!(runtime_batch.signals.len(), 1);
    assert_eq!(
        runtime_batch.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.turn_id.as_deref()),
        Some("turn-runtime")
    );
    assert_eq!(
        runtime_batch.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.session_id.as_deref()),
        Some("session-2")
    );
}

#[test]
fn prompt_detail_on_attaches_prompt_without_persisting_it() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&rollout_path, &[session_meta_line("session-1")]);
    write_lines(
            &index_path,
            &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
        );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    watcher
        .poll(
            &watcher_config(AnswerDetail::Full, PromptDetail::On),
            &source_config(),
        )
        .and_then(|batch| watcher.commit(batch))
        .expect("first poll should pass");

    append_line(&rollout_path, &user_message_line("Please fix the route."));
    append_line(
        &rollout_path,
        &task_complete_line("turn-new", "2026-05-09T17:01:32.000Z", "fixed"),
    );
    let batch = watcher
        .poll(
            &watcher_config(AnswerDetail::Full, PromptDetail::On),
            &source_config(),
        )
        .expect("second poll should pass");

    assert_eq!(batch.signals.len(), 1);
    let conversation = batch.signals[0]
        .conversation
        .as_ref()
        .expect("conversation should be set");
    assert_eq!(
        conversation.prompt.as_deref(),
        Some("Please fix the route.")
    );
    assert_eq!(
        conversation
            .answer
            .as_ref()
            .map(|answer| answer.content.as_str()),
        Some("fixed")
    );
    watcher.commit(batch).expect("second batch should commit");

    let state = fs::read_to_string(&state_path).expect("state file should exist");
    assert!(!state.contains("Please fix the route."));
}

#[test]
fn partial_rollout_line_is_not_checkpointed() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&rollout_path, &[session_meta_line("session-1")]);
    write_lines(
            &index_path,
            &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
        );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    let first = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("first poll should pass");
    assert!(first.signals.is_empty());
    watcher.commit(first).expect("first batch should commit");

    let complete_line = task_complete_line("turn-new", "2026-05-09T17:01:32.000Z", "new result");
    let split_at = complete_line
        .find(r#""last_agent_message""#)
        .expect("fixture should contain last_agent_message");
    append_raw(&rollout_path, &complete_line[..split_at]);
    let partial = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("partial poll should pass");
    assert!(partial.signals.is_empty());
    watcher
        .commit(partial)
        .expect("partial batch should commit");

    append_raw(&rollout_path, &complete_line[split_at..]);
    append_raw(&rollout_path, "\n");
    let completed = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("completed poll should pass");

    assert_eq!(completed.signals.len(), 1);
    assert_eq!(
        completed.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.turn_id.as_deref()),
        Some("turn-new")
    );
}

#[test]
fn bootstrap_does_not_checkpoint_partial_rollout_line() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&rollout_path, &[session_meta_line("session-1")]);
    write_lines(
            &index_path,
            &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
        );

    let complete_line = task_complete_line("turn-new", "2026-05-09T17:01:32.000Z", "new result");
    let split_at = complete_line
        .find(r#""last_agent_message""#)
        .expect("fixture should contain last_agent_message");
    append_raw(&rollout_path, &complete_line[..split_at]);

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    let partial = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("partial poll should pass");
    assert!(partial.signals.is_empty());
    watcher
        .commit(partial)
        .expect("partial batch should commit");

    append_raw(&rollout_path, &complete_line[split_at..]);
    append_raw(&rollout_path, "\n");
    let completed = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("completed poll should pass");

    assert_eq!(completed.signals.len(), 1);
    assert_eq!(
        completed.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.turn_id.as_deref()),
        Some("turn-new")
    );
}

#[test]
fn uncommitted_poll_retries_task_complete() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&rollout_path, &[session_meta_line("session-1")]);
    write_lines(
            &index_path,
            &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
        );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .and_then(|batch| watcher.commit(batch))
        .expect("first poll should pass");

    append_line(
        &rollout_path,
        &task_complete_line("turn-new", "2026-05-09T17:01:32.000Z", "new result"),
    );
    let first_attempt = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("first task poll should pass");
    let retry_attempt = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("retry task poll should pass");

    assert_eq!(first_attempt.signals.len(), 1);
    assert_eq!(retry_attempt.signals.len(), 1);
    assert_eq!(
        retry_attempt.signals[0]
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.turn_id.as_deref()),
        Some("turn-new")
    );

    watcher
        .commit(retry_attempt)
        .expect("successful retry should commit");
    let after_commit = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("post-commit poll should pass");
    assert!(after_commit.signals.is_empty());
}

#[tokio::test]
async fn provider_failure_does_not_reopen_codex_desktop_rollout_events() {
    let dir = tempdir().expect("tempdir should be created");
    let sessions_dir = dir.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("05").join("10");
    fs::create_dir_all(&day_dir).expect("session dir should be created");
    let rollout_path = day_dir.join("rollout-2026-05-10T01-00-00-session-1.jsonl");
    let index_path = dir.path().join("session_index.jsonl");
    let state_path = dir.path().join("source-state.json");

    write_lines(&rollout_path, &[session_meta_line("session-1")]);
    write_lines(
        &index_path,
        &[r#"{"id":"session-1","thread_name":"agents-router sync report","updated_at":"2026-05-09T17:00:00Z"}"#.to_string()],
    );

    let mut watcher = CodexDesktopSessionWatcher::new(
        sessions_dir.clone(),
        index_path.clone(),
        state_path.clone(),
    )
    .expect("watcher should start");
    watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .and_then(|batch| watcher.commit(batch))
        .expect("first poll should pass");

    append_line(
        &rollout_path,
        &task_complete_line("turn-new", "2026-05-09T17:01:32.000Z", "new result"),
    );
    let batch = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("task poll should pass");
    assert_eq!(batch.signals.len(), 1);

    let provider = FailingProvider;
    let providers: Vec<&dyn Provider> = vec![&provider];
    route_and_checkpoint_batch(
        &mut watcher,
        &routing_config(),
        &providers,
        batch,
        None,
        None,
    )
    .await
    .expect("provider failure should not fail checkpointing");

    let after_failure = watcher
        .poll(
            &watcher_config(AnswerDetail::Preview, PromptDetail::Off),
            &source_config(),
        )
        .expect("post-failure poll should pass");
    assert!(
        after_failure.signals.is_empty(),
        "provider failure must not cause the same Codex Desktop completion to be emitted again"
    );
}

struct FailingProvider;

impl Provider for FailingProvider {
    fn id(&self) -> &str {
        "failing"
    }

    fn provider_type(&self) -> &str {
        "test"
    }

    fn send<'a>(&'a self, signal: &'a Signal) -> ProviderFuture<'a> {
        Box::pin(async move {
            Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, self.id(), self.provider_type()),
                "send failed",
            ))
        })
    }
}

fn source_config() -> SourceConfig {
    SourceConfig {
        id: "codex_desktop".to_string(),
        source_type: SourceType::CodexDesktop,
    }
}

fn routing_config() -> ValidatedConfig {
    watcher_config(AnswerDetail::Preview, PromptDetail::Off)
}

fn watcher_config(answer_detail: AnswerDetail, prompt_detail: PromptDetail) -> ValidatedConfig {
    let answer_detail = match answer_detail {
        AnswerDetail::Preview => "preview",
        AnswerDetail::Full => "full",
    };
    let prompt_detail = match prompt_detail {
        PromptDetail::Off => "off",
        PromptDetail::On => "on",
    };
    ValidatedConfig::from_toml_str(&format!(
        r#"
schema_version = 1

[notification]
answer_detail = "{answer_detail}"
prompt_detail = "{prompt_detail}"

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "failing"
type = "webhook"
url = "http://127.0.0.1:9/unused"

[[routes]]
sources = ["codex_desktop"]
providers = ["failing"]
"#,
    ))
    .expect("watcher config should be valid")
}

fn session_meta_line(session_id: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-05-09T16:59:00.000Z","type":"session_meta","payload":{{"id":"{session_id}","timestamp":"2026-05-09T16:59:00.000Z","cwd":"/Users/tester/projects/agents-router","originator":"Codex Desktop","cli_version":"0.130.0-alpha.5","source":"vscode","model_provider":"openai","git":{{"branch":"main","commit_hash":"abc123","repository_url":"https://example.com/repo.git"}}}}}}"#
    )
}

fn task_complete_line(turn_id: &str, timestamp: &str, last_agent_message: &str) -> String {
    format!(
        r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"task_complete","turn_id":"{turn_id}","completed_at":1778348142,"duration_ms":92185,"time_to_first_token_ms":1200,"last_agent_message":"{last_agent_message}"}}}}"#
    )
}

fn turn_context_line(model: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-05-09T17:01:20.000Z","type":"turn_context","payload":{{"model":"{model}","collaboration_mode":{{"settings":{{"model":"{model}"}}}}}}}}"#
    )
}

fn user_message_line(message: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-05-09T17:01:00.000Z","type":"event_msg","payload":{{"type":"user_message","message":"{message}","images":[]}}}}"#
    )
}

fn write_lines(path: &Path, lines: &[String]) {
    let mut file = File::create(path).expect("file should be created");
    for line in lines {
        writeln!(file, "{line}").expect("line should be written");
    }
}

fn append_line(path: &Path, line: &str) {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("file should open");
    writeln!(file, "{line}").expect("line should be appended");
}

fn append_raw(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("file should open");
    write!(file, "{text}").expect("text should be appended");
}
