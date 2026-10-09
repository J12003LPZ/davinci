//! Regressions for the Package Intelligence audit findings (WOR-207..213).
use davinci_agent::runtime::cache::CacheRuntime;
use davinci_coding_agent::native_extensions::package_intelligence::{
    PackageInfoResult, PackageIntelligence, PackageWhyResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn tool(root: &Path, name: &str, args: Value) -> Result<String, String> {
    let intel = PackageIntelligence::with_root(root, CacheRuntime::default());
    intel
        .execute_tool(name, &args)
        .map(|result| result.content)
        .map_err(|err| err.to_string())
}

fn info(root: &Path, package: &str) -> PackageInfoResult {
    serde_json::from_str(&tool(root, "package_info", json!({ "package": package })).unwrap())
        .unwrap()
}

fn why(root: &Path, package: &str) -> PackageWhyResult {
    serde_json::from_str(&tool(root, "package_why", json!({ "package": package })).unwrap())
        .unwrap()
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
    // Symlinks need a privilege; a directory junction resolves the same way
    // under `canonicalize` and does not.
    // `mklink` rejects forward slashes, which `join("a/b")` leaves in paths.
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
fn workspace_symlink_outside_the_repository_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    write(
        outside.path(),
        "package.json",
        r#"{"dependencies":{"x":"9.9.9"}}"#,
    );
    let repo = tempfile::tempdir().unwrap();
    write(repo.path(), "package.json", r#"{"name":"app"}"#);
    if !symlink_dir(outside.path(), &repo.path().join("external")) {
        eprintln!("skipping: symlinks are unavailable here");
        return;
    }
    let err = tool(
        repo.path(),
        "package_info",
        json!({"package": "x", "workspace": "external"}),
    )
    .unwrap_err();
    assert!(err.contains("escapes repository root"), "{err}");
}

#[test]
fn pnpm_virtual_store_finds_scoped_packages_without_a_top_level_link() {
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"@scope/name":"^1.0.0"}}"#,
    );
    write(
        repo.path(),
        "node_modules/.pnpm/@scope+name@1.0.0/node_modules/@scope/name/package.json",
        r#"{"name":"@scope/name","version":"1.0.0"}"#,
    );
    // A different package that merely shares the name prefix must not match.
    write(
        repo.path(),
        "node_modules/.pnpm/foo-bar@3.0.0/node_modules/foo/package.json",
        r#"{"name":"foo","version":"3.0.0"}"#,
    );
    assert_eq!(
        info(repo.path(), "@scope/name")
            .installed_version
            .as_deref(),
        Some("1.0.0")
    );
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"foo":"^1"}}"#,
    );
    assert_eq!(info(repo.path(), "foo").installed_version, None);
}

#[test]
fn pnpm_virtual_store_link_outside_the_repository_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    write(
        outside.path(),
        "package.json",
        r#"{"name":"foo","version":"6.6.6"}"#,
    );
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"foo":"^1"}}"#,
    );
    let nm = repo
        .path()
        .join("node_modules/.pnpm/foo@1.0.0/node_modules");
    fs::create_dir_all(&nm).unwrap();
    if !symlink_dir(outside.path(), &nm.join("foo")) {
        eprintln!("skipping: symlinks are unavailable here");
        return;
    }
    let result = info(repo.path(), "foo");
    assert_eq!(result.installed_version, None);
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("security refusal")));
}

#[test]
fn pnpm_v9_snapshots_provide_dependency_edges() {
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"foo":"^1"}}"#,
    );
    write(
        repo.path(),
        "pnpm-lock.yaml",
        "lockfileVersion: '9.0'\n\
importers:\n  .:\n    dependencies:\n      foo:\n        specifier: ^1\n        version: 1.0.0\n\
packages:\n  foo@1.0.0:\n    resolution: {integrity: sha512-a}\n  bar@2.0.0:\n    resolution: {integrity: sha512-b}\n\
snapshots:\n  foo@1.0.0:\n    dependencies:\n      bar: 2.0.0\n  bar@2.0.0: {}\n",
    );
    let result = why(repo.path(), "bar");
    assert!(
        result
            .reasons
            .iter()
            .any(|reason| reason.from_package == "foo@1.0.0"
                && reason.dependency_type == "lock_dependency"),
        "{result:?}"
    );
}

#[test]
fn pnpm_optional_dependencies_are_lock_dependencies() {
    let repo = tempfile::tempdir().unwrap();
    write(repo.path(), "package.json", r#"{"name":"app"}"#);
    write(
        repo.path(),
        "pnpm-lock.yaml",
        "lockfileVersion: '6.0'\n\
packages:\n  /foo@1.0.0:\n    resolution: {integrity: sha512-a}\n    optionalDependencies:\n      bar: 1.0.0\n  /bar@1.0.0:\n    resolution: {integrity: sha512-b}\n",
    );
    let result = why(repo.path(), "bar");
    assert!(
        result
            .reasons
            .iter()
            .any(|reason| reason.from_package == "foo@1.0.0"),
        "{result:?}"
    );
}

#[test]
fn root_importer_version_wins_over_the_name_keyed_package_map() {
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"foo":"^2"}}"#,
    );
    write(
        repo.path(),
        "pnpm-lock.yaml",
        "lockfileVersion: '6.0'\n\
importers:\n  .:\n    dependencies:\n      foo:\n        specifier: ^2\n        version: 2.0.0\n  packages/a:\n    dependencies:\n      foo:\n        specifier: ^1\n        version: 1.0.0\n\
packages:\n  /foo@1.0.0:\n    resolution: {integrity: sha512-a}\n  /foo@2.0.0:\n    resolution: {integrity: sha512-b}\n",
    );
    assert_eq!(
        info(repo.path(), "foo").locked_version.as_deref(),
        Some("2.0.0")
    );
}

#[test]
fn cached_results_follow_the_installed_package() {
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"pkg":"*"}}"#,
    );
    write(
        repo.path(),
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","version":"1.0.0"}"#,
    );
    // One shared cache across both calls, like a long-lived session.
    let intel = PackageIntelligence::with_root(repo.path(), CacheRuntime::default());
    let version = |intel: &PackageIntelligence| -> Option<String> {
        let content = intel
            .execute_tool("package_info", &json!({"package": "pkg"}))
            .unwrap()
            .content;
        serde_json::from_str::<PackageInfoResult>(&content)
            .unwrap()
            .installed_version
    };
    assert_eq!(version(&intel).as_deref(), Some("1.0.0"));
    write(
        repo.path(),
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","version":"2.0.0"}"#,
    );
    assert_eq!(version(&intel).as_deref(), Some("2.0.0"));
}
