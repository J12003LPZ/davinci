//! Durable controller stack. Terminal UI phase is not an execution cursor.
use super::mutation::MutationBaseline;
use super::types::{ImplementationPlan, PatchReport};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeIndices {
    pub plan: u32,
    pub implement: u32,
    pub review: u32,
    #[serde(default)]
    pub security: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStage {
    Plan,
    Implement,
    Verify,
    Review,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryCheckpoint {
    pub stage: DeliveryStage,
    pub baseline: MutationBaseline,
    pub attempt_baseline: Option<MutationBaseline>,
    pub plan: Option<ImplementationPlan>,
    pub patch: Option<PatchReport>,
    pub plan_task: Option<String>,
    pub writer_task: Option<String>,
    pub review_task: Option<String>,
    #[serde(default)]
    pub review_inputs: Option<String>,
    #[serde(default)]
    pub verification_inputs: Option<String>,
    pub revision_notes: Option<String>,
    pub replan_reason: Option<String>,
    pub revision_cycles: u32,
    pub replans: u32,
}

impl DeliveryCheckpoint {
    pub fn prepare_revision(&mut self, notes: Option<String>, cycles: u32) {
        self.stage = DeliveryStage::Implement;
        self.writer_task = None;
        self.review_task = None;
        self.review_inputs = None;
        self.verification_inputs = None;
        self.attempt_baseline = None;
        self.patch = None;
        self.revision_notes = notes;
        self.revision_cycles = cycles;
    }

    pub fn prepare_replan(&mut self, reason: String, replans: u32) {
        self.prepare_revision(None, self.revision_cycles);
        self.stage = DeliveryStage::Plan;
        self.plan_task = None;
        self.plan = None;
        self.replan_reason = Some(reason);
        self.replans = replans;
    }

    pub fn new(baseline: MutationBaseline, needs_plan: bool) -> Self {
        Self {
            stage: if needs_plan {
                DeliveryStage::Plan
            } else {
                DeliveryStage::Implement
            },
            baseline,
            attempt_baseline: None,
            plan: None,
            patch: None,
            plan_task: None,
            writer_task: None,
            review_task: None,
            review_inputs: None,
            verification_inputs: None,
            revision_notes: None,
            replan_reason: None,
            revision_cycles: 0,
            replans: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphContinuation {
    pub version: u32,
    pub completed_milestones: usize,
    pub indices: NodeIndices,
    pub evidence_digest: Option<String>,
    pub delivery: Option<DeliveryCheckpoint>,
    #[serde(default)]
    pub completed_delivery: Option<DeliveryCheckpoint>,
    #[serde(default)]
    pub completion_inputs: Option<String>,
    pub saved_baseline: Option<MutationBaseline>,
    #[serde(default)]
    pub saved_attempt_baselines: std::collections::BTreeMap<String, MutationBaseline>,
    #[serde(default)]
    pub saved_stage_inputs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub saved_verifications: std::collections::BTreeMap<String, super::types::VerificationResult>,
    #[serde(default)]
    pub saved_security:
        std::collections::BTreeMap<String, crate::native_extensions::SecurityVerification>,
    /// Authoritative history; companion records may be ahead after a crash.
    #[serde(default)]
    pub attempt_history: std::collections::BTreeMap<String, Vec<super::store::TaskAttemptRecord>>,
}

impl Default for GraphContinuation {
    fn default() -> Self {
        Self {
            version: 1,
            completed_milestones: 0,
            indices: NodeIndices::default(),
            evidence_digest: None,
            delivery: None,
            completed_delivery: None,
            completion_inputs: None,
            saved_baseline: None,
            saved_attempt_baselines: Default::default(),
            saved_stage_inputs: Default::default(),
            saved_verifications: Default::default(),
            saved_security: Default::default(),
            attempt_history: Default::default(),
        }
    }
}

/// Convert a crashed in-flight worker into a failed retry boundary only when
/// its private conversation is no longer owned and its durable tool ledger is
/// safe to continue. The caller persists the returned attempt records together
/// with the updated run before dispatching another worker.
pub fn reconcile_interrupted_attempts(
    run: &mut super::types::GraphRun,
    cwd: &std::path::Path,
) -> Result<Vec<super::store::TaskAttemptRecord>, String> {
    use super::types::TaskStatus;
    let running: Vec<String> = run
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Running)
        .map(|task| task.id.clone())
        .collect();
    if running.is_empty() {
        return Ok(Vec::new());
    }
    let mut reconciled = Vec::with_capacity(running.len());
    for id in running {
        let attempt = run.task(&id).map(|task| task.attempts).unwrap_or_default();
        let record = run
            .continuation
            .as_ref()
            .and_then(|cursor| cursor.attempt_history.get(&id))
            .and_then(|history| history.last())
            .filter(|record| record.attempt == attempt && record.status == TaskStatus::Running)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Interrupted worker '{id}' has no durable running attempt binding; reconciliation required"
                )
            })?;
        let binding = record.worker_session.as_ref().ok_or_else(|| {
            format!(
                "Interrupted worker '{id}' has no private conversation binding; reconciliation required"
            )
        })?;
        binding.validate_retry_safety().map_err(|error| {
            format!("Interrupted worker '{id}' requires reconciliation: {error}")
        })?;
        let effects = super::store::artifact_path(cwd, &run.run_id, &id)
            .with_extension("effects.jsonl");
        if effects.exists() {
            davinci_agent::runtime::effects::read_effect_report(&effects).map_err(|error| {
                format!("Interrupted worker '{id}' has an invalid effect report: {error}")
            })?;
        }
        let ended_at = super::store::now_ms();
        let reason = "controller interrupted before the worker published a terminal result; recovered ownership is safe to retry".to_string();
        let mut recovered = record;
        recovered.status = TaskStatus::Failed;
        recovered.exit_code = Some(-1);
        recovered.ended_at = Some(ended_at);
        recovered.error = Some(reason.clone());
        if let Some(last) = run
            .continuation
            .as_mut()
            .and_then(|cursor| cursor.attempt_history.get_mut(&id))
            .and_then(|history| history.last_mut())
        {
            *last = recovered.clone();
        }
        if let Some(task) = run.tasks.iter_mut().find(|task| task.id == id) {
            task.status = TaskStatus::Failed;
            task.ended_at = Some(ended_at);
            task.error = Some(reason);
        }
        reconciled.push(recovered);
    }
    Ok(reconciled)
}

pub fn validate_resume(run: &super::types::GraphRun, cwd: &std::path::Path) -> Result<(), String> {
    if run.version != 1 || !super::store::is_safe_run_id(&run.run_id) || run.revision == u64::MAX {
        return Err("Checkpoint identity/version/revision is not resumable".into());
    }
    let stored = std::path::Path::new(&run.cwd)
        .canonicalize()
        .map_err(|error| format!("Cannot resolve checkpoint workspace: {error}"))?;
    if stored != cwd.canonicalize().map_err(|error| error.to_string())? {
        return Err("Checkpoint belongs to a different workspace".into());
    }
    let cursor = run
        .continuation
        .as_ref()
        .ok_or("Checkpoint has no durable execution cursor; automatic restart is refused")?;
    if cursor.version != 1 {
        return Err("Unsupported execution cursor version".into());
    }
    validate_attempt_history(run, cursor)?;
    if run.saved_definition.is_none() {
        let milestones = run
            .milestones
            .as_ref()
            .map_or(1, |items| items.len().max(1));
        if cursor.completed_milestones > milestones
            || (cursor.completed_milestones == milestones && cursor.delivery.is_some())
            || (run.classification.is_none()
                && (cursor.completed_milestones != 0 || cursor.delivery.is_some()))
        {
            return Err(
                "Checkpoint milestone cursor is inconsistent with its classification".into(),
            );
        }
        if cursor.completed_milestones > 0
            && (cursor.completed_delivery.is_none() || cursor.completion_inputs.is_none())
        {
            return Err("Completed milestone has no retained delivery evidence".into());
        }
        if let Some(delivery) = &cursor.delivery {
            if delivery.revision_cycles > run.counters.revision_cycles
                || delivery.revision_cycles > run.budgets.max_revision_cycles
                || delivery.replans > run.counters.replans
                || delivery.replans > run.budgets.max_replans
            {
                return Err(
                    "Checkpoint delivery budgets are inconsistent with retained spending".into(),
                );
            }
            for (id, prefix, index, role) in [
                (
                    &delivery.plan_task,
                    "plan",
                    cursor.indices.plan,
                    super::types::Role::Planner,
                ),
                (
                    &delivery.writer_task,
                    "implement",
                    cursor.indices.implement,
                    super::types::Role::Writer,
                ),
                (
                    &delivery.review_task,
                    "review",
                    cursor.indices.review,
                    super::types::Role::Reviewer,
                ),
            ] {
                if let Some(id) = id {
                    if index == 0
                        || *id != format!("{prefix}-{index}")
                        || run.task(id).is_some_and(|task| task.role != role)
                    {
                        return Err(format!("Checkpoint has an inconsistent {prefix} cursor"));
                    }
                }
            }
            if matches!(
                delivery.stage,
                DeliveryStage::Verify | DeliveryStage::Review
            ) {
                let writer = delivery.writer_task.as_ref().and_then(|id| run.task(id));
                if delivery.patch.is_none()
                    || delivery.attempt_baseline.is_none()
                    || writer.is_none_or(|task| task.status != super::types::TaskStatus::Succeeded)
                {
                    return Err(
                        "Checkpoint verification cursor has no completed writer boundary".into(),
                    );
                }
            }
        }
    }
    if run.phase == super::types::Phase::Done {
        return Err("Run is already complete".into());
    }
    if run
        .tasks
        .iter()
        .any(|task| task.status == super::types::TaskStatus::Running)
    {
        return Err("Interrupted worker attempts require reconciliation before resume".into());
    }
    if run.tasks.iter().any(|task| {
        task.error
            .as_deref()
            .is_some_and(|error| error.contains("reconciliation required"))
    }) {
        return Err(
            "A worker outcome is still uncertain; reconcile its effects before resuming".into(),
        );
    }
    if let Some(definition) = &run.definition {
        super::topology::validate_definition(definition)
            .map_err(|error| format!("Invalid checkpoint topology: {error}"))?;
    }
    let saved_plan = run
        .saved_definition
        .as_ref()
        .map(super::bindings::compile_saved_definition)
        .transpose()?;
    if let Some(plan) = &saved_plan {
        if run.definition_digest.as_ref() != Some(&plan.definition_digest)
            || run
                .definition
                .as_ref()
                .is_some_and(|definition| *definition != plan.topology)
            || (!run.tasks.is_empty() && run.definition.is_none())
        {
            return Err("Saved checkpoint definition differs from its compiled identity".into());
        }
        if !run.tasks.is_empty() && cursor.saved_baseline.is_none() {
            return Err("Saved checkpoint has no original mutation baseline".into());
        }
        for task in &run.tasks {
            let node = plan
                .topology
                .node(&task.id)
                .ok_or_else(|| format!("Saved checkpoint contains unknown task '{}'", task.id))?;
            if task.role != node.role || task.expect != node.expect {
                return Err(format!(
                    "Saved task '{}' has incompatible role or contract",
                    task.id
                ));
            }
            if node.allows_mutation
                && task.attempts > 0
                && !cursor.saved_attempt_baselines.contains_key(&task.id)
            {
                return Err(format!(
                    "Saved writer '{}' has no original attempt baseline",
                    task.id
                ));
            }
        }
    }
    let mut ids = std::collections::HashSet::new();
    for task in &run.tasks {
        if !super::store::is_safe_run_id(&task.id) || !ids.insert(&task.id) {
            return Err("Checkpoint contains an invalid or duplicate task identity".into());
        }
        if task.status == super::types::TaskStatus::Succeeded && task.artifact_file.is_none() {
            let native_check = saved_plan
                .as_ref()
                .and_then(|plan| plan.bindings.get(&task.id))
                .map(|binding| binding.stage);
            let valid = match native_check {
                Some(super::bindings::SupportedStage::Verify) => cursor
                    .saved_verifications
                    .get(&task.id)
                    .is_some_and(|result| result.passed),
                Some(super::bindings::SupportedStage::Security) => matches!(
                    cursor.saved_security.get(&task.id),
                    Some(crate::native_extensions::SecurityVerification::Passed { .. })
                ),
                None if saved_plan.is_none() => {
                    task.attempts == 0 && task.role == super::types::Role::TestAnalyzer
                }
                _ => false,
            };
            if !valid || (saved_plan.is_some() && !cursor.saved_stage_inputs.contains_key(&task.id))
            {
                return Err(format!(
                    "Completed task '{}' has no durable artifact or check receipt",
                    task.id
                ));
            }
        }
        if task.status == super::types::TaskStatus::Succeeded && task.artifact_file.is_some() {
            if task.artifact_file.as_deref() != Some(&format!("artifacts/{}.json", task.id)) {
                return Err(format!(
                    "Task '{}' artifact identity is inconsistent",
                    task.id
                ));
            }
            let artifact = super::store::read_artifact(cwd, &run.run_id, &task.id, task.expect)
                .map_err(|error| {
                    format!(
                        "Cannot restore completed task '{}': {}",
                        task.id,
                        error.join("; ")
                    )
                })?;
            let fingerprint = task
                .fingerprint
                .as_ref()
                .ok_or_else(|| format!("Completed task '{}' has no input fingerprint", task.id))?;
            if fingerprint.contract_hash != super::replay::compute_contract_hash(task.expect)
                || fingerprint.graph_version
                    != run
                        .definition
                        .as_ref()
                        .map_or(run.version, |definition| definition.version)
                || fingerprint.definition_digest != run.definition_digest
                || (fingerprint.simulated.unwrap_or(false) && !run.dry_run)
            {
                return Err(format!(
                    "Completed task '{}' has an incompatible artifact contract",
                    task.id
                ));
            }
            if let Some(delivery) = &cursor.delivery {
                if delivery.writer_task.as_deref() == Some(&task.id)
                    && delivery.patch.is_some()
                    && artifact.as_patch_report() != delivery.patch.as_ref()
                {
                    return Err(
                        "Checkpoint patch disagrees with its completed writer artifact".into(),
                    );
                }
                if delivery.plan_task.as_deref() == Some(&task.id)
                    && delivery.plan.is_some()
                    && artifact.as_plan() != delivery.plan.as_ref()
                {
                    return Err(
                        "Checkpoint plan disagrees with its completed planner artifact".into(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_attempt_history(
    run: &super::types::GraphRun,
    cursor: &GraphContinuation,
) -> Result<(), String> {
    use super::types::TaskStatus;
    for (id, records) in &cursor.attempt_history {
        let invalid = || format!("Checkpoint attempt history is inconsistent for '{id}'");
        let task = run.task(id).ok_or_else(invalid)?;
        if !super::store::is_safe_run_id(id)
            || records
                .last()
                .is_none_or(|record| record.attempt != task.attempts)
        {
            return Err(invalid());
        }
        let mut previous: Option<u32> = None;
        for record in records {
            if record.task_id != *id
                || record.attempt == 0
                || previous.is_some_and(|value| value.checked_add(1) != Some(record.attempt))
                || record.started_at.is_none()
            {
                return Err(invalid());
            }
            let running = record.status == TaskStatus::Running;
            if running != record.ended_at.is_none()
                || running != record.exit_code.is_none()
                || (running && record.attempt != task.attempts)
                || record.status == TaskStatus::Pending
            {
                return Err(invalid());
            }
            let fingerprint = record.fingerprint.as_ref().ok_or_else(invalid)?;
            if fingerprint.contract_hash != super::replay::compute_contract_hash(task.expect)
                || fingerprint.definition_digest != run.definition_digest
                || fingerprint.graph_version
                    != run.definition.as_ref().map_or(run.version, |d| d.version)
            {
                return Err(invalid());
            }
            if record.status == TaskStatus::Succeeded && record.artifact_file.is_none() {
                return Err(invalid());
            }
            if let Some(binding) = &record.worker_session {
                if binding.graph_run != run.run_id
                    || binding.task_id != *id
                    || binding.attempt != record.attempt
                    || binding.role != task.role
                    || binding.expect != task.expect
                    || binding.graph_revision > run.revision
                {
                    return Err(invalid());
                }
                binding
                    .validate(std::path::Path::new(&run.cwd))
                    .map_err(|error| {
                        format!("Cannot restore worker conversation for '{id}': {error}")
                    })?;
            }
            if let Some(path) = &record.artifact_file {
                let identity = format!("{id}.attempt_{}.artifact", record.attempt);
                if *path != format!("artifacts/{identity}.json") {
                    return Err(invalid());
                }
                super::store::read_artifact(
                    std::path::Path::new(&run.cwd),
                    &run.run_id,
                    &identity,
                    task.expect,
                )
                .map_err(|errors| {
                    format!(
                        "Cannot restore attempt history for '{id}': {}",
                        errors.join("; ")
                    )
                })?;
            }
            previous = Some(record.attempt);
        }
    }
    Ok(())
}
