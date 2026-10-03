//! Controls are applied to the executing state and checkpointed before acknowledgement.
use super::super::control::{
    self, ControlReceiptState, GraphControl, GraphControlAction, GraphControlReceipt,
};
use super::*;

pub(super) struct ActiveOperation(pub Arc<AtomicUsize>);

impl Drop for ActiveOperation {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl GraphExecution {
    pub(in crate::native_extensions::graph) fn apply_control(
        &self,
        request: &GraphControl,
    ) -> Result<GraphControlReceipt, String> {
        let (snapshot, receipt) = {
            let mut run = self.run.lock().map_err(|_| "Graph state lock poisoned")?;
            let mut failure = self
                .persistence_error
                .lock()
                .map_err(|_| "Graph persistence lock poisoned")?;
            if let Some(reason) = failure.as_ref() {
                return Err(reason.clone());
            }
            let mut candidate = run.clone();
            let before = candidate.control_history.len();
            // Include workers whose durable dispatch reservation is being prepared.
            let in_flight = self.active_workers.load(Ordering::SeqCst).max(
                run.tasks
                    .iter()
                    .filter(|task| task.status == TaskStatus::Running)
                    .count(),
            );
            let mut receipt = control::reduce_control(
                &mut candidate,
                request,
                &mut control::ControlTracker::new(),
                in_flight,
                true,
            );
            if candidate.control_history.len() == before {
                return Ok(receipt);
            }
            if request.action == GraphControlAction::RetryNode
                && receipt.state == ControlReceiptState::Applied
            {
                // The current stack cannot rewind to an arbitrary earlier node.
                // Refuse instead of acknowledging work it will never schedule.
                candidate = run.clone();
                receipt.state = ControlReceiptState::Rejected;
                receipt.accepted_revision = run.revision;
                receipt.affected_nodes.clear();
                receipt.reason =
                    Some("Stop the executing controller before retrying a node.".into());
                candidate.control_history.push(control::GraphControlRecord {
                    request: request.clone(),
                    receipt: receipt.clone(),
                });
            }
            self.acknowledge_controls(&mut candidate);
            if let Err(error) = save_run(&mut candidate) {
                let reason = format!("checkpoint persistence failed: {error}");
                *failure = Some(reason.clone());
                self.exec_abort.store(true, Ordering::SeqCst);
                run.phase = Phase::Blocked;
                run.lifecycle = Some(GraphLifecycle::Stopped);
                run.blocked_reason = Some(reason.clone());
                let snapshot = run.clone();
                drop(failure);
                drop(run);
                (self.deps.on_update)(
                    &snapshot,
                    Some("control checkpoint failed; stopped state not saved"),
                );
                return Err(reason);
            }
            *run = candidate;
            if matches!(
                receipt.state,
                ControlReceiptState::Accepted | ControlReceiptState::Applied
            ) {
                match request.action {
                    GraphControlAction::StopGraph => {
                        self.options.abort.store(true, Ordering::SeqCst);
                        self.exec_abort.store(true, Ordering::SeqCst);
                    }
                    GraphControlAction::StopNode => {
                        if let Some(id) = &request.node_id {
                            self.register_node_abort(id).store(true, Ordering::SeqCst);
                        }
                    }
                    _ => {}
                }
            }
            (run.clone(), receipt)
        };
        (self.deps.on_update)(&snapshot, Some("graph control checkpoint saved"));
        Ok(receipt)
    }

    pub(super) fn acknowledge_controls(&self, run: &mut GraphRun) {
        let quiescent = self.active_workers.load(Ordering::SeqCst) == 0
            && !run
                .tasks
                .iter()
                .any(|task| task.status == TaskStatus::Running);
        if quiescent && run.current_lifecycle() == GraphLifecycle::PauseRequested {
            run.lifecycle = Some(GraphLifecycle::Paused);
        }
        let lifecycle = run.current_lifecycle();
        for entry in &mut run.control_history {
            if entry.receipt.state != ControlReceiptState::Accepted {
                continue;
            }
            if entry.request.action == GraphControlAction::Pause
                && !matches!(
                    lifecycle,
                    GraphLifecycle::PauseRequested | GraphLifecycle::Paused
                )
            {
                entry.receipt.state = ControlReceiptState::Rejected;
                entry.receipt.reason =
                    Some("Pause superseded before its boundary was reached".into());
                continue;
            }
            let applied = match entry.request.action {
                GraphControlAction::Pause => quiescent && lifecycle == GraphLifecycle::Paused,
                GraphControlAction::StopGraph => quiescent && lifecycle == GraphLifecycle::Stopped,
                GraphControlAction::StopNode => entry.request.node_id.as_ref().is_some_and(|id| {
                    run.tasks
                        .iter()
                        .any(|task| task.id == *id && task.status == TaskStatus::Cancelled)
                }),
                _ => false,
            };
            if applied {
                entry.receipt.state = ControlReceiptState::Applied;
                entry.receipt.reason = None;
            }
        }
    }

    pub(super) fn wait_until_running(&self) -> bool {
        loop {
            if self.exec_abort.load(Ordering::SeqCst) {
                return false;
            }
            if self
                .run_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.budget_abort("run deadline exceeded".into());
                return false;
            }
            let lifecycle = self.snapshot().current_lifecycle();
            if graph_dispatch_allowed(lifecycle.as_str(), true, true) {
                return true;
            }
            match lifecycle {
                GraphLifecycle::Paused => {}
                GraphLifecycle::PauseRequested => {
                    if self.active_workers.load(Ordering::SeqCst) == 0 {
                        self.checkpoint(Some("waiting at pause boundary"));
                    }
                }
                _ => return false,
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}
