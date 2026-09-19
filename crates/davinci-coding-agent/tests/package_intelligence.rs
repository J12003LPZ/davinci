use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::{PermissionPolicy, ToolClass};
use davinci_coding_agent::native_extensions::package_intelligence::{
    PackageExportsResult, PackageInfoResult, PackageIntelligence, PackageSymbolResult,
    PackageWhyResult, TOOL_NAMES,
};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use davinci_coding_agent::settings::Settings;
use serde_json::json;
use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/package_intelligence")
        .join(name)
}

#[test]
fn test_npm_v1_resolution_and_tools() {
    let root = fixture_path("npm_v1");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    // 1. package_info
    let res = pkg_intel
        .execute_tool("package_info", &json!({"package": "chalk"}))
        .expect("package_info should succeed");
    assert!(!res.is_error);
    let info: PackageInfoResult =
        serde_json::from_str(&res.content).expect("valid PackageInfoResult json");
    assert_eq!(info.package_name, "chalk");
    assert_eq!(info.declared_version.as_deref(), Some("^4.0.0"));
    assert_eq!(info.installed_version.as_deref(), Some("4.1.2"));
    assert_eq!(info.locked_version.as_deref(), Some("4.1.2"));
    assert_eq!(info.lock_kind.as_deref(), Some("npm"));
    assert!(info.types_path.is_some());
    assert!(info.types_path.as_ref().unwrap().ends_with("index.d.ts"));
    assert!(info.dependents.contains(&"npm-v1-app".to_string()));

    // 2. package_exports
    let res = pkg_intel
        .execute_tool("package_exports", &json!({"package": "chalk"}))
        .expect("package_exports should succeed");
    assert!(!res.is_error);
    let exports: PackageExportsResult =
        serde_json::from_str(&res.content).expect("valid PackageExportsResult json");
    assert_eq!(exports.package_name, "chalk");
    assert_eq!(exports.installed_version.as_deref(), Some("4.1.2"));

    // 3. package_why
    let res = pkg_intel
        .execute_tool("package_why", &json!({"package": "chalk"}))
        .expect("package_why should succeed");
    assert!(!res.is_error);
    let why: PackageWhyResult =
        serde_json::from_str(&res.content).expect("valid PackageWhyResult json");
    assert_eq!(why.package_name, "chalk");
    assert_eq!(why.dependency_type.as_deref(), Some("dependencies"));
    assert!(!why.reasons.is_empty());
}

#[test]
fn test_npm_v3_symbols_and_types() {
    let root = fixture_path("npm_v3");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    // 1. package_info for zod
    let res = pkg_intel
        .execute_tool("package_info", &json!({"package": "zod"}))
        .expect("package_info should succeed");
    let info: PackageInfoResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(info.lock_kind.as_deref(), Some("npm"));
    assert_eq!(info.installed_version.as_deref(), Some("3.22.4"));

    // 2. package_symbol for scoped symbol z.object
    let res = pkg_intel
        .execute_tool(
            "package_symbol",
            &json!({"package": "zod", "symbol": "z.object"}),
        )
        .expect("package_symbol should succeed");
    let sym: PackageSymbolResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(sym.symbol, "z.object");
    assert!(sym.found, "symbol z.object should be found in index.d.ts");
    assert!(sym.line.is_some());
    assert!(sym.declaration.as_ref().unwrap().contains("object"));

    // 3. package_symbol for top-level scopedHelper in @scope/pkg
    let res = pkg_intel
        .execute_tool(
            "package_symbol",
            &json!({"package": "@scope/pkg", "symbol": "scopedHelper"}),
        )
        .expect("package_symbol should succeed");
    let sym: PackageSymbolResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(sym.symbol, "scopedHelper");
    assert!(sym.found, "scopedHelper should be found");
    assert_eq!(sym.kind.as_deref(), Some("function"));

    // 4. package_symbol for nonexistent symbol
    let res = pkg_intel
        .execute_tool(
            "package_symbol",
            &json!({"package": "zod", "symbol": "nonExistent"}),
        )
        .expect("package_symbol should succeed");
    let sym: PackageSymbolResult = serde_json::from_str(&res.content).unwrap();
    assert!(!sym.found);
}

#[test]
fn test_version_mismatch_warning() {
    let root = fixture_path("version_mismatch");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    let res = pkg_intel
        .execute_tool("package_info", &json!({"package": "zod"}))
        .expect("package_info should succeed");
    let info: PackageInfoResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(info.declared_version.as_deref(), Some("^3.20.0"));
    assert_eq!(info.locked_version.as_deref(), Some("3.21.4"));
    assert_eq!(info.installed_version.as_deref(), Some("3.22.4"));
    assert!(
        info.warnings.iter().any(|w| w.contains("mismatch")),
        "should produce version mismatch warning, got: {:?}",
        info.warnings
    );
}

#[test]
fn test_pnpm_workspace_resolution() {
    let root = fixture_path("pnpm_workspace");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    let res = pkg_intel
        .execute_tool(
            "package_info",
            &json!({"package": "zod", "workspace": "packages/api"}),
        )
        .expect("package_info should succeed");
    let info: PackageInfoResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(info.package_name, "zod");
    assert_eq!(info.workspace_scope, "packages/api");
    assert_eq!(info.lock_kind.as_deref(), Some("pnpm"));
    assert_eq!(info.declared_version.as_deref(), Some("^3.22.0"));
    assert_eq!(info.installed_version.as_deref(), Some("3.22.4"));
}

#[test]
fn test_yarn_classic_and_berry() {
    // 1. Yarn Classic
    let root_classic = fixture_path("yarn_classic");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root_classic, cache);
    let res = pkg_intel
        .execute_tool("package_info", &json!({"package": "ms"}))
        .expect("yarn classic package_info should succeed");
    let info: PackageInfoResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(info.lock_kind.as_deref(), Some("yarn-classic"));
    assert_eq!(info.locked_version.as_deref(), Some("2.1.3"));

    // 2. Yarn Berry
    let root_berry = fixture_path("yarn_berry");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root_berry, cache);
    let res = pkg_intel
        .execute_tool("package_info", &json!({"package": "debug"}))
        .expect("yarn berry package_info should succeed");
    let info: PackageInfoResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(info.lock_kind.as_deref(), Some("yarn-berry"));
    assert_eq!(info.locked_version.as_deref(), Some("4.3.4"));
}

#[test]
fn test_conditional_exports() {
    let root = fixture_path("conditional_exports");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    let res = pkg_intel
        .execute_tool("package_exports", &json!({"package": "isomorphic-pkg"}))
        .expect("package_exports should succeed");
    let exports: PackageExportsResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(exports.package_name, "isomorphic-pkg");

    let root_target = exports.exports.root_target.expect("root target must exist");
    assert_eq!(root_target.import.as_deref(), Some("./dist/index.mjs"));
    assert_eq!(root_target.require.as_deref(), Some("./dist/index.cjs"));
    assert_eq!(root_target.types.as_deref(), Some("./dist/index.d.ts"));

    let sub_feature = exports
        .exports
        .subpaths
        .get("./feature")
        .expect("./feature export target must exist");
    assert_eq!(sub_feature.types.as_deref(), Some("./dist/feature.d.ts"));
}

#[test]
fn test_security_guards_and_path_traversal() {
    let root = fixture_path("malicious");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    // Traversal in package name
    for bad_name in [
        "../../etc/passwd",
        "../outside",
        "foo/../../bar",
        "/root/pwd",
        "foo/bar/baz",
    ] {
        let err = pkg_intel.execute_tool("package_info", &json!({"package": bad_name}));
        assert!(
            err.is_err(),
            "traversal package name '{bad_name}' should be rejected"
        );
    }

    // Traversal in workspace parameter
    let err = pkg_intel.execute_tool(
        "package_info",
        &json!({"package": "evil-pkg", "workspace": "../../outside"}),
    );
    assert!(
        err.is_err(),
        "traversal workspace parameter should be rejected"
    );

    // Ensure scripts in package.json were never run
    let evil_witness = root.join("evil_executed.txt");
    assert!(
        !evil_witness.exists(),
        "lifecycle scripts must never be executed"
    );
}

#[test]
fn test_cache_runtime_caching_and_telemetry() {
    let root = fixture_path("npm_v1");
    let cache = CacheRuntime::default();
    let pkg_intel = PackageIntelligence::with_root(&root, cache);

    // Initial status
    let status_before = pkg_intel.status();
    let misses_before = status_before["telemetry"]["misses"].as_u64().unwrap_or(0);
    let hits_before = status_before["telemetry"]["hits"].as_u64().unwrap_or(0);

    // First call: cache miss
    let _ = pkg_intel
        .execute_tool("package_info", &json!({"package": "chalk"}))
        .expect("first call succeeds");

    let status_after_first = pkg_intel.status();
    let misses_first = status_after_first["telemetry"]["misses"].as_u64().unwrap();
    assert_eq!(misses_first, misses_before + 1);

    // Second call: cache hit
    let _ = pkg_intel
        .execute_tool("package_info", &json!({"package": "chalk"}))
        .expect("second call succeeds");

    let status_after_second = pkg_intel.status();
    let hits_second = status_after_second["telemetry"]["hits"].as_u64().unwrap();
    assert_eq!(hits_second, hits_before + 1);
}

#[test]
fn test_settings_parsing_package_intelligence() {
    let raw = json!({
        "packageIntelligence": {
            "enabled": true
        }
    });
    let settings: Settings = serde_json::from_value(raw).unwrap();
    assert!(settings.package_intelligence.is_some());
    assert!(settings.package_intelligence.unwrap().enabled);

    let raw_disabled = json!({
        "packageIntelligence": {
            "enabled": false
        }
    });
    let settings_disabled: Settings = serde_json::from_value(raw_disabled).unwrap();
    assert!(!settings_disabled.package_intelligence.unwrap().enabled);
}

#[test]
fn test_permission_classification_and_host_integration() {
    // 1. Permission classification: all 5 tools must be Read
    let policy = PermissionPolicy::default();
    for tool in TOOL_NAMES {
        assert_eq!(
            policy.class_of(tool),
            ToolClass::Read,
            "tool {tool} must be classified as ToolClass::Read"
        );
    }

    // 2. NativeExtensionHost tool discovery and specs
    let mut host = NativeExtensionHost::default();
    for tool in TOOL_NAMES {
        assert!(
            host.has_tool(tool),
            "NativeExtensionHost must register {tool}"
        );
        let spec = NativeExtensionHost::describe_tool(tool);
        assert!(spec.is_some(), "describe_tool must return spec for {tool}");
        let spec = spec.unwrap();
        assert_eq!(spec.name, *tool);
        assert!(!spec.description.is_empty());
    }

    // 3. package-status command execution
    let res = host
        .command("package-status", "")
        .expect("package-status should succeed");
    assert!(res.is_some());
    let status_val = res.unwrap();
    assert_eq!(status_val["enabled"], true);
    assert!(status_val["telemetry"]["hits"].is_number());
    assert!(status_val["telemetry"]["misses"].is_number());
}
