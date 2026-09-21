use super::super::store::{atomic_write, run_dir, write_task_attempt, TaskAttemptRecord};
use super::*;

impl GraphExecution {
    pub(super) fn begin_attempt(&self, spec: &WorkerSpec, attempt: u32) -> bool {
        let record = {
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
            }
        };
        self.checkpoint_with(
            Some(&format!(
                "{}: attempt {attempt} ({})",
                spec.task_id, spec.role
            )),
            |run| {
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
