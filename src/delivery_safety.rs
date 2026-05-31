use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::delivery_safety_state_path;
use crate::signal::{Signal, SignalAnswerKind, SignalEventKind};

const STATE_SCHEMA_VERSION: u32 = 1;
const DUPLICATE_WINDOW: Duration = Duration::seconds(10);
const DUPLICATE_THRESHOLD: usize = 5;
const GLOBAL_PAUSE_WINDOW: Duration = Duration::seconds(120);
const GLOBAL_PAUSE_THRESHOLD: usize = 3;

#[derive(Clone)]
pub struct DeliverySafetyGuard {
    inner: Arc<Mutex<DeliverySafetyInner>>,
    state_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct DeliverySafetyInner {
    delivery_windows: HashMap<String, VecDeque<DateTime<Utc>>>,
    suppressed_messages: HashMap<String, DateTime<Utc>>,
    global_pause: Option<GlobalDeliveryPause>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalDeliveryPause {
    pub paused_at: DateTime<Utc>,
    pub reason: String,
    pub message_fingerprint_count: usize,
    pub window_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverySafetyStatus {
    pub paused: bool,
    pub pause: Option<GlobalDeliveryPause>,
}

pub struct DeliveryAttempt<'a> {
    pub signal: &'a Signal,
    pub provider_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliverySafetyDecision {
    Allow {
        message_fingerprint_hash: String,
        delivery_key_hash: String,
    },
    Suppress(DeliverySuppression),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverySuppression {
    pub reason: DeliverySuppressionReason,
    pub message_fingerprint_hash: String,
    pub delivery_key_hash: String,
    pub window_seconds: u64,
    pub count: usize,
    pub global_paused: bool,
    pub pause_started: Option<DeliveryPauseStarted>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliverySuppressionReason {
    DuplicateStorm,
    GlobalPaused,
}

impl DeliverySuppressionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateStorm => "duplicate_storm",
            Self::GlobalPaused => "global_paused",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryPauseStarted {
    pub message_fingerprint_count: usize,
    pub window_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeliverySafetyStateFile {
    schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    global_pause: Option<GlobalDeliveryPause>,
}

impl DeliverySafetyGuard {
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(DeliverySafetyInner::default())),
            state_path: None,
        }
    }

    pub fn load_default() -> anyhow::Result<Self> {
        Self::load(delivery_safety_state_path()?)
    }

    pub fn load(state_path: PathBuf) -> anyhow::Result<Self> {
        let global_pause = load_state_file(&state_path)?.global_pause;
        Ok(Self {
            inner: Arc::new(Mutex::new(DeliverySafetyInner {
                global_pause,
                ..DeliverySafetyInner::default()
            })),
            state_path: Some(state_path),
        })
    }

    pub fn check(&self, attempt: DeliveryAttempt<'_>) -> anyhow::Result<DeliverySafetyDecision> {
        self.check_at(attempt, Utc::now())
    }

    pub fn check_for_provider_target(
        &self,
        attempt: DeliveryAttempt<'_>,
        provider_target_id: &str,
    ) -> anyhow::Result<DeliverySafetyDecision> {
        self.check_at_for_provider_target(attempt, provider_target_id, Utc::now())
    }

    pub fn reset(&self) -> anyhow::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("delivery safety state lock is poisoned"))?;
        inner.delivery_windows.clear();
        inner.suppressed_messages.clear();
        inner.global_pause = None;
        self.save_locked(&inner)
    }

    pub fn status(&self) -> anyhow::Result<DeliverySafetyStatus> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("delivery safety state lock is poisoned"))?;
        Ok(status_from_pause(inner.global_pause.clone()))
    }

    pub fn reset_default_file() -> anyhow::Result<()> {
        save_state_file(
            &delivery_safety_state_path()?,
            &DeliverySafetyStateFile {
                schema_version: STATE_SCHEMA_VERSION,
                global_pause: None,
            },
        )
    }

    pub fn default_file_status() -> anyhow::Result<DeliverySafetyStatus> {
        Ok(status_from_pause(
            load_state_file(&delivery_safety_state_path()?)?.global_pause,
        ))
    }

    fn check_at(
        &self,
        attempt: DeliveryAttempt<'_>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<DeliverySafetyDecision> {
        self.check_at_inner(attempt, now, None)
    }

    fn check_at_for_provider_target(
        &self,
        attempt: DeliveryAttempt<'_>,
        provider_target_id: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<DeliverySafetyDecision> {
        anyhow::ensure!(
            !provider_target_id.trim().is_empty()
                && provider_target_id.trim() == provider_target_id,
            "provider_target_id must be present and trimmed"
        );
        self.check_at_inner(attempt, now, Some(provider_target_id))
    }

    fn check_at_inner(
        &self,
        attempt: DeliveryAttempt<'_>,
        now: DateTime<Utc>,
        provider_target_id: Option<&str>,
    ) -> anyhow::Result<DeliverySafetyDecision> {
        let message_fingerprint_hash = message_fingerprint_hash(attempt.signal);
        let delivery_key_hash = delivery_key_hash(
            attempt.provider_id,
            provider_target_id,
            &message_fingerprint_hash,
        );
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("delivery safety state lock is poisoned"))?;

        if inner.global_pause.is_some() {
            return Ok(DeliverySafetyDecision::Suppress(DeliverySuppression {
                reason: DeliverySuppressionReason::GlobalPaused,
                message_fingerprint_hash,
                delivery_key_hash,
                window_seconds: 0,
                count: 0,
                global_paused: true,
                pause_started: None,
            }));
        }

        let cutoff = now - DUPLICATE_WINDOW;
        prune_delivery_windows(&mut inner.delivery_windows, cutoff);
        let window = inner
            .delivery_windows
            .entry(delivery_key_hash.clone())
            .or_default();
        while window.front().is_some_and(|timestamp| *timestamp < cutoff) {
            window.pop_front();
        }
        window.push_back(now);
        let count = window.len();

        if count < DUPLICATE_THRESHOLD {
            return Ok(DeliverySafetyDecision::Allow {
                message_fingerprint_hash,
                delivery_key_hash,
            });
        }

        let pause_started = record_suppressed_message(&mut inner, &message_fingerprint_hash, now);
        if pause_started.is_some() {
            self.save_locked(&inner)?;
        }

        Ok(DeliverySafetyDecision::Suppress(DeliverySuppression {
            reason: DeliverySuppressionReason::DuplicateStorm,
            message_fingerprint_hash,
            delivery_key_hash,
            window_seconds: DUPLICATE_WINDOW.num_seconds() as u64,
            count,
            global_paused: inner.global_pause.is_some(),
            pause_started,
        }))
    }

    fn save_locked(&self, inner: &DeliverySafetyInner) -> anyhow::Result<()> {
        let Some(state_path) = &self.state_path else {
            return Ok(());
        };

        save_state_file(
            state_path,
            &DeliverySafetyStateFile {
                schema_version: STATE_SCHEMA_VERSION,
                global_pause: inner.global_pause.clone(),
            },
        )
    }
}

fn record_suppressed_message(
    inner: &mut DeliverySafetyInner,
    message_fingerprint_hash: &str,
    now: DateTime<Utc>,
) -> Option<DeliveryPauseStarted> {
    let cutoff = now - GLOBAL_PAUSE_WINDOW;
    inner
        .suppressed_messages
        .retain(|_, timestamp| *timestamp >= cutoff);
    inner
        .suppressed_messages
        .insert(message_fingerprint_hash.to_string(), now);

    if inner.global_pause.is_none() && inner.suppressed_messages.len() >= GLOBAL_PAUSE_THRESHOLD {
        let pause = GlobalDeliveryPause {
            paused_at: now,
            reason: "multiple_duplicate_storms".to_string(),
            message_fingerprint_count: inner.suppressed_messages.len(),
            window_seconds: GLOBAL_PAUSE_WINDOW.num_seconds() as u64,
        };
        let started = DeliveryPauseStarted {
            message_fingerprint_count: pause.message_fingerprint_count,
            window_seconds: pause.window_seconds,
        };
        inner.global_pause = Some(pause);
        Some(started)
    } else {
        None
    }
}

fn load_state_file(path: &Path) -> anyhow::Result<DeliverySafetyStateFile> {
    if !path.exists() {
        return Ok(DeliverySafetyStateFile {
            schema_version: STATE_SCHEMA_VERSION,
            global_pause: None,
        });
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read delivery safety state `{}`", path.display()))?;
    let state: DeliverySafetyStateFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse delivery safety state `{}`", path.display()))?;
    if state.schema_version != STATE_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported delivery safety state schema version `{}` in `{}`",
            state.schema_version,
            path.display()
        );
    }
    Ok(state)
}

fn save_state_file(path: &Path, state: &DeliverySafetyStateFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create delivery safety state directory `{}`",
                parent.display()
            )
        })?;
    }

    let raw =
        serde_json::to_vec_pretty(state).context("failed to serialize delivery safety state")?;
    let temp_path = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ));
    fs::write(&temp_path, raw).with_context(|| {
        format!(
            "failed to write delivery safety state `{}`",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace delivery safety state `{}`",
            path.display()
        )
    })?;
    Ok(())
}

fn status_from_pause(global_pause: Option<GlobalDeliveryPause>) -> DeliverySafetyStatus {
    DeliverySafetyStatus {
        paused: global_pause.is_some(),
        pause: global_pause,
    }
}

fn message_fingerprint_hash(signal: &Signal) -> String {
    let conversation = signal.conversation.as_ref();
    let workspace = signal.workspace.as_ref();
    let answer = conversation.and_then(|conversation| conversation.answer.as_ref());
    let answer_kind = answer
        .map(|answer| answer_kind_key(answer.kind))
        .unwrap_or("none");
    let answer_hash = answer
        .map(|answer| stable_hash(&answer.content))
        .unwrap_or_else(|| "none".to_string());
    let display_hash = if answer.is_none() {
        stable_hash(&format!(
            "title={}\nsummary={}\n",
            signal.display.title, signal.display.summary
        ))
    } else {
        "structured".to_string()
    };
    let canonical = format!(
        "v=1\nsource.id={}\nsource.type={}\nevent.kind={}\nevent.raw={}\nsession.id={}\nturn.id={}\nproject.path={}\nanswer.kind={}\nanswer.hash={}\ndisplay.hash={}\n",
        signal.source_id(),
        signal.source_type(),
        event_kind_key(&signal.event.kind),
        signal.event.raw_name.as_deref().unwrap_or(""),
        conversation
            .and_then(|conversation| conversation.session_id.as_deref())
            .unwrap_or(""),
        conversation
            .and_then(|conversation| conversation.turn_id.as_deref())
            .unwrap_or(""),
        workspace
            .and_then(|workspace| workspace.project_path.as_deref())
            .unwrap_or(""),
        answer_kind,
        answer_hash,
        display_hash,
    );
    stable_hash(&canonical)
}

fn delivery_key_hash(
    provider_id: &str,
    provider_target_id: Option<&str>,
    message_fingerprint_hash: &str,
) -> String {
    stable_hash(&format!(
        "v=2\nprovider.id={provider_id}\nprovider.target_id={}\nmessage.fingerprint={message_fingerprint_hash}\n",
        provider_target_id.unwrap_or("")
    ))
}

fn stable_hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn event_kind_key(kind: &SignalEventKind) -> &'static str {
    match kind {
        SignalEventKind::TurnCompleted => "turn_completed",
        SignalEventKind::Custom => "custom",
    }
}

fn prune_delivery_windows(
    delivery_windows: &mut HashMap<String, VecDeque<DateTime<Utc>>>,
    cutoff: DateTime<Utc>,
) {
    delivery_windows.retain(|_, window| {
        while window.front().is_some_and(|timestamp| *timestamp < cutoff) {
            window.pop_front();
        }
        !window.is_empty()
    });
}

fn answer_kind_key(kind: SignalAnswerKind) -> &'static str {
    match kind {
        SignalAnswerKind::Preview => "preview",
        SignalAnswerKind::Full => "full",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::signal::{
        SignalAnswer, SignalAnswerKind, SignalConversation, SignalDisplay, SignalEvent,
        SignalEventKind, SignalSource, SignalWorkspace,
    };
    use tempfile::tempdir;

    #[test]
    fn suppresses_same_delivery_key_on_fifth_attempt_within_ten_seconds() {
        let guard = DeliverySafetyGuard::in_memory();
        let signal = test_signal("signal-1", "session-1", "turn-1");
        let now = timestamp(0);

        for second in 0..4 {
            assert!(matches!(
                guard
                    .check_at(
                        DeliveryAttempt::new(&signal, "work_chat"),
                        now + Duration::seconds(second)
                    )
                    .expect("guard should allow"),
                DeliverySafetyDecision::Allow { .. }
            ));
        }

        let decision = guard
            .check_at(
                DeliveryAttempt::new(&signal, "work_chat"),
                now + Duration::seconds(4),
            )
            .expect("guard should suppress");

        assert!(matches!(
            decision,
            DeliverySafetyDecision::Suppress(DeliverySuppression {
                reason: DeliverySuppressionReason::DuplicateStorm,
                count: 5,
                ..
            })
        ));
    }

    #[test]
    fn duplicate_window_expires() {
        let guard = DeliverySafetyGuard::in_memory();
        let signal = test_signal("signal-1", "session-1", "turn-1");
        let now = timestamp(0);

        for second in 0..4 {
            guard
                .check_at(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    now + Duration::seconds(second),
                )
                .expect("guard should allow");
        }

        let decision = guard
            .check_at(
                DeliveryAttempt::new(&signal, "work_chat"),
                now + Duration::seconds(15),
            )
            .expect("guard should allow after window");

        assert!(matches!(decision, DeliverySafetyDecision::Allow { .. }));
    }

    #[test]
    fn provider_delivery_keys_are_counted_independently() {
        let guard = DeliverySafetyGuard::in_memory();
        let signal = test_signal("signal-1", "session-1", "turn-1");
        let now = timestamp(0);

        for second in 0..4 {
            guard
                .check_at(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    now + Duration::seconds(second),
                )
                .expect("first provider should allow");
        }

        let decision = guard
            .check_at(
                DeliveryAttempt::new(&signal, "debug_webhook"),
                now + Duration::seconds(4),
            )
            .expect("second provider should have a separate count");

        assert!(matches!(decision, DeliverySafetyDecision::Allow { .. }));
    }

    #[test]
    fn provider_target_delivery_keys_are_counted_independently() {
        let guard = DeliverySafetyGuard::in_memory();
        let signal = test_signal("signal-1", "session-1", "turn-1");
        let now = timestamp(0);

        for second in 0..4 {
            guard
                .check_at_for_provider_target(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    "oc_room_one",
                    now + Duration::seconds(second),
                )
                .expect("first room should allow");
        }

        let decision = guard
            .check_at_for_provider_target(
                DeliveryAttempt::new(&signal, "work_chat"),
                "oc_room_two",
                now + Duration::seconds(4),
            )
            .expect("second room should have a separate count");

        assert!(matches!(decision, DeliverySafetyDecision::Allow { .. }));
    }

    #[test]
    fn multiple_suppressed_messages_start_global_pause() {
        let guard = DeliverySafetyGuard::in_memory();
        let now = timestamp(0);

        suppress_message(&guard, "session-1", "turn-1", now);
        suppress_message(&guard, "session-2", "turn-1", now + Duration::seconds(20));
        let decision = suppress_message(&guard, "session-3", "turn-1", now + Duration::seconds(40));

        assert!(matches!(
            decision,
            DeliverySafetyDecision::Suppress(DeliverySuppression {
                pause_started: Some(DeliveryPauseStarted {
                    message_fingerprint_count: 3,
                    window_seconds: 120,
                }),
                ..
            })
        ));

        let different_signal = test_signal("signal-4", "session-4", "turn-1");
        assert!(matches!(
            guard
                .check_at(
                    DeliveryAttempt::new(&different_signal, "work_chat"),
                    now + Duration::seconds(41)
                )
                .expect("global pause should suppress"),
            DeliverySafetyDecision::Suppress(DeliverySuppression {
                reason: DeliverySuppressionReason::GlobalPaused,
                ..
            })
        ));
    }

    #[test]
    fn repeated_suppression_refreshes_global_pause_window() {
        let guard = DeliverySafetyGuard::in_memory();
        let now = timestamp(0);

        suppress_message(&guard, "session-a", "turn-1", now);
        suppress_message(&guard, "session-a", "turn-1", now + Duration::seconds(100));
        suppress_message(&guard, "session-b", "turn-1", now + Duration::seconds(150));
        let decision =
            suppress_message(&guard, "session-c", "turn-1", now + Duration::seconds(151));

        assert!(matches!(
            decision,
            DeliverySafetyDecision::Suppress(DeliverySuppression {
                pause_started: Some(DeliveryPauseStarted {
                    message_fingerprint_count: 3,
                    window_seconds: 120,
                }),
                ..
            })
        ));
    }

    #[test]
    fn unstructured_display_text_distinguishes_logical_messages() {
        let first = unstructured_signal("signal-1", "First", "First message.");
        let second = unstructured_signal("signal-2", "Second", "Second message.");

        assert_ne!(
            message_fingerprint_hash(&first),
            message_fingerprint_hash(&second)
        );
    }

    #[test]
    fn display_text_distinguishes_messages_with_session_but_no_answer() {
        let first = session_notification("signal-1", "session-1", "First notification.");
        let second = session_notification("signal-2", "session-1", "Second notification.");

        assert_ne!(
            message_fingerprint_hash(&first),
            message_fingerprint_hash(&second)
        );
    }

    #[test]
    fn delivery_windows_drop_expired_keys() {
        let guard = DeliverySafetyGuard::in_memory();
        let now = timestamp(0);

        for index in 0..3 {
            let signal = test_signal(
                &format!("signal-{index}"),
                &format!("session-{index}"),
                "turn-1",
            );
            guard
                .check_at(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    now + Duration::seconds(index),
                )
                .expect("unique message should allow");
        }

        let later_signal = test_signal("signal-later", "session-later", "turn-1");
        guard
            .check_at(
                DeliveryAttempt::new(&later_signal, "work_chat"),
                now + Duration::seconds(20),
            )
            .expect("later message should allow");

        let inner = guard
            .inner
            .lock()
            .expect("guard lock should not be poisoned");
        assert_eq!(inner.delivery_windows.len(), 1);
    }

    #[test]
    fn global_pause_is_persisted_and_reset() {
        let dir = tempdir().expect("tempdir should be created");
        let state_path = dir.path().join("delivery-safety-state.json");
        let guard =
            DeliverySafetyGuard::load(state_path.clone()).expect("guard should load empty state");
        let now = timestamp(0);

        suppress_message(&guard, "session-1", "turn-1", now);
        suppress_message(&guard, "session-2", "turn-1", now + Duration::seconds(20));
        suppress_message(&guard, "session-3", "turn-1", now + Duration::seconds(40));

        let raw = fs::read_to_string(&state_path).expect("state should be persisted");
        assert!(!raw.contains("Answer for"));

        let reloaded = DeliverySafetyGuard::load(state_path).expect("guard should reload state");
        assert!(reloaded.status().expect("status should read").paused);
        reloaded.reset().expect("reset should clear state");
        assert!(!reloaded.status().expect("status should read").paused);
    }

    #[test]
    fn signal_id_is_not_part_of_message_fingerprint() {
        let guard = DeliverySafetyGuard::in_memory();
        let now = timestamp(0);

        for index in 0..4 {
            let signal = test_signal(&format!("signal-{index}"), "session-1", "turn-1");
            guard
                .check_at(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    now + Duration::seconds(index),
                )
                .expect("guard should allow same logical message");
        }

        let signal = test_signal("signal-5", "session-1", "turn-1");
        assert!(matches!(
            guard
                .check_at(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    now + Duration::seconds(4)
                )
                .expect("same logical message should suppress"),
            DeliverySafetyDecision::Suppress(_)
        ));
    }

    fn suppress_message(
        guard: &DeliverySafetyGuard,
        session_id: &str,
        turn_id: &str,
        now: DateTime<Utc>,
    ) -> DeliverySafetyDecision {
        let signal = test_signal("signal-1", session_id, turn_id);
        let mut decision = DeliverySafetyDecision::Allow {
            message_fingerprint_hash: String::new(),
            delivery_key_hash: String::new(),
        };
        for offset in 0..5 {
            decision = guard
                .check_at(
                    DeliveryAttempt::new(&signal, "work_chat"),
                    now + Duration::seconds(offset),
                )
                .expect("guard should check");
        }
        decision
    }

    fn test_signal(signal_id: &str, session_id: &str, turn_id: &str) -> Signal {
        let mut signal = Signal::new_structured_with_timestamp(
            signal_id.to_string(),
            SignalSource {
                id: "codex_desktop".to_string(),
                source_type: "codex_desktop".to_string(),
            },
            SignalEvent {
                kind: SignalEventKind::TurnCompleted,
                raw_name: Some("task_complete".to_string()),
            },
            SignalDisplay {
                title: "Codex Desktop".to_string(),
                summary: "Codex Desktop finished a task.".to_string(),
            },
            timestamp(0),
            BTreeMap::new(),
        );
        signal.workspace = Some(SignalWorkspace {
            cwd: None,
            project_name: Some("agents-router".to_string()),
            project_path: Some("/Users/tester/projects/agents-router".to_string()),
            branch: None,
            worktree: None,
        });
        signal.conversation = Some(SignalConversation {
            session_id: Some(session_id.to_string()),
            session_title: Some("Review".to_string()),
            turn_id: Some(turn_id.to_string()),
            prompt: None,
            answer: Some(SignalAnswer {
                kind: SignalAnswerKind::Preview,
                content: format!("Answer for {session_id}/{turn_id}"),
            }),
            model: Some("gpt-5.5".to_string()),
        });
        signal
    }

    fn unstructured_signal(signal_id: &str, title: &str, summary: &str) -> Signal {
        Signal::new_structured_with_timestamp(
            signal_id.to_string(),
            SignalSource {
                id: "agents_router".to_string(),
                source_type: "agents_router".to_string(),
            },
            SignalEvent {
                kind: SignalEventKind::Custom,
                raw_name: None,
            },
            SignalDisplay {
                title: title.to_string(),
                summary: summary.to_string(),
            },
            timestamp(0),
            BTreeMap::new(),
        )
    }

    fn session_notification(signal_id: &str, session_id: &str, summary: &str) -> Signal {
        let mut signal = unstructured_signal(signal_id, "Notification", summary);
        signal.conversation = Some(SignalConversation {
            session_id: Some(session_id.to_string()),
            session_title: None,
            turn_id: None,
            prompt: None,
            answer: None,
            model: None,
        });
        signal
    }

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::seconds(seconds)
    }

    impl<'a> DeliveryAttempt<'a> {
        fn new(signal: &'a Signal, provider_id: &'a str) -> Self {
            Self {
                signal,
                provider_id,
            }
        }
    }
}
