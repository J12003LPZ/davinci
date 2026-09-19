use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use serde_json::{json, Value};
use std::sync::Arc;

fn write(root: &std::path::Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn host(root: &std::path::Path, state: &std::path::Path) -> NativeExtensionHost {
    std::fs::write(
        state.join("settings.json"),
        r#"{"verificationPlanner":{"enabled":true}}"#,
    )
    .unwrap();
    let host =
        NativeExtensionHost::new_with_agent_dir("verification-planner-test", root, Some(state));
    host.verification_planner
        .set_permissions(Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        ))));
    host
}

fn call(host: &mut NativeExtensionHost, root: &std::path::Path, args: Value) -> Value {
    host.execute_tool(root, "verification_plan", &args)
        .unwrap()
        .details
        .unwrap()
}

#[test]
fn registration_status_permissions_and_planning_only_contract_are_wired() {
    let mut host = NativeExtensionHost::default();
    assert!(host.has_tool("verification_plan"));
    assert_eq!(
        NativeExtensionHost::describe_tool("verification_plan")
            .unwrap()
            .parameters["type"],
        "object"
    );
    assert_eq!(
        davinci_agent::tool_class("verification_plan"),
        davinci_agent::ToolClass::Read
    );
    assert!(host.command("verification-status", "").unwrap().is_some());
}

#[test]
fn docs_only_plans_have_no_runtime_steps() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    write(root.path(), "README.md", "documentation");
    let mut host = host(root.path(), state.path());
    let result = call(&mut host, root.path(), json!({"files":["README.md"]}));
    assert_eq!(result["enabled"], true);
    assert!(result["steps"].as_array().unwrap().is_empty());
    assert!(result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("documentation-only")));
    assert_eq!(result["complete"], false);
    assert!(result["sourceIdentity"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn frontend_security_plan_orders_cheap_checks_before_broader_checks() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "package.json",
        r#"{"scripts":{"test":"vitest","typecheck":"tsc","lint":"eslint","build":"vite build"}}"#,
    );
    write(root.path(), "src/Login.tsx", "export function Login() {}");
    write(
        root.path(),
        "src/auth.ts",
        "export const authorize = () => true;",
    );
    let mut host = host(root.path(), state.path());
    let result = call(
        &mut host,
        root.path(),
        json!({
            "files":["src/Login.tsx","src/auth.ts"],
            "changedSymbols":["Login"],
            "userRequirements":["browser flow and security review"]
        }),
    );
    let operations: Vec<&str> = result["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["operation"].as_str().unwrap())
        .collect();
    let tests = operations.iter().position(|item| *item == "tests").unwrap();
    let browser = operations
        .iter()
        .position(|item| *item == "browser")
        .unwrap();
    let security = operations
        .iter()
        .position(|item| *item == "security")
        .unwrap();
    assert!(tests < browser && browser < security);
    assert!(result["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|requirement| requirement["kind"] == "symbol-impact"));
    assert!(result["steps"]
        .as_array()
        .unwrap()
        .iter()
        .all(|step| step["status"] == "planned"));
}

#[test]
fn traversal_is_rejected_before_any_plan_is_returned() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut host = host(root.path(), state.path());
    let result = host.execute_tool(
        root.path(),
        "verification_plan",
        &json!({"path":"../secret.txt"}),
    );
    assert!(result.is_err());
}

#[test]
fn malformed_settings_fail_closed_and_cancellation_is_partial() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    write(root.path(), "src/lib.rs", "pub fn value() -> u8 { 1 }");
    std::fs::write(
        state.path().join("settings.json"),
        r#"{"verificationPlanner":{"enabled":"yes"}}"#,
    )
    .unwrap();
    let mut disabled =
        NativeExtensionHost::new_with_agent_dir("disabled", root.path(), Some(state.path()));
    let result = disabled
        .execute_tool(
            root.path(),
            "verification_plan",
            &json!({"path":"src/lib.rs"}),
        )
        .unwrap()
        .details
        .unwrap();
    assert_eq!(result["enabled"], false);

    let state = tempfile::tempdir().unwrap();
    let mut cancelled = host(root.path(), state.path());
    let signal = Arc::new(std::sync::atomic::AtomicBool::new(true));
    cancelled
        .verification_planner
        .set_cancellation(Some(signal));
    let result = call(&mut cancelled, root.path(), json!({"path":"src/lib.rs"}));
    assert_eq!(result["partial"], true);
    assert_eq!(result["complete"], false);
    assert!(result["warnings"][0]
        .as_str()
        .unwrap()
        .contains("cancelled"));
}
