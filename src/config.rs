use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::agent_integration_catalog::{AgentIntegrationId, agent_integration_descriptor};
use crate::notification_detail_policy::{rejects_full_answer, rejects_prompt_detail};
use crate::provider_urls::{
    validate_custom_webhook_url, validate_discord_webhook_url, validate_feishu_lark_webhook_url,
    validate_microsoft_teams_webhook_url, validate_slack_webhook_url,
};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfig {
    pub raw: RawConfig,
    pub validated: ValidatedConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub cli: CliConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub notification: NotificationConfig,
    pub sources: Vec<SourceConfig>,
    pub providers: Vec<RawProviderConfig>,
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedConfig {
    pub schema_version: u32,
    pub cli: CliConfig,
    pub log: LogConfig,
    pub notification: NotificationConfig,
    pub sources: Vec<SourceConfig>,
    pub providers: Vec<ProviderConfig>,
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CliConfig {
    #[serde(default)]
    pub language: CliLanguage,
}

impl CliConfig {
    pub fn new(language: CliLanguage) -> Self {
        Self { language }
    }
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            language: CliLanguage::English,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum CliLanguage {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

impl CliLanguage {
    pub fn config_value(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        let normalized = input.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "en" | "english" => Some(Self::English),
            "zh" | "zh-cn" | "zh-hans" | "cn" | "chinese" | "simplified-chinese" => {
                Some(Self::SimplifiedChinese)
            }
            _ if input.trim() == "简体中文" || input.trim() == "中文" => {
                Some(Self::SimplifiedChinese)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NotificationConfig {
    #[serde(default)]
    pub answer_detail: AnswerDetail,
    #[serde(default)]
    pub prompt_detail: PromptDetail,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            answer_detail: AnswerDetail::Preview,
            prompt_detail: PromptDetail::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerDetail {
    #[default]
    Preview,
    Full,
}

impl AnswerDetail {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Full => "Full Answer",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptDetail {
    #[default]
    Off,
    On,
}

impl PromptDetail {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub source_type: SourceType,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    #[serde(alias = "agents_notifier")]
    AgentsRouter,
    AgentHook,
    CodexDesktop,
    CodexCli,
    ClaudeCode,
}

impl SourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentsRouter => "agents_router",
            Self::AgentHook => "agent_hook",
            Self::CodexDesktop => "codex_desktop",
            Self::CodexCli => "codex_cli",
            Self::ClaudeCode => "claude_code",
        }
    }

    pub fn from_signal_value(value: &str) -> Option<Self> {
        match value {
            "agents_router" | "agents_notifier" => Some(Self::AgentsRouter),
            "agent_hook" => Some(Self::AgentHook),
            "codex_desktop" => Some(Self::CodexDesktop),
            "codex_cli" => Some(Self::CodexCli),
            "claude_code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawProviderConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_secret_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_registration_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_token_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_phone_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<EmailSmtpSecurity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_token_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub id: String,
    pub detail: ProviderConfigDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderConfigDetail {
    Ntfy(NtfyProviderConfig),
    Webhook(WebhookProviderConfig),
    FeishuLark(FeishuLarkProviderConfig),
    Pushover(PushoverProviderConfig),
    Slack(SlackProviderConfig),
    Discord(DiscordProviderConfig),
    Telegram(TelegramProviderConfig),
    Whatsapp(WhatsappProviderConfig),
    Wechat(WechatProviderConfig),
    MicrosoftTeams(MicrosoftTeamsProviderConfig),
    EmailSmtp(EmailSmtpProviderConfig),
}

impl ProviderConfig {
    pub fn provider_type(&self) -> ProviderType {
        self.detail.provider_type()
    }
}

impl ProviderConfigDetail {
    pub fn provider_type(&self) -> ProviderType {
        match self {
            Self::Ntfy(_) => ProviderType::Ntfy,
            Self::Webhook(_) => ProviderType::Webhook,
            Self::FeishuLark(_) => ProviderType::FeishuLark,
            Self::Pushover(_) => ProviderType::Pushover,
            Self::Slack(_) => ProviderType::Slack,
            Self::Discord(_) => ProviderType::Discord,
            Self::Telegram(_) => ProviderType::Telegram,
            Self::Whatsapp(_) => ProviderType::Whatsapp,
            Self::Wechat(_) => ProviderType::Wechat,
            Self::MicrosoftTeams(_) => ProviderType::MicrosoftTeams,
            Self::EmailSmtp(_) => ProviderType::EmailSmtp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtfyProviderConfig {
    pub server: String,
    pub topic: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookProviderConfig {
    pub url: UrlSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeishuLarkProviderConfig {
    CustomBot(FeishuLarkCustomBotProviderConfig),
    AppBot(FeishuLarkAppBotProviderConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuLarkCustomBotProviderConfig {
    pub url: UrlSource,
    pub secret: Option<SecretSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuLarkAppBotProviderConfig {
    pub domain: FeishuLarkAppDomain,
    pub app_id: String,
    pub app_secret: SecretSource,
    pub app_registration_source: Option<String>,
    pub tenant_key: Option<String>,
    pub chat_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeishuLarkAppDomain {
    Feishu,
    Lark,
}

impl FeishuLarkAppDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
            Self::Lark => "lark",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushoverProviderConfig {
    pub app_token: SecretSource,
    pub user_key: SecretSource,
    pub device: Option<String>,
    pub sound: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackProviderConfig {
    pub url: UrlSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordProviderConfig {
    pub url: UrlSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramProviderConfig {
    pub bot_token: SecretSource,
    pub chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsappProviderConfig {
    pub access_token: SecretSource,
    pub phone_number_id: String,
    pub recipient_phone_number: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatProviderConfig {
    pub base_url: String,
    pub token: SecretSource,
    pub recipient_user_id: String,
    pub context_token: SecretSource,
    pub route_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftTeamsProviderConfig {
    pub url: UrlSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSmtpProviderConfig {
    pub host: String,
    pub port: u16,
    pub security: EmailSmtpSecurity,
    pub username: Option<SecretSource>,
    pub password: Option<SecretSource>,
    pub from: String,
    pub to: Vec<String>,
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlSource {
    Inline(String),
    Env(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    Inline(String),
    Env(String),
}

impl UrlSource {
    pub fn resolve_runtime_value(
        &self,
        provider_id: &str,
        provider_type: &str,
        field: &'static str,
    ) -> anyhow::Result<String> {
        match self {
            Self::Inline(value) => Ok(value.clone()),
            Self::Env(env_name) => std::env::var(env_name).with_context(|| {
                format!(
                    "{provider_type} provider `{provider_id}` env var `{env_name}` from `{field}` is not set"
                )
            }),
        }
    }
}

impl SecretSource {
    pub fn resolve_runtime_value(
        &self,
        provider_id: &str,
        provider_type: &str,
        field: &'static str,
    ) -> anyhow::Result<String> {
        match self {
            Self::Inline(value) => Ok(value.clone()),
            Self::Env(env_name) => std::env::var(env_name).with_context(|| {
                format!(
                    "{provider_type} provider `{provider_id}` env var `{env_name}` from `{field}` is not set"
                )
            }),
        }
    }

    pub fn is_configured(&self) -> bool {
        match self {
            Self::Inline(value) | Self::Env(value) => !value.trim().is_empty(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailSmtpSecurity {
    Starttls,
    ImplicitTls,
}

impl EmailSmtpSecurity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starttls => "starttls",
            Self::ImplicitTls => "implicit_tls",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Ntfy,
    Webhook,
    FeishuLark,
    Pushover,
    Slack,
    Discord,
    Telegram,
    Whatsapp,
    Wechat,
    MicrosoftTeams,
    EmailSmtp,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ntfy => "ntfy",
            Self::Webhook => "webhook",
            Self::FeishuLark => "feishu_lark",
            Self::Pushover => "pushover",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::Whatsapp => "whatsapp",
            Self::Wechat => "wechat",
            Self::MicrosoftTeams => "microsoft_teams",
            Self::EmailSmtp => "email_smtp",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RouteConfig {
    pub sources: Vec<String>,
    pub providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_task_duration_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only_forward_from_project_paths: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "ResponseSurfaceRouteConfig::is_disabled"
    )]
    pub response_surface: ResponseSurfaceRouteConfig,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResponseSurfaceRouteConfig {
    #[serde(default)]
    pub enabled: bool,
}

impl ResponseSurfaceRouteConfig {
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

impl RouteConfig {
    pub fn new(sources: Vec<String>, providers: Vec<String>) -> Self {
        Self {
            sources,
            providers,
            minimum_task_duration_minutes: None,
            only_forward_from_project_paths: Vec::new(),
            response_surface: ResponseSurfaceRouteConfig::disabled(),
        }
    }

    pub fn minimum_task_duration_ms(&self) -> Option<u64> {
        self.minimum_task_duration_minutes
            .and_then(|minutes| minutes.checked_mul(60_000))
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found at {path}")]
    NotFound { path: String },
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("unsupported config schema_version {found}; expected {expected}")]
    UnsupportedSchema { found: u32, expected: u32 },
    #[error("duplicate source id `{0}`")]
    DuplicateSourceId(String),
    #[error("{source_type} source must use id `{expected_id}` because {reason}; got `{source_id}`")]
    InvalidSourceIdForType {
        source_id: String,
        source_type: &'static str,
        expected_id: &'static str,
        reason: &'static str,
    },
    #[error("duplicate provider id `{0}`")]
    DuplicateProviderId(String),
    #[error("route references unknown source `{0}`")]
    UnknownRouteSource(String),
    #[error("route references unknown provider `{0}`")]
    UnknownRouteProvider(String),
    #[error("route {route_index} `minimum_task_duration_minutes` must be greater than 0")]
    InvalidMinimumTaskDuration { route_index: usize },
    #[error("route {route_index} `minimum_task_duration_minutes` is too large")]
    MinimumTaskDurationTooLarge { route_index: usize },
    #[error(
        "route {route_index} `only_forward_from_project_paths` contains invalid project path `{path}`; paths must be clean absolute paths"
    )]
    InvalidRouteProjectPath { route_index: usize, path: String },
    #[error("provider `{provider_id}` requires `{field}`")]
    MissingProviderField {
        provider_id: String,
        field: &'static str,
    },
    #[error("provider `{provider_id}` has invalid `{field}`: {message}")]
    InvalidProviderUrl {
        provider_id: String,
        field: &'static str,
        message: String,
    },
    #[error("webhook provider `{provider_id}` must set exactly one of `url` or `url_env`")]
    InvalidWebhookUrlSource { provider_id: String },
    #[error("feishu_lark provider `{provider_id}` must set exactly one of `url` or `url_env`")]
    InvalidFeishuLarkUrlSource { provider_id: String },
    #[error(
        "feishu_lark provider `{provider_id}` must set at most one of `secret` or `secret_env`"
    )]
    InvalidFeishuLarkSecretSource { provider_id: String },
    #[error("feishu_lark provider `{provider_id}` has unsupported mode `{mode}`")]
    InvalidFeishuLarkMode { provider_id: String, mode: String },
    #[error(
        "feishu_lark provider `{provider_id}` must set `mode = \"app_bot\"` before using Feishu/Lark app credential fields"
    )]
    MissingFeishuLarkAppBotMode { provider_id: String },
    #[error("feishu_lark app provider `{provider_id}` must not set incoming webhook fields")]
    InvalidFeishuLarkAppBotWebhookFields { provider_id: String },
    #[error(
        "feishu_lark incoming webhook provider `{provider_id}` must not set Feishu/Lark app credential or tenant fields"
    )]
    InvalidFeishuLarkCustomBotAppFields { provider_id: String },
    #[error("feishu_lark app provider `{provider_id}` has unsupported domain `{domain}`")]
    InvalidFeishuLarkAppBotDomain { provider_id: String, domain: String },
    #[error(
        "feishu_lark app provider `{provider_id}` must set exactly one of `app_secret` or `app_secret_env`"
    )]
    InvalidFeishuLarkAppSecretSource { provider_id: String },
    #[error(
        "pushover provider `{provider_id}` must set exactly one of `app_token` or `app_token_env`"
    )]
    InvalidPushoverAppTokenSource { provider_id: String },
    #[error(
        "pushover provider `{provider_id}` must set exactly one of `user_key` or `user_key_env`"
    )]
    InvalidPushoverUserKeySource { provider_id: String },
    #[error("slack provider `{provider_id}` must set exactly one of `url` or `url_env`")]
    InvalidSlackUrlSource { provider_id: String },
    #[error("discord provider `{provider_id}` must set exactly one of `url` or `url_env`")]
    InvalidDiscordUrlSource { provider_id: String },
    #[error(
        "telegram provider `{provider_id}` must set exactly one of `bot_token` or `bot_token_env`"
    )]
    InvalidTelegramBotTokenSource { provider_id: String },
    #[error(
        "whatsapp provider `{provider_id}` must set exactly one of `access_token` or `access_token_env`"
    )]
    InvalidWhatsappAccessTokenSource { provider_id: String },
    #[error("wechat provider `{provider_id}` must set exactly one of `token` or `token_env`")]
    InvalidWechatTokenSource { provider_id: String },
    #[error(
        "wechat provider `{provider_id}` must set exactly one of `context_token` or `context_token_env`"
    )]
    InvalidWechatContextTokenSource { provider_id: String },
    #[error("microsoft_teams provider `{provider_id}` must set exactly one of `url` or `url_env`")]
    InvalidMicrosoftTeamsUrlSource { provider_id: String },
    #[error(
        "email_smtp provider `{provider_id}` must set at most one of `username` or `username_env`"
    )]
    InvalidEmailSmtpUsernameSource { provider_id: String },
    #[error(
        "email_smtp provider `{provider_id}` must set at most one of `password` or `password_env`"
    )]
    InvalidEmailSmtpPasswordSource { provider_id: String },
    #[error("email_smtp provider `{provider_id}` must set both username and password, or neither")]
    InvalidEmailSmtpCredentials { provider_id: String },
    #[error(
        "`prompt_detail = \"on\"` is not supported with `{provider_type}` provider `{provider_id}` because that provider has a message size limit"
    )]
    PromptDetailNotSupportedForProvider {
        provider_id: String,
        provider_type: &'static str,
    },
    #[error(
        "`answer_detail = \"full\"` is not supported with `{provider_type}` provider `{provider_id}` because that provider has a message size limit"
    )]
    AnswerDetailNotSupportedForProvider {
        provider_id: String,
        provider_type: &'static str,
    },
}

impl LoadedConfig {
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw = RawConfig::from_path(path)?;
        let validated = raw.validate()?;
        Ok(Self { raw, validated })
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, ConfigError> {
        let raw = RawConfig::from_toml_str(raw)?;
        let validated = raw.validate()?;
        Ok(Self { raw, validated })
    }
}

impl RawConfig {
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| {
            if source.kind() == ErrorKind::NotFound {
                ConfigError::NotFound {
                    path: path.display().to_string(),
                }
            } else {
                ConfigError::Read {
                    path: path.display().to_string(),
                    source,
                }
            }
        })?;

        Self::from_toml_str(&raw)
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(raw)?)
    }

    pub fn source(&self, id: &str) -> Option<&SourceConfig> {
        self.sources.iter().find(|source| source.id == id)
    }

    pub fn provider(&self, id: &str) -> Option<&RawProviderConfig> {
        self.providers.iter().find(|provider| provider.id == id)
    }

    pub fn validate(&self) -> Result<ValidatedConfig, ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchema {
                found: self.schema_version,
                expected: CONFIG_SCHEMA_VERSION,
            });
        }

        let mut source_ids = HashSet::new();
        for source in &self.sources {
            if !source_ids.insert(source.id.as_str()) {
                return Err(ConfigError::DuplicateSourceId(source.id.clone()));
            }
            source.validate()?;
        }

        let mut provider_ids = HashSet::new();
        for provider in &self.providers {
            if !provider_ids.insert(provider.id.as_str()) {
                return Err(ConfigError::DuplicateProviderId(provider.id.clone()));
            }
        }

        let providers = self
            .providers
            .iter()
            .map(RawProviderConfig::to_validated)
            .collect::<Result<Vec<_>, _>>()?;

        for (route_index, route) in self.routes.iter().enumerate() {
            for source_id in &route.sources {
                if !source_ids.contains(source_id.as_str()) {
                    return Err(ConfigError::UnknownRouteSource(source_id.clone()));
                }
            }

            for provider_id in &route.providers {
                if !provider_ids.contains(provider_id.as_str()) {
                    return Err(ConfigError::UnknownRouteProvider(provider_id.clone()));
                }
            }

            route.validate_filters(route_index)?;
        }

        self.validate_notification_provider_compatibility()?;

        Ok(ValidatedConfig {
            schema_version: self.schema_version,
            cli: self.cli.clone(),
            log: self.log.clone(),
            notification: self.notification.clone(),
            sources: self.sources.clone(),
            providers,
            routes: self.routes.clone(),
        })
    }

    fn validate_notification_provider_compatibility(&self) -> Result<(), ConfigError> {
        if self.notification.answer_detail != AnswerDetail::Full
            && self.notification.prompt_detail != PromptDetail::On
        {
            return Ok(());
        }

        for route in &self.routes {
            for provider_id in &route.providers {
                let Some(provider) = self.provider(provider_id) else {
                    continue;
                };
                if self.notification.answer_detail == AnswerDetail::Full
                    && rejects_full_answer(provider.provider_type)
                {
                    return Err(ConfigError::AnswerDetailNotSupportedForProvider {
                        provider_id: provider.id.clone(),
                        provider_type: provider.provider_type.as_str(),
                    });
                }

                if self.notification.prompt_detail == PromptDetail::On
                    && rejects_prompt_detail(provider.provider_type)
                {
                    return Err(ConfigError::PromptDetailNotSupportedForProvider {
                        provider_id: provider.id.clone(),
                        provider_type: provider.provider_type.as_str(),
                    });
                }
            }
        }

        Ok(())
    }
}

impl ValidatedConfig {
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        LoadedConfig::from_path(path).map(|loaded| loaded.validated)
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, ConfigError> {
        LoadedConfig::from_toml_str(raw).map(|loaded| loaded.validated)
    }

    pub fn source(&self, id: &str) -> Option<&SourceConfig> {
        self.sources.iter().find(|source| source.id == id)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|provider| provider.id == id)
    }
}

impl SourceConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let Some(requirement) = canonical_source_id_requirement(self.source_type) else {
            return Ok(());
        };
        if self.id == requirement.expected_id {
            return Ok(());
        }

        Err(ConfigError::InvalidSourceIdForType {
            source_id: self.id.clone(),
            source_type: self.source_type.as_str(),
            expected_id: requirement.expected_id,
            reason: requirement.reason,
        })
    }
}

struct CanonicalSourceIdRequirement {
    expected_id: &'static str,
    reason: &'static str,
}

fn canonical_source_id_requirement(
    source_type: SourceType,
) -> Option<CanonicalSourceIdRequirement> {
    match source_type {
        SourceType::CodexCli => Some(CanonicalSourceIdRequirement {
            expected_id: agent_integration_descriptor(AgentIntegrationId::CodexCli)
                .source_capability
                .canonical_source_id,
            reason: "Codex CLI has one global Stop hook",
        }),
        SourceType::ClaudeCode => Some(CanonicalSourceIdRequirement {
            expected_id: agent_integration_descriptor(AgentIntegrationId::ClaudeCode)
                .source_capability
                .canonical_source_id,
            reason: "Claude Code has one global settings file",
        }),
        SourceType::AgentsRouter | SourceType::AgentHook | SourceType::CodexDesktop => None,
    }
}

impl RouteConfig {
    fn validate_filters(&self, route_index: usize) -> Result<(), ConfigError> {
        if let Some(minutes) = self.minimum_task_duration_minutes {
            if minutes == 0 {
                return Err(ConfigError::InvalidMinimumTaskDuration { route_index });
            }
            if minutes.checked_mul(60_000).is_none() {
                return Err(ConfigError::MinimumTaskDurationTooLarge { route_index });
            }
        }

        for path in &self.only_forward_from_project_paths {
            if !is_clean_absolute_project_path(path) {
                return Err(ConfigError::InvalidRouteProjectPath {
                    route_index,
                    path: path.clone(),
                });
            }
        }

        Ok(())
    }
}

pub(crate) fn is_clean_absolute_project_path(value: &str) -> bool {
    if value.trim().is_empty() || value.trim() != value {
        return false;
    }

    let path = Path::new(value);
    path.is_absolute()
        && !has_dot_project_path_component(value)
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
}

fn has_dot_project_path_component(value: &str) -> bool {
    value
        .split(['/', '\\'])
        .any(|component| component == "." || component == "..")
}

impl RawProviderConfig {
    pub fn new(id: impl Into<String>, provider_type: ProviderType) -> Self {
        Self {
            id: id.into(),
            provider_type,
            mode: None,
            base_url: None,
            domain: None,
            server: None,
            topic: None,
            url: None,
            url_env: None,
            secret: None,
            secret_env: None,
            app_id: None,
            app_secret: None,
            app_secret_env: None,
            app_registration_source: None,
            tenant_key: None,
            app_token: None,
            app_token_env: None,
            user_key: None,
            user_key_env: None,
            device: None,
            sound: None,
            bot_token: None,
            bot_token_env: None,
            chat_id: None,
            access_token: None,
            access_token_env: None,
            phone_number_id: None,
            recipient_phone_number: None,
            host: None,
            port: None,
            security: None,
            username: None,
            username_env: None,
            password: None,
            password_env: None,
            from: None,
            to: None,
            reply_to: None,
            token: None,
            token_env: None,
            recipient_user_id: None,
            context_token: None,
            context_token_env: None,
            route_tag: None,
        }
    }

    fn to_validated(&self) -> Result<ProviderConfig, ConfigError> {
        let detail = match self.provider_type {
            ProviderType::Ntfy => ProviderConfigDetail::Ntfy(NtfyProviderConfig {
                server: self.required_exact_string("server", self.server.as_deref())?,
                topic: self.required_exact_string("topic", self.topic.as_deref())?,
            }),
            ProviderType::Webhook => ProviderConfigDetail::Webhook(WebhookProviderConfig {
                url: self.required_url_source(
                    ConfigError::InvalidWebhookUrlSource {
                        provider_id: self.id.clone(),
                    },
                    validate_custom_webhook_url,
                )?,
            }),
            ProviderType::FeishuLark => {
                ProviderConfigDetail::FeishuLark(self.to_validated_feishu_lark_provider_config()?)
            }
            ProviderType::Pushover => ProviderConfigDetail::Pushover(PushoverProviderConfig {
                app_token: self.required_exact_secret_source(
                    "app_token",
                    self.app_token.as_deref(),
                    "app_token_env",
                    self.app_token_env.as_deref(),
                    ConfigError::InvalidPushoverAppTokenSource {
                        provider_id: self.id.clone(),
                    },
                )?,
                user_key: self.required_exact_secret_source(
                    "user_key",
                    self.user_key.as_deref(),
                    "user_key_env",
                    self.user_key_env.as_deref(),
                    ConfigError::InvalidPushoverUserKeySource {
                        provider_id: self.id.clone(),
                    },
                )?,
                device: present_exact_owned(self.device.as_deref()),
                sound: present_exact_owned(self.sound.as_deref()),
            }),
            ProviderType::Slack => ProviderConfigDetail::Slack(SlackProviderConfig {
                url: self.required_url_source(
                    ConfigError::InvalidSlackUrlSource {
                        provider_id: self.id.clone(),
                    },
                    validate_slack_webhook_url,
                )?,
            }),
            ProviderType::Discord => ProviderConfigDetail::Discord(DiscordProviderConfig {
                url: self.required_url_source(
                    ConfigError::InvalidDiscordUrlSource {
                        provider_id: self.id.clone(),
                    },
                    validate_discord_webhook_url,
                )?,
            }),
            ProviderType::Telegram => ProviderConfigDetail::Telegram(TelegramProviderConfig {
                bot_token: self.required_exact_secret_source(
                    "bot_token",
                    self.bot_token.as_deref(),
                    "bot_token_env",
                    self.bot_token_env.as_deref(),
                    ConfigError::InvalidTelegramBotTokenSource {
                        provider_id: self.id.clone(),
                    },
                )?,
                chat_id: self.required_exact_string("chat_id", self.chat_id.as_deref())?,
            }),
            ProviderType::Whatsapp => ProviderConfigDetail::Whatsapp(WhatsappProviderConfig {
                access_token: self.required_exact_secret_source(
                    "access_token",
                    self.access_token.as_deref(),
                    "access_token_env",
                    self.access_token_env.as_deref(),
                    ConfigError::InvalidWhatsappAccessTokenSource {
                        provider_id: self.id.clone(),
                    },
                )?,
                phone_number_id: self
                    .required_exact_string("phone_number_id", self.phone_number_id.as_deref())?,
                recipient_phone_number: self.required_exact_string(
                    "recipient_phone_number",
                    self.recipient_phone_number.as_deref(),
                )?,
            }),
            ProviderType::Wechat => ProviderConfigDetail::Wechat(WechatProviderConfig {
                base_url: self.required_exact_string("base_url", self.base_url.as_deref())?,
                token: self.required_exact_secret_source(
                    "token",
                    self.token.as_deref(),
                    "token_env",
                    self.token_env.as_deref(),
                    ConfigError::InvalidWechatTokenSource {
                        provider_id: self.id.clone(),
                    },
                )?,
                recipient_user_id: self.required_exact_string(
                    "recipient_user_id",
                    self.recipient_user_id.as_deref(),
                )?,
                context_token: self.required_exact_secret_source(
                    "context_token",
                    self.context_token.as_deref(),
                    "context_token_env",
                    self.context_token_env.as_deref(),
                    ConfigError::InvalidWechatContextTokenSource {
                        provider_id: self.id.clone(),
                    },
                )?,
                route_tag: present_exact_owned(self.route_tag.as_deref()),
            }),
            ProviderType::MicrosoftTeams => {
                ProviderConfigDetail::MicrosoftTeams(MicrosoftTeamsProviderConfig {
                    url: self.required_url_source(
                        ConfigError::InvalidMicrosoftTeamsUrlSource {
                            provider_id: self.id.clone(),
                        },
                        validate_microsoft_teams_webhook_url,
                    )?,
                })
            }
            ProviderType::EmailSmtp => {
                let username = self.optional_trimmed_secret_source(
                    "username",
                    self.username.as_deref(),
                    "username_env",
                    self.username_env.as_deref(),
                    ConfigError::InvalidEmailSmtpUsernameSource {
                        provider_id: self.id.clone(),
                    },
                )?;
                let password = self.optional_trimmed_secret_source(
                    "password",
                    self.password.as_deref(),
                    "password_env",
                    self.password_env.as_deref(),
                    ConfigError::InvalidEmailSmtpPasswordSource {
                        provider_id: self.id.clone(),
                    },
                )?;
                if username.is_some() != password.is_some() {
                    return Err(ConfigError::InvalidEmailSmtpCredentials {
                        provider_id: self.id.clone(),
                    });
                }

                ProviderConfigDetail::EmailSmtp(EmailSmtpProviderConfig {
                    host: self.required_trimmed_string("host", self.host.as_deref())?,
                    port: self.port.ok_or_else(|| ConfigError::MissingProviderField {
                        provider_id: self.id.clone(),
                        field: "port",
                    })?,
                    security: self
                        .security
                        .ok_or_else(|| ConfigError::MissingProviderField {
                            provider_id: self.id.clone(),
                            field: "security",
                        })?,
                    username,
                    password,
                    from: self.required_trimmed_string("from", self.from.as_deref())?,
                    to: self
                        .to
                        .as_ref()
                        .filter(|recipients| !recipients.is_empty())
                        .cloned()
                        .ok_or_else(|| ConfigError::MissingProviderField {
                            provider_id: self.id.clone(),
                            field: "to",
                        })?,
                    reply_to: present_trimmed_owned(self.reply_to.as_deref()),
                })
            }
        };

        Ok(ProviderConfig {
            id: self.id.clone(),
            detail,
        })
    }

    fn to_validated_feishu_lark_provider_config(
        &self,
    ) -> Result<FeishuLarkProviderConfig, ConfigError> {
        match self.feishu_lark_provider_mode()? {
            FeishuLarkRawProviderMode::CustomBot => {
                self.reject_feishu_lark_app_bot_fields()?;
                Ok(FeishuLarkProviderConfig::CustomBot(
                    FeishuLarkCustomBotProviderConfig {
                        url: self.required_url_source(
                            ConfigError::InvalidFeishuLarkUrlSource {
                                provider_id: self.id.clone(),
                            },
                            validate_feishu_lark_webhook_url,
                        )?,
                        secret: self.optional_exact_secret_source(
                            "secret",
                            self.secret.as_deref(),
                            "secret_env",
                            self.secret_env.as_deref(),
                            ConfigError::InvalidFeishuLarkSecretSource {
                                provider_id: self.id.clone(),
                            },
                        )?,
                    },
                ))
            }
            FeishuLarkRawProviderMode::AppBot => {
                self.reject_feishu_lark_custom_bot_fields()?;
                Ok(FeishuLarkProviderConfig::AppBot(
                    FeishuLarkAppBotProviderConfig {
                        domain: self.feishu_lark_app_bot_domain()?,
                        app_id: self.required_exact_string("app_id", self.app_id.as_deref())?,
                        app_secret: self.required_exact_secret_source(
                            "app_secret",
                            self.app_secret.as_deref(),
                            "app_secret_env",
                            self.app_secret_env.as_deref(),
                            ConfigError::InvalidFeishuLarkAppSecretSource {
                                provider_id: self.id.clone(),
                            },
                        )?,
                        app_registration_source: present_exact_owned(
                            self.app_registration_source.as_deref(),
                        ),
                        tenant_key: present_exact_owned(self.tenant_key.as_deref()),
                        chat_id: present_exact_owned(self.chat_id.as_deref()),
                    },
                ))
            }
        }
    }

    fn feishu_lark_provider_mode(&self) -> Result<FeishuLarkRawProviderMode, ConfigError> {
        let Some(mode) = present_trimmed_owned(self.mode.as_deref()) else {
            if self.has_feishu_lark_app_bot_fields() {
                return Err(ConfigError::MissingFeishuLarkAppBotMode {
                    provider_id: self.id.clone(),
                });
            }
            return Ok(FeishuLarkRawProviderMode::CustomBot);
        };

        match mode.as_str() {
            "custom_bot" => Ok(FeishuLarkRawProviderMode::CustomBot),
            "app_bot" => Ok(FeishuLarkRawProviderMode::AppBot),
            _ => Err(ConfigError::InvalidFeishuLarkMode {
                provider_id: self.id.clone(),
                mode,
            }),
        }
    }

    fn feishu_lark_app_bot_domain(&self) -> Result<FeishuLarkAppDomain, ConfigError> {
        let domain = self.required_exact_string("domain", self.domain.as_deref())?;
        match domain.as_str() {
            "feishu" => Ok(FeishuLarkAppDomain::Feishu),
            "lark" => Ok(FeishuLarkAppDomain::Lark),
            _ => Err(ConfigError::InvalidFeishuLarkAppBotDomain {
                provider_id: self.id.clone(),
                domain,
            }),
        }
    }

    fn reject_feishu_lark_app_bot_fields(&self) -> Result<(), ConfigError> {
        if self.has_feishu_lark_app_bot_fields() {
            return Err(ConfigError::InvalidFeishuLarkCustomBotAppFields {
                provider_id: self.id.clone(),
            });
        }
        Ok(())
    }

    fn reject_feishu_lark_custom_bot_fields(&self) -> Result<(), ConfigError> {
        if self.has_feishu_lark_custom_bot_fields() {
            return Err(ConfigError::InvalidFeishuLarkAppBotWebhookFields {
                provider_id: self.id.clone(),
            });
        }
        Ok(())
    }

    fn has_feishu_lark_app_bot_fields(&self) -> bool {
        [
            self.domain.as_deref(),
            self.app_id.as_deref(),
            self.app_secret.as_deref(),
            self.app_secret_env.as_deref(),
            self.app_registration_source.as_deref(),
            self.tenant_key.as_deref(),
            self.chat_id.as_deref(),
        ]
        .into_iter()
        .any(is_present)
    }

    fn has_feishu_lark_custom_bot_fields(&self) -> bool {
        [
            self.url.as_deref(),
            self.url_env.as_deref(),
            self.secret.as_deref(),
            self.secret_env.as_deref(),
        ]
        .into_iter()
        .any(is_present)
    }

    fn required_exact_string(
        &self,
        field: &'static str,
        value: Option<&str>,
    ) -> Result<String, ConfigError> {
        present_exact_owned(value).ok_or_else(|| ConfigError::MissingProviderField {
            provider_id: self.id.clone(),
            field,
        })
    }

    fn required_trimmed_string(
        &self,
        field: &'static str,
        value: Option<&str>,
    ) -> Result<String, ConfigError> {
        present_trimmed_owned(value).ok_or_else(|| ConfigError::MissingProviderField {
            provider_id: self.id.clone(),
            field,
        })
    }

    fn required_url_source(
        &self,
        source_error: ConfigError,
        validate: fn(&str) -> anyhow::Result<String>,
    ) -> Result<UrlSource, ConfigError> {
        let has_url = is_present(self.url.as_deref());
        let has_url_env = is_present(self.url_env.as_deref());
        if has_url == has_url_env {
            return Err(source_error);
        }
        if let Some(raw_url) = present_exact_owned(self.url.as_deref()) {
            validate(&raw_url)
                .map_err(|error| ConfigError::InvalidProviderUrl {
                    provider_id: self.id.clone(),
                    field: "url",
                    message: error.to_string(),
                })
                .map(UrlSource::Inline)
        } else {
            Ok(UrlSource::Env(
                present_exact_owned(self.url_env.as_deref()).expect("url_env presence was checked"),
            ))
        }
    }

    fn required_exact_secret_source(
        &self,
        _value_field: &'static str,
        value: Option<&str>,
        _env_field: &'static str,
        env_name: Option<&str>,
        source_error: ConfigError,
    ) -> Result<SecretSource, ConfigError> {
        match (present_exact_owned(value), present_exact_owned(env_name)) {
            (Some(value), None) => Ok(SecretSource::Inline(value)),
            (None, Some(env_name)) => Ok(SecretSource::Env(env_name)),
            (None, None) | (Some(_), Some(_)) => Err(source_error),
        }
    }

    fn optional_exact_secret_source(
        &self,
        _value_field: &'static str,
        value: Option<&str>,
        _env_field: &'static str,
        env_name: Option<&str>,
        source_error: ConfigError,
    ) -> Result<Option<SecretSource>, ConfigError> {
        match (present_exact_owned(value), present_exact_owned(env_name)) {
            (Some(value), None) => Ok(Some(SecretSource::Inline(value))),
            (None, Some(env_name)) => Ok(Some(SecretSource::Env(env_name))),
            (None, None) => Ok(None),
            (Some(_), Some(_)) => Err(source_error),
        }
    }

    fn optional_trimmed_secret_source(
        &self,
        _value_field: &'static str,
        value: Option<&str>,
        _env_field: &'static str,
        env_name: Option<&str>,
        source_error: ConfigError,
    ) -> Result<Option<SecretSource>, ConfigError> {
        match (
            present_trimmed_owned(value),
            present_trimmed_owned(env_name),
        ) {
            (Some(value), None) => Ok(Some(SecretSource::Inline(value))),
            (None, Some(env_name)) => Ok(Some(SecretSource::Env(env_name))),
            (None, None) => Ok(None),
            (Some(_), Some(_)) => Err(source_error),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeishuLarkRawProviderMode {
    CustomBot,
    AppBot,
}

fn is_present(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn present_exact_owned(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn present_trimmed_owned(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests;
