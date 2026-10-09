//! Regressions for the Build Intelligence audit findings (WOR-244, 245, 247-255).
use davinci_agent::runtime::cache::CacheRuntime;
use davinci_coding_agent::native_extensions::build_intelligence::{
    BuildCommandResult, BuildIntelligence, BuildTargetsResult, WorkspacePackagesResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn call<T: serde::de::DeserializeOwned>(root: &Path, tool: &str, args: Value) -> T {
    let intel = BuildIntelligence::with_root(root, CacheRuntime::default());
    let result = intel.execute_tool(tool, &args).expect("tool succeeds");
    serde_json::from_str(&result.content).expect("valid result")
}

fn package_names(root: &Path) -> Vec<String> {
    let result: WorkspacePackagesResult = call(root, "workspace_packages", json!({}));
    result.packages.into_iter().map(|p| p.name).collect()
}

fn pkg(name: &str) -> String {
    format!(r#"{{"name":"{name}","scripts":{{"build":"echo {name}"}}}}"#)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    if std::os::windows::fs::symlink_dir(target, link).is_ok() {
        return true;
    }
    let backslashed = |path: &Path| path.to_string_lossy().replace('/', "\\");
    std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(backslashed(link))
        .arg(backslashed(target))
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn nested_and_negated_workspace_globs() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root,
        "package.json",
        r#"{"name":"root","workspaces":["packages/**","!packages/fixtures/*","{apps,libs}/*"]}"#,
    );
    write(root, "packages/a/b/package.json", &pkg("nested"));
    write(root, "packages/fixtures/x/package.json", &pkg("fixture"));
    write(root, "apps/web/package.json", &pkg("web"));
    write(root, "libs/core/package.json", &pkg("core"));
    let names = package_names(root);
    for expected in ["nested", "web", "core"] {
        assert!(names.contains(&expected.to_string()), "{names:?}");
    }
    assert!(!names.contains(&"fixture".to_string()), "{names:?}");
}

#[test]
fn symlinked_workspace_outside_the_repository_is_not_loaded() {
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "package.json", &pkg("outside"));
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root,
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    write(root, "packages/in/package.json", &pkg("inside"));
    if !symlink_dir(outside.path(), &root.join("packages/outside")) {
        eprintln!("skipping: symlinks are unavailable here");
        return;
    }
    let names = package_names(root);
    assert!(names.contains(&"inside".to_string()));
    assert!(!names.contains(&"outside".to_string()), "{names:?}");
}

fn build_command(root: &Path, packages: &[&str]) -> BuildCommandResult {
    call(root, "build_command", json!({"packages": packages}))
}

#[test]
fn argv_is_arguments_only_and_targets_the_selected_workspace() {
    for (lock, packages, program, expected) in [
        (
            "package-lock.json",
            vec!["app-a"],
            "npm",
            vec!["run", "build", "--workspace=app-a"],
        ),
        (
            "yarn.lock",
            vec!["app-a"],
            "yarn",
            vec!["workspace", "app-a", "run", "build"],
        ),
        (
            "bun.lock",
            vec!["app-a"],
            "bun",
            vec!["--filter", "app-a", "run", "build"],
        ),
        (
            "pnpm-lock.yaml",
            vec!["app-a"],
            "pnpm",
            vec!["--filter=app-a...", "run", "build"],
        ),
        ("package-lock.json", vec![], "npm", vec!["run", "build"]),
    ] {
        let repo = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            "package.json",
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        );
        write(repo.path(), lock, "");
        write(repo.path(), "packages/app-a/package.json", &pkg("app-a"));
        let command = build_command(repo.path(), &packages);
        assert_eq!(command.program, program, "{lock}");
        assert_eq!(command.argv, expected, "{lock}");
        assert_eq!(
            command.raw_command,
            format!("{program} {}", expected.join(" "))
        );
    }
}

#[test]
fn package_manager_follows_corepack_field_and_bun_text_lockfile() {
    let declared = tempfile::tempdir().unwrap();
    write(
        declared.path(),
        "package.json",
        r#"{"name":"root","packageManager":"pnpm@9.12.0","scripts":{"build":"x"}}"#,
    );
    assert_eq!(build_command(declared.path(), &[]).program, "pnpm");

    let bun = tempfile::tempdir().unwrap();
    write(bun.path(), "package.json", r#"{"name":"root","scripts":{"build":"x"}}"#);
    write(bun.path(), "bun.lock", "");
    assert_eq!(build_command(bun.path(), &[]).program, "bun");
}

#[test]
fn tsconfig_with_comments_and_trailing_commas_keeps_its_references() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(root, "package.json", r#"{"name":"root"}"#);
    write(
        root,
        "tsconfig.json",
        "{\n  // projects\n  \"references\": [{\"path\": \"./lib\"},],\n}\n",
    );
    write(root, "lib/package.json", &pkg("lib"));
    assert!(package_names(root).contains(&"lib".to_string()));
}

#[test]
fn repo_intelligence_reads_jsonc_tsconfig_aliases() {
    use davinci_coding_agent::native_extensions::repo_intelligence::RepoIntelligence;
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        repo.path(),
        "tsconfig.json",
        "{\n  // path aliases\n  \"compilerOptions\": {\"baseUrl\": \".\", \"paths\": {\"@/*\": [\"src/*\",],},},\n}\n",
    );
    write(repo.path(), "src/lib.ts", "export const lib = 1;");
    write(repo.path(), "src/app.ts", "import { lib } from '@/lib'; lib;");
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let deps = manager
        .query("file_dependencies", &json!({"path":"src/app.ts"}))
        .unwrap();
    assert!(deps["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["target"] == "src/lib.ts" && r["external"] == false));
}

#[test]
fn nx_cache_false_is_not_overridden_and_defaults_are_not_targets() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root,
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    write(
        root,
        "nx.json",
        r#"{"targetDefaults":{"build":{"cache":true},"lint":{"cache":true}}}"#,
    );
    write(root, "packages/a/package.json", &pkg("a"));
    write(
        root,
        "packages/a/project.json",
        r#"{"name":"a","targets":{"build":{"command":"echo","cache":false}}}"#,
    );
    write(root, "packages/b/package.json", &pkg("b"));
    let result: BuildTargetsResult = call(root, "build_targets", json!({}));
    let build_a = result
        .targets
        .iter()
        .find(|t| t.package == "a" && t.target == "build")
        .expect("a:build");
    assert!(!build_a.cacheable);
    assert!(
        !result.targets.iter().any(|t| t.target == "lint"),
        "defaults must not create targets: {:?}",
        result.targets
    );
}

#[test]
fn turbo_package_override_replaces_the_generic_task() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root,
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    write(
        root,
        "turbo.json",
        r#"{"tasks":{"build":{"cache":true},"app#build":{"cache":false}}}"#,
    );
    write(root, "packages/app/package.json", &pkg("app"));
    let result: BuildTargetsResult = call(root, "build_targets", json!({"package": "app"}));
    let builds: Vec<_> = result.targets.iter().filter(|t| t.target == "build").collect();
    assert_eq!(builds.len(), 1, "{builds:?}");
    assert!(!builds[0].cacheable);
}
