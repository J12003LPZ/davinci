// One-shot branch regression coverage for the reliability follow-up fixes.
use davinci_agent::{
    Agent, OutputPolicy, ReplayPolicy, ReservationOutcome, RuntimeCapabilityRegistry,
};
use davinci_session::JsonlSession;
use serde_json::json;

#[test]
fn runtime_output_policy_matches_governor_contract() {
    let registry = RuntimeCapabilityRegistry::with_builtins();

    assert_eq!(
        registry.get("read").unwrap().output_policy,
        OutputPolicy::LosslessRequired
    );
    assert_eq!(
        registry.get("batch").unwrap().output_policy,
        OutputPolicy::LosslessRequired
    );
    assert_eq!(
        registry.get("exec_command").unwrap().output_policy,
        OutputPolicy::Compressible
    );
}

#[test]
fn restart_restores_uncertain_mutation_and_blocks_replay() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let session_path = session.path.clone();
    let args = json!({"command": "deploy production"});

    let mut agent = Agent::new("fixture");
    agent.load_from_session(session).unwrap();
    agent.tool_ledger.lock().unwrap().record_start_with_policy(
        "call-shell",
        "exec_command",
        &args,
        ReplayPolicy::NeverAutoReplay,
    );

    assert!(
        session_path.with_extension("tool-ledger.json").is_file(),
        "started attempts must be durable before a possible crash"
    );
    drop(agent);

    let mut resumed = Agent::new("fixture");
    resumed
        .load_from_session(JsonlSession::open(&session_path).unwrap())
        .unwrap();
    let outcome = resumed.tool_ledger.lock().unwrap().reserve_call_with_policy(
        "call-shell",
        "exec_command",
        &args,
        ReplayPolicy::NeverAutoReplay,
    );

    assert!(matches!(outcome, Ok(ReservationOutcome::ReplayBlocked(_))));
}
