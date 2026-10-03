//! Offline regressions for the Design host boundary and shared RPC watcher.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn fixture_runtime() -> (tempfile::TempDir, RpcRuntime) {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("system");
    agent.cwd = directory.path().to_path_buf();
    let runtime = RpcRuntime::with_models(
        agent,
        directory.path().join("sessions"),
        directory.path().to_path_buf(),
        Vec::new(),
    );
    (directory, runtime)
}

#[test]
fn malformed_design_command_has_a_structured_failure_and_nonzero_exit() {
    let (_directory, mut runtime) = fixture_runtime();
    runtime
        .agent
        .prompt_user_with("/design new \"unterminated", &[]);
    let parsed = Args {
        offline: true,
        no_extensions: true,
        ..Args::default()
    };
    let (reply, events) = complete_prompt_with_host(&parsed, &mut runtime.agent, None, false);
    assert!(!reply.is_empty());
    assert_eq!(print_text_exit(&events), (1, Some(reply.clone())));
    assert!(print_run_failed(&events, &reply));
    let wire = to_json_print_event(events.last().unwrap()).unwrap();
    assert_eq!(wire["message"]["designCommandError"], reply);
}

#[test]
fn offline_design_generation_fails_before_any_provider_operation() {
    let (_directory, mut runtime) = fixture_runtime();
    runtime
        .agent
        .prompt_user_with("/design new a settings screen", &[]);
    let parsed = Args {
        offline: true,
        no_extensions: true,
        ..Args::default()
    };
    let (reply, events) = complete_prompt_with_host(&parsed, &mut runtime.agent, None, false);
    assert_eq!(print_text_exit(&events).0, 1);
    assert!(reply.contains("offline"));
    assert!(events
        .iter()
        .all(|event| matches!(event, AgentEvent::MessageEnd { .. })));
}

#[test]
fn rpc_abort_is_consumed_while_a_host_operation_is_blocked() {
    let (_directory, mut runtime) = fixture_runtime();
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = Mutex::new(rx);
    let leftover = Mutex::new(std::collections::VecDeque::new());
    let active = Mutex::new(None);
    let original = Arc::new(AtomicBool::new(false));
    runtime.agent.abort_signal = Some(original.clone());
    // An unrelated request must be retained, not discarded or mistaken for abort.
    tx.send(r#"{"type":"get_state","id":"later"}"#.to_string())
        .unwrap();
    tx.send(r#"{"type":"abort","id":"cancel-design"}"#.to_string())
        .unwrap();
    with_rpc_operation_watch(&mut runtime, &leftover, &rx, &active, |runtime| {
        let signal = runtime.agent.abort_signal.as_ref().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !signal.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            signal.load(Ordering::Acquire),
            "RPC abort was queued instead of consumed"
        );
        // Exercise the actual Design facade too: it must inherit this flag,
        // rather than fail later on the deliberately absent session/model.
        use davinci_coding_agent::design::{controller::*, error::DesignError, store::DesignStore};
        let controller =
            DesignController::new(DesignStore::new(runtime.cwd.join("unused-blobs")), true);
        assert!(matches!(
            controller.execute(&mut runtime.agent, &runtime.cwd, DesignRequest::List {}),
            Err(DesignError::Cancelled)
        ));
        assert!(!runtime.cwd.join("unused-blobs").exists());
    });
    assert!(active.lock().unwrap().is_none());
    assert!(Arc::ptr_eq(
        runtime.agent.abort_signal.as_ref().unwrap(),
        &original
    ));
    assert!(!original.load(Ordering::Acquire));
    assert!(leftover
        .lock()
        .unwrap()
        .pop_front()
        .unwrap()
        .contains("get_state"));
}

#[test]
fn rpc_supervisor_restores_abort_bindings_after_a_host_panic() {
    let (_directory, mut runtime) = fixture_runtime();
    let (_tx, rx) = std::sync::mpsc::channel();
    let rx = Mutex::new(rx);
    let leftover = Mutex::new(std::collections::VecDeque::new());
    let original = Arc::new(AtomicBool::new(false));
    let original_ui = Arc::new(AtomicBool::new(false));
    let active = Mutex::new(Some(original_ui.clone()));
    runtime.agent.abort_signal = Some(original.clone());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_rpc_operation_watch(&mut runtime, &leftover, &rx, &active, |_| {
            panic!("recorded host-operation failure");
        });
    }));
    assert!(outcome.is_err());
    assert!(Arc::ptr_eq(
        runtime.agent.abort_signal.as_ref().unwrap(),
        &original
    ));
    assert!(Arc::ptr_eq(
        active.lock().unwrap().as_ref().unwrap(),
        &original_ui
    ));
}
