use super::*;

pub(in crate::cli) fn print_heading(title: &str) {
    println!("{}", style(title).bold());
}

pub(in crate::cli) fn print_section(title: &str) {
    println!();
    print_heading(title);
}

pub(in crate::cli) fn print_field(label: &str, value: impl Display) {
    println!("  {} {}", style(format!("{label}:")).dim(), value);
}

pub(in crate::cli) fn print_success_field(label: &str, value: impl Display) {
    print_field(label, style(value.to_string()).green());
}

pub(in crate::cli) fn print_path_field(label: &str, value: impl Display) {
    print_field(label, style(value.to_string()).cyan());
}

pub(in crate::cli) fn print_bool_field(label: &str, value: bool) {
    if value {
        print_success_field(label, "yes");
    } else {
        print_field(label, style("no").yellow());
    }
}

pub(in crate::cli) fn print_setup_summary(
    mode: ConfigWriteMode,
    path: &Path,
    agent: setup::AgentIntegrationId,
    answer_detail: AnswerDetail,
    prompt_detail: PromptDetail,
    route_filters: &SetupRouteFilters,
    i18n: I18n,
) {
    println!(
        "{} config: {}",
        style(mode.past_tense(i18n)).green(),
        style(path.display()).cyan()
    );
    println!(
        "{}: {}",
        i18n.text(Text::AgentField),
        style(agent.display_name()).green()
    );
    println!(
        "{}: {}",
        i18n.text(Text::AnswerDetailField),
        style(i18n.answer_detail(answer_detail)).green()
    );
    println!(
        "{}: {}",
        i18n.text(Text::PromptDetailField),
        style(i18n.prompt_detail(prompt_detail)).green()
    );
    println!(
        "{}: {}",
        i18n.text(Text::NotificationPreferenceField),
        style(notification_preference_summary(
            route_filters.minimum_task_duration_minutes,
            i18n
        ))
        .green()
    );
    if !route_filters.only_forward_from_project_paths.is_empty() {
        println!(
            "{}: {}",
            i18n.text(Text::ProjectFilterField),
            style(project_filter_summary(
                route_filters.only_forward_from_project_paths.len(),
                i18n
            ))
            .green()
        );
    }
}

pub(in crate::cli) fn notification_preference_summary(
    minimum_task_duration_minutes: Option<u64>,
    i18n: I18n,
) -> String {
    match minimum_task_duration_minutes {
        None => localized(i18n, "Every completed task", "每个完成的任务").to_string(),
        Some(minutes) => localized_string(
            i18n,
            format!("Tasks {minutes} minutes or longer"),
            format!("{minutes} 分钟或更久的任务"),
        ),
    }
}

pub(in crate::cli) fn project_filter_summary(count: usize, i18n: I18n) -> String {
    localized_string(
        i18n,
        format!("{count} project path{}", if count == 1 { "" } else { "s" }),
        format!("{count} 个项目路径"),
    )
}

pub(in crate::cli) fn print_provider_configured(provider: &str, i18n: I18n) {
    let provider = localized_provider_name(provider, i18n);
    println!("{provider}: {}", style(i18n.text(Text::Configured)).green());
}

fn localized_provider_name(provider: &str, i18n: I18n) -> &str {
    if provider == "WeChat" && i18n.language() == CliLanguage::SimplifiedChinese {
        "微信"
    } else {
        provider
    }
}

pub(in crate::cli) fn print_setup_provider_summary(config: &RawConfig, i18n: I18n) {
    for summary in setup_provider_summaries(config) {
        print_provider_configured(summary.provider_name, i18n);
        for field in &summary.fields {
            print_setup_provider_summary_field(field, i18n);
        }
    }
}

pub(in crate::cli) fn print_setup_provider_summary_field(
    field: &SetupProviderSummaryField,
    i18n: I18n,
) {
    let value = localized_summary_value(&field.value, i18n);
    match field.tone {
        SetupProviderSummaryTone::Plain => print_field(field.label, value),
        SetupProviderSummaryTone::Success => print_success_field(field.label, value),
        SetupProviderSummaryTone::Warning => print_field(field.label, style(value).yellow()),
    }
}

pub(in crate::cli) fn localized_summary_value(value: &str, i18n: I18n) -> &str {
    if i18n.language() == CliLanguage::English {
        return value;
    }

    match value {
        "configured" => i18n.text(Text::Configured),
        "not configured" => i18n.text(Text::NotConfigured),
        "not used" => i18n.text(Text::NotUsed),
        "all devices" => i18n.text(Text::AllDevices),
        "account default" => i18n.text(Text::AccountDefault),
        _ => value,
    }
}

pub(in crate::cli) fn setup_provider_summaries(config: &RawConfig) -> Vec<SetupProviderSummary> {
    config
        .providers
        .iter()
        .map(setup_provider_summary)
        .collect()
}

pub(in crate::cli) fn setup_provider_summary(provider: &RawProviderConfig) -> SetupProviderSummary {
    match &provider.provider_type {
        ProviderType::Ntfy => SetupProviderSummary {
            provider_name: "ntfy",
            fields: vec![
                plain_summary_field("server", required_summary_string(provider, "server")),
                plain_summary_field("topic", required_summary_string(provider, "topic")),
            ],
        },
        ProviderType::FeishuLark => feishu_lark_setup_provider_summary(provider),
        ProviderType::Webhook => SetupProviderSummary {
            provider_name: "Webhook",
            fields: vec![plain_summary_field(
                "endpoint",
                provider_url_summary(provider),
            )],
        },
        ProviderType::Pushover => SetupProviderSummary {
            provider_name: "Pushover",
            fields: vec![
                configured_or_not_summary_field(
                    "application token",
                    provider_secret_configured(&provider.app_token, &provider.app_token_env),
                ),
                configured_or_not_summary_field(
                    "user or group key",
                    provider_secret_configured(&provider.user_key, &provider.user_key_env),
                ),
                plain_summary_field(
                    "device",
                    provider
                        .device
                        .as_deref()
                        .unwrap_or("all devices")
                        .to_string(),
                ),
                plain_summary_field(
                    "sound",
                    provider
                        .sound
                        .as_deref()
                        .unwrap_or("account default")
                        .to_string(),
                ),
            ],
        },
        ProviderType::Slack => SetupProviderSummary {
            provider_name: "Slack",
            fields: vec![plain_summary_field(
                "webhook",
                provider_url_summary(provider),
            )],
        },
        ProviderType::Discord => SetupProviderSummary {
            provider_name: "Discord",
            fields: vec![plain_summary_field(
                "webhook",
                provider_url_summary(provider),
            )],
        },
        ProviderType::Telegram => SetupProviderSummary {
            provider_name: "Telegram",
            fields: vec![
                configured_or_not_summary_field(
                    "bot token",
                    provider_secret_configured(&provider.bot_token, &provider.bot_token_env),
                ),
                plain_summary_field("chat id", required_summary_string(provider, "chat_id")),
            ],
        },
        ProviderType::Whatsapp => SetupProviderSummary {
            provider_name: "WhatsApp",
            fields: vec![
                configured_or_not_summary_field(
                    "access token",
                    provider_secret_configured(&provider.access_token, &provider.access_token_env),
                ),
                plain_summary_field(
                    "phone number id",
                    required_summary_string(provider, "phone_number_id"),
                ),
                plain_summary_field(
                    "recipient",
                    required_summary_string(provider, "recipient_phone_number"),
                ),
            ],
        },
        ProviderType::Wechat => SetupProviderSummary {
            provider_name: "WeChat",
            fields: vec![
                configured_or_not_summary_field(
                    "iLink token",
                    provider_secret_configured(&provider.token, &provider.token_env),
                ),
                configured_or_not_summary_field(
                    "context token",
                    provider_secret_configured(
                        &provider.context_token,
                        &provider.context_token_env,
                    ),
                ),
                plain_summary_field("endpoint", required_summary_host(provider, "base_url")),
                plain_summary_field(
                    "recipient",
                    required_summary_string(provider, "recipient_user_id"),
                ),
            ],
        },
        ProviderType::MicrosoftTeams => SetupProviderSummary {
            provider_name: "Microsoft Teams",
            fields: vec![plain_summary_field(
                "webhook",
                provider_url_summary(provider),
            )],
        },
        ProviderType::EmailSmtp => {
            let mut fields = vec![
                plain_summary_field("server", email_smtp_server_summary(provider)),
                plain_summary_field("security", email_smtp_security_summary(provider)),
                email_smtp_authentication_summary_field(provider),
                plain_summary_field("from", required_summary_string(provider, "from")),
                plain_summary_field("to", required_summary_recipients(provider)),
            ];

            if let Some(reply_to) = present_summary_str(provider.reply_to.as_deref()) {
                fields.push(plain_summary_field("reply-to", reply_to.to_string()));
            }

            SetupProviderSummary {
                provider_name: "Email SMTP",
                fields,
            }
        }
    }
}

pub(in crate::cli) fn plain_summary_field(
    label: &'static str,
    value: String,
) -> SetupProviderSummaryField {
    SetupProviderSummaryField {
        label,
        value,
        tone: SetupProviderSummaryTone::Plain,
    }
}

fn feishu_lark_setup_provider_summary(provider: &RawProviderConfig) -> SetupProviderSummary {
    if provider.mode.as_deref() == Some("app_bot") {
        return SetupProviderSummary {
            provider_name: feishu_lark_app_provider_summary_name(provider),
            fields: {
                let mut fields = vec![
                    plain_summary_field("domain", required_summary_string(provider, "domain")),
                    plain_summary_field("app id", required_summary_string(provider, "app_id")),
                    configured_or_not_summary_field(
                        "app secret",
                        provider_secret_configured(&provider.app_secret, &provider.app_secret_env),
                    ),
                ];
                fields.push(match present_summary_str(provider.chat_id.as_deref()) {
                    Some(chat_id) => plain_summary_field("default room", chat_id.to_string()),
                    None => plain_summary_field(
                        "default room",
                        "no global room; use direct chat or project rooms".to_string(),
                    ),
                });
                if present_summary_str(provider.chat_id.as_deref()).is_none() {
                    fields.push(personal_agent_room_events_summary_field(
                        provider.app_registration_source.as_deref(),
                    ));
                }
                if let Some(tenant_key) = present_summary_str(provider.tenant_key.as_deref()) {
                    fields.push(plain_summary_field("tenant key", tenant_key.to_string()));
                }
                fields
            },
        };
    }

    SetupProviderSummary {
        provider_name: "Feishu/Lark custom bot",
        fields: vec![
            plain_summary_field("webhook", provider_url_summary(provider)),
            configured_or_not_summary_field(
                "signature verification",
                provider_secret_configured(&provider.secret, &provider.secret_env),
            ),
        ],
    }
}

fn personal_agent_room_events_summary_field(source: Option<&str>) -> SetupProviderSummaryField {
    match lark_personal_agent_channel::registration_source_status(source) {
        lark_personal_agent_channel::RegistrationSourceStatus::Current => {
            SetupProviderSummaryField {
                label: "room events",
                value: "configured".to_string(),
                tone: SetupProviderSummaryTone::Success,
            }
        }
        lark_personal_agent_channel::RegistrationSourceStatus::Missing => {
            SetupProviderSummaryField {
                label: "room events",
                value: "configured; run a direct chat or room smoke".to_string(),
                tone: SetupProviderSummaryTone::Warning,
            }
        }
        lark_personal_agent_channel::RegistrationSourceStatus::Mismatch => {
            SetupProviderSummaryField {
                label: "room events",
                value: "configured; source marker differs from this build".to_string(),
                tone: SetupProviderSummaryTone::Warning,
            }
        }
    }
}

fn feishu_lark_app_provider_summary_name(provider: &RawProviderConfig) -> &'static str {
    if present_summary_str(provider.chat_id.as_deref()).is_some() {
        "Feishu/Lark App Bot"
    } else {
        "Feishu/Lark Personal Agent"
    }
}

pub(in crate::cli) fn configured_or_not_summary_field(
    label: &'static str,
    configured: bool,
) -> SetupProviderSummaryField {
    SetupProviderSummaryField {
        label,
        value: if configured {
            "configured".to_string()
        } else {
            "not configured".to_string()
        },
        tone: if configured {
            SetupProviderSummaryTone::Success
        } else {
            SetupProviderSummaryTone::Warning
        },
    }
}

pub(in crate::cli) fn provider_secret_configured(
    inline: &Option<String>,
    env: &Option<String>,
) -> bool {
    present_summary_str(inline.as_deref()).is_some()
        || present_summary_str(env.as_deref()).is_some()
}

pub(in crate::cli) fn provider_url_summary(provider: &RawProviderConfig) -> String {
    if let Some(url) = present_summary_str(provider.url.as_deref()) {
        return safe_url_host(url);
    }
    if let Some(env_name) = present_summary_str(provider.url_env.as_deref()) {
        return format!("env:{env_name}");
    }

    panic!(
        "{} provider `{}` is missing a webhook URL source",
        provider.provider_type.as_str(),
        provider.id
    );
}

pub(in crate::cli) fn required_summary_host(
    provider: &RawProviderConfig,
    field: &'static str,
) -> String {
    safe_url_host(&required_summary_string(provider, field))
}

pub(in crate::cli) fn required_summary_string(
    provider: &RawProviderConfig,
    field: &'static str,
) -> String {
    let value = match field {
        "base_url" => provider.base_url.as_deref(),
        "domain" => provider.domain.as_deref(),
        "server" => provider.server.as_deref(),
        "topic" => provider.topic.as_deref(),
        "app_id" => provider.app_id.as_deref(),
        "tenant_key" => provider.tenant_key.as_deref(),
        "chat_id" => provider.chat_id.as_deref(),
        "phone_number_id" => provider.phone_number_id.as_deref(),
        "recipient_phone_number" => provider.recipient_phone_number.as_deref(),
        "recipient_user_id" => provider.recipient_user_id.as_deref(),
        "from" => provider.from.as_deref(),
        _ => panic!("unsupported setup summary field `{field}`"),
    };

    present_summary_str(value)
        .unwrap_or_else(|| {
            panic!(
                "{} provider `{}` is missing `{field}`",
                provider.provider_type.as_str(),
                provider.id
            )
        })
        .to_string()
}

pub(in crate::cli) fn required_summary_recipients(provider: &RawProviderConfig) -> String {
    provider
        .to
        .as_ref()
        .filter(|recipients| !recipients.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "{} provider `{}` is missing `to`",
                provider.provider_type.as_str(),
                provider.id
            )
        })
        .join(", ")
}

pub(in crate::cli) fn email_smtp_server_summary(provider: &RawProviderConfig) -> String {
    let host = provider.host.as_deref().unwrap_or_else(|| {
        panic!(
            "{} provider `{}` is missing `host`",
            provider.provider_type.as_str(),
            provider.id
        )
    });
    let port = provider.port.unwrap_or_else(|| {
        panic!(
            "{} provider `{}` is missing `port`",
            provider.provider_type.as_str(),
            provider.id
        )
    });

    format!("{host}:{port}")
}

pub(in crate::cli) fn email_smtp_security_summary(provider: &RawProviderConfig) -> String {
    provider
        .security
        .map(email_smtp_security_display)
        .unwrap_or_else(|| {
            panic!(
                "{} provider `{}` is missing `security`",
                provider.provider_type.as_str(),
                provider.id
            )
        })
        .to_string()
}

pub(in crate::cli) fn email_smtp_authentication_summary_field(
    provider: &RawProviderConfig,
) -> SetupProviderSummaryField {
    match (
        provider_secret_configured(&provider.username, &provider.username_env),
        provider_secret_configured(&provider.password, &provider.password_env),
    ) {
        (true, true) => configured_or_not_summary_field("authentication", true),
        (false, false) => plain_summary_field("authentication", "not used".to_string()),
        _ => panic!(
            "{} provider `{}` must configure both username and password, or neither",
            provider.provider_type.as_str(),
            provider.id
        ),
    }
}

pub(in crate::cli) fn present_summary_str(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
