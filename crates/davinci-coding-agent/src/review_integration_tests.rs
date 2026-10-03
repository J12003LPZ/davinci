//! Offline regression coverage for the combined PR execution boundaries.
use super::*;
use davinci_coding_agent::design::{admission::AuthorizedDesignContext, error::DesignError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn review_design_failure_is_typed_and_has_nonzero_exit_status() {
    for error in [
        DesignError::InvalidInput("bad command".into()),
        DesignError::MissingCapability("no admitted runtime".into()),
        DesignError::NotFound("missing artifact".into()),
        DesignError::Cancelled,
    ] {
        let mut agent = Agent::new("fixture");
        let (text, events) = record_design_result(&mut agent, Err(error.clone()));
        assert_eq!(print_text_exit(&events).0, 1, "{error}");
        assert!(print_run_failed(&events, &text));
        let AgentEvent::MessageEnd { message } = &events[0] else {
            panic!("terminal event required")
        };
        assert_eq!(message.extra["details"]["success"], false);
        assert_eq!(
            message.extra["details"]["error"],
            serde_json::to_value(error).unwrap()
        );
        let wire = to_json_print_event(&events[0]).unwrap();
        assert_eq!(wire["type"], "message_end");
    }
}

#[test]
fn review_design_success_does_not_turn_error_looking_content_into_failure() {
    let mut agent = Agent::new("fixture");
    let (text, events) = record_design_result(
        &mut agent,
        Ok(serde_json::json!({"label":"InvalidInput: user text"})),
    );
    assert_eq!(print_text_exit(&events).0, 0);
    assert!(!print_run_failed(&events, &text));
    let AgentEvent::MessageEnd { message } = &events[0] else {
        panic!("terminal event required")
    };
    assert_eq!(message.extra["details"]["success"], true);
}

#[test]
fn review_malformed_design_command_fails_through_the_real_host_dispatcher() {
    let mut agent = Agent::new("fixture");
    agent.prompt("/design export");
    let (_, events) = complete_prompt_with_host(&Args::default(), &mut agent, None, false);
    assert_eq!(print_text_exit(&events).0, 1);
    assert_eq!(agent.stats.model_turns, 0);
}

fn rpc_fixture() -> (tempfile::TempDir, RpcRuntime) {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.cwd = cwd.clone();
    agent.set_permission_mode(davinci_agent::PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&cwd.join("sessions"), &cwd.to_string_lossy(), None)
            .unwrap(),
    );
    let runtime = RpcRuntime::with_models(agent, cwd.join("sessions"), cwd, vec![]);
    (temp, runtime)
}

#[test]
fn review_rpc_abort_reaches_blocked_design_and_preserves_other_commands() {
    let (_temp, mut runtime) = rpc_fixture();
    let previous = Arc::new(AtomicBool::new(false));
    runtime.agent.abort_signal = Some(previous.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = Arc::new(Mutex::new(rx));
    let leftover = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let active = Arc::new(Mutex::new(None));
    let queued = r#"{"type":"get_state","id":"after-design"}"#.to_string();
    tx.send(queued.clone()).unwrap();
    tx.send(r#"{"type":"abort","id":"cancel-design"}"#.to_string())
        .unwrap();
    let cancelled =
        with_cancellable_rpc_operation(&mut runtime, &leftover, &rx, &active, |runtime| {
            let context =
                AuthorizedDesignContext::from_agent(&runtime.agent, &runtime.cwd).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if context.check_session(runtime.agent.session.as_ref().unwrap())
                    == Err(DesignError::Cancelled)
                {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
    assert!(
        cancelled,
        "transport abort must reach the active Design context"
    );
    assert!(Arc::ptr_eq(
        runtime.agent.abort_signal.as_ref().unwrap(),
        &previous
    ));
    assert!(!previous.load(Ordering::Acquire));
    assert!(active.lock().unwrap().is_none());
    assert_eq!(rpc_next_line(&leftover, &rx), Some(queued));
}

#[test]
fn review_rpc_supervisor_stops_and_restores_state_when_operation_panics() {
    let (_temp, mut runtime) = rpc_fixture();
    let previous = Arc::new(AtomicBool::new(false));
    runtime.agent.abort_signal = Some(previous.clone());
    let (_tx, rx) = std::sync::mpsc::channel();
    let rx = Arc::new(Mutex::new(rx));
    let leftover = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let active = Arc::new(Mutex::new(None));
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_cancellable_rpc_operation(&mut runtime, &leftover, &rx, &active, |_| {
            panic!("fixture panic")
        });
    }));
    assert!(panic.is_err());
    assert!(Arc::ptr_eq(
        runtime.agent.abort_signal.as_ref().unwrap(),
        &previous
    ));
    assert!(active.lock().unwrap().is_none());
}

#[test]
fn review_design_context_checks_host_abort_without_overwriting_runtime_abort() {
    let (_temp, mut runtime) = rpc_fixture();
    let host = Arc::new(AtomicBool::new(false));
    let background = Arc::new(AtomicBool::new(false));
    runtime.agent.abort_signal = Some(host.clone());
    runtime.agent.tool_context.abort = Some(background.clone());
    let context = AuthorizedDesignContext::from_agent(&runtime.agent, &runtime.cwd).unwrap();
    host.store(true, Ordering::Release);
    assert_eq!(
        context.check_session(runtime.agent.session.as_ref().unwrap()),
        Err(DesignError::Cancelled)
    );
    assert!(!background.load(Ordering::Acquire));
}

#[test]
fn review_saved_unsupported_fast_is_labelled_unavailable_and_can_be_disabled() {
    let mut agent = Agent::new("fixture");
    agent.provider = "openai-codex".into();
    agent.model_id = "fixture-model".into();
    agent.service_tier = davinci_ai::CodexServiceTier::Fast;
    assert!(
        fast::speed_label(&agent, davinci_ai::FastCapability::Unsupported).contains("unavailable")
    );
    fast::toggle(&mut agent, davinci_ai::FastCapability::Unsupported).unwrap();
    assert_eq!(agent.service_tier, davinci_ai::CodexServiceTier::Standard);
}
