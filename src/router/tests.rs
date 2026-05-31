use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use super::*;
use tempfile::tempdir;

use crate::bridge_binding_ledger::{BridgeBindingLedgerStore, RoomProjectBindingInput};
use crate::config::{
    CliConfig, LogConfig, NotificationConfig, ProviderType, RawConfig, RawProviderConfig,
    RouteConfig, SourceConfig, SourceType, ValidatedConfig,
};
use crate::delivery::{DeliveryErrorContext, ProviderDeliveryReceipt};
use crate::delivery_safety::DeliverySafetyGuard;
use crate::response_surface_ledger::{
    ResponseSurfaceLedger, ResponseSurfaceLedgerStore, ResponseSurfaceLookupQuery,
    ResponseSurfaceLookupResult,
};
use crate::signal::{SignalLifecycle, SignalWorkspace};

struct TestProvider {
    id: String,
    provider_type: String,
    calls: Arc<Mutex<Vec<String>>>,
    dynamic_calls: Option<Arc<Mutex<Vec<String>>>>,
    result: Result<(), DeliveryErrorKind>,
    delivery_receipt: Option<ProviderDeliveryReceipt>,
}

impl TestProvider {
    fn succeeding(id: &str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            id: id.to_string(),
            provider_type: "test".to_string(),
            calls,
            dynamic_calls: None,
            result: Ok(()),
            delivery_receipt: None,
        }
    }

    fn succeeding_with_dynamic_target(
        id: &str,
        calls: Arc<Mutex<Vec<String>>>,
        dynamic_calls: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            id: id.to_string(),
            provider_type: "test".to_string(),
            calls,
            dynamic_calls: Some(dynamic_calls),
            result: Ok(()),
            delivery_receipt: None,
        }
    }

    fn succeeding_with_receipt(
        id: &str,
        provider_type: &str,
        calls: Arc<Mutex<Vec<String>>>,
        delivery_receipt: ProviderDeliveryReceipt,
    ) -> Self {
        Self {
            id: id.to_string(),
            provider_type: provider_type.to_string(),
            calls,
            dynamic_calls: None,
            result: Ok(()),
            delivery_receipt: Some(delivery_receipt),
        }
    }

    fn failing(id: &str, calls: Arc<Mutex<Vec<String>>>, kind: DeliveryErrorKind) -> Self {
        Self {
            id: id.to_string(),
            provider_type: "test".to_string(),
            calls,
            dynamic_calls: None,
            result: Err(kind),
            delivery_receipt: None,
        }
    }
}

impl Provider for TestProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn provider_type(&self) -> &str {
        &self.provider_type
    }

    fn send<'a>(&'a self, signal: &'a Signal) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(signal.id.clone());
            match &self.result {
                Ok(()) => {
                    let mut result =
                        ProviderSendResult::sent(&self.id, &self.provider_type, signal);
                    if let Some(receipt) = self.delivery_receipt.clone() {
                        result = result.with_delivery_receipt(receipt);
                    }
                    Ok(result)
                }
                Err(kind) => Err(DeliveryError::new(
                    *kind,
                    DeliveryErrorContext::provider_send(signal, &self.id, &self.provider_type),
                    "send failed",
                )),
            }
        })
    }

    fn send_to_provider_conversation<'a>(
        &'a self,
        signal: &'a Signal,
        provider_conversation_id: &'a str,
    ) -> Option<ProviderFuture<'a>> {
        let dynamic_calls = self.dynamic_calls.as_ref()?;
        Some(Box::pin(async move {
            dynamic_calls
                .lock()
                .unwrap()
                .push(format!("{}:{}", provider_conversation_id, signal.id));
            Ok(ProviderSendResult::sent(
                &self.id,
                &self.provider_type,
                signal,
            ))
        }))
    }
}

#[tokio::test]
async fn sends_signal_to_all_matching_providers() {
    let config = test_config(vec![RouteConfig::new(
        vec!["codex_cli".to_string()],
        vec!["phone".to_string(), "debug".to_string()],
    )]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));
    let debug = TestProvider::succeeding("debug", Arc::clone(&calls));

    let report = Router::new(&config)
        .route(&test_signal("codex_cli"), &[&phone, &debug])
        .await
        .expect("providers should succeed");

    assert_eq!(report.matched_routes, 1);
    assert_eq!(report.attempted, 2);
    assert_eq!(report.succeeded, 2);
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn unmatched_signal_does_not_send() {
    let config = test_config(vec![RouteConfig::new(
        vec!["codex_desktop".to_string()],
        vec!["phone".to_string()],
    )]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));

    let report = Router::new(&config)
        .route(&test_signal("codex_cli"), &[&phone])
        .await
        .expect("no matching route is not an error");

    assert_eq!(report.matched_routes, 0);
    assert_eq!(report.attempted, 0);
    assert_eq!(report.succeeded, 0);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provider_failure_does_not_block_other_providers() {
    let config = test_config(vec![RouteConfig::new(
        vec!["codex_cli".to_string()],
        vec!["phone".to_string(), "debug".to_string()],
    )]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::failing(
        "phone",
        Arc::clone(&calls),
        DeliveryErrorKind::ProviderRejected,
    );
    let debug = TestProvider::succeeding("debug", Arc::clone(&calls));

    let err = Router::new(&config)
        .route(&test_signal("codex_cli"), &[&phone, &debug])
        .await
        .expect_err("provider failure should be returned");

    assert_eq!(calls.lock().unwrap().len(), 2);
    let RouterError::ProviderFailures(failures) = err;
    assert_eq!(
        failures,
        vec![ProviderFailure {
            signal_id: "signal-1".to_string(),
            source_id: "codex_cli".to_string(),
            provider_id: "phone".to_string(),
            provider_type: "test".to_string(),
            kind: DeliveryErrorKind::ProviderRejected,
            message: "send failed".to_string(),
            http_status: None,
            provider_code: None,
            retriable: false,
        }]
    );
}

#[tokio::test]
async fn provider_failure_display_includes_actionable_context() {
    let config = test_config(vec![RouteConfig::new(
        vec!["codex_cli".to_string()],
        vec!["phone".to_string()],
    )]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::failing(
        "phone",
        Arc::clone(&calls),
        DeliveryErrorKind::ProviderRejected,
    );

    let err = Router::new(&config)
        .route(&test_signal("codex_cli"), &[&phone])
        .await
        .expect_err("provider failure should be returned");
    let message = err.to_string();

    assert!(message.contains("phone"));
    assert!(message.contains("test"));
    assert!(message.contains("provider_rejected"));
    assert!(message.contains("send failed"));
}

#[tokio::test]
async fn duplicate_delivery_safety_suppresses_without_provider_failure() {
    let config = test_config(vec![RouteConfig::new(
        vec!["codex_cli".to_string()],
        vec!["phone".to_string()],
    )]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));
    let safety = DeliverySafetyGuard::in_memory();
    let signal = test_signal("codex_cli");
    let mut last_report = None;

    for _ in 0..5 {
        last_report = Some(
            Router::new(&config)
                .route_with_safety(&signal, &[&phone], Some(&safety))
                .await
                .expect("suppressed delivery is not a provider failure"),
        );
    }

    let report = last_report.expect("report should exist");
    assert_eq!(report.attempted, 1);
    assert_eq!(report.succeeded, 0);
    assert_eq!(report.suppressed, 1);
    assert_eq!(calls.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn app_bot_surface_ready_delivery_creates_response_surface() {
    let temp = tempdir().expect("temp dir should exist");
    let ledger_store = ResponseSurfaceLedgerStore::new(temp.path().join("ledger.json"))
        .expect("ledger store should build");
    let config = lark_app_bot_response_surface_config();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = TestProvider::succeeding_with_receipt(
        "work_chat",
        ProviderType::FeishuLark.as_str(),
        Arc::clone(&calls),
        ProviderDeliveryReceipt::surface_ready(
            Some("tenant-1".to_string()),
            Some("chat-1".to_string()),
            Some("message-root-1".to_string()),
            Some("message-root-1".to_string()),
        ),
    );

    let report = Router::new(&config)
        .route_with_safety_and_response_surfaces(
            &codex_desktop_signal_with_session(),
            &[&provider],
            None,
            Some(&ledger_store),
            None,
        )
        .await
        .expect("provider send should succeed");

    assert_eq!(report.succeeded, 1);
    let mut ledger = ResponseSurfaceLedger::load(ledger_store.state_path().to_path_buf())
        .expect("ledger should load");
    let lookup = ledger
        .lookup_surface_at(
            ResponseSurfaceLookupQuery {
                provider_id: "work_chat".to_string(),
                provider_account_id: "tenant-1".to_string(),
                provider_conversation_id: "chat-1".to_string(),
                provider_thread_id: "message-root-1".to_string(),
            },
            Utc::now(),
        )
        .expect("surface lookup should succeed");

    assert!(matches!(
        lookup,
        ResponseSurfaceLookupResult::Hit(record)
            if record.source_session_id == "session-1"
                && record.route_binding_hash.is_some()
    ));
}

#[tokio::test]
async fn surface_ready_delivery_binds_provider_thread_to_source_session() {
    let temp = tempdir().expect("temp dir should exist");
    let ledger_store = ResponseSurfaceLedgerStore::new(temp.path().join("surfaces.json"))
        .expect("ledger store should build");
    let bridge_binding_ledger = BridgeBindingLedgerStore::new(temp.path().join("bindings.json"))
        .expect("bridge binding store should build");
    let config = lark_app_bot_response_surface_config();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = TestProvider::succeeding_with_receipt(
        "work_chat",
        ProviderType::FeishuLark.as_str(),
        Arc::clone(&calls),
        ProviderDeliveryReceipt::surface_ready(
            Some("tenant-1".to_string()),
            Some("chat-1".to_string()),
            Some("message-root-1".to_string()),
            Some("message-root-1".to_string()),
        ),
    );

    Router::new(&config)
        .route_with_safety_and_response_surfaces(
            &codex_desktop_signal_with_session_and_project_path(),
            &[&provider],
            None,
            Some(&ledger_store),
            Some(&bridge_binding_ledger),
        )
        .await
        .expect("provider send should succeed");

    let binding = bridge_binding_ledger
        .update(|ledger| {
            Ok(
                ledger.lookup_thread_session(&crate::bridge_binding_ledger::ThreadBindingQuery {
                    provider_id: "work_chat".to_string(),
                    provider_type: ProviderType::FeishuLark.as_str().to_string(),
                    provider_account_id: "tenant-1".to_string(),
                    provider_conversation_id: "chat-1".to_string(),
                    provider_thread_id: "message-root-1".to_string(),
                }),
            )
        })
        .await
        .expect("binding ledger should read")
        .expect("thread should be bound");

    assert_eq!(binding.project_path, "/Users/tester/projects/agents-router");
    assert_eq!(binding.source_id, "codex_desktop");
    assert_eq!(binding.source_session_id, "session-1");
}

#[tokio::test]
async fn candidate_delivery_receipt_does_not_create_response_surface() {
    let temp = tempdir().expect("temp dir should exist");
    let ledger_store = ResponseSurfaceLedgerStore::new(temp.path().join("ledger.json"))
        .expect("ledger store should build");
    let config = lark_app_bot_response_surface_config();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = TestProvider::succeeding_with_receipt(
        "work_chat",
        ProviderType::FeishuLark.as_str(),
        Arc::clone(&calls),
        ProviderDeliveryReceipt::candidate(
            Some("tenant-1".to_string()),
            Some("chat-1".to_string()),
            Some("message-root-1".to_string()),
            Some("message-root-1".to_string()),
        ),
    );

    Router::new(&config)
        .route_with_safety_and_response_surfaces(
            &codex_desktop_signal_with_session(),
            &[&provider],
            None,
            Some(&ledger_store),
            None,
        )
        .await
        .expect("provider send should succeed");

    let mut ledger = ResponseSurfaceLedger::load(ledger_store.state_path().to_path_buf())
        .expect("ledger should load");
    let lookup = ledger
        .lookup_surface_at(
            ResponseSurfaceLookupQuery {
                provider_id: "work_chat".to_string(),
                provider_account_id: "tenant-1".to_string(),
                provider_conversation_id: "chat-1".to_string(),
                provider_thread_id: "message-root-1".to_string(),
            },
            Utc::now(),
        )
        .expect("surface lookup should succeed");

    assert_eq!(lookup, ResponseSurfaceLookupResult::Miss);
}

#[tokio::test]
async fn project_binding_routes_to_bound_room_instead_of_static_provider_target() {
    let temp = tempdir().expect("temp dir should exist");
    let bridge_binding_ledger = BridgeBindingLedgerStore::new(temp.path().join("bindings.json"))
        .expect("bridge binding store should build");
    bridge_binding_ledger
        .update(|ledger| {
            ledger.connect_room_project_at(
                RoomProjectBindingInput {
                    provider_id: "debug".to_string(),
                    provider_type: "test".to_string(),
                    provider_account_id: "tenant-1".to_string(),
                    provider_conversation_id: "oc_agents_router_room".to_string(),
                    project_path: "/Users/tester/projects/agents-router".to_string(),
                },
                Utc::now(),
            )?;
            Ok(())
        })
        .await
        .expect("project room should bind");
    let config = test_config(vec![RouteConfig::new(
        vec!["codex_cli".to_string()],
        vec!["debug".to_string()],
    )]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dynamic_calls = Arc::new(Mutex::new(Vec::new()));
    let provider = TestProvider::succeeding_with_dynamic_target(
        "debug",
        Arc::clone(&calls),
        Arc::clone(&dynamic_calls),
    );

    let report = Router::new(&config)
        .route_with_safety_and_response_surfaces(
            &test_signal_with_project_path(
                "codex_cli",
                "/Users/tester/projects/agents-router/crate",
            ),
            &[&provider],
            None,
            None,
            Some(&bridge_binding_ledger),
        )
        .await
        .expect("bound project should route to room");

    assert_eq!(report.attempted, 1);
    assert_eq!(report.succeeded, 1);
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        *dynamic_calls.lock().unwrap(),
        vec!["oc_agents_router_room:signal-1".to_string()]
    );
}

#[tokio::test]
async fn duration_filter_matches_tasks_at_or_above_threshold() {
    let mut route = RouteConfig::new(vec!["codex_cli".to_string()], vec!["phone".to_string()]);
    route.minimum_task_duration_minutes = Some(5);
    let config = test_config(vec![route]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));

    let report = Router::new(&config)
        .route(&test_signal_with_duration("codex_cli", 300_000), &[&phone])
        .await
        .expect("duration at threshold should match");

    assert_eq!(report.matched_routes, 1);
    assert_eq!(report.attempted, 1);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn duration_filter_rejects_short_duration_but_allows_missing_duration() {
    let mut route = RouteConfig::new(vec!["codex_cli".to_string()], vec!["phone".to_string()]);
    route.minimum_task_duration_minutes = Some(5);
    let config = test_config(vec![route]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));

    let short_report = Router::new(&config)
        .route(&test_signal_with_duration("codex_cli", 299_999), &[&phone])
        .await
        .expect("filtered route is not an error");
    let missing_report = Router::new(&config)
        .route(&test_signal("codex_cli"), &[&phone])
        .await
        .expect("missing duration should not drop a completion notification");

    assert_eq!(short_report.matched_routes, 0);
    assert_eq!(missing_report.matched_routes, 1);
    assert_eq!(missing_report.attempted, 1);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn project_path_filter_matches_exact_or_descendant_paths() {
    let mut route = RouteConfig::new(vec!["codex_cli".to_string()], vec!["phone".to_string()]);
    route.only_forward_from_project_paths =
        vec!["/Users/tester/projects/agents-router".to_string()];
    let config = test_config(vec![route]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));

    let exact_report = Router::new(&config)
        .route(
            &test_signal_with_project_path("codex_cli", "/Users/tester/projects/agents-router"),
            &[&phone],
        )
        .await
        .expect("exact project path should match");
    let descendant_report = Router::new(&config)
        .route(
            &test_signal_with_project_path(
                "codex_cli",
                "/Users/tester/projects/agents-router/crate",
            ),
            &[&phone],
        )
        .await
        .expect("descendant project path should match");

    assert_eq!(exact_report.matched_routes, 1);
    assert_eq!(descendant_report.matched_routes, 1);
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn project_path_filter_rejects_prefix_or_missing_project_path() {
    let mut route = RouteConfig::new(vec!["codex_cli".to_string()], vec!["phone".to_string()]);
    route.only_forward_from_project_paths = vec!["/Users/tester/projects/app".to_string()];
    let config = test_config(vec![route]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));

    let prefix_report = Router::new(&config)
        .route(
            &test_signal_with_project_path("codex_cli", "/Users/tester/projects/app-copy"),
            &[&phone],
        )
        .await
        .expect("filtered route is not an error");
    let missing_report = Router::new(&config)
        .route(&test_signal("codex_cli"), &[&phone])
        .await
        .expect("filtered route is not an error");

    assert_eq!(prefix_report.matched_routes, 0);
    assert_eq!(missing_report.matched_routes, 0);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn route_filters_are_combined_with_and_semantics() {
    let mut route = RouteConfig::new(vec!["codex_cli".to_string()], vec!["phone".to_string()]);
    route.minimum_task_duration_minutes = Some(5);
    route.only_forward_from_project_paths =
        vec!["/Users/tester/projects/agents-router".to_string()];
    let config = test_config(vec![route]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let phone = TestProvider::succeeding("phone", Arc::clone(&calls));

    let wrong_project_report = Router::new(&config)
        .route(
            &test_signal_with_duration_and_project_path(
                "codex_cli",
                300_000,
                "/Users/tester/projects/other",
            ),
            &[&phone],
        )
        .await
        .expect("filtered route is not an error");
    let short_report = Router::new(&config)
        .route(
            &test_signal_with_duration_and_project_path(
                "codex_cli",
                299_999,
                "/Users/tester/projects/agents-router",
            ),
            &[&phone],
        )
        .await
        .expect("filtered route is not an error");
    let matched_report = Router::new(&config)
        .route(
            &test_signal_with_duration_and_project_path(
                "codex_cli",
                300_000,
                "/Users/tester/projects/agents-router",
            ),
            &[&phone],
        )
        .await
        .expect("both filters satisfied should match");

    assert_eq!(wrong_project_report.matched_routes, 0);
    assert_eq!(short_report.matched_routes, 0);
    assert_eq!(matched_report.matched_routes, 1);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

fn test_signal(source_id: &str) -> Signal {
    let timestamp = DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    Signal::new_with_timestamp(
        "signal-1",
        source_id,
        source_id,
        "Codex",
        "Ready for review.",
        timestamp,
        BTreeMap::new(),
    )
}

fn test_signal_with_duration(source_id: &str, duration_ms: u64) -> Signal {
    let mut signal = test_signal(source_id);
    signal.lifecycle = Some(SignalLifecycle {
        status: None,
        started_at: None,
        completed_at: None,
        duration_ms: Some(duration_ms),
    });
    signal
}

fn test_signal_with_project_path(source_id: &str, project_path: &str) -> Signal {
    let mut signal = test_signal(source_id);
    signal.workspace = Some(SignalWorkspace {
        cwd: None,
        project_name: None,
        project_path: Some(project_path.to_string()),
        branch: None,
        worktree: None,
    });
    signal
}

fn test_signal_with_duration_and_project_path(
    source_id: &str,
    duration_ms: u64,
    project_path: &str,
) -> Signal {
    let mut signal = test_signal_with_duration(source_id, duration_ms);
    signal.workspace = Some(SignalWorkspace {
        cwd: None,
        project_name: None,
        project_path: Some(project_path.to_string()),
        branch: None,
        worktree: None,
    });
    signal
}

fn codex_desktop_signal_with_session() -> Signal {
    let mut signal = test_signal("codex_desktop");
    signal.conversation = Some(crate::signal::SignalConversation {
        session_id: Some("session-1".to_string()),
        session_title: None,
        turn_id: Some("turn-1".to_string()),
        prompt: None,
        answer: None,
        model: None,
    });
    signal
}

fn codex_desktop_signal_with_session_and_project_path() -> Signal {
    let mut signal = codex_desktop_signal_with_session();
    signal.workspace = Some(SignalWorkspace {
        cwd: None,
        project_name: Some("agents-router".to_string()),
        project_path: Some("/Users/tester/projects/agents-router".to_string()),
        branch: None,
        worktree: None,
    });
    signal
}

fn test_config(routes: Vec<RouteConfig>) -> ValidatedConfig {
    let mut ntfy = RawProviderConfig::new("phone", ProviderType::Ntfy);
    ntfy.server = Some("https://ntfy.sh".to_string());
    ntfy.topic = Some("topic".to_string());

    let mut webhook = RawProviderConfig::new("debug", ProviderType::Webhook);
    webhook.url = Some("https://example.com/hook".to_string());

    RawConfig {
        schema_version: 1,
        cli: CliConfig::default(),
        log: LogConfig::default(),
        notification: NotificationConfig::default(),
        sources: vec![
            SourceConfig {
                id: "codex_cli".to_string(),
                source_type: SourceType::CodexCli,
            },
            SourceConfig {
                id: "codex_desktop".to_string(),
                source_type: SourceType::CodexDesktop,
            },
            SourceConfig {
                id: "claude_code".to_string(),
                source_type: SourceType::ClaudeCode,
            },
        ],
        providers: vec![ntfy, webhook],
        routes,
    }
    .validate()
    .expect("test config should validate")
}

fn lark_app_bot_response_surface_config() -> ValidatedConfig {
    let mut lark = RawProviderConfig::new("work_chat", ProviderType::FeishuLark);
    lark.mode = Some("app_bot".to_string());
    lark.domain = Some("lark".to_string());
    lark.app_id = Some("cli_test".to_string());
    lark.app_secret = Some("test-secret".to_string());
    lark.tenant_key = Some("tenant-1".to_string());
    lark.chat_id = Some("chat-1".to_string());

    let mut route = RouteConfig::new(
        vec!["codex_desktop".to_string()],
        vec!["work_chat".to_string()],
    );
    route.response_surface.enabled = true;

    RawConfig {
        schema_version: 1,
        cli: CliConfig::default(),
        log: LogConfig::default(),
        notification: NotificationConfig::default(),
        sources: vec![SourceConfig {
            id: "codex_desktop".to_string(),
            source_type: SourceType::CodexDesktop,
        }],
        providers: vec![lark],
        routes: vec![route],
    }
    .validate()
    .expect("test config should validate")
}
