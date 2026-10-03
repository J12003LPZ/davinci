use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState, ToolClass};
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
        r#"{"workspaceSnapshots":{"enabled":true,"maxFiles":8}}"#,
    )
    .unwrap();
    let host =
        NativeExtensionHost::new_with_agent_dir("workspace-snapshot-test", root, Some(state));
    host.workspace_snapshot
        .set_permissions(Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        ))));
    host
}

fn call(host: &mut NativeExtensionHost, root: &std::path::Path, name: &str, args: Value) -> Value {
    host.execute_tool(root, name, &args)
        .unwrap()
        .details
        .unwrap()
}

#[test]
fn registration_settings_status_and_least_privilege_are_wired() {
    let mut host = NativeExtensionHost::default();
    for name in [
        "workspace_checkpoint",
        "workspace_diff",
        "workspace_restore",
    ] {
        assert!(host.has_tool(name));
        assert_eq!(
            NativeExtensionHost::describe_tool(name).unwrap().parameters["type"],
            "object"
        );
    }
    assert_eq!(
        davinci_agent::tool_class("workspace_checkpoint"),
        ToolClass::Read
    );
    assert_eq!(davinci_agent::tool_class("workspace_diff"), ToolClass::Read);
    assert_eq!(
        davinci_agent::tool_class("workspace_restore"),
        ToolClass::Edit
    );
    assert!(host.command("workspace-status", "").unwrap().is_some());
}

#[test]
fn checkpoint_and_diff_are_bounded_and_do_not_expose_file_bytes() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    write(root.path(), "src/value.rs", "pub fn value() -> u8 { 1 }");
    let mut host = host(root.path(), state.path());
    let checkpoint = call(
        &mut host,
        root.path(),
        "workspace_checkpoint",
        json!({"paths":["src/value.rs"],"label":"before edit"}),
    );
    assert_eq!(checkpoint["complete"], true);
    let id = checkpoint["checkpoint"]["id"].as_str().unwrap();
    assert!(checkpoint["checkpoint"]["entries"][0]
        .get("bytes")
        .is_none());
    write(root.path(), "src/value.rs", "pub fn value() -> u8 { 2 }");
    let diff = call(
        &mut host,
        root.path(),
        "workspace_diff",
        json!({"checkpointId":id}),
    );
    assert_eq!(diff["complete"], true);
    assert_eq!(diff["changes"][0]["status"], "changed");
    assert_eq!(diff["changes"][0]["safe_to_restore"], false);
}

#[test]
fn context_workspace_uses_its_own_snapshot_store_and_journal() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let worker = root.path().join("worker");
    std::fs::create_dir_all(&worker).unwrap();
    write(&worker, "src/value.rs", "pub fn value() -> u8 { 1 }");
    let host = host(root.path(), state.path());
    let context = davinci_agent::ToolContext::default();

    let checkpoint = host
        .workspace_snapshot
        .execute_with_context(
            &worker,
            "workspace_checkpoint",
            &json!({"path":"src/value.rs"}),
            Some(&context),
        )
        .unwrap()
        .details
        .unwrap();
    let id = checkpoint["checkpoint"]["id"].as_str().unwrap();
    assert!(worker
        .join(".davinci-workspace-snapshots")
        .join(format!("{id}.json"))
        .is_file());
    assert!(!root
        .path()
        .join(".davinci-workspace-snapshots")
        .join(format!("{id}.json"))
        .exists());

    write(&worker, "src/value.rs", "pub fn value() -> u8 { 2 }");
    let diff = host
        .workspace_snapshot
        .execute_with_context(
            &worker,
            "workspace_diff",
            &json!({"checkpointId":id}),
            Some(&context),
        )
        .unwrap()
        .details
        .unwrap();
    assert_eq!(diff["changes"][0]["status"], "changed");
}

#[test]
fn checkpoint_identity_cannot_be_reused_in_another_workspace() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first_state = tempfile::tempdir().unwrap();
    let second_state = tempfile::tempdir().unwrap();
    write(first.path(), "src/value.rs", "pub fn value() -> u8 { 1 }");
    write(second.path(), "src/value.rs", "pub fn value() -> u8 { 1 }");

    let mut first_host = host(first.path(), first_state.path());
    let checkpoint = call(
        &mut first_host,
        first.path(),
        "workspace_checkpoint",
        json!({"path":"src/value.rs"}),
    );
    let id = checkpoint["checkpoint"]["id"].as_str().unwrap();
    let second_store = second.path().join(".davinci-workspace-snapshots");
    std::fs::create_dir_all(&second_store).unwrap();
    std::fs::copy(
        first
            .path()
            .join(".davinci-workspace-snapshots")
            .join(format!("{id}.json")),
        second_store.join(format!("{id}.json")),
    )
    .unwrap();

    let mut second_host = host(second.path(), second_state.path());
    assert!(second_host
        .execute_tool(second.path(), "workspace_diff", &json!({"checkpointId":id}),)
        .is_err());
}

#[test]
fn traversal_and_sensitive_paths_fail_before_capture() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut host = host(root.path(), state.path());
    for path in ["../secret.txt", ".env", ".git/config"] {
        assert!(host
            .execute_tool(root.path(), "workspace_checkpoint", &json!({"path":path}),)
            .is_err());
    }
}

#[cfg(unix)]
#[test]
fn symlink_targets_that_escape_are_rejected() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    write(root.path(), "src/value.rs", "pub fn value() -> u8 { 1 }");
    symlink(
        "/tmp/outside-davinci-snapshot",
        root.path().join("src/link"),
    )
    .unwrap();
    let mut host = host(root.path(), state.path());
    assert!(host
        .execute_tool(
            root.path(),
            "workspace_checkpoint",
            &json!({"path":"src/link"}),
        )
        .is_err());
}

#[test]
fn cancellation_is_partial_and_disabled_settings_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    write(root.path(), "src/value.rs", "pub fn value() -> u8 { 1 }");
    let mut enabled = host(root.path(), state.path());
    enabled
        .workspace_snapshot
        .set_cancellation(Some(Arc::new(std::sync::atomic::AtomicBool::new(true))));
    let cancelled = call(
        &mut enabled,
        root.path(),
        "workspace_checkpoint",
        json!({"path":"src/value.rs"}),
    );
    assert_eq!(cancelled["cancelled"], true);
    assert_eq!(cancelled["mutation"], Value::Null);

    std::fs::write(
        state.path().join("settings.json"),
        r#"{"workspaceSnapshots":{"enabled":false}}"#,
    )
    .unwrap();
    let mut disabled = NativeExtensionHost::new_with_agent_dir(
        "workspace-snapshot-disabled",
        root.path(),
        Some(state.path()),
    );
    let result = disabled
        .execute_tool(
            root.path(),
            "workspace_checkpoint",
            &json!({"path":"src/value.rs"}),
        )
        .unwrap()
        .details
        .unwrap();
    assert_eq!(result["enabled"], false);
    assert_eq!(result["partial"], true);
}
