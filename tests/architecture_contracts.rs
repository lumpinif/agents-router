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
        let content = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", file.display()));
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
