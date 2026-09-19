use davinci_coding_agent::native_extensions::repo_intelligence::{
    parse_source, RepoIntelligence, SemanticEvidence, SemanticLanguageProvider, SemanticOperation,
    SourceRange,
};
use serde_json::json;
use std::{fs, sync::Arc};

#[test]
fn repo_intelligence_semantic_provider_and_strict_inputs() {
    #[derive(Debug)]
    struct Provider;
    impl SemanticLanguageProvider for Provider {
        fn query(
            &self,
            operation: SemanticOperation,
            path: &str,
            range: &SourceRange,
            _: usize,
        ) -> Result<Vec<SemanticEvidence>, String> {
            assert!(matches!(operation, SemanticOperation::Implementations));
            Ok(vec![
                SemanticEvidence {
                    path: path.into(),
                    range: range.clone(),
                    description: "semantic implementation".into(),
                },
                SemanticEvidence {
                    path: "../escape.ts".into(),
                    range: range.clone(),
                    description: "denied".into(),
                },
            ])
        }
    }
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(
        repo.path().join("a.ts"),
        "export interface CacheProvider { get(): string }",
    )
    .unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default())
        .with_semantic_provider(Arc::new(Provider));
    let result = manager
        .query(
            "code_query",
            &json!({"query":"What implements CacheProvider?"}),
        )
        .unwrap();
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["source"] == "lsp"));
    assert!(!result.to_string().contains("escape.ts"));
    for args in [
        json!({"query":"x","useLsp":true}),
        json!({"query":"x","includeEvidence":"yes"}),
        json!({"query":"x","limit":-1}),
        json!({"query":"x","path":"/etc"}),
    ] {
        assert!(manager.query("code_query", &args).is_err(), "{args}");
    }
}

#[test]
fn repo_intelligence_parser_exports_and_shadowing() {
    let file = parse_source(
        "module.cjs",
        "const helper = () => 1; exports.helper = helper; module.exports.other = helper;",
    )
    .unwrap();
    assert!(
        file.symbols
            .iter()
            .find(|s| s.name == "helper")
            .unwrap()
            .exported
    );
    let file = parse_source("module.ts", "export default function() { return 1; }").unwrap();
    assert!(file
        .symbols
        .iter()
        .any(|s| s.name == "default" && s.exported));
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(
        repo.path().join("a.ts"),
        "function helper() {} function run(helper: () => void) { helper(); }",
    )
    .unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let index = manager.refresh().unwrap();
    let run = index.files["a.ts"]
        .symbols
        .iter()
        .find(|s| s.name == "run")
        .unwrap();
    let result = manager
        .query("symbol_relationships", &json!({"symbolId":run.id}))
        .unwrap();
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "calls")
        .all(|r| r["resolved"] == false));
}

#[test]
fn repo_intelligence_nested_ignores_bounds_and_fallback_counts() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join("src")).unwrap();
    fs::write(repo.path().join(".gitignore"), "*.ts\n!src/keep.ts\n").unwrap();
    fs::write(repo.path().join("src/.gitignore"), "!nested.ts\n").unwrap();
    fs::write(repo.path().join("src/keep.ts"), "export const keep = 1;").unwrap();
    fs::write(
        repo.path().join("src/nested.ts"),
        "export const nested = 1;",
    )
    .unwrap();
    fs::write(repo.path().join("src/no.ts"), "export const no = 1;").unwrap();
    fs::write(repo.path().join("README.md"), "unique-literal\n").unwrap();
    fs::write(repo.path().join("huge.js"), "x".repeat(1_000_001)).unwrap();
    fs::create_dir(repo.path().join("secrets")).unwrap();
    fs::write(
        repo.path().join("secrets/key.ts"),
        "export const sensitive = 1;",
    )
    .unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let index = manager.refresh().unwrap();
    assert_eq!(index.files.len(), 2);
    assert!(!index.warnings.is_empty());
    let result = manager
        .query("code_query", &json!({"query":"\"unique-literal\""}))
        .unwrap();
    assert_eq!(result["total"], result["results"].as_array().unwrap().len());
    assert_eq!(result["results"][0]["source"], "text");
}

#[test]
fn repo_intelligence_monorepo_aliases_and_cycle_distances() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("packages/core/src")).unwrap();
    fs::create_dir_all(repo.path().join("apps/web/src")).unwrap();
    fs::write(
        repo.path().join("packages/core/package.json"),
        r#"{"name":"@app/core","exports":"./src/index.ts"}"#,
    )
    .unwrap();
    fs::write(
        repo.path().join("packages/core/src/index.ts"),
        "export function core() {}",
    )
    .unwrap();
    fs::write(repo.path().join("apps/web/tsconfig.json"), r##"{"compilerOptions":{"baseUrl":".","paths":{"#core":["../../packages/core/src/index.ts"]}}}"##).unwrap();
    fs::write(
        repo.path().join("apps/web/src/a.ts"),
        "import { core } from '#core'; import './b'; core();",
    )
    .unwrap();
    fs::write(
        repo.path().join("apps/web/src/b.ts"),
        "import '@app/core'; import './a';",
    )
    .unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let deps = manager
        .query("file_dependencies", &json!({"path":"apps/web/src/a.ts"}))
        .unwrap();
    assert!(deps["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["target"] == "packages/core/src/index.ts" && r["external"] == false));
    let deps = manager
        .query("file_dependencies", &json!({"path":"apps/web/src/b.ts"}))
        .unwrap();
    assert!(deps["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["target"] == "packages/core/src/index.ts"));
    let related = manager
        .query(
            "related_files",
            &json!({"path":"packages/core/src/index.ts"}),
        )
        .unwrap();
    assert!(related["results"].as_array().unwrap().len() <= 25);
}

#[test]
fn repo_intelligence_query_path_impact_and_partial_evidence() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(repo.path().join("core.ts"), "export function core() {}\n").unwrap();
    fs::write(
        repo.path().join("core.test.ts"),
        "// test pair without import\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("app.ts"),
        "import {core} from './core'; core();\n",
    )
    .unwrap();
    fs::write(repo.path().join("README.md"), "needle\nneedle\nneedle\n").unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let outline = manager
        .query("file_symbols", &json!({"path":"./core.ts"}))
        .unwrap();
    assert!(!outline["results"].as_array().unwrap().is_empty());
    let result = manager
        .query("code_query", &json!({"query":"dependencies of app.ts"}))
        .unwrap();
    assert_eq!(result["route"], "dependency");
    let result = manager
        .query("code_query", &json!({"query":"impact of core"}))
        .unwrap();
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["path"] == "core.test.ts"));
    let result = manager
        .query("code_query", &json!({"query":"\"needle\"", "limit":1}))
        .unwrap();
    assert_eq!(result["truncated"], true);
    assert_eq!(result["text_search_partial"], true);
    assert!(manager
        .query("related_files", &json!({"symbolId":"unknown"}))
        .is_err());
}

#[test]
fn repo_intelligence_export_aliases_and_barrel_calls() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(
        repo.path().join("core.ts"),
        "function local() {} export {local as publicFn}; export default local;",
    )
    .unwrap();
    fs::write(
        repo.path().join("index.ts"),
        "export {publicFn as api} from './core';",
    )
    .unwrap();
    fs::write(repo.path().join("use.ts"), "import {api} from './index'; import main from './core'; export function run() { api(); main(); }").unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let index = manager.refresh().unwrap();
    let symbol = index.files["use.ts"]
        .symbols
        .iter()
        .find(|s| s.name == "run")
        .unwrap();
    let result = manager
        .query("symbol_relationships", &json!({"symbolId":symbol.id}))
        .unwrap();
    let calls: Vec<_> = result["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "calls")
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|r| r["resolved"] == true));
}

#[cfg(unix)]
#[test]
fn repo_intelligence_symlink_source_and_ignore_never_escape() {
    use std::os::unix::fs::symlink;
    let repo = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join("private.ts"),
        "export const privateData = 1;",
    )
    .unwrap();
    fs::write(outside.path().join("ignore"), "*.ts").unwrap();
    symlink(
        outside.path().join("private.ts"),
        repo.path().join("escape.ts"),
    )
    .unwrap();
    fs::create_dir(repo.path().join("sub")).unwrap();
    symlink(
        outside.path().join("ignore"),
        repo.path().join("sub/.gitignore"),
    )
    .unwrap();
    fs::write(repo.path().join("sub/safe.ts"), "let safe;").unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    assert!(manager.refresh().unwrap().files.is_empty());
    assert!(manager.validate_path("escape.ts").is_err());
}
