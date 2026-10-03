use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use serde_json::json;
use std::sync::Arc;

#[test]
fn workspace_snapshot_eval_preserves_conflict_first_restore_contract() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(
        state.path().join("settings.json"),
        r#"{"workspaceSnapshots":{"enabled":true}}"#,
    )
    .unwrap();
    std::fs::write(root.path().join("value.txt"), "before").unwrap();
    let host = NativeExtensionHost::new_with_agent_dir(
        "workspace-snapshot-eval",
        root.path(),
        Some(state.path()),
    );
    host.workspace_snapshot
        .set_permissions(Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        ))));
    let mut host = host;
    let checkpoint = host
        .execute_tool(
            root.path(),
            "workspace_checkpoint",
            &json!({"path":"value.txt"}),
        )
        .unwrap()
        .details
        .unwrap();
    let checkpoint_id = checkpoint["checkpoint"]["id"].as_str().unwrap();
    std::fs::write(root.path().join("value.txt"), "unowned edit").unwrap();
    let restore = host
        .execute_tool(
            root.path(),
            "workspace_restore",
            &json!({"checkpointId":checkpoint_id}),
        )
        .unwrap()
        .details
        .unwrap();
    assert_eq!(restore["complete"], false);
    assert_eq!(restore["partial"], true);
    assert_eq!(restore["mutation"], "none");
    assert!(restore["conflicts"].as_array().unwrap().len() == 1);
    assert_eq!(
        std::fs::read_to_string(root.path().join("value.txt")).unwrap(),
        "unowned edit"
    );
}
