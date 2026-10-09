//! Regressions for the Change Impact audit findings (WOR-260..264).
use davinci_agent::runtime::cache::CacheRuntime;
use davinci_coding_agent::native_extensions::{
    build_intelligence::BuildIntelligence,
    change_impact::{ChangeImpact, ChangeImpactConfig, ChangeImpactReport},
    git_intelligence::GitIntelligence,
    language_intelligence::LanguageIntelligence,
    package_intelligence::PackageIntelligence,
    repo_intelligence::RepoIntelligence,
    test_impact::TestImpact,
    NativeExtensionHost,
};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

struct Workspace {
    dir: TempDir,
}

impl Workspace {
    fn new() -> Self {
        let ws = Self {
            dir: TempDir::new().unwrap(),
        };
        ws.git(&["init", "-b", "main"]);
        ws.git(&["config", "user.name", "Test Committer"]);
        ws.git(&["config", "user.email", "committer@example.com"]);
        ws.git(&["config", "commit.gpgsign", "false"]);
        ws
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) {
        Command::new("git")
            .current_dir(self.path())
            .args(args)
            .output()
            .expect("git");
    }

    fn write(&self, rel: &str, content: &str) {
        let full = self.path().join(rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", "snapshot"]);
    }

    fn analyze(&self, args: Value) -> Result<ChangeImpactReport, String> {
        let cache = CacheRuntime::default();
        let agent_dir = self.path().join(".davinci");
        let repo = RepoIntelligence::new(self.path(), &agent_dir, Default::default());
        let test_impact =
            TestImpact::new(self.path(), repo.clone(), cache.clone(), Default::default());
        let impact = ChangeImpact::new(
            self.path(),
            cache.clone(),
            ChangeImpactConfig::default(),
            repo,
            test_impact,
            PackageIntelligence::with_root(self.path(), cache.clone()),
            BuildIntelligence::with_root(self.path(), cache.clone()),
            GitIntelligence::with_root(self.path(), cache),
            LanguageIntelligence::default(),
        );
        impact
            .execute_tool("impact_analyze", &args)
            .map_err(|err| err.to_string())
            .map(|res| serde_json::from_str(&res.content).expect("valid report"))
    }
}

fn tested_workspace() -> Workspace {
    let ws = Workspace::new();
    ws.write(
        "package.json",
        r#"{"name":"app","scripts":{"test":"vitest run"},"devDependencies":{"vitest":"1"}}"#,
    );
    ws.write("src/util.ts", "export function answer() { return 1; }\n");
    ws.write(
        "src/util.test.ts",
        "import { answer } from './util';\nanswer();\n",
    );
    ws.commit();
    ws
}

#[test]
fn test_impact_results_reach_the_report() {
    let ws = tested_workspace();
    let report = ws.analyze(json!({"files": ["src/util.ts"]})).unwrap();
    assert!(
        report
            .tests
            .selected_tests
            .contains(&"src/util.test.ts".to_string()),
        "{:?}",
        report.tests
    );
}

#[test]
fn symbol_names_are_resolved_to_test_impact_ids() {
    let ws = tested_workspace();
    let report = ws.analyze(json!({"symbols": ["answer"]})).unwrap();
    assert!(
        report
            .tests
            .selected_tests
            .contains(&"src/util.test.ts".to_string()),
        "{:?} / {:?}",
        report.tests,
        report.warnings
    );
    let unknown = ws
        .analyze(json!({"files": ["src/util.ts"], "symbols": ["noSuchSymbol"]}))
        .unwrap();
    assert!(
        unknown.warnings.iter().any(|w| w.contains("noSuchSymbol")),
        "{:?}",
        unknown.warnings
    );
}

#[test]
fn scope_limits_the_analyzed_inputs() {
    let ws = Workspace::new();
    ws.write("apps/a/one.ts", "export const one = 1;\n");
    ws.write("apps/b/two.ts", "export const two = 2;\n");
    ws.commit();
    let report = ws
        .analyze(json!({"files": ["apps/a/one.ts", "apps/b/two.ts"], "scope": "apps/a"}))
        .unwrap();
    assert_eq!(report.files, vec!["apps/a/one.ts".to_string()]);
    assert!(report.warnings.iter().any(|w| w.contains("outside scope")));
    assert!(ws
        .analyze(json!({"files": ["apps/b/two.ts"], "scope": "apps/a"}))
        .is_err());
}

#[test]
fn uncommitted_edits_are_the_default_change_set() {
    let ws = Workspace::new();
    ws.write("src/a.ts", "export const a = 1;\n");
    ws.write("src/b.ts", "export const b = 1;\n");
    ws.commit();
    ws.write("src/a.ts", "export const a = 2;\n");
    ws.write("src/new.ts", "export const fresh = 1;\n");
    let report = ws.analyze(json!({})).unwrap();
    assert!(
        report.files.contains(&"src/a.ts".to_string()),
        "{:?}",
        report.files
    );
    assert!(
        report.files.contains(&"src/new.ts".to_string()),
        "{:?}",
        report.files
    );
    assert!(!report.files.contains(&"src/b.ts".to_string()));
}

#[test]
fn imports_match_by_resolved_module_not_by_name_suffix() {
    let ws = Workspace::new();
    ws.write("src/utils.ts", "export const u = 1;\n");
    ws.write("src/otherutils.ts", "export const o = 1;\n");
    ws.write("src/a.ts", "import { u } from './utils';\nu;\n");
    ws.commit();
    let report = ws.analyze(json!({"files": ["src/otherutils.ts"]})).unwrap();
    assert!(
        !report
            .structural_impact
            .importing_modules
            .contains(&"src/a.ts".to_string()),
        "{:?}",
        report.structural_impact.importing_modules
    );
    let real = ws.analyze(json!({"files": ["src/utils.ts"]})).unwrap();
    assert!(real
        .structural_impact
        .importing_modules
        .contains(&"src/a.ts".to_string()));
}
