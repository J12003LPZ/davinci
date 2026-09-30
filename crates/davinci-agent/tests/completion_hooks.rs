//! Blocking completion feedback is executed by the shared agent loop.
use davinci_agent::{Agent, AgentEvent, CompletionHook};
use davinci_ai::AssistantMessage;
use std::sync::{Arc, Mutex};

fn reply(_: &Agent) -> Result<AssistantMessage, String> {
    serde_json::from_value(serde_json::json!({
        "id":"answer", "role":"assistant", "model":"fixture",
        "content":[{"type":"text", "text":"The final answer."}], "stopReason":"stop"
    }))
    .map_err(|error| error.to_string())
}

fn agent(root: &std::path::Path) -> Agent {
    let mut agent = Agent::new("fixture");
    agent.cwd = root.into();
    agent.auto_compaction = false;
    agent.prompt("Explain the parser");
    agent
}

#[test]
fn hook_block_then_allow_sets_active_flag_and_keeps_reason_out_of_session() {
    let root = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = agent(root.path());
    agent.session =
        Some(davinci_session::JsonlSession::create(sessions.path(), "hooks", None).unwrap());
    let flags = Arc::new(Mutex::new(Vec::new()));
    let captured = flags.clone();
    agent.completion_hook = Some(CompletionHook(Arc::new(move |active, _cancelled| {
        let mut flags = captured.lock().unwrap();
        flags.push(active);
        (flags.len() == 1).then(|| "Run the required repo checks.".into())
    })));
    let mut attempts = 0;
    let events = agent
        .run_loop(|agent| {
            attempts += 1;
            if attempts == 2 {
                assert!(serde_json::to_string(&agent.messages_for_provider())
                    .unwrap()
                    .contains("Run the required repo checks."));
            }
            reply(agent)
        })
        .unwrap();
    assert_eq!(*flags.lock().unwrap(), [false, true]);
    assert_eq!(attempts, 2);
    assert_eq!(agent.run_stats().completion_hook_blocks, 1);
    assert!(events.iter().any(|event| matches!(event, AgentEvent::CompletionReminder { reason_code } if reason_code == "completion.hook_block")));
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::CompletionNotice { .. })));
    assert!(
        !std::fs::read_to_string(&agent.session.as_ref().unwrap().path)
            .unwrap()
            .contains("Run the required repo checks.")
    );
    assert!(!serde_json::to_string(&agent.messages_for_provider())
        .unwrap()
        .contains("Run the required repo checks."));
    assert_eq!(
        agent.last_assistant_text().as_deref(),
        Some("The final answer.")
    );
}

#[test]
fn three_hook_blocks_finish_with_visible_notice_and_no_fourth_hook_call() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(dir.path());
    let flags = Arc::new(Mutex::new(Vec::new()));
    let captured = flags.clone();
    agent.completion_hook = Some(CompletionHook(Arc::new(move |active, _cancelled| {
        captured.lock().unwrap().push(active);
        Some("Keep checking".into())
    })));
    let events = agent.run_loop(reply).unwrap();
    assert_eq!(*flags.lock().unwrap(), [false, true, true]);
    assert_eq!(agent.run_stats().model_turns, 4);
    assert_eq!(agent.run_stats().completion_hook_blocks, 3);
    assert_eq!(agent.run_stats().completion_hook_limit_hits, 1);
    assert!(events.iter().any(|event| matches!(event, AgentEvent::CompletionNotice { reason_code, text } if reason_code == "completion.hook_limit" && text.contains('3'))));
    assert_eq!(
        agent.last_assistant_text().as_deref(),
        Some("The final answer.")
    );
}

#[test]
fn follow_up_prompt_resets_hook_active_and_block_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(dir.path());
    agent.queues.enqueue_follow_up("Now explain the tests");
    let flags = Arc::new(Mutex::new(Vec::new()));
    let captured = flags.clone();
    agent.completion_hook = Some(CompletionHook(Arc::new(move |active, _cancelled| {
        captured.lock().unwrap().push(active);
        (!active).then(|| "Check again".into())
    })));
    agent.run_loop(reply).unwrap();
    assert_eq!(*flags.lock().unwrap(), [false, true, false, true]);
    assert_eq!(agent.run_stats().completion_hook_blocks, 2);
}

#[test]
fn abort_after_provider_reply_skips_hook_and_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(dir.path());
    let signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
    agent.abort_signal = Some(signal.clone());
    agent.completion_hook = Some(CompletionHook(Arc::new(|_, _| panic!("hook after abort"))));
    let events = agent
        .run_loop(|agent| {
            signal.store(true, std::sync::atomic::Ordering::SeqCst);
            reply(agent)
        })
        .unwrap();
    assert_eq!(agent.run_stats().model_turns, 1);
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::CompletionReminder { .. })));
}

#[test]
fn abort_during_hook_does_not_queue_another_model_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(dir.path());
    let signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
    agent.abort_signal = Some(signal.clone());
    agent.completion_hook = Some(CompletionHook(Arc::new(move |_, _| {
        signal.store(true, std::sync::atomic::Ordering::SeqCst);
        Some("Must not continue".into())
    })));
    let events = agent.run_loop(reply).unwrap();
    assert_eq!(agent.run_stats().model_turns, 1);
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::CompletionReminder { .. })));
}

#[test]
fn completion_hook_sees_the_users_abort_while_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(dir.path());
    let signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
    agent.abort_signal = Some(signal.clone());
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = observed.clone();
    agent.completion_hook = Some(CompletionHook(Arc::new(move |_, cancelled| {
        assert!(!cancelled());
        // Esc while the hook's checks are still running.
        signal.store(true, std::sync::atomic::Ordering::SeqCst);
        seen.store(cancelled(), std::sync::atomic::Ordering::SeqCst);
        None
    })));
    agent.run_loop(reply).unwrap();
    assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(agent.run_stats().model_turns, 1);
}
