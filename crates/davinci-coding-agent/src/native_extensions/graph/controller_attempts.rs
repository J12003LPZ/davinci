use super::super::store::{atomic_write, run_dir, write_task_attempt, TaskAttemptRecord};
use super::*;

impl GraphExecution {
    pub(super) fn retry_recovery_input(
        &self,
        task_id: &str,
        attempt: u32,
        local_attempt: usize,
        recommendation: RetryDecision,
        binding: Option<&super::super::worker_sessions::WorkerSessionBinding>,
    ) -> RetryRecoveryInput {
        let budget_snapshot = self.snapshot();
        let budget_available = Self::budget_exceeded(&budget_snapshot, now_ms()).is_none();
        let (original_binding_valid, prior_owner_quiescent, source_reconciled) = binding
            .map(|binding| {
                let original = binding.validate(&self.options.cwd).is_ok();
                let safe = original && binding.validate_retry_safety().is_ok();
                (original, safe, safe)
            })
            .unwrap_or((false, false, false));

        let operation_binding = self
            .snapshot()
            .continuation
            .as_ref()
            .and_then(|cursor| cursor.attempt_history.get(task_id))
            .and_then(|history| history.iter().find(|record| record.attempt == attempt))
            .and_then(|record| record.operation_binding.clone());
        let mut current_authority_valid = true;
        let mut operation_recovered = operation_binding.is_none();
        let mut operation_completed = false;
        let mut operation_state = None;
        let mut effect_status = None;
        if let Some(operation_binding) = operation_binding.as_ref() {
            operation_recovered = false;
            if let Some(runtime) = self.deps.runtime.as_ref() {
                match super::super::operation_bridge::inspect_retry_recovery(
                    runtime,
                    &self.options.cwd,
                    operation_binding,
                ) {
                    Ok(evidence) => {
                        operation_recovered = evidence.safe_to_retry;
                        operation_completed = evidence.already_completed;
                        operation_state = Some(evidence.state);
                        effect_status = Some(evidence.effect_status);
                    }
                    Err(error) => {
                        current_authority_valid = !matches!(
                            error,
                            super::super::operation_bridge::GraphOperationBridgeError::ParentAuthorityChanged
                        );
                    }
                }
            }
        }

        RetryRecoveryInput {
            recommendation,
            attempt: local_attempt,
            budget_available,
            prior_owner_quiescent,
            original_binding_valid,
            current_authority_valid,
            source_reconciled,
            operation_recovered,
            operation_completed,
            operation_state,
            effect_status,
        }
    }

    pub(super) fn record_retry_recovery(
        &self,
        task_id: &str,
        attempt: u32,
        recovery: super::super::recovery::RetryRecoveryRecord,
    ) -> bool {
        self.checkpoint_with(
            Some(&format!("{task_id}: retry recovery decision recorded")),
            |run| {
                let record = run
                    .continuation
                    .as_mut()
                    .and_then(|cursor| cursor.attempt_history.get_mut(task_id))
                    .and_then(|history| history.iter_mut().find(|record| record.attempt == attempt))
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("missing attempt record for {task_id} attempt {attempt}"),
                        )
                    })?;
                record.retry_recovery = Some(recovery);
                let snapshot = record.clone();
                write_task_attempt(
                    Path::new(&run.cwd),
                    &run.run_id,
                    task_id,
                    attempt,
                    &snapshot,
                )?;
                Ok(())
            },
        )
    }

    pub(super) fn begin_attempt(&self, spec: &mut WorkerSpec, attempt: u32) -> bool {
        let mut record = {
            let run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            TaskAttemptRecord {
                task_id: spec.task_id.clone(),
                attempt,
                status: TaskStatus::Running,
                exit_code: None,
                child_pid: None,
                timed_out: false,
                run_deadline_exceeded: false,
                artifact_file: None,
                error: None,
                usage: WorkerUsage::default(),
                started_at: Some(now_ms()),
                ended_at: None,
                fingerprint: Some(ReplayFingerprint::for_saved_task(
                    &spec.cwd,
                    run.definition.as_ref().map_or(run.version, |d| d.version),
                    &spec.briefing,
                    spec.expect,
                    run.definition_digest.as_deref(),
                    run.dry_run,
                )),
                worker_session: None,
                operation_binding: None,
                retry_recovery: None,
            }
        };
        self.checkpoint_with(
            Some(&format!(
                "{}: attempt {attempt} ({})",
                spec.task_id, spec.role
            )),
            |run| {
                let previous = run
                    .continuation
                    .as_ref()
                    .and_then(|continuation| continuation.attempt_history.get(&spec.task_id))
                    .and_then(|history| history.last())
                    .and_then(|record| record.worker_session.clone());
                let binding = super::super::worker_sessions::WorkerSessionBinding::create(
                    spec,
                    &run.run_id,
                    run.revision,
                    attempt,
                    self.deps.runtime.as_ref(),
                    previous.as_ref(),
                )
                .map_err(std::io::Error::other)?;
                let operation_binding = self
                    .deps
                    .runtime
                    .as_ref()
                    .filter(|runtime| runtime.operations.is_some())
                    .map(|runtime| {
                        super::super::operation_bridge::launch_worker(
                            runtime,
                            &spec.cwd,
                            &run.run_id,
                            &spec.task_id,
                            attempt,
                            &binding,
                            binding.contract_digest.as_deref(),
                        )
                        .map(|launch| launch.binding)
                    })
                    .transpose()
                    .map_err(std::io::Error::other)?;
                spec.worker_session = Some(binding.clone());
                record.worker_session = Some(binding);
                record.operation_binding = operation_binding;
                write_task_attempt(&spec.cwd, &run.run_id, &spec.task_id, attempt, &record)?;
                run.continuation
                    .as_mut()
                    .unwrap()
                    .attempt_history
                    .entry(spec.task_id.clone())
                    .or_default()
                    .push(record);
                Ok(())
            },
        )
    }

    pub(super) fn finish_attempt(
        &self,
        spec: &WorkerSpec,
        attempt: u32,
        result: &WorkerResult,
    ) -> bool {
        let record = {
            let run = self.run.lock().unwrap_or_else(|error| error.into_inner());
            let mut record = run
                .continuation
                .as_ref()
                .unwrap()
                .attempt_history
                .get(&spec.task_id)
                .unwrap()
                .last()
                .unwrap()
                .clone();
            debug_assert_eq!(record.attempt, attempt);
            record.status = if result.ok && result.artifact.is_some() {
                TaskStatus::Succeeded
            } else if result.run_deadline_exceeded
                || self.exec_abort.load(Ordering::SeqCst)
                || spec
                    .node_abort
                    .as_ref()
                    .is_some_and(|abort| abort.load(Ordering::SeqCst))
            {
                TaskStatus::Cancelled
            } else {
                TaskStatus::Failed
            };
            record.exit_code = Some(result.exit_code);
            record.child_pid = result.child_pid;
            record.timed_out = result.timed_out;
            record.run_deadline_exceeded = result.run_deadline_exceeded;
            record.usage = result.usage;
            record.ended_at = Some(now_ms());
            record.error = (record.status != TaskStatus::Succeeded).then(|| {
                result.failure_reason.clone().unwrap_or_else(|| {
                    if result.timed_out {
                        "timed out".into()
                    } else {
                        format!(
                            "exit {}; {}",
                            result.exit_code,
                            truncate(&result.stderr, 300)
                        )
                    }
                })
            });
            record.artifact_file = result
                .artifact
                .as_ref()
                .map(|_| format!("artifacts/{}.attempt_{attempt}.artifact.json", spec.task_id));
            record
        };
        if let (Some(runtime), Some(binding)) = (
            self.deps
                .runtime
                .as_ref()
                .filter(|runtime| runtime.operations.is_some()),
            record.operation_binding.as_ref(),
        ) {
            if let Err(error) =
                super::super::operation_bridge::complete_worker(runtime, &spec.cwd, binding, result)
            {
                eprintln!(
                    "graph worker operation result could not be persisted for {} attempt {}: {error}",
                    spec.task_id, attempt
                );
                return false;
            }
        }
        self.checkpoint_with(None, |run| {
            let root = run_dir(&spec.cwd, &run.run_id);
            if let (Some(path), Some(artifact)) = (&record.artifact_file, &result.artifact) {
                write_artifact(&root.join(path), artifact)?;
            }
            let effects = spec.artifact_path.with_extension("effects.jsonl");
            match std::fs::read(&effects) {
                Ok(bytes) => atomic_write(
                    &root.join(format!(
                        "artifacts/{}.attempt_{attempt}.effects.jsonl",
                        spec.task_id,
                    )),
                    &bytes,
                )?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            write_task_attempt(&spec.cwd, &run.run_id, &spec.task_id, attempt, &record)?;
            *run.continuation
                .as_mut()
                .unwrap()
                .attempt_history
                .get_mut(&spec.task_id)
                .unwrap()
                .last_mut()
                .unwrap() = record;
            Ok(())
        })
    }
}
