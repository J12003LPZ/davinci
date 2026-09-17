use davinci_coding_agent::native_extensions::repo_intelligence::{parse_source, LanguageAdapter};
use davinci_coding_agent::native_extensions::repo_intelligence::{
    RepoIntelligence, RepoIntelligenceConfig,
};
use std::fs;

#[test]
fn repo_intelligence_native_host_registration_and_execution() {
    use davinci_coding_agent::native_extensions::NativeExtensionHost;
    use serde_json::json;
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(repo.path().join("entry.ts"), "export function hello() {}").unwrap();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("repo-test", repo.path(), Some(cache.path()));
    for name in [
        "repo_map",
        "symbol_search",
        "file_symbols",
        "file_dependencies",
        "symbol_relationships",
        "related_files",
        "code_query",
    ] {
        assert!(host.has_tool(name), "{name} missing");
        assert!(host.tool_names().contains(&name.to_string()));
        assert_eq!(
            NativeExtensionHost::describe_tool(name).unwrap().parameters["type"],
            "object"
        );
        assert_eq!(
            davinci_agent::tool_class(name),
            davinci_agent::ToolClass::Read
        );
    }
    let result = host
        .execute_tool(repo.path(), "symbol_search", &json!({"query":"hello"}))
        .unwrap();
    assert!(result.content.contains("hello"));
    assert!(host.command("repo-index-status", "").unwrap().is_some());
}

#[test]
fn repo_intelligence_native_output_retention_and_disabled_settings() {
    use davinci_coding_agent::native_extensions::{
        NativeExtensionHost, OutputStore, TokenGovernor, TokenGovernorConfig,
    };
    use serde_json::json;
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    use std::fmt::Write;
    let mut source = String::new();
    for n in 0..40 {
        writeln!(source, "export function function{n}() {{ return {n}; }}").unwrap();
    }
    fs::write(repo.path().join("entry.ts"), source).unwrap();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("retention-test", repo.path(), Some(cache.path()));
    let store = OutputStore::new(cache.path().join("outputs/retention-test"));
    host.governor = TokenGovernor::new(
        "retention-test",
        TokenGovernorConfig {
            compress_threshold_bytes: 50,
            compress_threshold_lines: 1,
            store_dir: Some(cache.path().to_path_buf()),
            ..Default::default()
        },
    );
    let args = json!({"path":"entry.ts"});
    let original = host
        .execute_tool(repo.path(), "file_symbols", &args)
        .unwrap();
    let content = original.content.clone();
    let processed = host.after_tool("file_symbols", &args, original);
    let id = processed.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
        .as_str()
        .unwrap();
    assert!(processed.content.contains("retrieve_output"));
    assert_eq!(store.load(id).unwrap(), content);
    fs::write(
        cache.path().join("settings.json"),
        r#"{"repoIntelligence":{"enabled":false}}"#,
    )
    .unwrap();
    let mut disabled =
        NativeExtensionHost::new_with_agent_dir("disabled-test", repo.path(), Some(cache.path()));
    assert!(disabled
        .execute_tool(repo.path(), "repo_map", &json!({}))
        .is_err());
}

#[test]
fn repo_intelligence_incremental_cache_boundaries_and_corruption() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(repo.path().join("a.ts"), "export function alpha() {}").unwrap();
    fs::write(repo.path().join("b.js"), "export const beta = 1;").unwrap();
    fs::write(repo.path().join(".gitignore"), "ignored.ts\n").unwrap();
    fs::write(repo.path().join("ignored.ts"), "const secret = 1;").unwrap();
    fs::create_dir(repo.path().join("node_modules")).unwrap();
    fs::write(repo.path().join("node_modules/x.js"), "let nope;").unwrap();
    let manager =
        RepoIntelligence::new(repo.path(), cache.path(), RepoIntelligenceConfig::default());
    assert!(!manager.status()["initialized"].as_bool().unwrap());
    let first = manager.refresh().unwrap();
    assert_eq!(first.files.len(), 2);
    assert_eq!(first.reparsed, 2);
    assert_eq!(manager.refresh().unwrap().reparsed, 0);
    // Same-length replacement, irrespective of timestamp granularity.
    fs::write(repo.path().join("a.ts"), "export function omega() {}").unwrap();
    let changed = manager.refresh().unwrap();
    assert_eq!(changed.reparsed, 1);
    assert_eq!(changed.files["a.ts"].symbols[0].name, "omega");
    fs::rename(repo.path().join("a.ts"), repo.path().join("new.ts")).unwrap();
    fs::remove_file(repo.path().join("b.js")).unwrap();
    assert_eq!(
        manager
            .refresh()
            .unwrap()
            .files
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["new.ts"]
    );
    let cache_path = manager.cache_path().unwrap();
    drop(manager);
    let reloaded =
        RepoIntelligence::new(repo.path(), cache.path(), RepoIntelligenceConfig::default());
    assert_eq!(reloaded.refresh().unwrap().reparsed, 0);
    drop(reloaded);
    fs::write(cache_path, "corrupt").unwrap();
    let recovered =
        RepoIntelligence::new(repo.path(), cache.path(), RepoIntelligenceConfig::default());
    assert_eq!(recovered.refresh().unwrap().reparsed, 1);
    assert!(recovered.validate_path("../outside.ts").is_err());
    assert!(recovered.validate_path("node_modules/x.js").is_err());
}

#[test]
fn repo_intelligence_concurrent_managers_share_one_snapshot() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(repo.path().join("a.ts"), "export function alpha() {}").unwrap();
    let managers: Vec<_> = (0..8)
        .map(|_| RepoIntelligence::new(repo.path(), cache.path(), Default::default()))
        .collect();
    let results: Vec<_> = std::thread::scope(|scope| {
        managers
            .iter()
            .map(|m| scope.spawn(|| m.refresh().unwrap()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });
    assert!(results.iter().all(|r| r.files.len() == 1));
    assert_eq!(results.iter().map(|r| r.reparsed).sum::<usize>(), 1);
}

#[test]
fn repo_intelligence_queries_dependencies_related_files_and_provenance() {
    use serde_json::json;
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("src")).unwrap();
    fs::create_dir_all(repo.path().join("tests")).unwrap();
    fs::write(
        repo.path().join("src/session.ts"),
        "export function createSession() { return 'session-literal'; }",
    )
    .unwrap();
    fs::write(repo.path().join("src/auth.ts"), "import { createSession } from './session'; export class AuthService { login() { return createSession(); } }").unwrap();
    fs::write(
        repo.path().join("tests/session.test.ts"),
        "import { createSession } from '../src/session'; createSession();",
    )
    .unwrap();
    fs::write(repo.path().join("package.json"), "{}").unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let search = manager
        .query("symbol_search", &json!({"query":"AuthService"}))
        .unwrap();
    assert_eq!(search["results"][0]["symbol"]["name"], "AuthService");
    assert_eq!(search["results"][0]["source"], "ast");
    let deps = manager
        .query("file_dependencies", &json!({"path":"src/session.ts"}))
        .unwrap();
    assert!(deps.to_string().contains("src/auth.ts"));
    let related = manager
        .query("related_files", &json!({"path":"src/session.ts"}))
        .unwrap();
    assert!(related.to_string().contains("tests/session.test.ts"));
    assert!(related.to_string().contains("direct importer"));
    let map = manager.query("repo_map", &json!({})).unwrap();
    assert!(map.to_string().contains("package.json"));
    let exact = manager
        .query(
            "code_query",
            &json!({"query":"Where is AuthService defined?"}),
        )
        .unwrap();
    assert_eq!(exact["route"], "symbol");
    let literal = manager
        .query("code_query", &json!({"query":"\"session-literal\""}))
        .unwrap();
    assert_eq!(literal["results"][0]["source"], "text");
    let bounded = manager
        .query("symbol_search", &json!({"query":"s", "limit":1}))
        .unwrap();
    assert!(bounded["results"].as_array().unwrap().len() <= 1);
    assert!(manager
        .query("symbol_search", &json!({"query":"a", "limit":0}))
        .is_err());
    assert!(manager
        .query("file_symbols", &json!({"path":"../outside.ts"}))
        .is_err());
}

#[test]
fn repo_intelligence_parser_typescript_structure_and_stable_ids() {
    let source = r#"
import { token } from './token';
export { Token } from './token';
export interface Provider { login(): string }
export type Session = { id: string };
export enum State { Open, Closed }
export class AuthService extends Base implements Provider {
  session: Session;
  login(): string { return token(); }
}
export function createSession(id: string): Session { return { id }; }
export const active = (id: string) => createSession(id);
"#;
    let file = parse_source("src/auth.ts", source).unwrap();
    assert_eq!(file.language, LanguageAdapter::TypeScript);
    assert_eq!(file.parse_status, "ok");
    for (name, kind) in [
        ("Provider", "interface"),
        ("Session", "type_alias"),
        ("State", "enum"),
        ("AuthService", "class"),
        ("login", "method"),
        ("createSession", "function"),
        ("active", "function"),
    ] {
        assert!(
            file.symbols
                .iter()
                .any(|s| s.name == name && s.kind == kind),
            "{name}: {:?}",
            file.symbols
        );
    }
    assert!(file.imports.iter().any(|i| i.specifier == "./token"));
    assert!(file.edges.iter().any(|e| e.kind == "reexports"));
    assert!(file
        .edges
        .iter()
        .any(|e| e.kind == "extends" && e.target == "Base"));
    assert!(file
        .edges
        .iter()
        .any(|e| e.kind == "implements" && e.target == "Provider"));
    assert!(file
        .edges
        .iter()
        .any(|e| e.kind == "calls" && e.target == "createSession"));
    let shifted = parse_source("src/auth.ts", &format!("\n// moved\n{source}")).unwrap();
    assert_eq!(
        file.symbols.iter().map(|s| &s.id).collect::<Vec<_>>(),
        shifted.symbols.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
}

#[test]
fn repo_intelligence_parser_all_extensions_and_dynamic_limits() {
    for extension in ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"] {
        let source = "const dep = require('./dep'); export function run() { return dep(); }";
        let file = parse_source(&format!("module.{extension}"), source).unwrap();
        assert!(file.symbols.iter().any(|s| s.name == "run"));
        assert!(file.imports.iter().any(|i| i.specifier == "./dep"));
    }
    for extension in ["tsx", "jsx"] {
        let file = parse_source(
            &format!("view.{extension}"),
            "export const View = () => <div>Hello</div>;",
        )
        .unwrap();
        assert_eq!(file.parse_status, "ok");
    }
    let dynamic = parse_source("dynamic.js", "require(name); import(path); obj[key]();").unwrap();
    assert!(!dynamic.unresolved.is_empty());
    assert!(dynamic.imports.is_empty());
    assert!(parse_source("lib.rs", "fn main() {}").is_err());
    assert_ne!(
        parse_source("broken.ts", "export function broken( {")
            .unwrap()
            .parse_status,
        "ok"
    );
}
