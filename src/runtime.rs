use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use tokio::time::{self, MissedTickBehavior};
use tracing::{info, warn};

#[cfg(target_os = "linux")]
use crate::config::SourceType;
use crate::config::{LoadedConfig, ValidatedConfig};
use crate::delivery_safety::DeliverySafetyGuard;
use crate::local_integrations::{self, LocalSourceIntegrationPaths, LocalSourceIntegrationReport};
use crate::providers::build_providers;
use crate::response_surface_ledger::ResponseSurfaceLedgerStore;
use crate::router::Provider;

const CONFIG_RELOAD_POLL_INTERVAL: Duration = Duration::from_secs(1);
const CONFIG_RELOAD_SETTLE_DELAY: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub struct RuntimeState {
    current: Arc<RwLock<Arc<RuntimeSnapshot>>>,
    delivery_safety: DeliverySafetyGuard,
    response_surface_ledger: ResponseSurfaceLedgerStore,
}

pub struct RuntimeSnapshot {
    pub config: ValidatedConfig,
    providers: Vec<Box<dyn Provider>>,
}

impl RuntimeState {
    pub fn new(config: ValidatedConfig) -> anyhow::Result<Self> {
        Self::new_with_delivery_safety(config, DeliverySafetyGuard::in_memory())
    }

    pub fn new_with_delivery_safety(
        config: ValidatedConfig,
        delivery_safety: DeliverySafetyGuard,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            current: Arc::new(RwLock::new(Arc::new(RuntimeSnapshot::new(config)?))),
            delivery_safety,
            response_surface_ledger: ResponseSurfaceLedgerStore::load_default()?,
        })
    }

    pub fn current(&self) -> anyhow::Result<Arc<RuntimeSnapshot>> {
        self.current
            .read()
            .map(|current| Arc::clone(&current))
            .map_err(|_| anyhow::anyhow!("runtime state lock is poisoned"))
    }

    pub fn delivery_safety(&self) -> DeliverySafetyGuard {
        self.delivery_safety.clone()
    }

    pub fn response_surface_ledger(&self) -> ResponseSurfaceLedgerStore {
        self.response_surface_ledger.clone()
    }

    pub fn reload_from_path(&self, path: &Path) -> anyhow::Result<()> {
        let snapshot = RuntimeSnapshot::from_path(path)?;
        self.replace(snapshot)
    }

    pub fn reload_from_path_with_local_integrations(
        &self,
        path: &Path,
        integration_paths: &LocalSourceIntegrationPaths,
    ) -> anyhow::Result<LocalSourceIntegrationReport> {
        let snapshot = RuntimeSnapshot::from_path(path)?;
        let report = local_integrations::ensure_local_source_integrations_with_paths(
            &snapshot.config,
            integration_paths,
        )?;
        self.replace(snapshot)?;
        Ok(report)
    }

    fn replace(&self, snapshot: RuntimeSnapshot) -> anyhow::Result<()> {
        let mut current = self
            .current
            .write()
            .map_err(|_| anyhow::anyhow!("runtime state lock is poisoned"))?;
        *current = Arc::new(snapshot);
        Ok(())
    }
}

impl RuntimeSnapshot {
    pub fn new(config: ValidatedConfig) -> anyhow::Result<Self> {
        ensure_sources_supported_on_current_platform(&config)?;
        let providers = build_providers(&config).context("provider setup failed")?;
        Ok(Self { config, providers })
    }

    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let config = LoadedConfig::from_path(path)
            .with_context(|| format!("failed to load config `{}`", path.display()))?;
        Self::new(config.validated)
    }

    pub fn provider_refs(&self) -> Vec<&dyn Provider> {
        self.providers
            .iter()
            .map(|provider| provider.as_ref() as &dyn Provider)
            .collect()
    }
}

pub fn ensure_sources_supported_on_current_platform(
    config: &ValidatedConfig,
) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    if config
        .sources
        .iter()
        .any(|source| source.source_type == SourceType::CodexDesktop)
    {
        anyhow::bail!("Codex Desktop source is supported only on macOS and Windows");
    }

    let _ = config;
    Ok(())
}

pub async fn reload_config_on_change(
    config_path: PathBuf,
    runtime: RuntimeState,
) -> anyhow::Result<()> {
    let integration_paths = LocalSourceIntegrationPaths::detect()?;
    let mut observed_state = ConfigFileState::read(&config_path);
    let mut interval = time::interval(CONFIG_RELOAD_POLL_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    info!(
        config.path = %config_path.display(),
        event = "config.reload.watching",
    );

    loop {
        interval.tick().await;

        let next_state = ConfigFileState::read(&config_path);
        if next_state == observed_state {
            continue;
        }

        time::sleep(CONFIG_RELOAD_SETTLE_DELAY).await;
        let settled_state = ConfigFileState::read(&config_path);
        if settled_state == observed_state {
            continue;
        }

        match &settled_state {
            ConfigFileState::Present(_) => match runtime
                .reload_from_path_with_local_integrations(&config_path, &integration_paths)
            {
                Ok(integration_report) => {
                    let snapshot = runtime.current()?;
                    info!(
                        config.path = %config_path.display(),
                        sources = snapshot.config.sources.len(),
                        providers = snapshot.config.providers.len(),
                        routes = snapshot.config.routes.len(),
                        local_integrations_changed = integration_report.has_changes(),
                        event = "config.reload.succeeded",
                    );
                }
                Err(error) => {
                    warn!(
                        config.path = %config_path.display(),
                        error = %error,
                        event = "config.reload.failed",
                    );
                }
            },
            ConfigFileState::Unavailable(reason) => {
                warn!(
                    config.path = %config_path.display(),
                    reason = %reason,
                    event = "config.reload.failed",
                );
            }
        }

        observed_state = settled_state;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigFileState {
    Present(ConfigFileFingerprint),
    Unavailable(String),
}

impl ConfigFileState {
    fn read(path: &Path) -> Self {
        match fs::metadata(path) {
            Ok(metadata) => Self::Present(ConfigFileFingerprint {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            }),
            Err(error) => Self::Unavailable(error.kind().to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigFileFingerprint {
    modified: Option<SystemTime>,
    len: u64,
}

#[cfg(test)]
mod tests;
