use super::*;

pub fn ntfy_subscriptions(config: &RawConfig) -> Vec<NtfySubscription> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Ntfy)
        .filter_map(|provider| {
            Some(NtfySubscription {
                provider_id: provider.id.clone(),
                server: provider.server.as_ref()?.clone(),
                topic: provider.topic.as_ref()?.clone(),
            })
        })
        .collect()
}

pub fn feishu_lark_targets(config: &RawConfig) -> Vec<FeishuLarkTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::FeishuLark)
        .filter(|provider| provider.mode.as_deref() != Some("app_bot"))
        .filter_map(|provider| {
            let webhook_host = match (
                provider
                    .url
                    .as_deref()
                    .filter(|value| !value.trim().is_empty()),
                provider
                    .url_env
                    .as_deref()
                    .filter(|value| !value.trim().is_empty()),
            ) {
                (Some(url), _) => webhook_host(url),
                (None, Some(env_name)) => format!("env:{env_name}"),
                (None, None) => return None,
            };

            Some(FeishuLarkTarget {
                provider_id: provider.id.clone(),
                webhook_host,
                signed: provider
                    .secret
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || provider
                        .secret_env
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
            })
        })
        .collect()
}

pub fn feishu_lark_app_bot_targets(config: &RawConfig) -> Vec<FeishuLarkAppBotTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::FeishuLark)
        .filter(|provider| provider.mode.as_deref() == Some("app_bot"))
        .filter_map(|provider| {
            Some(FeishuLarkAppBotTarget {
                provider_id: provider.id.clone(),
                domain: provider.domain.as_ref()?.clone(),
                app_id: provider.app_id.as_ref()?.clone(),
                app_secret_configured: provider
                    .app_secret
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || provider
                        .app_secret_env
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
                app_registration_source: provider.app_registration_source.clone(),
                operator_open_id: provider.operator_open_id.clone(),
                tenant_key: provider.tenant_key.clone(),
                chat_id: provider.chat_id.clone(),
            })
        })
        .collect()
}

pub fn webhook_targets(config: &RawConfig) -> Vec<WebhookTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Webhook)
        .filter_map(|provider| {
            let webhook_host = match (
                provider
                    .url
                    .as_deref()
                    .filter(|value| !value.trim().is_empty()),
                provider
                    .url_env
                    .as_deref()
                    .filter(|value| !value.trim().is_empty()),
            ) {
                (Some(url), _) => webhook_host(url),
                (None, Some(env_name)) => format!("env:{env_name}"),
                (None, None) => return None,
            };

            Some(WebhookTarget {
                provider_id: provider.id.clone(),
                webhook_host,
            })
        })
        .collect()
}

pub fn pushover_targets(config: &RawConfig) -> Vec<PushoverTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Pushover)
        .map(|provider| PushoverTarget {
            provider_id: provider.id.clone(),
            device: provider.device.clone(),
            sound: provider.sound.clone(),
        })
        .collect()
}

pub fn slack_targets(config: &RawConfig) -> Vec<SlackTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Slack)
        .filter_map(|provider| {
            provider_url_host(provider).map(|webhook_host| SlackTarget {
                provider_id: provider.id.clone(),
                webhook_host,
            })
        })
        .collect()
}

pub fn discord_targets(config: &RawConfig) -> Vec<DiscordTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Discord)
        .filter_map(|provider| {
            provider_url_host(provider).map(|webhook_host| DiscordTarget {
                provider_id: provider.id.clone(),
                webhook_host,
            })
        })
        .collect()
}

pub fn telegram_targets(config: &RawConfig) -> Vec<TelegramTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Telegram)
        .filter_map(|provider| {
            Some(TelegramTarget {
                provider_id: provider.id.clone(),
                chat_id: provider.chat_id.as_ref()?.clone(),
            })
        })
        .collect()
}

pub fn whatsapp_targets(config: &RawConfig) -> Vec<WhatsappTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Whatsapp)
        .filter_map(|provider| {
            Some(WhatsappTarget {
                provider_id: provider.id.clone(),
                recipient_phone_number: provider.recipient_phone_number.as_ref()?.clone(),
            })
        })
        .collect()
}

pub fn wechat_targets(config: &RawConfig) -> Vec<WechatTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::Wechat)
        .filter_map(|provider| {
            let base_url = provider.base_url.as_ref()?;
            Some(WechatTarget {
                provider_id: provider.id.clone(),
                base_url_host: host_label(base_url),
                recipient_user_id: provider.recipient_user_id.as_ref()?.clone(),
            })
        })
        .collect()
}

pub fn microsoft_teams_targets(config: &RawConfig) -> Vec<MicrosoftTeamsTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::MicrosoftTeams)
        .filter_map(|provider| {
            provider_url_host(provider).map(|webhook_host| MicrosoftTeamsTarget {
                provider_id: provider.id.clone(),
                webhook_host,
            })
        })
        .collect()
}

pub fn email_smtp_targets(config: &RawConfig) -> Vec<EmailSmtpTarget> {
    config
        .providers
        .iter()
        .filter(|provider| provider.provider_type == ProviderType::EmailSmtp)
        .filter_map(|provider| {
            Some(EmailSmtpTarget {
                provider_id: provider.id.clone(),
                host: provider.host.as_ref()?.clone(),
                port: provider.port?,
                from: provider.from.as_ref()?.clone(),
                to: provider.to.as_ref()?.clone(),
            })
        })
        .collect()
}
