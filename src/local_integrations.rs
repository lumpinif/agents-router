use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Map, Value, json};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};
use uuid::Uuid;

use crate::agent_integration_catalog::{
    AgentIntegrationDescriptor, AgentIntegrationId, LocalIntegrationKind,
    agent_integration_descriptor, agent_integration_for_source,
    claude_code_hook_command as catalog_claude_code_hook_command,
    codex_cli_stop_hook_command as catalog_codex_cli_stop_hook_command,
};
use crate::config::{SourceType, ValidatedConfig};
use crate::paths::{claude_code_settings_path, codex_cli_config_path};

pub fn codex_cli_stop_hook_command() -> String {
    catalog_codex_cli_stop_hook_command()
}

pub fn claude_code_hook_command() -> String {
    catalog_claude_code_hook_command()
}

const CODEX_CLI_STOP_HOOK_TIMEOUT_SECONDS: i64 = 10;
const CODEX_CLI_STOP_HOOK_STATUS_MESSAGE: &str = "Forwarding completion to Agents Router";
const CLAUDE_CODE_HOOK_TIMEOUT_SECONDS: i64 = 10;
const CLAUDE_CODE_HOOK_EVENTS: &[&str] =
    &["SessionStart", "UserPromptSubmit", "Stop", "Notification"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSourceIntegrationPaths {
    pub codex_cli_config_path: PathBuf,
    pub claude_code_settings_path: PathBuf,
}

impl LocalSourceIntegrationPaths {
    pub fn detect() -> anyhow::Result<Self> {
        Ok(Self {
            codex_cli_config_path: codex_cli_config_path()?,
            claude_code_settings_path: claude_code_settings_path()?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalSourceIntegrationReport {
    pub codex_cli_stop_hook: Option<CodexCliStopHookSetup>,
    pub claude_code_hooks: Option<ClaudeCodeHooksSetup>,
}

impl LocalSourceIntegrationReport {
    pub fn has_changes(&self) -> bool {
        self.codex_cli_stop_hook
            .as_ref()
            .is_some_and(CodexCliStopHookSetup::changed)
            || self
                .claude_code_hooks
                .as_ref()
                .is_some_and(ClaudeCodeHooksSetup::changed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliStopHookSetup {
    pub path: PathBuf,
    pub status: CodexCliStopHookSetupStatus,
}

impl CodexCliStopHookSetup {
    pub fn changed(&self) -> bool {
        matches!(
            self.status,
            CodexCliStopHookSetupStatus::CreatedConfig | CodexCliStopHookSetupStatus::UpdatedConfig
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexCliStopHookSetupStatus {
    CreatedConfig,
    UpdatedConfig,
    AlreadyConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeHooksSetup {
    pub path: PathBuf,
    pub status: ClaudeCodeHooksSetupStatus,
}

impl ClaudeCodeHooksSetup {
    pub fn changed(&self) -> bool {
        matches!(
            self.status,
            ClaudeCodeHooksSetupStatus::CreatedSettings
                | ClaudeCodeHooksSetupStatus::UpdatedSettings
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeCodeHooksSetupStatus {
    CreatedSettings,
    UpdatedSettings,
    AlreadyConfigured,
}

pub fn ensure_local_source_integrations(
    config: &ValidatedConfig,
) -> anyhow::Result<LocalSourceIntegrationReport> {
    let paths = LocalSourceIntegrationPaths::detect()?;
    ensure_local_source_integrations_with_paths(config, &paths)
}

pub fn ensure_local_source_integrations_with_paths(
    config: &ValidatedConfig,
    paths: &LocalSourceIntegrationPaths,
) -> anyhow::Result<LocalSourceIntegrationReport> {
    let mut report = LocalSourceIntegrationReport::default();
    let descriptors = config
        .sources
        .iter()
        .filter_map(|source| local_integration_descriptor(source).transpose())
        .collect::<anyhow::Result<Vec<_>>>()?;

    for descriptor in descriptors {
        match descriptor.source_capability.local_integration {
            LocalIntegrationKind::CodexCliStopHook => {
                if report.codex_cli_stop_hook.is_none() {
                    report.codex_cli_stop_hook =
                        Some(ensure_codex_cli_stop_hook(&paths.codex_cli_config_path)?);
                }
            }
            LocalIntegrationKind::ClaudeCodeHooks => {
                if report.claude_code_hooks.is_none() {
                    report.claude_code_hooks =
                        Some(ensure_claude_code_hooks(&paths.claude_code_settings_path)?);
                }
            }
            LocalIntegrationKind::None => {}
        }
    }

    Ok(report)
}

fn local_integration_descriptor(
    source: &crate::config::SourceConfig,
) -> anyhow::Result<Option<&'static AgentIntegrationDescriptor>> {
    match source.source_type {
        SourceType::CodexCli => {
            let descriptor = agent_integration_descriptor(AgentIntegrationId::CodexCli);
            if source.id != descriptor.source_capability.canonical_source_id {
                anyhow::bail!(
                    "Codex CLI source id must be `codex_cli` because Codex CLI has one global Stop hook; got `{}`",
                    source.id
                );
            }
            Ok(Some(descriptor))
        }
        SourceType::ClaudeCode => {
            let descriptor = agent_integration_descriptor(AgentIntegrationId::ClaudeCode);
            if source.id != descriptor.source_capability.canonical_source_id {
                anyhow::bail!(
                    "Claude Code source id must be `claude_code` because Claude Code has one global settings file; got `{}`",
                    source.id
                );
            }
            Ok(Some(descriptor))
        }
        SourceType::CodexDesktop => Ok(Some(agent_integration_descriptor(
            AgentIntegrationId::CodexDesktop,
        ))),
        SourceType::AgentHook => Ok(agent_integration_for_source(&source.id, source.source_type)),
        SourceType::AgentsRouter => Ok(None),
    }
}

pub fn ensure_claude_code_hooks(path: &Path) -> anyhow::Result<ClaudeCodeHooksSetup> {
    let original = read_existing_claude_settings(path)?;
    let existed = original.is_some();
    let mut settings = parse_claude_settings(path, original.as_deref())?;

    ensure_claude_code_hooks_in_settings(&mut settings)?;

    let rendered = serde_json::to_string_pretty(&settings)
        .context("failed to serialize Claude Code settings")?
        + "\n";
    if original.as_deref() == Some(rendered.as_str()) {
        return Ok(ClaudeCodeHooksSetup {
            path: path.to_path_buf(),
            status: ClaudeCodeHooksSetupStatus::AlreadyConfigured,
        });
    }

    write_json_config(path, rendered, "Claude Code settings")?;

    Ok(ClaudeCodeHooksSetup {
        path: path.to_path_buf(),
        status: if existed {
            ClaudeCodeHooksSetupStatus::UpdatedSettings
        } else {
            ClaudeCodeHooksSetupStatus::CreatedSettings
        },
    })
}

fn read_existing_claude_settings(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    fs::read_to_string(path)
        .with_context(|| format!("failed to read Claude Code settings `{}`", path.display()))
        .map(Some)
}

fn parse_claude_settings(path: &Path, raw: Option<&str>) -> anyhow::Result<Value> {
    let Some(raw) = raw else {
        return Ok(Value::Object(Map::new()));
    };

    serde_json::from_str(raw)
        .with_context(|| format!("failed to parse Claude Code settings `{}`", path.display()))
}

fn ensure_claude_code_hooks_in_settings(settings: &mut Value) -> anyhow::Result<()> {
    let root = settings
        .as_object_mut()
        .context("Claude Code settings root must be a JSON object")?;
    if root
        .get("disableAllHooks")
        .and_then(Value::as_bool)
        .is_some_and(|disabled| disabled)
    {
        anyhow::bail!(
            "Claude Code settings `disableAllHooks` is true; enable hooks before Agents Router can configure Claude Code notifications"
        );
    }
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .context("Claude Code settings `hooks` must be a JSON object")?;

    for event_name in CLAUDE_CODE_HOOK_EVENTS {
        ensure_claude_code_hook_event(hooks, event_name)?;
    }

    Ok(())
}

fn ensure_claude_code_hook_event(
    hooks: &mut Map<String, Value>,
    event_name: &str,
) -> anyhow::Result<()> {
    let event_hooks = hooks
        .entry(event_name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let event_hooks = event_hooks
        .as_array_mut()
        .with_context(|| format!("Claude Code settings `hooks.{event_name}` must be an array"))?;

    if has_claude_code_hook_command(event_hooks, event_name)? {
        return Ok(());
    }

    let command = claude_code_hook_command();
    event_hooks.push(json!({
        "hooks": [
            {
                "type": "command",
                "command": command,
                "timeout": CLAUDE_CODE_HOOK_TIMEOUT_SECONDS
            }
        ]
    }));
    Ok(())
}

fn has_claude_code_hook_command(event_hooks: &[Value], event_name: &str) -> anyhow::Result<bool> {
    let expected_command = claude_code_hook_command();
    for (hook_index, hook) in event_hooks.iter().enumerate() {
        let hook = hook.as_object().with_context(|| {
            format!("Claude Code settings `hooks.{event_name}[{hook_index}]` must be an object")
        })?;
        let Some(commands) = hook.get("hooks") else {
            continue;
        };
        let commands = commands.as_array().with_context(|| {
            format!(
                "Claude Code settings `hooks.{event_name}[{hook_index}].hooks` must be an array"
            )
        })?;
        for (command_index, command) in commands.iter().enumerate() {
            let command = command.as_object().with_context(|| {
                format!(
                    "Claude Code settings `hooks.{event_name}[{hook_index}].hooks[{command_index}]` must be an object"
                )
            })?;
            if command.get("type").and_then(Value::as_str) == Some("command")
                && command.get("command").and_then(Value::as_str) == Some(expected_command.as_str())
            {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub fn ensure_codex_cli_stop_hook(path: &Path) -> anyhow::Result<CodexCliStopHookSetup> {
    let original = read_existing_codex_config(path)?;
    let existed = original.is_some();
    let mut document = parse_codex_config(path, original.as_deref())?;

    ensure_codex_lifecycle_hooks_feature_enabled(&mut document)?;
    if !has_agents_router_stop_hook(&document) {
        append_agents_router_stop_hook(&mut document)?;
    }

    let rendered = document.to_string();
    if original.as_deref() == Some(rendered.as_str()) {
        return Ok(CodexCliStopHookSetup {
            path: path.to_path_buf(),
            status: CodexCliStopHookSetupStatus::AlreadyConfigured,
        });
    }

    write_text_config(path, rendered, "Codex CLI config")?;

    Ok(CodexCliStopHookSetup {
        path: path.to_path_buf(),
        status: if existed {
            CodexCliStopHookSetupStatus::UpdatedConfig
        } else {
            CodexCliStopHookSetupStatus::CreatedConfig
        },
    })
}

fn read_existing_codex_config(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    fs::read_to_string(path)
        .with_context(|| format!("failed to read Codex CLI config `{}`", path.display()))
        .map(Some)
}

fn parse_codex_config(path: &Path, raw: Option<&str>) -> anyhow::Result<DocumentMut> {
    let Some(raw) = raw else {
        return Ok(DocumentMut::new());
    };

    raw.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse Codex CLI config `{}`", path.display()))
}

fn ensure_codex_lifecycle_hooks_feature_enabled(document: &mut DocumentMut) -> anyhow::Result<()> {
    let features_item = document
        .as_table_mut()
        .entry("features")
        .or_insert(Item::Table(Table::new()));
    let features = features_item
        .as_table_like_mut()
        .context("Codex CLI config `features` must be a TOML table")?;

    features.remove("codex_hooks");

    if features
        .get("hooks")
        .and_then(Item::as_bool)
        .is_some_and(|enabled| enabled)
    {
        return Ok(());
    }

    features.insert("hooks", value(true));
    Ok(())
}

fn has_agents_router_stop_hook(document: &DocumentMut) -> bool {
    let expected_command = codex_cli_stop_hook_command();
    document
        .get("hooks")
        .and_then(Item::as_table_like)
        .and_then(|hooks| hooks.get("Stop"))
        .and_then(Item::as_array_of_tables)
        .is_some_and(|stop_hooks| {
            stop_hooks.iter().any(|stop_hook| {
                stop_hook
                    .get("hooks")
                    .and_then(Item::as_array_of_tables)
                    .is_some_and(|commands| {
                        commands.iter().any(|command| {
                            command.get("type").and_then(Item::as_str) == Some("command")
                                && command.get("command").and_then(Item::as_str)
                                    == Some(expected_command.as_str())
                        })
                    })
            })
        })
}

fn append_agents_router_stop_hook(document: &mut DocumentMut) -> anyhow::Result<()> {
    let hooks_item = document
        .as_table_mut()
        .entry("hooks")
        .or_insert(Item::Table(Table::new()));
    let hooks = hooks_item
        .as_table_like_mut()
        .context("Codex CLI config `hooks` must be a TOML table")?;
    let stop_hooks_item = hooks
        .entry("Stop")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let stop_hooks = stop_hooks_item
        .as_array_of_tables_mut()
        .context("Codex CLI config `hooks.Stop` must be an array of tables")?;

    let mut command_hook = Table::new();
    command_hook.insert("type", value("command"));
    command_hook.insert("command", value(codex_cli_stop_hook_command()));
    command_hook.insert("timeout", value(CODEX_CLI_STOP_HOOK_TIMEOUT_SECONDS));
    command_hook.insert("statusMessage", value(CODEX_CLI_STOP_HOOK_STATUS_MESSAGE));

    let mut command_hooks = ArrayOfTables::new();
    command_hooks.push(command_hook);

    let mut stop_hook = Table::new();
    stop_hook.insert("hooks", Item::ArrayOfTables(command_hooks));
    stop_hooks.push(stop_hook);

    Ok(())
}

fn write_json_config(path: &Path, raw: String, label: &str) -> anyhow::Result<()> {
    write_text_config(path, raw, label)
}

fn write_text_config(path: &Path, raw: String, label: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create {label} directory `{}`", parent.display())
        })?;
    }

    let temp_path = temporary_config_path(path)?;
    fs::write(&temp_path, raw)
        .with_context(|| format!("failed to write {label} `{}`", temp_path.display()))?;
    preserve_existing_permissions(path, &temp_path)
        .with_context(|| format!("failed to preserve {label} permissions"))?;
    fs::rename(&temp_path, path).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to replace {label} `{}` with `{}`",
            path.display(),
            temp_path.display()
        )
    })
}

#[cfg(unix)]
fn preserve_existing_permissions(path: &Path, temp_path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let mode = fs::metadata(path)
        .with_context(|| format!("failed to read permissions for `{}`", path.display()))?
        .permissions()
        .mode();
    fs::set_permissions(temp_path, fs::Permissions::from_mode(mode)).with_context(|| {
        format!(
            "failed to set permissions on temporary config `{}`",
            temp_path.display()
        )
    })
}

#[cfg(windows)]
fn preserve_existing_permissions(_path: &Path, _temp_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn temporary_config_path(path: &Path) -> anyhow::Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .context("config path must include a file name")?;
    let temp_file_name = format!(".{file_name}.{}.tmp", Uuid::new_v4());
    Ok(path.with_file_name(temp_file_name))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::config::{
        CliConfig, LogConfig, NotificationConfig, ProviderType, RawConfig, RawProviderConfig,
        RouteConfig, SourceConfig,
    };

    #[test]
    fn skips_config_without_local_source_integrations() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };

        let report = ensure_local_source_integrations_with_paths(
            &test_config("agents_router", SourceType::AgentsRouter),
            &paths,
        )
        .expect("config without local source integrations should not require changes");

        assert_eq!(report, LocalSourceIntegrationReport::default());
        assert!(!paths.codex_cli_config_path.exists());
        assert!(!paths.claude_code_settings_path.exists());
    }

    #[test]
    fn custom_codex_desktop_source_id_does_not_require_local_integration() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };

        let report = ensure_local_source_integrations_with_paths(
            &test_config("my_desktop", SourceType::CodexDesktop),
            &paths,
        )
        .expect("Codex Desktop source id should not require local integration");

        assert_eq!(report, LocalSourceIntegrationReport::default());
        assert!(!paths.codex_cli_config_path.exists());
        assert!(!paths.claude_code_settings_path.exists());
    }

    #[test]
    fn applies_codex_cli_stop_hook_for_canonical_source() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path.clone(),
            claude_code_settings_path: claude_settings_path,
        };

        let report = ensure_local_source_integrations_with_paths(
            &test_config("codex_cli", SourceType::CodexCli),
            &paths,
        )
        .expect("Codex CLI integration should be applied");

        assert_eq!(
            report
                .codex_cli_stop_hook
                .as_ref()
                .map(|setup| setup.status),
            Some(CodexCliStopHookSetupStatus::CreatedConfig)
        );
        assert!(report.has_changes());
        assert!(
            fs::read_to_string(codex_config_path)
                .expect("config should be written")
                .contains(codex_cli_stop_hook_command().as_str())
        );
    }

    #[test]
    fn rejects_noncanonical_codex_cli_source_id() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };

        let config = unchecked_config(vec![SourceConfig {
            id: "my_codex".to_string(),
            source_type: SourceType::CodexCli,
        }]);

        let err = ensure_local_source_integrations_with_paths(&config, &paths)
            .expect_err("noncanonical Codex CLI source id should fail");

        assert!(
            err.to_string()
                .contains("Codex CLI source id must be `codex_cli`")
        );
        assert!(!paths.codex_cli_config_path.exists());
    }

    #[test]
    fn agent_hook_with_codex_cli_source_id_does_not_mutate_codex_cli_config() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };

        let report = ensure_local_source_integrations_with_paths(
            &test_config("codex_cli", SourceType::AgentHook),
            &paths,
        )
        .expect("generic agent_hook should not be treated as Codex CLI");

        assert_eq!(report, LocalSourceIntegrationReport::default());
        assert!(!paths.codex_cli_config_path.exists());
        assert!(!paths.claude_code_settings_path.exists());
    }

    #[test]
    fn applies_claude_code_hooks_for_canonical_source() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path.clone(),
        };

        let report = ensure_local_source_integrations_with_paths(
            &test_config("claude_code", SourceType::ClaudeCode),
            &paths,
        )
        .expect("Claude Code integration should be applied");

        assert_eq!(
            report.claude_code_hooks.as_ref().map(|setup| setup.status),
            Some(ClaudeCodeHooksSetupStatus::CreatedSettings)
        );
        assert!(report.has_changes());
        let settings: Value = serde_json::from_str(
            &fs::read_to_string(claude_settings_path).expect("settings should be written"),
        )
        .expect("settings should be valid JSON");
        for event_name in CLAUDE_CODE_HOOK_EVENTS {
            assert!(
                claude_settings_has_command(&settings, event_name),
                "{event_name} should include Agents Router hook"
            );
            assert_eq!(
                claude_settings_event_command_timeout(
                    &settings,
                    event_name,
                    claude_code_hook_command().as_str()
                ),
                Some(CLAUDE_CODE_HOOK_TIMEOUT_SECONDS),
                "{event_name} should set an explicit hook timeout"
            );
        }
    }

    #[test]
    fn rejects_noncanonical_claude_code_source_id() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };

        let config = unchecked_config(vec![SourceConfig {
            id: "my_claude".to_string(),
            source_type: SourceType::ClaudeCode,
        }]);

        let err = ensure_local_source_integrations_with_paths(&config, &paths)
            .expect_err("noncanonical Claude Code source id should fail");

        assert!(
            err.to_string()
                .contains("Claude Code source id must be `claude_code`")
        );
        assert!(!paths.claude_code_settings_path.exists());
    }

    #[test]
    fn agent_hook_with_claude_code_source_id_does_not_mutate_claude_settings() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };

        let report = ensure_local_source_integrations_with_paths(
            &test_config("claude_code", SourceType::AgentHook),
            &paths,
        )
        .expect("generic agent_hook should not be treated as Claude Code");

        assert_eq!(report, LocalSourceIntegrationReport::default());
        assert!(!paths.codex_cli_config_path.exists());
        assert!(!paths.claude_code_settings_path.exists());
    }

    #[test]
    fn rejects_later_noncanonical_claude_code_source_id_after_canonical_source() {
        let dir = tempdir().expect("tempdir should be created");
        let codex_config_path = dir.path().join("config.toml");
        let claude_settings_path = dir.path().join("settings.json");
        let paths = LocalSourceIntegrationPaths {
            codex_cli_config_path: codex_config_path,
            claude_code_settings_path: claude_settings_path,
        };
        let mut config = test_config("claude_code", SourceType::ClaudeCode);
        config.sources.push(SourceConfig {
            id: "my_claude".to_string(),
            source_type: SourceType::ClaudeCode,
        });

        let err = ensure_local_source_integrations_with_paths(&config, &paths)
            .expect_err("all Claude Code source ids should be canonical");

        assert!(
            err.to_string()
                .contains("Claude Code source id must be `claude_code`")
        );
        assert!(!paths.claude_code_settings_path.exists());
        assert!(!paths.codex_cli_config_path.exists());
    }

    #[test]
    fn creates_claude_code_settings_with_all_hooks() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join(".claude").join("settings.json");

        let setup = ensure_claude_code_hooks(&path).expect("hooks should be configured");

        assert_eq!(setup.status, ClaudeCodeHooksSetupStatus::CreatedSettings);
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(path).expect("settings should be written"))
                .expect("settings should be valid JSON");
        for event_name in CLAUDE_CODE_HOOK_EVENTS {
            assert!(
                claude_settings_has_command(&settings, event_name),
                "{event_name} should include Agents Router hook"
            );
            assert_eq!(
                claude_settings_event_command_timeout(
                    &settings,
                    event_name,
                    claude_code_hook_command().as_str()
                ),
                Some(CLAUDE_CODE_HOOK_TIMEOUT_SECONDS)
            );
        }
    }

    #[test]
    fn preserves_existing_claude_code_settings_and_hooks() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
  "permissions": {
    "allow": [
      "Bash(git status:*)"
    ]
  },
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo existing"
          }
        ]
      }
    ]
  }
}
"#,
        )
        .expect("existing settings should be written");

        let setup = ensure_claude_code_hooks(&path).expect("hooks should be configured");

        assert_eq!(setup.status, ClaudeCodeHooksSetupStatus::UpdatedSettings);
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(path).expect("settings should be written"))
                .expect("settings should be valid JSON");
        assert_eq!(
            settings["permissions"]["allow"][0],
            Value::String("Bash(git status:*)".to_string())
        );
        assert!(claude_settings_event_has_command(
            &settings,
            "Stop",
            "echo existing"
        ));
        for event_name in CLAUDE_CODE_HOOK_EVENTS {
            assert!(
                claude_settings_has_command(&settings, event_name),
                "{event_name} should include Agents Router hook"
            );
        }
    }

    #[test]
    fn rejects_claude_code_settings_when_all_hooks_are_disabled() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
  "disableAllHooks": true
}
"#,
        )
        .expect("settings should be written");

        let err =
            ensure_claude_code_hooks(&path).expect_err("disabled hooks should fail explicitly");

        assert!(err.to_string().contains("disableAllHooks"));
        let settings: Value = serde_json::from_str(
            &fs::read_to_string(path).expect("settings should remain readable"),
        )
        .expect("settings should remain valid JSON");
        assert_eq!(settings["disableAllHooks"], Value::Bool(true));
        assert!(settings.get("hooks").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_existing_claude_code_settings_permissions() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
  "permissions": {
    "allow": []
  }
}
"#,
        )
        .expect("settings should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("test permissions should be set");

        ensure_claude_code_hooks(&path).expect("hooks should be configured");

        let mode = fs::metadata(path)
            .expect("settings metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn is_idempotent_after_claude_code_hooks_exist() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("settings.json");

        ensure_claude_code_hooks(&path).expect("hooks should be configured");
        let first = fs::read_to_string(&path).expect("settings should be written");

        let setup = ensure_claude_code_hooks(&path).expect("second setup should succeed");
        let second = fs::read_to_string(&path).expect("settings should be written");

        assert_eq!(setup.status, ClaudeCodeHooksSetupStatus::AlreadyConfigured);
        assert_eq!(second, first);
        assert_eq!(
            second.matches(claude_code_hook_command().as_str()).count(),
            4
        );
    }

    #[test]
    fn fails_without_overwriting_invalid_claude_code_settings_json() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"hooks":"#).expect("invalid settings should be written");

        let err =
            ensure_claude_code_hooks(&path).expect_err("invalid Claude Code settings should fail");

        assert!(
            err.to_string()
                .contains("failed to parse Claude Code settings")
        );
        assert_eq!(
            fs::read_to_string(path).expect("settings should still exist"),
            r#"{"hooks":"#
        );
    }

    #[test]
    fn creates_codex_config_with_stop_hook() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join(".codex").join("config.toml");

        let setup = ensure_codex_cli_stop_hook(&path).expect("hook should be configured");

        assert_eq!(setup.status, CodexCliStopHookSetupStatus::CreatedConfig);
        let raw = fs::read_to_string(path).expect("config should be written");
        assert!(raw.contains("[features]"));
        assert!(raw.contains("hooks = true"));
        assert!(!raw.contains("codex_hooks"));
        assert!(raw.contains("[[hooks.Stop]]"));
        assert!(raw.contains("[[hooks.Stop.hooks]]"));
        assert!(raw.contains(codex_cli_stop_hook_command().as_str()));
    }

    #[test]
    fn preserves_existing_notify_when_adding_stop_hook() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"notify = ["/Applications/Existing.app/Contents/MacOS/helper", "turn-ended"]
"#,
        )
        .expect("existing config should be written");

        let setup = ensure_codex_cli_stop_hook(&path).expect("hook should be configured");

        assert_eq!(setup.status, CodexCliStopHookSetupStatus::UpdatedConfig);
        let raw = fs::read_to_string(path).expect("config should be written");
        assert!(raw.contains(
            r#"notify = ["/Applications/Existing.app/Contents/MacOS/helper", "turn-ended"]"#
        ));
        assert!(raw.contains(codex_cli_stop_hook_command().as_str()));
    }

    #[test]
    fn keeps_existing_stop_hooks_and_appends_agents_router_hook() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"[features]
codex_hooks = true

[[hooks.Stop]]
[[hooks.Stop.hooks]]
type = "command"
command = "echo existing"
"#,
        )
        .expect("existing config should be written");

        ensure_codex_cli_stop_hook(&path).expect("hook should be configured");

        let raw = fs::read_to_string(path).expect("config should be written");
        assert!(raw.contains("echo existing"));
        assert!(raw.contains("hooks = true"));
        assert!(!raw.contains("codex_hooks"));
        assert!(raw.contains(codex_cli_stop_hook_command().as_str()));
    }

    #[test]
    fn is_idempotent_after_stop_hook_exists() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");

        ensure_codex_cli_stop_hook(&path).expect("hook should be configured");
        let first = fs::read_to_string(&path).expect("config should be written");

        let setup = ensure_codex_cli_stop_hook(&path).expect("second setup should succeed");
        let second = fs::read_to_string(&path).expect("config should be written");

        assert_eq!(setup.status, CodexCliStopHookSetupStatus::AlreadyConfigured);
        assert_eq!(second, first);
        assert_eq!(
            second
                .matches(codex_cli_stop_hook_command().as_str())
                .count(),
            1
        );
    }

    #[test]
    fn enables_hooks_when_existing_hook_is_present_but_feature_is_off() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");
        let command = codex_cli_stop_hook_command();
        fs::write(
            &path,
            format!(
                r#"[features]
hooks = false

[[hooks.Stop]]
[[hooks.Stop.hooks]]
type = "command"
command = "{command}"
"#
            ),
        )
        .expect("existing config should be written");

        let setup = ensure_codex_cli_stop_hook(&path).expect("hook should be configured");

        assert_eq!(setup.status, CodexCliStopHookSetupStatus::UpdatedConfig);
        let raw = fs::read_to_string(path).expect("config should be written");
        assert!(raw.contains("hooks = true"));
        assert_eq!(
            raw.matches(codex_cli_stop_hook_command().as_str()).count(),
            1
        );
    }

    #[test]
    fn removes_deprecated_codex_hooks_feature_when_config_is_migrated() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");
        let command = codex_cli_stop_hook_command();
        fs::write(
            &path,
            format!(
                r#"[features]
codex_hooks = true

[[hooks.Stop]]
[[hooks.Stop.hooks]]
type = "command"
command = "{command}"
"#
            ),
        )
        .expect("existing config should be written");

        let setup = ensure_codex_cli_stop_hook(&path).expect("hook should be configured");

        assert_eq!(setup.status, CodexCliStopHookSetupStatus::UpdatedConfig);
        let raw = fs::read_to_string(path).expect("config should be written");
        assert!(raw.contains("hooks = true"));
        assert!(!raw.contains("codex_hooks"));
        assert_eq!(
            raw.matches(codex_cli_stop_hook_command().as_str()).count(),
            1
        );
    }

    #[test]
    fn fails_without_overwriting_invalid_toml() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");
        fs::write(&path, "notify = [").expect("invalid config should be written");

        let err = ensure_codex_cli_stop_hook(&path).expect_err("invalid TOML should fail");

        assert!(err.to_string().contains("failed to parse Codex CLI config"));
        assert_eq!(
            fs::read_to_string(path).expect("config should still exist"),
            "notify = ["
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_existing_codex_cli_config_permissions() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.toml");
        fs::write(&path, "").expect("config should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("test permissions should be set");

        ensure_codex_cli_stop_hook(&path).expect("hook should be configured");

        let mode = fs::metadata(path)
            .expect("config metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    fn claude_settings_has_command(settings: &Value, event_name: &str) -> bool {
        claude_settings_event_has_command(settings, event_name, claude_code_hook_command().as_str())
    }

    fn claude_settings_event_has_command(
        settings: &Value,
        event_name: &str,
        command: &str,
    ) -> bool {
        settings["hooks"][event_name]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .filter_map(|hook| hook.get("hooks"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_object)
            .any(|hook| {
                hook.get("type").and_then(Value::as_str) == Some("command")
                    && hook.get("command").and_then(Value::as_str) == Some(command)
            })
    }

    fn claude_settings_event_command_timeout(
        settings: &Value,
        event_name: &str,
        command: &str,
    ) -> Option<i64> {
        settings["hooks"][event_name]
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|hook| hook.get("hooks"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_object)
            .find(|hook| {
                hook.get("type").and_then(Value::as_str) == Some("command")
                    && hook.get("command").and_then(Value::as_str) == Some(command)
            })
            .and_then(|hook| hook.get("timeout"))
            .and_then(Value::as_i64)
    }

    fn test_config(source_id: &str, source_type: SourceType) -> ValidatedConfig {
        let mut provider = RawProviderConfig::new("phone", ProviderType::Ntfy);
        provider.server = Some("https://ntfy.sh".to_string());
        provider.topic = Some("agents-router-test".to_string());

        RawConfig {
            schema_version: 1,
            cli: CliConfig::default(),
            log: LogConfig::default(),
            notification: NotificationConfig::default(),
            sources: vec![SourceConfig {
                id: source_id.to_string(),
                source_type,
            }],
            providers: vec![provider],
            routes: vec![RouteConfig::new(
                vec![source_id.to_string()],
                vec!["phone".to_string()],
            )],
        }
        .validate()
        .expect("test config should validate")
    }

    fn unchecked_config(sources: Vec<SourceConfig>) -> ValidatedConfig {
        let mut config = test_config("agents_router", SourceType::AgentsRouter);
        config.sources = sources;
        config
    }
}
