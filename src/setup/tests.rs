use tempfile::tempdir;

use super::*;

fn read_valid_written_config(path: &std::path::Path) -> RawConfig {
    crate::config::LoadedConfig::from_path(path)
        .expect("written config should parse and validate")
        .raw
}

#[test]
fn empty_topic_input_uses_generated_topic() {
    let topic = resolve_ntfy_topic("  ", "agents-router-test")
        .expect("empty input should use generated topic");

    assert_eq!(topic, "agents-router-test");
}

#[test]
fn rejects_topic_with_path_separator() {
    let err = resolve_ntfy_topic("bad/topic", "agents-router-test")
        .expect_err("path separators should be rejected");

    assert!(err.to_string().contains("letters, numbers"));
}

#[test]
fn writes_parseable_ntfy_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_ntfy_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(parsed.notification.answer_detail, AnswerDetail::Preview);
    assert_eq!(
        parsed
            .provider("ntfy")
            .and_then(|provider| provider.topic.as_deref()),
        Some("agents-router-test")
    );
    assert!(parsed.source("codex_desktop").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_cli").is_none());
    assert_eq!(parsed.routes[0].sources, vec!["codex_desktop".to_string()]);
    assert_eq!(parsed.routes[0].minimum_task_duration_minutes, None);
    assert!(parsed.routes[0].only_forward_from_project_paths.is_empty());
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
}

#[test]
fn writes_parseable_full_answer_detail_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_webhook_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Full,
        PromptDetail::Off,
        "https://example.com/hook",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(parsed.notification.answer_detail, AnswerDetail::Full);
}

#[test]
fn writes_parseable_on_prompt_detail_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_webhook_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::On,
        "https://example.com/hook",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(parsed.notification.prompt_detail, PromptDetail::On);
}

#[test]
fn writes_parseable_codex_cli_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_ntfy_config(
        AgentIntegrationId::CodexCli,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert!(parsed.source("codex_cli").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_desktop").is_none());
    assert_eq!(parsed.routes[0].sources, vec!["codex_cli".to_string()]);
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
}

#[test]
fn writes_parseable_claude_code_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_ntfy_config(
        AgentIntegrationId::ClaudeCode,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert!(parsed.source("claude_code").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_desktop").is_none());
    assert!(parsed.source("codex_cli").is_none());
    assert_eq!(parsed.routes[0].sources, vec!["claude_code".to_string()]);
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
}

#[test]
fn writes_parseable_agent_hook_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_ntfy_config(
        AgentIntegrationId::OpenCodeCli,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    let source = parsed
        .source("opencode_cli")
        .expect("OpenCode CLI source should be configured");
    assert_eq!(source.source_type, SourceType::AgentHook);
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_desktop").is_none());
    assert!(parsed.source("codex_cli").is_none());
    assert_eq!(parsed.routes[0].sources, vec!["opencode_cli".to_string()]);
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
}

#[test]
fn applies_agent_route_filters_without_filtering_setup_test_route() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let mut config = build_ntfy_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );

    apply_agent_route_filters(
        &mut config,
        AgentIntegrationId::CodexDesktop,
        Some(12),
        vec!["/Users/tester/projects/agents-router".to_string()],
    );
    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(parsed.routes.len(), 2);
    assert_eq!(parsed.routes[0].sources, vec!["codex_desktop".to_string()]);
    assert_eq!(parsed.routes[0].minimum_task_duration_minutes, Some(12));
    assert_eq!(
        parsed.routes[0].only_forward_from_project_paths,
        vec!["/Users/tester/projects/agents-router".to_string()]
    );
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
    assert_eq!(parsed.routes[1].minimum_task_duration_minutes, None);
    assert!(parsed.routes[1].only_forward_from_project_paths.is_empty());
}

#[test]
fn writes_parseable_feishu_lark_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_feishu_lark_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://open.larksuite.com/open-apis/bot/v2/hook/test",
        Some("secret".to_string()),
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("feishu_lark")
            .and_then(|provider| provider.url.as_deref()),
        Some("https://open.larksuite.com/open-apis/bot/v2/hook/test")
    );
    assert_eq!(
        parsed
            .provider("feishu_lark")
            .and_then(|provider| provider.secret.as_deref()),
        Some("secret")
    );
    assert!(parsed.source("codex_desktop").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_cli").is_none());
}

#[test]
fn writes_parseable_feishu_lark_app_bot_config_with_codex_reply_route() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_feishu_lark_app_bot_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    let provider = parsed
        .provider("feishu_lark")
        .expect("Feishu/Lark app provider should be configured");
    assert_eq!(provider.mode.as_deref(), Some("app_bot"));
    assert_eq!(provider.domain.as_deref(), Some("lark"));
    assert_eq!(provider.app_secret.as_deref(), Some("test-app-secret"));
    assert!(provider.app_secret_env.is_none());
    assert_eq!(provider.tenant_key.as_deref(), Some("2ca1d211f64f6438"));
    assert_eq!(
        provider.chat_id.as_deref(),
        Some("oc_5ce6d572455d361153b7xx51da133945")
    );
    assert_eq!(parsed.routes[0].sources, vec!["codex_desktop".to_string()]);
    assert!(parsed.routes[0].response_surface.enabled);
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
    assert!(!parsed.routes[1].response_surface.enabled);
}

#[test]
fn writes_parseable_feishu_lark_personal_agent_config_without_default_room() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_feishu_lark_personal_agent_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    let provider = parsed
        .provider("feishu_lark")
        .expect("Personal Agent provider should be configured");
    assert_eq!(provider.mode.as_deref(), Some("app_bot"));
    assert_eq!(provider.domain.as_deref(), Some("lark"));
    assert_eq!(provider.app_id.as_deref(), Some("cli_9f5343c580712544"));
    assert_eq!(provider.app_secret.as_deref(), Some("test-app-secret"));
    assert!(provider.tenant_key.is_none());
    assert!(provider.chat_id.is_none());
    assert_eq!(parsed.routes[0].sources, vec!["codex_desktop".to_string()]);
    assert!(parsed.routes[0].response_surface.enabled);
    assert_eq!(parsed.routes[1].sources, vec!["agents_router".to_string()]);
    assert!(!parsed.routes[1].response_surface.enabled);
}

#[test]
fn feishu_lark_app_bot_replies_are_not_enabled_for_non_codex_desktop_agents() {
    let config = build_feishu_lark_app_bot_config(
        AgentIntegrationId::ClaudeCode,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );

    assert_eq!(config.routes[0].sources, vec!["claude_code".to_string()]);
    assert!(!config.routes[0].response_surface.enabled);
    assert_eq!(config.routes[1].sources, vec!["agents_router".to_string()]);
    assert!(!config.routes[1].response_surface.enabled);
}

#[test]
fn writes_parseable_webhook_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_webhook_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://example.com/hook",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("webhook")
            .and_then(|provider| provider.url.as_deref()),
        Some("https://example.com/hook")
    );
    assert!(parsed.source("codex_desktop").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_cli").is_none());
}

#[test]
fn writes_parseable_pushover_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_pushover_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456789012345678901234567890",
        "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ",
        Some("iphone".to_string()),
        Some("pushover".to_string()),
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("pushover")
            .and_then(|provider| provider.app_token.as_deref()),
        Some("123456789012345678901234567890")
    );
    assert_eq!(
        parsed
            .provider("pushover")
            .and_then(|provider| provider.user_key.as_deref()),
        Some("ABCDEFGHIJABCDEFGHIJABCDEFGHIJ")
    );
    assert!(parsed.source("codex_desktop").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_cli").is_none());
}

#[test]
fn writes_parseable_slack_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_slack_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        &slack_test_url(),
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("slack")
            .and_then(|provider| provider.url.as_deref()),
        Some(slack_test_url().as_str())
    );
    assert!(parsed.source("codex_desktop").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_cli").is_none());
}

#[test]
fn writes_parseable_discord_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_discord_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://discord.com/api/webhooks/123456789012345678/token",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("discord")
            .and_then(|provider| provider.url.as_deref()),
        Some("https://discord.com/api/webhooks/123456789012345678/token")
    );
    assert!(parsed.source("codex_desktop").is_some());
    assert!(parsed.source("agents_router").is_some());
    assert!(parsed.source("codex_cli").is_none());
}

#[test]
fn writes_parseable_telegram_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_telegram_config(
        AgentIntegrationId::GithubCopilotCli,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456:test-token",
        "123456789",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("telegram")
            .and_then(|provider| provider.chat_id.as_deref()),
        Some("123456789")
    );
    assert_eq!(
        parsed
            .provider("telegram")
            .and_then(|provider| provider.bot_token.as_deref()),
        Some("123456:test-token")
    );
    let source = parsed
        .source("github_copilot_cli")
        .expect("GitHub Copilot CLI source should be configured");
    assert_eq!(source.source_type, SourceType::AgentHook);
}

#[test]
fn writes_parseable_whatsapp_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_whatsapp_config(
        AgentIntegrationId::GeminiCli,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "test-access-token",
        "123456789",
        "15551234567",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("whatsapp")
            .and_then(|provider| provider.phone_number_id.as_deref()),
        Some("123456789")
    );
    assert_eq!(
        parsed
            .provider("whatsapp")
            .and_then(|provider| provider.recipient_phone_number.as_deref()),
        Some("15551234567")
    );
    let source = parsed
        .source("gemini_cli")
        .expect("Gemini CLI source should be configured");
    assert_eq!(source.source_type, SourceType::AgentHook);
}

#[test]
fn writes_parseable_wechat_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_wechat_config(
        AgentIntegrationId::Aider,
        AnswerDetail::Preview,
        PromptDetail::Off,
        DEFAULT_WECHAT_BASE_URL,
        "test-token",
        "user@im.wechat",
        "test-context-token",
        Some("test-route".to_string()),
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("wechat")
            .and_then(|provider| provider.base_url.as_deref()),
        Some(DEFAULT_WECHAT_BASE_URL)
    );
    assert_eq!(
        parsed
            .provider("wechat")
            .and_then(|provider| provider.recipient_user_id.as_deref()),
        Some("user@im.wechat")
    );
    let source = parsed
        .source("aider")
        .expect("Aider source should be configured");
    assert_eq!(source.source_type, SourceType::AgentHook);
}

#[test]
fn writes_parseable_microsoft_teams_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_microsoft_teams_config(
        AgentIntegrationId::Aider,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://example.com/workflow?sig=secret",
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    assert_eq!(
        parsed
            .provider("microsoft_teams")
            .and_then(|provider| provider.url.as_deref()),
        Some("https://example.com/workflow?sig=secret")
    );
    let source = parsed
        .source("aider")
        .expect("Aider source should be configured");
    assert_eq!(source.source_type, SourceType::AgentHook);
}

#[test]
fn writes_parseable_email_smtp_config() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("config.toml");
    let config = build_email_smtp_config(
        AgentIntegrationId::CodexCli,
        AnswerDetail::Full,
        PromptDetail::On,
        "smtp.example.com",
        587,
        EmailSmtpSecurity::Starttls,
        Some("alerts@example.com".to_string()),
        Some("smtp-password".to_string()),
        "Agents Router <alerts@example.com>",
        vec!["Felix <felix@example.com>".to_string()],
        Some("reply@example.com".to_string()),
    );

    write_config(&path, &config).expect("config should be written");

    let parsed = read_valid_written_config(&path);
    let provider = parsed
        .provider("email_smtp")
        .expect("email_smtp provider should be configured");
    assert_eq!(provider.host.as_deref(), Some("smtp.example.com"));
    assert_eq!(provider.port, Some(587));
    assert_eq!(provider.security, Some(EmailSmtpSecurity::Starttls));
    assert_eq!(
        provider.to.as_ref().expect("recipients should exist"),
        &vec!["Felix <felix@example.com>".to_string()]
    );
    assert!(parsed.source("codex_cli").is_some());
}

#[test]
fn accepts_feishu_and_lark_webhook_urls() {
    assert_eq!(
        resolve_feishu_lark_webhook_url("https://open.feishu.cn/open-apis/bot/v2/hook/abc")
            .expect("Feishu webhook should be valid"),
        "https://open.feishu.cn/open-apis/bot/v2/hook/abc"
    );
    assert_eq!(
        resolve_feishu_lark_webhook_url("https://open.larksuite.com/open-apis/bot/v2/hook/abc")
            .expect("Lark webhook should be valid"),
        "https://open.larksuite.com/open-apis/bot/v2/hook/abc"
    );
}

#[test]
fn accepts_feishu_lark_app_bot_setup_fields() {
    assert_eq!(
        resolve_feishu_lark_app_domain("lark").expect("lark domain should be valid"),
        "lark"
    );
    assert_eq!(
        resolve_feishu_lark_app_id("cli_9f5343c580712544").expect("App ID should be valid"),
        "cli_9f5343c580712544"
    );
    assert_eq!(
        resolve_feishu_lark_app_secret("test-app-secret").expect("App Secret should be valid"),
        "test-app-secret"
    );
    assert_eq!(
        resolve_feishu_lark_tenant_key("2ca1d211f64f6438").expect("tenant_key should be valid"),
        "2ca1d211f64f6438"
    );
    assert_eq!(
        resolve_feishu_lark_chat_id("oc_5ce6d572455d361153b7xx51da133945")
            .expect("chat_id should be valid"),
        "oc_5ce6d572455d361153b7xx51da133945"
    );
}

#[test]
fn rejects_non_custom_bot_webhook_url() {
    let err = resolve_feishu_lark_webhook_url("https://example.com/hook")
        .expect_err("wrong URL should fail");

    assert!(err.to_string().contains("webhook URL must start"));
}

#[test]
fn accepts_https_and_local_webhook_urls() {
    assert_eq!(
        resolve_webhook_url("https://example.com/agents-router")
            .expect("HTTPS webhook should be valid"),
        "https://example.com/agents-router"
    );
    assert_eq!(
        resolve_webhook_url("http://127.0.0.1:8080/hook")
            .expect("local HTTP webhook should be valid"),
        "http://127.0.0.1:8080/hook"
    );
}

#[test]
fn accepts_official_slack_and_discord_webhook_urls() {
    assert_eq!(
        resolve_slack_webhook_url(&slack_test_url()).expect("Slack webhook should be valid"),
        slack_test_url()
    );
    assert_eq!(
        resolve_discord_webhook_url("https://discord.com/api/webhooks/123456789012345678/token")
            .expect("Discord webhook should be valid"),
        "https://discord.com/api/webhooks/123456789012345678/token"
    );
}

#[test]
fn accepts_telegram_whatsapp_wechat_and_microsoft_teams_inputs() {
    assert_eq!(
        resolve_telegram_bot_token("123456:test-token")
            .expect("Telegram bot token should be valid"),
        "123456:test-token"
    );
    assert_eq!(
        resolve_telegram_chat_id("@agents_router").expect("Telegram chat id should be valid"),
        "@agents_router"
    );
    assert_eq!(
        resolve_whatsapp_access_token("test-access-token")
            .expect("WhatsApp access token should be valid"),
        "test-access-token"
    );
    assert_eq!(
        resolve_whatsapp_phone_number_id("123456789")
            .expect("WhatsApp phone number id should be valid"),
        "123456789"
    );
    assert_eq!(
        resolve_whatsapp_recipient_phone_number("15551234567")
            .expect("WhatsApp recipient should be valid"),
        "15551234567"
    );
    assert_eq!(
        resolve_wechat_base_url(DEFAULT_WECHAT_BASE_URL).expect("WeChat base URL should be valid"),
        DEFAULT_WECHAT_BASE_URL
    );
    assert_eq!(
        resolve_wechat_token("test-token").expect("WeChat token should be valid"),
        "test-token"
    );
    assert_eq!(
        resolve_wechat_recipient_user_id("user@im.wechat")
            .expect("WeChat recipient user id should be valid"),
        "user@im.wechat"
    );
    assert_eq!(
        resolve_wechat_context_token("test-context-token")
            .expect("WeChat context token should be valid"),
        "test-context-token"
    );
    assert_eq!(
        resolve_wechat_route_tag("test-route").expect("WeChat route tag should be valid"),
        Some("test-route".to_string())
    );
    assert_eq!(
        resolve_microsoft_teams_webhook_url("https://example.com/workflow?sig=secret")
            .expect("Microsoft Teams webhook should be valid"),
        "https://example.com/workflow?sig=secret"
    );
}

#[test]
fn accepts_email_smtp_inputs() {
    assert_eq!(
        resolve_email_smtp_host("smtp.example.com").expect("SMTP host should be valid"),
        "smtp.example.com"
    );
    assert_eq!(
        resolve_email_smtp_port("587").expect("SMTP port should be valid"),
        587
    );
    assert_eq!(
        resolve_email_smtp_security("starttls").expect("SMTP security should be valid"),
        EmailSmtpSecurity::Starttls
    );
    assert_eq!(
        resolve_email_smtp_optional_username("alerts@example.com")
            .expect("SMTP username should be valid"),
        Some("alerts@example.com".to_string())
    );
    assert_eq!(
        resolve_email_smtp_password("smtp-password").expect("SMTP password should be valid"),
        "smtp-password"
    );
    assert_eq!(
        resolve_email_smtp_mailbox("Agents Router <alerts@example.com>", "from")
            .expect("SMTP mailbox should be valid"),
        "Agents Router <alerts@example.com>"
    );
    assert_eq!(
        resolve_email_smtp_recipients("Felix <felix@example.com>, team@example.com")
            .expect("SMTP recipients should be valid"),
        vec![
            "Felix <felix@example.com>".to_string(),
            "team@example.com".to_string()
        ]
    );
}

#[test]
fn accepts_valid_pushover_identifiers_and_device() {
    assert_eq!(
        resolve_pushover_app_token("123456789012345678901234567890")
            .expect("app token should be valid"),
        "123456789012345678901234567890"
    );
    assert_eq!(
        resolve_pushover_user_key("ABCDEFGHIJABCDEFGHIJABCDEFGHIJ")
            .expect("user key should be valid"),
        "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ"
    );
    assert_eq!(
        resolve_pushover_device("iphone,work_mac").expect("device should be valid"),
        Some("iphone,work_mac".to_string())
    );
}

#[test]
fn rejects_invalid_pushover_identifier_and_device() {
    let token_err =
        resolve_pushover_app_token("too-short").expect_err("short app token should fail");
    assert!(token_err.to_string().contains("30 letters or numbers"));

    let device_err =
        resolve_pushover_device("bad/device").expect_err("bad device name should fail");
    assert!(
        device_err
            .to_string()
            .contains("comma-separated device names")
    );
}

#[test]
fn rejects_insecure_remote_webhook_url() {
    let err = resolve_webhook_url("http://example.com/hook")
        .expect_err("remote HTTP webhook should fail");

    assert!(err.to_string().contains("must use HTTPS"));
}

#[test]
fn rejects_webhook_url_with_basic_auth() {
    let err = resolve_webhook_url("https://user:pass@example.com/hook")
        .expect_err("webhook URL credentials should fail");

    assert!(err.to_string().contains("must not include username"));
}

#[test]
fn extracts_ntfy_subscriptions_from_config() {
    let config = build_ntfy_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );

    let subscriptions = ntfy_subscriptions(&config);

    assert_eq!(
        subscriptions,
        vec![NtfySubscription {
            provider_id: "ntfy".to_string(),
            server: "https://ntfy.sh".to_string(),
            topic: "agents-router-test".to_string(),
        }]
    );
}

#[test]
fn extracts_feishu_lark_targets_without_printing_webhook_token() {
    let config = build_feishu_lark_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://open.larksuite.com/open-apis/bot/v2/hook/secret-token",
        Some("secret".to_string()),
    );

    let targets = feishu_lark_targets(&config);

    assert_eq!(
        targets,
        vec![FeishuLarkTarget {
            provider_id: "feishu_lark".to_string(),
            webhook_host: "open.larksuite.com".to_string(),
            signed: true,
        }]
    );
}

#[test]
fn extracts_webhook_targets_without_printing_full_url() {
    let config = build_webhook_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://example.com/secret-token",
    );

    let targets = webhook_targets(&config);

    assert_eq!(
        targets,
        vec![WebhookTarget {
            provider_id: "webhook".to_string(),
            webhook_host: "example.com".to_string(),
        }]
    );
}

#[test]
fn extracts_pushover_targets_without_printing_private_keys() {
    let config = build_pushover_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456789012345678901234567890",
        "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ",
        Some("iphone".to_string()),
        None,
    );

    let targets = pushover_targets(&config);

    assert_eq!(
        targets,
        vec![PushoverTarget {
            provider_id: "pushover".to_string(),
            device: Some("iphone".to_string()),
            sound: None,
        }]
    );
}

#[test]
fn extracts_slack_and_discord_targets_without_printing_webhook_tokens() {
    let slack_config = build_slack_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        &slack_test_url(),
    );
    let discord_config = build_discord_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://discord.com/api/webhooks/123456789012345678/token",
    );

    assert_eq!(
        slack_targets(&slack_config),
        vec![SlackTarget {
            provider_id: "slack".to_string(),
            webhook_host: "hooks.slack.com".to_string(),
        }]
    );
    assert_eq!(
        discord_targets(&discord_config),
        vec![DiscordTarget {
            provider_id: "discord".to_string(),
            webhook_host: "discord.com".to_string(),
        }]
    );
}

#[test]
fn extracts_new_provider_targets_without_printing_private_tokens() {
    let telegram_config = build_telegram_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456:test-token",
        "@agents_router",
    );
    let whatsapp_config = build_whatsapp_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "test-access-token",
        "123456789",
        "15551234567",
    );
    let wechat_config = build_wechat_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        DEFAULT_WECHAT_BASE_URL,
        "test-token",
        "user@im.wechat",
        "test-context-token",
        None,
    );
    let teams_config = build_microsoft_teams_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://example.com/workflow?sig=secret",
    );
    let email_config = build_email_smtp_config(
        AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "smtp.example.com",
        587,
        EmailSmtpSecurity::Starttls,
        Some("alerts@example.com".to_string()),
        Some("smtp-password".to_string()),
        "Agents Router <alerts@example.com>",
        vec!["Felix <felix@example.com>".to_string()],
        None,
    );

    assert_eq!(
        telegram_targets(&telegram_config),
        vec![TelegramTarget {
            provider_id: "telegram".to_string(),
            chat_id: "@agents_router".to_string(),
        }]
    );
    assert_eq!(
        whatsapp_targets(&whatsapp_config),
        vec![WhatsappTarget {
            provider_id: "whatsapp".to_string(),
            recipient_phone_number: "15551234567".to_string(),
        }]
    );
    assert_eq!(
        wechat_targets(&wechat_config),
        vec![WechatTarget {
            provider_id: "wechat".to_string(),
            base_url_host: "ilinkai.weixin.qq.com".to_string(),
            recipient_user_id: "user@im.wechat".to_string(),
        }]
    );
    assert_eq!(
        microsoft_teams_targets(&teams_config),
        vec![MicrosoftTeamsTarget {
            provider_id: "microsoft_teams".to_string(),
            webhook_host: "example.com".to_string(),
        }]
    );
    assert_eq!(
        email_smtp_targets(&email_config),
        vec![EmailSmtpTarget {
            provider_id: "email_smtp".to_string(),
            host: "smtp.example.com".to_string(),
            port: 587,
            from: "Agents Router <alerts@example.com>".to_string(),
            to: vec!["Felix <felix@example.com>".to_string()],
        }]
    );
}

#[test]
fn skipped_test_notification_message_confirms_running_service() {
    assert!(TEST_NOTIFICATION_SKIPPED_MESSAGE.contains("Service is running"));
    assert!(TEST_NOTIFICATION_SKIPPED_MESSAGE.contains("No test notification was sent"));
}

fn slack_test_url() -> String {
    format!(
        "https://hooks.slack.com/services/{}/{}/{}",
        "T00000000", "B00000000", "test-token"
    )
}
