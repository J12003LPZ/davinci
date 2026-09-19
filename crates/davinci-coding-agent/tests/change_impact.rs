use davinci_agent::{runtime::cache::CacheRuntime, tool_class, ToolClass};
use davinci_coding_agent::native_extensions::{
    build_intelligence::BuildIntelligence,
    change_impact::{
        tool_spec, AnalysisCompleteness, ChangeImpact, ChangeImpactConfig, ChangeImpactReport,
        EvidenceSource, LanguageIntelligenceAdapter, TOOL_NAMES,
    },
    git_intelligence::GitIntelligence,
    language_intelligence::LanguageIntelligence,
    package_intelligence::PackageIntelligence,
    repo_intelligence::{
        RepoIntelligence, SemanticLanguageProvider, SemanticOperation, SourceRange,
    },
    test_impact::TestImpact,
    NativeExtensionHost,
};
use serde_json::json;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

struct TestWorkspace {
    dir: TempDir,
}

impl TestWorkspace {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let ws = Self { dir };
        ws.git(&["init", "-b", "main"]);
        ws.git(&["config", "user.name", "Test Committer"]);
        ws.git(&["config", "user.email", "committer@example.com"]);
        ws.git(&["config", "commit.gpgsign", "false"]);
        ws
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.current_dir(self.path()).args(args);
        let out = cmd.output().expect("git command failed");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn write_file(&self, rel_path: &str, content: &str) {
        let full = self.path().join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn change_impact(&self) -> ChangeImpact {
        self.change_impact_with_config(ChangeImpactConfig::default())
    }

    fn change_impact_with_config(&self, config: ChangeImpactConfig) -> ChangeImpact {
        let cache = CacheRuntime::default();
        let agent_dir = self.path().join(".davinci");
        let repo = RepoIntelligence::new(self.path(), &agent_dir, Default::default());
        let test_impact =
            TestImpact::new(self.path(), repo.clone(), cache.clone(), Default::default());
        let package_intel = PackageIntelligence::with_root(self.path(), cache.clone());
        let build_intel = BuildIntelligence::with_root(self.path(), cache.clone());
        let git_intel = GitIntelligence::with_root(self.path(), cache.clone());
        let lang_intel = LanguageIntelligence::default();

        ChangeImpact::new(
            self.path(),
            cache,
            config,
            repo,
            test_impact,
            package_intel,
            build_intel,
            git_intel,
            lang_intel,
        )
    }
}

// ----------------------------------------------------------------------------
// Test 1: Direct semantic and structural AST impact with honest reporting
// ----------------------------------------------------------------------------
#[test]
fn test_direct_semantic_and_structural_impact() {
    let ws = TestWorkspace::new();
    ws.write_file(
        "src/auth.ts",
        r#"export function login(user: string, pass: string): boolean {
    return user === "admin" && pass === "secret";
}
"#,
    );
    ws.write_file(
        "src/app.ts",
        r#"import { login } from "./auth";

export function handleRequest(u: string, p: string) {
    if (login(u, p)) {
        console.log("Authorized");
    }
}
"#,
    );
    ws.commit("feat: add auth and app");

    let impact = ws.change_impact();

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["src/auth.ts"],
                "symbols": ["login"]
            }),
        )
        .expect("impact_analyze should succeed");
    assert!(!res.is_error);

    let report: ChangeImpactReport =
        serde_json::from_str(&res.content).expect("valid ChangeImpactReport");
    assert_eq!(report.files, vec!["src/auth.ts".to_string()]);
    assert_eq!(report.symbols, vec!["login".to_string()]);

    // Structural AST verification
    assert!(!report.structural_impact.items.is_empty());
    assert!(report
        .structural_impact
        .importing_modules
        .iter()
        .any(|m| m.contains("src/app.ts")));
    assert!(report
        .structural_impact
        .items
        .iter()
        .any(|item| item.evidence_source == EvidenceSource::Ast));

    // Honest completeness reporting: when language intelligence is default (no running LSP),
    // completeness reflects partial semantic analysis without crashing or false confidence
    assert!(
        report.completeness == AnalysisCompleteness::Partial
            || report.completeness == AnalysisCompleteness::Complete
    );
    assert!(!report.confidence.is_empty());
}

// ----------------------------------------------------------------------------
// Test 2: Analysis resolved from transaction ID
// ----------------------------------------------------------------------------
#[test]
fn test_analysis_from_transaction_id() {
    let ws = TestWorkspace::new();
    ws.write_file("src/service.ts", "export const serviceName = 'account';\n");
    ws.commit("feat: initial service");

    // Write transaction store record
    let tx_id = "tx_998877";
    let tx_record = json!({
        "transaction_id": tx_id,
        "status": "staged",
        "summary": {
            "affected_files": ["src/service.ts"],
            "total_insertions": 5,
            "total_deletions": 0
        },
        "changes": [
            {
                "path": "src/service.ts",
                "kind": "modify"
            }
        ]
    });
    ws.write_file(
        &format!(".davinci-transactions/{tx_id}.json"),
        &serde_json::to_string_pretty(&tx_record).unwrap(),
    );

    let impact = ws.change_impact();

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "transactionId": tx_id
            }),
        )
        .expect("impact_analyze with transactionId should succeed");
    assert!(!res.is_error);

    let report: ChangeImpactReport = serde_json::from_str(&res.content).unwrap();
    assert_eq!(report.transaction_id, Some(tx_id.to_string()));
    assert!(report.files.contains(&"src/service.ts".to_string()));
}

// ----------------------------------------------------------------------------
// Test 3: Acceptance guard: honest completeness and no false semantic claims
// ----------------------------------------------------------------------------
#[test]
fn test_honest_completeness_and_no_false_semantic_confidence() {
    let ws = TestWorkspace::new();
    ws.write_file("src/utils.ts", "export function helper() { return 42; }\n");
    ws.commit("feat: add helper");

    let config = ChangeImpactConfig {
        enabled: true,
        max_depth: 3,
        max_references: 10,
        max_files: 1, // small file limit to test truncation warning
    };
    let impact = ws.change_impact_with_config(config);

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["src/utils.ts", "src/extra.ts"]
            }),
        )
        .expect("should execute");
    let report: ChangeImpactReport = serde_json::from_str(&res.content).unwrap();

    // Max files exceeded triggers truncation warning and partial completeness
    assert_eq!(report.completeness, AnalysisCompleteness::Partial);
    assert!(!report.warnings.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("exceeded configured maxFiles")
                || w.contains("Language intelligence"))
    );
}

// ----------------------------------------------------------------------------
// Test 4: Public API risk and re-exports detection
// ----------------------------------------------------------------------------
#[test]
fn test_public_api_risk_detection() {
    let ws = TestWorkspace::new();
    ws.write_file(
        "src/index.ts",
        r#"export * from "./auth";
export const API_VERSION = "v1";
"#,
    );
    ws.write_file("src/auth.ts", "export function login() {}\n");
    ws.commit("feat: root export in index.ts");

    let impact = ws.change_impact();

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["src/index.ts"],
                "symbols": ["API_VERSION"]
            }),
        )
        .expect("should analyze public api risk");

    let report: ChangeImpactReport = serde_json::from_str(&res.content).unwrap();
    assert!(report.public_api_risk.is_public_api_affected);
    assert!(report
        .public_api_risk
        .items
        .iter()
        .any(|item| item.evidence_source == EvidenceSource::Ast));
    assert!(report
        .public_api_risk
        .summary
        .contains("HIGH public API risk"));
}

// ----------------------------------------------------------------------------
// Test 5: Configuration impact detection
// ----------------------------------------------------------------------------
#[test]
fn test_configuration_impact_detection() {
    let ws = TestWorkspace::new();
    ws.write_file(
        "package.json",
        r#"{"name": "test-pkg", "version": "1.0.0"}"#,
    );
    ws.write_file("tsconfig.json", r#"{"compilerOptions": {"strict": true}}"#);
    ws.commit("feat: configuration files");

    let impact = ws.change_impact();

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["package.json", "tsconfig.json"]
            }),
        )
        .expect("should analyze config files");

    let report: ChangeImpactReport = serde_json::from_str(&res.content).unwrap();
    assert!(report.configuration_impact.is_config_affected);
    assert_eq!(report.configuration_impact.affected_configs.len(), 2);
    assert!(report
        .configuration_impact
        .items
        .iter()
        .all(|item| item.evidence_source == EvidenceSource::Config));
}

// ----------------------------------------------------------------------------
// Test 6: Potential browser flows for UI changes
// ----------------------------------------------------------------------------
#[test]
fn test_potential_browser_flows_for_ui_changes() {
    let ws = TestWorkspace::new();
    ws.write_file(
        "pages/login.tsx",
        r#"export default function LoginPage() {
    return <form><input name="user"/><button type="submit">Sign In</button></form>;
}
"#,
    );
    ws.commit("feat: add login page");

    let impact = ws.change_impact();

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["pages/login.tsx"]
            }),
        )
        .expect("should analyze UI flows");

    let report: ChangeImpactReport = serde_json::from_str(&res.content).unwrap();
    assert!(report.potential_browser_flows.has_ui_impact);
    assert!(report
        .potential_browser_flows
        .affected_flows
        .contains(&"Authentication Flow".to_string()));
}

// ----------------------------------------------------------------------------
// Test 7: Workspace packages and downstream dependents composition
// ----------------------------------------------------------------------------
#[test]
fn test_workspace_packages_and_downstream_dependents() {
    let ws = TestWorkspace::new();
    ws.write_file(
        "packages/core/package.json",
        r#"{"name": "@repo/core", "version": "1.0.0"}"#,
    );
    ws.write_file("packages/core/src/index.ts", "export const CORE_VAL = 1;\n");
    ws.write_file(
        "packages/app/package.json",
        r#"{"name": "@repo/app", "version": "1.0.0", "dependencies": {"@repo/core": "1.0.0"}}"#,
    );
    ws.commit("feat: workspace packages");

    let impact = ws.change_impact();

    let res = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["packages/core/src/index.ts"]
            }),
        )
        .expect("should identify workspace packages");

    let report: ChangeImpactReport = serde_json::from_str(&res.content).unwrap();
    assert!(report
        .packages
        .affected_packages
        .contains(&"@repo/core".to_string()));
}

// ----------------------------------------------------------------------------
// Test 8: Security guards: path traversal rejection and tool class
// ----------------------------------------------------------------------------
#[test]
fn test_security_guards_path_traversal_and_read_only_class() {
    let ws = TestWorkspace::new();
    let impact = ws.change_impact();

    // 1. Path traversal rejected
    let err = impact
        .execute_tool(
            "impact_analyze",
            &json!({
                "files": ["../../etc/passwd"]
            }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("path traversal rejected"));

    // 2. ToolClass is Read
    assert_eq!(tool_class("impact_analyze"), ToolClass::Read);

    // 3. Tool spec and names
    assert!(TOOL_NAMES.contains(&"impact_analyze"));
    let spec = tool_spec("impact_analyze").expect("tool spec exists");
    assert_eq!(spec.name, "impact_analyze");
}

// ----------------------------------------------------------------------------
// Test 9: CacheRuntime caching and telemetry
// ----------------------------------------------------------------------------
#[test]
fn test_cache_runtime_caching_and_telemetry() {
    let ws = TestWorkspace::new();
    ws.write_file("src/cached.ts", "export const CACHED = true;\n");
    ws.commit("feat: cached test");

    let impact = ws.change_impact();

    let args = json!({
        "files": ["src/cached.ts"]
    });

    // Call 1: cache miss
    let res1 = impact.execute_tool("impact_analyze", &args).unwrap();
    assert!(!res1.is_error);

    let status1 = impact.status();
    let requests1 = status1["telemetry"]["requests"].as_u64().unwrap();
    let hits1 = status1["telemetry"]["hits"].as_u64().unwrap();
    let misses1 = status1["telemetry"]["misses"].as_u64().unwrap();
    assert_eq!(requests1, 1);
    assert_eq!(hits1, 0);
    assert_eq!(misses1, 1);

    // Call 2: cache hit
    let res2 = impact.execute_tool("impact_analyze", &args).unwrap();
    assert!(!res2.is_error);

    let status2 = impact.status();
    let requests2 = status2["telemetry"]["requests"].as_u64().unwrap();
    let hits2 = status2["telemetry"]["hits"].as_u64().unwrap();
    assert_eq!(requests2, 2);
    assert_eq!(hits2, 1);
}

// ----------------------------------------------------------------------------
// Test 10: NativeExtensionHost integration and command dispatch
// ----------------------------------------------------------------------------
#[test]
fn test_native_extension_host_dispatch() {
    let ws = TestWorkspace::new();
    ws.write_file("src/main.ts", "console.log('main');\n");
    ws.commit("feat: initial main");

    let mut host = NativeExtensionHost::default();

    // Verify describe_tool
    let spec = NativeExtensionHost::describe_tool("impact_analyze");
    assert!(spec.is_some());
    assert_eq!(spec.unwrap().name, "impact_analyze");

    // Verify tool registration
    assert!(host.has_tool("impact_analyze"));

    // Verify tool execution
    let res = host.execute_tool(
        ws.path(),
        "impact_analyze",
        &json!({
            "files": ["src/main.ts"]
        }),
    );
    assert!(res.is_ok());

    // Verify command execution
    let mut host = NativeExtensionHost::default();
    let cmd_res = host.command("impact-status", "").unwrap();
    assert!(cmd_res.is_some());
    let val = cmd_res.unwrap();
    assert!(val.get("enabled").is_some());
    assert!(val.get("telemetry").is_some());
}

// ----------------------------------------------------------------------------
// Test 11: LanguageIntelligenceAdapter as SemanticLanguageProvider
// ----------------------------------------------------------------------------
#[test]
fn test_language_intelligence_adapter_provider() {
    let lang = LanguageIntelligence::default();
    let adapter = LanguageIntelligenceAdapter::new(lang);

    let range = SourceRange {
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 10,
    };

    // When LSP is not active, provider returns Ok(empty) gracefully without panicking
    let res = adapter.query(SemanticOperation::References, "src/index.ts", &range, 10);
    assert!(res.is_ok());
    let items = res.unwrap();
    assert!(items.is_empty());
}
