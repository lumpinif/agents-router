use super::*;

mod output;
mod prompts;

pub(super) use output::*;
pub(super) use prompts::*;

pub(super) struct LoadedConfig {
    pub(super) raw: RawConfig,
    pub(super) validated: ValidatedConfig,
    pub(super) guided_setup: Option<GuidedSetup>,
}

pub(super) enum ProviderSetupOutcome {
    Loaded(Box<LoadedConfig>),
    Exit(i32),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SetupRouteFilters {
    pub(super) minimum_task_duration_minutes: Option<u64>,
    pub(super) only_forward_from_project_paths: Vec<String>,
}

#[derive(Clone, Copy)]
pub(super) struct ProviderSetupContext<'a> {
    path: &'a Path,
    mode: ConfigWriteMode,
    agent: setup::AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    route_filters: &'a SetupRouteFilters,
    defaults: &'a SetupDefaults,
    i18n: I18n,
}

pub(super) enum GuidedSetup {
    Ntfy {
        agent: setup::AgentIntegrationId,
        topic: String,
    },
    FeishuLark {
        agent: setup::AgentIntegrationId,
        mode: FeishuLarkSetupMode,
    },
    Webhook {
        agent: setup::AgentIntegrationId,
    },
    Pushover {
        agent: setup::AgentIntegrationId,
    },
    Slack {
        agent: setup::AgentIntegrationId,
    },
    Discord {
        agent: setup::AgentIntegrationId,
    },
    Telegram {
        agent: setup::AgentIntegrationId,
    },
    Whatsapp {
        agent: setup::AgentIntegrationId,
    },
    Wechat {
        agent: setup::AgentIntegrationId,
    },
    MicrosoftTeams {
        agent: setup::AgentIntegrationId,
    },
    EmailSmtp {
        agent: setup::AgentIntegrationId,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FeishuLarkSetupMode {
    #[default]
    PersonalAgentApp,
    ExistingSelfBuiltApp,
    CustomBotWebhook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WechatSetupMethod {
    QrLogin,
    ExistingToken,
}

pub(super) struct WechatLogin {
    token: String,
    base_url: String,
    scanned_user_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NotificationPreference {
    EveryCompletedTask,
    FiveMinutesOrLonger,
    CustomMinimumDuration,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SetupDefaults {
    pub(super) language: Option<CliLanguage>,
    pub(super) source_integration: Option<setup::AgentIntegrationId>,
    pub(super) answer_detail: Option<AnswerDetail>,
    pub(super) prompt_detail: Option<PromptDetail>,
    pub(super) minimum_task_duration_minutes: Option<u64>,
    pub(super) only_forward_from_project_paths: Vec<String>,
    pub(super) provider_type: Option<ProviderType>,
    pub(super) ntfy_topic: Option<String>,
    pub(super) feishu_lark_mode: Option<FeishuLarkSetupMode>,
    pub(super) feishu_lark_webhook_url: Option<String>,
    pub(super) feishu_lark_secret: Option<String>,
    pub(super) feishu_lark_app_domain: Option<String>,
    pub(super) feishu_lark_app_id: Option<String>,
    pub(super) feishu_lark_app_secret: Option<String>,
    pub(super) feishu_lark_tenant_key: Option<String>,
    pub(super) feishu_lark_chat_id: Option<String>,
    pub(super) webhook_url: Option<String>,
    pub(super) pushover_app_token: Option<String>,
    pub(super) pushover_user_key: Option<String>,
    pub(super) pushover_device: Option<String>,
    pub(super) pushover_sound: Option<String>,
    pub(super) slack_webhook_url: Option<String>,
    pub(super) discord_webhook_url: Option<String>,
    pub(super) telegram_bot_token: Option<String>,
    pub(super) telegram_chat_id: Option<String>,
    pub(super) whatsapp_access_token: Option<String>,
    pub(super) whatsapp_phone_number_id: Option<String>,
    pub(super) whatsapp_recipient_phone_number: Option<String>,
    pub(super) wechat_base_url: Option<String>,
    pub(super) wechat_token: Option<String>,
    pub(super) wechat_recipient_user_id: Option<String>,
    pub(super) wechat_context_token: Option<String>,
    pub(super) wechat_route_tag: Option<String>,
    pub(super) microsoft_teams_webhook_url: Option<String>,
    pub(super) email_smtp_host: Option<String>,
    pub(super) email_smtp_port: Option<u16>,
    pub(super) email_smtp_security: Option<EmailSmtpSecurity>,
    pub(super) email_smtp_username: Option<String>,
    pub(super) email_smtp_password: Option<String>,
    pub(super) email_smtp_from: Option<String>,
    pub(super) email_smtp_to: Option<Vec<String>>,
    pub(super) email_smtp_reply_to: Option<String>,
}

impl SetupDefaults {
    pub(super) fn from_config(config: &RawConfig) -> Self {
        let agent_route = first_agent_route(config);
        let source_integration = first_configured_agent(config);
        let supports_duration_filter =
            source_integration.is_some_and(setup::AgentIntegrationId::supports_duration_filter);
        Self {
            language: Some(config.cli.language),
            source_integration,
            answer_detail: Some(config.notification.answer_detail),
            prompt_detail: Some(config.notification.prompt_detail),
            minimum_task_duration_minutes: if supports_duration_filter {
                agent_route.and_then(|route| route.minimum_task_duration_minutes)
            } else {
                None
            },
            only_forward_from_project_paths: agent_route
                .map(|route| route.only_forward_from_project_paths.clone())
                .unwrap_or_default(),
            provider_type: first_configured_provider(config),
            ntfy_topic: first_provider_of_type(config, ProviderType::Ntfy)
                .and_then(|provider| provider.topic.clone()),
            feishu_lark_mode: first_provider_of_type(config, ProviderType::FeishuLark)
                .map(feishu_lark_setup_mode_from_raw_provider),
            feishu_lark_webhook_url: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(configured_provider_url),
            feishu_lark_secret: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(configured_provider_secret),
            feishu_lark_app_domain: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(|provider| provider.domain.clone()),
            feishu_lark_app_id: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(|provider| provider.app_id.clone()),
            feishu_lark_app_secret: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(|provider| provider.app_secret.clone()),
            feishu_lark_tenant_key: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(|provider| provider.tenant_key.clone()),
            feishu_lark_chat_id: first_provider_of_type(config, ProviderType::FeishuLark)
                .and_then(|provider| provider.chat_id.clone()),
            webhook_url: first_provider_of_type(config, ProviderType::Webhook)
                .and_then(configured_provider_url),
            pushover_app_token: first_provider_of_type(config, ProviderType::Pushover)
                .and_then(|provider| provider.app_token.clone()),
            pushover_user_key: first_provider_of_type(config, ProviderType::Pushover)
                .and_then(|provider| provider.user_key.clone()),
            pushover_device: first_provider_of_type(config, ProviderType::Pushover)
                .and_then(|provider| provider.device.clone()),
            pushover_sound: first_provider_of_type(config, ProviderType::Pushover)
                .and_then(|provider| provider.sound.clone()),
            slack_webhook_url: first_provider_of_type(config, ProviderType::Slack)
                .and_then(configured_provider_url),
            discord_webhook_url: first_provider_of_type(config, ProviderType::Discord)
                .and_then(configured_provider_url),
            telegram_bot_token: first_provider_of_type(config, ProviderType::Telegram)
                .and_then(|provider| provider.bot_token.clone()),
            telegram_chat_id: first_provider_of_type(config, ProviderType::Telegram)
                .and_then(|provider| provider.chat_id.clone()),
            whatsapp_access_token: first_provider_of_type(config, ProviderType::Whatsapp)
                .and_then(|provider| provider.access_token.clone()),
            whatsapp_phone_number_id: first_provider_of_type(config, ProviderType::Whatsapp)
                .and_then(|provider| provider.phone_number_id.clone()),
            whatsapp_recipient_phone_number: first_provider_of_type(config, ProviderType::Whatsapp)
                .and_then(|provider| provider.recipient_phone_number.clone()),
            wechat_base_url: first_provider_of_type(config, ProviderType::Wechat)
                .and_then(|provider| provider.base_url.clone()),
            wechat_token: first_provider_of_type(config, ProviderType::Wechat)
                .and_then(|provider| provider.token.clone()),
            wechat_recipient_user_id: first_provider_of_type(config, ProviderType::Wechat)
                .and_then(|provider| provider.recipient_user_id.clone()),
            wechat_context_token: first_provider_of_type(config, ProviderType::Wechat)
                .and_then(|provider| provider.context_token.clone()),
            wechat_route_tag: first_provider_of_type(config, ProviderType::Wechat)
                .and_then(|provider| provider.route_tag.clone()),
            microsoft_teams_webhook_url: first_provider_of_type(
                config,
                ProviderType::MicrosoftTeams,
            )
            .and_then(configured_provider_url),
            email_smtp_host: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.host.clone()),
            email_smtp_port: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.port),
            email_smtp_security: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.security),
            email_smtp_username: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.username.clone()),
            email_smtp_password: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.password.clone()),
            email_smtp_from: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.from.clone()),
            email_smtp_to: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.to.clone()),
            email_smtp_reply_to: first_provider_of_type(config, ProviderType::EmailSmtp)
                .and_then(|provider| provider.reply_to.clone()),
        }
    }

    fn from_path(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let loaded = ParsedConfig::from_path(path)?;
        Ok(Self::from_config(&loaded.raw))
    }
}

fn feishu_lark_setup_mode_from_raw_provider(provider: &RawProviderConfig) -> FeishuLarkSetupMode {
    match provider.mode.as_deref() {
        Some("app_bot") if provider.chat_id.is_some() => FeishuLarkSetupMode::ExistingSelfBuiltApp,
        Some("app_bot") => FeishuLarkSetupMode::PersonalAgentApp,
        _ => FeishuLarkSetupMode::CustomBotWebhook,
    }
}

fn first_agent_route(config: &RawConfig) -> Option<&RouteConfig> {
    let agent = first_configured_agent(config)?;
    let source_id = agent.source_id();
    config
        .routes
        .iter()
        .find(|route| route.sources.iter().any(|source| source == source_id))
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ConfigWriteMode {
    Created,
    Updated,
}

impl ConfigWriteMode {
    fn past_tense(self, i18n: I18n) -> &'static str {
        match self {
            Self::Created => i18n.text(Text::ConfigCreated),
            Self::Updated => i18n.text(Text::ConfigUpdated),
        }
    }
}

pub(super) async fn load_config(path: &Path, allow_setup: bool) -> anyhow::Result<LoadedConfig> {
    match ParsedConfig::from_path(path) {
        Ok(config) => Ok(LoadedConfig {
            raw: config.raw,
            validated: config.validated,
            guided_setup: None,
        }),
        Err(ConfigError::NotFound { path }) if allow_setup && is_interactive_terminal() => {
            run_first_start_setup(Path::new(&path)).await
        }
        Err(ConfigError::NotFound { path }) => {
            anyhow::bail!("{}", setup::missing_config_message(&path))
        }
        Err(error) => Err(error).context("config load failed"),
    }
}

fn loaded_config(
    config: RawConfig,
    guided_setup: Option<GuidedSetup>,
) -> anyhow::Result<LoadedConfig> {
    let validated = config.validate()?;
    Ok(LoadedConfig {
        raw: config,
        validated,
        guided_setup,
    })
}

pub(super) fn is_interactive_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub(super) fn prompt_theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

pub(super) fn prompt_text(
    prompt: impl Into<String>,
    read_context: impl Into<String>,
) -> anyhow::Result<String> {
    let theme = prompt_theme();
    let read_context = read_context.into();
    Input::<String>::with_theme(&theme)
        .with_prompt(prompt.into())
        .allow_empty(true)
        .interact()
        .with_context(|| read_context)
}

pub(super) fn prompt_private_text(
    prompt: impl Into<String>,
    initial_text: Option<&str>,
    read_context: impl Into<String>,
) -> anyhow::Result<String> {
    let theme = prompt_theme();
    let read_context = read_context.into();
    let input = Input::<String>::with_theme(&theme)
        .with_prompt(prompt.into())
        .allow_empty(true)
        .report(false);
    let input = if let Some(initial_text) = initial_text {
        input.with_initial_text(initial_text)
    } else {
        input
    };

    input.interact().with_context(|| read_context)
}

pub(super) fn prompt_secret(
    prompt: impl Into<String>,
    read_context: impl Into<String>,
) -> anyhow::Result<String> {
    let theme = prompt_theme();
    let read_context = read_context.into();
    Password::with_theme(&theme)
        .with_prompt(prompt.into())
        .allow_empty_password(true)
        .report(false)
        .interact()
        .with_context(|| read_context)
}

pub(super) fn prompt_confirm(prompt: &str, default: bool) -> anyhow::Result<bool> {
    let theme = prompt_theme();
    Confirm::with_theme(&theme)
        .with_prompt(prompt)
        .default(default)
        .interact()
        .context("failed to read confirmation")
}

pub(super) fn prompt_for_language(default: Option<CliLanguage>) -> anyhow::Result<CliLanguage> {
    let effective_default = language_default(default)?;
    let options = [CliLanguage::English, CliLanguage::SimplifiedChinese];
    let default_index = options
        .iter()
        .position(|language| *language == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|language| language.display_name())
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(I18n::new(effective_default).text(Text::LanguagePrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read language choice")?;

    Ok(options[selection])
}

pub(super) fn language_default(default: Option<CliLanguage>) -> anyhow::Result<CliLanguage> {
    if let Ok(value) = std::env::var("AGENTS_ROUTER_LANGUAGE") {
        let Some(language) = CliLanguage::parse(&value) else {
            anyhow::bail!(
                "AGENTS_ROUTER_LANGUAGE must be `en` or `zh-CN`; got `{}`",
                value
            );
        };
        return Ok(language);
    }

    Ok(default.unwrap_or_default())
}

pub(super) fn localized(
    i18n: I18n,
    english: &'static str,
    simplified_chinese: &'static str,
) -> &'static str {
    match i18n.language() {
        CliLanguage::English => english,
        CliLanguage::SimplifiedChinese => simplified_chinese,
    }
}

pub(super) fn localized_string(i18n: I18n, english: String, simplified_chinese: String) -> String {
    match i18n.language() {
        CliLanguage::English => english,
        CliLanguage::SimplifiedChinese => simplified_chinese,
    }
}

pub(super) fn write_setup_config(
    path: &Path,
    config: &mut RawConfig,
    language: CliLanguage,
) -> anyhow::Result<()> {
    config.cli = CliConfig::new(language);
    setup::write_config(path, config)
}

fn write_setup_config_with_route_filters(
    path: &Path,
    config: &mut RawConfig,
    language: CliLanguage,
    agent: setup::AgentIntegrationId,
    route_filters: &SetupRouteFilters,
) -> anyhow::Result<()> {
    setup::apply_agent_route_filters(
        config,
        agent,
        route_filters.minimum_task_duration_minutes,
        route_filters.only_forward_from_project_paths.clone(),
    );
    write_setup_config(path, config, language)
}

pub(super) async fn run_first_start_setup(path: &Path) -> anyhow::Result<LoadedConfig> {
    let language = prompt_for_language(None)?;
    let i18n = I18n::new(language);

    match language {
        CliLanguage::English => {
            println!("No Agents Router config found at `{}`.", path.display())
        }
        CliLanguage::SimplifiedChinese => {
            println!("没有找到 Agents Router config：`{}`。", path.display())
        }
    }
    println!();
    println!("{}", i18n.text(Text::FirstStartStartingSetup));
    println!("{}", i18n.text(Text::FirstStartSetupScope));
    println!();

    run_guided_provider_setup(
        path,
        ConfigWriteMode::Created,
        SetupDefaults {
            language: Some(language),
            ..SetupDefaults::default()
        },
        i18n,
    )
    .await
}

pub(super) async fn run_provider_setup(path: &Path) -> anyhow::Result<ProviderSetupOutcome> {
    if !is_interactive_terminal() {
        anyhow::bail!("{}", I18n::default().text(Text::SetupInteractiveRequired));
    }

    let mode = if path.exists() {
        ConfigWriteMode::Updated
    } else {
        ConfigWriteMode::Created
    };
    let mut defaults = SetupDefaults::from_path(path)?;
    let language = prompt_for_language(defaults.language)?;
    defaults.language = Some(language);
    let i18n = I18n::new(language);

    match super::update_flow::maybe_update_before_setup(path, i18n).await? {
        super::update_flow::SetupUpdateOutcome::Continue => {}
        super::update_flow::SetupUpdateOutcome::Exit(code) => {
            return Ok(ProviderSetupOutcome::Exit(code));
        }
    }

    print_heading(i18n.text(Text::SetupTitle));
    match language {
        CliLanguage::English => println!(
            "This replaces the notification, agent, provider, and route sections in `{}`.",
            path.display()
        ),
        CliLanguage::SimplifiedChinese => println!(
            "这会替换 `{}` 里的 notification、agent、provider 和 route 配置。",
            path.display()
        ),
    }
    println!("{}", i18n.text(Text::SetupDoesNotChangeAgentSettings));
    println!();

    run_guided_provider_setup(path, mode, defaults, i18n)
        .await
        .map(|loaded| ProviderSetupOutcome::Loaded(Box::new(loaded)))
}

pub(super) async fn run_guided_provider_setup(
    path: &Path,
    mode: ConfigWriteMode,
    defaults: SetupDefaults,
    i18n: I18n,
) -> anyhow::Result<LoadedConfig> {
    let agent = prompt_for_agent(defaults.source_integration, i18n)?;
    let provider = prompt_for_initial_provider(defaults.provider_type, i18n)?;
    let answer_detail = answer_detail_for_provider(provider, defaults.answer_detail, i18n)?;
    let prompt_detail = prompt_detail_for_provider(provider, defaults.prompt_detail, i18n)?;
    let minimum_task_duration_minutes = if agent.supports_duration_filter() {
        prompt_for_notification_preference(defaults.minimum_task_duration_minutes, i18n)?
    } else {
        None
    };
    let route_filters = SetupRouteFilters {
        minimum_task_duration_minutes,
        only_forward_from_project_paths: defaults.only_forward_from_project_paths.clone(),
    };
    let context = ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters: &route_filters,
        defaults: &defaults,
        i18n,
    };

    match provider {
        ProviderType::Ntfy => run_ntfy_setup(context),
        ProviderType::FeishuLark => run_feishu_lark_setup(context).await,
        ProviderType::Webhook => run_webhook_setup(context),
        ProviderType::Pushover => run_pushover_setup(context),
        ProviderType::Slack => run_slack_setup(context),
        ProviderType::Discord => run_discord_setup(context),
        ProviderType::Telegram => run_telegram_setup(context),
        ProviderType::Whatsapp => run_whatsapp_setup(context),
        ProviderType::Wechat => run_wechat_setup(context).await,
        ProviderType::MicrosoftTeams => run_microsoft_teams_setup(context),
        ProviderType::EmailSmtp => run_email_smtp_setup(context),
    }
}

pub(super) fn answer_detail_for_provider(
    provider: ProviderType,
    default: Option<AnswerDetail>,
    i18n: I18n,
) -> anyhow::Result<AnswerDetail> {
    match answer_detail_support(provider) {
        NotificationDetailSupport::Configurable => prompt_for_answer_detail(default, i18n),
        NotificationDetailSupport::FixedPreview { constraints } => {
            println!();
            println!(
                "{}",
                fixed_answer_detail_message(
                    i18n,
                    localized_setup_provider_display_name(provider, i18n),
                    &provider_constraint_reason(provider, constraints, CliLanguage::English),
                    &provider_constraint_reason(
                        provider,
                        constraints,
                        CliLanguage::SimplifiedChinese,
                    ),
                )
            );
            Ok(AnswerDetail::Preview)
        }
        NotificationDetailSupport::FixedOff { .. } => {
            unreachable!("answer detail policy never returns FixedOff")
        }
    }
}

pub(super) fn prompt_detail_for_provider(
    provider: ProviderType,
    default: Option<PromptDetail>,
    i18n: I18n,
) -> anyhow::Result<PromptDetail> {
    match prompt_detail_support(provider) {
        NotificationDetailSupport::Configurable => prompt_for_prompt_detail(default, i18n),
        NotificationDetailSupport::FixedOff { constraints } => {
            println!();
            println!(
                "{}",
                fixed_prompt_detail_message(
                    i18n,
                    localized_setup_provider_display_name(provider, i18n),
                    &provider_constraint_reason(provider, constraints, CliLanguage::English),
                    &provider_constraint_reason(
                        provider,
                        constraints,
                        CliLanguage::SimplifiedChinese,
                    ),
                )
            );
            Ok(PromptDetail::Off)
        }
        NotificationDetailSupport::FixedPreview { .. } => {
            unreachable!("prompt detail policy never returns FixedPreview")
        }
    }
}

fn localized_setup_provider_display_name(provider: ProviderType, i18n: I18n) -> &'static str {
    if provider == ProviderType::Wechat && i18n.language() == CliLanguage::SimplifiedChinese {
        "微信"
    } else {
        provider_descriptor(provider).display_name
    }
}

pub(super) fn provider_constraint_reason(
    provider: ProviderType,
    constraints: &[ProviderMessageConstraint],
    language: CliLanguage,
) -> String {
    match provider {
        ProviderType::Ntfy => match language {
            CliLanguage::English => format!(
                "ntfy's default server message body limit is {}",
                constraint_limit_text(constraints, MessageSurface::MessageBody, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "ntfy 默认 server 的消息正文限制是 {}",
                constraint_limit_text(constraints, MessageSurface::MessageBody, language)
            ),
        },
        ProviderType::Pushover => match language {
            CliLanguage::English => format!(
                "Pushover messages are limited to {}",
                constraint_limit_text(constraints, MessageSurface::MessageBody, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Pushover 消息最多 {}",
                constraint_limit_text(constraints, MessageSurface::MessageBody, language)
            ),
        },
        ProviderType::Slack => match language {
            CliLanguage::English => format!(
                "Agents Router uses a {} delivery guard for Slack text",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Agents Router 对 Slack 文本使用 {} 发送保护线",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
        },
        ProviderType::Discord => match language {
            CliLanguage::English => format!(
                "Discord webhook content is limited to {}",
                constraint_limit_text(constraints, MessageSurface::WebhookContent, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Discord webhook 内容最多 {}",
                constraint_limit_text(constraints, MessageSurface::WebhookContent, language)
            ),
        },
        ProviderType::Telegram => match language {
            CliLanguage::English => format!(
                "Telegram Bot API text messages are limited to {}",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Telegram Bot API 文本消息最多 {}",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
        },
        ProviderType::Whatsapp => match language {
            CliLanguage::English => format!(
                "Agents Router uses a {} delivery guard for WhatsApp text bodies",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Agents Router 对 WhatsApp 文本正文使用 {} 发送保护线",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
        },
        ProviderType::Wechat => match language {
            CliLanguage::English => format!(
                "Agents Router uses a {} delivery guard for WeChat iLink text messages",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Agents Router 对微信 iLink 文本消息使用 {} 发送保护线",
                constraint_limit_text(constraints, MessageSurface::TextBody, language)
            ),
        },
        ProviderType::MicrosoftTeams => match language {
            CliLanguage::English => format!(
                "Teams incoming webhook payloads are limited to {}",
                constraint_limit_text(constraints, MessageSurface::Payload, language)
            ),
            CliLanguage::SimplifiedChinese => format!(
                "Teams incoming webhook payload 限制是 {}",
                constraint_limit_text(constraints, MessageSurface::Payload, language)
            ),
        },
        ProviderType::FeishuLark | ProviderType::Webhook | ProviderType::EmailSmtp => {
            unreachable!("unconstrained providers should not produce fixed detail reasons")
        }
    }
}

fn constraint_limit_text(
    constraints: &[ProviderMessageConstraint],
    surface: MessageSurface,
    language: CliLanguage,
) -> String {
    let constraint = constraints
        .iter()
        .find(|constraint| constraint.surface == surface)
        .expect("provider message constraint must be cataloged for setup reason");
    message_limit_text(*constraint, language)
}

fn message_limit_text(constraint: ProviderMessageConstraint, language: CliLanguage) -> String {
    let Some(limit) = constraint.limit else {
        return match language {
            CliLanguage::English => "the provider limit".to_string(),
            CliLanguage::SimplifiedChinese => "平台限制".to_string(),
        };
    };

    if constraint.unit == MessageLimitUnit::Bytes && limit % 1024 == 0 {
        return format!("{} KB", limit / 1024);
    }

    match language {
        CliLanguage::English => {
            format!(
                "{limit} {}",
                message_limit_unit_label(constraint.unit, language)
            )
        }
        CliLanguage::SimplifiedChinese => {
            format!(
                "{limit} {}",
                message_limit_unit_label(constraint.unit, language)
            )
        }
    }
}

fn message_limit_unit_label(unit: MessageLimitUnit, language: CliLanguage) -> &'static str {
    match language {
        CliLanguage::English => match unit {
            MessageLimitUnit::Characters => "characters",
            MessageLimitUnit::Bytes => "bytes",
        },
        CliLanguage::SimplifiedChinese => match unit {
            MessageLimitUnit::Characters => "字符",
            MessageLimitUnit::Bytes => "字节",
        },
    }
}

pub(super) fn fixed_answer_detail_message(
    i18n: I18n,
    provider: &str,
    english_reason: &str,
    chinese_reason: &str,
) -> String {
    match i18n.language() {
        CliLanguage::English => format!(
            "Answer detail is fixed to Preview for {provider} because {english_reason}. Agents Router will keep {provider} notifications short enough for reliable delivery."
        ),
        CliLanguage::SimplifiedChinese => format!(
            "{provider} 的回答内容固定为 Preview，因为{chinese_reason}。Agents Router 会保持通知短小，确保可靠送达。"
        ),
    }
}

pub(super) fn fixed_prompt_detail_message(
    i18n: I18n,
    provider: &str,
    english_reason: &str,
    chinese_reason: &str,
) -> String {
    match i18n.language() {
        CliLanguage::English => format!(
            "Prompt detail is disabled for {provider} because {english_reason}. Agents Router will keep prompts out of {provider} notifications for reliable delivery."
        ),
        CliLanguage::SimplifiedChinese => format!(
            "{provider} 已关闭 prompt 内容，因为{chinese_reason}。Agents Router 不会把 prompt 放进 {provider} 通知里。"
        ),
    }
}

pub(super) fn run_ntfy_setup(context: ProviderSetupContext<'_>) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    let generated_topic = setup::generated_ntfy_topic();

    println!();
    println!("{}", i18n.text(Text::NtfyIntro1));
    println!("{}", i18n.text(Text::NtfyIntro2));
    println!();

    let topic = prompt_for_ntfy_topic(&generated_topic, defaults.ntfy_topic.as_deref(), i18n)?;
    let mut config = setup::build_ntfy_config(agent, answer_detail, prompt_detail, &topic);
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Ntfy { agent, topic }))
}

pub(super) async fn run_feishu_lark_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    let feishu_lark_mode = prompt_for_feishu_lark_mode(agent, defaults.feishu_lark_mode, i18n)?;
    let route_filters = feishu_lark_route_filters_for_mode(feishu_lark_mode, route_filters);
    let mut config = match feishu_lark_mode {
        FeishuLarkSetupMode::PersonalAgentApp => {
            if let Some(existing) = existing_personal_agent_credentials(defaults) {
                print_existing_personal_agent_reuse_notice(i18n);
                setup::build_feishu_lark_personal_agent_config(
                    agent,
                    answer_detail,
                    prompt_detail,
                    existing.domain,
                    existing.app_id,
                    existing.app_secret,
                )
            } else {
                print_feishu_lark_personal_agent_setup_intro(i18n);
                let registration = run_lark_personal_agent_qr_registration(i18n).await?;
                setup::build_feishu_lark_personal_agent_config(
                    agent,
                    answer_detail,
                    prompt_detail,
                    registration.domain.as_str(),
                    &registration.app_id,
                    &registration.app_secret,
                )
            }
        }
        FeishuLarkSetupMode::CustomBotWebhook => {
            println!();
            println!("{}", i18n.text(Text::FeishuLarkIntro1));
            println!("{}", i18n.text(Text::FeishuLarkIntro2));
            println!("{}", i18n.text(Text::FeishuLarkIntro3));
            println!();

            let webhook_url = prompt_for_feishu_lark_webhook_url(
                defaults.feishu_lark_webhook_url.as_deref(),
                i18n,
            )?;
            let secret =
                prompt_for_feishu_lark_secret(defaults.feishu_lark_secret.as_deref(), i18n)?;
            setup::build_feishu_lark_config(
                agent,
                answer_detail,
                prompt_detail,
                &webhook_url,
                secret,
            )
        }
        FeishuLarkSetupMode::ExistingSelfBuiltApp => {
            print_feishu_lark_app_bot_setup_checklist(i18n);
            let domain = prompt_for_feishu_lark_app_domain(
                defaults.feishu_lark_app_domain.as_deref(),
                i18n,
            )?;
            let app_id = prompt_for_feishu_lark_app_id(defaults.feishu_lark_app_id.as_deref())?;
            let app_secret =
                prompt_for_feishu_lark_app_secret(defaults.feishu_lark_app_secret.is_some())?;
            let tenant_key =
                prompt_for_feishu_lark_tenant_key(defaults.feishu_lark_tenant_key.as_deref())?;
            let chat_id = prompt_for_feishu_lark_chat_id(defaults.feishu_lark_chat_id.as_deref())?;
            match app_secret {
                FeishuLarkAppSecretPromptResult::Inline(app_secret) => {
                    setup::build_feishu_lark_app_bot_config(
                        agent,
                        answer_detail,
                        prompt_detail,
                        &domain,
                        &app_id,
                        &app_secret,
                        &tenant_key,
                        &chat_id,
                    )
                }
                FeishuLarkAppSecretPromptResult::KeepExisting => {
                    if let Some(app_secret) = defaults.feishu_lark_app_secret.as_deref() {
                        setup::build_feishu_lark_app_bot_config(
                            agent,
                            answer_detail,
                            prompt_detail,
                            &domain,
                            &app_id,
                            app_secret,
                            &tenant_key,
                            &chat_id,
                        )
                    } else {
                        anyhow::bail!("Feishu/Lark App Secret is required");
                    }
                }
            }
        }
    };
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        &route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        &route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(
        config,
        Some(GuidedSetup::FeishuLark {
            agent,
            mode: feishu_lark_mode,
        }),
    )
}

pub(super) fn feishu_lark_route_filters_for_mode(
    mode: FeishuLarkSetupMode,
    route_filters: &SetupRouteFilters,
) -> SetupRouteFilters {
    let mut route_filters = route_filters.clone();
    if mode == FeishuLarkSetupMode::PersonalAgentApp {
        route_filters.only_forward_from_project_paths.clear();
    }
    route_filters
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExistingPersonalAgentCredentials<'a> {
    pub(super) domain: &'a str,
    pub(super) app_id: &'a str,
    pub(super) app_secret: &'a str,
}

pub(super) fn existing_personal_agent_credentials(
    defaults: &SetupDefaults,
) -> Option<ExistingPersonalAgentCredentials<'_>> {
    if defaults.feishu_lark_mode != Some(FeishuLarkSetupMode::PersonalAgentApp)
        || defaults.feishu_lark_chat_id.is_some()
    {
        return None;
    }

    Some(ExistingPersonalAgentCredentials {
        domain: defaults.feishu_lark_app_domain.as_deref()?,
        app_id: defaults.feishu_lark_app_id.as_deref()?,
        app_secret: defaults.feishu_lark_app_secret.as_deref()?,
    })
}

fn print_existing_personal_agent_reuse_notice(i18n: I18n) {
    println!();
    match i18n.language() {
        CliLanguage::English => {
            println!("Personal Agent setup:");
            println!("- Reusing the Personal Agent app already saved in this config.");
            println!(
                "- Direct chat: send `/bind /absolute/project/path`, then send any message to start a new Codex thread."
            );
            println!(
                "- Project rooms: add the Personal Agent to a room, mention it, and send `/bind /absolute/project/path`."
            );
        }
        CliLanguage::SimplifiedChinese => {
            println!("Personal Agent setup:");
            println!("- 复用当前 config 里已经保存的 Personal Agent app。");
            println!(
                "- 私聊：发送 `/bind /absolute/project/path`，之后直接发消息就会启动新的 Codex thread。"
            );
            println!(
                "- 项目群：把 Personal Agent 拉进群，@ 它并发送 `/bind /absolute/project/path`。"
            );
        }
    }
}

fn print_feishu_lark_personal_agent_setup_intro(i18n: I18n) {
    println!();
    match i18n.language() {
        CliLanguage::English => {
            println!("Personal Agent setup:");
            println!("- Agents Router will show a QR code in this terminal.");
            println!(
                "- Scan it with Lark or Feishu, then finish creating the Personal Agent app on the page that opens."
            );
            println!("- Keep this terminal open; setup continues automatically after creation.");
            println!(
                "- Direct chat: send `/bind /absolute/project/path`, then send any message to start a new Codex thread."
            );
            println!(
                "- Project rooms: add the Personal Agent to a room, mention it, and send `/bind /absolute/project/path`."
            );
            println!(
                "- New Codex Desktop updates from a bound project land in that room, with one Lark thread per Codex thread."
            );
        }
        CliLanguage::SimplifiedChinese => {
            println!("Personal Agent setup:");
            println!("- Agents Router 会在这个终端显示二维码。");
            println!("- 用 Lark 或飞书扫码，然后在打开的页面里完成 Personal Agent app 创建。");
            println!("- 保持这个终端打开；创建成功后 setup 会自动继续。");
            println!(
                "- 私聊：发送 `/bind /absolute/project/path`，之后直接发消息就会启动新的 Codex thread。"
            );
            println!(
                "- 项目群：把 Personal Agent 拉进群，@ 它并发送 `/bind /absolute/project/path`。"
            );
            println!(
                "- 绑定项目的新 Codex Desktop 更新会进入项目群，一个 Codex thread 对应一个 Lark thread。"
            );
        }
    }
    println!();
}

fn print_feishu_lark_app_bot_setup_checklist(i18n: I18n) {
    println!();
    match i18n.language() {
        CliLanguage::English => {
            println!("Self-built app checklist:");
            println!("- Add Bot capability.");
            println!(
                "- Enable permissions: im:message:send_as_bot, im:message.group_msg:readonly, im:chat:readonly."
            );
            println!("- Subscribe to im.message.receive_v1 with Long Connection / WebSocket.");
            println!("- Publish a new app version after permission or event changes.");
            println!(
                "Guide: https://github.com/lumpinif/agents-router/blob/main/docs/providers/feishu-lark-app-bot.md"
            );
            println!(
                "Paste App Secret only into this local setup prompt; it is not printed later."
            );
        }
        CliLanguage::SimplifiedChinese => {
            println!("自建应用检查清单:");
            println!("- 添加 Bot 能力。");
            println!(
                "- 开启权限：im:message:send_as_bot、im:message.group_msg:readonly、im:chat:readonly。"
            );
            println!("- 用 Long Connection / WebSocket 订阅 im.message.receive_v1。");
            println!("- 修改权限或事件后，发布新版本并等待审批通过。");
            println!(
                "Guide: https://github.com/lumpinif/agents-router/blob/main/docs/providers/feishu-lark-app-bot.md"
            );
            println!("App Secret 只粘贴到这个本机 setup 输入里；之后不会打印出来。");
        }
    }
    println!();
}

pub(super) fn run_webhook_setup(context: ProviderSetupContext<'_>) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::WebhookIntro1));
    println!("{}", i18n.text(Text::WebhookIntro2));
    println!();

    let webhook_url = prompt_for_webhook_url(defaults.webhook_url.as_deref(), i18n)?;
    let mut config = setup::build_webhook_config(agent, answer_detail, prompt_detail, &webhook_url);
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Webhook { agent }))
}

pub(super) fn run_pushover_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::PushoverIntro1));
    println!("{}", i18n.text(Text::PushoverIntro2));
    println!();

    let app_token = prompt_for_pushover_app_token(defaults.pushover_app_token.as_deref(), i18n)?;
    let user_key = prompt_for_pushover_user_key(defaults.pushover_user_key.as_deref(), i18n)?;
    let device = prompt_for_pushover_device(defaults.pushover_device.as_deref(), i18n)?;
    let sound = prompt_for_pushover_sound(defaults.pushover_sound.as_deref(), i18n)?;
    let mut config = setup::build_pushover_config(
        agent,
        answer_detail,
        prompt_detail,
        &app_token,
        &user_key,
        device,
        sound,
    );
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Pushover { agent }))
}

pub(super) fn run_slack_setup(context: ProviderSetupContext<'_>) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::SlackIntro1));
    println!("{}", i18n.text(Text::SlackIntro2));
    println!();

    let webhook_url = prompt_for_slack_webhook_url(defaults.slack_webhook_url.as_deref(), i18n)?;
    let mut config = setup::build_slack_config(agent, answer_detail, prompt_detail, &webhook_url);
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Slack { agent }))
}

pub(super) fn run_discord_setup(context: ProviderSetupContext<'_>) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::DiscordIntro1));
    println!("{}", i18n.text(Text::DiscordIntro2));
    println!();

    let webhook_url =
        prompt_for_discord_webhook_url(defaults.discord_webhook_url.as_deref(), i18n)?;
    let mut config = setup::build_discord_config(agent, answer_detail, prompt_detail, &webhook_url);
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Discord { agent }))
}

pub(super) fn run_telegram_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::TelegramIntro1));
    println!("{}", i18n.text(Text::TelegramIntro2));
    println!();

    let bot_token = prompt_for_telegram_bot_token(defaults.telegram_bot_token.as_deref(), i18n)?;
    let chat_id = prompt_for_telegram_chat_id(defaults.telegram_chat_id.as_deref(), i18n)?;
    let mut config =
        setup::build_telegram_config(agent, answer_detail, prompt_detail, &bot_token, &chat_id);
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Telegram { agent }))
}

pub(super) fn run_whatsapp_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::WhatsappIntro1));
    println!("{}", i18n.text(Text::WhatsappIntro2));
    println!();

    let access_token =
        prompt_for_whatsapp_access_token(defaults.whatsapp_access_token.as_deref(), i18n)?;
    let phone_number_id =
        prompt_for_whatsapp_phone_number_id(defaults.whatsapp_phone_number_id.as_deref(), i18n)?;
    let recipient_phone_number = prompt_for_whatsapp_recipient_phone_number(
        defaults.whatsapp_recipient_phone_number.as_deref(),
        i18n,
    )?;
    let mut config = setup::build_whatsapp_config(
        agent,
        answer_detail,
        prompt_detail,
        &access_token,
        &phone_number_id,
        &recipient_phone_number,
    );
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Whatsapp { agent }))
}

pub(super) async fn run_wechat_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::WechatIntro1));
    println!("{}", i18n.text(Text::WechatIntro2));
    println!();

    let mut base_url = prompt_for_wechat_base_url(defaults.wechat_base_url.as_deref(), i18n)?;
    let route_tag = prompt_for_wechat_route_tag(defaults.wechat_route_tag.as_deref(), i18n)?;
    let setup_method = prompt_for_wechat_setup_method(i18n)?;

    let (token, scanned_user_id) = match setup_method {
        WechatSetupMethod::QrLogin => {
            let login = run_wechat_qr_login(&base_url, route_tag.as_deref(), i18n).await?;
            base_url = login.base_url;
            (login.token, login.scanned_user_id)
        }
        WechatSetupMethod::ExistingToken => {
            let token = prompt_for_wechat_token(defaults.wechat_token.as_deref(), i18n)?;
            println!("{}", i18n.text(Text::WechatTokenVerifying));
            setup::verify_wechat_token(&base_url, &token, route_tag.as_deref()).await?;
            println!("{}", style(i18n.text(Text::WechatTokenVerified)).green());
            (token, None)
        }
    };

    let token_changed = defaults.wechat_token.as_deref() != Some(token.as_str());
    let linked_recipient = if let (false, Some(recipient_user_id), Some(context_token)) = (
        token_changed,
        defaults.wechat_recipient_user_id.as_ref(),
        defaults.wechat_context_token.as_ref(),
    ) {
        if prompt_confirm(i18n.text(Text::WechatKeepRecipientPrompt), true)? {
            setup::WechatRecipientLink {
                recipient_user_id: recipient_user_id.clone(),
                context_token: context_token.clone(),
            }
        } else {
            link_wechat_recipient(
                &base_url,
                &token,
                route_tag.as_deref(),
                scanned_user_id.as_deref(),
                i18n,
            )
            .await?
        }
    } else {
        link_wechat_recipient(
            &base_url,
            &token,
            route_tag.as_deref(),
            scanned_user_id.as_deref(),
            i18n,
        )
        .await?
    };

    let mut config = setup::build_wechat_config(
        agent,
        answer_detail,
        prompt_detail,
        &base_url,
        &token,
        &linked_recipient.recipient_user_id,
        &linked_recipient.context_token,
        route_tag,
    );
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::Wechat { agent }))
}

pub(super) fn run_microsoft_teams_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::TeamsIntro1));
    println!("{}", i18n.text(Text::TeamsIntro2));
    println!();

    let webhook_url = prompt_for_microsoft_teams_webhook_url(
        defaults.microsoft_teams_webhook_url.as_deref(),
        i18n,
    )?;
    let mut config =
        setup::build_microsoft_teams_config(agent, answer_detail, prompt_detail, &webhook_url);
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::MicrosoftTeams { agent }))
}

pub(super) fn run_email_smtp_setup(
    context: ProviderSetupContext<'_>,
) -> anyhow::Result<LoadedConfig> {
    let ProviderSetupContext {
        path,
        mode,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        defaults,
        i18n,
    } = context;
    println!();
    println!("{}", i18n.text(Text::EmailIntro1));
    println!("{}", i18n.text(Text::EmailIntro2));
    println!();

    let host = prompt_for_email_smtp_host(defaults.email_smtp_host.as_deref(), i18n)?;
    let security = prompt_for_email_smtp_security(defaults.email_smtp_security, i18n)?;
    let default_port = defaults
        .email_smtp_port
        .unwrap_or_else(|| default_email_smtp_port(security));
    let port = prompt_for_email_smtp_port(default_port, i18n)?;
    let username = prompt_for_email_smtp_username(defaults.email_smtp_username.as_deref(), i18n)?;
    let password = if username.is_some() {
        Some(prompt_for_email_smtp_password(
            defaults.email_smtp_password.as_deref(),
            i18n,
        )?)
    } else {
        None
    };
    let from_label = localized(i18n, "From email", "发件人邮箱");
    let reply_to_label = localized(i18n, "Reply-To email", "Reply-To 邮箱");
    let from =
        prompt_for_email_smtp_mailbox(from_label, defaults.email_smtp_from.as_deref(), i18n)?;
    let to = prompt_for_email_smtp_recipients(defaults.email_smtp_to.as_deref(), i18n)?;
    let reply_to = prompt_for_email_smtp_optional_mailbox(
        reply_to_label,
        defaults.email_smtp_reply_to.as_deref(),
        i18n,
    )?;
    let mut config = setup::build_email_smtp_config(
        agent,
        answer_detail,
        prompt_detail,
        &host,
        port,
        security,
        username,
        password,
        &from,
        to,
        reply_to,
    );
    write_setup_config_with_route_filters(
        path,
        &mut config,
        i18n.language(),
        agent,
        route_filters,
    )?;

    println!();
    print_setup_summary(
        mode,
        path,
        agent,
        answer_detail,
        prompt_detail,
        route_filters,
        i18n,
    );
    print_setup_provider_summary(&config, i18n);

    loaded_config(config, Some(GuidedSetup::EmailSmtp { agent }))
}
