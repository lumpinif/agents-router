use super::*;

pub(in crate::cli) fn prompt_for_agent(
    default: Option<setup::AgentIntegrationId>,
    i18n: I18n,
) -> anyhow::Result<setup::AgentIntegrationId> {
    let options = supported_agent_options();
    let supported_default =
        default.filter(|agent| options.iter().any(|(_, candidate)| candidate == agent));
    let effective_default = supported_default.unwrap_or_else(default_agent_for_platform);
    let default_index = options
        .iter()
        .position(|(_, agent)| *agent == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|(_, agent)| {
            let mut label = agent.display_name().to_string();
            if Some(*agent) == supported_default {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            } else if supported_default.is_none() && *agent == effective_default {
                label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(i18n.text(Text::AgentPrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read agent choice")?;

    Ok(options[selection].1)
}

pub(in crate::cli) fn supported_agent_options() -> Vec<(String, setup::AgentIntegrationId)> {
    supported_agent_options_for_platform(RuntimePlatform::current())
}

pub(in crate::cli) fn supported_agent_options_for_platform(
    platform: RuntimePlatform,
) -> Vec<(String, setup::AgentIntegrationId)> {
    setup_agent_integration_descriptors_for_platform(platform)
        .enumerate()
        .map(|(index, descriptor)| ((index + 1).to_string(), descriptor.id))
        .collect()
}

pub(in crate::cli) fn default_agent_for_platform() -> setup::AgentIntegrationId {
    default_agent_for_runtime_platform(RuntimePlatform::current())
}

pub(in crate::cli) fn default_agent_for_runtime_platform(
    platform: RuntimePlatform,
) -> setup::AgentIntegrationId {
    default_agent_integration_for_platform(platform).id
}

pub(in crate::cli) fn prompt_for_answer_detail(
    default: Option<AnswerDetail>,
    i18n: I18n,
) -> anyhow::Result<AnswerDetail> {
    let effective_default = default.unwrap_or(AnswerDetail::Preview);
    let options = [AnswerDetail::Preview, AnswerDetail::Full];
    let default_index = options
        .iter()
        .position(|answer_detail| *answer_detail == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|answer_detail| {
            let mut label = i18n.answer_detail(*answer_detail).to_string();
            if Some(*answer_detail) == default {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            } else if default.is_none() && *answer_detail == AnswerDetail::Preview {
                label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(i18n.text(Text::AnswerDetailPrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read answer detail choice")?;

    Ok(options[selection])
}

pub(in crate::cli) fn prompt_for_prompt_detail(
    default: Option<PromptDetail>,
    i18n: I18n,
) -> anyhow::Result<PromptDetail> {
    let effective_default = default.unwrap_or(PromptDetail::Off);
    let options = [PromptDetail::Off, PromptDetail::On];
    let default_index = options
        .iter()
        .position(|prompt_detail| *prompt_detail == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|prompt_detail| {
            let mut label = i18n.prompt_detail(*prompt_detail).to_string();
            if Some(*prompt_detail) == default {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            } else if default.is_none() && *prompt_detail == PromptDetail::Off {
                label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(i18n.text(Text::PromptDetailPrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read prompt detail choice")?;

    Ok(options[selection])
}

pub(in crate::cli) fn prompt_for_notification_preference(
    default: Option<u64>,
    i18n: I18n,
) -> anyhow::Result<Option<u64>> {
    let effective_default = match default {
        None => NotificationPreference::EveryCompletedTask,
        Some(5) => NotificationPreference::FiveMinutesOrLonger,
        Some(_) => NotificationPreference::CustomMinimumDuration,
    };
    let options = [
        NotificationPreference::EveryCompletedTask,
        NotificationPreference::FiveMinutesOrLonger,
        NotificationPreference::CustomMinimumDuration,
    ];
    let default_index = options
        .iter()
        .position(|preference| *preference == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|preference| {
            let mut label = notification_preference_label(*preference, default, i18n);
            if *preference == effective_default && default.is_some() {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            } else if default.is_none() && *preference == NotificationPreference::EveryCompletedTask
            {
                label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(i18n.text(Text::NotificationPreferencePrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read notification preference")?;

    match options[selection] {
        NotificationPreference::EveryCompletedTask => Ok(None),
        NotificationPreference::FiveMinutesOrLonger => Ok(Some(5)),
        NotificationPreference::CustomMinimumDuration => {
            prompt_for_custom_minimum_task_duration_minutes(default, i18n).map(Some)
        }
    }
}

fn notification_preference_label(
    preference: NotificationPreference,
    default: Option<u64>,
    i18n: I18n,
) -> String {
    match preference {
        NotificationPreference::EveryCompletedTask => {
            localized(i18n, "Every completed task", "每个完成的任务").to_string()
        }
        NotificationPreference::FiveMinutesOrLonger => {
            localized(i18n, "Tasks 5 minutes or longer", "5 分钟或更久的任务").to_string()
        }
        NotificationPreference::CustomMinimumDuration => match default {
            Some(minutes) if minutes != 5 => localized_string(
                i18n,
                format!("Custom minimum duration ({minutes} minutes)"),
                format!("自定义最短任务时长（{minutes} 分钟）"),
            ),
            _ => localized(i18n, "Custom minimum duration", "自定义最短任务时长").to_string(),
        },
    }
}

fn prompt_for_custom_minimum_task_duration_minutes(
    default: Option<u64>,
    i18n: I18n,
) -> anyhow::Result<u64> {
    loop {
        let prompt = match default {
            Some(minutes) => localized_string(
                i18n,
                format!("Minimum task duration in minutes [{minutes}, press Enter to keep]"),
                format!("最短任务时长，单位分钟 [{minutes}，按 Enter 保留]"),
            ),
            None => localized(
                i18n,
                "Minimum task duration in minutes",
                "最短任务时长，单位分钟",
            )
            .to_string(),
        };
        let input = prompt_text(prompt, "failed to read minimum task duration")?;
        let candidate = if input.trim().is_empty() {
            default.map(|minutes| minutes.to_string())
        } else {
            Some(input.trim().to_string())
        };

        let Some(candidate) = candidate else {
            println!(
                "{}",
                localized(
                    i18n,
                    "Enter a whole number greater than 0.",
                    "请输入大于 0 的整数。"
                )
            );
            continue;
        };

        match candidate.parse::<u64>() {
            Ok(minutes) if minutes > 0 && minutes.checked_mul(60_000).is_some() => {
                return Ok(minutes);
            }
            _ => println!(
                "{}",
                localized(
                    i18n,
                    "Enter a whole number greater than 0.",
                    "请输入大于 0 的整数。"
                )
            ),
        }
    }
}

pub(in crate::cli) fn prompt_for_initial_provider(
    default: Option<ProviderType>,
    i18n: I18n,
) -> anyhow::Result<ProviderType> {
    let setup_default_provider = default_setup_provider_type();
    let effective_default = default.unwrap_or(setup_default_provider);
    let options = setup_provider_descriptors()
        .map(|descriptor| descriptor.provider_type)
        .collect::<Vec<_>>();
    let default_index = options
        .iter()
        .position(|provider| *provider == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|provider| {
            let mut label = localized_provider_display_name(*provider, i18n).to_string();
            if Some(*provider) == default {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            } else if default.is_none() && *provider == setup_default_provider {
                label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(i18n.text(Text::ProviderPrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read provider choice")?;

    Ok(options[selection])
}

fn localized_provider_display_name(provider: ProviderType, i18n: I18n) -> &'static str {
    if provider == ProviderType::Wechat && i18n.language() == CliLanguage::SimplifiedChinese {
        "微信"
    } else {
        provider_descriptor(provider).display_name
    }
}

pub(in crate::cli) fn prompt_for_ntfy_topic(
    generated_topic: &str,
    current_topic: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let default_topic = current_topic.unwrap_or(generated_topic);
        let prompt = if let Some(current_topic) = current_topic {
            localized_string(
                i18n,
                format!("Topic [{current_topic}, press Enter to keep]"),
                format!("Topic [{current_topic}，按 Enter 保留]"),
            )
        } else {
            format!("Topic [{generated_topic}]")
        };
        let input = prompt_text(prompt, "failed to read topic")?;

        match setup::resolve_ntfy_topic(&input, default_topic) {
            Ok(topic) => return Ok(topic),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_feishu_lark_webhook_url(
    current_url: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_url) = current_url {
            localized_string(
                i18n,
                format!(
                    "Webhook URL [configured: {}, edit or press Enter to keep]",
                    safe_url_host(current_url)
                ),
                format!(
                    "Webhook URL [已配置：{}，修改或按 Enter 保留]",
                    safe_url_host(current_url)
                ),
            )
        } else {
            "Webhook URL".to_string()
        };
        let input = prompt_private_text(prompt, current_url, "failed to read webhook URL")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_url.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_feishu_lark_webhook_url(candidate) {
            Ok(url) => return Ok(url),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_feishu_lark_secret(
    current_secret: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<Option<String>> {
    let prompt = if current_secret.is_some() {
        localized(
            i18n,
            "Signing secret (optional) [configured, press Enter to keep, type `none` to clear]",
            "签名 secret（可选）[已配置，按 Enter 保留，输入 `none` 清除]",
        )
    } else {
        localized(i18n, "Signing secret (optional)", "签名 secret（可选）")
    };
    let input = prompt_secret(prompt, "failed to read signing secret")?;

    let input = input.trim();
    if input.is_empty() {
        return Ok(current_secret.map(ToOwned::to_owned));
    }
    if input.eq_ignore_ascii_case("none") {
        return Ok(None);
    }

    Ok(setup::resolve_feishu_lark_secret(input))
}

pub(in crate::cli) fn prompt_for_feishu_lark_mode(
    agent: setup::AgentIntegrationId,
    default: Option<FeishuLarkSetupMode>,
    i18n: I18n,
) -> anyhow::Result<FeishuLarkSetupMode> {
    let effective_default = default.unwrap_or_default();
    let options = [
        FeishuLarkSetupMode::CustomBotWebhook,
        FeishuLarkSetupMode::AppBot,
    ];
    let default_index = options
        .iter()
        .position(|mode| *mode == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|mode| feishu_lark_mode_option_label(*mode, agent, default, i18n))
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(localized(i18n, "Feishu/Lark mode", "Feishu/Lark 模式"))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read Feishu/Lark mode")?;

    Ok(options[selection])
}

fn feishu_lark_mode_option_label(
    mode: FeishuLarkSetupMode,
    agent: setup::AgentIntegrationId,
    default: Option<FeishuLarkSetupMode>,
    i18n: I18n,
) -> String {
    let mut label = match mode {
        FeishuLarkSetupMode::CustomBotWebhook => {
            "Custom Bot Webhook — Stable\n  Send one-way notifications to a group. Fastest setup. No replies."
                .to_string()
        }
        FeishuLarkSetupMode::AppBot => {
            if let Some(release_stage) = agent
                .descriptor()
                .continuation_capability()
                .release_stage()
            {
                format!(
                    "App Bot — {}\n  Send notifications through an app bot. Lark thread replies can continue Codex Desktop sessions; validate Feishu before relying on replies.",
                    continuation_release_stage_label(release_stage)
                )
            } else {
                "App Bot\n  Send notifications through an app bot. Replies are only available for Codex Desktop when continuation support is available."
                    .to_string()
            }
        }
    };

    if Some(mode) == default {
        label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
    } else if default.is_none() && mode == FeishuLarkSetupMode::CustomBotWebhook {
        label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
    }

    label
}

fn continuation_release_stage_label(release_stage: ContinuationReleaseStage) -> &'static str {
    match release_stage {
        ContinuationReleaseStage::Experimental => "Experimental",
        ContinuationReleaseStage::Stable => "Stable",
    }
}

pub(in crate::cli) fn prompt_for_feishu_lark_app_domain(
    current_domain: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    let options = ["lark", "feishu"];
    let current_domain =
        current_domain.and_then(|domain| options.iter().position(|candidate| *candidate == domain));
    let default_index = current_domain.unwrap_or(0);
    let items = options
        .iter()
        .enumerate()
        .map(|(index, domain)| {
            let mut label = (*domain).to_string();
            if Some(index) == current_domain {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt("Domain")
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read Feishu/Lark App Bot domain")?;

    setup::resolve_feishu_lark_app_domain(options[selection])
}

pub(in crate::cli) fn prompt_for_feishu_lark_app_id(
    current_app_id: Option<&str>,
) -> anyhow::Result<String> {
    prompt_for_required_feishu_lark_app_bot_field(
        "App ID",
        current_app_id,
        setup::resolve_feishu_lark_app_id,
        "failed to read Feishu/Lark App ID",
    )
}

pub(in crate::cli) fn prompt_for_feishu_lark_app_secret_env(
    current_env: Option<&str>,
    domain: &str,
) -> anyhow::Result<String> {
    let default_env = match domain {
        "feishu" => "AGENTS_ROUTER_FEISHU_APP_SECRET",
        _ => "AGENTS_ROUTER_LARK_APP_SECRET",
    };
    let fallback = current_env.unwrap_or(default_env);
    let prompt = if let Some(current_env) = current_env {
        format!("App Secret env var name [{current_env}, press Enter to keep]")
    } else {
        format!("App Secret env var name [{default_env}]")
    };
    loop {
        let input = prompt_text(
            prompt.clone(),
            "failed to read Feishu/Lark App Secret env var",
        )?;
        let candidate = if input.trim().is_empty() {
            fallback
        } else {
            input.trim()
        };

        match setup::resolve_feishu_lark_app_secret_env(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_feishu_lark_tenant_key(
    current_tenant_key: Option<&str>,
) -> anyhow::Result<String> {
    prompt_for_required_feishu_lark_app_bot_field(
        "Tenant Key",
        current_tenant_key,
        setup::resolve_feishu_lark_tenant_key,
        "failed to read Feishu/Lark tenant_key",
    )
}

pub(in crate::cli) fn prompt_for_feishu_lark_chat_id(
    current_chat_id: Option<&str>,
) -> anyhow::Result<String> {
    prompt_for_required_feishu_lark_app_bot_field(
        "Chat ID",
        current_chat_id,
        setup::resolve_feishu_lark_chat_id,
        "failed to read Feishu/Lark chat_id",
    )
}

fn prompt_for_required_feishu_lark_app_bot_field(
    label: &'static str,
    current_value: Option<&str>,
    resolve: fn(&str) -> anyhow::Result<String>,
    read_context: &'static str,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_value) = current_value {
            format!("{label} [{current_value}, press Enter to keep]")
        } else {
            label.to_string()
        };
        let input = prompt_text(prompt, read_context)?;
        let candidate = if input.trim().is_empty() {
            current_value.unwrap_or("")
        } else {
            input.trim()
        };

        match resolve(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_webhook_url(
    current_url: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_url) = current_url {
            localized_string(
                i18n,
                format!(
                    "Webhook URL [configured: {}, edit or press Enter to keep]",
                    safe_url_host(current_url)
                ),
                format!(
                    "Webhook URL [已配置：{}，修改或按 Enter 保留]",
                    safe_url_host(current_url)
                ),
            )
        } else {
            "Webhook URL".to_string()
        };
        let input = prompt_private_text(prompt, current_url, "failed to read webhook URL")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_url.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_webhook_url(candidate) {
            Ok(url) => return Ok(url),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_slack_webhook_url(
    current_url: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_url) = current_url {
            localized_string(
                i18n,
                format!(
                    "Slack webhook URL [configured: {}, edit or press Enter to keep]",
                    safe_url_host(current_url)
                ),
                format!(
                    "Slack webhook URL [已配置：{}，修改或按 Enter 保留]",
                    safe_url_host(current_url)
                ),
            )
        } else {
            "Slack webhook URL".to_string()
        };
        let input = prompt_private_text(prompt, current_url, "failed to read Slack webhook URL")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_url.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_slack_webhook_url(candidate) {
            Ok(url) => return Ok(url),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_discord_webhook_url(
    current_url: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_url) = current_url {
            localized_string(
                i18n,
                format!(
                    "Discord webhook URL [configured: {}, edit or press Enter to keep]",
                    safe_url_host(current_url)
                ),
                format!(
                    "Discord webhook URL [已配置：{}，修改或按 Enter 保留]",
                    safe_url_host(current_url)
                ),
            )
        } else {
            "Discord webhook URL".to_string()
        };
        let input = prompt_private_text(prompt, current_url, "failed to read Discord webhook URL")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_url.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_discord_webhook_url(candidate) {
            Ok(url) => return Ok(url),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_telegram_bot_token(
    current_token: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if current_token.is_some() {
            localized(
                i18n,
                "Telegram bot token [configured, press Enter to keep]",
                "Telegram bot token [已配置，按 Enter 保留]",
            )
        } else {
            "Telegram bot token"
        };
        let input = prompt_secret(prompt, "failed to read Telegram bot token")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_token.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_telegram_bot_token(candidate) {
            Ok(token) => return Ok(token),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_telegram_chat_id(
    current_chat_id: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_chat_id) = current_chat_id {
            localized_string(
                i18n,
                format!("Telegram chat id [{current_chat_id}, press Enter to keep]"),
                format!("Telegram chat id [{current_chat_id}，按 Enter 保留]"),
            )
        } else {
            localized(
                i18n,
                "Telegram chat id, for example 123456789 or @channelusername",
                "Telegram chat id，例如 123456789 或 @channelusername",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read Telegram chat id")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_chat_id.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_telegram_chat_id(candidate) {
            Ok(chat_id) => return Ok(chat_id),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_whatsapp_access_token(
    current_token: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if current_token.is_some() {
            localized(
                i18n,
                "WhatsApp access token [configured, press Enter to keep]",
                "WhatsApp access token [已配置，按 Enter 保留]",
            )
        } else {
            "WhatsApp access token"
        };
        let input = prompt_secret(prompt, "failed to read WhatsApp access token")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_token.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_whatsapp_access_token(candidate) {
            Ok(token) => return Ok(token),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_whatsapp_phone_number_id(
    current_phone_number_id: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_phone_number_id) = current_phone_number_id {
            localized_string(
                i18n,
                format!(
                    "WhatsApp phone number ID [{current_phone_number_id}, press Enter to keep]"
                ),
                format!("WhatsApp phone number ID [{current_phone_number_id}，按 Enter 保留]"),
            )
        } else {
            "WhatsApp phone number ID".to_string()
        };
        let input = prompt_text(prompt, "failed to read WhatsApp phone number ID")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_phone_number_id.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_whatsapp_phone_number_id(candidate) {
            Ok(phone_number_id) => return Ok(phone_number_id),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_whatsapp_recipient_phone_number(
    current_phone_number: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_phone_number) = current_phone_number {
            localized_string(
                i18n,
                format!(
                    "WhatsApp recipient phone number [{current_phone_number}, press Enter to keep]"
                ),
                format!("WhatsApp recipient phone number [{current_phone_number}，按 Enter 保留]"),
            )
        } else {
            localized(
                i18n,
                "WhatsApp recipient phone number, digits only with country code",
                "WhatsApp 接收手机号，只填数字，包含国家区号",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read WhatsApp recipient phone number")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_phone_number.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_whatsapp_recipient_phone_number(candidate) {
            Ok(phone_number) => return Ok(phone_number),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_wechat_base_url(
    current_base_url: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let default_base_url = current_base_url.unwrap_or(setup::DEFAULT_WECHAT_BASE_URL);
        let prompt = if let Some(current_base_url) = current_base_url {
            localized_string(
                i18n,
                format!(
                    "WeChat gateway URL [{current_base_url}, press Enter to keep unless your iLink provider gave you another URL]"
                ),
                format!(
                    "微信 gateway URL [{current_base_url}，没有新的 iLink URL 就按 Enter 保留]"
                ),
            )
        } else {
            localized_string(
                i18n,
                format!(
                    "WeChat gateway URL [{}; press Enter for the default]",
                    setup::DEFAULT_WECHAT_BASE_URL
                ),
                format!(
                    "微信 gateway URL [{}；按 Enter 使用默认值]",
                    setup::DEFAULT_WECHAT_BASE_URL
                ),
            )
        };
        let input = prompt_text(prompt, "failed to read WeChat iLink base URL")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            default_base_url
        } else {
            input
        };

        match setup::resolve_wechat_base_url(candidate) {
            Ok(base_url) => return Ok(base_url),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_wechat_route_tag(
    current_route_tag: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<Option<String>> {
    loop {
        let prompt = if let Some(current_route_tag) = current_route_tag {
            localized_string(
                i18n,
                format!(
                    "Optional WeChat route tag [{current_route_tag}, press Enter to keep, type `none` to clear]"
                ),
                format!(
                    "可选微信 route tag [{current_route_tag}，按 Enter 保留，输入 `none` 清除]"
                ),
            )
        } else {
            localized(
                i18n,
                "Optional WeChat route tag (advanced; press Enter to skip unless your iLink provider gave you an SKRouteTag)",
                "可选微信 route tag（高级项；没有服务方给你的 SKRouteTag 就直接按 Enter 跳过）",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read WeChat SKRouteTag")?;

        let input = input.trim();
        if input.is_empty() {
            return Ok(current_route_tag.map(ToOwned::to_owned));
        }
        if current_route_tag.is_some() && input.eq_ignore_ascii_case("none") {
            return Ok(None);
        }

        match setup::resolve_wechat_route_tag(input) {
            Ok(route_tag) => return Ok(route_tag),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_wechat_setup_method(
    i18n: I18n,
) -> anyhow::Result<WechatSetupMethod> {
    let options = [WechatSetupMethod::QrLogin, WechatSetupMethod::ExistingToken];
    let default_index = options
        .iter()
        .position(|method| *method == WechatSetupMethod::QrLogin)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|method| match method {
            WechatSetupMethod::QrLogin => i18n.text(Text::WechatScanQrOption),
            WechatSetupMethod::ExistingToken => i18n.text(Text::WechatExistingTokenOption),
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(i18n.text(Text::WechatSetupMethodPrompt))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read WeChat setup method")?;

    Ok(options[selection])
}

pub(in crate::cli) fn prompt_for_wechat_token(
    current_token: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if current_token.is_some() {
            localized(
                i18n,
                "WeChat iLink token [configured, press Enter to keep]",
                "微信 iLink token [已配置，按 Enter 保留]",
            )
        } else {
            "WeChat iLink token"
        };
        let input = prompt_secret(prompt, "failed to read WeChat iLink token")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_token.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_wechat_token(candidate) {
            Ok(token) => return Ok(token),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) async fn run_wechat_qr_login(
    base_url: &str,
    route_tag: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<WechatLogin> {
    let mut qr =
        setup::fetch_wechat_qr_code(base_url, route_tag, setup::DEFAULT_WECHAT_BOT_TYPE).await?;
    print_wechat_qr_code(&qr.qr_content, i18n)?;

    let deadline = Instant::now() + setup::DEFAULT_WECHAT_QR_TIMEOUT;
    let mut refresh_count = 1;
    let mut scanned_message_printed = false;

    while Instant::now() < deadline {
        let status = setup::poll_wechat_qr_status(base_url, &qr.qr_key, route_tag).await?;
        match status.status.as_str() {
            "" | "wait" => {
                sleep(Duration::from_secs(1)).await;
            }
            "scaned" | "scanned" => {
                if !scanned_message_printed {
                    println!("{}", i18n.text(Text::WechatQrScanned));
                    scanned_message_printed = true;
                }
                sleep(Duration::from_secs(1)).await;
            }
            "expired" => {
                refresh_count += 1;
                if refresh_count > 3 {
                    anyhow::bail!("WeChat QR code expired too many times; run setup again");
                }
                println!("{}", i18n.text(Text::WechatQrExpired));
                qr = setup::fetch_wechat_qr_code(
                    base_url,
                    route_tag,
                    setup::DEFAULT_WECHAT_BOT_TYPE,
                )
                .await?;
                print_wechat_qr_code(&qr.qr_content, i18n)?;
                scanned_message_printed = false;
            }
            "confirmed" => {
                let token = status
                    .token
                    .ok_or_else(|| anyhow::anyhow!("WeChat login confirmed without token"))?;
                let token = setup::resolve_wechat_token(&token)?;
                let base_url = status
                    .base_url
                    .as_deref()
                    .unwrap_or(base_url)
                    .trim_end_matches('/');
                let base_url = setup::resolve_wechat_base_url(base_url)?;
                println!("{}", style(i18n.text(Text::WechatQrConfirmed)).green());
                return Ok(WechatLogin {
                    token,
                    base_url,
                    scanned_user_id: status.scanned_user_id,
                });
            }
            _ => {
                sleep(Duration::from_secs(1)).await;
            }
        }
    }

    anyhow::bail!("timed out waiting for WeChat QR login")
}

pub(in crate::cli) async fn link_wechat_recipient(
    base_url: &str,
    token: &str,
    route_tag: Option<&str>,
    expected_user_id: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<setup::WechatRecipientLink> {
    println!();
    println!(
        "{}",
        style(i18n.text(Text::WechatActionHeading)).yellow().bold()
    );
    println!(
        "1. {} {}",
        i18n.text(Text::WechatActionStep1),
        style("WeixinClawBot").cyan().bold()
    );
    println!(
        "2. {} {}",
        i18n.text(Text::WechatActionStep2),
        style("hi").green().bold()
    );
    println!(
        "{}",
        style(i18n.text(Text::WechatActionDoNotType)).red().bold()
    );
    wait_for_enter(
        &style(i18n.text(Text::WechatActionDone))
            .green()
            .bold()
            .to_string(),
    )?;
    println!("{}", i18n.text(Text::WechatWaitingForMessage));
    setup::poll_wechat_recipient_link(
        base_url,
        token,
        route_tag,
        expected_user_id,
        setup::DEFAULT_WECHAT_LINK_TIMEOUT,
    )
    .await
}

pub(in crate::cli) fn print_wechat_qr_code(qr_content: &str, i18n: I18n) -> anyhow::Result<()> {
    println!();
    println!("{}", i18n.text(Text::WechatQrScanPrompt));
    let code = QrCode::new(qr_content.as_bytes()).context("failed to render WeChat QR code")?;
    let image = code.render::<unicode::Dense1x2>().quiet_zone(true).build();
    println!("{image}");
    println!("QR content: {qr_content}");
    println!();
    Ok(())
}

pub(in crate::cli) fn prompt_for_microsoft_teams_webhook_url(
    current_url: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_url) = current_url {
            localized_string(
                i18n,
                format!(
                    "Microsoft Teams webhook URL [configured: {}, edit or press Enter to keep]",
                    safe_url_host(current_url)
                ),
                format!(
                    "Microsoft Teams webhook URL [已配置：{}，修改或按 Enter 保留]",
                    safe_url_host(current_url)
                ),
            )
        } else {
            "Microsoft Teams webhook URL".to_string()
        };
        let input = prompt_private_text(
            prompt,
            current_url,
            "failed to read Microsoft Teams webhook URL",
        )?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_url.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_microsoft_teams_webhook_url(candidate) {
            Ok(url) => return Ok(url),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_host(
    current_host: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_host) = current_host {
            localized_string(
                i18n,
                format!("SMTP host [{current_host}, press Enter to keep]"),
                format!("SMTP host [{current_host}，按 Enter 保留]"),
            )
        } else {
            localized(
                i18n,
                "SMTP host, for example smtp.gmail.com",
                "SMTP host，例如 smtp.gmail.com",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read SMTP host")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_host.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_email_smtp_host(candidate) {
            Ok(host) => return Ok(host),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_security(
    current_security: Option<EmailSmtpSecurity>,
    i18n: I18n,
) -> anyhow::Result<EmailSmtpSecurity> {
    let effective_default = current_security.unwrap_or(EmailSmtpSecurity::Starttls);
    let options = [EmailSmtpSecurity::Starttls, EmailSmtpSecurity::ImplicitTls];
    let default_index = options
        .iter()
        .position(|security| *security == effective_default)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|security| {
            let mut label = email_smtp_security_display(*security).to_string();
            if Some(*security) == current_security {
                label.push_str(&format!(" ({})", i18n.text(Text::CurrentSuffix)));
            } else if current_security.is_none() && *security == EmailSmtpSecurity::Starttls {
                label.push_str(&format!(" ({})", i18n.text(Text::RecommendedSuffix)));
            }
            label
        })
        .collect::<Vec<_>>();
    let theme = prompt_theme();
    let selection = Select::with_theme(&theme)
        .with_prompt(localized(i18n, "SMTP security", "SMTP 加密方式"))
        .items(&items)
        .default(default_index)
        .interact()
        .context("failed to read SMTP security")?;

    Ok(options[selection])
}

pub(in crate::cli) fn prompt_for_email_smtp_port(
    default_port: u16,
    i18n: I18n,
) -> anyhow::Result<u16> {
    loop {
        let input = prompt_text(
            localized_string(
                i18n,
                format!("SMTP port [{default_port}, press Enter to keep]"),
                format!("SMTP port [{default_port}，按 Enter 保留]"),
            ),
            "failed to read SMTP port",
        )?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            default_port.to_string()
        } else {
            input.to_string()
        };

        match setup::resolve_email_smtp_port(&candidate) {
            Ok(port) => return Ok(port),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_username(
    current_username: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<Option<String>> {
    loop {
        let prompt = if let Some(current_username) = current_username {
            localized_string(
                i18n,
                format!(
                    "SMTP username [{current_username}, press Enter to keep, type `none` for no auth]"
                ),
                format!("SMTP username [{current_username}，按 Enter 保留，输入 `none` 关闭 auth]"),
            )
        } else {
            localized(
                i18n,
                "SMTP username, or press Enter for no auth",
                "SMTP username。没有 auth 就按 Enter",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read SMTP username")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_username.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_email_smtp_optional_username(candidate) {
            Ok(username) => return Ok(username),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_password(
    current_password: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if current_password.is_some() {
            localized(
                i18n,
                "SMTP password [configured, press Enter to keep]",
                "SMTP password [已配置，按 Enter 保留]",
            )
        } else {
            localized(
                i18n,
                "SMTP password or SMTP API key",
                "SMTP password 或 SMTP API key",
            )
        };
        let input = prompt_secret(prompt, "failed to read SMTP password")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_password.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_email_smtp_password(candidate) {
            Ok(password) => return Ok(password),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_mailbox(
    label: &'static str,
    current_mailbox: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if let Some(current_mailbox) = current_mailbox {
            localized_string(
                i18n,
                format!("{label} [{current_mailbox}, press Enter to keep]"),
                format!("{label} [{current_mailbox}，按 Enter 保留]"),
            )
        } else {
            localized_string(
                i18n,
                format!("{label}, for example Agents Router <alerts@example.com>"),
                format!("{label}，例如 Agents Router <alerts@example.com>"),
            )
        };
        let input = prompt_text(prompt, format!("failed to read {label}"))?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_mailbox.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_email_smtp_mailbox(candidate, label) {
            Ok(mailbox) => return Ok(mailbox),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_optional_mailbox(
    label: &'static str,
    current_mailbox: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<Option<String>> {
    loop {
        let prompt = if let Some(current_mailbox) = current_mailbox {
            localized_string(
                i18n,
                format!("{label} [{current_mailbox}, press Enter to keep, type `none` to clear]"),
                format!("{label} [{current_mailbox}，按 Enter 保留，输入 `none` 清除]"),
            )
        } else {
            localized_string(
                i18n,
                format!("{label}, or press Enter to skip"),
                format!("{label}，跳过就按 Enter"),
            )
        };
        let input = prompt_text(prompt, format!("failed to read {label}"))?;

        let input = input.trim();
        if input.is_empty() {
            return Ok(current_mailbox.map(ToOwned::to_owned));
        }
        if input.eq_ignore_ascii_case("none") {
            return Ok(None);
        }

        match setup::resolve_email_smtp_mailbox(input, label) {
            Ok(mailbox) => return Ok(Some(mailbox)),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_email_smtp_recipients(
    current_recipients: Option<&[String]>,
    i18n: I18n,
) -> anyhow::Result<Vec<String>> {
    loop {
        let prompt = if let Some(current_recipients) = current_recipients {
            localized_string(
                i18n,
                format!(
                    "Recipient emails [{}; press Enter to keep]",
                    current_recipients.join(", ")
                ),
                format!(
                    "收件人邮箱 [{}；按 Enter 保留]",
                    current_recipients.join(", ")
                ),
            )
        } else {
            localized(
                i18n,
                "Recipient emails, comma-separated",
                "收件人邮箱，多个用英文逗号分隔",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read recipient emails")?;

        let input = input.trim();
        if input.is_empty()
            && let Some(current_recipients) = current_recipients
        {
            return Ok(current_recipients.to_vec());
        }

        match setup::resolve_email_smtp_recipients(input) {
            Ok(recipients) => return Ok(recipients),
            Err(error) => println!("{error}"),
        }
    }
}

pub(in crate::cli) fn prompt_for_pushover_app_token(
    current_token: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if current_token.is_some() {
            localized(
                i18n,
                "Pushover application API token [configured, press Enter to keep]",
                "Pushover application API token [已配置，按 Enter 保留]",
            )
        } else {
            "Pushover application API token"
        };
        let input = prompt_secret(prompt, "failed to read Pushover application API token")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_token.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_pushover_app_token(candidate) {
            Ok(token) => return Ok(token),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_pushover_user_key(
    current_user_key: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<String> {
    loop {
        let prompt = if current_user_key.is_some() {
            localized(
                i18n,
                "Pushover user or group key [configured, press Enter to keep]",
                "Pushover user 或 group key [已配置，按 Enter 保留]",
            )
        } else {
            "Pushover user or group key"
        };
        let input = prompt_secret(prompt, "failed to read Pushover user or group key")?;

        let input = input.trim();
        let candidate = if input.is_empty() {
            current_user_key.unwrap_or(input)
        } else {
            input
        };

        match setup::resolve_pushover_user_key(candidate) {
            Ok(user_key) => return Ok(user_key),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_pushover_device(
    current_device: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<Option<String>> {
    loop {
        let prompt = if let Some(current_device) = current_device {
            localized_string(
                i18n,
                format!(
                    "Pushover device [{current_device}, press Enter to keep, type `none` to clear]"
                ),
                format!("Pushover device [{current_device}，按 Enter 保留，输入 `none` 清除]"),
            )
        } else {
            localized(
                i18n,
                "Pushover device, or press Enter to send to all devices",
                "Pushover device。发给所有设备就按 Enter",
            )
            .to_string()
        };
        let input = prompt_text(prompt, "failed to read Pushover device")?;

        let input = input.trim();
        if input.is_empty() {
            return Ok(current_device.map(ToOwned::to_owned));
        }
        if input.eq_ignore_ascii_case("none") {
            return Ok(None);
        }

        match setup::resolve_pushover_device(input) {
            Ok(device) => return Ok(device),
            Err(error) => {
                println!("{error}");
            }
        }
    }
}

pub(in crate::cli) fn prompt_for_pushover_sound(
    current_sound: Option<&str>,
    i18n: I18n,
) -> anyhow::Result<Option<String>> {
    let prompt = if let Some(current_sound) = current_sound {
        localized_string(
            i18n,
            format!("Pushover sound [{current_sound}, press Enter to keep, type `none` to clear]"),
            format!("Pushover sound [{current_sound}，按 Enter 保留，输入 `none` 清除]"),
        )
    } else {
        localized(
            i18n,
            "Pushover sound, or press Enter to use your account default",
            "Pushover sound。使用账号默认声音就按 Enter",
        )
        .to_string()
    };
    let input = prompt_text(prompt, "failed to read Pushover sound")?;

    let input = input.trim();
    if input.is_empty() {
        return Ok(current_sound.map(ToOwned::to_owned));
    }
    if input.eq_ignore_ascii_case("none") {
        return Ok(None);
    }

    Ok(setup::resolve_pushover_sound(input))
}
