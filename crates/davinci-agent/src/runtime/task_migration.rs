//! Validate legacy observer history before importing authoritative task state.
//! Native migration contract; vendor/davinci has no equivalent task journal.

use std::collections::{HashMap, HashSet, VecDeque};

use super::tasks::{initial_task_state, BlockReason, TaskLineage};
use super::{
    RuntimeEvent, RuntimeEventEnvelope, TaskError, TaskId, TaskRecord, TaskRegistry, TaskState,
};

fn invalid(message: &str) -> TaskError {
    TaskError::Persistence(format!("legacy task migration: {message}"))
}

/// Host-supplied metadata required to recover a legacy `TaskCreated` event
/// that did not carry a `TaskRecord`.
///
/// Identity, ownership, result, evidence, revision and timestamps are all
/// derived from the observer event or subsequent events. Keeping those fields
/// out of this type prevents a recovery mapping from manufacturing authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTaskRecovery {
    pub title: String,
    pub description: Option<String>,
    pub dependencies: Vec<TaskId>,
    pub parent_plan_step: Option<super::tasks::PlanStepRef>,
    pub decision_prerequisites: Vec<super::tasks::DecisionPrerequisiteRef>,
}

impl LegacyTaskRecovery {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            dependencies: Vec::new(),
            parent_plan_step: None,
            decision_prerequisites: Vec::new(),
        }
    }
}

impl TaskRegistry {
    /// Build an initial projection from the host-selected session's observer log.
    pub fn legacy_snapshot(
        events: &[RuntimeEventEnvelope],
        session_id: &str,
    ) -> Result<Vec<TaskRecord>, TaskError> {
        Self::legacy_snapshot_with_recovery(events, session_id, &HashMap::new())
    }

    /// Build an initial projection using explicit host metadata for record-less
    /// task creation events. Missing mappings and unused mappings fail closed;
    /// no owner, result, evidence or consent is inferred by this method.
    pub fn legacy_snapshot_with_recovery(
        events: &[RuntimeEventEnvelope],
        session_id: &str,
        recovery: &HashMap<TaskId, LegacyTaskRecovery>,
    ) -> Result<Vec<TaskRecord>, TaskError> {
        let mut seen = HashMap::new();
        let mut creations = HashMap::new();
        let mut tasks = HashMap::new();
        let mut used_recovery = HashSet::new();
        for event in events {
            if !matches!(
                event.payload,
                RuntimeEvent::TaskCreated { .. }
                    | RuntimeEvent::TaskAssigned { .. }
                    | RuntimeEvent::TaskCompleted { .. }
            ) {
                continue;
            }
            // TaskRegistry historically emitted no session ID and maintained its
            // own sequence. The host-selected log and file order define the source.
            if event.schema_version != 1
                || event
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id != session_id)
            {
                return Err(invalid("unsupported schema or foreign session"));
            }
            if let Some(previous) = seen.insert(event.event_id, event) {
                if previous != event {
                    return Err(invalid("divergent duplicate event"));
                }
                continue;
            }
            apply_event(
                event,
                &mut tasks,
                &mut creations,
                recovery,
                &mut used_recovery,
            )?;
        }
        if used_recovery.len() != recovery.len() {
            return Err(invalid("recovery mapping contains an unknown task"));
        }
        reconcile_dependencies(&mut tasks)?;
        let mut snapshot: Vec<_> = tasks.into_values().collect();
        snapshot.sort_by_key(|task| task.id);
        Ok(snapshot)
    }
}

fn apply_event(
    event: &RuntimeEventEnvelope,
    tasks: &mut HashMap<TaskId, TaskRecord>,
    creations: &mut HashMap<TaskId, TaskRecord>,
    recovery: &HashMap<TaskId, LegacyTaskRecovery>,
    used_recovery: &mut HashSet<TaskId>,
) -> Result<(), TaskError> {
    let task_id = match &event.payload {
        RuntimeEvent::TaskCreated { task_id, record } => {
            let recovered = if let Some(record) = record {
                record.validate()?;
                if record.id != *task_id || record.run_id != event.run_id {
                    return Err(invalid("creation identity mismatch"));
                }
                record.clone()
            } else {
                let metadata = recovery.get(task_id).ok_or_else(|| {
                    invalid("task creation metadata is unavailable; recovery required")
                })?;
                if !used_recovery.insert(*task_id) {
                    return Err(invalid("duplicate record-less creation"));
                }
                let mut recovered = TaskRecord::new(event.run_id, metadata.title.clone());
                recovered.id = *task_id;
                recovered.description = metadata.description.clone();
                recovered.dependencies = metadata.dependencies.clone();
                recovered.parent_plan_step = metadata.parent_plan_step.clone();
                recovered.decision_prerequisites = metadata.decision_prerequisites.clone();
                recovered.created_at_ms = event.timestamp_ms;
                recovered.updated_at_ms = event.timestamp_ms;
                recovered.revision = 0;
                recovered.owner_generation = 0;
                recovered.assigned_to = None;
                recovered.result = None;
                recovered.evidence_refs.clear();
                recovered.blocked_reasons = if recovered.decision_prerequisites.is_empty() {
                    Vec::new()
                } else {
                    vec![BlockReason {
                        code: "decision_prerequisites_unverified".into(),
                        message: "Decision prerequisites require host evaluation before execution"
                            .into(),
                    }]
                };
                recovered.attempt = 0;
                recovered.contract_digest = None;
                recovered.validate()?;
                recovered
            };
            if let Some(previous) = creations.get(task_id) {
                if previous != &recovered {
                    return Err(invalid("divergent duplicate creation"));
                }
                return Ok(());
            }
            creations.insert(*task_id, recovered.clone());
            tasks.insert(*task_id, recovered);
            return Ok(());
        }
        RuntimeEvent::TaskAssigned { task_id, .. }
        | RuntimeEvent::TaskCompleted { task_id, .. } => *task_id,
        _ => return Ok(()),
    };
    if matches!(
        event.payload,
        RuntimeEvent::TaskCompleted { success: true, .. }
    ) {
        let task = tasks
            .get(&task_id)
            .ok_or_else(|| invalid("event has no preceding creation"))?;
        let lineage = TaskLineage {
            primary: event.run_id,
            origins: tasks.values().map(|task| task.run_id).collect(),
        };
        if task.state == TaskState::Blocked
            || initial_task_state(task, tasks, Some(&lineage))? != TaskState::Ready
        {
            return Err(invalid("completion precedes dependency readiness"));
        }
    }
    let task = tasks
        .get_mut(&task_id)
        .ok_or_else(|| invalid("event has no preceding creation"))?;
    if task.run_id != event.run_id || task.state.is_terminal() {
        return Err(invalid(
            "event has foreign origin or changes terminal history",
        ));
    }
    match &event.payload {
        RuntimeEvent::TaskAssigned { agent_id, .. } => {
            task.assigned_to = Some(*agent_id);
            if task.state == TaskState::Ready {
                task.state = TaskState::Running;
            }
        }
        RuntimeEvent::TaskCompleted { success, .. } => {
            task.state = if *success {
                TaskState::Completed
            } else {
                TaskState::Failed
            };
        }
        _ => unreachable!(),
    }
    // Legacy events have no revision/generation or result payload. Preserve only
    // recorded metadata; never manufacture execution receipts or owner consent.
    task.updated_at_ms = event.timestamp_ms;
    Ok(())
}

fn reconcile_dependencies(tasks: &mut HashMap<TaskId, TaskRecord>) -> Result<(), TaskError> {
    let Some(first) = tasks.values().next() else {
        return Ok(());
    };
    let lineage = TaskLineage {
        primary: first.run_id,
        origins: tasks.values().map(|task| task.run_id).collect(),
    };
    let mut dependents: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
    for task in tasks.values() {
        initial_task_state(task, tasks, Some(&lineage))?;
        for dependency in &task.dependencies {
            dependents.entry(*dependency).or_default().push(task.id);
        }
    }
    let mut remaining: HashMap<_, _> = tasks
        .values()
        .map(|task| (task.id, task.dependencies.len()))
        .collect();
    let mut queue: VecDeque<_> = remaining
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    while let Some(id) = queue.pop_front() {
        let task = &tasks[&id];
        if matches!(task.state, TaskState::Ready | TaskState::Pending) {
            let state = initial_task_state(task, tasks, Some(&lineage))?;
            tasks.get_mut(&id).expect("queued task exists").state = state;
        }
        // The validated DAG admits each node exactly once, after its dependencies.
        for dependent in dependents.get(&id).into_iter().flatten() {
            let count = remaining.get_mut(dependent).expect("dependent exists");
            *count -= 1;
            if *count == 0 {
                queue.push_back(*dependent);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    #[test]
    fn f03_legacy_snapshot_rejects_completion_before_dependencies() {
        let first = TaskRecord::new(RunId::new(), "unfinished");
        let mut dependent = TaskRecord::new(first.run_id, "dependent");
        dependent.dependencies.push(first.id);
        dependent.state = TaskState::Pending;
        let completed = |record: &TaskRecord| {
            RuntimeEventEnvelope::new(
                2,
                record.run_id,
                None,
                None,
                None,
                RuntimeEvent::TaskCompleted {
                    task_id: record.id,
                    success: true,
                },
            )
        };
        assert!(TaskRegistry::legacy_snapshot(
            &[created(&first), created(&dependent), completed(&dependent)],
            "session"
        )
        .is_err());
        let snapshot = TaskRegistry::legacy_snapshot(
            &[
                created(&first),
                created(&dependent),
                completed(&first),
                completed(&dependent),
            ],
            "session",
        )
        .unwrap();
        assert!(snapshot
            .iter()
            .all(|task| task.state == TaskState::Completed));
    }

    use super::*;
    use crate::runtime::{AgentId, RunId, RuntimeEvent, TaskState};

    fn created(record: &TaskRecord) -> RuntimeEventEnvelope {
        RuntimeEventEnvelope::new(
            1,
            record.run_id,
            None,
            None,
            None,
            RuntimeEvent::TaskCreated {
                task_id: record.id,
                record: Some(record.clone()),
            },
        )
    }

    #[test]
    fn f03_legacy_snapshot_validates_and_preserves_multiple_runs() {
        let first = TaskRecord::new(RunId::new(), "first");
        let mut second = TaskRecord::new(RunId::new(), "dependent");
        second.dependencies.push(first.id);
        second.state = TaskState::Pending;
        let mut historical = TaskRecord::new(RunId::new(), "cancelled");
        historical.state = TaskState::Cancelled;
        historical.revision = 8;
        historical.result = Some("historical detail".into());
        let actor = AgentId::new();
        let events = vec![
            created(&first),
            created(&second),
            created(&historical),
            RuntimeEventEnvelope::new(
                1,
                first.run_id,
                None,
                Some(actor),
                None,
                RuntimeEvent::TaskAssigned {
                    task_id: first.id,
                    agent_id: actor,
                },
            ),
            RuntimeEventEnvelope::new(
                2,
                first.run_id,
                Some("session".into()),
                Some(actor),
                None,
                RuntimeEvent::TaskCompleted {
                    task_id: first.id,
                    success: true,
                },
            ),
        ];
        let snapshot = TaskRegistry::legacy_snapshot(&events, "session").unwrap();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(
            snapshot.iter().find(|t| t.id == historical.id),
            Some(&historical)
        );
        let completed = snapshot.iter().find(|t| t.id == first.id).unwrap();
        assert_eq!(completed.run_id, first.run_id);
        assert_eq!(completed.state, TaskState::Completed);
        assert_eq!(completed.assigned_to, Some(actor));
        assert!(completed.evidence_refs.is_empty());
        assert!(completed.result.is_none());
        assert_eq!(
            snapshot.iter().find(|t| t.id == second.id).unwrap().state,
            TaskState::Ready
        );
    }

    #[test]
    fn f03_legacy_snapshot_rejects_ambiguous_identity_and_history() {
        let task = TaskRecord::new(RunId::new(), "original");
        let valid = created(&task);
        let mut foreign = valid.clone();
        foreign.session_id = Some("different-session".into());
        let mut schema = valid.clone();
        schema.schema_version = 2;
        let mut origin = valid.clone();
        origin.run_id = RunId::new();
        let mut identity = valid.clone();
        identity.payload = RuntimeEvent::TaskCreated {
            task_id: TaskId::new(),
            record: Some(task.clone()),
        };
        let mut absent = valid.clone();
        absent.payload = RuntimeEvent::TaskCreated {
            task_id: task.id,
            record: None,
        };
        let completion = RuntimeEventEnvelope::new(
            2,
            task.run_id,
            None,
            None,
            None,
            RuntimeEvent::TaskCompleted {
                task_id: task.id,
                success: true,
            },
        );
        let mut wrong_completion = completion.clone();
        wrong_completion.run_id = RunId::new();
        let mut duplicate = valid.clone();
        duplicate.timestamp_ms += 1;
        let mut divergent = task.clone();
        divergent.title = "different".into();
        for events in [
            vec![foreign],
            vec![schema],
            vec![origin],
            vec![identity],
            vec![absent],
            vec![completion.clone()],
            vec![valid.clone(), wrong_completion],
            vec![valid.clone(), duplicate],
            vec![valid.clone(), created(&divergent)],
            vec![
                valid.clone(),
                completion,
                RuntimeEventEnvelope::new(
                    3,
                    task.run_id,
                    None,
                    None,
                    None,
                    RuntimeEvent::TaskAssigned {
                        task_id: task.id,
                        agent_id: AgentId::new(),
                    },
                ),
            ],
        ] {
            assert!(TaskRegistry::legacy_snapshot(&events, "session").is_err());
        }
        assert_eq!(
            TaskRegistry::legacy_snapshot(&[valid.clone(), valid, created(&task)], "session")
                .unwrap(),
            vec![task]
        );
    }

    #[test]
    fn f03_legacy_snapshot_validates_graph_and_failure_cascades() {
        let mut first = TaskRecord::new(RunId::new(), "first");
        let mut second = TaskRecord::new(RunId::new(), "second");
        second.dependencies.push(first.id);
        first.dependencies.push(second.id);
        assert!(
            TaskRegistry::legacy_snapshot(&[created(&first), created(&second)], "session").is_err()
        );
        first.dependencies.clear();
        first.state = TaskState::Failed;
        let mut third = TaskRecord::new(RunId::new(), "third");
        third.dependencies.push(second.id);
        let snapshot = TaskRegistry::legacy_snapshot(
            &[created(&third), created(&second), created(&first)],
            "session",
        )
        .unwrap();
        for id in [second.id, third.id] {
            assert_eq!(
                snapshot.iter().find(|t| t.id == id).unwrap().state,
                TaskState::Blocked
            );
        }
        assert!(TaskRegistry::legacy_snapshot(&[], "session")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn f03_recordless_legacy_recovery_requires_host_metadata_without_authority() {
        let run = RunId::new();
        let task_id = TaskId::new();
        let actor = AgentId::new();
        let created = RuntimeEventEnvelope::new(
            1,
            run,
            None,
            None,
            None,
            RuntimeEvent::TaskCreated {
                task_id,
                record: None,
            },
        );
        let assigned = RuntimeEventEnvelope::new(
            2,
            run,
            None,
            Some(actor),
            None,
            RuntimeEvent::TaskAssigned {
                task_id,
                agent_id: actor,
            },
        );
        let completed = RuntimeEventEnvelope::new(
            3,
            run,
            None,
            Some(actor),
            None,
            RuntimeEvent::TaskCompleted {
                task_id,
                success: true,
            },
        );
        assert!(TaskRegistry::legacy_snapshot(&[created.clone()], "session").is_err());

        let mut recovery = HashMap::new();
        let mut metadata = LegacyTaskRecovery::new("recovered task");
        metadata.description = Some("host supplied description".into());
        recovery.insert(task_id, metadata);
        let snapshot = TaskRegistry::legacy_snapshot_with_recovery(
            &[created.clone(), assigned, completed],
            "session",
            &recovery,
        )
        .unwrap();
        let recovered = &snapshot[0];
        assert_eq!(recovered.id, task_id);
        assert_eq!(recovered.run_id, run);
        assert_eq!(recovered.title, "recovered task");
        assert_eq!(recovered.assigned_to, Some(actor));
        assert_eq!(recovered.state, TaskState::Completed);
        assert_eq!(recovered.result, None);
        assert!(recovered.evidence_refs.is_empty());
        assert_eq!(recovered.revision, 0);
        assert_eq!(recovered.owner_generation, 0);

        let decision_task_id = TaskId::new();
        let decision_created = RuntimeEventEnvelope::new(
            4,
            run,
            None,
            None,
            None,
            RuntimeEvent::TaskCreated {
                task_id: decision_task_id,
                record: None,
            },
        );
        let mut decision_recovery = HashMap::new();
        let mut decision_metadata = LegacyTaskRecovery::new("decision-bound task");
        decision_metadata.decision_prerequisites.push(
            super::super::tasks::DecisionPrerequisiteRef::new("decision-1", 4),
        );
        decision_recovery.insert(decision_task_id, decision_metadata);
        let decision_snapshot = TaskRegistry::legacy_snapshot_with_recovery(
            &[decision_created.clone()],
            "session",
            &decision_recovery,
        )
        .unwrap();
        assert_eq!(decision_snapshot[0].state, TaskState::Blocked);
        let decision_completed = RuntimeEventEnvelope::new(
            5,
            run,
            None,
            None,
            None,
            RuntimeEvent::TaskCompleted {
                task_id: decision_task_id,
                success: true,
            },
        );
        assert!(TaskRegistry::legacy_snapshot_with_recovery(
            &[decision_created, decision_completed],
            "session",
            &decision_recovery
        )
        .is_err());

        let mut missing = HashMap::new();
        missing.insert(TaskId::new(), LegacyTaskRecovery::new("unused"));
        assert!(TaskRegistry::legacy_snapshot_with_recovery(
            &[created.clone()],
            "session",
            &missing
        )
        .is_err());
        let mut extra = recovery;
        extra.insert(TaskId::new(), LegacyTaskRecovery::new("extra"));
        assert!(
            TaskRegistry::legacy_snapshot_with_recovery(&[created], "session", &extra).is_err()
        );
    }
}
