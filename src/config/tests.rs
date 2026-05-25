use super::*;
use crate::providers::build_providers;

const VALID_CONFIG: &str = r#"
schema_version = 1

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "my-codex-alerts"

[[providers]]
id = "debug_webhook"
type = "webhook"
url_env = "AGENTS_ROUTER_WEBHOOK_URL"

[[providers]]
id = "work_chat"
type = "feishu_lark"
url_env = "AGENTS_ROUTER_FEISHU_LARK_WEBHOOK_URL"
secret_env = "AGENTS_ROUTER_FEISHU_LARK_SECRET"

[[providers]]
id = "pushover"
type = "pushover"
app_token_env = "AGENTS_ROUTER_PUSHOVER_APP_TOKEN"
user_key_env = "AGENTS_ROUTER_PUSHOVER_USER_KEY"

[[providers]]
id = "slack"
type = "slack"
url_env = "AGENTS_ROUTER_SLACK_WEBHOOK_URL"

[[providers]]
id = "discord"
type = "discord"
url_env = "AGENTS_ROUTER_DISCORD_WEBHOOK_URL"

[[providers]]
id = "telegram"
type = "telegram"
bot_token_env = "AGENTS_ROUTER_TELEGRAM_BOT_TOKEN"
chat_id = "123456789"

[[providers]]
id = "whatsapp"
type = "whatsapp"
access_token_env = "AGENTS_ROUTER_WHATSAPP_ACCESS_TOKEN"
phone_number_id = "123456789"
recipient_phone_number = "15551234567"

[[providers]]
id = "wechat"
type = "wechat"
base_url = "https://ilinkai.weixin.qq.com"
token_env = "AGENTS_ROUTER_WECHAT_TOKEN"
recipient_user_id = "user@im.wechat"
context_token_env = "AGENTS_ROUTER_WECHAT_CONTEXT_TOKEN"

[[providers]]
id = "microsoft_teams"
type = "microsoft_teams"
url_env = "AGENTS_ROUTER_MICROSOFT_TEAMS_WEBHOOK_URL"

[[providers]]
id = "email"
type = "email_smtp"
host = "smtp.example.com"
port = 587
security = "starttls"
username_env = "AGENTS_ROUTER_EMAIL_SMTP_USERNAME"
password_env = "AGENTS_ROUTER_EMAIL_SMTP_PASSWORD"
from = "Agents Router <alerts@example.com>"
to = ["felix@example.com"]

[[routes]]
sources = ["codex_desktop", "codex_cli"]
providers = ["phone", "debug_webhook", "work_chat", "pushover", "slack", "discord", "telegram", "whatsapp", "wechat", "microsoft_teams", "email"]
"#;

#[test]
fn parses_valid_config() {
    let config = ValidatedConfig::from_toml_str(VALID_CONFIG).expect("valid config should parse");

    assert_eq!(config.schema_version, 1);
    assert_eq!(config.sources.len(), 2);
    assert_eq!(config.providers.len(), 11);
    assert_eq!(config.routes.len(), 1);
    assert_eq!(config.cli.language, CliLanguage::English);
    assert_eq!(config.log.level, "info");
    assert_eq!(config.notification.answer_detail, AnswerDetail::Preview);
    assert_eq!(config.notification.prompt_detail, PromptDetail::Off);
}

#[test]
fn parses_and_serializes_route_filters() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_desktop"]
providers = ["debug_webhook"]
minimum_task_duration_minutes = 5
only_forward_from_project_paths = [
  "/Users/tester/projects/agents-router",
  "/Users/tester/projects/other",
]
"#;

    let loaded = LoadedConfig::from_toml_str(raw).expect("route filters should parse");
    let config = loaded.validated;

    assert_eq!(config.routes[0].minimum_task_duration_minutes, Some(5));
    assert_eq!(
        config.routes[0].only_forward_from_project_paths,
        vec![
            "/Users/tester/projects/agents-router".to_string(),
            "/Users/tester/projects/other".to_string(),
        ]
    );

    let serialized = toml::to_string_pretty(&loaded.raw).expect("config should serialize");
    assert!(serialized.contains("minimum_task_duration_minutes = 5"));
    assert!(serialized.contains("only_forward_from_project_paths"));
    assert!(!serialized.contains("response_surface"));
}

#[test]
fn route_response_surface_is_disabled_by_default_and_not_serialized() {
    let loaded = LoadedConfig::from_toml_str(VALID_CONFIG).expect("valid config should parse");

    assert!(!loaded.validated.routes[0].response_surface.enabled);
    assert!(!loaded.raw.routes[0].response_surface.enabled);

    let serialized = toml::to_string_pretty(&loaded.raw).expect("config should serialize");
    assert!(!serialized.contains("response_surface"));
    assert!(!serialized.contains("enabled = false"));
}

#[test]
fn route_response_surface_toml_enables_replies_when_explicit() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_desktop"]
providers = ["debug_webhook"]
response_surface = { enabled = true }
"#;

    let loaded = LoadedConfig::from_toml_str(raw).expect("response surface TOML should parse");

    assert!(loaded.raw.routes[0].response_surface.enabled);
    assert!(loaded.validated.routes[0].response_surface.enabled);

    let serialized = toml::to_string_pretty(&loaded.raw).expect("config should serialize");
    assert!(serialized.contains("response_surface"));
    assert!(serialized.contains("enabled = true"));
}

#[test]
fn rejects_zero_route_minimum_task_duration() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_desktop"]
providers = ["debug_webhook"]
minimum_task_duration_minutes = 0
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("zero route duration should be rejected");

    assert!(matches!(
        err,
        ConfigError::InvalidMinimumTaskDuration { route_index: 0 }
    ));
}

#[test]
fn rejects_too_large_route_minimum_task_duration() {
    let raw = format!(
        r#"
schema_version = 1

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_desktop"]
providers = ["debug_webhook"]
minimum_task_duration_minutes = {}
"#,
        u64::MAX
    );

    let err = ValidatedConfig::from_toml_str(&raw)
        .expect_err("oversized route duration should be rejected");

    assert!(matches!(
        err,
        ConfigError::MinimumTaskDurationTooLarge { route_index: 0 }
    ));
}

#[test]
fn rejects_invalid_route_project_paths() {
    for invalid_path in [
        "",
        "relative/project",
        "/Users/tester/../project",
        "/Users/./project",
    ] {
        let raw = format!(
            r#"
schema_version = 1

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_desktop"]
providers = ["debug_webhook"]
only_forward_from_project_paths = ["{invalid_path}"]
"#
        );

        let err = ValidatedConfig::from_toml_str(&raw)
            .expect_err("invalid project path should be rejected");

        assert!(matches!(
            err,
            ConfigError::InvalidRouteProjectPath { route_index: 0, .. }
        ));
    }
}

#[test]
fn parses_cli_language() {
    let raw = r#"
schema_version = 1

[cli]
language = "zh-CN"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_cli"]
providers = ["debug_webhook"]
"#;

    let config = ValidatedConfig::from_toml_str(raw).expect("valid config should parse");

    assert_eq!(config.cli.language, CliLanguage::SimplifiedChinese);
}

#[test]
fn parses_cli_language_aliases() {
    assert_eq!(CliLanguage::parse("en"), Some(CliLanguage::English));
    assert_eq!(
        CliLanguage::parse("zh_CN"),
        Some(CliLanguage::SimplifiedChinese)
    );
    assert_eq!(
        CliLanguage::parse("简体中文"),
        Some(CliLanguage::SimplifiedChinese)
    );
    assert_eq!(CliLanguage::parse("fr"), None);
}

#[test]
fn parses_full_answer_detail() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_cli"]
providers = ["debug_webhook"]
"#;

    let config = ValidatedConfig::from_toml_str(raw).expect("valid config should parse");

    assert_eq!(config.notification.answer_detail, AnswerDetail::Full);
}

#[test]
fn rejects_full_answer_detail_with_ntfy_provider() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["codex_cli"]
providers = ["phone"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("ntfy should reject full answer detail");

    assert!(matches!(
        err,
        ConfigError::AnswerDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "phone" && provider_type == "ntfy"
    ));
}

#[test]
fn rejects_full_answer_detail_with_pushover_provider() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "pushover"
type = "pushover"
app_token = "123456789012345678901234567890"
user_key = "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ"

[[routes]]
sources = ["codex_cli"]
providers = ["pushover"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("Pushover should reject full answer detail");

    assert!(matches!(
        err,
        ConfigError::AnswerDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "pushover" && provider_type == "pushover"
    ));
}

#[test]
fn rejects_full_answer_detail_with_slack_provider() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "slack"
type = "slack"
url = "https://hooks.slack.com/services/T00000000/B00000000/test-token"

[[routes]]
sources = ["codex_cli"]
providers = ["slack"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("Slack should reject full answer detail");

    assert!(matches!(
        err,
        ConfigError::AnswerDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "slack" && provider_type == "slack"
    ));
}

#[test]
fn rejects_full_answer_detail_with_discord_provider() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "discord"
type = "discord"
url = "https://discord.com/api/webhooks/123456789012345678/token"

[[routes]]
sources = ["codex_cli"]
providers = ["discord"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("Discord should reject full answer detail");

    assert!(matches!(
        err,
        ConfigError::AnswerDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "discord" && provider_type == "discord"
    ));
}

#[test]
fn rejects_full_answer_detail_with_telegram_provider() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "telegram"
type = "telegram"
bot_token = "123456:test-token"
chat_id = "123456789"

[[routes]]
sources = ["codex_cli"]
providers = ["telegram"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("Telegram should reject full answer detail");

    assert!(matches!(
        err,
        ConfigError::AnswerDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "telegram" && provider_type == "telegram"
    ));
}

#[test]
fn rejects_full_answer_detail_with_wechat_provider() {
    let raw = r#"
schema_version = 1

[notification]
answer_detail = "full"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "wechat"
type = "wechat"
base_url = "https://ilinkai.weixin.qq.com"
token = "test-token"
recipient_user_id = "user@im.wechat"
context_token = "test-context-token"

[[routes]]
sources = ["codex_cli"]
providers = ["wechat"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("WeChat should reject full answer detail");

    assert!(matches!(
        err,
        ConfigError::AnswerDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "wechat" && provider_type == "wechat"
    ));
}

#[test]
fn parses_on_prompt_detail() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = "https://example.com/hook"

[[routes]]
sources = ["codex_cli"]
providers = ["debug_webhook"]
"#;

    let config = ValidatedConfig::from_toml_str(raw).expect("valid config should parse");

    assert_eq!(config.notification.prompt_detail, PromptDetail::On);
}

#[test]
fn rejects_prompt_detail_on_with_ntfy_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["codex_cli"]
providers = ["phone"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("ntfy should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "phone" && provider_type == "ntfy"
    ));
}

#[test]
fn rejects_prompt_detail_on_with_pushover_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "pushover"
type = "pushover"
app_token = "123456789012345678901234567890"
user_key = "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ"

[[routes]]
sources = ["codex_cli"]
providers = ["pushover"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("Pushover should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "pushover" && provider_type == "pushover"
    ));
}

#[test]
fn rejects_prompt_detail_on_with_slack_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "slack"
type = "slack"
url = "https://hooks.slack.com/services/T00000000/B00000000/test-token"

[[routes]]
sources = ["codex_cli"]
providers = ["slack"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("Slack should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "slack" && provider_type == "slack"
    ));
}

#[test]
fn rejects_prompt_detail_on_with_discord_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "discord"
type = "discord"
url = "https://discord.com/api/webhooks/123456789012345678/token"

[[routes]]
sources = ["codex_cli"]
providers = ["discord"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("Discord should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "discord" && provider_type == "discord"
    ));
}

#[test]
fn rejects_prompt_detail_on_with_whatsapp_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "whatsapp"
type = "whatsapp"
access_token = "test-access-token"
phone_number_id = "123456789"
recipient_phone_number = "15551234567"

[[routes]]
sources = ["codex_cli"]
providers = ["whatsapp"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("WhatsApp should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "whatsapp" && provider_type == "whatsapp"
    ));
}

#[test]
fn rejects_prompt_detail_on_with_wechat_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "wechat"
type = "wechat"
base_url = "https://ilinkai.weixin.qq.com"
token = "test-token"
recipient_user_id = "user@im.wechat"
context_token = "test-context-token"

[[routes]]
sources = ["codex_cli"]
providers = ["wechat"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("WeChat should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "wechat" && provider_type == "wechat"
    ));
}

#[test]
fn rejects_prompt_detail_on_with_microsoft_teams_provider() {
    let raw = r#"
schema_version = 1

[notification]
prompt_detail = "on"

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "microsoft_teams"
type = "microsoft_teams"
url = "https://example.com/workflow"

[[routes]]
sources = ["codex_cli"]
providers = ["microsoft_teams"]
"#;

    let err = ValidatedConfig::from_toml_str(raw)
        .expect_err("Microsoft Teams should reject prompt detail");

    assert!(matches!(
        err,
        ConfigError::PromptDetailNotSupportedForProvider {
            provider_id,
            provider_type
        } if provider_id == "microsoft_teams" && provider_type == "microsoft_teams"
    ));
}

#[test]
fn rejects_unsupported_schema_version() {
    let err = ValidatedConfig::from_toml_str(
        &VALID_CONFIG.replace("schema_version = 1", "schema_version = 2"),
    )
    .expect_err("unsupported schema should fail");

    assert!(matches!(
        err,
        ConfigError::UnsupportedSchema { found: 2, .. }
    ));
}

#[test]
fn rejects_duplicate_source_id() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[sources]]
id = "codex_cli"
type = "codex_desktop"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["codex_cli"]
providers = ["phone"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("duplicate source id should fail");

    assert!(matches!(err, ConfigError::DuplicateSourceId(id) if id == "codex_cli"));
}

#[test]
fn rejects_noncanonical_codex_cli_source_id_during_validation() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "my_codex"
type = "codex_cli"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["my_codex"]
providers = ["phone"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("noncanonical Codex CLI id should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidSourceIdForType {
            source_id,
            source_type: "codex_cli",
            expected_id: "codex_cli",
            ..
        } if source_id == "my_codex"
    ));
}

#[test]
fn rejects_noncanonical_claude_code_source_id_during_validation() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "my_claude"
type = "claude_code"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["my_claude"]
providers = ["phone"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("noncanonical Claude Code id should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidSourceIdForType {
            source_id,
            source_type: "claude_code",
            expected_id: "claude_code",
            ..
        } if source_id == "my_claude"
    ));
}

#[test]
fn accepts_custom_codex_desktop_source_id() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "my_desktop"
type = "codex_desktop"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["my_desktop"]
providers = ["phone"]
"#;

    let config =
        ValidatedConfig::from_toml_str(raw).expect("custom Codex Desktop id should validate");

    assert_eq!(config.sources[0].id, "my_desktop");
    assert_eq!(config.sources[0].source_type, SourceType::CodexDesktop);
}

#[test]
fn accepts_agent_hook_that_reuses_codex_cli_source_id() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "agent_hook"

[[providers]]
id = "phone"
type = "ntfy"
server = "https://ntfy.sh"
topic = "topic"

[[routes]]
sources = ["codex_cli"]
providers = ["phone"]
"#;

    let config =
        ValidatedConfig::from_toml_str(raw).expect("generic agent_hook id should validate");

    assert_eq!(config.sources[0].id, "codex_cli");
    assert_eq!(config.sources[0].source_type, SourceType::AgentHook);
}

#[test]
fn rejects_unknown_route_source() {
    let err = ValidatedConfig::from_toml_str(
        &VALID_CONFIG.replace("codex_desktop\", \"codex_cli", "missing"),
    )
    .expect_err("unknown source should fail");

    assert!(matches!(err, ConfigError::UnknownRouteSource(id) if id == "missing"));
}

#[test]
fn rejects_unknown_route_provider() {
    let err = ValidatedConfig::from_toml_str(
        &VALID_CONFIG.replace("phone\", \"debug_webhook", "missing"),
    )
    .expect_err("unknown provider should fail");

    assert!(matches!(err, ConfigError::UnknownRouteProvider(id) if id == "missing"));
}

#[test]
fn rejects_webhook_with_both_url_sources() {
    let raw = VALID_CONFIG.replace(
        "url_env = \"AGENTS_ROUTER_WEBHOOK_URL\"",
        "url = \"https://example.com/hook\"\nurl_env = \"AGENTS_ROUTER_WEBHOOK_URL\"",
    );

    let err = ValidatedConfig::from_toml_str(&raw).expect_err("ambiguous webhook url should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidWebhookUrlSource { provider_id } if provider_id == "debug_webhook"
    ));
}

#[test]
fn rejects_feishu_lark_without_webhook_url() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_chat"
type = "feishu_lark"

[[routes]]
sources = ["codex_cli"]
providers = ["work_chat"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("missing webhook URL should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkUrlSource { provider_id } if provider_id == "work_chat"
    ));
}

#[test]
fn rejects_feishu_lark_with_both_secret_sources() {
    let raw = VALID_CONFIG.replace(
        "secret_env = \"AGENTS_ROUTER_FEISHU_LARK_SECRET\"",
        "secret = \"inline-secret\"\nsecret_env = \"AGENTS_ROUTER_FEISHU_LARK_SECRET\"",
    );

    let err =
        ValidatedConfig::from_toml_str(&raw).expect_err("ambiguous secret source should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkSecretSource { provider_id } if provider_id == "work_chat"
    ));
}

#[test]
fn parses_explicit_feishu_lark_app_bot_config_without_reading_env() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_lark_app"
type = "feishu_lark"
mode = "app_bot"
domain = "lark"
app_id = "cli_9f5343c580712544"
app_secret_env = "AGENTS_ROUTER_LARK_APP_SECRET"
tenant_key = "2ca1d211f64f6438"
chat_id = "oc_5ce6d572455d361153b7xx51da133945"

[[routes]]
sources = ["codex_cli"]
providers = ["work_lark_app"]
"#;

    let config = ValidatedConfig::from_toml_str(raw).expect("explicit App Bot config should parse");
    let provider = config
        .provider("work_lark_app")
        .expect("validated provider should exist");

    assert!(matches!(
        &provider.detail,
        ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(
            FeishuLarkAppBotProviderConfig {
                domain: FeishuLarkAppDomain::Lark,
                app_id,
                app_secret: SecretSource::Env(app_secret_env),
                tenant_key,
                chat_id,
            }
        )) if app_id == "cli_9f5343c580712544"
            && app_secret_env == "AGENTS_ROUTER_LARK_APP_SECRET"
            && tenant_key == "2ca1d211f64f6438"
            && chat_id == "oc_5ce6d572455d361153b7xx51da133945"
    ));
}

#[test]
fn feishu_lark_app_bot_mode_must_be_explicit() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_lark_app"
type = "feishu_lark"
domain = "lark"
app_id = "cli_9f5343c580712544"
app_secret_env = "AGENTS_ROUTER_LARK_APP_SECRET"
tenant_key = "2ca1d211f64f6438"
chat_id = "oc_5ce6d572455d361153b7xx51da133945"

[[routes]]
sources = ["codex_cli"]
providers = ["work_lark_app"]
"#;

    let err = ValidatedConfig::from_toml_str(raw)
        .expect_err("App Bot fields without explicit mode should fail");

    assert!(matches!(
        err,
        ConfigError::MissingFeishuLarkAppBotMode { provider_id }
            if provider_id == "work_lark_app"
    ));
}

#[test]
fn feishu_lark_app_bot_rejects_custom_bot_webhook_fields() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_lark_app"
type = "feishu_lark"
mode = "app_bot"
domain = "lark"
app_id = "cli_9f5343c580712544"
app_secret_env = "AGENTS_ROUTER_LARK_APP_SECRET"
tenant_key = "2ca1d211f64f6438"
chat_id = "oc_5ce6d572455d361153b7xx51da133945"
url = "https://open.larksuite.com/open-apis/bot/v2/hook/test"

[[routes]]
sources = ["codex_cli"]
providers = ["work_lark_app"]
"#;

    let err = ValidatedConfig::from_toml_str(raw)
        .expect_err("App Bot must not accept Custom Bot webhook fields");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkAppBotWebhookFields { provider_id }
            if provider_id == "work_lark_app"
    ));
}

#[test]
fn feishu_lark_custom_bot_rejects_app_bot_fields() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_chat"
type = "feishu_lark"
mode = "custom_bot"
url = "https://open.larksuite.com/open-apis/bot/v2/hook/test"
app_id = "cli_9f5343c580712544"

[[routes]]
sources = ["codex_cli"]
providers = ["work_chat"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("Custom Bot must not accept App Bot fields");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkCustomBotAppFields { provider_id }
            if provider_id == "work_chat"
    ));
}

#[test]
fn rejects_unknown_feishu_lark_mode() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_chat"
type = "feishu_lark"
mode = "socket_mode"

[[routes]]
sources = ["codex_cli"]
providers = ["work_chat"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("unknown Feishu/Lark mode should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkMode { provider_id, mode }
            if provider_id == "work_chat" && mode == "socket_mode"
    ));
}

#[test]
fn rejects_invalid_feishu_lark_app_bot_domain() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_lark_app"
type = "feishu_lark"
mode = "app_bot"
domain = "global"
app_id = "cli_9f5343c580712544"
app_secret_env = "AGENTS_ROUTER_LARK_APP_SECRET"
tenant_key = "2ca1d211f64f6438"
chat_id = "oc_5ce6d572455d361153b7xx51da133945"

[[routes]]
sources = ["codex_cli"]
providers = ["work_lark_app"]
"#;

    let err = ValidatedConfig::from_toml_str(raw)
        .expect_err("unsupported Feishu/Lark App Bot domain should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkAppBotDomain {
            provider_id,
            domain
        } if provider_id == "work_lark_app" && domain == "global"
    ));
}

#[test]
fn rejects_feishu_lark_app_bot_with_both_secret_sources() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "work_lark_app"
type = "feishu_lark"
mode = "app_bot"
domain = "lark"
app_id = "cli_9f5343c580712544"
app_secret = "inline-secret"
app_secret_env = "AGENTS_ROUTER_LARK_APP_SECRET"
tenant_key = "2ca1d211f64f6438"
chat_id = "oc_5ce6d572455d361153b7xx51da133945"

[[routes]]
sources = ["codex_cli"]
providers = ["work_lark_app"]
"#;

    let err = ValidatedConfig::from_toml_str(raw)
        .expect_err("ambiguous App Bot app secret source should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidFeishuLarkAppSecretSource { provider_id }
            if provider_id == "work_lark_app"
    ));
}

#[test]
fn rejects_pushover_without_app_token_source() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "pushover"
type = "pushover"
user_key = "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ"

[[routes]]
sources = ["codex_cli"]
providers = ["pushover"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("missing app token should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidPushoverAppTokenSource { provider_id } if provider_id == "pushover"
    ));
}

#[test]
fn rejects_pushover_with_both_user_key_sources() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "pushover"
type = "pushover"
app_token = "123456789012345678901234567890"
user_key = "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ"
user_key_env = "AGENTS_ROUTER_PUSHOVER_USER_KEY"

[[routes]]
sources = ["codex_cli"]
providers = ["pushover"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("ambiguous user key should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidPushoverUserKeySource { provider_id } if provider_id == "pushover"
    ));
}

#[test]
fn rejects_slack_without_webhook_url() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "slack"
type = "slack"

[[routes]]
sources = ["codex_cli"]
providers = ["slack"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("missing Slack webhook URL should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidSlackUrlSource { provider_id } if provider_id == "slack"
    ));
}

#[test]
fn rejects_invalid_inline_provider_urls() {
    let cases = [
        (
            "webhook",
            r#"type = "webhook"
url = "http://example.com/hook""#,
        ),
        (
            "work_chat",
            r#"type = "feishu_lark"
url = "https://example.com/hook""#,
        ),
        (
            "slack",
            r#"type = "slack"
url = "https://example.com/services/T00000000/B00000000/test-token""#,
        ),
        (
            "discord",
            r#"type = "discord"
url = "https://example.com/api/webhooks/123456789012345678/token""#,
        ),
        (
            "microsoft_teams",
            r#"type = "microsoft_teams"
url = "http://example.com/workflow""#,
        ),
    ];

    for (provider_id, provider_fields) in cases {
        let raw = format!(
            r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "{provider_id}"
{provider_fields}

[[routes]]
sources = ["codex_cli"]
providers = ["{provider_id}"]
"#
        );

        let err =
            ValidatedConfig::from_toml_str(&raw).expect_err("invalid provider URL should fail");

        assert!(matches!(
            err,
            ConfigError::InvalidProviderUrl {
                provider_id: actual_provider_id,
                field: "url",
                ..
            } if actual_provider_id == provider_id
        ));
    }
}

#[test]
fn rejects_discord_with_both_url_sources() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "discord"
type = "discord"
url = "https://discord.com/api/webhooks/123456789012345678/token"
url_env = "AGENTS_ROUTER_DISCORD_WEBHOOK_URL"

[[routes]]
sources = ["codex_cli"]
providers = ["discord"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("ambiguous Discord URL should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidDiscordUrlSource { provider_id } if provider_id == "discord"
    ));
}

#[test]
fn rejects_telegram_without_bot_token_source() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "telegram"
type = "telegram"
chat_id = "123456789"

[[routes]]
sources = ["codex_cli"]
providers = ["telegram"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("missing Telegram bot token should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidTelegramBotTokenSource { provider_id } if provider_id == "telegram"
    ));
}

#[test]
fn rejects_whatsapp_without_access_token_source() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "whatsapp"
type = "whatsapp"
phone_number_id = "123456789"
recipient_phone_number = "15551234567"

[[routes]]
sources = ["codex_cli"]
providers = ["whatsapp"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("missing WhatsApp access token should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidWhatsappAccessTokenSource { provider_id } if provider_id == "whatsapp"
    ));
}

#[test]
fn rejects_wechat_without_token_source() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "wechat"
type = "wechat"
base_url = "https://ilinkai.weixin.qq.com"
recipient_user_id = "user@im.wechat"
context_token = "test-context-token"

[[routes]]
sources = ["codex_cli"]
providers = ["wechat"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("missing WeChat token should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidWechatTokenSource { provider_id } if provider_id == "wechat"
    ));
}

#[test]
fn rejects_wechat_without_context_token_source() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "wechat"
type = "wechat"
base_url = "https://ilinkai.weixin.qq.com"
token = "test-token"
recipient_user_id = "user@im.wechat"

[[routes]]
sources = ["codex_cli"]
providers = ["wechat"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("missing WeChat context token should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidWechatContextTokenSource { provider_id } if provider_id == "wechat"
    ));
}

#[test]
fn rejects_microsoft_teams_with_both_url_sources() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "microsoft_teams"
type = "microsoft_teams"
url = "https://example.com/workflow"
url_env = "AGENTS_ROUTER_MICROSOFT_TEAMS_WEBHOOK_URL"

[[routes]]
sources = ["codex_cli"]
providers = ["microsoft_teams"]
"#;

    let err =
        ValidatedConfig::from_toml_str(raw).expect_err("ambiguous Microsoft Teams URL should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidMicrosoftTeamsUrlSource { provider_id } if provider_id == "microsoft_teams"
    ));
}

#[test]
fn rejects_email_smtp_without_required_fields() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "email"
type = "email_smtp"
host = "smtp.example.com"
port = 587
security = "starttls"
from = "alerts@example.com"

[[routes]]
sources = ["codex_cli"]
providers = ["email"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("missing recipients should fail");

    assert!(matches!(
        err,
        ConfigError::MissingProviderField { provider_id, field }
            if provider_id == "email" && field == "to"
    ));
}

#[test]
fn rejects_email_smtp_partial_credentials() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "email"
type = "email_smtp"
host = "smtp.example.com"
port = 587
security = "starttls"
username = "alerts@example.com"
from = "alerts@example.com"
to = ["felix@example.com"]

[[routes]]
sources = ["codex_cli"]
providers = ["email"]
"#;

    let err = ValidatedConfig::from_toml_str(raw).expect_err("partial credentials should fail");

    assert!(matches!(
        err,
        ConfigError::InvalidEmailSmtpCredentials { provider_id } if provider_id == "email"
    ));
}

#[test]
fn provider_structure_errors_still_win_before_route_reference_errors() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"

[[routes]]
sources = ["missing_source"]
providers = ["debug_webhook"]
"#;

    let err = ValidatedConfig::from_toml_str(raw)
        .expect_err("provider structure errors should stay first");

    assert!(matches!(
        err,
        ConfigError::InvalidWebhookUrlSource { provider_id } if provider_id == "debug_webhook"
    ));
}

#[test]
fn validated_provider_type_is_derived_from_detail() {
    let provider = ProviderConfig {
        id: "debug".to_string(),
        detail: ProviderConfigDetail::Webhook(WebhookProviderConfig {
            url: UrlSource::Inline("https://example.com/hook".to_string()),
        }),
    };

    assert_eq!(provider.provider_type(), ProviderType::Webhook);
}

#[test]
fn validation_preserves_exact_provider_identifiers_and_secrets() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "telegram"
type = "telegram"
bot_token = " 123456:secret "
chat_id = " 123456789 "

[[routes]]
sources = ["codex_cli"]
providers = ["telegram"]
"#;

    let config =
        ValidatedConfig::from_toml_str(raw).expect("structure validation should not trim tokens");
    let provider = config
        .provider("telegram")
        .expect("validated provider should exist");

    assert!(matches!(
        &provider.detail,
        ProviderConfigDetail::Telegram(TelegramProviderConfig {
            bot_token: SecretSource::Inline(bot_token),
            chat_id,
        }) if bot_token == " 123456:secret " && chat_id == " 123456789 "
    ));

    let err = match build_providers(&config) {
        Ok(_) => panic!("runtime provider build should reject the bad token"),
        Err(error) => error,
    };
    assert!(err.to_string().contains("bot_token"));
}

#[test]
fn validation_preserves_exact_provider_env_names() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"
url_env = " AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL "

[[routes]]
sources = ["codex_cli"]
providers = ["debug_webhook"]
"#;

    let config = ValidatedConfig::from_toml_str(raw)
        .expect("structure validation should not trim env names");
    let provider = config
        .provider("debug_webhook")
        .expect("validated provider should exist");

    assert!(matches!(
        &provider.detail,
        ProviderConfigDetail::Webhook(WebhookProviderConfig {
            url: UrlSource::Env(env_name),
        }) if env_name == " AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL "
    ));
}

#[test]
fn validation_keeps_inline_url_normalization_explicit() {
    let raw = r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"
url = " https://example.com/hook "

[[routes]]
sources = ["codex_cli"]
providers = ["debug_webhook"]
"#;

    let config =
        ValidatedConfig::from_toml_str(raw).expect("inline URL normalization should be accepted");
    let provider = config
        .provider("debug_webhook")
        .expect("validated provider should exist");

    assert!(matches!(
        &provider.detail,
        ProviderConfigDetail::Webhook(WebhookProviderConfig {
            url: UrlSource::Inline(url),
        }) if url == "https://example.com/hook"
    ));
}

#[test]
fn raw_parse_and_validation_do_not_read_provider_env_values() {
    let _guard = EnvVarGuard::unset("AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL");
    let raw = runtime_env_webhook_config();

    let parsed = RawConfig::from_toml_str(raw).expect("raw config should parse without env");
    assert_eq!(
        parsed.providers[0].url_env.as_deref(),
        Some("AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL")
    );

    let validated =
        ValidatedConfig::from_toml_str(raw).expect("validated config should not read env");
    let provider = validated
        .provider("debug_webhook")
        .expect("validated provider should exist");
    assert!(matches!(
        &provider.detail,
        ProviderConfigDetail::Webhook(WebhookProviderConfig {
            url: UrlSource::Env(env_name),
        }) if env_name == "AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL"
    ));
}

#[test]
fn provider_build_reads_env_values_at_runtime_boundary() {
    let _guard = EnvVarGuard::unset("AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL");
    let config = ValidatedConfig::from_toml_str(runtime_env_webhook_config())
        .expect("validated config should not read env");

    let err = match build_providers(&config) {
        Ok(_) => panic!("provider build should read missing env"),
        Err(error) => error,
    };

    assert!(
        err.to_string()
            .contains("AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL")
    );
}

fn runtime_env_webhook_config() -> &'static str {
    r#"
schema_version = 1

[[sources]]
id = "codex_cli"
type = "codex_cli"

[[providers]]
id = "debug_webhook"
type = "webhook"
url_env = "AGENTS_ROUTER_TEST_RUNTIME_WEBHOOK_URL"

[[routes]]
sources = ["codex_cli"]
providers = ["debug_webhook"]
"#
}

struct EnvVarGuard {
    name: &'static str,
}

impl EnvVarGuard {
    fn unset(name: &'static str) -> Self {
        // SAFETY: this test uses a unique env var name scoped to this module.
        unsafe { std::env::remove_var(name) };
        Self { name }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: this removes only the unique env var controlled by the test guard.
        unsafe { std::env::remove_var(self.name) };
    }
}
