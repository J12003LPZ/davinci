use davinci_agent::{
    runtime::cache::{CacheConfig, CacheRuntime},
    tool_class, ToolClass,
};
use davinci_coding_agent::native_extensions::{
    git_intelligence::{
        model::*, resolver::GitResolver, runner, tools::TOOL_NAMES, GitIntelligence,
    },
    NativeExtensionHost,
};
use serde_json::json;
use std::{path::Path, process::Command};
use tempfile::TempDir;

struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let repo = Self { dir };
        repo.git(&["init", "-b", "main"]);
        repo.git(&["config", "user.name", "Test Committer"]);
        repo.git(&["config", "user.email", "committer@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
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
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

// ----------------------------------------------------------------------------
// Test 1: Symbol history across modifications and file renames
// ----------------------------------------------------------------------------
#[test]
fn test_symbol_history_with_modifications_and_renames() {
    let repo = TestRepo::new();
    // Commit 1: add src/auth.ts with login function
    repo.write_file(
        "src/auth.ts",
        r#"export function login(user: string, pass: string): boolean {
    return user === "admin" && pass === "secret";
}
"#,
    );
    let c1 = repo.commit("feat(auth): initial login implementation (#10)");

    // Commit 2: modify login function
    repo.write_file(
        "src/auth.ts",
        r#"export function login(user: string, pass: string): boolean {
    const valid = user === "admin" && pass === "secret";
    return valid;
}
"#,
    );
    let _c2 = repo.commit("fix(auth): add local variable to login (#11)");

    // Commit 3: rename src/auth.ts -> src/auth_service.ts
    repo.git(&["mv", "src/auth.ts", "src/auth_service.ts"]);
    let _c3 = repo.commit("refactor(auth): rename auth to auth_service");

    // Commit 4: add logout function
    repo.write_file(
        "src/auth_service.ts",
        r#"export function login(user: string, pass: string): boolean {
    const valid = user === "admin" && pass === "secret";
    return valid;
}

export function logout(): void {
    // clear token
}
"#,
    );
    let _c4 = repo.commit("feat(auth): add logout function (#12)");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();
    let res = resolver
        .symbol_history("login", Some("src/auth_service.ts"), None)
        .unwrap();

    assert_eq!(res.symbol, "login");
    assert_eq!(res.file, "src/auth_service.ts");
    assert_eq!(res.introduction_status, IntroductionStatus::Introduced);

    let intro = res.introduced_commit.as_ref().unwrap();
    assert_eq!(intro.commit, c1);
    assert_eq!(intro.author, "Test Committer");
    assert_eq!(intro.change_kind, "introduced");

    // Check that file movement was tracked
    assert_eq!(res.file_movements.len(), 1);
    assert_eq!(res.file_movements[0].from_path, "src/auth.ts");
    assert_eq!(res.file_movements[0].to_path, "src/auth_service.ts");

    // Check facts vs inference separation
    assert!(res.facts.commits_inspected >= 4);
    assert!(res.facts.revisions_with_symbol >= 3);
    assert!(!res.inference.notes.is_empty());
}

// ----------------------------------------------------------------------------
// Test 2: Acceptance guard for shallow and bounded history
// ----------------------------------------------------------------------------
#[test]
fn test_acceptance_guard_shallow_and_bounded_history() {
    let repo = TestRepo::new();
    repo.write_file(
        "src/calc.ts",
        "export function add(a: number, b: number): number { return a + b; }\n",
    );
    let c1 = repo.commit("initial calc");

    repo.write_file(
        "src/calc.ts",
        "export function add(a: number, b: number): number { const sum = a + b; return sum; }\n",
    );
    repo.commit("update calc");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();

    // With max_commits = 1 (truncating history), introduction must report Partial, NOT Introduced
    let res_bounded = resolver
        .symbol_history("add", Some("src/calc.ts"), Some(1))
        .unwrap();
    assert_eq!(res_bounded.introduction_status, IntroductionStatus::Partial);
    assert!(res_bounded.introduced_commit.is_none());

    // With shallow repo (simulated via .git/shallow with valid commit SHA), must report Partial/Unknown
    std::fs::write(repo.path().join(".git").join("shallow"), format!("{c1}\n")).unwrap();
    let res_shallow = resolver
        .symbol_history("add", Some("src/calc.ts"), None)
        .unwrap();
    assert!(matches!(
        res_shallow.introduction_status,
        IntroductionStatus::Partial | IntroductionStatus::Unknown
    ));
}

// ----------------------------------------------------------------------------
// Test 3: Facts vs inference separation in related commits & commit context
// ----------------------------------------------------------------------------
#[test]
fn test_facts_vs_inference_separation() {
    let repo = TestRepo::new();
    repo.write_file("token.ts", "export const token = 'abc';\n");
    let sha = repo.commit("fix(auth): resolve token refresh issue (#42)");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();

    // 1. related_commits
    let related = resolver
        .related_commits(Some("token"), None, None, None)
        .unwrap();
    assert_eq!(related.total_commits, 1);
    let c = &related.commits[0];
    // Objective facts:
    assert_eq!(c.commit, sha);
    assert_eq!(c.facts.files_changed, 1);
    assert_eq!(c.facts.insertions, 1);
    // Inferences:
    assert_eq!(c.inference.pr_number, Some(42));
    assert!(c
        .inference
        .intent_summary
        .as_ref()
        .unwrap()
        .contains("bug fix"));

    // 2. commit_context
    let context = resolver.commit_context(&sha).unwrap();
    assert_eq!(context.commit, sha);
    assert_eq!(context.facts.files_changed, vec!["token.ts"]);
    assert_eq!(context.facts.insertions, 1);
    assert_eq!(context.inference.pr_reference, Some("#42".to_string()));
    assert!(context.inference.intent_summary.contains("bug fix"));
}

// ----------------------------------------------------------------------------
// Test 4: AST changed symbols between commits
// ----------------------------------------------------------------------------
#[test]
fn test_changed_symbols_ast_comparison() {
    let repo = TestRepo::new();
    repo.write_file(
        "src/user.ts",
        r#"export function getUser(): string { return "user"; }
export function deleteUser(): void {}
"#,
    );
    let base_sha = repo.commit("base commit");

    repo.write_file(
        "src/user.ts",
        r#"export function getUser(): string {
    // updated body
    return "updated_user";
}
export function createUser(): void {}
"#,
    );
    let head_sha = repo.commit("head commit");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();
    let res = resolver
        .changed_symbols(Some(&base_sha), Some(&head_sha), None)
        .unwrap();

    assert_eq!(res.base, base_sha);
    assert_eq!(res.head, head_sha);

    let added: Vec<_> = res
        .symbols
        .iter()
        .filter(|s| s.change_type == "added")
        .collect();
    let deleted: Vec<_> = res
        .symbols
        .iter()
        .filter(|s| s.change_type == "deleted")
        .collect();
    let modified: Vec<_> = res
        .symbols
        .iter()
        .filter(|s| s.change_type == "modified")
        .collect();

    assert!(added.iter().any(|s| s.name == "createUser"));
    assert!(deleted.iter().any(|s| s.name == "deleteUser"));
    assert!(modified.iter().any(|s| s.name == "getUser"));
}

// ----------------------------------------------------------------------------
// Test 5: Branch diff with merge-base and stats
// ----------------------------------------------------------------------------
#[test]
fn test_branch_diff_with_merge_base() {
    let repo = TestRepo::new();
    repo.write_file("main.txt", "main content\n");
    let base_sha = repo.commit("main initial");

    // Create feature branch
    repo.git(&["checkout", "-b", "feature"]);
    repo.write_file("feature.txt", "feature content\n");
    repo.commit("feature commit 1");
    repo.write_file("feature2.txt", "feature 2 content\n");
    let feature_head = repo.commit("feature commit 2");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();
    let diff = resolver
        .branch_diff(Some(&base_sha), Some(&feature_head), Some(false), None)
        .unwrap();

    assert_eq!(diff.commits_ahead, 2);
    assert_eq!(diff.commits_behind, 0);
    assert_eq!(diff.files_changed, 2);
    assert!(diff.insertions >= 2);
    assert!(diff.merge_base.is_some());
    assert_eq!(diff.files.len(), 2);
}

// ----------------------------------------------------------------------------
// Test 6: Blame symbol with porcelain line attribution
// ----------------------------------------------------------------------------
#[test]
fn test_blame_symbol_porcelain() {
    let repo = TestRepo::new();
    repo.write_file(
        "src/util.ts",
        r#"export function calculate(x: number): number {
    const factor = 2;
    return x * factor;
}
"#,
    );
    let sha = repo.commit("add calculate function");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();
    let blame = resolver
        .blame_symbol("calculate", "src/util.ts", None)
        .unwrap();

    assert_eq!(blame.symbol, "calculate");
    assert_eq!(blame.file, "src/util.ts");
    assert!(blame.total_lines >= 4);
    assert_eq!(blame.authors.len(), 1);
    assert_eq!(blame.authors[0].author, "Test Committer");
    assert_eq!(blame.authors[0].percentage, 100.0);
    assert_eq!(blame.most_recent_commit, Some(sha));
}

// ----------------------------------------------------------------------------
// Test 7: Conflict explain on unmerged index stages (zero mutation guaranteed)
// ----------------------------------------------------------------------------
#[test]
fn test_conflict_explain_zero_mutation() {
    let repo = TestRepo::new();
    repo.write_file(
        "src/service.ts",
        r#"export function processData(): string {
<<<<<<< HEAD
    return "ours";
=======
    return "theirs";
>>>>>>> feature
}
"#,
    );

    // Populate unmerged stages 1, 2, 3 in index using git hash-object and update-index
    let mut hash_base = Command::new("git");
    hash_base
        .current_dir(repo.path())
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = hash_base.spawn().unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"base data\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let base_blob = String::from_utf8(out.stdout).unwrap();

    let mut hash_ours = Command::new("git");
    hash_ours
        .current_dir(repo.path())
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = hash_ours.spawn().unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"ours data\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let ours_blob = String::from_utf8(out.stdout).unwrap();

    let mut hash_theirs = Command::new("git");
    hash_theirs
        .current_dir(repo.path())
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = hash_theirs.spawn().unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"theirs data\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let theirs_blob = String::from_utf8(out.stdout).unwrap();

    let index_info = format!(
        "100644 {} 1\tsrc/service.ts\n100644 {} 2\tsrc/service.ts\n100644 {} 3\tsrc/service.ts\n",
        base_blob.trim(),
        ours_blob.trim(),
        theirs_blob.trim()
    );

    let mut update_cmd = Command::new("git");
    update_cmd
        .current_dir(repo.path())
        .args(["update-index", "--index-info"])
        .stdin(std::process::Stdio::piped());
    let mut child = update_cmd.spawn().unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(index_info.as_bytes())
        .unwrap();
    child.wait().unwrap();

    let content_before = std::fs::read_to_string(repo.path().join("src/service.ts")).unwrap();

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();
    let conflict = resolver.conflict_explain(Some("src/service.ts")).unwrap();

    assert_eq!(conflict.total_conflicted_files, 1);
    assert!(conflict.zero_mutation_guaranteed);

    let report = &conflict.files[0];
    assert_eq!(report.path, "src/service.ts");
    assert!(report.has_base_stage);
    assert!(report.has_ours_stage);
    assert!(report.has_theirs_stage);
    assert_eq!(report.conflict_markers_count, 1);
    assert_eq!(report.conflicting_ranges.len(), 1);
    assert!(report
        .overlapping_symbols
        .contains(&"processData".to_string()));

    // Verify ZERO MUTATION: file on disk was untouched
    let content_after = std::fs::read_to_string(repo.path().join("src/service.ts")).unwrap();
    assert_eq!(content_before, content_after);
}

// ----------------------------------------------------------------------------
// Test 8: Security guards: option injection and path traversal
// ----------------------------------------------------------------------------
#[test]
fn test_security_guards_option_injection_and_traversal() {
    let repo = TestRepo::new();
    repo.write_file("file.txt", "hello\n");
    repo.commit("initial");

    let resolver = GitResolver::new(repo.path(), GitIntelligenceConfig::default()).unwrap();

    // 1. Option injection in revision
    assert!(runner::validate_revision("--output=/etc/pwned").is_err());
    assert!(resolver.commit_context("--output=/tmp/bad").is_err());

    // 2. Path traversal in path
    assert!(runner::validate_path(repo.path(), "../../etc/passwd").is_err());
    assert!(resolver
        .symbol_history("foo", Some("../outside.ts"), None)
        .is_err());

    // 3. Control character in revision
    assert!(runner::validate_revision("HEAD\nrm -rf /").is_err());
}

// ----------------------------------------------------------------------------
// Test 9: Non-git directory handling
// ----------------------------------------------------------------------------
#[test]
fn test_non_git_directory_handling() {
    let empty_dir = TempDir::new().unwrap();
    let res = GitResolver::new(empty_dir.path(), GitIntelligenceConfig::default());
    assert!(res.is_err());

    let intel = GitIntelligence::new(
        empty_dir.path(),
        CacheRuntime::default(),
        GitIntelligenceConfig::default(),
    );
    let tool_res = intel.execute_tool("git_commit_context", &json!({"commit": "HEAD"}));
    assert!(tool_res.is_ok());
    assert!(tool_res.unwrap().is_error);
}

// ----------------------------------------------------------------------------
// Test 10: CacheRuntime caching with CacheNamespace::Git and telemetry
// ----------------------------------------------------------------------------
#[test]
fn test_caching_and_telemetry() {
    let repo = TestRepo::new();
    repo.write_file("data.ts", "export const v = 1;\n");
    let sha = repo.commit("add data");

    let cache_dir = TempDir::new().unwrap();
    let cache = CacheRuntime::shared(CacheConfig::default(), cache_dir.path().into());
    let intel = GitIntelligence::new(repo.path(), cache, GitIntelligenceConfig::default());

    let args = json!({"commit": sha});

    // First call: cache miss
    let res1 = intel.execute_tool("git_commit_context", &args).unwrap();
    assert!(!res1.is_error);

    let status1 = intel.status();
    let tel1 = &status1["telemetry"];
    assert_eq!(tel1["requests"], 1);
    assert_eq!(tel1["misses"], 1);
    assert_eq!(tel1["hits"], 0);

    // Second call: cache hit!
    let res2 = intel.execute_tool("git_commit_context", &args).unwrap();
    assert!(!res2.is_error);

    let status2 = intel.status();
    let tel2 = &status2["telemetry"];
    assert_eq!(tel2["requests"], 2);
    assert_eq!(tel2["hits"], 1);
}

// ----------------------------------------------------------------------------
// Test 11: Permission classification and NativeExtensionHost integration
// ----------------------------------------------------------------------------
#[test]
fn test_permission_classification_and_host_integration() {
    // Verify all 7 tools are ToolClass::Read
    for tool in TOOL_NAMES {
        assert_eq!(
            tool_class(tool),
            ToolClass::Read,
            "tool {tool} must be classified as ToolClass::Read"
        );
    }

    // Verify NativeExtensionHost exposes all 7 tools
    let host = NativeExtensionHost::default();
    for tool in TOOL_NAMES {
        assert!(
            host.has_tool(tool),
            "NativeExtensionHost must register {tool}"
        );
        let spec = NativeExtensionHost::describe_tool(tool);
        assert!(spec.is_some(), "ToolSpec must exist for {tool}");
        let spec = spec.unwrap();
        assert_eq!(spec.name, *tool);
    }

    // Verify command /git-status
    let mut host = NativeExtensionHost::default();
    let status_res = host.command("git-status", "");
    assert!(status_res.is_ok());
    let status_val = status_res.unwrap();
    assert!(status_val.is_some());
    let val = status_val.unwrap();
    assert!(val.get("enabled").is_some());
}
