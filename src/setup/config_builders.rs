use super::*;

pub fn build_ntfy_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    topic: &str,
) -> RawConfig {
    let mut provider = RawProviderConfig::new(ProviderType::Ntfy.as_str(), ProviderType::Ntfy);
    provider.server = Some(DEFAULT_NTFY_SERVER.to_string());
    provider.topic = Some(topic.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_feishu_lark_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    webhook_url: &str,
    secret: Option<String>,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::FeishuLark.as_str(), ProviderType::FeishuLark);
    provider.url = Some(webhook_url.to_string());
    provider.secret = secret;

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_webhook_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    webhook_url: &str,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::Webhook.as_str(), ProviderType::Webhook);
    provider.url = Some(webhook_url.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_pushover_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    app_token: &str,
    user_key: &str,
    device: Option<String>,
    sound: Option<String>,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::Pushover.as_str(), ProviderType::Pushover);
    provider.app_token = Some(app_token.to_string());
    provider.user_key = Some(user_key.to_string());
    provider.device = device;
    provider.sound = sound;

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_slack_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    webhook_url: &str,
) -> RawConfig {
    let mut provider = RawProviderConfig::new(ProviderType::Slack.as_str(), ProviderType::Slack);
    provider.url = Some(webhook_url.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_discord_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    webhook_url: &str,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::Discord.as_str(), ProviderType::Discord);
    provider.url = Some(webhook_url.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_telegram_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    bot_token: &str,
    chat_id: &str,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::Telegram.as_str(), ProviderType::Telegram);
    provider.bot_token = Some(bot_token.to_string());
    provider.chat_id = Some(chat_id.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_whatsapp_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    access_token: &str,
    phone_number_id: &str,
    recipient_phone_number: &str,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::Whatsapp.as_str(), ProviderType::Whatsapp);
    provider.access_token = Some(access_token.to_string());
    provider.phone_number_id = Some(phone_number_id.to_string());
    provider.recipient_phone_number = Some(recipient_phone_number.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

#[allow(clippy::too_many_arguments)]
pub fn build_wechat_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    base_url: &str,
    token: &str,
    recipient_user_id: &str,
    context_token: &str,
    route_tag: Option<String>,
) -> RawConfig {
    let mut provider = RawProviderConfig::new(ProviderType::Wechat.as_str(), ProviderType::Wechat);
    provider.base_url = Some(base_url.to_string());
    provider.token = Some(token.to_string());
    provider.recipient_user_id = Some(recipient_user_id.to_string());
    provider.context_token = Some(context_token.to_string());
    provider.route_tag = route_tag;

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn build_microsoft_teams_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    webhook_url: &str,
) -> RawConfig {
    let mut provider = RawProviderConfig::new(
        ProviderType::MicrosoftTeams.as_str(),
        ProviderType::MicrosoftTeams,
    );
    provider.url = Some(webhook_url.to_string());

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

#[allow(clippy::too_many_arguments)]
pub fn build_email_smtp_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    host: &str,
    port: u16,
    security: EmailSmtpSecurity,
    username: Option<String>,
    password: Option<String>,
    from: &str,
    to: Vec<String>,
    reply_to: Option<String>,
) -> RawConfig {
    let mut provider =
        RawProviderConfig::new(ProviderType::EmailSmtp.as_str(), ProviderType::EmailSmtp);
    provider.host = Some(host.to_string());
    provider.port = Some(port);
    provider.security = Some(security);
    provider.username = username;
    provider.password = password;
    provider.from = Some(from.to_string());
    provider.to = Some(to);
    provider.reply_to = reply_to;

    build_config(agent, answer_detail, prompt_detail, vec![provider])
}

pub fn apply_agent_route_filters(
    config: &mut RawConfig,
    agent: AgentIntegrationId,
    minimum_task_duration_minutes: Option<u64>,
    only_forward_from_project_paths: Vec<String>,
) {
    let source_id = agent.source_id();
    let minimum_task_duration_minutes = if agent.supports_duration_filter() {
        minimum_task_duration_minutes
    } else {
        None
    };
    if let Some(route) = config
        .routes
        .iter_mut()
        .find(|route| route.sources.iter().any(|source| source == source_id))
    {
        route.minimum_task_duration_minutes = minimum_task_duration_minutes;
        route.only_forward_from_project_paths = only_forward_from_project_paths;
    }
}

fn build_config(
    agent: AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    providers: Vec<RawProviderConfig>,
) -> RawConfig {
    let provider_ids: Vec<String> = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect();
    let agent_source = agent.source_config();
    let agent_source_id = agent_source.id.clone();

    RawConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        cli: CliConfig::default(),
        log: LogConfig::default(),
        notification: NotificationConfig {
            answer_detail,
            prompt_detail,
        },
        sources: vec![
            agent_source,
            SourceConfig {
                id: "agents_router".to_string(),
                source_type: SourceType::AgentsRouter,
            },
        ],
        providers,
        routes: vec![
            RouteConfig::new(vec![agent_source_id], provider_ids.clone()),
            RouteConfig::new(vec!["agents_router".to_string()], provider_ids),
        ],
    }
}
