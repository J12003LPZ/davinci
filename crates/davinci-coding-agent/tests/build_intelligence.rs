use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::{PermissionPolicy, ToolClass};
use davinci_coding_agent::native_extensions::build_intelligence::{
    BuildAffectedResult, BuildCommandResult, BuildDependenciesResult, BuildIntelligence,
    BuildTargetsResult, WorkspacePackagesResult, TOOL_NAMES,
};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use davinci_coding_agent::settings::Settings;
use serde_json::json;
use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/build_intelligence")
        .join(name)
}

#[test]
fn test_workspace_packages_discovery_turbo() {
    let root = fixture_path("turbo_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    let res = build_intel
        .execute_tool("workspace_packages", &json!({}))
        .expect("workspace_packages should succeed");
    assert!(!res.is_error);

    let ws: WorkspacePackagesResult =
        serde_json::from_str(&res.content).expect("valid WorkspacePackagesResult");
    assert_eq!(ws.package_manager, "pnpm");
    assert_eq!(ws.task_runner.as_deref(), Some("turbo"));
    assert!(ws.packages.len() >= 2);

    let names: Vec<&str> = ws.packages.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"@repo/ui"));
    assert!(names.contains(&"@repo/web"));

    // Test scope filtering
    let scoped_res = build_intel
        .execute_tool("workspace_packages", &json!({"scope": "packages/ui"}))
        .expect("scoped workspace_packages should succeed");
    assert!(!scoped_res.is_error);
    let scoped_ws: WorkspacePackagesResult = serde_json::from_str(&scoped_res.content).unwrap();
    assert_eq!(scoped_ws.packages.len(), 1);
    assert_eq!(scoped_ws.packages[0].name, "@repo/ui");
}

#[test]
fn test_turbo_targets_and_pipeline() {
    let root = fixture_path("turbo_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    let res = build_intel
        .execute_tool("build_targets", &json!({}))
        .expect("build_targets should succeed");
    assert!(!res.is_error);

    let targets_res: BuildTargetsResult =
        serde_json::from_str(&res.content).expect("valid BuildTargetsResult");
    assert_eq!(targets_res.task_runner.as_deref(), Some("turbo"));

    let target_names: Vec<&str> = targets_res
        .targets
        .iter()
        .map(|t| t.target.as_str())
        .collect();
    assert!(target_names.contains(&"build"));
    assert!(target_names.contains(&"check"));

    let build_target = targets_res
        .targets
        .iter()
        .find(|t| t.target == "build")
        .unwrap();
    assert_eq!(build_target.depends_on, vec!["^build"]);
    assert!(build_target.cacheable);
    assert!(build_target
        .outputs
        .iter()
        .any(|o| o.contains("dist") || o.contains(".next")));

    // Package-specific target query
    let pkg_res = build_intel
        .execute_tool("build_targets", &json!({"package": "@repo/web"}))
        .expect("package build_targets should succeed");
    assert!(!pkg_res.is_error);
    let pkg_targets: BuildTargetsResult = serde_json::from_str(&pkg_res.content).unwrap();
    assert!(pkg_targets.targets.iter().all(|t| t.package == "@repo/web"));
}

#[test]
fn test_nx_targets_and_depends_on() {
    let root = fixture_path("nx_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    let res = build_intel
        .execute_tool("build_targets", &json!({}))
        .expect("build_targets should succeed");
    assert!(!res.is_error);

    let targets_res: BuildTargetsResult =
        serde_json::from_str(&res.content).expect("valid BuildTargetsResult");
    assert_eq!(targets_res.task_runner.as_deref(), Some("nx"));

    let app_build = targets_res
        .targets
        .iter()
        .find(|t| (t.package == "@nx-repo/app" || t.package == "app") && t.target == "build");
    assert!(app_build.is_some());
    let app_build = app_build.unwrap();
    assert_eq!(app_build.depends_on, vec!["^build"]);
    assert_eq!(app_build.executor.as_deref(), Some("@nx/webpack:webpack"));

    let core_build = targets_res
        .targets
        .iter()
        .find(|t| (t.package == "@nx-repo/core" || t.package == "core") && t.target == "build");
    assert!(core_build.is_some());
    let core_build = core_build.unwrap();
    assert_eq!(core_build.executor.as_deref(), Some("@nx/js:tsc"));
}

#[test]
fn test_tsconfig_references_graph() {
    let root = fixture_path("tsconfig_references");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    // 1. Discover packages
    let ws_res = build_intel
        .execute_tool("workspace_packages", &json!({}))
        .expect("workspace_packages should succeed");
    assert!(!ws_res.is_error);
    let ws: WorkspacePackagesResult = serde_json::from_str(&ws_res.content).unwrap();
    assert!(ws.packages.iter().any(|p| p.name == "@ts/core"));
    assert!(ws.packages.iter().any(|p| p.name == "@ts/app"));

    // 2. Discover build dependencies for @ts/app
    let dep_res = build_intel
        .execute_tool("build_dependencies", &json!({"package": "@ts/app"}))
        .expect("build_dependencies should succeed");
    assert!(!dep_res.is_error);

    let deps: BuildDependenciesResult =
        serde_json::from_str(&dep_res.content).expect("valid BuildDependenciesResult");
    assert_eq!(deps.package, "@ts/app");
    assert!(
        deps.dependencies.contains(&"@ts/core".to_string())
            || deps.project_references.iter().any(|r| r.contains("core")),
        "expected @ts/core in dependencies or project_references"
    );
}

#[test]
fn test_build_affected_with_downstream_dependents() {
    let root = fixture_path("turbo_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    // Non-omission guard: changing file in @repo/ui MUST affect downstream dependent @repo/web!
    let res = build_intel
        .execute_tool(
            "build_affected",
            &json!({
                "files": ["packages/ui/src/button.ts"]
            }),
        )
        .expect("build_affected should succeed");
    assert!(!res.is_error);

    let affected: BuildAffectedResult =
        serde_json::from_str(&res.content).expect("valid BuildAffectedResult");
    assert!(
        affected.affected_packages.contains(&"@repo/ui".to_string()),
        "directly modified package @repo/ui must be affected"
    );
    assert!(
        affected
            .affected_packages
            .contains(&"@repo/web".to_string()),
        "downstream dependent @repo/web must be affected (non-omission guard)"
    );

    // Test reverse dependency traversal when passing package name directly
    let res_pkg = build_intel
        .execute_tool(
            "build_affected",
            &json!({
                "packages": ["@repo/ui"]
            }),
        )
        .expect("build_affected with package should succeed");
    assert!(!res_pkg.is_error);
    let affected_pkg: BuildAffectedResult = serde_json::from_str(&res_pkg.content).unwrap();
    assert!(affected_pkg
        .affected_packages
        .contains(&"@repo/web".to_string()));

    // Changing only leaf web package should NOT affect upstream ui package
    let res_leaf = build_intel
        .execute_tool(
            "build_affected",
            &json!({
                "files": ["packages/web/src/index.ts"]
            }),
        )
        .expect("build_affected for leaf should succeed");
    assert!(!res_leaf.is_error);
    let affected_leaf: BuildAffectedResult = serde_json::from_str(&res_leaf.content).unwrap();
    assert!(affected_leaf
        .affected_packages
        .contains(&"@repo/web".to_string()));
    assert!(
        !affected_leaf
            .affected_packages
            .contains(&"@repo/ui".to_string()),
        "leaf change should not affect independent or upstream package"
    );
}

#[test]
fn test_build_command_generation_turbo_and_nx() {
    // 1. Turbo monorepo build command
    let turbo_root = fixture_path("turbo_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&turbo_root, cache);

    let res = build_intel
        .execute_tool(
            "build_command",
            &json!({
                "packages": ["@repo/web"],
                "target": "build"
            }),
        )
        .expect("build_command should succeed");
    assert!(!res.is_error);

    let cmd: BuildCommandResult =
        serde_json::from_str(&res.content).expect("valid BuildCommandResult");
    assert!(
        cmd.command.contains("turbo") && cmd.command.contains("build"),
        "command should use turbo run build: {}",
        cmd.command
    );
    assert!(
        cmd.command.contains("--filter=@repo/web") || cmd.command.contains("--filter @repo/web"),
        "command should filter to @repo/web: {}",
        cmd.command
    );
    assert!(cmd.supports_cache);

    // 2. Turbo monorepo with affected files
    let res_files = build_intel
        .execute_tool(
            "build_command",
            &json!({
                "files": ["packages/ui/src/button.ts"]
            }),
        )
        .expect("build_command with files should succeed");
    assert!(!res_files.is_error);
    let cmd_files: BuildCommandResult = serde_json::from_str(&res_files.content).unwrap();
    assert!(cmd_files.command.contains("turbo"));

    // 3. Nx monorepo build command
    let nx_root = fixture_path("nx_monorepo");
    let build_intel_nx = BuildIntelligence::with_root(&nx_root, CacheRuntime::default());
    let nx_res = build_intel_nx
        .execute_tool(
            "build_command",
            &json!({
                "packages": ["app"],
                "target": "build"
            }),
        )
        .expect("nx build_command should succeed");
    assert!(!nx_res.is_error);
    let nx_cmd: BuildCommandResult = serde_json::from_str(&nx_res.content).unwrap();
    assert!(
        nx_cmd.command.contains("nx") && nx_cmd.command.contains("build"),
        "command should use nx build: {}",
        nx_cmd.command
    );
}

#[test]
fn test_cyclic_and_unknown_scripts() {
    let root = fixture_path("cyclic_unknown");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    // Circular dependency pkg-a <-> pkg-b must not loop infinitely
    let res = build_intel
        .execute_tool(
            "build_affected",
            &json!({
                "packages": ["pkg-a"]
            }),
        )
        .expect("build_affected should resolve cycles safely");
    assert!(!res.is_error);

    let affected: BuildAffectedResult = serde_json::from_str(&res.content).unwrap();
    assert!(affected.affected_packages.contains(&"pkg-a".to_string()));
    assert!(affected.affected_packages.contains(&"pkg-b".to_string()));

    // Custom script discovery
    let tg_res = build_intel
        .execute_tool("build_targets", &json!({"package": "pkg-a"}))
        .expect("custom targets should be discovered");
    assert!(!tg_res.is_error);
    let targets: BuildTargetsResult = serde_json::from_str(&tg_res.content).unwrap();
    assert!(targets.targets.iter().any(|t| t.target == "custom-task"));
}

#[test]
fn test_vite_and_next_framework_detection() {
    let root = fixture_path("vite_next_mixed");
    let cache = CacheRuntime::default();

    // Standalone vite app
    let vite_dir = root.join("vite-app");
    let vite_intel = BuildIntelligence::with_root(&vite_dir, cache.clone());
    let res = vite_intel
        .execute_tool("workspace_packages", &json!({}))
        .expect("vite workspace_packages should succeed");
    let ws: WorkspacePackagesResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(ws.framework.as_deref(), Some("vite"));

    // Standalone next app
    let next_dir = root.join("next-app");
    let next_intel = BuildIntelligence::with_root(&next_dir, cache);
    let res = next_intel
        .execute_tool("workspace_packages", &json!({}))
        .expect("next workspace_packages should succeed");
    let ws: WorkspacePackagesResult = serde_json::from_str(&res.content).unwrap();
    assert_eq!(ws.framework.as_deref(), Some("nextjs"));
}

#[test]
fn test_security_guards_and_path_traversal() {
    let root = fixture_path("turbo_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    // Traversal in scope parameter
    for bad_scope in [
        "../../etc/passwd",
        "../outside",
        "foo/../../bar",
        "/root/pwd",
    ] {
        let res = build_intel.execute_tool("workspace_packages", &json!({"scope": bad_scope}));
        assert!(
            res.is_err() || res.unwrap().is_error,
            "traversal scope '{bad_scope}' should be rejected"
        );
    }

    // Traversal in package parameter
    for bad_pkg in [
        "../../etc/passwd",
        "../outside",
        "foo/../../bar",
        "/root/pwd",
    ] {
        let res = build_intel.execute_tool("build_dependencies", &json!({"package": bad_pkg}));
        assert!(
            res.is_err() || res.unwrap().is_error,
            "traversal package '{bad_pkg}' should be rejected"
        );
    }
}

#[test]
fn test_cache_runtime_caching_and_telemetry() {
    let root = fixture_path("turbo_monorepo");
    let cache = CacheRuntime::default();
    let build_intel = BuildIntelligence::with_root(&root, cache);

    let status_before = build_intel.status();
    let misses_before = status_before["telemetry"]["misses"].as_u64().unwrap_or(0);
    let hits_before = status_before["telemetry"]["hits"].as_u64().unwrap_or(0);

    // First call: miss
    let _ = build_intel
        .execute_tool("workspace_packages", &json!({}))
        .expect("first call succeeds");
    let status_mid = build_intel.status();
    let misses_mid = status_mid["telemetry"]["misses"].as_u64().unwrap();
    assert_eq!(misses_mid, misses_before + 1);

    // Second call: hit
    let _ = build_intel
        .execute_tool("workspace_packages", &json!({}))
        .expect("second call succeeds");
    let status_after = build_intel.status();
    let hits_after = status_after["telemetry"]["hits"].as_u64().unwrap();
    assert_eq!(hits_after, hits_before + 1);
}

#[test]
fn test_settings_parsing_build_intelligence() {
    let raw = json!({
        "buildIntelligence": {
            "enabled": true
        }
    });
    let settings: Settings = serde_json::from_value(raw).unwrap();
    assert!(settings.build_intelligence.is_some());
    assert!(settings.build_intelligence.unwrap().enabled);

    let raw_disabled = json!({
        "buildIntelligence": {
            "enabled": false
        }
    });
    let settings_disabled: Settings = serde_json::from_value(raw_disabled).unwrap();
    assert!(!settings_disabled.build_intelligence.unwrap().enabled);
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

    // 3. build-status command execution
    let res = host
        .command("build-status", "")
        .expect("build-status should succeed");
    assert!(res.is_some());
    let status_val = res.unwrap();
    assert_eq!(status_val["enabled"], true);
    assert!(status_val["telemetry"]["hits"].is_number());
    assert!(status_val["telemetry"]["misses"].is_number());
}
