//! Public managed-process tool contract; lifecycle fixtures live beside the owner.
#[test]
fn managed_process_tools_have_native_schemas() {
    let schemas = davinci_agent::tools::tool_specs();
    for name in [
        "process_start",
        "process_status",
        "process_output",
        "process_write",
        "process_stop",
        "process_list",
    ] {
        let schema = schemas
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("missing managed process tool: {name}"));
        assert_eq!(schema.parameters["type"], "object");
        assert_eq!(schema.parameters["additionalProperties"], false);
    }
    let start = &schemas
        .iter()
        .find(|tool| tool.name == "process_start")
        .unwrap()
        .parameters["properties"];
    assert_eq!(start["restart"]["properties"]["max_restarts"]["maximum"], 3);
    assert_eq!(start["restart"]["additionalProperties"], false);
    assert_eq!(start["ports"]["maxItems"], 8);
    assert_eq!(start["ports"]["uniqueItems"], true);
}

#[test]
fn managed_process_permissions_preserve_shell_denies_and_modes() {
    use davinci_agent::{PermissionMode, PermissionPolicy, PermissionRule, PermissionVerdict};
    use serde_json::json;
    let cwd = std::path::Path::new(".");
    for shell in ["Bash", "powershell", "exec_command"] {
        let mut policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
        policy
            .deny
            .push(PermissionRule::parse(&format!("{shell}(git push *)")).unwrap());
        for executable in ["git", "C:/Program Files/Git/bin/git.exe"] {
            assert!(
                matches!(
                    policy.decide(
                        "call",
                        "process_start",
                        &json!({"executable":executable,"argv":["push","origin","main"]}),
                        cwd
                    ),
                    PermissionVerdict::Deny { .. }
                ),
                "{shell} / {executable}"
            );
        }
    }
    let read_only = PermissionPolicy::new(PermissionMode::ReadOnly);
    assert!(matches!(
        read_only.decide("call", "process_status", &json!({"id":1}), cwd),
        PermissionVerdict::Allow
    ));
    assert!(matches!(
        read_only.decide(
            "call",
            "process_start",
            &json!({"executable":"node","argv":["server.js"]}),
            cwd
        ),
        PermissionVerdict::Deny { .. }
    ));
    let auto = PermissionPolicy::new(PermissionMode::Auto);
    assert!(matches!(
        auto.decide(
            "call",
            "process_start",
            &json!({"executable":"npm","argv":["test"]}),
            cwd
        ),
        PermissionVerdict::Ask(_)
    ));
}
