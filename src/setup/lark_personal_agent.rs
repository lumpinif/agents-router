use std::time::Duration;

use anyhow::{Context, ensure};
use reqwest::Url;
use serde::Deserialize;

use crate::config::FeishuLarkAppDomain;
use crate::lark_personal_agent_channel;

const REGISTRATION_ENDPOINT: &str = "/oauth/v1/app/registration";
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;
const DEFAULT_EXPIRES_IN_SECONDS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LarkPersonalAgentRegistrationBegin {
    pub verification_url: String,
    pub device_code: String,
    pub expires_in: Duration,
    pub poll_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LarkPersonalAgentRegistrationResult {
    pub app_id: String,
    pub app_secret: String,
    pub domain: FeishuLarkAppDomain,
    pub operator_open_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LarkPersonalAgentRegistrationPoll {
    Pending,
    SlowDown,
    DomainSwitched,
    Complete(LarkPersonalAgentRegistrationResult),
}

pub async fn begin_lark_personal_agent_registration(
    domain: FeishuLarkAppDomain,
) -> anyhow::Result<LarkPersonalAgentRegistrationBegin> {
    let response = request_registration(
        domain,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;

    let verification_uri_complete = required_response_field(
        "verification_uri_complete",
        response.verification_uri_complete,
    )?;
    let device_code = required_response_field("device_code", response.device_code)?;
    let verification_url = lark_personal_agent_verification_url(&verification_uri_complete)?;

    Ok(LarkPersonalAgentRegistrationBegin {
        verification_url: verification_url.to_string(),
        device_code,
        expires_in: Duration::from_secs(
            response
                .expires_in
                .unwrap_or(DEFAULT_EXPIRES_IN_SECONDS)
                .max(1),
        ),
        poll_interval: Duration::from_secs(
            response
                .interval
                .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS)
                .max(1),
        ),
    })
}

pub async fn poll_lark_personal_agent_registration(
    domain: FeishuLarkAppDomain,
    device_code: &str,
) -> anyhow::Result<LarkPersonalAgentRegistrationPoll> {
    ensure!(
        !device_code.trim().is_empty(),
        "Lark app registration device_code is required"
    );

    let response =
        request_registration(domain, &[("action", "poll"), ("device_code", device_code)]).await?;

    if let (Some(app_id), Some(app_secret)) = (response.client_id, response.client_secret) {
        let tenant_brand = response
            .user_info
            .as_ref()
            .and_then(|user_info| user_info.tenant_brand.as_deref());
        let domain = match tenant_brand {
            Some("lark") => FeishuLarkAppDomain::Lark,
            _ => domain,
        };
        return Ok(LarkPersonalAgentRegistrationPoll::Complete(
            LarkPersonalAgentRegistrationResult {
                app_id,
                app_secret,
                domain,
                operator_open_id: response.user_info.and_then(|user_info| user_info.open_id),
            },
        ));
    }

    if response
        .user_info
        .as_ref()
        .and_then(|user_info| user_info.tenant_brand.as_deref())
        == Some("lark")
        && domain == FeishuLarkAppDomain::Feishu
    {
        return Ok(LarkPersonalAgentRegistrationPoll::DomainSwitched);
    }

    match response.error.as_deref() {
        Some("authorization_pending") | None => Ok(LarkPersonalAgentRegistrationPoll::Pending),
        Some("slow_down") => Ok(LarkPersonalAgentRegistrationPoll::SlowDown),
        Some(error) => {
            let description = response
                .error_description
                .unwrap_or_else(|| "unknown error".to_string());
            anyhow::bail!("Lark app registration failed: {error}: {description}")
        }
    }
}

async fn request_registration(
    domain: FeishuLarkAppDomain,
    params: &[(&str, &str)],
) -> anyhow::Result<LarkPersonalAgentRegistrationResponse> {
    let response = reqwest::Client::new()
        .post(format!(
            "{}{}",
            registration_base_url(domain),
            REGISTRATION_ENDPOINT
        ))
        .form(params)
        .send()
        .await
        .context("failed to request Lark Personal Agent app registration")?;
    let body = response
        .text()
        .await
        .context("failed to read Lark Personal Agent app registration response")?;

    serde_json::from_str(&body)
        .with_context(|| "Lark Personal Agent app registration returned invalid JSON".to_string())
}

fn lark_personal_agent_verification_url(verification_uri_complete: &str) -> anyhow::Result<String> {
    let mut verification_url = Url::parse(verification_uri_complete)
        .context("Lark app registration returned an invalid verification URL")?;
    verification_url.query_pairs_mut().extend_pairs([
        ("from", "sdk"),
        (
            "source",
            lark_personal_agent_channel::SDK_SOURCE_QUERY_VALUE,
        ),
        ("tp", "sdk"),
        ("name", "Agents Router {user}"),
        (
            "desc",
            "Local-first bridge for Codex, Claude Code, and Lark project rooms.",
        ),
    ]);
    Ok(verification_url.to_string())
}

fn registration_base_url(domain: FeishuLarkAppDomain) -> &'static str {
    match domain {
        FeishuLarkAppDomain::Feishu => "https://accounts.feishu.cn",
        FeishuLarkAppDomain::Lark => "https://accounts.larksuite.com",
    }
}

fn required_response_field(field: &'static str, value: Option<String>) -> anyhow::Result<String> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        anyhow::bail!("Lark app registration response did not include `{field}`");
    };
    Ok(value)
}

#[derive(Debug, Deserialize)]
struct LarkPersonalAgentRegistrationResponse {
    verification_uri_complete: Option<String>,
    device_code: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
    client_id: Option<String>,
    client_secret: Option<String>,
    user_info: Option<LarkPersonalAgentRegistrationUserInfo>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LarkPersonalAgentRegistrationUserInfo {
    open_id: Option<String>,
    tenant_brand: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_url_matches_official_node_sdk_source_format() {
        let url = lark_personal_agent_verification_url(
            "https://open.feishu.cn/page/launcher?user_code=ABCD",
        )
        .expect("verification URL should build");

        let parsed = Url::parse(&url).expect("verification URL should parse");
        let query = parsed.query_pairs().collect::<Vec<_>>();

        assert!(query.iter().any(|(key, value)| key == "source"
            && value == lark_personal_agent_channel::SDK_SOURCE_QUERY_VALUE));
        assert!(
            query
                .iter()
                .any(|(key, value)| key == "from" && value == "sdk")
        );
        assert!(
            query
                .iter()
                .any(|(key, value)| key == "tp" && value == "sdk")
        );
    }
}
