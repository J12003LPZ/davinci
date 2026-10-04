use davinci_agent::{Agent, PermissionMode, ToolApprovalDecision, ToolApprover};
use davinci_coding_agent::design::{controller::*, error::*, records::*, store::*, types::*};
use davinci_session::JsonlSession;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

fn input() -> DesignRequest {
    DesignRequest::Create {
        input: CreateDesign {
            title: "Test".into(),
            brief: "Test design".into(),
            kind: DesignKind::Product,
            variants: 2,
            operation_id: OperationId::new(),
        },
    }
}
#[test]
fn host_commands_use_existing_one_shot_approval_without_granting_session_permissions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    agent.set_permission_mode(PermissionMode::Ask);
    let count = Arc::new(AtomicU32::new(0));
    let seen = count.clone();
    agent.approver = Some(ToolApprover(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        ToolApprovalDecision::AllowOnce
    })));
    let controller = DesignController::new(DesignStore::new(root.join("blobs")), true);
    controller.execute(&mut agent, &root, input()).unwrap();
    controller.execute(&mut agent, &root, input()).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    agent.approver = None;
    assert!(matches!(
        controller.execute(&mut agent, &root, input()),
        Err(DesignError::Denied(_))
    ));
    agent.set_permission_mode(PermissionMode::ReadOnly);
    assert!(matches!(
        controller.execute(&mut agent, &root, input()),
        Err(DesignError::Denied(_))
    ));
}
#[test]
fn disabled_controller_and_forged_browser_owner_fail_before_effects() {
    let temp = tempfile::tempdir().unwrap();
    let controller = DesignController::new(DesignStore::new(temp.path().join("absent")), false);
    assert!(matches!(
        controller.execute(&mut Agent::new("fixture"), temp.path(), input()),
        Err(DesignError::MissingCapability(_))
    ));
    assert!(!temp.path().join("absent").exists());
    for value in [
        serde_json::json!({"operation":"list","payload":{"owner_session":"forged"}}),
        serde_json::json!({"operation":"list","payload":{},"owner_session":"forged"}),
        serde_json::json!({"operation":"apply","payload":{}}),
    ] {
        assert!(serde_json::from_value::<DesignRequest>(value).is_err());
    }
}
