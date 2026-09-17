//! Shared normal-mode mutation path observed by existing intelligence managers.
use davinci_agent::tools::{execute_tool_with, ToolContext};
use davinci_coding_agent::native_extensions::{
    language_intelligence::{LanguageIntelligence, LanguageIntelligenceConfig},
    repo_intelligence::{RepoIntelligence, RepoIntelligenceConfig},
};
use serde_json::json;
use std::{fs, sync::Arc};

#[test]
fn transaction_apply_and_rollback_refresh_existing_repo_and_lsp_sessions() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let package = root.path().join("node_modules/typescript-language-server");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"typescript-language-server","version":"fixture","bin":"server.cjs"}"#,
    )
    .unwrap();
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/language-server.cjs"
        ),
        package.join("server.cjs"),
    )
    .unwrap();
    fs::write(root.path().join("a.ts"), "good").unwrap();
    fs::write(root.path().join("b.ts"), "export function alpha() {}\n").unwrap();
    let repo = RepoIntelligence::new(root.path(), cache.path(), RepoIntelligenceConfig::default());
    let language = LanguageIntelligence::new(root.path(), LanguageIntelligenceConfig::default());
    let mut policy =
        davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove);
    policy.project_trusted = true;
    language.set_permissions(Some(Arc::new(davinci_agent::PermissionState::new(policy))));
    let diagnostics = || {
        let result = language
            .execute("lsp_diagnostics", &json!({"path":"a.ts"}))
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        result.details.unwrap()
    };
    assert_eq!(diagnostics()["total"], 0);
    assert_eq!(
        repo.refresh().unwrap().files["b.ts"].symbols[0].name,
        "alpha"
    );
    let context = ToolContext::default();
    let applied = execute_tool_with(root.path(), "apply_patch", &json!({"input":
        "*** Begin Patch\n*** Update File: a.ts\n@@\n-good\n+bad\n*** Update File: b.ts\n@@\n-export function alpha() {}\n+export function omega() {}\n*** End Patch"
    }), &context).unwrap();
    assert!(!applied.is_error, "{}", applied.content);
    let id = applied.details.unwrap()["transaction"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(diagnostics()["total"], 1);
    let changed = repo.refresh().unwrap();
    assert_eq!(changed.files["b.ts"].symbols[0].name, "omega");
    assert_eq!(changed.reparsed, 2);
    let rolled_back = execute_tool_with(
        root.path(),
        "patch_rollback",
        &json!({"id":id,"paths":["a.ts","b.ts"]}),
        &context,
    )
    .unwrap();
    assert!(!rolled_back.is_error, "{}", rolled_back.content);
    assert_eq!(diagnostics()["total"], 0);
    assert_eq!(
        repo.refresh().unwrap().files["b.ts"].symbols[0].name,
        "alpha"
    );
    assert_eq!(language.status()["sessions"][0]["starts"], 1);
    language.shutdown();
    assert_eq!(language.status()["sessions"], json!([]));
}
