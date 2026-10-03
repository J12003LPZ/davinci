#[path = "support/test_impact_fixture.rs"]
#[allow(dead_code)]
mod fixture;

use davinci_coding_agent::native_extensions::NativeExtensionHost;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[test]
fn impact_permission_checks_every_input_before_approval_shortcuts() {
    use davinci_agent::{PermissionMode, PermissionPolicy, PermissionVerdict};
    let root = tempfile::tempdir().unwrap();
    for mode in [PermissionMode::Ask, PermissionMode::AlwaysApprove] {
        let policy = PermissionPolicy::new(mode);
        for args in [
            json!({"paths":["src/token.ts",".env"]}),
            json!({"paths":["src/token.ts","../outside.ts"]}),
            json!({"paths":"src/token.ts"}),
            json!({"paths":[42]}),
        ] {
            assert!(
                matches!(
                    policy.decide("impact", "test_plan", &args, root.path()),
                    PermissionVerdict::Deny { .. }
                ),
                "{args}"
            );
        }
    }
}

#[test]
fn malformed_impact_settings_disable_only_impact() {
    let settings: davinci_coding_agent::settings::Settings = serde_json::from_value(json!({
        "testImpact":{"enabled":"yes"},"theme":"preserved"
    }))
    .unwrap();
    assert!(!settings.test_impact.unwrap().enabled);
    assert_eq!(settings.theme.as_deref(), Some("preserved"));
}

#[test]
fn impact_aliases_symbols_and_config_changes_share_the_existing_repo_index() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::write(
        root.path(),
        "package.json",
        r#"{"scripts":{"test":"vitest run"}}"#,
    );
    fixture::write(
        root.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@source/*":["src/*"]}}}"#,
    );
    fixture::write(
        root.path(),
        "src/value.ts",
        "export function value() { return 1; }",
    );
    fixture::write(
        root.path(),
        "src/other.ts",
        "export function other() { return 2; }",
    );
    fixture::write(
        root.path(),
        "tests/consumer.spec.ts",
        "import {value} from '@source/value'; value();",
    );
    let mut host =
        NativeExtensionHost::new_with_agent_dir("aliases", root.path(), Some(state.path()));
    let symbol = host.repo_intelligence.refresh().unwrap().files["src/value.ts"].symbols[0]
        .id
        .clone();
    let first = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"symbolIds":[symbol]}),
    );
    assert_eq!(first["results"][0]["path"], "tests/consumer.spec.ts");
    assert_eq!(first["results"][0]["reasons"][0]["source"], "ast");
    fixture::write(
        root.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@source/value":["src/other.ts"]}}}"#,
    );
    let next = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"path":"src/value.ts"}),
    );
    assert_eq!(next["total"], 0);
    assert_ne!(first["source_identity"], next["source_identity"]);
    assert!(!next["broader_verification"].as_array().unwrap().is_empty());
    fixture::write(root.path(), "package.json", "{broken");
    let malformed = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"path":"src/value.ts"}),
    );
    assert_eq!(malformed["partial"], true);
    assert!(malformed["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("invalid package manifest")));
}

#[test]
fn impact_large_selection_uses_complete_package_commands_and_retains_exact_output() {
    use davinci_coding_agent::native_extensions::{
        OutputStore, TokenGovernor, TokenGovernorConfig,
    };
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::write(
        root.path(),
        "package.json",
        r#"{"scripts":{"test":"node --test"}}"#,
    );
    fixture::write(root.path(), "value.ts", "export const value = 1;");
    for n in 0..80 {
        fixture::write(
            root.path(),
            &format!("tests/example{n}.test.ts"),
            "import {value} from '../value';",
        );
    }
    let mut host =
        NativeExtensionHost::new_with_agent_dir("impact-output", root.path(), Some(state.path()));
    host.governor = std::sync::Arc::new(std::sync::Mutex::new(TokenGovernor::new(
        "impact-output",
        TokenGovernorConfig {
            compress_threshold_bytes: 50,
            compress_threshold_lines: 1,
            store_dir: Some(state.path().to_path_buf()),
            ..Default::default()
        },
    )));
    let args = json!({"path":"value.ts","limit":100});
    let original = host.execute_tool(root.path(), "test_plan", &args).unwrap();
    let details = original.details.as_ref().unwrap();
    assert_eq!(details["total"], 80);
    assert_eq!(details["first_tier"][0]["argv"], json!(["run", "test"]));
    assert!(original.content.len() < 128 * 1024);
    let exact = original.content.clone();
    let processed = host.after_tool("test_plan", &args, original);
    let id = processed.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
        .as_str()
        .unwrap();
    assert!(processed.content.contains("retrieve_output"));
    assert_eq!(
        OutputStore::new(state.path().join("outputs/impact-output"))
            .load(id)
            .unwrap(),
        exact
    );
}

#[test]
fn impact_rechecks_permissions_after_a_warm_mapping() {
    use davinci_agent::{PermissionMode, PermissionPolicy, PermissionRule, PermissionState};
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    let mut host =
        NativeExtensionHost::new_with_agent_dir("revocation", root.path(), Some(state.path()));
    let permissions = std::sync::Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::AlwaysApprove,
    )));
    host.test_impact.set_permissions(permissions.clone());
    let args = json!({"paths":[fixture::CHANGED]});
    call(&mut host, root.path(), "test_plan", args.clone());
    let warm = call(&mut host, root.path(), "test_plan", args.clone());
    assert_eq!(warm["telemetry"]["mapping_cache_hit"], true);
    permissions
        .lock()
        .unwrap()
        .deny
        .push(PermissionRule::subject("read", fixture::IMPACTED[2]));
    assert!(host.execute_tool(root.path(), "test_plan", &args).is_err());
}

#[test]
fn impact_cancellation_is_retryable_and_status_does_not_start_an_observer() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    let mut host =
        NativeExtensionHost::new_with_agent_dir("cancel-impact", root.path(), Some(state.path()));
    assert_eq!(
        host.test_impact.status()["repository"]["observation"]["started"],
        false
    );
    let cancel = Arc::new(AtomicBool::new(true));
    host.test_impact.set_cancellation(Some(cancel.clone()));
    assert!(host
        .execute_tool(root.path(), "test_plan", &json!({"path":fixture::CHANGED}))
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    assert_eq!(
        host.test_impact.status()["telemetry"]["last_failure"],
        "cancelled"
    );
    assert_eq!(
        host.test_impact.status()["repository"]["observation"]["started"],
        false
    );
    cancel.store(false, Ordering::Release);
    let result = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"path":fixture::CHANGED}),
    );
    assert_eq!(result["total"], 3);
    assert!(result["telemetry"]["latency_ms"].is_number());
    assert_eq!(host.test_impact.status()["telemetry"]["requests"], 2);
}

#[test]
fn incomplete_dependencies_preserve_workspace_wide_broader_verification() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    fixture::write(
        root.path(),
        "packages/auth/src/unknown.mjs",
        "import './missing.mjs';",
    );
    let mut host =
        NativeExtensionHost::new_with_agent_dir("partial-impact", root.path(), Some(state.path()));
    let result = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"path":fixture::CHANGED}),
    );
    assert_eq!(result["partial"], true);
    let broader = result["broader_verification"].as_array().unwrap();
    assert!(broader.iter().any(|command| command["cwd"] == "."));
    assert!(broader
        .iter()
        .any(|command| command["cwd"] == "packages/other"));
}

#[test]
fn impact_keeps_deleted_sources_and_cycles_in_the_dependency_evidence() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    std::fs::remove_file(root.path().join(fixture::CHANGED)).unwrap();
    std::fs::write(
        root.path().join("packages/auth/src/cycle.mjs"),
        "export * from './session.mjs';",
    )
    .unwrap();
    let session = root.path().join("packages/auth/src/session.mjs");
    let body = std::fs::read_to_string(&session).unwrap();
    std::fs::write(session, format!("export * from './cycle.mjs';\n{body}")).unwrap();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("deleted", root.path(), Some(state.path()));
    let result = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"paths":[fixture::CHANGED]}),
    );
    let paths: BTreeSet<_> = result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, fixture::IMPACTED.into_iter().collect());
    assert_eq!(result["partial"], true);
    assert!(result["results"].as_array().unwrap().iter().all(|row| {
        row["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["source"] == "unresolved-import-candidate")
    }));
}

#[test]
fn workspace_lock_change_selects_all_tests_with_configuration_evidence() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let all = fixture::populate(root.path());
    fixture::write(root.path(), "pnpm-lock.yaml", "lockfileVersion: '9.0'\n");
    let mut host =
        NativeExtensionHost::new_with_agent_dir("lock-impact", root.path(), Some(state.path()));
    let result = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"path":"pnpm-lock.yaml","limit":100}),
    );
    assert_eq!(result["total"], all.len());
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["reasons"][0]["kind"] == "configuration"));
}

#[test]
fn impact_respects_package_managers_and_never_narrows_unknown_commands() {
    for (manager, script, expected_args) in [
        (
            "pnpm@10.0.0",
            "vitest run",
            json!(["run", "test", "test/example.test.ts"]),
        ),
        (
            "yarn@4.0.0",
            "jest",
            json!([
                "run",
                "test",
                "--runTestsByPath",
                "--watch=false",
                "--watchAll=false",
                "test/example.test.ts"
            ]),
        ),
        (
            "bun@1.0.0",
            "vitest",
            json!(["run", "test", "--run", "test/example.test.ts"]),
        ),
        ("npm@10.0.0", "vitest bench", json!(["run", "test"])),
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("test")).unwrap();
        std::fs::write(root.path().join("example.ts"), "export const value = 1;").unwrap();
        std::fs::write(
            root.path().join("test/example.test.ts"),
            "import { value } from '../example';",
        )
        .unwrap();
        std::fs::write(
            root.path().join("package.json"),
            json!({"packageManager":manager,"scripts":{"test":script}}).to_string(),
        )
        .unwrap();
        let mut host =
            NativeExtensionHost::new_with_agent_dir("commands", root.path(), Some(state.path()));
        let result = call(
            &mut host,
            root.path(),
            "test_plan",
            json!({"paths":["example.ts"]}),
        );
        let command = &result["first_tier"][0];
        assert_eq!(command["program"], manager.split('@').next().unwrap());
        assert_eq!(command["argv"], expected_args, "{script}");
        assert_eq!(command["requires_authorization"], true);
    }
}

#[test]
fn impact_warm_requests_read_only_changed_sources_and_allow_forced_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    let mut host =
        NativeExtensionHost::new_with_agent_dir("incremental", root.path(), Some(state.path()));
    assert_eq!(
        host.repo_intelligence.status()["observation"]["started"],
        false
    );
    let args = json!({"paths":[fixture::CHANGED]});
    let cold = call(&mut host, root.path(), "test_plan", args.clone());
    let warm = call(&mut host, root.path(), "test_plan", args.clone());
    assert_eq!(warm["telemetry"]["mapping_cache_hit"], true);
    assert!(
        warm["telemetry"]["source_bytes_read"].as_u64().unwrap()
            < cold["telemetry"]["source_bytes_read"].as_u64().unwrap() / 2,
        "{warm}"
    );
    fixture::plant_failure(root.path());
    let changed = call(&mut host, root.path(), "test_plan", args);
    assert_eq!(changed["telemetry"]["reparsed"], 1);
    assert_ne!(changed["source_identity"], cold["source_identity"]);
    let forced = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"paths":[fixture::CHANGED],"refresh":true}),
    );
    assert_eq!(forced["source_identity"], changed["source_identity"]);
    assert_eq!(forced["telemetry"]["refresh_mode"], "full");
    assert!(
        forced["telemetry"]["source_bytes_read"].as_u64().unwrap()
            > warm["telemetry"]["source_bytes_read"].as_u64().unwrap()
    );
}

fn call(host: &mut NativeExtensionHost, root: &std::path::Path, name: &str, args: Value) -> Value {
    let result = host.execute_tool(root, name, &args).unwrap();
    assert!(!result.is_error, "{}", result.content);
    serde_json::from_str(&result.content).unwrap()
}

#[test]
fn test_impact_finds_cross_package_consumers_with_evidence() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    let mut host =
        NativeExtensionHost::new_with_agent_dir("test-impact", root.path(), Some(state.path()));
    let result = call(
        &mut host,
        root.path(),
        "test_impacted",
        json!({"paths":[fixture::CHANGED]}),
    );
    let rows = result["results"].as_array().unwrap();
    let paths: BTreeSet<_> = rows
        .iter()
        .map(|row| row["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, fixture::IMPACTED.into_iter().collect());
    for row in rows {
        assert!(!row["reasons"].as_array().unwrap().is_empty());
        assert!(row["package"].as_str().unwrap().starts_with("packages/"));
    }
    let login = rows
        .iter()
        .find(|row| row["path"] == fixture::IMPACTED[2])
        .unwrap();
    assert!(login["reasons"].as_array().unwrap().iter().any(|reason| {
        reason["chain"]
            .as_array()
            .is_some_and(|chain| chain.len() == 4)
    }));
    assert_eq!(result["remaining"], 0);
    assert!(result["broader_verification"]
        .as_array()
        .is_some_and(|commands| !commands.is_empty()));
}

#[test]
fn test_impact_is_a_native_read_capability_and_limits_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    let mut host =
        NativeExtensionHost::new_with_agent_dir("test-capability", root.path(), Some(state.path()));
    for name in ["test_related", "test_impacted", "test_plan"] {
        assert!(host.has_tool(name), "{name} missing");
        assert_eq!(
            davinci_agent::tool_class(name),
            davinci_agent::ToolClass::Read
        );
        assert_eq!(
            NativeExtensionHost::describe_tool(name).unwrap().parameters["type"],
            "object"
        );
    }
    let limited = call(
        &mut host,
        root.path(),
        "test_plan",
        json!({"paths":[fixture::CHANGED],"limit":1}),
    );
    assert_eq!(limited["results"].as_array().unwrap().len(), 1);
    assert_eq!(limited["total"], 3);
    assert_eq!(limited["remaining"], 2);
    assert_eq!(limited["truncated"], true);
}

#[test]
fn test_impact_rejects_unsafe_paths_unknown_fields_and_disabled_requests() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fixture::populate(root.path());
    let mut host =
        NativeExtensionHost::new_with_agent_dir("test-boundaries", root.path(), Some(state.path()));
    for args in [
        json!({"paths":["../outside.ts"]}),
        json!({"paths":[".env"]}),
        json!({"paths":["node_modules/private/file.ts"]}),
        json!({"paths":[fixture::CHANGED],"execute":true}),
        json!({"paths":[fixture::CHANGED],"limit":0}),
    ] {
        assert!(host.execute_tool(root.path(), "test_plan", &args).is_err());
    }
    std::fs::write(
        state.path().join("settings.json"),
        r#"{"testImpact":{"enabled":false}}"#,
    )
    .unwrap();
    let mut disabled =
        NativeExtensionHost::new_with_agent_dir("test-disabled", root.path(), Some(state.path()));
    let error = disabled
        .execute_tool(
            root.path(),
            "test_impacted",
            &json!({"paths":[fixture::CHANGED]}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("disabled"), "{error}");
}
