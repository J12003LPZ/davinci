//! Terminal completion-state regressions (G-07).
//!
//! These tests exercise the real mutation/verification accounting helpers, not
//! a parallel state machine. A model claim never appears in this file: only
//! observed mutations and observed verification can move the terminal status.

use super::*;

fn verify(agent: &Agent, command: &str, passed: bool) {
    // Production records the attempted call before execution and the outcome
    // afterwards; a timeout/unavailable command has the first but not the
    // second.
    agent.remember_verification_command("bash", command);
    agent.record_verification_command(command, passed);
}

fn noticed_statuses(events: &[AgentEvent]) -> Vec<CompletionStatus> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::VerificationNotice { status, .. } => Some(*status),
            _ => None,
        })
        .collect()
}

#[test]
fn edit_then_passing_verification_is_verified() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    verify(&agent, "cargo test", true);
    assert_eq!(agent.completion_status(), CompletionStatus::Verified);
    assert!(agent.completion_status().is_verified());
}

#[test]
fn edit_then_failing_verification_is_failed() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    verify(&agent, "cargo test", false);
    assert_eq!(
        agent.completion_status(),
        CompletionStatus::VerificationFailed
    );
}

#[test]
fn edit_without_verification_is_not_verified() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    assert_eq!(
        agent.completion_status(),
        CompletionStatus::VerificationUnavailable
    );
    assert!(!agent.completion_status().is_verified());
}

#[test]
fn verification_then_another_edit_is_stale() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    verify(&agent, "cargo test", true);
    assert_eq!(agent.completion_status(), CompletionStatus::Verified);

    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    assert_eq!(agent.completion_status(), CompletionStatus::Unverified);
}

#[test]
fn verification_of_an_earlier_generation_does_not_cover_a_later_edit() {
    use crate::verification::{Assessment, CheckKind};

    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    verify(&agent, "cargo test", true);
    let verified_generation = agent.mutation_verification_state().mutation_generation;
    assert_eq!(agent.completion_status(), CompletionStatus::Verified);

    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    // A late result from the earlier generation must be ignored.
    agent.record_verification_assessment(
        verified_generation,
        "cargo test",
        &Assessment {
            kind: CheckKind::Suite,
            covered: vec!["src/lib.rs".into()],
            complete: true,
            full_workspace: true,
            reason: "late result",
            ran_in: None,
        },
        Some(true),
    );
    assert_eq!(agent.completion_status(), CompletionStatus::Unverified);
}

#[test]
fn verification_timeout_is_not_verified() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    // The call started but produced no terminal receipt.
    agent.remember_verification_command("bash", "cargo test");
    assert_eq!(agent.completion_status(), CompletionStatus::Unverified);
    assert!(!agent.completion_status().is_verified());
}

#[test]
fn verification_command_unavailable_is_not_verified() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    agent.remember_verification_command("bash", "missing-test-runner");
    assert_eq!(agent.completion_status(), CompletionStatus::Unverified);
    assert!(!agent.completion_status().is_verified());
}

#[test]
fn multiple_independent_edits_require_fresh_verification() {
    let agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/a.rs".into()]);
    verify(&agent, "cargo test", true);
    assert_eq!(agent.completion_status(), CompletionStatus::Verified);

    agent.record_successful_mutation_paths(vec!["src/b.rs".into()]);
    assert_eq!(agent.completion_status(), CompletionStatus::Unverified);

    verify(&agent, "cargo test", true);
    assert_eq!(agent.completion_status(), CompletionStatus::Verified);
}

#[test]
fn session_interruption_before_verification_persists_unverified() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("test");
    agent.session = Some(crate::JsonlSession::create(dir.path(), "interrupted", None).unwrap());
    agent.prompt_user_with("Change the parser", &[]);
    agent.record_successful_mutation_paths(vec!["src/parser.rs".into()]);
    agent.abort();
    assert!(agent.abort_requested());

    let mut events = Vec::new();
    agent.emit_verification_notice(&mut events);
    assert_eq!(
        noticed_statuses(&events),
        vec![CompletionStatus::VerificationUnavailable]
    );
    agent.ensure_session_persistence().unwrap();

    let path = agent.session.as_ref().unwrap().path.clone();
    assert!(!agent.completion_status().is_verified());
    assert_eq!(
        agent.last_recorded_completion_status().unwrap().status,
        CompletionStatus::VerificationUnavailable
    );

    let mut resumed = Agent::new("test");
    resumed
        .load_from_session(crate::JsonlSession::open(&path).unwrap())
        .unwrap();
    assert!(!resumed.completion_status().is_verified());
    let record = resumed
        .last_recorded_completion_status()
        .expect("terminal status must survive a reload");
    assert_eq!(record.status, CompletionStatus::VerificationUnavailable);
    assert_eq!(record.mutation_generation, 1);
}

#[test]
fn resume_keeps_a_verified_mutation_verified() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("test");
    agent.session = Some(crate::JsonlSession::create(dir.path(), "verified", None).unwrap());
    agent.prompt_user_with("Fix the parser", &[]);
    agent.record_successful_mutation_paths(vec!["src/parser.rs".into()]);
    verify(&agent, "cargo test", true);
    let mut events = Vec::new();
    agent.emit_verification_notice(&mut events);
    // A verified turn needs no warning notice.
    assert!(noticed_statuses(&events).is_empty());
    agent.ensure_session_persistence().unwrap();

    let path = agent.session.as_ref().unwrap().path.clone();
    let mut resumed = Agent::new("test");
    resumed
        .load_from_session(crate::JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(resumed.completion_status(), CompletionStatus::Verified);
    assert_eq!(
        resumed.last_recorded_completion_status().unwrap().status,
        CompletionStatus::Verified
    );
}

#[test]
fn resume_with_unverified_changes_never_reports_verified() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("test");
    agent.session = Some(crate::JsonlSession::create(dir.path(), "unverified", None).unwrap());
    agent.prompt_user_with("Refactor the cache", &[]);
    agent.record_successful_mutation_paths(vec!["src/cache.rs".into()]);
    // Production persists mutation state at the turn boundary; a resumed
    // session must not lose an unresolved edit.
    agent.persist_mutation_verification();
    agent.ensure_session_persistence().unwrap();

    let path = agent.session.as_ref().unwrap().path.clone();
    let mut resumed = Agent::new("test");
    resumed
        .load_from_session(crate::JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(
        resumed.completion_status(),
        CompletionStatus::VerificationUnavailable
    );
    assert_eq!(resumed.mutation_verification_state().mutation_generation, 1);
    assert!(resumed.last_recorded_completion_status().is_none());
}

#[test]
fn compaction_keeps_pending_verification_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("test");
    agent.session = Some(crate::JsonlSession::create(dir.path(), "compaction", None).unwrap());
    agent.prompt_user_with(&"context line for compaction ".repeat(80), &[]);
    agent.record_assistant(&"answer line for compaction ".repeat(80));
    agent.record_successful_mutation_paths(vec!["src/cache.rs".into()]);
    let mut events = Vec::new();
    agent.emit_verification_notice(&mut events);
    assert_eq!(
        noticed_statuses(&events),
        vec![CompletionStatus::VerificationUnavailable]
    );

    let compacted = agent.compact(Some("keep the pending edit"));
    assert!(compacted.compacted);
    assert_eq!(
        agent.completion_status(),
        CompletionStatus::VerificationUnavailable
    );
    assert_eq!(
        agent.last_recorded_completion_status().unwrap().status,
        CompletionStatus::VerificationUnavailable
    );
}

#[test]
fn user_stop_without_verification_is_recorded_as_unverified() {
    let mut agent = Agent::new("test");
    agent.record_successful_mutation_paths(vec!["src/lib.rs".into()]);
    agent.abort();
    let mut events = Vec::new();
    agent.emit_verification_notice(&mut events);
    assert_eq!(
        noticed_statuses(&events),
        vec![CompletionStatus::VerificationUnavailable]
    );
    assert!(!agent.completion_status().is_verified());
}
