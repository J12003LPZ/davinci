use davinci_agent::semantic::SemanticService;
use davinci_coding_agent::native_extensions::language_intelligence::{
    LanguageIntelligence, LanguageIntelligenceConfig, TOOL_NAMES,
};
use davinci_coding_agent::semantic::SemanticServiceFacade;
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

fn fixture_config() -> LanguageIntelligenceConfig {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/language-server.cjs")
        .canonicalize()
        .unwrap();
    LanguageIntelligenceConfig::from_value(json!({
        "typescript": {
            "server": {
                "program": node(),
                "args": [fixture]
            }
        }
    }))
}

fn trust(manager: &LanguageIntelligence) {
    let mut policy =
        davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove);
    policy.project_trusted = true;
    manager.set_permissions(Some(Arc::new(davinci_agent::PermissionState::new(policy))));
}

#[test]
fn lsp_tool_names_stay_eight() {
    assert_eq!(
        TOOL_NAMES,
        &[
            "lsp_definition",
            "lsp_references",
            "lsp_hover",
            "lsp_document_symbols",
            "lsp_workspace_symbols",
            "lsp_implementations",
            "lsp_type_definition",
            "lsp_diagnostics",
        ]
    );
}

#[test]
fn lsp_core_native_definition_same_location_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("package.json"), "{}").unwrap();
    std::fs::write(dir.path().join("a.ts"), "export const value = 1;\n").unwrap();
    let manager = LanguageIntelligence::new(dir.path(), fixture_config());
    trust(&manager);

    let native = manager
        .execute(
            "lsp_definition",
            &json!({"path":"a.ts","line":1,"column":1}),
        )
        .unwrap();
    assert!(!native.is_error, "{}", native.content);
    let native = native.details.unwrap();
    assert_eq!(native["items"][0]["path"], "a.ts");
    assert_eq!(native["items"][0]["range"]["start"]["line"], 1);
    assert_eq!(native["items"][0]["range"]["start"]["column"], 1);

    let facade = SemanticServiceFacade::local(manager.clone());
    let core = facade.definition(dir.path(), "a.ts", 0, 0).unwrap();
    assert_eq!(core.locations[0].path, "a.ts");
    assert_eq!(core.locations[0].range.start.line, 0);
    assert_eq!(core.locations[0].range.start.character, 0);
    assert_eq!(manager.status()["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(manager.status()["sessions"][0]["starts"], 1);
    manager.shutdown();
}
