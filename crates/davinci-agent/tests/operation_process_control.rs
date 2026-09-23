use davinci_agent::{
    jobs::{supervisor::SupervisorCommand, JobBook},
    process_manager::ProcessManager,
    PermissionMode, PermissionPolicy, PermissionState,
};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

fn manager(directory: &Path) -> ProcessManager {
    let permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::AlwaysApprove,
    )));
    ProcessManager::new(
        directory,
        Arc::new(Mutex::new(JobBook::default())),
        permissions,
        SupervisorCommand {
            executable: std::env::current_exe().unwrap(),
            argv: vec![
                "--exact".into(),
                "operation_process_control_host_fixture".into(),
                "--nocapture".into(),
            ],
        },
    )
    .unwrap()
}

fn start(directory: &Path, manager: &ProcessManager) -> (u32, Uuid) {
    let result = manager
        .execute(
            directory,
            "process_start",
            &json!({
                "executable": "node",
                "argv": ["-e", "process.stdin.on('data',b=>process.stdout.write(b));setTimeout(()=>{},20000)"]
            }),
            None,
            None,
        )
        .unwrap();
    let details = result.details.unwrap();
    let process = &details["process"];
    (
        process["id"].as_u64().unwrap() as u32,
        Uuid::parse_str(process["lifetime"].as_str().unwrap()).unwrap(),
    )
}

#[test]
fn operation_process_control_host_fixture() {
    // This test is only a supervisor entry point when a child invokes it.
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
        davinci_agent::jobs::supervisor::run();
    }
}

#[test]
fn duplicate_stop_stays_stopping_until_exit_is_observed() {
    let directory = tempfile::tempdir().unwrap();
    let manager = manager(directory.path());
    let (id, lifetime) = start(directory.path(), &manager);
    let first = manager
        .execute(
            directory.path(),
            "process_stop",
            &json!({"id": id, "lifetime": lifetime}),
            None,
            None,
        )
        .unwrap();
    let first_control = &first.details.as_ref().unwrap()["control"];
    assert!(matches!(
        first_control["status"].as_str(),
        Some("stopping") | Some("stopped")
    ));
    if first_control["status"] == "stopping" {
        assert_eq!(first_control["terminated"], false);
        assert!(!first_control["unresolved_descendants"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    let second = manager
        .execute(
            directory.path(),
            "process_stop",
            &json!({"id": id, "lifetime": lifetime}),
            None,
            None,
        )
        .unwrap();
    assert!(matches!(
        second.details.as_ref().unwrap()["control"]["status"].as_str(),
        Some("stopping") | Some("stopped")
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = manager
            .execute(
                directory.path(),
                "process_status",
                &json!({"id": id}),
                None,
                None,
            )
            .unwrap();
        if status.details.unwrap()["state"] == "exited" {
            break;
        }
        assert!(Instant::now() < deadline, "stop did not become observable");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn stale_lifetime_rejects_same_numeric_process_control() {
    let directory = tempfile::tempdir().unwrap();
    let manager = manager(directory.path());
    let (id, lifetime) = start(directory.path(), &manager);
    let stale = Uuid::new_v4();
    let result = manager
        .execute(
            directory.path(),
            "process_write",
            &json!({"id": id, "lifetime": stale, "text": "must-not-run"}),
            None,
            None,
        )
        .unwrap();
    assert!(result.is_error);
    assert_eq!(result.details.unwrap()["control"]["status"], "stale");

    let stop = manager
        .execute(
            directory.path(),
            "process_stop",
            &json!({"id": id, "lifetime": lifetime}),
            None,
            None,
        )
        .unwrap();
    assert!(stop.details.unwrap().get("control").is_some());
}

#[test]
fn acknowledged_stdin_controls_are_distinct_and_not_replayed_by_manager() {
    let directory = tempfile::tempdir().unwrap();
    let manager = manager(directory.path());
    let (id, lifetime) = start(directory.path(), &manager);
    for text in ["one\n", "two\n"] {
        let result = manager
            .execute(
                directory.path(),
                "process_write",
                &json!({"id": id, "lifetime": lifetime, "text": text}),
                None,
                None,
            )
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.details.unwrap()["control"]["effect"], "observed");
    }
    let _ = manager.execute(
        directory.path(),
        "process_stop",
        &json!({"id": id, "lifetime": lifetime}),
        None,
        None,
    );
}

#[test]
fn control_receipt_keeps_owner_session_and_lifetime_binding() {
    let directory = tempfile::tempdir().unwrap();
    let manager = manager(directory.path());
    let (id, lifetime) = start(directory.path(), &manager);
    let status = manager
        .execute(
            directory.path(),
            "process_status",
            &json!({"id": id, "lifetime": lifetime}),
            None,
            None,
        )
        .unwrap();
    let process = status.details.unwrap();
    assert_eq!(process["lifetime"], lifetime.to_string());
    assert!(process["owner"].as_str().is_some());
    assert!(process["session"].as_str().is_some());
    let _ = manager.execute(
        directory.path(),
        "process_stop",
        &json!({"id": id, "lifetime": lifetime}),
        None,
        None,
    );
}
