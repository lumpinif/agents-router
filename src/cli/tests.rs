use std::path::{Path, PathBuf};

use super::*;

#[test]
fn uninstall_removes_script_installed_binary_paths() {
    assert!(should_remove_current_binary_for_install_method(
        Path::new("/Users/tester/.local/bin/agents-router"),
        Some("script")
    ));
}

#[test]
fn uninstall_keeps_unknown_binary_paths() {
    assert!(!should_remove_current_binary(Path::new(
        "/Users/tester/.local/bin/agents-router"
    )));
    assert!(!should_remove_current_binary(Path::new(
        "/opt/homebrew/bin/agents-router"
    )));
}

#[test]
fn uninstall_keeps_development_binary_paths_even_with_script_marker() {
    assert!(!should_remove_current_binary(Path::new(
        "/repo/target/debug/agents-router"
    )));
    assert!(!should_remove_current_binary_for_install_method(
        Path::new("/repo/target/release/agents-router"),
        Some("script")
    ));
}

#[cfg(not(windows))]
#[test]
fn stable_service_binary_path_uses_user_local_bin() {
    assert_eq!(
        stable_service_binary_path_for_home(Path::new("/Users/tester")),
        PathBuf::from("/Users/tester/.local/bin/agents-router")
    );
}

#[cfg(unix)]
#[test]
fn stable_service_binary_install_replaces_symlink_with_real_file() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let home = dir.path().join("home");
    let current_binary = dir
        .path()
        .join("repo")
        .join("target")
        .join("debug")
        .join("agents-router");
    std::fs::create_dir_all(current_binary.parent().unwrap())
        .expect("current binary parent should be created");
    std::fs::write(&current_binary, b"current-binary").expect("current binary should be written");

    let stable_binary = stable_service_binary_path_for_home(&home);
    std::fs::create_dir_all(stable_binary.parent().unwrap())
        .expect("stable binary parent should be created");
    std::os::unix::fs::symlink(&current_binary, &stable_binary)
        .expect("stable binary symlink should be created");

    let installed =
        install_stable_service_binary(&current_binary, &home).expect("binary should install");

    assert_eq!(installed, stable_binary);
    assert!(
        !std::fs::symlink_metadata(&stable_binary)
            .expect("stable binary metadata should be readable")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(&stable_binary).expect("stable binary should be readable"),
        b"current-binary"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn parses_first_valid_macos_codesign_identity() {
    let output = r#"
  1) CAB519C644B66B6093C5E261A10B0BC2CC7986D5 "Apple Development: tester@example.com (TEAMID1234)"
     1 valid identities found
"#;

    assert_eq!(
        parse_macos_codesign_identity(output),
        Some("Apple Development: tester@example.com (TEAMID1234)".to_string())
    );
}

#[cfg(target_os = "macos")]
#[test]
fn missing_macos_codesign_identity_is_not_selected() {
    assert_eq!(
        parse_macos_codesign_identity("     0 valid identities found\n"),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn detects_macho_binary_magic_without_trusting_file_extension() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let binary = dir.path().join("agents-router");
    let text = dir.path().join("agents-router.txt");

    std::fs::write(&binary, [0xcf, 0xfa, 0xed, 0xfe]).expect("binary fixture should be written");
    std::fs::write(&text, b"not a Mach-O binary").expect("text fixture should be written");

    assert!(looks_like_macho_binary(&binary));
    assert!(!looks_like_macho_binary(&text));
}

#[test]
fn uninstall_keeps_npm_managed_binary_paths() {
    assert!(!should_remove_current_binary_for_install_method(
        Path::new("/usr/local/lib/node_modules/agents-router-linux-x64-gnu/bin/agents-router"),
        Some("npm")
    ));
    assert!(!should_remove_current_binary_for_install_method(
        Path::new(
            r"C:\Users\tester\AppData\Roaming\npm\node_modules\agents-router-win32-x64-msvc\bin\agents-router.exe"
        ),
        Some("npm")
    ));
}

#[test]
fn setup_defaults_preserve_existing_config_answers() {
    let mut config = setup::build_feishu_lark_config(
        setup::AgentIntegrationId::ClaudeCode,
        AnswerDetail::Full,
        PromptDetail::On,
        "https://open.larksuite.com/open-apis/bot/v2/hook/secret-token",
        Some("signing-secret".to_string()),
    );
    setup::apply_agent_route_filters(
        &mut config,
        setup::AgentIntegrationId::ClaudeCode,
        Some(12),
        vec!["/Users/tester/projects/agents-router".to_string()],
    );
    config.cli.language = CliLanguage::SimplifiedChinese;

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(defaults.language, Some(CliLanguage::SimplifiedChinese));
    assert_eq!(
        defaults.source_integration,
        Some(setup::AgentIntegrationId::ClaudeCode)
    );
    assert_eq!(defaults.answer_detail, Some(AnswerDetail::Full));
    assert_eq!(defaults.prompt_detail, Some(PromptDetail::On));
    assert_eq!(defaults.minimum_task_duration_minutes, Some(12));
    assert_eq!(
        defaults.only_forward_from_project_paths,
        vec!["/Users/tester/projects/agents-router".to_string()]
    );
    assert_eq!(defaults.provider_type, Some(ProviderType::FeishuLark));
    assert_eq!(
        defaults.feishu_lark_webhook_url.as_deref(),
        Some("https://open.larksuite.com/open-apis/bot/v2/hook/secret-token")
    );
    assert_eq!(
        defaults.feishu_lark_secret.as_deref(),
        Some("signing-secret")
    );
}

#[test]
fn setup_defaults_preserve_duration_filter_for_codex_desktop() {
    let mut config = setup::build_ntfy_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );
    setup::apply_agent_route_filters(
        &mut config,
        setup::AgentIntegrationId::CodexDesktop,
        Some(12),
        Vec::new(),
    );

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(
        defaults.source_integration,
        Some(setup::AgentIntegrationId::CodexDesktop)
    );
    assert_eq!(defaults.minimum_task_duration_minutes, Some(12));
}

#[test]
fn setup_defaults_drop_duration_filter_for_agents_without_duration_support() {
    let mut config = setup::build_ntfy_config(
        setup::AgentIntegrationId::Aider,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );
    setup::apply_agent_route_filters(
        &mut config,
        setup::AgentIntegrationId::Aider,
        Some(12),
        Vec::new(),
    );

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(
        defaults.source_integration,
        Some(setup::AgentIntegrationId::Aider)
    );
    assert_eq!(defaults.minimum_task_duration_minutes, None);
}

#[test]
fn setup_defaults_require_source_id_and_type_to_match_catalog_integration() {
    let mut config = setup::build_ntfy_config(
        setup::AgentIntegrationId::CodexCli,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );
    config.sources[0].source_type = SourceType::AgentHook;

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(defaults.source_integration, None);
    assert_eq!(
        configured_agents(&config),
        vec!["Agent hook (codex_cli)".to_string()]
    );
}

#[test]
fn setup_defaults_keep_codex_desktop_selection_for_custom_source_id() {
    let mut config = setup::build_ntfy_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "agents-router-test",
    );
    config.sources[0].id = "my_desktop".to_string();
    config.routes[0].sources = vec!["my_desktop".to_string()];

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(
        defaults.source_integration,
        Some(setup::AgentIntegrationId::CodexDesktop)
    );
    assert_eq!(
        configured_agents(&config),
        vec!["Codex Desktop".to_string()]
    );
}

#[test]
fn catalog_ingest_hook_commands_parse_as_cli_commands() {
    for descriptor in agents_router::agent_integration_catalog::all_agent_integration_descriptors()
    {
        let Some(command) = descriptor.source_capability.hook_command else {
            continue;
        };
        if command.requires_emit_fields() {
            continue;
        }

        Cli::try_parse_from(command.command().split_whitespace())
            .expect("catalog ingest hook command should parse as a CLI command");
    }
}

#[test]
fn unfinished_recovery_notice_keeps_agent_submit_boundary_conservative() {
    assert_eq!(
        unfinished_recovery_notice(&InboundEventDedupStatus::ClaimedBeforeSubmit),
        Some((
            RESTART_BEFORE_SUBMIT_NOTICE_TEXT,
            InboundEventDedupStatus::FailedNotified
        ))
    );
    assert_eq!(
        unfinished_recovery_notice(&InboundEventDedupStatus::SubmittedPossible),
        Some((
            RESTART_AFTER_SUBMIT_NOTICE_TEXT,
            InboundEventDedupStatus::SubmittedUnknownNotified
        ))
    );
    assert_eq!(
        unfinished_recovery_notice(&InboundEventDedupStatus::Processed),
        None
    );
}

#[test]
fn catalog_emit_hook_commands_are_explicit_prefixes() {
    for descriptor in agents_router::agent_integration_catalog::all_agent_integration_descriptors()
    {
        let Some(command) = descriptor.source_capability.hook_command else {
            continue;
        };
        if !command.requires_emit_fields() {
            continue;
        }

        let err = Cli::try_parse_from(command.command().split_whitespace())
            .expect_err("emit command prefix should still require title and body");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }
}

#[test]
fn ingress_ready_requires_running_service_and_live_ping() {
    assert!(!ingress_ready_from_probe(false, false));
    assert!(!ingress_ready_from_probe(false, true));
    assert!(!ingress_ready_from_probe(true, false));
    assert!(ingress_ready_from_probe(true, true));
}

#[test]
fn emit_event_includes_supplied_duration() {
    let event = local_event_from_emit(
        "aider",
        "Aider".to_string(),
        "Aider finished a task.".to_string(),
        Some(420_000),
    );

    assert_eq!(event.source_id, "aider");
    assert_eq!(event.title, "Aider");
    assert_eq!(event.body, "Aider finished a task.");
    assert_eq!(
        event
            .lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.status),
        Some(SignalLifecycleStatus::Completed)
    );
    assert_eq!(
        event
            .lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.duration_ms),
        Some(420_000)
    );
}

#[test]
fn emit_event_without_duration_preserves_existing_shape() {
    let event = local_event_from_emit(
        "aider",
        "Aider".to_string(),
        "Aider finished a task.".to_string(),
        None,
    );

    assert_eq!(event.source_id, "aider");
    assert!(event.lifecycle.is_none());
}

#[test]
fn emit_rejects_negative_duration_argument() {
    let err = Cli::try_parse_from([
        "agents-router",
        "emit",
        "--source",
        "aider",
        "--title",
        "Aider",
        "--body",
        "Aider finished a task.",
        "--duration-ms",
        "-1",
    ])
    .expect_err("negative duration should be rejected by CLI parsing");

    assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn setup_defaults_do_not_inline_env_values() {
    let config = RawConfig::from_toml_str(
        r#"
schema_version = 1

[notification]
answer_detail = "preview"

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[sources]]
id = "agents_router"
type = "agents_router"

[[providers]]
id = "work_chat"
type = "feishu_lark"
url_env = "AGENTS_ROUTER_FEISHU_LARK_WEBHOOK_URL"
secret_env = "AGENTS_ROUTER_FEISHU_LARK_SECRET"

[[routes]]
sources = ["codex_desktop", "agents_router"]
providers = ["work_chat"]
"#,
    )
    .expect("test config should be valid");

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(defaults.provider_type, Some(ProviderType::FeishuLark));
    assert_eq!(defaults.feishu_lark_webhook_url, None);
    assert_eq!(defaults.feishu_lark_secret, None);
}

#[test]
fn setup_defaults_preserve_existing_pushover_config() {
    let config = setup::build_pushover_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456789012345678901234567890",
        "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ",
        Some("iphone".to_string()),
        Some("pushover".to_string()),
    );

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(defaults.provider_type, Some(ProviderType::Pushover));
    assert_eq!(
        defaults.pushover_app_token.as_deref(),
        Some("123456789012345678901234567890")
    );
    assert_eq!(
        defaults.pushover_user_key.as_deref(),
        Some("ABCDEFGHIJABCDEFGHIJABCDEFGHIJ")
    );
    assert_eq!(defaults.pushover_device.as_deref(), Some("iphone"));
    assert_eq!(defaults.pushover_sound.as_deref(), Some("pushover"));
}

#[test]
fn setup_defaults_preserve_existing_slack_and_discord_config() {
    let slack_config = setup::build_slack_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        &slack_test_url(),
    );
    let slack_defaults = SetupDefaults::from_config(&slack_config);

    assert_eq!(slack_defaults.provider_type, Some(ProviderType::Slack));
    assert_eq!(
        slack_defaults.slack_webhook_url.as_deref(),
        Some(slack_test_url().as_str())
    );

    let discord_config = setup::build_discord_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://discord.com/api/webhooks/123456789012345678/token",
    );
    let discord_defaults = SetupDefaults::from_config(&discord_config);

    assert_eq!(discord_defaults.provider_type, Some(ProviderType::Discord));
    assert_eq!(
        discord_defaults.discord_webhook_url.as_deref(),
        Some("https://discord.com/api/webhooks/123456789012345678/token")
    );
}

#[test]
fn prompt_detail_is_forced_off_for_length_limited_providers() {
    let i18n = I18n::default();
    assert_eq!(
        prompt_detail_for_provider(ProviderType::Ntfy, Some(PromptDetail::On), i18n)
            .expect("ntfy prompt detail should resolve"),
        PromptDetail::Off
    );
    assert_eq!(
        prompt_detail_for_provider(ProviderType::Pushover, Some(PromptDetail::On), i18n)
            .expect("Pushover prompt detail should resolve"),
        PromptDetail::Off
    );
    assert_eq!(
        prompt_detail_for_provider(ProviderType::Slack, Some(PromptDetail::On), i18n)
            .expect("Slack prompt detail should resolve"),
        PromptDetail::Off
    );
    assert_eq!(
        prompt_detail_for_provider(ProviderType::Discord, Some(PromptDetail::On), i18n)
            .expect("Discord prompt detail should resolve"),
        PromptDetail::Off
    );
    assert_eq!(
        prompt_detail_for_provider(ProviderType::Wechat, Some(PromptDetail::On), i18n)
            .expect("WeChat prompt detail should resolve"),
        PromptDetail::Off
    );
}

#[test]
fn answer_detail_is_forced_to_preview_for_length_limited_providers() {
    let i18n = I18n::default();
    assert_eq!(
        answer_detail_for_provider(ProviderType::Ntfy, Some(AnswerDetail::Full), i18n)
            .expect("ntfy answer detail should resolve"),
        AnswerDetail::Preview
    );
    assert_eq!(
        answer_detail_for_provider(ProviderType::Pushover, Some(AnswerDetail::Full), i18n)
            .expect("Pushover answer detail should resolve"),
        AnswerDetail::Preview
    );
    assert_eq!(
        answer_detail_for_provider(ProviderType::Slack, Some(AnswerDetail::Full), i18n)
            .expect("Slack answer detail should resolve"),
        AnswerDetail::Preview
    );
    assert_eq!(
        answer_detail_for_provider(ProviderType::Discord, Some(AnswerDetail::Full), i18n)
            .expect("Discord answer detail should resolve"),
        AnswerDetail::Preview
    );
    assert_eq!(
        answer_detail_for_provider(ProviderType::Wechat, Some(AnswerDetail::Full), i18n)
            .expect("WeChat answer detail should resolve"),
        AnswerDetail::Preview
    );
}

#[test]
fn setup_provider_limit_reasons_use_natural_copy_with_catalog_limits() {
    let telegram_constraints =
        agents_router::provider_catalog::provider_message_constraints(ProviderType::Telegram);
    assert_eq!(
        provider_constraint_reason(
            ProviderType::Telegram,
            telegram_constraints,
            CliLanguage::English
        ),
        "Telegram Bot API text messages are limited to 4096 characters"
    );

    let teams_constraints =
        agents_router::provider_catalog::provider_message_constraints(ProviderType::MicrosoftTeams);
    assert_eq!(
        provider_constraint_reason(
            ProviderType::MicrosoftTeams,
            teams_constraints,
            CliLanguage::English
        ),
        "Teams incoming webhook payloads are limited to 28 KB"
    );
}

#[test]
fn macos_and_windows_offer_codex_desktop_as_default_agent() {
    assert_eq!(
        default_agent_for_runtime_platform(RuntimePlatform::Macos),
        setup::AgentIntegrationId::CodexDesktop
    );
    assert_eq!(
        default_agent_for_runtime_platform(RuntimePlatform::Windows),
        setup::AgentIntegrationId::CodexDesktop
    );

    let macos_options = supported_agent_options_for_platform(RuntimePlatform::Macos);
    let windows_options = supported_agent_options_for_platform(RuntimePlatform::Windows);

    assert_eq!(
        macos_options.first().map(|(_, agent)| *agent),
        Some(setup::AgentIntegrationId::CodexDesktop)
    );
    assert_eq!(
        windows_options.first().map(|(_, agent)| *agent),
        Some(setup::AgentIntegrationId::CodexDesktop)
    );
}

#[test]
fn linux_starts_setup_at_codex_cli() {
    assert_eq!(
        default_agent_for_runtime_platform(RuntimePlatform::Linux),
        setup::AgentIntegrationId::CodexCli
    );

    let options = supported_agent_options_for_platform(RuntimePlatform::Linux);

    assert_eq!(
        options.first().map(|(_, agent)| *agent),
        Some(setup::AgentIntegrationId::CodexCli)
    );
    assert!(
        !options
            .iter()
            .any(|(_, agent)| *agent == setup::AgentIntegrationId::CodexDesktop)
    );
}

#[test]
fn setup_provider_summary_reports_feishu_lark_without_exposing_webhook_token() {
    let config = setup::build_feishu_lark_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Full,
        PromptDetail::On,
        "https://open.larksuite.com/open-apis/bot/v2/hook/secret-token",
        None,
    );

    let summary = single_setup_provider_summary(&config);

    assert_eq!(
        summary,
        SetupProviderSummary {
            provider_name: "Feishu/Lark custom bot",
            fields: vec![
                plain_summary_field("webhook", "open.larksuite.com".to_string()),
                SetupProviderSummaryField {
                    label: "signature verification",
                    value: "not configured".to_string(),
                    tone: SetupProviderSummaryTone::Warning,
                },
            ],
        }
    );
    assert!(!format!("{summary:?}").contains("secret-token"));
}

#[test]
fn setup_provider_summary_reports_signature_without_exposing_secret() {
    let config = setup::build_feishu_lark_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Full,
        PromptDetail::On,
        "https://open.larksuite.com/open-apis/bot/v2/hook/secret-token",
        Some("signing-secret".to_string()),
    );

    let summary = single_setup_provider_summary(&config);

    assert_eq!(
        summary.fields[1],
        SetupProviderSummaryField {
            label: "signature verification",
            value: "configured".to_string(),
            tone: SetupProviderSummaryTone::Success,
        }
    );
    let rendered = format!("{summary:?}");
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("signing-secret"));
}

#[test]
fn setup_provider_summary_reports_self_built_app_without_exposing_secret_value() {
    let config = setup::build_feishu_lark_app_bot_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );

    let summary = single_setup_provider_summary(&config);

    assert_eq!(
        summary,
        SetupProviderSummary {
            provider_name: "Feishu/Lark Self-built App",
            fields: vec![
                plain_summary_field("domain", "lark".to_string()),
                plain_summary_field("app id", "cli_9f5343c580712544".to_string()),
                SetupProviderSummaryField {
                    label: "app secret",
                    value: "configured".to_string(),
                    tone: SetupProviderSummaryTone::Success,
                },
                plain_summary_field(
                    "fixed room",
                    "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                ),
                plain_summary_field("tenant key", "2ca1d211f64f6438".to_string()),
            ],
        }
    );
    let rendered = format!("{summary:?}");
    assert!(!rendered.contains("test-app-secret"));
}

#[test]
fn setup_provider_summary_reports_personal_agent_without_fixed_room() {
    let config = setup::build_feishu_lark_personal_agent_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        Some("ou_operator"),
    );

    let summary = single_setup_provider_summary(&config);

    assert_eq!(summary.provider_name, "Feishu/Lark Personal Agent");
    assert!(summary.fields.iter().any(|field| {
        field.label == "routing" && field.value == "direct chat and project rooms"
    }));
    let rendered = format!("{summary:?}");
    assert!(!rendered.contains("test-app-secret"));
}

#[test]
fn personal_agent_setup_leaves_project_filtering_to_room_bindings() {
    let route_filters = SetupRouteFilters {
        minimum_task_duration_minutes: Some(5),
        only_forward_from_project_paths: vec![
            "/Users/tester/projects/old-project".to_string(),
            "/Users/tester/projects/another-old-project".to_string(),
        ],
    };

    let personal_agent_filters =
        feishu_lark_route_filters_for_mode(FeishuLarkSetupMode::PersonalAgentApp, &route_filters);
    let app_bot_filters = feishu_lark_route_filters_for_mode(
        FeishuLarkSetupMode::ExistingSelfBuiltApp,
        &route_filters,
    );

    assert_eq!(
        personal_agent_filters.minimum_task_duration_minutes,
        Some(5)
    );
    assert!(
        personal_agent_filters
            .only_forward_from_project_paths
            .is_empty()
    );
    assert_eq!(
        app_bot_filters.only_forward_from_project_paths,
        route_filters.only_forward_from_project_paths
    );
}

#[test]
fn setup_defaults_preserve_existing_app_bot_mode_and_fields() {
    let mut config = setup::build_feishu_lark_app_bot_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );
    config.cli.language = CliLanguage::English;

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(
        defaults.feishu_lark_mode,
        Some(FeishuLarkSetupMode::ExistingSelfBuiltApp)
    );
    assert_eq!(defaults.feishu_lark_app_domain.as_deref(), Some("lark"));
    assert_eq!(
        defaults.feishu_lark_app_id.as_deref(),
        Some("cli_9f5343c580712544")
    );
    assert_eq!(
        defaults.feishu_lark_app_secret.as_deref(),
        Some("test-app-secret")
    );
    assert_eq!(
        defaults.feishu_lark_tenant_key.as_deref(),
        Some("2ca1d211f64f6438")
    );
    assert_eq!(
        defaults.feishu_lark_chat_id.as_deref(),
        Some("oc_5ce6d572455d361153b7xx51da133945")
    );
}

#[test]
fn setup_defaults_requires_personal_agent_owner_before_reuse() {
    let config = RawConfig::from_toml_str(
        r#"
schema_version = 1

[notification]
answer_detail = "preview"

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[providers]]
id = "feishu_lark"
type = "feishu_lark"
mode = "app_bot"
domain = "lark"
app_id = "cli_9f5343c580712544"
app_secret = "test-app-secret"

[[routes]]
sources = ["codex_desktop"]
providers = ["feishu_lark"]
"#,
    )
    .expect("old Personal Agent config should parse");

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(
        defaults.feishu_lark_mode,
        Some(FeishuLarkSetupMode::PersonalAgentApp)
    );
    assert!(existing_personal_agent_credentials(&defaults).is_none());
}

#[test]
fn setup_defaults_accept_current_personal_agent_room_event_registration() {
    let config = setup::build_feishu_lark_personal_agent_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        Some("ou_operator"),
    );

    let defaults = SetupDefaults::from_config(&config);

    let provider = config
        .providers
        .iter()
        .find(|provider| provider.provider_type == ProviderType::FeishuLark)
        .expect("Personal Agent provider should exist");
    assert_eq!(
        provider.app_registration_source.as_deref(),
        Some(lark_personal_agent_channel::REGISTRATION_SOURCE)
    );
    let credentials = existing_personal_agent_credentials(&defaults)
        .expect("current Personal Agent config should be reusable");
    assert_eq!(credentials.operator_open_id, "ou_operator");
}

#[test]
fn setup_defaults_do_not_treat_app_secret_env_as_inline_app_secret() {
    let config = RawConfig::from_toml_str(
        r#"
schema_version = 1

[notification]
answer_detail = "preview"

[[sources]]
id = "codex_desktop"
type = "codex_desktop"

[[sources]]
id = "agents_router"
type = "agents_router"

[[providers]]
id = "feishu_lark"
type = "feishu_lark"
mode = "app_bot"
domain = "lark"
app_id = "cli_9f5343c580712544"
app_secret_env = "AGENTS_ROUTER_LARK_APP_SECRET"
tenant_key = "2ca1d211f64f6438"
chat_id = "oc_5ce6d572455d361153b7xx51da133945"

[[routes]]
sources = ["codex_desktop"]
providers = ["feishu_lark"]
"#,
    )
    .expect("test config should be valid");

    let defaults = SetupDefaults::from_config(&config);

    assert_eq!(
        defaults.feishu_lark_mode,
        Some(FeishuLarkSetupMode::ExistingSelfBuiltApp)
    );
    assert_eq!(defaults.feishu_lark_app_secret, None);
}

#[test]
fn feishu_lark_mode_options_are_localized() {
    assert_eq!(
        FeishuLarkSetupMode::default(),
        FeishuLarkSetupMode::PersonalAgentApp
    );

    let chinese_custom_bot = feishu_lark_mode_option_label(
        FeishuLarkSetupMode::CustomBotWebhook,
        setup::AgentIntegrationId::CodexDesktop,
        None,
        I18n::new(CliLanguage::SimplifiedChinese),
    );
    assert!(chinese_custom_bot.contains("备用"));
    assert!(!chinese_custom_bot.contains('\n'));
    assert!(!chinese_custom_bot.contains("Fastest setup"));
    assert!(
        feishu_lark_mode_option_description(
            FeishuLarkSetupMode::CustomBotWebhook,
            setup::AgentIntegrationId::CodexDesktop,
            I18n::new(CliLanguage::SimplifiedChinese),
        )
        .contains("单向发送通知到一个群")
    );

    let chinese_personal_agent = feishu_lark_mode_option_label(
        FeishuLarkSetupMode::PersonalAgentApp,
        setup::AgentIntegrationId::CodexDesktop,
        Some(FeishuLarkSetupMode::PersonalAgentApp),
        I18n::new(CliLanguage::SimplifiedChinese),
    );
    assert!(chinese_personal_agent.contains("实验性"));
    assert!(!chinese_personal_agent.contains('\n'));
    assert!(!chinese_personal_agent.contains("validate Feishu"));
    assert!(
        feishu_lark_mode_option_description(
            FeishuLarkSetupMode::PersonalAgentApp,
            setup::AgentIntegrationId::CodexDesktop,
            I18n::new(CliLanguage::SimplifiedChinese),
        )
        .contains("扫码创建 agent")
    );

    let english_personal_agent = feishu_lark_mode_option_label(
        FeishuLarkSetupMode::PersonalAgentApp,
        setup::AgentIntegrationId::CodexDesktop,
        None,
        I18n::new(CliLanguage::English),
    );
    assert!(english_personal_agent.contains("Experimental"));
    assert!(!english_personal_agent.contains('\n'));
    assert!(english_personal_agent.contains("Recommended"));
    assert!(
        feishu_lark_mode_option_description(
            FeishuLarkSetupMode::PersonalAgentApp,
            setup::AgentIntegrationId::CodexDesktop,
            I18n::new(CliLanguage::English),
        )
        .contains("Direct chat uses `/new` for Codex tasks and `/bind` for project room choices")
    );
}

#[test]
fn self_built_app_test_notification_body_explains_real_thread_reply_test() {
    let config = setup::build_feishu_lark_app_bot_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );

    let body = test_notification_body_for_config(&config);

    assert!(body.contains("confirms your self-built app can send messages"));
    assert!(body.contains("Lark thread replies are Experimental"));
    assert!(body.contains("Feishu uses the same app setup shape"));
    assert!(body.contains("Replies to this test message will not continue Codex"));
}

#[test]
fn self_built_app_test_notification_does_not_advertise_replies_for_unsupported_agents() {
    let config = setup::build_feishu_lark_app_bot_config(
        setup::AgentIntegrationId::ClaudeCode,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );

    let body = test_notification_body_for_config(&config);

    assert!(body.contains("confirms your self-built app can send messages"));
    assert!(body.contains("Replies are only available for Codex Desktop"));
    assert!(!body.contains("Lark thread replies are Experimental"));
}

#[test]
fn personal_agent_without_fixed_room_cannot_send_start_test_notification() {
    let config = setup::build_feishu_lark_personal_agent_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        Some("ou_operator"),
    );

    assert!(!can_send_test_notification(&config));
}

#[test]
fn self_built_app_with_fixed_room_can_send_start_test_notification() {
    let config = setup::build_feishu_lark_app_bot_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "lark",
        "cli_9f5343c580712544",
        "test-app-secret",
        "2ca1d211f64f6438",
        "oc_5ce6d572455d361153b7xx51da133945",
    );

    assert!(can_send_test_notification(&config));
}

#[test]
fn custom_bot_with_fixed_webhook_can_send_start_test_notification() {
    let config = setup::build_feishu_lark_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "https://open.larksuite.com/open-apis/bot/v2/hook/secret-token",
        None,
    );

    assert!(can_send_test_notification(&config));
}

#[test]
fn setup_provider_summary_reports_hidden_credentials_without_printing_values() {
    let telegram_config = setup::build_telegram_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456:test-token",
        "-1001234567890",
    );
    let telegram = single_setup_provider_summary(&telegram_config);

    assert_eq!(
        telegram.fields,
        vec![
            SetupProviderSummaryField {
                label: "bot token",
                value: "configured".to_string(),
                tone: SetupProviderSummaryTone::Success,
            },
            plain_summary_field("chat id", "-1001234567890".to_string()),
        ]
    );
    assert!(!format!("{telegram:?}").contains("123456:test-token"));

    let pushover_config = setup::build_pushover_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Preview,
        PromptDetail::Off,
        "123456789012345678901234567890",
        "ABCDEFGHIJABCDEFGHIJABCDEFGHIJ",
        Some("iphone".to_string()),
        Some("pushover".to_string()),
    );
    let pushover = single_setup_provider_summary(&pushover_config);

    assert_eq!(
        pushover.fields,
        vec![
            SetupProviderSummaryField {
                label: "application token",
                value: "configured".to_string(),
                tone: SetupProviderSummaryTone::Success,
            },
            SetupProviderSummaryField {
                label: "user or group key",
                value: "configured".to_string(),
                tone: SetupProviderSummaryTone::Success,
            },
            plain_summary_field("device", "iphone".to_string()),
            plain_summary_field("sound", "pushover".to_string()),
        ]
    );
    let rendered = format!("{pushover:?}");
    assert!(!rendered.contains("123456789012345678901234567890"));
    assert!(!rendered.contains("ABCDEFGHIJABCDEFGHIJABCDEFGHIJ"));
}

#[test]
fn setup_provider_summary_reports_email_without_printing_password() {
    let config = setup::build_email_smtp_config(
        setup::AgentIntegrationId::CodexDesktop,
        AnswerDetail::Full,
        PromptDetail::On,
        "smtp.example.com",
        587,
        EmailSmtpSecurity::Starttls,
        Some("alerts@example.com".to_string()),
        Some("smtp-password".to_string()),
        "Agents Router <alerts@example.com>",
        vec!["team@example.com".to_string()],
        Some("reply@example.com".to_string()),
    );

    let summary = single_setup_provider_summary(&config);

    assert_eq!(
        summary.fields,
        vec![
            plain_summary_field("server", "smtp.example.com:587".to_string()),
            plain_summary_field("security", "STARTTLS".to_string()),
            SetupProviderSummaryField {
                label: "authentication",
                value: "configured".to_string(),
                tone: SetupProviderSummaryTone::Success,
            },
            plain_summary_field("from", "Agents Router <alerts@example.com>".to_string()),
            plain_summary_field("to", "team@example.com".to_string()),
            plain_summary_field("reply-to", "reply@example.com".to_string()),
        ]
    );
    assert!(!format!("{summary:?}").contains("smtp-password"));
}

#[test]
fn safe_url_host_does_not_expose_webhook_token() {
    assert_eq!(
        safe_url_host("https://open.larksuite.com/open-apis/bot/v2/hook/secret-token"),
        "open.larksuite.com"
    );
}

fn single_setup_provider_summary(config: &RawConfig) -> SetupProviderSummary {
    let mut summaries = setup_provider_summaries(config);
    assert_eq!(summaries.len(), 1);
    summaries.remove(0)
}

fn slack_test_url() -> String {
    format!(
        "https://hooks.slack.com/services/{}/{}/{}",
        "T00000000", "B00000000", "test-token"
    )
}
