use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;
use uuid::Uuid;

use crate::config::{
    AnswerDetail, CONFIG_SCHEMA_VERSION, CliConfig, EmailSmtpSecurity, LogConfig,
    NotificationConfig, PromptDetail, ProviderType, RawConfig, RawProviderConfig, RouteConfig,
    SourceConfig, SourceType,
};
use crate::provider_urls::{
    host_label, validate_custom_webhook_url, validate_discord_webhook_url,
    validate_feishu_lark_webhook_url, validate_microsoft_teams_webhook_url,
    validate_slack_webhook_url,
};

const DEFAULT_NTFY_SERVER: &str = "https://ntfy.sh";
pub const DEFAULT_WECHAT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
pub const DEFAULT_WECHAT_BOT_TYPE: &str = "3";
pub const DEFAULT_WECHAT_QR_TIMEOUT: Duration = Duration::from_secs(480);
pub const DEFAULT_WECHAT_LINK_TIMEOUT: Duration = Duration::from_secs(180);
const WECHAT_SETUP_CHANNEL_VERSION: &str = "agents-router-wechat-setup/1.0";
pub const TEST_NOTIFICATION_SKIPPED_MESSAGE: &str =
    "Service is running. No test notification was sent.";

mod agents;
mod config_builders;
mod resolve;
mod targets;
mod wechat_setup;

pub use agents::*;
pub use config_builders::*;
pub use resolve::*;
pub use targets::*;
pub use wechat_setup::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtfySubscription {
    pub provider_id: String,
    pub server: String,
    pub topic: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuLarkTarget {
    pub provider_id: String,
    pub webhook_host: String,
    pub signed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuLarkAppBotTarget {
    pub provider_id: String,
    pub domain: String,
    pub app_id: String,
    pub app_secret_configured: bool,
    pub tenant_key: String,
    pub chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookTarget {
    pub provider_id: String,
    pub webhook_host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushoverTarget {
    pub provider_id: String,
    pub device: Option<String>,
    pub sound: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackTarget {
    pub provider_id: String,
    pub webhook_host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordTarget {
    pub provider_id: String,
    pub webhook_host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramTarget {
    pub provider_id: String,
    pub chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsappTarget {
    pub provider_id: String,
    pub recipient_phone_number: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatTarget {
    pub provider_id: String,
    pub base_url_host: String,
    pub recipient_user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftTeamsTarget {
    pub provider_id: String,
    pub webhook_host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatQrCode {
    pub qr_key: String,
    pub qr_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatQrStatus {
    pub status: String,
    pub token: Option<String>,
    pub account_id: Option<String>,
    pub base_url: Option<String>,
    pub scanned_user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatRecipientLink {
    pub recipient_user_id: String,
    pub context_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSmtpTarget {
    pub provider_id: String,
    pub host: String,
    pub port: u16,
    pub from: String,
    pub to: Vec<String>,
}

pub fn write_config(path: &Path, config: &RawConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory `{}`", parent.display()))?;
    }

    let raw = toml::to_string_pretty(config).context("failed to serialize config")?;
    let temp_path = temporary_config_path(path)?;
    fs::write(&temp_path, raw)
        .with_context(|| format!("failed to write config `{}`", temp_path.display()))?;
    fs::rename(&temp_path, path).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to replace config `{}` with `{}`",
            path.display(),
            temp_path.display()
        )
    })
}

fn temporary_config_path(path: &Path) -> anyhow::Result<std::path::PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .context("config path must include a file name")?;
    let temp_file_name = format!(".{file_name}.{}.tmp", Uuid::new_v4());
    Ok(path.with_file_name(temp_file_name))
}

pub fn missing_config_message(path: &str) -> String {
    format!(
        r#"No agents-router config found at `{path}`.

Run `agents-router setup` in an interactive terminal to choose an agent and a notification provider.

If you are running non-interactively, create the config file first or pass `--config <PATH>`."#
    )
}

fn is_valid_ntfy_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn is_pushover_identifier(value: &str) -> bool {
    value.len() == 30 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn webhook_host(url: &str) -> String {
    host_label(url)
}

fn provider_url_host(provider: &RawProviderConfig) -> Option<String> {
    match (
        provider
            .url
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
        provider
            .url_env
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
    ) {
        (Some(url), _) => Some(webhook_host(url)),
        (None, Some(env_name)) => Some(format!("env:{env_name}")),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
