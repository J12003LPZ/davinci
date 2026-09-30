//! Native DaVinci default activation, intentionally not TypeScript parity.
use davinci_agent::{Agent, PermissionMode};
use davinci_protocol::{SandboxBackendKind, SandboxSpec};

fn policy(root: &std::path::Path) -> SandboxSpec {
    serde_json::from_value(serde_json::json!({
        "id": "probed-default", "mode": "workspace_write", "backend": "auto",
        "workspace": root.canonicalize().unwrap().to_string_lossy(),
        "network": {"mode":"denied"}
    }))
    .unwrap()
}

#[test]
fn entering_auto_through_public_mode_api_activates_default_before_next_tool() {
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::Ask);
    agent.configure_auto_sandbox(Some(policy(root.path())));
    assert!(agent.tool_context.sandbox.is_none());
    agent.set_permission_mode(PermissionMode::Auto);
    assert_eq!(
        agent.tool_context.sandbox.as_ref().unwrap().id.0,
        "probed-default"
    );
    // Mode changes cannot silently widen an activated policy.
    agent.set_permission_mode(PermissionMode::Ask);
    assert!(agent.tool_context.sandbox.is_some());
    agent.set_permission_mode(PermissionMode::Auto);
    agent.set_plan_mode(true);
    assert_eq!(agent.permission_mode(), PermissionMode::ReadOnly);
    agent.set_plan_mode(false);
    assert_eq!(agent.permission_mode(), PermissionMode::Auto);
    assert!(agent.tool_context.sandbox.is_some());
}

#[test]
fn activation_guard_observes_execution_started_after_default_configuration() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::Ask);
    let legacy = Arc::new(AtomicBool::new(false));
    let observed = legacy.clone();
    agent.set_auto_sandbox_activation_guard(Arc::new(move || {
        observed
            .load(Ordering::SeqCst)
            .then(|| "unowned JS descendant history".into())
    }));
    agent.configure_auto_sandbox(Some(policy(root.path())));
    legacy.store(true, Ordering::SeqCst);
    agent.set_permission_mode(PermissionMode::Auto);
    assert!(agent.tool_context.sandbox.is_none());
    assert_eq!(
        agent.auto_sandbox_unavailable_reason(),
        Some("unowned JS descendant history")
    );
}

#[test]
fn unowned_preexisting_execution_prevents_future_auto_boundary() {
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::Ask);
    agent.configure_auto_sandbox(Some(policy(root.path())));
    agent.disable_auto_sandbox("legacy MCP requires restart in Auto");
    agent.set_permission_mode(PermissionMode::Auto);
    assert!(agent.tool_context.sandbox.is_none());
    assert_eq!(
        agent.auto_sandbox_unavailable_reason(),
        Some("legacy MCP requires restart in Auto")
    );
}

#[test]
fn defaults_never_replace_explicit_policy_and_no_backend_keeps_legacy_path() {
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    let mut explicit = policy(root.path());
    explicit.id.0 = "explicit".into();
    explicit.backend = SandboxBackendKind::Host;
    agent.tool_context.sandbox = Some(explicit);
    agent.configure_auto_sandbox(Some(policy(root.path())));
    agent.set_permission_mode(PermissionMode::Auto);
    assert_eq!(
        agent.tool_context.sandbox.as_ref().unwrap().id.0,
        "explicit"
    );
    let mut unavailable = Agent::new("fixture");
    unavailable.configure_auto_sandbox(None);
    unavailable.set_permission_mode(PermissionMode::Auto);
    assert!(unavailable.tool_context.sandbox.is_none());
}

#[test]
fn plan_exit_and_mode_cycle_use_same_auto_activation_and_host_fence() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::ReadOnly);
    let enabled = Arc::new(AtomicBool::new(false));
    let observer = enabled.clone();
    agent.set_permission_mode_change_hook(Arc::new(move |mode, _context| {
        if mode == PermissionMode::Auto {
            observer.store(true, Ordering::SeqCst);
        }
    }));
    agent.configure_auto_sandbox(Some(policy(root.path())));
    for _ in 0..10 {
        if agent.cycle_permission_mode() == PermissionMode::Auto {
            break;
        }
    }
    assert_eq!(agent.permission_mode(), PermissionMode::Auto);
    assert!(enabled.load(Ordering::SeqCst));
    assert!(agent.tool_context.sandbox.is_some());
}

#[test]
fn containment_does_not_loosen_network_or_outside_command_approval() {
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::Auto);
    let commands = [
        "curl https://example.invalid",
        "cat /etc/shadow",
        "touch /tmp/outside-workspace",
    ];
    let before: Vec<_> = commands
        .iter()
        .map(|command| {
            agent.permissions.lock().unwrap().decide(
                "fixture",
                "bash",
                &serde_json::json!({"command":command}),
                root.path(),
            )
        })
        .collect();
    agent.configure_auto_sandbox(Some(policy(root.path())));
    for (command, before) in commands.iter().zip(before) {
        let after = agent.permissions.lock().unwrap().decide(
            "fixture",
            "bash",
            &serde_json::json!({"command":command}),
            root.path(),
        );
        assert_eq!(before, after);
        assert!(!matches!(after, davinci_agent::PermissionVerdict::Allow));
    }
}
