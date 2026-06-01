use super::*;

pub fn generated_ntfy_topic() -> String {
    let id = Uuid::new_v4().simple().to_string();
    format!("agents-router-{}", &id[..12])
}

pub fn resolve_ntfy_topic(input: &str, generated_topic: &str) -> anyhow::Result<String> {
    let topic = input.trim();
    let topic = if topic.is_empty() {
        generated_topic
    } else {
        topic
    };

    if !is_valid_ntfy_topic(topic) {
        anyhow::bail!("topic must only contain letters, numbers, hyphens, and underscores");
    }

    Ok(topic.to_string())
}

pub fn resolve_feishu_lark_webhook_url(input: &str) -> anyhow::Result<String> {
    validate_feishu_lark_webhook_url(input)
}

pub fn resolve_feishu_lark_secret(input: &str) -> Option<String> {
    let secret = input.trim();
    if secret.is_empty() {
        None
    } else {
        Some(secret.to_string())
    }
}

pub fn resolve_feishu_lark_app_domain(input: &str) -> anyhow::Result<String> {
    match input.trim() {
        "feishu" => Ok("feishu".to_string()),
        "lark" => Ok("lark".to_string()),
        _ => anyhow::bail!("Feishu/Lark app domain must be `feishu` or `lark`"),
    }
}

pub fn resolve_feishu_lark_app_id(input: &str) -> anyhow::Result<String> {
    resolve_non_empty_no_whitespace("Feishu/Lark App ID", input)
}

pub fn resolve_feishu_lark_app_secret(input: &str) -> anyhow::Result<String> {
    resolve_non_empty_no_whitespace("Feishu/Lark App Secret", input)
}

pub fn resolve_feishu_lark_tenant_key(input: &str) -> anyhow::Result<String> {
    resolve_non_empty_no_whitespace("Feishu/Lark tenant_key", input)
}

pub fn resolve_feishu_lark_chat_id(input: &str) -> anyhow::Result<String> {
    resolve_non_empty_no_whitespace("Feishu/Lark room chat_id", input)
}

pub fn resolve_webhook_url(input: &str) -> anyhow::Result<String> {
    validate_custom_webhook_url(input)
}

pub fn resolve_slack_webhook_url(input: &str) -> anyhow::Result<String> {
    validate_slack_webhook_url(input)
}

pub fn resolve_discord_webhook_url(input: &str) -> anyhow::Result<String> {
    validate_discord_webhook_url(input)
}

pub fn resolve_telegram_bot_token(input: &str) -> anyhow::Result<String> {
    let token = input.trim();
    let Some((bot_id, secret)) = token.split_once(':') else {
        anyhow::bail!("Telegram bot token must look like `123456:ABC...`");
    };

    let valid = !bot_id.is_empty()
        && bot_id.bytes().all(|byte| byte.is_ascii_digit())
        && !secret.is_empty()
        && secret.bytes().all(|byte| !byte.is_ascii_whitespace());
    if !valid {
        anyhow::bail!("Telegram bot token must look like `123456:ABC...`");
    }

    Ok(token.to_string())
}

pub fn resolve_telegram_chat_id(input: &str) -> anyhow::Result<String> {
    let chat_id = input.trim();
    let valid = if let Some(username) = chat_id.strip_prefix('@') {
        !username.is_empty()
            && username
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    } else {
        let digits = chat_id.strip_prefix('-').unwrap_or(chat_id);
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    };

    if !valid {
        anyhow::bail!("Telegram chat id must be a numeric chat id or an `@channelusername`");
    }

    Ok(chat_id.to_string())
}

pub fn resolve_whatsapp_access_token(input: &str) -> anyhow::Result<String> {
    let access_token = input.trim();
    if access_token.is_empty() || access_token.bytes().any(|byte| byte.is_ascii_whitespace()) {
        anyhow::bail!("WhatsApp access token must be non-empty and must not contain whitespace");
    }

    Ok(access_token.to_string())
}

pub fn resolve_whatsapp_phone_number_id(input: &str) -> anyhow::Result<String> {
    let phone_number_id = input.trim();
    if phone_number_id.is_empty() || !phone_number_id.bytes().all(|byte| byte.is_ascii_digit()) {
        anyhow::bail!("WhatsApp phone number ID must contain only digits");
    }

    Ok(phone_number_id.to_string())
}

pub fn resolve_whatsapp_recipient_phone_number(input: &str) -> anyhow::Result<String> {
    let phone_number = input.trim();
    if !(7..=15).contains(&phone_number.len())
        || !phone_number.bytes().all(|byte| byte.is_ascii_digit())
    {
        anyhow::bail!(
            "WhatsApp recipient phone number must be 7 to 15 digits without spaces or punctuation"
        );
    }

    Ok(phone_number.to_string())
}

pub fn resolve_wechat_base_url(input: &str) -> anyhow::Result<String> {
    let base_url = input.trim().trim_end_matches('/');
    let parsed =
        reqwest::Url::parse(base_url).context("WeChat iLink base URL must be a valid HTTPS URL")?;
    let has_root_path = parsed.path().is_empty() || parsed.path() == "/";
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !has_root_path
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        anyhow::bail!(
            "WeChat iLink base URL must be an HTTPS origin like `https://ilinkai.weixin.qq.com`"
        );
    }

    Ok(base_url.to_string())
}

pub fn resolve_wechat_token(input: &str) -> anyhow::Result<String> {
    resolve_wechat_secret("WeChat iLink token", input)
}

pub fn resolve_wechat_recipient_user_id(input: &str) -> anyhow::Result<String> {
    let recipient_user_id = input.trim();
    if recipient_user_id.is_empty()
        || recipient_user_id
            .bytes()
            .any(|byte| byte.is_ascii_whitespace())
    {
        anyhow::bail!("WeChat recipient user id must be non-empty and must not contain whitespace");
    }

    Ok(recipient_user_id.to_string())
}

pub fn resolve_wechat_context_token(input: &str) -> anyhow::Result<String> {
    resolve_wechat_secret("WeChat context token", input)
}

pub fn resolve_wechat_route_tag(input: &str) -> anyhow::Result<Option<String>> {
    let route_tag = input.trim();
    if route_tag.is_empty() || route_tag.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if route_tag.bytes().any(|byte| byte.is_ascii_whitespace()) {
        anyhow::bail!("WeChat SKRouteTag must not contain whitespace");
    }

    Ok(Some(route_tag.to_string()))
}

fn resolve_wechat_secret(label: &'static str, input: &str) -> anyhow::Result<String> {
    let token = input.trim();
    if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace()) {
        anyhow::bail!("{label} must be non-empty and must not contain whitespace");
    }

    Ok(token.to_string())
}

fn resolve_non_empty_no_whitespace(label: &'static str, input: &str) -> anyhow::Result<String> {
    let value = input.trim();
    if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        anyhow::bail!("{label} must be non-empty and must not contain whitespace");
    }

    Ok(value.to_string())
}

pub fn resolve_microsoft_teams_webhook_url(input: &str) -> anyhow::Result<String> {
    validate_microsoft_teams_webhook_url(input)
}

pub fn resolve_email_smtp_host(input: &str) -> anyhow::Result<String> {
    let host = input.trim();
    if host.is_empty() {
        anyhow::bail!("Email SMTP host is required");
    }
    if host.contains("://")
        || host.contains('/')
        || host.contains('@')
        || host.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        anyhow::bail!("Email SMTP host must be a hostname, not a URL");
    }

    Ok(host.to_string())
}

pub fn resolve_email_smtp_port(input: &str) -> anyhow::Result<u16> {
    let port = input
        .trim()
        .parse::<u16>()
        .context("Email SMTP port must be a number")?;
    if port == 0 {
        anyhow::bail!("Email SMTP port must be greater than 0");
    }

    Ok(port)
}

pub fn resolve_email_smtp_security(input: &str) -> anyhow::Result<EmailSmtpSecurity> {
    match input.trim() {
        "starttls" => Ok(EmailSmtpSecurity::Starttls),
        "implicit_tls" => Ok(EmailSmtpSecurity::ImplicitTls),
        _ => anyhow::bail!("Email SMTP security must be `starttls` or `implicit_tls`"),
    }
}

pub fn resolve_email_smtp_optional_username(input: &str) -> anyhow::Result<Option<String>> {
    let username = input.trim();
    if username.is_empty() || username.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if username.bytes().any(|byte| byte.is_ascii_whitespace()) {
        anyhow::bail!("Email SMTP username must not contain whitespace");
    }

    Ok(Some(username.to_string()))
}

pub fn resolve_email_smtp_password(input: &str) -> anyhow::Result<String> {
    let password = input.trim();
    if password.is_empty() {
        anyhow::bail!("Email SMTP password is required when username is configured");
    }

    Ok(password.to_string())
}

pub fn resolve_email_smtp_mailbox(input: &str, field: &'static str) -> anyhow::Result<String> {
    let mailbox = input.trim();
    if mailbox.is_empty() {
        anyhow::bail!("Email SMTP {field} is required");
    }
    mailbox
        .parse::<lettre::message::Mailbox>()
        .with_context(|| format!("Email SMTP {field} must be a valid email mailbox"))?;

    Ok(mailbox.to_string())
}

pub fn resolve_email_smtp_recipients(input: &str) -> anyhow::Result<Vec<String>> {
    let recipients = input
        .split(',')
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(|recipient| resolve_email_smtp_mailbox(recipient, "recipient"))
        .collect::<anyhow::Result<Vec<_>>>()?;

    if recipients.is_empty() {
        anyhow::bail!("Email SMTP recipients are required");
    }

    Ok(recipients)
}

pub fn resolve_pushover_app_token(input: &str) -> anyhow::Result<String> {
    let token = input.trim();
    if !is_pushover_identifier(token) {
        anyhow::bail!("Pushover app token must be 30 letters or numbers");
    }

    Ok(token.to_string())
}

pub fn resolve_pushover_user_key(input: &str) -> anyhow::Result<String> {
    let user_key = input.trim();
    if !is_pushover_identifier(user_key) {
        anyhow::bail!("Pushover user or group key must be 30 letters or numbers");
    }

    Ok(user_key.to_string())
}

pub fn resolve_pushover_device(input: &str) -> anyhow::Result<Option<String>> {
    let device = input.trim();
    if device.is_empty() {
        return Ok(None);
    }

    let valid = device.split(',').all(|name| {
        !name.is_empty()
            && name.len() <= 25
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    });
    if !valid {
        anyhow::bail!(
            "Pushover device must be one or more comma-separated device names using letters, numbers, hyphens, and underscores"
        );
    }

    Ok(Some(device.to_string()))
}

pub fn resolve_pushover_sound(input: &str) -> Option<String> {
    let sound = input.trim();
    if sound.is_empty() {
        None
    } else {
        Some(sound.to_string())
    }
}
