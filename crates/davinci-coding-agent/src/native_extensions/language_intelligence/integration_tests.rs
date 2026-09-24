//! Opt-in real-server verification. Uses existing packages, never downloads.
use super::*;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

fn copy_package(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_name() == "node_modules" {
            continue;
        }
        if path.is_dir() {
            copy_package(&path, &target.join(entry.file_name()));
        } else {
            std::fs::copy(&path, target.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "requires DAVINCI_TEST_TYPESCRIPT and DAVINCI_TEST_LANGUAGE_SERVER package directories"]
fn real_typescript_semantics_and_edit_synchronization() {
    let ts =
        std::env::var_os("DAVINCI_TEST_TYPESCRIPT").expect("installed TypeScript package path");
    let tls = std::env::var_os("DAVINCI_TEST_LANGUAGE_SERVER")
        .expect("installed language server package path");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    copy_package(Path::new(&ts), &root.join("node_modules/typescript"));
    copy_package(
        Path::new(&tls),
        &root.join("node_modules/typescript-language-server"),
    );
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src"]}"#,
    )
    .unwrap();
    std::fs::write(root.join("src/a.ts"), "export interface Greeter { greet(name: string): string; }\nexport class English implements Greeter { greet(name: string): string { return name; } }\nexport function greet(name: string): string { return `Hello ${name}`; }\n").unwrap();
    let good = "import { greet, Greeter, English } from './a';\ngreet('DaVinci');\nconst person: Greeter = new English();\nperson.greet('friend');\n";
    std::fs::write(
        root.join("src/b.ts"),
        format!("{good}const broken: string = 42;\n"),
    )
    .unwrap();
    let mut config = LanguageIntelligenceConfig::default();
    config.typescript.request_timeout_ms = 30000;
    config.typescript.backend = super::servers::Backend::TypeScriptLanguageServer;
    let started = Instant::now();
    let manager = LanguageIntelligence::new(root, config);
    let startup = started.elapsed();
    assert_eq!(manager.status()["sessions"], json!([]));
    let mut policy =
        davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove);
    policy.project_trusted = true;
    manager.set_permissions(Some(Arc::new(davinci_agent::PermissionState::new(policy))));
    let call = |name: &str, args: Value| {
        let result = manager.execute(name, &args).unwrap();
        assert!(!result.is_error, "{name}: {}", result.content);
        result.details.unwrap()
    };
    let started = Instant::now();
    let definition = call(
        "lsp_definition",
        json!({"path":"src/b.ts","line":2,"column":2}),
    );
    let first = started.elapsed();
    assert_eq!(definition["items"][0]["path"], "src/a.ts");
    let started = Instant::now();
    let hover = call("lsp_hover", json!({"path":"src/b.ts","line":2,"column":2}));
    let warm = started.elapsed();
    assert!(hover["text"].as_str().unwrap().contains("name: string"));
    let refs = call(
        "lsp_references",
        json!({"path":"src/b.ts","line":2,"column":2,"includeDeclaration":true}),
    );
    assert!(refs["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["path"] == "src/a.ts"));
    assert!(refs["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["path"] == "src/b.ts"));
    let symbols = call("lsp_document_symbols", json!({"path":"src/a.ts"}));
    assert!(symbols["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == "greet"));
    let symbols = call("lsp_workspace_symbols", json!({"query":"greet"}));
    assert!(symbols["total"].as_u64().unwrap() > 0);
    let implementations = call(
        "lsp_implementations",
        json!({"path":"src/a.ts","line":1,"column":19}),
    );
    assert!(implementations["total"].as_u64().unwrap() > 0);
    let types = call(
        "lsp_type_definition",
        json!({"path":"src/b.ts","line":4,"column":2}),
    );
    assert_eq!(types["items"][0]["path"], "src/a.ts");
    let diagnostics = call(
        "lsp_diagnostics",
        json!({"path":"src/b.ts","severity":"error"}),
    );
    assert!(
        diagnostics["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["code"] == 2322),
        "{diagnostics}"
    );
    std::fs::write(root.join("src/b.ts"), good).unwrap();
    let diagnostics = call("lsp_diagnostics", json!({"path":"src/b.ts"}));
    assert_eq!(diagnostics["total"], 0, "{diagnostics}");
    assert_eq!(manager.status()["sessions"][0]["starts"], 1);
    println!("language intelligence: lazy construction={startup:?}; first definition={first:?}; warm hover={warm:?}; status={}", manager.status());
    manager.shutdown();
}
