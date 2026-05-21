use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::config::{FeishuLarkCustomBotProviderConfig, FeishuLarkProviderConfig, UrlSource};
use crate::delivery::{DeliveryErrorKind, ProviderDeliveryReceiptStatus, ProviderSendStatus};
use crate::signal::{
    SignalAnswer, SignalAnswerKind, SignalConversation, SignalLifecycle, SignalLifecycleStatus,
    SignalLink, SignalWorkspace,
};

#[test]
fn rejects_invalid_webhook_url_from_env() {
    let _guard = EnvVarGuard::set(
        "AGENTS_ROUTER_TEST_FEISHU_LARK_WEBHOOK_URL",
        "https://example.com/hook",
    );
    let config = ProviderConfig {
        id: "work_chat".to_string(),
        detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::CustomBot(
            FeishuLarkCustomBotProviderConfig {
                url: UrlSource::Env("AGENTS_ROUTER_TEST_FEISHU_LARK_WEBHOOK_URL".to_string()),
                secret: None,
            },
        )),
    };

    let err = FeishuLarkProvider::from_config(&config)
        .expect_err("invalid Feishu/Lark URL env value should fail");

    assert!(err.to_string().contains("webhook URL is invalid"));
}

#[tokio::test]
async fn sends_interactive_card_to_custom_bot_webhook() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/bot/v2/hook/test"))
        .and(body_partial_json(json!({
            "msg_type": "interactive"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {}
        })))
        .mount(&server)
        .await;

    let provider = test_provider(format!("{}/open-apis/bot/v2/hook/test", server.uri()), None);

    let result = provider
        .send(&test_signal())
        .await
        .expect("feishu_lark send should succeed");

    assert_eq!(result.provider_id, "work_chat");
    assert_eq!(result.provider_type, "feishu_lark");
    assert_eq!(result.signal_id, "signal-1");
    assert_eq!(result.status, ProviderSendStatus::Sent);
    assert_eq!(result.http_status, Some(200));

    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    let body: serde_json::Value = requests[0]
        .body_json()
        .expect("request body should be JSON");
    assert_eq!(
        body["card"]["header"]["title"]["content"],
        "Codex · Test Mac"
    );
    assert_eq!(body["card"]["header"]["template"], "purple");
    assert_eq!(body["card"]["config"]["wide_screen_mode"], true);
    let elements = body["card"]["elements"]
        .as_array()
        .expect("card elements should be present");
    assert!(
        elements
            .iter()
            .any(|element| element["content"] == "**Ready for review.**")
    );
}

#[tokio::test]
async fn app_bot_sends_message_and_returns_unverified_candidate_receipt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .and(body_partial_json(json!({
            "app_id": "cli_test",
            "app_secret": "test-app-secret"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "ok",
            "tenant_access_token": "test-tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .and(query_param("receive_id_type", "chat_id"))
        .and(header("authorization", "Bearer test-tenant-token"))
        .and(body_partial_json(json!({
            "receive_id": "oc_test_chat",
            "msg_type": "interactive"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {
                "message_id": "om_root_message",
                "root_id": "om_root_message",
                "thread_id": "omt_candidate_thread",
                "chat_id": "oc_test_chat",
                "sender": {
                    "id": "cli_test",
                    "id_type": "app_id",
                    "sender_type": "app",
                    "tenant_key": "tenant_test"
                }
            }
        })))
        .mount(&server)
        .await;

    let provider = test_app_bot_provider(server.uri());
    let result = provider
        .send(&test_signal())
        .await
        .expect("App Bot send should succeed");

    assert_eq!(result.provider_id, "work_chat");
    assert_eq!(result.provider_type, "feishu_lark");
    assert_eq!(result.status, ProviderSendStatus::Sent);
    assert_eq!(result.http_status, Some(200));
    assert_eq!(
        result.provider_message_id.as_deref(),
        Some("om_root_message")
    );

    let receipt = result
        .delivery_receipt
        .expect("App Bot should return candidate delivery receipt");
    assert_eq!(receipt.status, ProviderDeliveryReceiptStatus::Candidate);
    assert_eq!(receipt.provider_account_id.as_deref(), Some("tenant_test"));
    assert_eq!(
        receipt.provider_conversation_id.as_deref(),
        Some("oc_test_chat")
    );
    assert_eq!(
        receipt.provider_message_id.as_deref(),
        Some("om_root_message")
    );
    assert_eq!(
        receipt.provider_thread_id, None,
        "root/thread lookup is still unverified and must not create a surface-ready receipt"
    );

    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    let send_request = requests
        .iter()
        .find(|request| request.url.path() == "/open-apis/im/v1/messages")
        .expect("send request should be recorded");
    let body: serde_json::Value = send_request
        .body_json()
        .expect("send request body should be JSON");
    let content = body["content"]
        .as_str()
        .expect("App Bot send content should be a JSON string");
    let card: serde_json::Value =
        serde_json::from_str(content).expect("App Bot card content should be JSON");
    assert_eq!(card["header"]["title"]["content"], "Codex · Test Mac");
    assert_eq!(card["header"]["template"], "purple");
}

#[tokio::test]
async fn app_bot_token_error_does_not_expose_token_or_app_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 99991663,
            "msg": "invalid app credentials",
            "tenant_access_token": "test-tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;

    let provider = test_app_bot_provider(server.uri());
    let err = provider
        .send(&test_signal())
        .await
        .expect_err("token provider error should fail");

    assert_eq!(err.kind, DeliveryErrorKind::ProviderRejected);
    assert_eq!(err.provider_code.as_deref(), Some("99991663"));
    assert!(!err.to_string().contains("test-app-secret"));
    assert!(!err.to_string().contains("test-tenant-token"));
}

#[tokio::test]
async fn app_bot_send_response_tenant_mismatch_fails_without_candidate_receipt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "ok",
            "tenant_access_token": "test-tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {
                "message_id": "om_root_message",
                "chat_id": "oc_test_chat",
                "sender": {
                    "tenant_key": "other_tenant"
                }
            }
        })))
        .mount(&server)
        .await;

    let provider = test_app_bot_provider(server.uri());
    let err = provider
        .send(&test_signal())
        .await
        .expect_err("tenant mismatch should fail");

    assert_eq!(err.kind, DeliveryErrorKind::ProviderResponse);
    assert!(
        err.to_string()
            .contains("tenant_key did not match provider config")
    );
}

#[tokio::test]
async fn sends_signature_fields_when_secret_is_configured() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/bot/v2/hook/test"))
        .and(body_partial_json(json!({
            "msg_type": "interactive",
            "card": {
                "header": {
                    "title": {
                        "content": "Codex · Test Mac"
                    }
                }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {}
        })))
        .mount(&server)
        .await;

    let provider = test_provider(
        format!("{}/open-apis/bot/v2/hook/test", server.uri()),
        Some("demo".to_string()),
    );

    provider
        .send(&test_signal())
        .await
        .expect("feishu_lark send should succeed");

    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    let body: serde_json::Value = requests[0]
        .body_json()
        .expect("request body should be JSON");
    assert!(
        body["timestamp"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(body["sign"].as_str().is_some_and(|value| !value.is_empty()));
}

#[test]
fn signs_request_with_feishu_lark_algorithm() {
    assert_eq!(
        sign_request("100", "demo"),
        "jquNHnVOwmDRfw+vqTIrY5dooJAgi5EcRtLsQE4wfXg="
    );
}

#[test]
fn formats_card_elements_with_supplied_time() {
    assert_eq!(
        format_signal_card_elements_with_time(&test_signal(), "2026-05-08 20:00:00 +08:00"),
        vec![
            FeishuLarkCardElement::ColumnSet {
                flex_mode: "bisect",
                background_style: "default",
                columns: vec![metric_column("Time", "2026-05-08 20:00:00", None)]
            },
            FeishuLarkCardElement::Markdown {
                content: "**Ready for review.**".to_string()
            }
        ]
    );
}

#[test]
fn formats_codex_desktop_message_with_clickable_open_link() {
    let signal = structured_codex_signal(
        Some("agents-router sync report"),
        None,
        Some(SignalAnswer {
            kind: SignalAnswerKind::Preview,
            content: "Ready for review.".to_string(),
        }),
    );

    assert_eq!(
        format_signal_card_elements_with_time(&signal, "2026-05-10 01:35:42 +08:00"),
        vec![
            FeishuLarkCardElement::Div {
                text: FeishuLarkLarkMarkdown {
                    tag: "lark_md",
                    content: "**agents-router sync report**".to_string()
                }
            },
            FeishuLarkCardElement::ColumnSet {
                flex_mode: "bisect",
                background_style: "default",
                columns: vec![
                    metric_column("Project", "agents-router", None),
                    metric_column("Branch", "main", None),
                    metric_column("Model", "gpt-5.2-codex", None)
                ]
            },
            FeishuLarkCardElement::ColumnSet {
                flex_mode: "bisect",
                background_style: "default",
                columns: vec![
                    metric_column("Duration", "1m 32s", None),
                    metric_column("Time", "2026-05-10 01:35:42", None)
                ]
            },
            FeishuLarkCardElement::ColumnSet {
                flex_mode: "bisect",
                background_style: "default",
                columns: vec![metric_column("Session ID", "session-1", None)]
            },
            FeishuLarkCardElement::Action {
                actions: vec![FeishuLarkCardAction {
                    tag: "button",
                    text: FeishuLarkPlainText {
                        tag: "plain_text",
                        content: "Open in Codex".to_string()
                    },
                    url: "http://127.0.0.1:17674/open/codex/thread/session-1".to_string(),
                    button_type: "primary"
                }]
            },
            FeishuLarkCardElement::Divider,
            FeishuLarkCardElement::Markdown {
                content: "**Preview**\nReady for review.".to_string()
            },
        ]
    );
}

#[test]
fn builds_card_body_from_structured_signal() {
    let signal = structured_codex_signal(
        None,
        None,
        Some(SignalAnswer {
            kind: SignalAnswerKind::Preview,
            content: "done".to_string(),
        }),
    );

    assert_eq!(
        FeishuLarkCardBody::from_signal(&signal, "2026-05-10 01:35:42 +08:00"),
        FeishuLarkCardBody {
            project: Some("agents-router".to_string()),
            project_path: Some("/Users/tester/projects/agents-router".to_string()),
            session_title: None,
            session_id: Some("session-1".to_string()),
            model: Some("gpt-5.2-codex".to_string()),
            duration: Some("1m 32s".to_string()),
            branch: Some("main".to_string()),
            time: Some("2026-05-10 01:35:42 +08:00".to_string()),
            other_details: Vec::new(),
            open_link: Some(FeishuLarkActionLink {
                label: "Open in Codex".to_string(),
                url: "http://127.0.0.1:17674/open/codex/thread/session-1".to_string(),
            }),
            prompt: None,
            answer: Some(FeishuLarkAnswerBlock {
                label: "Preview",
                content: "done".to_string(),
            }),
        }
    );
}

#[test]
fn builds_card_body_with_summary_when_simple_signal_only_adds_duration() {
    let mut signal = Signal::new_with_timestamp(
        "signal-1",
        "aider",
        "agent_hook",
        "Aider",
        "Aider finished a task.",
        DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        BTreeMap::new(),
    );
    signal.lifecycle = Some(SignalLifecycle {
        status: Some(SignalLifecycleStatus::Completed),
        started_at: None,
        completed_at: None,
        duration_ms: Some(420_000),
    });

    assert_eq!(
        FeishuLarkCardBody::from_signal(&signal, "2026-05-10 01:35:42 +08:00"),
        FeishuLarkCardBody {
            project: None,
            project_path: None,
            session_title: None,
            session_id: None,
            model: None,
            duration: Some("7m 0s".to_string()),
            branch: None,
            time: Some("2026-05-10 01:35:42 +08:00".to_string()),
            other_details: vec!["**Aider finished a task.**".to_string()],
            open_link: None,
            prompt: None,
            answer: None,
        }
    );
}

#[test]
fn builds_card_body_with_multiline_answer_block() {
    let signal = structured_codex_signal(
        None,
        None,
        Some(SignalAnswer {
            kind: SignalAnswerKind::Full,
            content: "Fixed the route.\n\nRun tests next.".to_string(),
        }),
    );

    assert_eq!(
        FeishuLarkCardBody::from_signal(&signal, "2026-05-10 01:35:42 +08:00")
            .answer
            .expect("answer should render"),
        FeishuLarkAnswerBlock {
            label: "Answer",
            content: "Fixed the route.\n\nRun tests next.".to_string(),
        }
    );
}

#[test]
fn builds_card_body_with_prompt_before_answer() {
    let signal = structured_codex_signal(
        None,
        Some("Fix the route."),
        Some(SignalAnswer {
            kind: SignalAnswerKind::Full,
            content: "Fixed the route.".to_string(),
        }),
    );

    assert_eq!(
        FeishuLarkCardBody::from_signal(&signal, "2026-05-10 01:35:42 +08:00"),
        FeishuLarkCardBody {
            project: Some("agents-router".to_string()),
            project_path: Some("/Users/tester/projects/agents-router".to_string()),
            session_title: None,
            session_id: Some("session-1".to_string()),
            model: Some("gpt-5.2-codex".to_string()),
            duration: Some("1m 32s".to_string()),
            branch: Some("main".to_string()),
            time: Some("2026-05-10 01:35:42 +08:00".to_string()),
            other_details: Vec::new(),
            open_link: Some(FeishuLarkActionLink {
                label: "Open in Codex".to_string(),
                url: "http://127.0.0.1:17674/open/codex/thread/session-1".to_string(),
            }),
            prompt: Some("Fix the route.".to_string()),
            answer: Some(FeishuLarkAnswerBlock {
                label: "Answer",
                content: "Fixed the route.".to_string(),
            }),
        }
    );
}

#[test]
fn formats_prompt_before_answer() {
    let signal = structured_codex_signal(
        None,
        Some("Fix the route."),
        Some(SignalAnswer {
            kind: SignalAnswerKind::Full,
            content: "Fixed the route.".to_string(),
        }),
    );

    let elements = format_signal_card_elements_with_time(&signal, "2026-05-10 01:35:42 +08:00");

    let prompt_index = elements
        .iter()
        .position(|element| {
            matches!(
                element,
                FeishuLarkCardElement::Markdown { content }
                    if content == "**Prompt**\nFix the route."
            )
        })
        .expect("prompt should be rendered");
    let answer_index = elements
        .iter()
        .position(|element| {
            matches!(
                element,
                FeishuLarkCardElement::Markdown { content }
                    if content == "**Answer**\nFixed the route."
            )
        })
        .expect("answer should be rendered");

    assert!(prompt_index < answer_index);
}

#[test]
fn serializes_card_with_header_action_and_preview() {
    let signal = structured_codex_signal(
        None,
        None,
        Some(SignalAnswer {
            kind: SignalAnswerKind::Preview,
            content: "done".to_string(),
        }),
    );

    let request = FeishuLarkInteractiveRequest::from_signal(&signal, None, "Test Mac");
    let body = serde_json::to_value(request).expect("request should serialize");

    assert_eq!(body["msg_type"], "interactive");
    assert_eq!(
        body["card"]["header"]["title"]["content"],
        "Codex Desktop · agents-router · Test Mac"
    );
    assert_eq!(body["card"]["header"]["template"], "purple");
    assert_eq!(
        body["card"]["elements"][4]["actions"][0]["url"],
        "http://127.0.0.1:17674/open/codex/thread/session-1"
    );
    assert_eq!(
        body["card"]["elements"][5],
        json!({
            "tag": "hr"
        })
    );
    assert_eq!(
        body["card"]["elements"][6],
        json!({
            "tag": "markdown",
            "content": "**Preview**\ndone"
        })
    );
}

#[test]
fn uses_later_link_when_primary_link_cannot_be_rendered_as_button() {
    let mut signal = structured_codex_signal(None, None, None);
    signal.links = vec![
        SignalLink {
            label: "Broken Codex Link".to_string(),
            url: "codex://threads/".to_string(),
        },
        SignalLink {
            label: "Open Logs".to_string(),
            url: "https://example.com/logs".to_string(),
        },
    ];

    let body = FeishuLarkCardBody::from_signal(&signal, "2026-05-10 01:35:42 +08:00");

    assert_eq!(
        body.open_link,
        Some(FeishuLarkActionLink {
            label: "Open Logs".to_string(),
            url: "https://example.com/logs".to_string(),
        })
    );
}

#[test]
fn formats_codex_desktop_header_with_machine_source_and_session() {
    let signal = structured_codex_signal(Some("为 Codex 桌面转发加 deeplink"), None, None);

    let request = FeishuLarkInteractiveRequest::from_signal(&signal, None, "Felix’s MacBook Pro");
    let body = serde_json::to_value(request).expect("request should serialize");

    assert_eq!(
        body["card"]["header"]["title"]["content"],
        "Codex Desktop · 为 Codex 桌面转发加 deeplink · Felix’s MacBook Pro"
    );
}

#[test]
fn formats_codex_desktop_session_name_as_card_title_and_session_id_as_metric() {
    let signal = structured_codex_signal(Some("Upgrade Wrangler and typegen"), None, None);

    let request = FeishuLarkInteractiveRequest::from_signal(&signal, None, "Felix’s MacBook Pro");
    let body = serde_json::to_value(request).expect("request should serialize");

    assert_eq!(
        body["card"]["header"]["title"]["content"],
        "Codex Desktop · Upgrade Wrangler and typegen · Felix’s MacBook Pro"
    );
    assert_eq!(
        body["card"]["elements"][0]["text"]["content"],
        "**Upgrade Wrangler and typegen**"
    );
    assert_eq!(
        body["card"]["elements"][3]["columns"][0]["elements"][0]["text"]["content"],
        "**Session ID**\nsession-1"
    );
}

#[test]
fn formats_local_timestamp_with_numeric_offset() {
    let timestamp = DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let formatted = format_local_timestamp(timestamp);

    assert_eq!(formatted.len(), "2026-05-08 20:00:00 +08:00".len());
    assert!(matches!(formatted.as_bytes()[20], b'+' | b'-'));
    assert!(!formatted.contains("UTC"));
}

#[tokio::test]
async fn returns_error_for_non_success_response_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/bot/v2/hook/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 19021,
            "msg": "sign match fail or timestamp is not within one hour from current time"
        })))
        .mount(&server)
        .await;

    let provider = test_provider(
        format!("{}/open-apis/bot/v2/hook/test", server.uri()),
        Some("demo".to_string()),
    );

    let err = provider
        .send(&test_signal())
        .await
        .expect_err("nonzero provider code should fail");

    assert!(err.to_string().contains("returned code 19021"));
    assert!(err.to_string().contains("sign match fail"));
    assert_eq!(err.kind, DeliveryErrorKind::ProviderRejected);
    assert_eq!(err.context.signal_id, "signal-1");
    assert_eq!(err.context.provider_id.as_deref(), Some("work_chat"));
    assert_eq!(err.context.provider_type.as_deref(), Some("feishu_lark"));
    assert_eq!(err.http_status, Some(200));
    assert_eq!(err.provider_code.as_deref(), Some("19021"));
    assert!(!err.retriable);
}

fn test_signal() -> Signal {
    let timestamp = DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    Signal::new_with_timestamp(
        "signal-1",
        "codex_desktop",
        "codex_desktop",
        "Codex",
        "Ready for review.",
        timestamp,
        BTreeMap::new(),
    )
}

fn structured_codex_signal(
    session_title: Option<&str>,
    prompt: Option<&str>,
    answer: Option<SignalAnswer>,
) -> Signal {
    let timestamp = DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut signal = Signal::new_with_timestamp(
        "signal-1",
        "codex_desktop",
        "codex_desktop",
        "Codex Desktop",
        "Codex Desktop finished a task.",
        timestamp,
        BTreeMap::new(),
    );

    signal.workspace = Some(SignalWorkspace {
        cwd: Some("/Users/tester/projects/agents-router".to_string()),
        project_name: Some("agents-router".to_string()),
        project_path: Some("/Users/tester/projects/agents-router".to_string()),
        branch: Some("main".to_string()),
        worktree: None,
    });
    signal.conversation = Some(SignalConversation {
        session_id: Some("session-1".to_string()),
        session_title: session_title.map(ToOwned::to_owned),
        turn_id: Some("turn-1".to_string()),
        prompt: prompt.map(ToOwned::to_owned),
        answer,
        model: Some("gpt-5.2-codex".to_string()),
    });
    signal.lifecycle = Some(SignalLifecycle {
        status: Some(SignalLifecycleStatus::Completed),
        started_at: None,
        completed_at: Some(timestamp),
        duration_ms: Some(92_185),
    });
    signal.links = vec![SignalLink {
        label: "Open in Codex".to_string(),
        url: "codex://threads/session-1".to_string(),
    }];

    signal
}

fn test_provider(url: String, secret: Option<String>) -> FeishuLarkProvider {
    FeishuLarkProvider {
        id: "work_chat".to_string(),
        runtime: FeishuLarkProviderRuntime::CustomBot(FeishuLarkCustomBotRuntime {
            url,
            secret,
            computer_name: "Test Mac".to_string(),
        }),
        client: reqwest::Client::new(),
    }
}

fn test_app_bot_provider(api_base_url: String) -> FeishuLarkProvider {
    FeishuLarkProvider {
        id: "work_chat".to_string(),
        runtime: FeishuLarkProviderRuntime::AppBot(FeishuLarkAppBotRuntime {
            api_base_url,
            app_id: "cli_test".to_string(),
            app_secret: "test-app-secret".to_string(),
            tenant_key: "tenant_test".to_string(),
            chat_id: "oc_test_chat".to_string(),
            computer_name: "Test Mac".to_string(),
        }),
        client: reqwest::Client::new(),
    }
}

struct EnvVarGuard {
    name: &'static str,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: &str) -> Self {
        // SAFETY: this test uses a unique env var name scoped to this module.
        unsafe { std::env::set_var(name, value) };
        Self { name }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: this removes only the unique env var set by the test guard.
        unsafe { std::env::remove_var(self.name) };
    }
}
