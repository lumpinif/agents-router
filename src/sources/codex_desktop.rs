mod rollout;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::time;
use tracing::{debug, info, warn};

use crate::config::{PromptDetail, SourceConfig, SourceType, ValidatedConfig};
use crate::delivery_safety::DeliverySafetyGuard;
use crate::paths::{
    codex_desktop_source_state_path, codex_session_index_path, codex_sessions_dir_path,
};
use crate::router::{Provider, Router};
use crate::runtime::RuntimeState;
use crate::signal::Signal;
use crate::signal_builder::SignalBuilder;
use rollout::{
    RolloutItem, SessionInfo, completed_rollout_offset, discover_rollout_paths,
    draft_from_task_complete, load_session_titles, path_key, read_new_rollout_items,
    read_session_info,
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub async fn watch(runtime: RuntimeState) -> anyhow::Result<()> {
    let mut watcher: Option<CodexDesktopSessionWatcher> = None;
    let mut interval = time::interval(POLL_INTERVAL);

    info!(event = "watch.supervisor.started",);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let snapshot = runtime.current()?;
                let Some(source) = codex_desktop_source(&snapshot.config) else {
                    if let Some(watcher) = watcher.take() {
                        watcher.save_state()?;
                        info!(event = "watch.stopped");
                    }
                    continue;
                };
                if watcher.is_none() {
                    watcher = Some(CodexDesktopSessionWatcher::new(
                        codex_sessions_dir_path()?,
                        codex_session_index_path()?,
                        codex_desktop_source_state_path()?,
                    )?);
                    info!(
                        source.id = %source.id,
                        source.type = %source.source_type.as_str(),
                        event = "watch.started",
                    );
                }
                let watcher = watcher
                    .as_mut()
                    .expect("Codex Desktop watcher should exist after start");
                let batch = watcher.poll(&snapshot.config, source)?;
                let providers = snapshot.provider_refs();
                let delivery_safety = runtime.delivery_safety();
                let response_surface_ledger = runtime.response_surface_ledger();
                route_and_checkpoint_batch(
                    watcher,
                    &snapshot.config,
                    &providers,
                    batch,
                    Some(&delivery_safety),
                    Some(&response_surface_ledger),
                )
                .await?;
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for shutdown signal")?;
                info!(event = "shutdown.requested");
                if let Some(watcher) = watcher {
                    watcher.save_state()?;
                }
                info!(event = "shutdown.completed");
                return Ok(());
            }
        }
    }
}

async fn route_and_checkpoint_batch(
    watcher: &mut CodexDesktopSessionWatcher,
    config: &ValidatedConfig,
    providers: &[&dyn Provider],
    batch: CodexDesktopPollBatch,
    delivery_safety: Option<&DeliverySafetyGuard>,
    response_surface_ledger: Option<&crate::response_surface_ledger::ResponseSurfaceLedgerStore>,
) -> anyhow::Result<()> {
    for signal in &batch.signals {
        info!(
            signal.id = %signal.id,
            source.id = %signal.source_id(),
            source.type = %signal.source_type(),
            event = "signal.created",
        );

        if let Err(error) = Router::new(config)
            .route_with_safety_and_response_surfaces(
                signal,
                providers,
                delivery_safety,
                response_surface_ledger,
            )
            .await
        {
            warn!(
                signal.id = %signal.id,
                source.id = %signal.source_id(),
                source.type = %signal.source_type(),
                error = %error,
                event = "provider.send.failed",
            );
        }
    }
    watcher.commit(batch)?;
    Ok(())
}

fn codex_desktop_source(config: &ValidatedConfig) -> Option<&SourceConfig> {
    config
        .sources
        .iter()
        .find(|source| source.source_type == SourceType::CodexDesktop)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexSessionOrigin {
    Desktop,
    Other,
    Unknown,
}

pub fn codex_session_origin(
    sessions_dir: &Path,
    session_id: &str,
) -> anyhow::Result<CodexSessionOrigin> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(CodexSessionOrigin::Unknown);
    }

    for path in discover_rollout_paths(sessions_dir)? {
        let session = read_session_info(&path)?;
        if session.id.as_deref() != Some(session_id) {
            continue;
        }

        return if session.is_codex_desktop() {
            Ok(CodexSessionOrigin::Desktop)
        } else {
            Ok(CodexSessionOrigin::Other)
        };
    }

    Ok(CodexSessionOrigin::Unknown)
}

struct CodexDesktopSessionWatcher {
    sessions_dir: PathBuf,
    session_index_path: PathBuf,
    state_path: PathBuf,
    state: WatchState,
    pending_prompts: BTreeMap<String, String>,
}

struct CodexDesktopPollBatch {
    signals: Vec<Signal>,
    state: WatchState,
    pending_prompts: BTreeMap<String, String>,
    changed: bool,
}

impl CodexDesktopSessionWatcher {
    fn new(
        sessions_dir: PathBuf,
        session_index_path: PathBuf,
        state_path: PathBuf,
    ) -> anyhow::Result<Self> {
        let state = WatchState::load(&state_path)?;
        let mut watcher = Self {
            sessions_dir,
            session_index_path,
            state_path,
            state,
            pending_prompts: BTreeMap::new(),
        };
        watcher.baseline_existing_files()?;
        Ok(watcher)
    }

    fn poll(
        &self,
        config: &ValidatedConfig,
        source: &SourceConfig,
    ) -> anyhow::Result<CodexDesktopPollBatch> {
        let prompt_detail = config.notification.prompt_detail;
        let titles = load_session_titles(&self.session_index_path)?;
        let mut state = self.state.clone();
        let mut pending_prompts = self.pending_prompts.clone();
        let mut signals = Vec::new();
        let mut changed = false;

        for path in discover_rollout_paths(&self.sessions_dir)? {
            let path_key = path_key(&path);
            if !state.files.contains_key(&path_key) {
                let offset = 0;
                let session = read_session_info(&path)?;
                state
                    .files
                    .insert(path_key.clone(), FileWatchState { offset, session });
                changed = true;
            }

            let mut result = read_new_rollout_items(
                &path,
                state
                    .files
                    .get(&path_key)
                    .expect("file state should exist after insertion")
                    .offset,
                prompt_detail == PromptDetail::On,
            )?;

            if result.offset_changed {
                let file_state = state
                    .files
                    .get_mut(&path_key)
                    .expect("file state should exist before update");
                file_state.offset = result.new_offset;
                changed = true;
            }

            for item in result.items.drain(..) {
                match item {
                    RolloutItem::SessionMeta(session) => {
                        state
                            .files
                            .get_mut(&path_key)
                            .expect("file state should exist before metadata update")
                            .session
                            .merge(session);
                        changed = true;
                    }
                    RolloutItem::TurnContext(session) => {
                        state
                            .files
                            .get_mut(&path_key)
                            .expect("file state should exist before context update")
                            .session
                            .merge_turn_context(session);
                        changed = true;
                    }
                    RolloutItem::UserMessage(prompt) => {
                        if prompt_detail == PromptDetail::On {
                            pending_prompts.insert(path_key.clone(), prompt);
                        }
                    }
                    RolloutItem::TaskComplete(task) => {
                        let session = state
                            .files
                            .get(&path_key)
                            .expect("file state should exist before task processing")
                            .session
                            .clone();
                        let Some(session_id) = session.id.as_deref() else {
                            debug!(
                                source.id = %source.id,
                                source.type = %source.source_type.as_str(),
                                event = "watch.line.ignored",
                            );
                            continue;
                        };
                        let delivery_key = format!("{}:{}", session_id, task.turn_id);
                        if state.delivered_turns.contains(&delivery_key) {
                            continue;
                        }
                        let prompt = if prompt_detail == PromptDetail::On {
                            pending_prompts.remove(&path_key)
                        } else {
                            None
                        };

                        if let Some(draft) = draft_from_task_complete(
                            source,
                            &session,
                            titles.get(session_id).map(String::as_str),
                            task,
                            prompt.as_deref(),
                        ) {
                            let signal = SignalBuilder::new(config).build(draft)?;
                            state.delivered_turns.insert(delivery_key);
                            state.prune_delivered_turns();
                            signals.push(signal);
                            changed = true;
                        }
                    }
                }
            }
        }

        Ok(CodexDesktopPollBatch {
            signals,
            state,
            pending_prompts,
            changed,
        })
    }

    fn commit(&mut self, batch: CodexDesktopPollBatch) -> anyhow::Result<()> {
        self.state = batch.state;
        self.pending_prompts = batch.pending_prompts;
        if batch.changed {
            self.save_state()?;
        }
        Ok(())
    }

    fn baseline_existing_files(&mut self) -> anyhow::Result<()> {
        // Completion notifications are live events. A watch restart should not
        // replay tasks that finished while agents-router was not running.
        let mut changed = false;
        let mut discovered_files = 0usize;
        let mut inserted_files = 0usize;
        let mut updated_offsets = 0usize;
        let mut refreshed_sessions = 0usize;
        for path in discover_rollout_paths(&self.sessions_dir)? {
            discovered_files += 1;
            let path_key = path_key(&path);
            let offset = completed_rollout_offset(&path)?;
            let session = read_session_info(&path)?;

            if let Some(file_state) = self.state.files.get_mut(&path_key) {
                if file_state.offset != offset {
                    file_state.offset = offset;
                    updated_offsets += 1;
                    changed = true;
                }
                file_state.session = session;
                refreshed_sessions += 1;
            } else {
                self.state
                    .files
                    .insert(path_key, FileWatchState { offset, session });
                inserted_files += 1;
                changed = true;
            }
        }

        if changed {
            self.save_state()?;
        }
        info!(
            files.discovered = discovered_files,
            files.inserted = inserted_files,
            files.offsets_updated = updated_offsets,
            files.sessions_refreshed = refreshed_sessions,
            state.path = %self.state_path.display(),
            event = "codex_desktop.watch.startup_baseline",
        );

        Ok(())
    }

    fn save_state(&self) -> anyhow::Result<()> {
        self.state.save(&self.state_path)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WatchState {
    files: BTreeMap<String, FileWatchState>,
    delivered_turns: BTreeSet<String>,
}

impl WatchState {
    fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path).with_context(|| {
            format!(
                "failed to read Codex Desktop source state `{}`",
                path.display()
            )
        })?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "failed to parse Codex Desktop source state `{}`",
                path.display()
            )
        })
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create Codex Desktop source state directory `{}`",
                    parent.display()
                )
            })?;
        }

        let raw = serde_json::to_string_pretty(self)
            .context("failed to serialize Codex Desktop source state")?;
        fs::write(path, raw).with_context(|| {
            format!(
                "failed to write Codex Desktop source state `{}`",
                path.display()
            )
        })
    }

    fn prune_delivered_turns(&mut self) {
        const MAX_DELIVERED_TURNS: usize = 2_000;
        while self.delivered_turns.len() > MAX_DELIVERED_TURNS {
            let Some(first) = self.delivered_turns.iter().next().cloned() else {
                break;
            };
            self.delivered_turns.remove(&first);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FileWatchState {
    offset: u64,
    #[serde(default)]
    session: SessionInfo,
}

#[cfg(test)]
mod tests;
