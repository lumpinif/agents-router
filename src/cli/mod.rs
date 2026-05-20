use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use qrcode::QrCode;
use qrcode::render::unicode;
use tokio::task::JoinSet;
use tokio::time::sleep;

use agents_router::agent_integration_catalog::{
    HookCommandTemplate, RuntimePlatform, SetupIntegrationKind, SourceIngestFormat,
    agent_integration_descriptor, agent_integration_for_source,
    default_agent_integration_for_platform, setup_agent_integration_descriptors_for_platform,
};
use agents_router::config::{
    AnswerDetail, CliConfig, CliLanguage, ConfigError, EmailSmtpSecurity,
    LoadedConfig as ParsedConfig, PromptDetail, ProviderType, RawConfig, RawProviderConfig,
    RouteConfig, SourceType, ValidatedConfig,
};
use agents_router::delivery_safety::DeliverySafetyGuard;
use agents_router::i18n::{I18n, Text};
use agents_router::legacy;
use agents_router::local_ingress::{self, LocalSignalEvent};
use agents_router::local_integrations::{
    self, ClaudeCodeHooksSetupStatus, CodexCliStopHookSetupStatus, LocalSourceIntegrationReport,
};
use agents_router::local_open_bridge;
use agents_router::notification_detail_policy::{
    NotificationDetailSupport, answer_detail_support, prompt_detail_support,
};
#[cfg(target_os = "macos")]
use agents_router::paths::pid_file_path;
use agents_router::paths::{
    app_support_dir_path, codex_sessions_dir_path, default_config_file_path, ingress_endpoint,
    log_file_path, service_metadata_path,
};
#[cfg(target_os = "macos")]
use agents_router::process::{StopOutcome, SystemProcessManager, stop_with_manager};
use agents_router::provider_catalog::{
    MessageLimitUnit, MessageSurface, ProviderMessageConstraint, default_setup_provider_type,
    provider_descriptor, setup_provider_descriptors,
};
use agents_router::providers::build_providers;
use agents_router::runtime::{
    RuntimeState, ensure_sources_supported_on_current_platform, reload_config_on_change,
};
use agents_router::service::{
    PlatformServiceManager, ServiceDefinition, ServiceStartOutcome, ServiceStopOutcome,
    load_metadata,
};
use agents_router::setup;
use agents_router::signal::{SignalLifecycle, SignalLifecycleStatus};
use agents_router::sources::{
    agent_hook, claude_code, codex_cli, codex_desktop, gemini_cli, github_copilot_cli, opencode_cli,
};
use agents_router::update;

mod setup_flow;
mod update_flow;

use setup_flow::*;

const SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const START_PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

struct ProgressLine {
    message: &'static str,
    spinner: Option<ProgressBar>,
}

enum ProgressOutcome {
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetupProviderSummary {
    provider_name: &'static str,
    fields: Vec<SetupProviderSummaryField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetupProviderSummaryField {
    label: &'static str,
    value: String,
    tone: SetupProviderSummaryTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupProviderSummaryTone {
    Plain,
    Success,
    Warning,
}

impl ProgressLine {
    fn start(message: &'static str) -> anyhow::Result<Self> {
        if !io::stderr().is_terminal() {
            println!("{message}...");
            return Ok(Self {
                message,
                spinner: None,
            });
        }

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::with_template("{spinner} {msg}")
                .context("failed to build progress style")?
                .tick_strings(&["-", "\\", "|", "/"]),
        );
        spinner.set_message(message);
        spinner.enable_steady_tick(START_PROGRESS_INTERVAL);

        Ok(Self {
            message,
            spinner: Some(spinner),
        })
    }

    fn finish(self, outcome: ProgressOutcome) -> anyhow::Result<()> {
        let Some(spinner) = self.spinner else {
            return Ok(());
        };

        spinner.finish_and_clear();

        match outcome {
            ProgressOutcome::Ready => println!("{} {}", self.message, style("ready.").green()),
            ProgressOutcome::Failed => println!("{} {}", self.message, style("failed.").red()),
        }
        Ok(())
    }
}

#[derive(Debug, Parser)]
#[command(name = "agents-router")]
#[command(version)]
#[command(about = "Local-first notification routing for coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Start the local notification service")]
    Start {
        #[arg(long, value_name = "PATH", help = "Path to the config file")]
        config: Option<PathBuf>,
    },
    #[command(about = "Set up Agents Router and start the service")]
    Setup {
        #[arg(long, value_name = "PATH", help = "Path to the config file")]
        config: Option<PathBuf>,
    },
    #[command(about = "Run the local notification service in the foreground")]
    Watch {
        #[arg(long, value_name = "PATH", help = "Path to the config file")]
        config: Option<PathBuf>,
    },
    #[command(about = "Restart the local notification service")]
    Restart {
        #[arg(long, value_name = "PATH", help = "Path to the config file")]
        config: Option<PathBuf>,
    },
    #[command(about = "Stop the local notification service")]
    Stop,
    #[command(about = "Stop the service and remove Agents Router local files")]
    Uninstall,
    #[command(about = "Show the local notification service status")]
    Status,
    #[command(about = "Manage delivery safety state")]
    Safety {
        #[command(subcommand)]
        command: SafetyCommand,
    },
    #[command(about = "Print the agents-router version")]
    Version,
    #[command(about = "Submit a hook event to the running local service")]
    Emit {
        #[arg(long, help = "Configured source id for this event")]
        source: String,
        #[arg(long, help = "Notification title")]
        title: String,
        #[arg(long, help = "Notification body")]
        body: String,
        #[arg(
            long,
            value_name = "MILLISECONDS",
            help = "Task duration in milliseconds, when your wrapper can provide it"
        )]
        duration_ms: Option<u64>,
    },
    #[command(about = "Submit a structured hook event from stdin")]
    Ingest {
        #[arg(long, help = "Configured source id for this event")]
        source: String,
        #[arg(long, value_parser = parse_source_ingest_format, help = "Structured hook input format")]
        format: SourceIngestFormat,
    },
    #[command(hide = true)]
    MigrateLegacy {
        #[arg(long, value_name = "PATH", help = "Path to the config file")]
        config: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum SafetyCommand {
    #[command(about = "Clear the delivery safety pause state")]
    Reset,
}

fn parse_source_ingest_format(value: &str) -> Result<SourceIngestFormat, String> {
    value.parse()
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Start {
            config: config_path,
        } => {
            let allow_setup = config_path.is_none();
            let config_path = resolve_config_path(config_path)?;
            if allow_setup {
                print_legacy_migration(legacy::migrate_legacy_installation(&config_path)?);
            }
            let loaded = load_config(&config_path, allow_setup).await?;
            run_start_service(&config_path, loaded, false, true).await
        }
        Command::Setup {
            config: config_path,
        } => {
            let config_path = resolve_config_path(config_path)?;
            print_legacy_migration(legacy::migrate_legacy_installation(&config_path)?);
            let loaded = match run_provider_setup(&config_path).await? {
                ProviderSetupOutcome::Loaded(loaded) => *loaded,
                ProviderSetupOutcome::Exit(code) => std::process::exit(code),
            };
            run_start_service(&config_path, loaded, true, true).await
        }
        Command::Watch {
            config: config_path,
        } => {
            let config_path = resolve_config_path(config_path)?;
            let loaded = load_config(&config_path, false).await?;
            init_tracing(&loaded.raw.log.level)?;
            run_watch(&config_path, loaded.validated).await
        }
        Command::Restart {
            config: config_path,
        } => {
            let config_path = resolve_restart_config_path(config_path)?;
            let loaded = load_config(&config_path, false).await?;
            run_start_service(&config_path, loaded, true, false).await
        }
        Command::Stop => run_stop(),
        Command::Uninstall => run_uninstall(),
        Command::Status => run_status(),
        Command::Safety { command } => run_safety(command).await,
        Command::Version => {
            println!("agents-router {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Emit {
            source,
            title,
            body,
            duration_ms,
        } => run_emit(&source, title, body, duration_ms).await,
        Command::Ingest { source, format } => run_ingest(&source, format).await,
        Command::MigrateLegacy {
            config: config_path,
        } => {
            let config_path = resolve_config_path(config_path)?;
            print_legacy_migration(legacy::migrate_legacy_installation(&config_path)?);
            Ok(())
        }
    }
}

fn print_legacy_migration(outcome: legacy::LegacyMigrationOutcome) {
    if !outcome.changed() {
        return;
    }

    if outcome.migrated_config {
        println!("Migrated existing Agents Notifier config to Agents Router.");
    }
    if outcome.removed_service {
        println!("Removed existing Agents Notifier service.");
    }
    if outcome.removed_files > 0 {
        println!("Removed existing Agents Notifier local files.");
    }
    for path in outcome.retained_cargo_binaries {
        println!(
            "Found existing Agents Notifier Cargo binary and left it in place: {}",
            path.display()
        );
    }
}

fn resolve_config_path(config: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match config {
        Some(path) => Ok(path),
        None => default_config_file_path(),
    }
}

fn resolve_restart_config_path(config: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(path) = config {
        return Ok(path);
    }

    let metadata_path = service_metadata_path()?;
    if metadata_path.exists() {
        let metadata = load_metadata(&metadata_path)?;
        Ok(PathBuf::from(metadata.config_path))
    } else {
        default_config_file_path()
    }
}

async fn run_start_service(
    config_path: &Path,
    loaded: LoadedConfig,
    force_restart: bool,
    offer_test_after_start: bool,
) -> anyhow::Result<()> {
    let i18n = I18n::new(loaded.raw.cli.language);
    ensure_sources_supported_on_current_platform(&loaded.validated)?;
    build_providers(&loaded.validated).context("provider setup failed")?;
    let guided_setup = loaded.guided_setup.is_some();
    let integration_report =
        local_integrations::ensure_local_source_integrations(&loaded.validated)?;
    if guided_setup || integration_report.has_changes() {
        print_local_source_integration_report(&integration_report, i18n);
    }
    if loaded.guided_setup.is_some() {
        println!();
    }
    let progress = ProgressLine::start("Starting agents-router")?;
    let start_result = async {
        cleanup_legacy_background_service()?;

        let manager = PlatformServiceManager::system()?;
        let definition = build_service_definition(config_path)?;
        let metadata_path = service_metadata_path()?;
        let outcome = if force_restart {
            manager.stop()?;
            cleanup_ingress_endpoint_if_exists()?;
            manager.install_or_update(&definition, &metadata_path)?
        } else {
            manager.install_or_update(&definition, &metadata_path)?
        };
        wait_for_service(&ingress_endpoint()?).await?;

        let status = manager.status()?;
        anyhow::Ok((outcome, status.pid, definition.log_file, status.platform))
    }
    .await;

    let (outcome, pid, log_file, manager_name) = match start_result {
        Ok(started) => {
            progress.finish(ProgressOutcome::Ready)?;
            started
        }
        Err(error) => {
            progress.finish(ProgressOutcome::Failed)?;
            return Err(error);
        }
    };

    print_service_start_outcome(outcome, pid, &log_file, manager_name, i18n);

    if let Some(setup) = loaded.guided_setup {
        finish_guided_setup(setup, i18n).await?;
    } else {
        print_notification_targets(&loaded.raw, i18n);
        if offer_test_after_start {
            offer_test_notification(&loaded.raw, i18n).await?;
        }
    }

    Ok(())
}

fn build_service_definition(config_path: &Path) -> anyhow::Result<ServiceDefinition> {
    let binary_path = resolved_current_exe()?;
    let working_dir = std::env::current_dir().context("failed to detect working directory")?;
    let config_path = if config_path.is_absolute() {
        config_path.to_path_buf()
    } else {
        working_dir.join(config_path)
    };
    let log_file = log_file_path()?;
    let home = home_env_value()?;
    let env_path = std::env::var("PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(default_service_env_path);

    Ok(ServiceDefinition {
        binary_path,
        config_path,
        working_dir,
        log_file,
        home,
        env_path,
    })
}

fn default_service_env_path() -> String {
    #[cfg(windows)]
    return r"C:\Windows\System32;C:\Windows;C:\Windows\System32\WindowsPowerShell\v1.0"
        .to_string();

    #[cfg(not(windows))]
    return "/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin".to_string();
}

fn home_env_value() -> anyhow::Result<String> {
    #[cfg(windows)]
    let name = "USERPROFILE";

    #[cfg(not(windows))]
    let name = "HOME";

    std::env::var(name).with_context(|| format!("{name} is not set"))
}

fn resolved_current_exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    Ok(fs::canonicalize(&exe).unwrap_or(exe))
}

fn print_service_start_outcome(
    outcome: ServiceStartOutcome,
    pid: Option<u32>,
    log_file: &Path,
    manager_name: &str,
    i18n: I18n,
) {
    print_section(i18n.text(Text::ServiceSection));
    match outcome {
        ServiceStartOutcome::AlreadyRunning => {
            print_success_field("status", i18n.text(Text::ServiceStatusAlreadyRunning));
        }
        ServiceStartOutcome::Started => {
            print_success_field("status", i18n.text(Text::ServiceStatusRunning));
        }
        ServiceStartOutcome::UpdatedAndStarted => {
            print_success_field("status", i18n.text(Text::ServiceStatusUpdatedAndRestarted));
        }
    }
    print_field("manager", manager_name);
    if let Some(pid) = pid {
        print_field("pid", pid);
    }
    print_path_field("log", log_file.display());
}

fn print_notification_targets(config: &RawConfig, i18n: I18n) {
    let agents = configured_agents(config);
    let subscriptions = setup::ntfy_subscriptions(config);
    let feishu_lark_targets = setup::feishu_lark_targets(config);
    let webhook_targets = setup::webhook_targets(config);
    let pushover_targets = setup::pushover_targets(config);
    let slack_targets = setup::slack_targets(config);
    let discord_targets = setup::discord_targets(config);
    let telegram_targets = setup::telegram_targets(config);
    let whatsapp_targets = setup::whatsapp_targets(config);
    let wechat_targets = setup::wechat_targets(config);
    let microsoft_teams_targets = setup::microsoft_teams_targets(config);
    let email_smtp_targets = setup::email_smtp_targets(config);

    print_section(i18n.text(Text::NotificationsSection));
    print_field(
        i18n.text(Text::AnswerDetailField),
        i18n.answer_detail(config.notification.answer_detail),
    );
    print_field(
        i18n.text(Text::PromptDetailField),
        i18n.prompt_detail(config.notification.prompt_detail),
    );

    if !agents.is_empty() {
        print_section(i18n.text(Text::WatchedAgentsSection));
        for agent in &agents {
            print_success_field(i18n.text(Text::AgentField), agent);
        }
    }

    if !subscriptions.is_empty() {
        print_section(i18n.text(Text::PhoneSubscriptionSection));
        for subscription in &subscriptions {
            print_field("server", &subscription.server);
            print_field("topic", &subscription.topic);
        }
    }

    if !feishu_lark_targets.is_empty() {
        print_section("Feishu/Lark custom bot");
        for target in &feishu_lark_targets {
            print_field("provider", &target.provider_id);
            print_field("webhook", &target.webhook_host);
            print_field(
                "signature verification",
                if target.signed {
                    style(i18n.text(Text::Configured)).green()
                } else {
                    style(i18n.text(Text::NotConfigured)).yellow()
                },
            );
        }
    }

    if !webhook_targets.is_empty() {
        print_section("Webhook");
        for target in &webhook_targets {
            print_field("provider", &target.provider_id);
            print_field("endpoint", &target.webhook_host);
        }
    }

    if !pushover_targets.is_empty() {
        print_section("Pushover");
        for target in &pushover_targets {
            print_field("provider", &target.provider_id);
            print_field(
                "device",
                target
                    .device
                    .as_deref()
                    .unwrap_or(i18n.text(Text::AllDevices)),
            );
            print_field(
                "sound",
                target
                    .sound
                    .as_deref()
                    .unwrap_or(i18n.text(Text::AccountDefault)),
            );
        }
    }

    if !slack_targets.is_empty() {
        print_section("Slack");
        for target in &slack_targets {
            print_field("provider", &target.provider_id);
            print_field("webhook", &target.webhook_host);
        }
    }

    if !discord_targets.is_empty() {
        print_section("Discord");
        for target in &discord_targets {
            print_field("provider", &target.provider_id);
            print_field("webhook", &target.webhook_host);
        }
    }

    if !telegram_targets.is_empty() {
        print_section("Telegram");
        for target in &telegram_targets {
            print_field("provider", &target.provider_id);
            print_field("chat id", &target.chat_id);
        }
    }

    if !whatsapp_targets.is_empty() {
        print_section("WhatsApp");
        for target in &whatsapp_targets {
            print_field("provider", &target.provider_id);
            print_field("recipient", &target.recipient_phone_number);
        }
    }

    if !wechat_targets.is_empty() {
        print_section(localized(i18n, "WeChat", "微信"));
        for target in &wechat_targets {
            print_field("provider", &target.provider_id);
            print_field("endpoint", &target.base_url_host);
            print_field("recipient", &target.recipient_user_id);
        }
    }

    if !microsoft_teams_targets.is_empty() {
        print_section("Microsoft Teams");
        for target in &microsoft_teams_targets {
            print_field("provider", &target.provider_id);
            print_field("webhook", &target.webhook_host);
        }
    }

    if !email_smtp_targets.is_empty() {
        print_section("Email SMTP");
        for target in &email_smtp_targets {
            print_field("provider", &target.provider_id);
            print_field("server", format!("{}:{}", target.host, target.port));
            print_field("from", &target.from);
            print_field("to", target.to.join(", "));
        }
    }
}

fn configured_agents(config: &RawConfig) -> Vec<String> {
    config
        .sources
        .iter()
        .filter_map(|source| match source.source_type {
            SourceType::CodexDesktop => Some(
                agent_integration_descriptor(setup::AgentIntegrationId::CodexDesktop)
                    .source_capability
                    .display_name
                    .to_string(),
            ),
            SourceType::CodexCli => Some(
                agent_integration_descriptor(setup::AgentIntegrationId::CodexCli)
                    .source_capability
                    .display_name
                    .to_string(),
            ),
            SourceType::ClaudeCode => Some(
                agent_integration_descriptor(setup::AgentIntegrationId::ClaudeCode)
                    .source_capability
                    .display_name
                    .to_string(),
            ),
            SourceType::AgentHook => Some(
                agent_integration_for_source(&source.id, source.source_type)
                    .map(|descriptor| descriptor.source_capability.display_name.to_string())
                    .unwrap_or_else(|| format!("Agent hook ({})", source.id)),
            ),
            SourceType::AgentsRouter => None,
        })
        .collect()
}

fn first_configured_agent(config: &RawConfig) -> Option<setup::AgentIntegrationId> {
    config
        .sources
        .iter()
        .find_map(|source| match source.source_type {
            SourceType::CodexDesktop => Some(setup::AgentIntegrationId::CodexDesktop),
            SourceType::CodexCli => Some(setup::AgentIntegrationId::CodexCli),
            SourceType::ClaudeCode => Some(setup::AgentIntegrationId::ClaudeCode),
            SourceType::AgentHook => agent_integration_for_source(&source.id, source.source_type)
                .map(|descriptor| descriptor.id),
            SourceType::AgentsRouter => None,
        })
}

fn first_configured_provider(config: &RawConfig) -> Option<ProviderType> {
    config
        .providers
        .first()
        .map(|provider| provider.provider_type)
}

fn first_provider_of_type(
    config: &RawConfig,
    provider_type: ProviderType,
) -> Option<&RawProviderConfig> {
    config
        .providers
        .iter()
        .find(|provider| provider.provider_type.as_str() == provider_type.as_str())
}

fn configured_provider_url(provider: &RawProviderConfig) -> Option<String> {
    provider.url.clone()
}

fn configured_provider_secret(provider: &RawProviderConfig) -> Option<String> {
    provider.secret.clone()
}

fn email_smtp_security_display(security: EmailSmtpSecurity) -> &'static str {
    match security {
        EmailSmtpSecurity::Starttls => "STARTTLS",
        EmailSmtpSecurity::ImplicitTls => "Implicit TLS",
    }
}

fn default_email_smtp_port(security: EmailSmtpSecurity) -> u16 {
    match security {
        EmailSmtpSecurity::Starttls => 587,
        EmailSmtpSecurity::ImplicitTls => 465,
    }
}

fn safe_url_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "configured".to_string())
}

fn print_status_notification_targets(config_path: &Path) {
    match ParsedConfig::from_path(config_path) {
        Ok(config) => {
            let i18n = I18n::new(config.raw.cli.language);
            print_notification_targets(&config.raw, i18n);
        }
        Err(error) => {
            print_section("Config");
            print_field("status", style(error.to_string()).red());
        }
    }
}

fn run_status() -> anyhow::Result<()> {
    let manager = PlatformServiceManager::system()?;
    let service_file = manager.service_file_path()?;
    let metadata_path = service_metadata_path()?;
    let endpoint = ingress_endpoint()?;
    let status = manager.status()?;
    let metadata = load_metadata(&metadata_path).ok();

    print_heading("agents-router status");
    print_section("Service");
    print_field("manager", status.platform);
    print_bool_field("installed", status.installed);
    print_bool_field("running", status.running);
    if let Some(pid) = status.pid {
        print_field("pid", pid);
    }
    if let Some(service_file) = service_file {
        print_path_field("service file", service_file.display());
    }
    print_path_field("ingress", endpoint.display());
    print_bool_field(
        "ingress ready",
        local_ingress_ready_now(&endpoint, status.running),
    );

    let config_path = metadata
        .as_ref()
        .map(|metadata| PathBuf::from(&metadata.config_path))
        .unwrap_or(default_config_file_path()?);
    let log_file = metadata
        .as_ref()
        .map(|metadata| PathBuf::from(&metadata.log_file))
        .unwrap_or(log_file_path()?);

    print_section("Files");
    print_path_field("config", config_path.display());
    print_path_field("log", log_file.display());
    print_status_notification_targets(&config_path);
    print_delivery_safety_status();

    Ok(())
}

async fn run_safety(command: SafetyCommand) -> anyhow::Result<()> {
    match command {
        SafetyCommand::Reset => run_safety_reset().await,
    }
}

async fn run_safety_reset() -> anyhow::Result<()> {
    let endpoint = ingress_endpoint()?;
    if local_ingress::is_ready(&endpoint).await {
        local_ingress::reset_delivery_safety(&endpoint)
            .await
            .context("failed to reset delivery safety through the running service")?;
    } else {
        DeliverySafetyGuard::reset_default_file()
            .context("failed to reset persisted delivery safety state")?;
    }

    println!("Delivery safety reset.");
    Ok(())
}

fn print_delivery_safety_status() {
    print_section("Delivery Safety");
    match DeliverySafetyGuard::default_file_status() {
        Ok(status) => {
            print_bool_field("paused", status.paused);
            if let Some(pause) = status.pause {
                print_field("reason", pause.reason);
                print_field("paused at", pause.paused_at);
            }
        }
        Err(error) => {
            print_field("status", style(error.to_string()).red());
        }
    }
}

fn local_ingress_ready_now(
    endpoint: &agents_router::paths::IngressEndpoint,
    service_running: bool,
) -> bool {
    #[cfg(unix)]
    let _ = service_running;

    match endpoint {
        #[cfg(unix)]
        agents_router::paths::IngressEndpoint::UnixSocket(path) => path.exists(),
        #[cfg(windows)]
        agents_router::paths::IngressEndpoint::WindowsNamedPipe(_) => service_running,
    }
}

#[cfg(not(target_os = "macos"))]
fn cleanup_legacy_background_service() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn cleanup_legacy_background_service() -> anyhow::Result<()> {
    let pid_file = pid_file_path()?;
    match stop_with_manager(&pid_file, &SystemProcessManager)? {
        StopOutcome::MissingPidFile => {}
        StopOutcome::RemovedStalePidFile { .. } | StopOutcome::Stopped { .. } => {
            cleanup_ingress_endpoint_if_exists()?;
        }
    }

    Ok(())
}

async fn finish_guided_setup(setup: GuidedSetup, i18n: I18n) -> anyhow::Result<()> {
    let endpoint = ingress_endpoint()?;
    wait_for_service(&endpoint).await?;

    match setup {
        GuidedSetup::Ntfy { agent, topic } => {
            println!();
            println!("{}", i18n.text(Text::NextPhone));
            println!("{}", i18n.text(Text::NtfyFinishStep1));
            println!("{}", i18n.text(Text::NtfyFinishStep2));
            println!("{}", i18n.text(Text::NtfyFinishStep3));
            println!("4. Topic: {topic}");
            println!();
            wait_for_enter(i18n.text(Text::SendTestPromptNtfy))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::FeishuLark { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextFeishuLark));
            wait_for_enter(i18n.text(Text::SendTestPromptFeishuLark))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Webhook { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextWebhook));
            wait_for_enter(i18n.text(Text::SendTestPromptWebhook))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Pushover { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextPushover));
            wait_for_enter(i18n.text(Text::SendTestPromptPushover))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Slack { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextSlack));
            wait_for_enter(i18n.text(Text::SendTestPromptSlack))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Discord { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextDiscord));
            wait_for_enter(i18n.text(Text::SendTestPromptDiscord))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Telegram { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextTelegram));
            wait_for_enter(i18n.text(Text::SendTestPromptTelegram))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Whatsapp { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextWhatsapp));
            wait_for_enter(i18n.text(Text::SendTestPromptWhatsapp))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::Wechat { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextWechat));
            wait_for_enter(i18n.text(Text::SendTestPromptWechat))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::MicrosoftTeams { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextTeams));
            wait_for_enter(i18n.text(Text::SendTestPromptTeams))?;
            print_source_integration_setup_note(agent, i18n);
        }
        GuidedSetup::EmailSmtp { agent } => {
            println!();
            println!("{}", i18n.text(Text::NextEmail));
            wait_for_enter(i18n.text(Text::SendTestPromptEmail))?;
            print_source_integration_setup_note(agent, i18n);
        }
    }

    send_test_notification().await?;

    println!("{}", style(i18n.text(Text::TestSent)).green());
    if prompt_confirm(i18n.text(Text::DidItArrive), true)? {
        println!("{}", style(i18n.text(Text::SetupComplete)).green());
    } else {
        println!("{}", style(i18n.text(Text::TestDidNotArrive)).yellow());
        println!("{}", i18n.text(Text::CheckProviderSettingsAndRetest));
        println!(
            "agents-router emit --source agents_router --title \"Agents Router\" --body \"Test notification from your computer.\""
        );
    }

    Ok(())
}

fn print_source_integration_setup_note(source_integration: setup::AgentIntegrationId, i18n: I18n) {
    let descriptor = agent_integration_descriptor(source_integration);

    match descriptor.source_capability.setup_integration {
        SetupIntegrationKind::CodexDesktopWatch => {
            if i18n.language() == CliLanguage::SimplifiedChinese {
                println!("Agents Router 会监听这台电脑上的 Codex Desktop 完成事件。");
            } else {
                println!("Agents Router will watch Codex Desktop completed jobs on this computer.");
            }
        }
        SetupIntegrationKind::CodexCliStopHook => {
            if i18n.language() == CliLanguage::SimplifiedChinese {
                println!("Agents Router 已配置 Codex CLI Stop hook，用来接收完成事件。");
                println!(
                    "setup 不会覆盖现有 `notify`；如果你手动改配置，请保持 Stop hook 指向 Agents Router。"
                );
            } else {
                println!("Agents Router configured the Codex CLI Stop hook for completion events.");
                println!(
                    "Setup does not overwrite existing `notify`; if you edit config manually, keep the Stop hook pointed at Agents Router."
                );
            }
        }
        SetupIntegrationKind::ClaudeCodeHooks => {
            if i18n.language() == CliLanguage::SimplifiedChinese {
                println!("Agents Router 已配置 Claude Code hooks，用来接收完成事件和任务耗时。");
                println!(
                    "如果你手动改 Claude Code settings，请保留 SessionStart、UserPromptSubmit、Stop 和 Notification hooks 指向 Agents Router。"
                );
            } else {
                println!(
                    "Agents Router configured Claude Code hooks for completion events and task duration."
                );
                println!(
                    "If you edit Claude Code settings manually, keep SessionStart, UserPromptSubmit, Stop, and Notification hooks pointed at Agents Router."
                );
            }
        }
        SetupIntegrationKind::ManualHookCommand { integration_point } => {
            if i18n.language() == CliLanguage::SimplifiedChinese {
                print_chinese_hook_command_note(
                    descriptor.source_capability.display_name,
                    integration_point.simplified_chinese(),
                    descriptor.source_capability.hook_command,
                );
            } else {
                print_english_hook_command_note(
                    descriptor.source_capability.display_name,
                    integration_point.english(),
                    descriptor.source_capability.hook_command,
                );
            }
        }
    }
}

fn print_english_hook_command_note(
    display_name: &str,
    integration_point: &str,
    command: Option<HookCommandTemplate>,
) {
    println!("Agents Router is ready for {display_name} hook events.");
    if let Some(command) = command {
        if command.requires_emit_fields() {
            println!(
                "Configure your {display_name} {integration_point} to call `{}` with `--title` and `--body` from the wrapper.",
                command.command()
            );
        } else {
            println!(
                "Configure your {display_name} {integration_point} to call `{}`.",
                command.command()
            );
        }
    }
}

fn print_chinese_hook_command_note(
    display_name: &str,
    integration_point: &str,
    command: Option<HookCommandTemplate>,
) {
    println!("Agents Router 已准备接收 {display_name} hook event。");
    if let Some(command) = command {
        if command.requires_emit_fields() {
            println!(
                "让 {display_name} {integration_point} 调用 `{}`，并由 wrapper 传入 `--title` 和 `--body`。",
                command.command()
            );
        } else {
            println!(
                "让 {display_name} {integration_point} 调用：`{}`。",
                command.command()
            );
        }
    }
}

async fn offer_test_notification(config: &RawConfig, i18n: I18n) -> anyhow::Result<()> {
    if !is_interactive_terminal() || !can_send_test_notification(config) {
        return Ok(());
    }

    if !prompt_confirm(i18n.text(Text::SendTestNow), true)? {
        println!("{}", setup::TEST_NOTIFICATION_SKIPPED_MESSAGE);
        return Ok(());
    }

    send_test_notification().await?;
    println!("{}", style(i18n.text(Text::TestSent)).green());
    if prompt_confirm(i18n.text(Text::DidItArrive), true)? {
        println!("{}", style(i18n.text(Text::Working)).green());
    } else {
        println!(
            "{}",
            style(i18n.text(Text::ServiceRunningButTestMissing)).yellow()
        );
        println!("{}", i18n.text(Text::CheckProviderSettingsPrinted));
    }

    Ok(())
}

fn can_send_test_notification(config: &RawConfig) -> bool {
    config.source("agents_router").is_some()
        && config.routes.iter().any(|route| {
            route.sources.iter().any(|source| source == "agents_router")
                && !route.providers.is_empty()
        })
}

fn print_local_source_integration_report(report: &LocalSourceIntegrationReport, i18n: I18n) {
    if let Some(setup) = &report.codex_cli_stop_hook {
        match i18n.language() {
            CliLanguage::English => match setup.status {
                CodexCliStopHookSetupStatus::CreatedConfig => {
                    println!(
                        "Created Codex CLI config at `{}` and added the Agents Router Stop hook.",
                        setup.path.display()
                    );
                    println!("Existing `notify` settings were left unchanged.");
                }
                CodexCliStopHookSetupStatus::UpdatedConfig => {
                    println!(
                        "Updated Codex CLI config at `{}` with the Agents Router Stop hook.",
                        setup.path.display()
                    );
                    println!("Existing `notify` settings were left unchanged.");
                }
                CodexCliStopHookSetupStatus::AlreadyConfigured => {
                    println!(
                        "Codex CLI Stop hook is already configured at `{}`.",
                        setup.path.display()
                    );
                    println!("Existing `notify` settings were left unchanged.");
                }
            },
            CliLanguage::SimplifiedChinese => match setup.status {
                CodexCliStopHookSetupStatus::CreatedConfig => {
                    println!(
                        "已创建 Codex CLI config：`{}`，并写入 Agents Router Stop hook。",
                        setup.path.display()
                    );
                    println!("已有 `notify` 设置保持不变。");
                }
                CodexCliStopHookSetupStatus::UpdatedConfig => {
                    println!(
                        "已更新 Codex CLI config：`{}`，写入 Agents Router Stop hook。",
                        setup.path.display()
                    );
                    println!("已有 `notify` 设置保持不变。");
                }
                CodexCliStopHookSetupStatus::AlreadyConfigured => {
                    println!(
                        "Codex CLI Stop hook 已经配置在：`{}`。",
                        setup.path.display()
                    );
                    println!("已有 `notify` 设置保持不变。");
                }
            },
        }
    }

    if let Some(setup) = &report.claude_code_hooks {
        match i18n.language() {
            CliLanguage::English => match setup.status {
                ClaudeCodeHooksSetupStatus::CreatedSettings => {
                    println!(
                        "Created Claude Code settings at `{}` and added Agents Router hooks.",
                        setup.path.display()
                    );
                }
                ClaudeCodeHooksSetupStatus::UpdatedSettings => {
                    println!(
                        "Updated Claude Code settings at `{}` with Agents Router hooks.",
                        setup.path.display()
                    );
                }
                ClaudeCodeHooksSetupStatus::AlreadyConfigured => {
                    println!(
                        "Claude Code hooks are already configured at `{}`.",
                        setup.path.display()
                    );
                }
            },
            CliLanguage::SimplifiedChinese => match setup.status {
                ClaudeCodeHooksSetupStatus::CreatedSettings => {
                    println!(
                        "已创建 Claude Code settings：`{}`，并写入 Agents Router hooks。",
                        setup.path.display()
                    );
                }
                ClaudeCodeHooksSetupStatus::UpdatedSettings => {
                    println!(
                        "已更新 Claude Code settings：`{}`，写入 Agents Router hooks。",
                        setup.path.display()
                    );
                }
                ClaudeCodeHooksSetupStatus::AlreadyConfigured => {
                    println!("Claude Code hooks 已经配置在：`{}`。", setup.path.display());
                }
            },
        }
    }
}

async fn send_test_notification() -> anyhow::Result<()> {
    let endpoint = ingress_endpoint()?;
    wait_for_service(&endpoint).await?;

    let report = local_ingress::submit_event_report(
        &endpoint,
        &LocalSignalEvent::new(
            "agents_router",
            "Agents Router",
            "Test notification from your computer. If this arrived, Agents Router is working.",
        ),
    )
    .await
    .context("failed to send test notification through the local service")?;

    if report.is_some_and(|report| report.suppressed > 0) {
        anyhow::bail!(
            "test notification was partially or fully suppressed by delivery safety; run `agents-router safety reset` and try again"
        );
    }

    Ok(())
}

async fn wait_for_service(endpoint: &agents_router::paths::IngressEndpoint) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        if local_ingress::is_ready(endpoint).await {
            return Ok(());
        }

        if started.elapsed() >= SERVICE_READY_TIMEOUT {
            anyhow::bail!(
                "agents-router service did not become ready at `{}`; check the log file",
                endpoint.display()
            );
        }

        sleep(SERVICE_READY_POLL_INTERVAL).await;
    }
}

fn wait_for_enter(prompt: &str) -> anyhow::Result<()> {
    let _ = prompt_text(prompt, "failed to read input")?;
    Ok(())
}

fn run_stop() -> anyhow::Result<()> {
    init_tracing("info")?;
    let manager = PlatformServiceManager::system()?;
    match manager.stop()? {
        ServiceStopOutcome::NotInstalled => {
            println!("agents-router service is not installed.");
        }
        ServiceStopOutcome::AlreadyStopped => {
            println!("agents-router service is already stopped.");
        }
        ServiceStopOutcome::Stopped => {
            println!("agents-router service stopped.");
        }
    }

    cleanup_ingress_endpoint_if_exists()?;
    cleanup_legacy_background_service()?;

    Ok(())
}

fn run_uninstall() -> anyhow::Result<()> {
    init_tracing("info")?;
    let manager = PlatformServiceManager::system()?;
    manager.uninstall()?;
    cleanup_ingress_endpoint_if_exists()?;
    cleanup_legacy_background_service()?;

    remove_path_if_exists(&app_support_dir_path()?)?;
    if let Some(log_dir) = log_file_path()?.parent() {
        remove_path_if_exists(log_dir)?;
    }
    if let Some(config_dir) = default_config_file_path()?.parent() {
        remove_path_if_exists(config_dir)?;
    }

    let current_binary = std::env::current_exe().context("failed to locate current binary")?;
    let marker_install_method = update::read_install_method_marker(&current_binary)?;
    let install_method = std::env::var(update::INSTALL_METHOD_ENV)
        .ok()
        .or(marker_install_method);
    if should_remove_current_binary_for_install_method(&current_binary, install_method.as_deref()) {
        remove_path_if_exists(&current_binary)?;
        if let Some(marker_path) = update::install_method_marker_path(&current_binary) {
            remove_path_if_exists(&marker_path)?;
        }
    } else if install_method.as_deref() == Some("npm") {
        println!(
            "left npm-managed binary in place: {}",
            current_binary.display()
        );
        println!("Remove the npm package with: npm uninstall -g agents-router");
    } else {
        println!(
            "left unmanaged binary in place: {}",
            current_binary.display()
        );
        println!("Remove it with the installer or package manager that owns this binary.");
    }

    println!("agents-router uninstalled.");
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect `{}`", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory `{}`", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove file `{}`", path.display()))?;
    }

    println!("removed: {}", path.display());
    Ok(())
}

#[cfg(test)]
fn should_remove_current_binary(path: &Path) -> bool {
    should_remove_current_binary_for_install_method(path, None)
}

fn should_remove_current_binary_for_install_method(
    path: &Path,
    install_method: Option<&str>,
) -> bool {
    if install_method != Some("script") {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if file_name != "agents-router" && file_name != "agents-router.exe" {
        return false;
    }

    !path
        .components()
        .any(|component| component.as_os_str() == "target")
}

fn cleanup_ingress_endpoint_if_exists() -> anyhow::Result<()> {
    let endpoint = ingress_endpoint()?;
    local_ingress::cleanup_endpoint(&endpoint)
}

async fn run_watch(config_path: &Path, config: ValidatedConfig) -> anyhow::Result<()> {
    local_integrations::ensure_local_source_integrations(&config)?;
    let runtime =
        RuntimeState::new_with_delivery_safety(config, DeliverySafetyGuard::load_default()?)?;
    let endpoint = ingress_endpoint()?;
    run_service_tasks(config_path.to_path_buf(), runtime, endpoint).await
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn run_service_tasks(
    config_path: PathBuf,
    runtime: RuntimeState,
    endpoint: agents_router::paths::IngressEndpoint,
) -> anyhow::Result<()> {
    let mut tasks = JoinSet::new();
    tasks.spawn(reload_config_on_change(config_path, runtime.clone()));
    tasks.spawn(codex_desktop::watch(runtime.clone()));
    tasks.spawn(local_ingress::serve(runtime.clone(), endpoint));
    tasks.spawn(local_open_bridge::serve_when_needed(runtime));
    wait_for_service_task(tasks).await
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn run_service_tasks(
    config_path: PathBuf,
    runtime: RuntimeState,
    endpoint: agents_router::paths::IngressEndpoint,
) -> anyhow::Result<()> {
    let mut tasks = JoinSet::new();
    tasks.spawn(reload_config_on_change(config_path, runtime.clone()));
    tasks.spawn(local_ingress::serve(runtime.clone(), endpoint));
    tasks.spawn(local_open_bridge::serve_when_needed(runtime));
    wait_for_service_task(tasks).await
}

async fn wait_for_service_task(mut tasks: JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
    let result = tasks
        .join_next()
        .await
        .context("service task set ended unexpectedly")?;
    tasks.abort_all();
    result.context("service task panicked")?
}

async fn run_emit(
    source_id: &str,
    title: String,
    body: String,
    duration_ms: Option<u64>,
) -> anyhow::Result<()> {
    let endpoint = ingress_endpoint()?;
    let event = local_event_from_emit(source_id, title, body, duration_ms);
    local_ingress::submit_event(&endpoint, &event).await
}

fn local_event_from_emit(
    source_id: &str,
    title: String,
    body: String,
    duration_ms: Option<u64>,
) -> LocalSignalEvent {
    let mut event = LocalSignalEvent::new(source_id, title, body);
    if let Some(duration_ms) = duration_ms {
        event.lifecycle = Some(SignalLifecycle {
            status: Some(SignalLifecycleStatus::Completed),
            started_at: None,
            completed_at: None,
            duration_ms: Some(duration_ms),
        });
    }
    event
}

async fn run_ingest(source_id: &str, format: SourceIngestFormat) -> anyhow::Result<()> {
    let mut raw = String::new();
    io::stdin()
        .read_to_string(&mut raw)
        .context("failed to read hook event from stdin")?;

    let event = match format {
        SourceIngestFormat::AgentHookEvent => {
            let input =
                serde_json::from_str(&raw).context("failed to parse agent_hook event JSON")?;
            agent_hook::local_event_from_structured_hook(source_id, input)?
        }
        SourceIngestFormat::ClaudeCodeHook => {
            let input =
                serde_json::from_str(&raw).context("failed to parse claude_code hook JSON")?;
            claude_code::local_event_from_hook(source_id, input)?
        }
        SourceIngestFormat::CodexCliStop => {
            let input =
                serde_json::from_str(&raw).context("failed to parse codex_cli stop hook JSON")?;
            if codex_cli_stop_hook_is_shadowed_by_codex_desktop(source_id, &input) {
                return Ok(());
            }
            codex_cli::local_event_from_stop_hook(source_id, input)?
        }
        SourceIngestFormat::GeminiCliHook => {
            let input =
                serde_json::from_str(&raw).context("failed to parse gemini_cli hook JSON")?;
            gemini_cli::local_event_from_hook(source_id, input)?
        }
        SourceIngestFormat::GithubCopilotCliNotification => {
            let input = serde_json::from_str(&raw)
                .context("failed to parse github_copilot_cli notification hook JSON")?;
            github_copilot_cli::local_event_from_notification_hook(source_id, input)?
        }
        SourceIngestFormat::OpencodeCliSession => {
            let input =
                serde_json::from_str(&raw).context("failed to parse opencode_cli session JSON")?;
            opencode_cli::local_event_from_session_input(source_id, input)?
        }
    };

    local_ingress::submit_event(&ingress_endpoint()?, &event).await
}

fn codex_cli_stop_hook_is_shadowed_by_codex_desktop(
    source_id: &str,
    input: &codex_cli::CodexCliStopHookInput,
) -> bool {
    if source_id != SourceType::CodexCli.as_str() {
        return false;
    }

    let Some(config) = codex_cli_stop_hook_shadowing_config() else {
        return false;
    };
    let Ok(sessions_dir) = codex_sessions_dir_path() else {
        return false;
    };

    codex_cli::stop_hook_input_is_shadowed_by_codex_desktop(&config, input, &sessions_dir)
        .unwrap_or(false)
}

fn codex_cli_stop_hook_shadowing_config() -> Option<ValidatedConfig> {
    if let Ok(metadata_path) = service_metadata_path()
        && let Ok(metadata) = load_metadata(&metadata_path)
    {
        return ParsedConfig::from_path(&PathBuf::from(metadata.config_path))
            .ok()
            .map(|config| config.validated);
    }

    let config_path = default_config_file_path().ok()?;
    ParsedConfig::from_path(&config_path)
        .ok()
        .map(|config| config.validated)
}

fn init_tracing(level: &str) -> anyhow::Result<()> {
    let level = match level {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        value => anyhow::bail!("unsupported log level `{value}`"),
    };

    let _ = tracing_subscriber::fmt().with_max_level(level).try_init();
    Ok(())
}

#[cfg(test)]
mod tests;
