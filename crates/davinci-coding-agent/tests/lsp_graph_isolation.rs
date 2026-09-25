use davinci_coding_agent::native_extensions::language_intelligence::{
    LanguageIntelligence, LanguageIntelligenceConfig,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn node() -> PathBuf {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|path| path.is_absolute())
        .map(|path| path.join(name))
        .find(|path| path.is_file())
        .and_then(|path| path.canonicalize().ok())
        .expect("Node is required by the deterministic LSP fixture")
}

fn config() -> LanguageIntelligenceConfig {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/language-server.cjs")
        .canonicalize()
        .unwrap();
    LanguageIntelligenceConfig::from_value(json!({
        "typescript": {
            "server": {"program": node(), "args": [fixture]}
        }
    }))
}

fn trust(manager: &LanguageIntelligence) {
    let mut policy =
        davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove);
    policy.project_trusted = true;
    manager.set_permissions(Some(Arc::new(davinci_agent::PermissionState::new(policy))));
}

fn source(root: &Path) {
    std::fs::write(root.join("package.json"), "{}").unwrap();
    std::fs::write(root.join("a.ts"), "export const value = 1;\n").unwrap();
}

#[test]
fn lsp_graph_same_worktree_reuses_owner_and_distinct_worktrees_isolate_sessions() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    source(first.path());
    source(second.path());

    let owner = LanguageIntelligence::new(first.path(), config());
    trust(&owner);
    let first_worker = owner.for_workspace(first.path());
    let peer_worker = owner.for_workspace(first.path());
    let second_worker = owner.for_workspace(second.path());

    for worker in [&first_worker, &peer_worker] {
        let result = worker
            .execute(
                "lsp_hover",
                &json!({"path":"a.ts","line":1,"column":1}),
            )
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
    }
    assert_eq!(owner.status()["sessions"].as_array().unwrap().len(), 1);

    let result = second_worker
        .execute(
            "lsp_hover",
            &json!({"path":"a.ts","line":1,"column":1}),
        )
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let sessions = owner.status()["sessions"].as_array().unwrap().to_vec();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|session| session["starts"] == 1));

    owner.shutdown();
    assert_eq!(owner.status()["sessions"], json!([]));
}
