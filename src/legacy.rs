use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use serde::Deserialize;

use crate::config::{RawConfig, SourceType, ValidatedConfig};

const LEGACY_APP_NAME: &str = "agents-notifier";
const LEGACY_BINARY_NAME: &str = "agents-notifier";
const LEGACY_SOURCE_ID: &str = "agents_notifier";
const NEW_SOURCE_ID: &str = "agents_router";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LegacyMigrationOutcome {
    pub migrated_config: bool,
    pub removed_service: bool,
    pub removed_files: usize,
    pub retained_cargo_binaries: Vec<PathBuf>,
}

impl LegacyMigrationOutcome {
    pub fn changed(&self) -> bool {
        self.migrated_config || self.removed_service || self.removed_files > 0
    }
}

#[derive(Debug, Deserialize)]
struct LegacyServiceMetadata {
    binary_path: Option<String>,
    config_path: Option<String>,
}

pub fn migrate_legacy_installation(config_path: &Path) -> anyhow::Result<LegacyMigrationOutcome> {
    let mut outcome = LegacyMigrationOutcome::default();
    let legacy_metadata_paths = legacy_service_metadata_paths()?;
    let legacy_metadata = legacy_metadata_paths
        .iter()
        .filter_map(|path| legacy_metadata(path).transpose())
        .collect::<anyhow::Result<Vec<_>>>()?;
    let legacy_binary_paths = legacy_metadata
        .iter()
        .filter_map(|metadata| metadata.binary_path.as_ref().map(PathBuf::from))
        .collect::<Vec<_>>();
    let legacy_config_paths = legacy_metadata
        .iter()
        .filter_map(|metadata| metadata.config_path.as_ref().map(PathBuf::from))
        .collect::<Vec<_>>();

    if migrate_legacy_config(config_path, &legacy_config_paths)? {
        outcome.migrated_config = true;
    }

    if cleanup_legacy_service()? {
        outcome.removed_service = true;
    }

    outcome.removed_files += cleanup_legacy_pid_processes()?;
    let binary_cleanup = cleanup_legacy_binaries(&legacy_binary_paths)?;
    outcome.removed_files += binary_cleanup.removed_files;
    outcome.retained_cargo_binaries = binary_cleanup.retained_cargo_binaries;
    outcome.removed_files += cleanup_legacy_files()?;

    Ok(outcome)
}

#[cfg(unix)]
fn cleanup_legacy_pid_processes() -> anyhow::Result<usize> {
    let mut removed = 0;
    for state_dir in legacy_state_dirs()? {
        let pid_file = state_dir.join("agents-notifier.pid");
        if !pid_file.exists() {
            continue;
        }

        let raw_pid = fs::read_to_string(&pid_file)
            .with_context(|| format!("failed to read legacy pid file `{}`", pid_file.display()))?;
        let pid = raw_pid
            .trim()
            .parse::<u32>()
            .with_context(|| format!("legacy pid file `{}` is invalid", pid_file.display()))?;
        match legacy_process_state(pid)? {
            LegacyProcessState::Missing => {
                remove_path_if_exists(&pid_file)?;
                removed += 1;
            }
            LegacyProcessState::AgentsNotifier => {
                terminate_process(pid)?;
                remove_path_if_exists(&pid_file)?;
                removed += 1;
            }
            LegacyProcessState::Other => {
                anyhow::bail!(
                    "legacy pid file `{}` points to `{pid}`, but it is not an agents-notifier process",
                    pid_file.display()
                );
            }
        }
    }

    Ok(removed)
}

#[cfg(not(unix))]
fn cleanup_legacy_pid_processes() -> anyhow::Result<usize> {
    Ok(0)
}

pub fn migrate_legacy_config(
    config_path: &Path,
    metadata_config_paths: &[PathBuf],
) -> anyhow::Result<bool> {
    if config_path.exists() {
        return Ok(false);
    }

    let Some(legacy_config_path) = legacy_config_candidates(metadata_config_paths)?
        .into_iter()
        .find(|path| path.exists())
    else {
        return Ok(false);
    };

    let raw = fs::read_to_string(&legacy_config_path).with_context(|| {
        format!(
            "failed to read legacy config `{}`",
            legacy_config_path.display()
        )
    })?;
    let migrated = migrate_legacy_config_content(&raw)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory `{}`", parent.display()))?;
    }
    fs::write(config_path, migrated).with_context(|| {
        format!(
            "failed to write migrated config `{}`",
            config_path.display()
        )
    })?;

    if legacy_config_path == legacy_config_file_path()? {
        remove_path_if_exists(&legacy_config_path)?;
        remove_empty_parent(&legacy_config_path)?;
    }

    Ok(true)
}

pub fn migrate_legacy_config_content(raw: &str) -> anyhow::Result<String> {
    let mut config: RawConfig =
        toml::from_str(raw).context("failed to parse legacy agents-notifier config")?;
    normalize_legacy_config(&mut config);
    let migrated =
        toml::to_string_pretty(&config).context("failed to serialize migrated config")?;

    ValidatedConfig::from_toml_str(&migrated).context("migrated config is invalid")?;
    Ok(migrated)
}

fn normalize_legacy_config(config: &mut RawConfig) {
    for source in &mut config.sources {
        if source.id == LEGACY_SOURCE_ID {
            source.id = NEW_SOURCE_ID.to_string();
            source.source_type = SourceType::AgentsRouter;
        }
    }

    for route in &mut config.routes {
        for source in &mut route.sources {
            if source == LEGACY_SOURCE_ID {
                *source = NEW_SOURCE_ID.to_string();
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn cleanup_legacy_service() -> anyhow::Result<bool> {
    let mut removed = false;
    let uid = current_uid()?;
    for domain in [format!("gui/{uid}"), format!("user/{uid}")] {
        let target = format!("{domain}/com.agents-notifier.service");
        let _ = Command::new("launchctl")
            .args(["bootout", &target])
            .output();
    }

    if let Some(path) = legacy_service_file_path()? {
        removed |= remove_path_if_exists(&path)?;
    }

    Ok(removed)
}

#[cfg(target_os = "linux")]
fn cleanup_legacy_service() -> anyhow::Result<bool> {
    let Some(unit_path) = legacy_service_file_path()? else {
        return Ok(false);
    };
    let installed = unit_path.exists();
    let active = Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "agents-notifier.service"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if active {
        run_systemctl(&["--user", "stop", "agents-notifier.service"])
            .context("failed to stop legacy agents-notifier service")?;
    }

    if installed {
        let _ = run_systemctl(&["--user", "disable", "agents-notifier.service"]);
        remove_path_if_exists(&unit_path)?;
        run_systemctl(&["--user", "daemon-reload"])
            .context("failed to reload systemd after removing legacy agents-notifier service")?;
        return Ok(true);
    }

    Ok(active)
}

#[cfg(windows)]
fn cleanup_legacy_service() -> anyhow::Result<bool> {
    let installed = Command::new("schtasks.exe")
        .args(["/Query", "/TN", r"\AgentsNotifier"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !installed {
        return Ok(false);
    }

    let _ = Command::new("schtasks.exe")
        .args(["/End", "/TN", r"\AgentsNotifier"])
        .output();
    let output = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", r"\AgentsNotifier", "/F"])
        .output()
        .context("failed to run schtasks.exe /Delete for legacy Agents Notifier task")?;
    if !output.status.success() {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        anyhow::bail!(
            "failed to delete legacy Agents Notifier task: {}",
            combined.trim()
        );
    }

    Ok(true)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn cleanup_legacy_service() -> anyhow::Result<bool> {
    Ok(false)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LegacyBinaryCleanupOutcome {
    removed_files: usize,
    retained_cargo_binaries: Vec<PathBuf>,
}

fn cleanup_legacy_binaries(
    metadata_binary_paths: &[PathBuf],
) -> anyhow::Result<LegacyBinaryCleanupOutcome> {
    let mut outcome = LegacyBinaryCleanupOutcome::default();
    let mut seen = HashSet::new();

    for path in metadata_binary_paths
        .iter()
        .cloned()
        .chain(legacy_marker_binary_paths()?)
        .chain(legacy_cargo_binary_paths()?)
    {
        if !seen.insert(path.clone()) || !is_legacy_binary_path(&path) || !path.exists() {
            continue;
        }

        if is_cargo_managed_binary_path(&path) {
            outcome.retained_cargo_binaries.push(path);
            continue;
        }

        if !should_remove_legacy_binary(&path) {
            continue;
        }

        if remove_path_if_exists(&path)? {
            outcome.removed_files += 1;
        }
        let marker = path.with_file_name(".agents-notifier-install-method");
        if remove_path_if_exists(&marker)? {
            outcome.removed_files += 1;
        }
        remove_empty_parent(&path)?;
    }

    Ok(outcome)
}

fn cleanup_legacy_files() -> anyhow::Result<usize> {
    let mut removed = 0;
    for path in legacy_cleanup_paths()? {
        if remove_path_if_exists(&path)? {
            removed += 1;
        }
    }

    Ok(removed)
}

fn legacy_marker_binary_paths() -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for dir in legacy_install_dirs()? {
        let marker = dir.join(".agents-notifier-install-method");
        if marker.exists() {
            let binary = dir.join(legacy_binary_file_name());
            if seen.insert(binary.clone()) {
                paths.push(binary);
            }
        }
    }

    Ok(paths)
}

fn legacy_cargo_binary_paths() -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    if let Some(cargo_home) = std::env::var_os("CARGO_HOME")
        && !cargo_home.is_empty()
    {
        push_unique_path(
            &mut paths,
            &mut seen,
            PathBuf::from(cargo_home)
                .join("bin")
                .join(legacy_binary_file_name()),
        );
    }

    push_unique_path(
        &mut paths,
        &mut seen,
        home_dir()?
            .join(".cargo")
            .join("bin")
            .join(legacy_binary_file_name()),
    );

    Ok(paths)
}

fn legacy_install_dirs() -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    for env_name in ["AGENTS_NOTIFIER_INSTALL_DIR", "AGENTS_ROUTER_INSTALL_DIR"] {
        if let Some(value) = std::env::var_os(env_name)
            && !value.is_empty()
        {
            push_unique_path(&mut dirs, &mut seen, PathBuf::from(value));
        }
    }

    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA")
            && !local_app_data.is_empty()
        {
            push_unique_path(
                &mut dirs,
                &mut seen,
                PathBuf::from(local_app_data)
                    .join("Programs")
                    .join(LEGACY_APP_NAME),
            );
        }
    }

    #[cfg(not(windows))]
    {
        push_unique_path(&mut dirs, &mut seen, home_dir()?.join(".local").join("bin"));
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        push_unique_path(&mut dirs, &mut seen, parent.to_path_buf());
    }

    Ok(dirs)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        paths.push(path);
    }
}

fn legacy_metadata(path: &Path) -> anyhow::Result<Option<LegacyServiceMetadata>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read legacy service metadata `{}`",
            path.display()
        )
    })?;
    let metadata = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse legacy service metadata `{}`",
            path.display()
        )
    })?;

    Ok(Some(metadata))
}

fn is_legacy_binary_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    file_name == LEGACY_BINARY_NAME || file_name == "agents-notifier.exe"
}

fn should_remove_legacy_binary(path: &Path) -> bool {
    if !is_legacy_binary_path(path) || is_cargo_managed_binary_path(path) {
        return false;
    }

    !path
        .components()
        .any(|component| component.as_os_str() == "target")
}

fn is_cargo_managed_binary_path(path: &Path) -> bool {
    if !is_legacy_binary_path(path) {
        return false;
    }

    if let Some(parent) = path.parent()
        && let Some(cargo_home) = std::env::var_os("CARGO_HOME")
        && !cargo_home.is_empty()
        && parent == PathBuf::from(cargo_home).join("bin")
    {
        return true;
    }

    let components = path
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();

    components
        .windows(2)
        .any(|window| window[0] == ".cargo" && window[1] == "bin")
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyProcessState {
    Missing,
    AgentsNotifier,
    Other,
}

#[cfg(unix)]
fn legacy_process_state(pid: u32) -> anyhow::Result<LegacyProcessState> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .with_context(|| format!("failed to inspect legacy process `{pid}`"))?;

    if !output.status.success() {
        return Ok(LegacyProcessState::Missing);
    }

    let command = String::from_utf8_lossy(&output.stdout);
    let name = Path::new(command.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if name == LEGACY_BINARY_NAME {
        Ok(LegacyProcessState::AgentsNotifier)
    } else {
        Ok(LegacyProcessState::Other)
    }
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> anyhow::Result<()> {
    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .with_context(|| format!("failed to terminate legacy process `{pid}`"))?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("failed to terminate legacy process `{pid}`")
    }
}

fn legacy_cleanup_paths() -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    if let Some(service_file) = legacy_service_file_path()? {
        paths.push(service_file);
    }
    paths.push(legacy_config_file_path()?);
    paths.extend(legacy_state_dirs()?);
    paths.extend(legacy_log_paths()?);

    Ok(paths)
}

fn legacy_service_metadata_paths() -> anyhow::Result<Vec<PathBuf>> {
    Ok(legacy_state_dirs()?
        .into_iter()
        .map(|dir| dir.join("service.json"))
        .collect())
}

fn legacy_config_candidates(metadata_config_paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for path in metadata_config_paths {
        push_unique_path(&mut paths, &mut seen, path.clone());
    }
    push_unique_path(&mut paths, &mut seen, legacy_config_file_path()?);

    Ok(paths)
}

fn legacy_state_dirs() -> anyhow::Result<Vec<PathBuf>> {
    let home = home_dir()?;
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    #[cfg(target_os = "macos")]
    {
        push_unique_path(
            &mut dirs,
            &mut seen,
            home.join("Library")
                .join("Application Support")
                .join(LEGACY_APP_NAME),
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(state_home) = std::env::var_os("XDG_STATE_HOME")
            && !state_home.is_empty()
        {
            push_unique_path(
                &mut dirs,
                &mut seen,
                PathBuf::from(state_home).join(LEGACY_APP_NAME),
            );
        }
        push_unique_path(
            &mut dirs,
            &mut seen,
            home.join(".local").join("state").join(LEGACY_APP_NAME),
        );
    }

    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA")
            && !local_app_data.is_empty()
        {
            push_unique_path(
                &mut dirs,
                &mut seen,
                PathBuf::from(local_app_data).join(LEGACY_APP_NAME),
            );
        }
        push_unique_path(
            &mut dirs,
            &mut seen,
            home.join("AppData").join("Local").join(LEGACY_APP_NAME),
        );
    }

    Ok(dirs)
}

fn legacy_log_paths() -> anyhow::Result<Vec<PathBuf>> {
    let home = home_dir()?;

    #[cfg(target_os = "macos")]
    return Ok(vec![
        home.join("Library").join("Logs").join(LEGACY_APP_NAME),
    ]);

    #[cfg(all(unix, not(target_os = "macos")))]
    return Ok(vec![]);

    #[cfg(windows)]
    return Ok(vec![
        home.join("AppData")
            .join("Local")
            .join(LEGACY_APP_NAME)
            .join("logs"),
    ]);
}

fn legacy_service_file_path() -> anyhow::Result<Option<PathBuf>> {
    let home = home_dir()?;

    #[cfg(target_os = "macos")]
    return Ok(Some(
        home.join("Library")
            .join("LaunchAgents")
            .join("com.agents-notifier.service.plist"),
    ));

    #[cfg(target_os = "linux")]
    return Ok(Some(
        home.join(".config")
            .join("systemd")
            .join("user")
            .join("agents-notifier.service"),
    ));

    #[cfg(windows)]
    return Ok(None);

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Ok(None);
}

fn legacy_config_file_path() -> anyhow::Result<PathBuf> {
    let home = home_dir()?;

    #[cfg(windows)]
    return Ok(home
        .join("AppData")
        .join("Roaming")
        .join(LEGACY_APP_NAME)
        .join("config.toml"));

    #[cfg(not(windows))]
    return Ok(home
        .join(".config")
        .join(LEGACY_APP_NAME)
        .join("config.toml"));
}

fn legacy_binary_file_name() -> &'static str {
    #[cfg(windows)]
    return "agents-notifier.exe";

    #[cfg(not(windows))]
    return LEGACY_BINARY_NAME;
}

fn remove_path_if_exists(path: &Path) -> anyhow::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect legacy path `{}`", path.display()));
        }
    };

    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove legacy directory `{}`", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove legacy file `{}`", path.display()))?;
    }

    Ok(true)
}

fn remove_empty_parent(path: &Path) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    match fs::remove_dir(parent) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove empty directory `{}`", parent.display())),
    }
}

fn home_dir() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;

    #[cfg(not(windows))]
    let home = std::env::var_os("HOME").context("HOME is not set")?;

    if home.is_empty() {
        #[cfg(windows)]
        anyhow::bail!("USERPROFILE is empty");

        #[cfg(not(windows))]
        anyhow::bail!("HOME is empty");
    }

    Ok(PathBuf::from(home))
}

#[cfg(target_os = "macos")]
fn current_uid() -> anyhow::Result<u32> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to detect current user id")?;
    if !output.status.success() {
        anyhow::bail!("failed to detect current user id");
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    raw.trim()
        .parse::<u32>()
        .context("failed to parse current user id")
}

#[cfg(target_os = "linux")]
fn run_systemctl(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .with_context(|| format!("failed to run systemctl {}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr).trim().to_string();

    if output.status.success() {
        Ok(combined)
    } else {
        anyhow::bail!("systemctl {} failed: {}", args.join(" "), combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_cargo_binary_is_not_a_migration_change() {
        let outcome = LegacyMigrationOutcome {
            retained_cargo_binaries: vec![PathBuf::from(
                "/Users/tester/.cargo/bin/agents-notifier",
            )],
            ..LegacyMigrationOutcome::default()
        };

        assert!(!outcome.changed());
    }

    #[test]
    fn migrates_legacy_source_without_rewriting_env_names() {
        let raw = r#"
schema_version = 1

[[sources]]
id = "agents_notifier"
type = "agents_notifier"

[[providers]]
id = "hook"
type = "webhook"
url_env = "AGENTS_NOTIFIER_WEBHOOK_URL"

[[routes]]
sources = ["agents_notifier"]
providers = ["hook"]
"#;

        let migrated = migrate_legacy_config_content(raw).expect("legacy config should migrate");
        ValidatedConfig::from_toml_str(&migrated).expect("migrated config should be valid");
        let config = RawConfig::from_toml_str(&migrated).expect("migrated config should parse");

        assert!(config.source(NEW_SOURCE_ID).is_some());
        assert_eq!(config.sources[0].source_type, SourceType::AgentsRouter);
        assert_eq!(config.routes[0].sources, vec![NEW_SOURCE_ID.to_string()]);
        assert_eq!(
            config.providers[0].url_env.as_deref(),
            Some("AGENTS_NOTIFIER_WEBHOOK_URL")
        );
        assert!(!migrated.contains(LEGACY_SOURCE_ID));
        assert!(migrated.contains("AGENTS_NOTIFIER_WEBHOOK_URL"));
    }

    #[test]
    fn keeps_unrelated_source_ids() {
        let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "hook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_cli"]
providers = ["hook"]
"#;

        let migrated = migrate_legacy_config_content(raw).expect("config should migrate");
        ValidatedConfig::from_toml_str(&migrated).expect("migrated config should be valid");
        let config = RawConfig::from_toml_str(&migrated).expect("migrated config should parse");

        assert!(config.source("codex_cli").is_some());
        assert!(config.source(NEW_SOURCE_ID).is_none());
        assert_eq!(config.routes[0].sources, vec!["codex_cli".to_string()]);
    }

    #[test]
    fn only_removes_legacy_binary_names_outside_target_dir() {
        assert!(should_remove_legacy_binary(Path::new(
            "/Users/tester/.local/bin/agents-notifier"
        )));
        assert!(!should_remove_legacy_binary(Path::new(
            "/repo/target/debug/agents-notifier"
        )));
        assert!(!should_remove_legacy_binary(Path::new(
            "/Users/tester/.cargo/bin/agents-notifier"
        )));
        assert!(!should_remove_legacy_binary(Path::new(
            "/Users/tester/.local/bin/agents-router"
        )));
    }

    #[test]
    fn recognizes_legacy_cargo_binary_paths() {
        assert!(is_cargo_managed_binary_path(Path::new(
            "/Users/tester/.cargo/bin/agents-notifier"
        )));
        assert!(!is_cargo_managed_binary_path(Path::new(
            "/Users/tester/.local/bin/agents-notifier"
        )));
        assert!(!is_cargo_managed_binary_path(Path::new(
            "/Users/tester/.cargo/bin/agents-router"
        )));
    }

    #[test]
    fn known_legacy_source_constants_stay_distinct() {
        assert_ne!(LEGACY_SOURCE_ID, NEW_SOURCE_ID);
    }
}
