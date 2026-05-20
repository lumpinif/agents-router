use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn production_source_paths_do_not_construct_final_signals_directly() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rust_files(&root.join("src/sources"), &mut files);
    files.push(root.join("src/local_ingress.rs"));

    // This is a lightweight architecture guard, not a compiler-enforced
    // boundary. It catches ordinary direct Signal construction in source paths;
    // if this boundary grows more critical, replace it with stronger module
    // visibility or a real lint.
    let forbidden = [
        "Signal::new(",
        "Signal::new_with_timestamp(",
        "Signal::new_structured_with_timestamp(",
    ];
    for file in files {
        let content = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", file.display()));
        for pattern in forbidden {
            assert!(
                !content.contains(pattern),
                "`{}` constructs final Signal directly with `{}`; source paths must go through SignalDraft and SignalBuilder",
                file.strip_prefix(&root).unwrap_or(&file).display(),
                pattern
            );
        }
    }
}

#[test]
fn human_provider_renderers_do_not_interpret_signal_structure_directly() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let providers_dir = root.join("src/providers");
    let mut files = Vec::new();
    collect_rust_files(&providers_dir, &mut files);

    let allowed = [
        providers_dir.join("notification_view.rs"),
        providers_dir.join("webhook.rs"),
        providers_dir.join("contract_test.rs"),
        providers_dir.join("http.rs"),
        providers_dir.join("mod.rs"),
    ];
    let forbidden = [
        ".workspace",
        ".conversation",
        ".links",
        ".summary()",
        "SignalAnswerKind",
    ];

    for file in files {
        if allowed.contains(&file) || file.file_name().is_some_and(|name| name == "tests.rs") {
            continue;
        }

        let content = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", file.display()));
        let production_content = content
            .split_once("\n#[cfg(test)]")
            .map_or(content.as_str(), |(production, _tests)| production);
        for pattern in forbidden {
            assert!(
                !production_content.contains(pattern),
                "`{}` reads Signal display structure directly with `{}`; human provider rendering must go through SignalNotificationView",
                file.strip_prefix(&root).unwrap_or(&file).display(),
                pattern
            );
        }
    }
}

#[test]
fn continuation_support_does_not_define_parallel_fact_sources() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = root.join("src");
    let mut files = Vec::new();
    collect_rust_files(&src_dir, &mut files);

    let forbidden = ["AgentControllerCapabilityCatalog", "controller_supported"];
    for file in files {
        let content = production_rust_content(&file);
        for pattern in forbidden {
            assert!(
                !content.contains(pattern),
                "`{}` defines or references `{}`; continuation support facts must live in AgentIntegrationCatalog",
                file.strip_prefix(&root).unwrap_or(&file).display(),
                pattern
            );
        }
    }
}

#[test]
fn response_surface_policy_does_not_hardcode_provider_or_agent_modes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let policy_path = root.join("src/response_surface_policy.rs");
    let content = production_rust_content(&policy_path);

    for pattern in [
        "ProviderType::Slack",
        "ProviderType::FeishuLark",
        "ProviderType::Webhook",
        "ProviderMode::SlackApp",
        "ProviderMode::SlackIncomingWebhook",
        "ProviderMode::FeishuLarkAppBot",
        "ProviderMode::FeishuLarkCustomBot",
        "codex_desktop",
        "claude_code",
    ] {
        assert!(
            !content.contains(pattern),
            "`src/response_surface_policy.rs` contains `{pattern}` in production code; policy must consume catalog facts instead of hardcoding provider modes or agent ids"
        );
    }
}

#[test]
fn response_surface_policy_does_not_depend_on_runtime_surfaces() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let policy_path = root.join("src/response_surface_policy.rs");
    let content = production_rust_content(&policy_path);

    for pattern in [
        "RawConfig",
        "response_surface_ledger",
        "agent_controller",
        "ProviderConfigDetail",
        "provider.send",
    ] {
        assert!(
            !content.contains(pattern),
            "`src/response_surface_policy.rs` contains `{pattern}`; Step 2 policy must not read raw config, write ledger, call controller, or depend on provider runtime details"
        );
    }
}

#[test]
fn response_surface_ledger_does_not_store_content_or_product_facts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ledger_path = root.join("src/response_surface_ledger.rs");
    let content = production_rust_content(&ledger_path);

    for pattern in [
        "prompt",
        "answer",
        "reply_text",
        "raw_inbound_payload",
        "provider_message_body",
        "rendered_payload",
        "agent_transcript",
        "tool_output",
        "controller_supported",
        "AgentControllerKind",
        "ContinuationSupportStatus",
        "ContinuationCapability",
        "agent_integration_catalog",
        "agent_controller",
    ] {
        assert!(
            !content.contains(pattern),
            "`src/response_surface_ledger.rs` contains `{pattern}` in production code; ledger must stay a minimal binding and dedup index, not a history store or product fact source"
        );
    }
}

#[test]
fn setup_paths_do_not_expose_response_surface_controls() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rust_files(&root.join("src/setup"), &mut files);
    collect_rust_files(&root.join("src/cli"), &mut files);

    for file in files {
        let content = production_rust_content(&file);
        assert!(
            !content.contains("response_surface"),
            "`{}` references `response_surface`; Step 2 must not expose replies in setup or CLI flows",
            file.strip_prefix(&root).unwrap_or(&file).display()
        );
    }
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read entry in `{}`: {error}", path.display())
        });
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn production_rust_content(path: &Path) -> String {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", path.display()));
    content
        .split_once("\n#[cfg(test)]")
        .map_or(content.as_str(), |(production, _tests)| production)
        .to_string()
}
