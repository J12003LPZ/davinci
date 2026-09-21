//! Graph execution lifecycle control, commands, and receipts.

use super::types::{GraphLifecycle, GraphRun};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub fn pause_state(
    requested: bool,
    active_workers: usize,
    checkpoint_durable: bool,
) -> &'static str {
    if !requested {
        "running"
    } else if active_workers > 0 {
        "pause_requested"
    } else if checkpoint_durable {
        "paused"
    } else {
        "recovery_required"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphControlAction {
    Pause,
    Resume,
    StopNode,
    StopGraph,
    RetryNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphControl {
    pub operation_id: String,
    pub run_id: String,
    pub expected_run_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_attempt: Option<u32>,
    pub action: GraphControlAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlReceiptState {
    Accepted,
    Applied,
    Rejected,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphControlReceipt {
    pub operation_id: String,
    pub accepted_revision: u64,
    pub state: ControlReceiptState,
    #[serde(default)]
    pub affected_nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphControlRecord {
    pub request: GraphControl,
    pub receipt: GraphControlReceipt,
}

#[derive(Debug, Default, Clone)]
pub struct ControlTracker {
    seen_ops: HashSet<String>,
    receipts: HashMap<String, GraphControlReceipt>,
}

#[allow(dead_code)]
impl ControlTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_receipt(&self, operation_id: &str) -> Option<&GraphControlReceipt> {
        self.receipts.get(operation_id)
    }

    pub fn record_receipt(&mut self, receipt: GraphControlReceipt) {
        self.seen_ops.insert(receipt.operation_id.clone());
        self.receipts.insert(receipt.operation_id.clone(), receipt);
    }

    pub fn is_seen(&self, operation_id: &str) -> bool {
        self.seen_ops.contains(operation_id)
    }
}

pub fn descendant_ids<'a>(
    start: &'a str,
    edges: &[(&'a str, &'a str)],
) -> std::collections::BTreeSet<&'a str> {
    let mut out = std::collections::BTreeSet::from([start]);
    loop {
        let before = out.len();
        for (from, to) in edges {
            if out.contains(from) {
                out.insert(*to);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

pub fn invalidate_descendants_for_retry(
    run: &mut GraphRun,
    retry_node_id: &str,
) -> Result<Vec<String>, String> {
    let task_idx = run
        .tasks
        .iter()
        .position(|t| t.id == retry_node_id)
        .ok_or_else(|| format!("node '{retry_node_id}' not found in graph"))?;

    let task = &run.tasks[task_idx];
    if task.status != super::types::TaskStatus::Failed
        && task.status != super::types::TaskStatus::Cancelled
    {
        return Err(format!(
            "node '{retry_node_id}' is in status '{:?}', only failed or cancelled nodes can be retried",
            task.status
        ));
    }

    // Check for ambiguous writer outcome
    if task.role == super::types::Role::Writer {
        if let Some(ref err) = task.error {
            if err.contains("unreconciled") || err.contains("ambiguous") {
                return Err(format!(
                    "cannot retry writer '{retry_node_id}' with unreconciled partial outcome"
                ));
            }
        }
    }

    // Check retry budget (attempts)
    const MAX_NODE_ATTEMPTS: u32 = 3;
    if task.attempts >= MAX_NODE_ATTEMPTS {
        return Err(format!(
            "node '{retry_node_id}' exceeded max attempts ({MAX_NODE_ATTEMPTS})"
        ));
    }

    // Verify dependencies: cannot retry if any dependency is failed or cancelled
    for dep in &task.depends_on {
        if let Some(dep_task) = run.tasks.iter().find(|t| t.id == *dep) {
            if dep_task.status != super::types::TaskStatus::Succeeded {
                return Err(format!(
                    "cannot retry '{retry_node_id}': dependency '{dep}' is in status '{:?}'",
                    dep_task.status
                ));
            }
        }
    }

    let mut edge_pairs: Vec<(String, String)> = Vec::new();
    for t in &run.tasks {
        for dep in &t.depends_on {
            edge_pairs.push((dep.clone(), t.id.clone()));
        }
    }
    if let Some(ref def) = run.definition {
        for edge in &def.edges {
            edge_pairs.push((edge.from.clone(), edge.to.clone()));
        }
    }
    let ref_edges: Vec<(&str, &str)> = edge_pairs
        .iter()
        .map(|(f, t)| (f.as_str(), t.as_str()))
        .collect();

    let affected_set = descendant_ids(retry_node_id, &ref_edges);
    let affected_ids: Vec<String> = affected_set.into_iter().map(String::from).collect();

    for task in &mut run.tasks {
        if affected_ids.contains(&task.id) {
            if task.id == retry_node_id {
                task.attempts += 1;
                task.status = super::types::TaskStatus::Pending;
                task.error = None;
                task.artifact_file = None;
                task.started_at = None;
                task.ended_at = None;
                task.last_activity = None;
                task.fingerprint = None;
            } else {
                task.status = super::types::TaskStatus::Pending;
                task.artifact_file = None;
                task.fingerprint = None;
                task.error = None;
            }
        }
    }

    let has_mutation_descendant = run.tasks.iter().any(|t| {
        affected_ids.contains(&t.id)
            && (t.role == super::types::Role::Writer || t.role == super::types::Role::Planner)
    });
    if has_mutation_descendant || affected_ids.contains(&"classify".to_string()) {
        run.verification = None;
        run.verification_bundle = None;
        run.review_coverage = None;
    }

    run.revision += 1;
    Ok(affected_ids)
}

pub fn reduce_control(
    run: &mut GraphRun,
    control: &GraphControl,
    tracker: &mut ControlTracker,
    active_workers: usize,
    checkpoint_durable: bool,
) -> GraphControlReceipt {
    let refusal = |state, reason: &str| GraphControlReceipt {
        operation_id: control.operation_id.clone(),
        accepted_revision: run.revision,
        state,
        affected_nodes: vec![],
        reason: Some(reason.into()),
    };
    if let Some(existing) = run
        .control_history
        .iter()
        .find(|entry| entry.request.operation_id == control.operation_id)
    {
        return if existing.request == *control {
            existing.receipt.clone()
        } else {
            refusal(
                ControlReceiptState::Conflict,
                "Operation ID was already used for a different request",
            )
        };
    }
    let invalid = if control.operation_id.trim().is_empty() {
        Some(refusal(
            ControlReceiptState::Rejected,
            "Missing operation ID",
        ))
    } else if run.revision == u64::MAX {
        Some(refusal(
            ControlReceiptState::Rejected,
            "Graph revision is exhausted",
        ))
    } else if control.action == GraphControlAction::Pause
        && !matches!(
            run.current_lifecycle(),
            GraphLifecycle::Running | GraphLifecycle::PauseRequested | GraphLifecycle::Paused
        )
    {
        Some(refusal(
            ControlReceiptState::Rejected,
            "Only a running or paused graph can be paused",
        ))
    } else if matches!(
        control.action,
        GraphControlAction::StopNode | GraphControlAction::RetryNode
    ) {
        match control
            .node_id
            .as_ref()
            .and_then(|id| run.tasks.iter().find(|task| task.id == *id))
        {
            None => Some(refusal(
                ControlReceiptState::Rejected,
                "Node ID is missing or does not exist in this run",
            )),
            Some(task) if control.expected_attempt != Some(task.attempts) => Some(refusal(
                ControlReceiptState::Conflict,
                "Node attempt is missing or does not match the current attempt",
            )),
            Some(task)
                if control.action == GraphControlAction::StopNode
                    && task.status != super::types::TaskStatus::Running =>
            {
                Some(refusal(
                    ControlReceiptState::Rejected,
                    "Only a running node attempt can be stopped",
                ))
            }
            _ => None,
        }
    } else {
        None
    };
    let receipt = invalid.unwrap_or_else(|| {
        reduce_unrecorded_control(run, control, tracker, active_workers, checkpoint_durable)
    });
    run.control_history.push(GraphControlRecord {
        request: control.clone(),
        receipt: receipt.clone(),
    });
    receipt
}

fn reduce_unrecorded_control(
    run: &mut GraphRun,
    control: &GraphControl,
    tracker: &mut ControlTracker,
    active_workers: usize,
    checkpoint_durable: bool,
) -> GraphControlReceipt {
    if let Some(existing) = tracker.get_receipt(&control.operation_id) {
        return existing.clone();
    }

    if control.run_id != run.run_id {
        let receipt = GraphControlReceipt {
            operation_id: control.operation_id.clone(),
            accepted_revision: run.revision,
            state: ControlReceiptState::Conflict,
            affected_nodes: vec![],
            reason: Some(format!(
                "Run ID mismatch: expected '{}', got '{}'",
                run.run_id, control.run_id
            )),
        };
        tracker.record_receipt(receipt.clone());
        return receipt;
    }

    if control.expected_run_revision != run.revision {
        let receipt = GraphControlReceipt {
            operation_id: control.operation_id.clone(),
            accepted_revision: run.revision,
            state: ControlReceiptState::Conflict,
            affected_nodes: vec![],
            reason: Some(format!(
                "Revision conflict: run is at revision {}, control expected {}",
                run.revision, control.expected_run_revision
            )),
        };
        tracker.record_receipt(receipt.clone());
        return receipt;
    }

    let receipt = match control.action {
        GraphControlAction::Pause => {
            let state_str = pause_state(true, active_workers, checkpoint_durable);
            match state_str {
                "paused" => {
                    run.lifecycle = Some(GraphLifecycle::Paused);
                    run.revision += 1;
                    GraphControlReceipt {
                        operation_id: control.operation_id.clone(),
                        accepted_revision: run.revision,
                        state: ControlReceiptState::Applied,
                        affected_nodes: vec![],
                        reason: None,
                    }
                }
                "pause_requested" => {
                    run.lifecycle = Some(GraphLifecycle::PauseRequested);
                    run.revision += 1;
                    GraphControlReceipt {
                        operation_id: control.operation_id.clone(),
                        accepted_revision: run.revision,
                        state: ControlReceiptState::Accepted,
                        affected_nodes: vec![],
                        reason: Some(
                            "Pause requested; waiting for active workers to reach safe boundary"
                                .into(),
                        ),
                    }
                }
                "recovery_required" => {
                    run.lifecycle = Some(GraphLifecycle::RecoveryRequired);
                    run.revision += 1;
                    GraphControlReceipt {
                        operation_id: control.operation_id.clone(),
                        accepted_revision: run.revision,
                        state: ControlReceiptState::Rejected,
                        affected_nodes: vec![],
                        reason: Some("Checkpoint durability failure; recovery required".into()),
                    }
                }
                _ => GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Rejected,
                    affected_nodes: vec![],
                    reason: Some("Unexpected pause state".into()),
                },
            }
        }
        GraphControlAction::Resume => {
            let current = run.current_lifecycle();
            if current == GraphLifecycle::Paused || current == GraphLifecycle::PauseRequested {
                run.lifecycle = Some(GraphLifecycle::Running);
                run.revision += 1;
                GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Applied,
                    affected_nodes: vec![],
                    reason: None,
                }
            } else if current == GraphLifecycle::Running {
                GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Applied,
                    affected_nodes: vec![],
                    reason: Some("Already running".into()),
                }
            } else {
                GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Rejected,
                    affected_nodes: vec![],
                    reason: Some(format!(
                        "Cannot resume run in lifecycle '{}'",
                        current.as_str()
                    )),
                }
            }
        }
        GraphControlAction::StopNode => {
            if let Some(ref node_id) = control.node_id {
                let affected = if run.tasks.iter().any(|t| t.id == *node_id) {
                    vec![node_id.clone()]
                } else {
                    vec![]
                };
                run.revision += 1;
                GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Accepted,
                    affected_nodes: affected,
                    reason: None,
                }
            } else {
                GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Rejected,
                    affected_nodes: vec![],
                    reason: Some("Missing node_id for StopNode".into()),
                }
            }
        }
        GraphControlAction::StopGraph => {
            run.lifecycle = Some(if active_workers > 0 {
                GraphLifecycle::StopRequested
            } else {
                GraphLifecycle::Stopped
            });
            run.revision += 1;
            GraphControlReceipt {
                operation_id: control.operation_id.clone(),
                accepted_revision: run.revision,
                state: if active_workers > 0 {
                    ControlReceiptState::Accepted
                } else {
                    ControlReceiptState::Applied
                },
                affected_nodes: vec![],
                reason: None,
            }
        }
        GraphControlAction::RetryNode => {
            if let Some(ref node_id) = control.node_id {
                match invalidate_descendants_for_retry(run, node_id) {
                    Ok(affected) => GraphControlReceipt {
                        operation_id: control.operation_id.clone(),
                        accepted_revision: run.revision,
                        state: ControlReceiptState::Applied,
                        affected_nodes: affected,
                        reason: None,
                    },
                    Err(err) => GraphControlReceipt {
                        operation_id: control.operation_id.clone(),
                        accepted_revision: run.revision,
                        state: ControlReceiptState::Rejected,
                        affected_nodes: vec![],
                        reason: Some(err),
                    },
                }
            } else {
                GraphControlReceipt {
                    operation_id: control.operation_id.clone(),
                    accepted_revision: run.revision,
                    state: ControlReceiptState::Rejected,
                    affected_nodes: vec![],
                    reason: Some("Missing node_id for RetryNode".into()),
                }
            }
        }
    };

    tracker.record_receipt(receipt.clone());
    receipt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::*;

    fn sample_test_run() -> GraphRun {
        GraphRun {
            version: 1,
            run_id: "run-ctrl-1".into(),
            goal: "control test".into(),
            cwd: ".".into(),
            phase: Phase::Investigate,
            forced: None,
            dry_run: false,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
            definition: None,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: vec![GraphTaskState::new(
                "task-a",
                Role::Researcher,
                ArtifactKind::Evidence,
                vec![],
                None,
            )],
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 1,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: Some(GraphLifecycle::Running),
            revision: 0,
            control_history: Vec::new(),
        }
    }

    #[test]
    fn f13_pause_ack_boundary() {
        assert_eq!(pause_state(true, 2, true), "pause_requested");
        assert_eq!(pause_state(true, 0, false), "recovery_required");
        assert_eq!(pause_state(true, 0, true), "paused");
    }

    fn pause_request() -> GraphControl {
        GraphControl {
            operation_id: "durable-control".into(),
            run_id: "run-ctrl-1".into(),
            expected_run_revision: 0,
            node_id: None,
            expected_attempt: None,
            action: GraphControlAction::Pause,
        }
    }

    #[test]
    fn control_receipt_survives_checkpoint_round_trip() {
        let mut run = sample_test_run();
        let request = pause_request();
        let first = reduce_control(&mut run, &request, &mut ControlTracker::new(), 0, true);
        let mut reopened: GraphRun =
            serde_json::from_slice(&serde_json::to_vec(&run).unwrap()).unwrap();
        let repeated = reduce_control(&mut reopened, &request, &mut ControlTracker::new(), 0, true);
        assert_eq!(repeated, first);
        assert_eq!(reopened.revision, 1);
    }

    #[test]
    fn control_operation_identity_cannot_be_reused_for_different_arguments() {
        let mut run = sample_test_run();
        let mut tracker = ControlTracker::new();
        let request = pause_request();
        reduce_control(&mut run, &request, &mut tracker, 0, true);
        let collision = GraphControl {
            action: GraphControlAction::StopGraph,
            ..request
        };
        let result = reduce_control(&mut run, &collision, &mut tracker, 0, true);
        assert_eq!(result.state, ControlReceiptState::Conflict);
        assert_eq!(run.current_lifecycle(), GraphLifecycle::Paused);
        assert_eq!(run.revision, 1);
    }

    #[test]
    fn control_rejects_stale_attempt_or_unknown_node_without_changing_revision() {
        for (id, attempt) in [("task-a", 9), ("missing", 0)] {
            let mut run = sample_test_run();
            let request = GraphControl {
                action: GraphControlAction::StopNode,
                node_id: Some(id.into()),
                expected_attempt: Some(attempt),
                ..pause_request()
            };
            let result = reduce_control(&mut run, &request, &mut ControlTracker::new(), 0, true);
            assert!(matches!(
                result.state,
                ControlReceiptState::Conflict | ControlReceiptState::Rejected
            ));
            assert_eq!(run.revision, 0);
        }
    }

    #[test]
    fn controls_reject_inapplicable_lifecycle_and_node_states() {
        for phase in [Phase::Done, Phase::Blocked, Phase::Cancelled] {
            let mut run = sample_test_run();
            run.phase = phase;
            let receipt = reduce_control(
                &mut run,
                &pause_request(),
                &mut ControlTracker::new(),
                0,
                true,
            );
            assert_eq!(receipt.state, ControlReceiptState::Rejected);
            assert_eq!(run.revision, 0);
        }
        for status in [
            TaskStatus::Pending,
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let mut run = sample_test_run();
            run.tasks[0].status = status;
            let request = GraphControl {
                action: GraphControlAction::StopNode,
                node_id: Some("task-a".into()),
                expected_attempt: Some(0),
                ..pause_request()
            };
            let receipt = reduce_control(&mut run, &request, &mut ControlTracker::new(), 0, true);
            assert_eq!(receipt.state, ControlReceiptState::Rejected);
            assert_eq!(run.revision, 0);
        }
    }

    #[test]
    fn test_pause_while_running() {
        assert_eq!(pause_state(false, 0, true), "running");
        assert_eq!(pause_state(true, 1, true), "pause_requested");
    }

    #[test]
    fn test_duplicate_pause() {
        let mut run = sample_test_run();
        let mut tracker = ControlTracker::new();

        let pause_ctrl = GraphControl {
            operation_id: "op-pause-1".into(),
            run_id: "run-ctrl-1".into(),
            expected_run_revision: 0,
            node_id: None,
            expected_attempt: None,
            action: GraphControlAction::Pause,
        };

        let receipt1 = reduce_control(&mut run, &pause_ctrl, &mut tracker, 0, true);
        assert_eq!(receipt1.state, ControlReceiptState::Applied);
        assert_eq!(receipt1.accepted_revision, 1);
        assert_eq!(run.revision, 1);
        assert_eq!(run.lifecycle, Some(GraphLifecycle::Paused));

        // Duplicate operation ID returns identical cached receipt without bumping revision
        let receipt2 = reduce_control(&mut run, &pause_ctrl, &mut tracker, 0, true);
        assert_eq!(receipt2.state, ControlReceiptState::Applied);
        assert_eq!(receipt2.accepted_revision, 1);
        assert_eq!(run.revision, 1);
    }

    #[test]
    fn test_stale_run_id() {
        let mut run = sample_test_run();
        let mut tracker = ControlTracker::new();

        let stale_ctrl = GraphControl {
            operation_id: "op-stale-1".into(),
            run_id: "wrong-run-id".into(),
            expected_run_revision: 0,
            node_id: None,
            expected_attempt: None,
            action: GraphControlAction::Pause,
        };

        let receipt = reduce_control(&mut run, &stale_ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Conflict);
        assert!(receipt.reason.unwrap().contains("Run ID mismatch"));
        assert_eq!(run.revision, 0);
    }

    #[test]
    fn test_persisted_paused_resume() {
        let mut run = sample_test_run();
        run.lifecycle = Some(GraphLifecycle::Paused);
        run.revision = 3;

        // Serialize and deserialize
        let json_str = serde_json::to_string(&run).unwrap();
        let mut loaded: GraphRun = serde_json::from_str(&json_str).unwrap();
        assert_eq!(loaded.current_lifecycle(), GraphLifecycle::Paused);
        assert_eq!(loaded.revision, 3);

        let mut tracker = ControlTracker::new();
        let resume_ctrl = GraphControl {
            operation_id: "op-resume-1".into(),
            run_id: "run-ctrl-1".into(),
            expected_run_revision: 3,
            node_id: None,
            expected_attempt: None,
            action: GraphControlAction::Resume,
        };

        let receipt = reduce_control(&mut loaded, &resume_ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Applied);
        assert_eq!(loaded.current_lifecycle(), GraphLifecycle::Running);
        assert_eq!(loaded.revision, 4);
    }

    #[test]
    fn test_old_state_v1_defaults() {
        let mut sample = sample_test_run();
        sample.lifecycle = None;
        sample.revision = 0;
        let mut val = serde_json::to_value(&sample).unwrap();
        if let Some(obj) = val.as_object_mut() {
            obj.remove("lifecycle");
            obj.remove("revision");
        }
        let legacy_json = serde_json::to_string(&val).unwrap();
        assert!(!legacy_json.contains("\"lifecycle\""));
        assert!(!legacy_json.contains("\"revision\":"));

        let run: GraphRun = serde_json::from_str(&legacy_json).unwrap();
        assert_eq!(run.lifecycle, None);
        assert_eq!(run.revision, 0);
        assert_eq!(run.current_lifecycle(), GraphLifecycle::Running);
    }

    #[test]
    fn test_bare_graph_does_not_unpause() {
        let mut run = sample_test_run();
        run.lifecycle = Some(GraphLifecycle::Paused);
        assert_eq!(run.current_lifecycle(), GraphLifecycle::Paused);
        let auto_resume_allowed =
            run.phase != Phase::Done && run.current_lifecycle() != GraphLifecycle::Paused;
        assert!(!auto_resume_allowed);
    }

    #[test]
    fn test_disk_failure() {
        assert_eq!(pause_state(true, 0, false), "recovery_required");
    }

    #[test]
    fn test_unknown_lifecycle_fail_closed() {
        assert_eq!(GraphLifecycle::parse("bogus"), None);
        assert_eq!(
            GraphLifecycle::parse("running"),
            Some(GraphLifecycle::Running)
        );
        assert_eq!(
            GraphLifecycle::parse("paused"),
            Some(GraphLifecycle::Paused)
        );
        assert!(GraphLifecycle::Running.is_running());
        assert!(!GraphLifecycle::Paused.is_running());
        assert!(GraphLifecycle::Paused.is_paused());
        assert!(GraphLifecycle::Stopped.is_terminal());
        assert!(GraphLifecycle::RecoveryRequired.is_terminal());
    }

    #[test]
    fn f13_invalidate_descendants() {
        let edges = vec![("a", "b"), ("b", "c"), ("x", "y")];
        let affected = descendant_ids("a", &edges);
        assert_eq!(affected, std::collections::BTreeSet::from(["a", "b", "c"]));
    }

    #[test]
    fn test_retry_failed_research_preserves_independent_success() {
        let mut run = sample_test_run();
        // research-1 succeeded
        let mut r1 = GraphTaskState::new(
            "research-1",
            Role::Researcher,
            ArtifactKind::Evidence,
            vec!["classify".into()],
            None,
        );
        r1.status = TaskStatus::Succeeded;
        r1.artifact_file = Some("artifacts/research-1.json".into());
        r1.usage.input = 500;

        // research-2 failed
        let mut r2 = GraphTaskState::new(
            "research-2",
            Role::Researcher,
            ArtifactKind::Evidence,
            vec!["classify".into()],
            None,
        );
        r2.status = TaskStatus::Failed;
        r2.error = Some("timeout".into());
        r2.usage.input = 300;

        // plan-1 depends on research-1 and research-2
        let mut plan = GraphTaskState::new(
            "plan-1",
            Role::Planner,
            ArtifactKind::Plan,
            vec!["research-1".into(), "research-2".into()],
            None,
        );
        plan.status = TaskStatus::Pending;

        run.tasks = vec![r1, r2, plan];
        let mut tracker = ControlTracker::new();

        let ctrl = GraphControl {
            operation_id: "op-retry-r2".into(),
            run_id: run.run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("research-2".into()),
            expected_attempt: Some(
                run.tasks
                    .iter()
                    .find(|task| task.id == "research-2")
                    .unwrap()
                    .attempts,
            ),
            action: GraphControlAction::RetryNode,
        };

        let receipt = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Applied);
        assert_eq!(receipt.affected_nodes, vec!["plan-1", "research-2"]);

        // research-1 preserved intact
        let r1_after = run.tasks.iter().find(|t| t.id == "research-1").unwrap();
        assert_eq!(r1_after.status, TaskStatus::Succeeded);
        assert_eq!(r1_after.usage.input, 500);

        // research-2 invalidated and attempts incremented
        let r2_after = run.tasks.iter().find(|t| t.id == "research-2").unwrap();
        assert_eq!(r2_after.status, TaskStatus::Pending);
        assert_eq!(r2_after.attempts, 1);
        assert_eq!(r2_after.usage.input, 300); // spent usage retained!
    }

    #[test]
    fn test_writer_unknown_outcome_refuses() {
        let mut run = sample_test_run();
        let mut writer = GraphTaskState::new(
            "implement-1",
            Role::Writer,
            ArtifactKind::PatchReport,
            vec![],
            None,
        );
        writer.status = TaskStatus::Failed;
        writer.error = Some("ambiguous mutation on disk".into());
        run.tasks = vec![writer];
        let mut tracker = ControlTracker::new();

        let ctrl = GraphControl {
            operation_id: "op-retry-w".into(),
            run_id: run.run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("implement-1".into()),
            expected_attempt: Some(
                run.tasks
                    .iter()
                    .find(|task| task.id == "implement-1")
                    .unwrap()
                    .attempts,
            ),
            action: GraphControlAction::RetryNode,
        };

        let receipt = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Rejected);
        assert!(receipt.reason.unwrap().contains("unreconciled"));
    }

    #[test]
    fn test_stale_review_security_invalidated() {
        let mut run = sample_test_run();
        let mut writer = GraphTaskState::new(
            "implement-1",
            Role::Writer,
            ArtifactKind::PatchReport,
            vec![],
            None,
        );
        writer.status = TaskStatus::Failed;
        run.tasks = vec![writer];
        run.verification = Some(VerificationResult {
            progress: None,
            passed: true,
            commands: vec![],
        });
        run.review_coverage = Some(super::super::review_coverage::ReviewCoverage::new(vec![
            "chunk-1".into(),
        ]));

        let mut tracker = ControlTracker::new();
        let ctrl = GraphControl {
            operation_id: "op-retry-impl".into(),
            run_id: run.run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("implement-1".into()),
            expected_attempt: Some(
                run.tasks
                    .iter()
                    .find(|task| task.id == "implement-1")
                    .unwrap()
                    .attempts,
            ),
            action: GraphControlAction::RetryNode,
        };

        let receipt = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Applied);
        assert!(run.verification.is_none());
        assert!(run.review_coverage.is_none());
    }

    #[test]
    fn test_cancelled_ancestor() {
        let mut run = sample_test_run();
        let mut parent =
            GraphTaskState::new("plan-1", Role::Planner, ArtifactKind::Plan, vec![], None);
        parent.status = TaskStatus::Cancelled;

        let mut child = GraphTaskState::new(
            "implement-1",
            Role::Writer,
            ArtifactKind::PatchReport,
            vec!["plan-1".into()],
            None,
        );
        child.status = TaskStatus::Cancelled;

        run.tasks = vec![parent, child];
        let mut tracker = ControlTracker::new();

        let ctrl = GraphControl {
            operation_id: "op-retry-child".into(),
            run_id: run.run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("implement-1".into()),
            expected_attempt: Some(
                run.tasks
                    .iter()
                    .find(|task| task.id == "implement-1")
                    .unwrap()
                    .attempts,
            ),
            action: GraphControlAction::RetryNode,
        };

        let receipt = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Rejected);
        assert!(receipt
            .reason
            .unwrap()
            .contains("dependency 'plan-1' is in status"));
    }

    #[test]
    fn test_max_retries() {
        let mut run = sample_test_run();
        let mut task = GraphTaskState::new(
            "research-1",
            Role::Researcher,
            ArtifactKind::Evidence,
            vec![],
            None,
        );
        task.status = TaskStatus::Failed;
        task.attempts = 3; // reached max attempts
        run.tasks = vec![task];
        let mut tracker = ControlTracker::new();

        let ctrl = GraphControl {
            operation_id: "op-max-retry".into(),
            run_id: run.run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("research-1".into()),
            expected_attempt: Some(
                run.tasks
                    .iter()
                    .find(|task| task.id == "research-1")
                    .unwrap()
                    .attempts,
            ),
            action: GraphControlAction::RetryNode,
        };

        let receipt = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt.state, ControlReceiptState::Rejected);
        assert!(receipt.reason.unwrap().contains("exceeded max attempts"));
    }

    #[test]
    fn test_repeated_control_id_creates_one_attempt() {
        let mut run = sample_test_run();
        let mut task = GraphTaskState::new(
            "research-1",
            Role::Researcher,
            ArtifactKind::Evidence,
            vec![],
            None,
        );
        task.status = TaskStatus::Failed;
        run.tasks = vec![task];
        let mut tracker = ControlTracker::new();

        let ctrl = GraphControl {
            operation_id: "op-idempotent-retry".into(),
            run_id: run.run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("research-1".into()),
            expected_attempt: Some(
                run.tasks
                    .iter()
                    .find(|task| task.id == "research-1")
                    .unwrap()
                    .attempts,
            ),
            action: GraphControlAction::RetryNode,
        };

        let receipt1 = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt1.state, ControlReceiptState::Applied);
        assert_eq!(run.tasks[0].attempts, 1);
        assert_eq!(run.revision, 1);

        // Second call with same operation_id returns cached receipt without bumping revision or attempts
        let receipt2 = reduce_control(&mut run, &ctrl, &mut tracker, 0, true);
        assert_eq!(receipt2.state, ControlReceiptState::Applied);
        assert_eq!(run.tasks[0].attempts, 1);
        assert_eq!(run.revision, 1);
    }
}
