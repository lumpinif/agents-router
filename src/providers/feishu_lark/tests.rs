use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::agent_controller::{
    AgentControllerAdapter, AgentControllerClosedLoopDecision, AgentControllerClosedLoopOutcome,
    AgentControllerFuture, AgentControllerRequest, AgentControllerRuntime,
    AgentControllerRuntimeSkipReason, AgentControllerSuccess, ProviderThreadReplyAdapter,
    ProviderThreadReplyRequest,
};
use crate::agent_integration_catalog::{
    AgentControllerKind, AgentIntegrationDescriptor, AgentIntegrationId, ContinuationCapability,
    agent_integration_descriptor,
};
use crate::config::{
    CONFIG_SCHEMA_VERSION, CliConfig, FeishuLarkAppBotProviderConfig,
    FeishuLarkCustomBotProviderConfig, FeishuLarkProviderConfig, LogConfig, NotificationConfig,
    RouteConfig, SecretSource, SourceConfig, SourceType, UrlSource, ValidatedConfig,
};
use crate::delivery::{DeliveryErrorKind, ProviderDeliveryReceiptStatus, ProviderSendStatus};
use crate::provider_catalog::{ProviderMode, provider_mode_capability};
use crate::provider_inbound::{
    ProviderInboundNormalizeResult, ProviderInboundReady,
    normalize_feishu_lark_long_connection_surface_reply,
};
use crate::providers::feishu_lark_long_connection::{
    FeishuLarkLongConnectionDecision, FeishuLarkLongConnectionEvent,
    FeishuLarkLongConnectionRuntime,
};
use crate::response_surface_ledger::{
    InboundEventClaimDecision, InboundEventDedupInput, InboundEventRecordDecision,
    NewResponseSurface, ResponseSurfaceLedger,
};
use crate::response_surface_policy::ResponseSurfacePolicySkipReason;
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
async fn app_bot_sends_message_and_returns_surface_ready_root_lookup_receipt() {
    let server = MockServer::start().await;
    let send_response: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/provider_inbound/feishu_lark_app_bot_send_response.json"
    ))
    .expect("send response fixture should be valid JSON");
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
            "receive_id": "oc_5ce6d572455d361153b7xx51da133945",
            "msg_type": "interactive"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(send_response))
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
        Some("om_root_message_id")
    );

    let receipt = result
        .delivery_receipt
        .expect("App Bot should return a delivery receipt");
    assert_eq!(receipt.status, ProviderDeliveryReceiptStatus::SurfaceReady);
    assert_eq!(
        receipt.provider_account_id.as_deref(),
        Some("2ca1d211f64f6438")
    );
    assert_eq!(
        receipt.provider_conversation_id.as_deref(),
        Some("oc_5ce6d572455d361153b7xx51da133945")
    );
    assert_eq!(
        receipt.provider_message_id.as_deref(),
        Some("om_root_message_id")
    );
    assert_eq!(
        receipt.provider_thread_id.as_deref(),
        Some("om_root_message_id"),
        "Feishu/Lark reply events use root_id as the surface lookup key"
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
async fn app_bot_outbound_message_id_matches_inbound_reply_root_lookup_key() {
    let server = MockServer::start().await;
    let send_response: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/provider_inbound/feishu_lark_app_bot_send_response.json"
    ))
    .expect("send response fixture should be valid JSON");
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
        .respond_with(ResponseTemplate::new(200).set_body_json(send_response))
        .mount(&server)
        .await;

    let provider = test_app_bot_provider(server.uri());
    let outbound = provider
        .send(&test_signal())
        .await
        .expect("App Bot send should succeed");
    let outbound_receipt = outbound
        .delivery_receipt
        .expect("App Bot should return a delivery receipt");
    let ProviderInboundNormalizeResult::SurfaceReply(inbound_reply) =
        normalize_feishu_lark_long_connection_surface_reply(
            "work_chat",
            include_bytes!(
                "../../../tests/fixtures/provider_inbound/feishu_lark_app_bot_reply_to_sent_message.json"
            ),
        )
        .expect("reply event should normalize")
    else {
        panic!("reply event should be a surface reply");
    };

    assert_eq!(
        outbound_receipt.status,
        ProviderDeliveryReceiptStatus::SurfaceReady
    );
    assert_eq!(
        outbound_receipt.provider_message_id,
        outbound_receipt.provider_thread_id
    );
    assert_eq!(
        inbound_reply.provider_thread_id,
        outbound_receipt
            .provider_thread_id
            .expect("surface-ready receipt has provider_thread_id")
    );
    assert_eq!(
        inbound_reply.provider_conversation_id,
        outbound_receipt
            .provider_conversation_id
            .expect("surface-ready receipt has provider_conversation_id")
    );
    assert_eq!(
        inbound_reply.provider_account_id,
        outbound_receipt
            .provider_account_id
            .expect("surface-ready receipt has provider_account_id")
    );
}

#[tokio::test]
async fn app_bot_sends_agent_result_text_to_same_root_thread() {
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
        .and(path("/open-apis/im/v1/messages/om_root_message_id/reply"))
        .and(header("authorization", "Bearer test-tenant-token"))
        .and(body_partial_json(json!({
            "msg_type": "text",
            "reply_in_thread": true
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {
                "message_id": "om_result_reply_message_id",
                "root_id": "om_root_message_id",
                "parent_id": "om_root_message_id",
                "thread_id": "omt_result_thread",
                "msg_type": "text"
            }
        })))
        .mount(&server)
        .await;

    let provider = test_app_bot_provider(server.uri());
    let result = provider
        .send_thread_reply(thread_reply_request("agent final result\nwith exact text"))
        .await
        .expect("thread reply should succeed");

    assert_eq!(
        result.provider_reply_message_id.as_deref(),
        Some("om_result_reply_message_id")
    );
    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    let reply_request = requests
        .iter()
        .find(|request| request.url.path() == "/open-apis/im/v1/messages/om_root_message_id/reply")
        .expect("reply request should be recorded");
    let body: serde_json::Value = reply_request
        .body_json()
        .expect("reply body should be JSON");
    assert_eq!(body["reply_in_thread"], true);
    assert_eq!(body["msg_type"], "text");
    assert_eq!(
        body["uuid"].as_str().expect("uuid should be present").len(),
        50
    );
    let content: serde_json::Value = serde_json::from_str(
        body["content"]
            .as_str()
            .expect("reply content should be a JSON string"),
    )
    .expect("reply content should be JSON text message content");
    assert_eq!(content["text"], "agent final result\nwith exact text");
    assert!(
        !content["text"]
            .as_str()
            .expect("text should be present")
            .contains("agents-router")
    );
}

#[tokio::test]
async fn hidden_lark_long_connection_closed_loop_replies_result_and_marks_processed() {
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
        .and(path("/open-apis/im/v1/messages/om_root_message_id/reply"))
        .and(header("authorization", "Bearer test-tenant-token"))
        .and(body_partial_json(json!({
            "msg_type": "text",
            "reply_in_thread": true
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {
                "message_id": "om_result_reply_message_id",
                "root_id": "om_root_message_id",
                "parent_id": "om_root_message_id",
                "thread_id": "omt_result_thread",
                "msg_type": "text"
            }
        })))
        .mount(&server)
        .await;

    let now = lark_hidden_e2e_time();
    let mut ledger = ResponseSurfaceLedger::in_memory();
    ledger
        .create_surface_at(lark_hidden_e2e_surface(), now)
        .expect("surface should be created");
    let long_connection =
        FeishuLarkLongConnectionRuntime::from_provider_config(&lark_hidden_e2e_provider_config())
            .expect("hidden App Bot long connection runtime should build");
    let provider = test_app_bot_provider(server.uri());
    let controller = RecordingCodexController::with_result("Codex final answer, unchanged.");
    let controller_runtime = AgentControllerRuntime::new(vec![&controller]);

    let claim_decision = long_connection
        .handle_event_before_platform_ack(
            &mut ledger,
            FeishuLarkLongConnectionEvent {
                raw_event: include_bytes!(
                    "../../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
                ),
                received_at: now + Duration::seconds(1),
            },
        )
        .expect("Lark reply should normalize, lookup, and claim");
    let FeishuLarkLongConnectionDecision::AckReadyAfterLocalClaim(ready) = claim_decision else {
        panic!("surface reply should be ready after local claim");
    };
    assert_eq!(ready.reply.reply_text, "@_user_1 continue with README");

    let decision = long_connection
        .continue_claimed_event_hidden_with_test_policy_facts(
            &lark_hidden_e2e_config(),
            &mut ledger,
            ready.clone(),
            &controller_runtime,
            &provider,
            now + Duration::seconds(2),
            Some(available_codex_desktop()),
            Some(provider_mode_capability(ProviderMode::FeishuLarkAppBot)),
        )
        .await
        .expect("hidden closed loop should not fail");

    let AgentControllerClosedLoopDecision::Completed(completion) = decision else {
        panic!("hidden closed loop should complete");
    };
    assert!(matches!(
        completion.outcome,
        AgentControllerClosedLoopOutcome::ControllerSucceeded(_)
    ));
    assert_eq!(
        completion.provider_reply_message_id.as_deref(),
        Some("om_result_reply_message_id")
    );
    assert!(matches!(
        completion.inbound_event,
        InboundEventRecordDecision::Recorded { .. }
    ));

    let controller_requests = controller.requests();
    assert_eq!(controller_requests.len(), 1);
    assert_eq!(controller_requests[0].source_session_id, "session-1");
    assert_eq!(
        controller_requests[0].reply_text,
        "@_user_1 continue with README"
    );

    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    let reply_request = requests
        .iter()
        .find(|request| request.url.path() == "/open-apis/im/v1/messages/om_root_message_id/reply")
        .expect("thread result reply should be sent");
    let body: serde_json::Value = reply_request
        .body_json()
        .expect("reply body should be JSON");
    let content: serde_json::Value = serde_json::from_str(
        body["content"]
            .as_str()
            .expect("reply content should be a JSON string"),
    )
    .expect("reply content should be text message JSON");
    assert_eq!(content["text"], "Codex final answer, unchanged.");
    assert_eq!(body["reply_in_thread"], true);

    assert!(matches!(
        ledger
            .claim_inbound_event_at(inbound_input_for_ready(&ready), now + Duration::seconds(3))
            .expect("processed event should stay deduped"),
        InboundEventClaimDecision::DuplicateProcessed { .. }
    ));
}

#[tokio::test]
async fn hidden_lark_closed_loop_still_skips_planned_codex_catalog() {
    let now = lark_hidden_e2e_time();
    let mut ledger = ResponseSurfaceLedger::in_memory();
    ledger
        .create_surface_at(lark_hidden_e2e_surface(), now)
        .expect("surface should be created");
    let long_connection =
        FeishuLarkLongConnectionRuntime::from_provider_config(&lark_hidden_e2e_provider_config())
            .expect("hidden App Bot long connection runtime should build");
    let provider = test_app_bot_provider("http://127.0.0.1:1".to_string());
    let controller = RecordingCodexController::with_result("should not run");
    let controller_runtime = AgentControllerRuntime::new(vec![&controller]);

    let claim_decision = long_connection
        .handle_event_before_platform_ack(
            &mut ledger,
            FeishuLarkLongConnectionEvent {
                raw_event: include_bytes!(
                    "../../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
                ),
                received_at: now + Duration::seconds(1),
            },
        )
        .expect("Lark reply should normalize, lookup, and claim");
    let FeishuLarkLongConnectionDecision::AckReadyAfterLocalClaim(ready) = claim_decision else {
        panic!("surface reply should be ready after local claim");
    };

    let decision = long_connection
        .continue_claimed_event_hidden(
            &lark_hidden_e2e_config(),
            &mut ledger,
            ready.clone(),
            &controller_runtime,
            &provider,
            now + Duration::seconds(2),
        )
        .await
        .expect("hidden closed loop should not fail");

    assert_eq!(
        decision,
        AgentControllerClosedLoopDecision::Skipped(AgentControllerRuntimeSkipReason::Policy(
            ResponseSurfacePolicySkipReason::AgentContinuationPlanned
        ))
    );
    assert!(controller.requests().is_empty());
    assert!(matches!(
        ledger
            .claim_inbound_event_at(inbound_input_for_ready(&ready), now + Duration::seconds(3))
            .expect("planned skip should release the claim"),
        InboundEventClaimDecision::Claimed { .. }
    ));
}

#[tokio::test]
async fn app_bot_thread_reply_root_mismatch_fails_without_success() {
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
        .and(path("/open-apis/im/v1/messages/om_root_message_id/reply"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "msg": "success",
            "data": {
                "message_id": "om_result_reply_message_id",
                "root_id": "om_other_root_message_id"
            }
        })))
        .mount(&server)
        .await;

    let provider = test_app_bot_provider(server.uri());
    let err = provider
        .send_thread_reply(thread_reply_request("agent final result"))
        .await
        .expect_err("root mismatch should fail");

    assert!(
        err.message
            .contains("root_id did not match the response surface thread")
    );
}

#[tokio::test]
async fn custom_bot_thread_reply_is_not_supported() {
    let provider = test_provider(
        "https://open.feishu.cn/open-apis/bot/v2/hook/test".to_string(),
        None,
    );

    let err = provider
        .send_thread_reply(thread_reply_request("agent final result"))
        .await
        .expect_err("custom bot should not support thread replies");

    assert!(
        err.message
            .contains("custom bot mode does not support thread replies")
    );
}

#[tokio::test]
async fn app_bot_thread_reply_token_error_does_not_expose_token_or_app_secret() {
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
        .send_thread_reply(thread_reply_request("agent final result"))
        .await
        .expect_err("token provider error should fail");

    assert!(err.message.contains("99991663"));
    assert!(!err.message.contains("test-app-secret"));
    assert!(!err.message.contains("test-tenant-token"));
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
async fn app_bot_send_response_tenant_mismatch_fails_without_delivery_receipt() {
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
                "chat_id": "oc_5ce6d572455d361153b7xx51da133945",
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
fn escapes_markdown_image_markers_in_card_markdown() {
    let signal = structured_codex_signal(
        Some("agents-router sync report"),
        None,
        Some(SignalAnswer {
            kind: SignalAnswerKind::Full,
            content: "See ![screenshot](/tmp/smoke.png) for the browser smoke.".to_string(),
        }),
    );

    let elements = format_signal_card_elements_with_time(&signal, "2026-05-10 01:35:42 +08:00");
    let answer = elements
        .iter()
        .find_map(|element| match element {
            FeishuLarkCardElement::Markdown { content } if content.contains("screenshot") => {
                Some(content.as_str())
            }
            _ => None,
        })
        .expect("answer markdown should be present");

    assert!(answer.contains(r"\![screenshot](/tmp/smoke.png)"));
    assert!(!answer.contains(" ![screenshot]"));
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
            tenant_key: "2ca1d211f64f6438".to_string(),
            chat_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
            computer_name: "Test Mac".to_string(),
        }),
        client: reqwest::Client::new(),
    }
}

fn thread_reply_request(text: &str) -> ProviderThreadReplyRequest {
    ProviderThreadReplyRequest {
        provider_id: "work_chat".to_string(),
        provider_type: "feishu_lark".to_string(),
        provider_account_id: "2ca1d211f64f6438".to_string(),
        provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
        provider_thread_id: "om_root_message_id".to_string(),
        surface_id: "surface-1".to_string(),
        provider_event_id_hash: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            .to_string(),
        text: text.to_string(),
    }
}

#[derive(Default)]
struct RecordingCodexController {
    requests: Arc<Mutex<Vec<AgentControllerRequest>>>,
    result_text: String,
}

impl RecordingCodexController {
    fn with_result(result_text: &str) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            result_text: result_text.to_string(),
        }
    }

    fn requests(&self) -> Vec<AgentControllerRequest> {
        self.requests
            .lock()
            .expect("controller requests mutex should not be poisoned")
            .clone()
    }
}

impl AgentControllerAdapter for RecordingCodexController {
    fn controller_kind(&self) -> AgentControllerKind {
        AgentControllerKind::CodexAppServer
    }

    fn continue_session<'a>(
        &'a self,
        request: AgentControllerRequest,
    ) -> AgentControllerFuture<'a> {
        Box::pin(async move {
            self.requests
                .lock()
                .expect("controller requests mutex should not be poisoned")
                .push(request);

            Ok(
                AgentControllerSuccess::from_result_text(self.result_text.clone())
                    .expect("test controller result must not be blank"),
            )
        })
    }
}

fn lark_hidden_e2e_config() -> ValidatedConfig {
    let mut route = RouteConfig::new(
        vec!["codex_desktop".to_string()],
        vec!["work_chat".to_string()],
    );
    route.response_surface.enabled = true;

    ValidatedConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        cli: CliConfig::default(),
        log: LogConfig::default(),
        notification: NotificationConfig::default(),
        sources: vec![SourceConfig {
            id: "codex_desktop".to_string(),
            source_type: SourceType::CodexDesktop,
        }],
        providers: vec![lark_hidden_e2e_provider_config()],
        routes: vec![route],
    }
}

fn lark_hidden_e2e_provider_config() -> ProviderConfig {
    ProviderConfig {
        id: "work_chat".to_string(),
        detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(
            FeishuLarkAppBotProviderConfig {
                domain: FeishuLarkAppDomain::Lark,
                app_id: "cli_test".to_string(),
                app_secret: SecretSource::Inline("test-app-secret".to_string()),
                tenant_key: "2ca1d211f64f6438".to_string(),
                chat_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
            },
        )),
    }
}

fn lark_hidden_e2e_surface() -> NewResponseSurface {
    NewResponseSurface {
        signal_id: "signal-1".to_string(),
        delivery_id: "delivery-1".to_string(),
        source_id: "codex_desktop".to_string(),
        source_type: "codex_desktop".to_string(),
        source_session_id: "session-1".to_string(),
        source_turn_id: Some("turn-1".to_string()),
        provider_id: "work_chat".to_string(),
        provider_type: ProviderType::FeishuLark.as_str().to_string(),
        provider_mode: ProviderMode::FeishuLarkAppBot,
        provider_account_id: "2ca1d211f64f6438".to_string(),
        provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
        provider_message_id: "om_root_message_id".to_string(),
        provider_thread_id: "om_root_message_id".to_string(),
        route_binding_hash: None,
    }
}

fn available_codex_desktop() -> AgentIntegrationDescriptor {
    let descriptor = *agent_integration_descriptor(AgentIntegrationId::CodexDesktop);
    let target = descriptor
        .continuation_capability
        .target()
        .expect("Codex Desktop planned continuation target should be cataloged");
    AgentIntegrationDescriptor {
        continuation_capability: ContinuationCapability::available_experimental(target),
        ..descriptor
    }
}

fn inbound_input_for_ready(ready: &ProviderInboundReady) -> InboundEventDedupInput {
    InboundEventDedupInput {
        provider_id: ready.reply.provider_id.clone(),
        provider_type: ready.reply.provider_type.clone(),
        provider_account_id: ready.reply.provider_account_id.clone(),
        provider_conversation_id: ready.reply.provider_conversation_id.clone(),
        provider_event_id: ready.reply.provider_event_id.clone(),
        surface_id: ready.surface.surface_id.clone(),
    }
}

fn lark_hidden_e2e_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 23, 1, 2, 3)
        .single()
        .expect("test time should be valid")
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
