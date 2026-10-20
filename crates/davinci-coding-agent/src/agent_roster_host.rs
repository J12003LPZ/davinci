//! Host side of the agent roster: it feeds the TUI model from the runtime and
//! sends what the user types in a worker's view.
//!
//! The model is plain data and never touches the runtime. Once per UI tick
//! `RosterHost::tick` copies the workers' rows and the viewed worker's new
//! activity in, runs the changed-files probe off the UI thread, takes the
//! messages typed in a worker's composer out, and moves each one's receipt
//! along as the mailbox reports it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use davinci_agent::runtime::operations::{
    ControlOperationAdapter, ControlOperationError, ControlReceiptValue,
};
use davinci_agent::runtime::worker_activity::{WorkerActivityEvent, WorkerActivityKind};
use davinci_agent::runtime::{
    AgentId, AgentKind, AgentState, ControlStatus, WorkerControlAction, WorkerControlCommand,
    WorkerController,
};
use davinci_agent::RuntimeHandle;
use davinci_tui::davinci::model::Model;
use davinci_tui::davinci::roster::{WorkerLine, WorkerLineKind};
use uuid::Uuid;

/// How often the viewed worker's worktree is asked what changed.
const PROBE_EVERY: Duration = Duration::from_secs(3);
/// A hung `git status` is given up on after this long.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Most activity lines copied into the view in one tick.
const PAGE: usize = 200;

/// What the user is told when a message reaches a worker that already ended.
pub(crate) const FINISHED_REASON: &str = "worker already finished; send a follow-up to main";

/// A send that was accepted but not fully recorded, asked again under its id.
struct Retry {
    out: davinci_tui::davinci::roster::WorkerOutbound,
    command_id: Uuid,
    attempts: u32,
    at: Instant,
}

const RETRY_AFTER: Duration = Duration::from_secs(1);
const MAX_ATTEMPTS: u32 = 4;

/// A finished probe: the worker it was for and the paths it found.
type ProbeResult = (String, Vec<String>);

#[derive(Default)]
pub(crate) struct RosterHost {
    last_probe: Option<Instant>,
    probing: Arc<AtomicBool>,
    probed: Arc<Mutex<Option<ProbeResult>>>,
    /// Worktree path of each listed worker, for the probe.
    worktrees: HashMap<String, std::path::PathBuf>,
    retries: Vec<Retry>,
}

impl RosterHost {
    /// One pass. Cheap when nothing changed, and never waits on a worker.
    pub(crate) fn tick(&mut self, runtime: &RuntimeHandle, model: &mut Model) {
        if !model.roster.enabled {
            return;
        }
        self.refresh_rows(runtime, model);
        self.refresh_activity(runtime, model);
        self.refresh_changes(model);
        self.pump(runtime, model);
    }

    /// Send what was typed and follow the receipts. Called on every pass of
    /// the UI loops, so a message does not wait for the next tick.
    pub(crate) fn pump(&mut self, runtime: &RuntimeHandle, model: &mut Model) {
        if !model.roster.enabled {
            return;
        }
        self.deliver(runtime, model);
        self.follow_receipts(runtime, model);
    }

    fn refresh_rows(&mut self, runtime: &RuntimeHandle, model: &mut Model) {
        let controller = WorkerController::new(runtime.registry.clone());
        let snapshots: Vec<_> = controller
            .build_all_snapshots()
            .into_iter()
            .filter(|snapshot| snapshot.kind != AgentKind::Main)
            .collect();
        self.worktrees = snapshots
            .iter()
            .filter_map(|snapshot| {
                snapshot
                    .workspace
                    .path
                    .clone()
                    .map(|path| (snapshot.agent_id.to_string(), path))
            })
            .collect();
        let rows = crate::davinci_surfaces::agents_from_snapshots(&snapshots);
        if rows != model.roster.workers {
            model.set_workers(rows);
        }
    }

    fn refresh_activity(&mut self, runtime: &RuntimeHandle, model: &mut Model) {
        let Some(viewed) = model.viewing_worker().map(str::to_string) else {
            return;
        };
        let Ok(agent_id) = viewed.parse::<AgentId>() else {
            return;
        };
        let cursor = model
            .roster
            .views
            .get(&viewed)
            .map(|view| view.cursor)
            .unwrap_or(0);
        let page = runtime.registry.activity().after(agent_id, cursor, PAGE);
        if page.events.is_empty() && page.dropped == 0 {
            return;
        }
        let lines = page.events.iter().map(worker_line).collect();
        model.push_worker_lines(&viewed, lines, page.dropped);
    }

    fn refresh_changes(&mut self, model: &mut Model) {
        if let Some((id, paths)) = self.probed.lock().ok().and_then(|mut slot| slot.take()) {
            model.set_worker_changed_paths(&id, paths);
        }
        let Some(viewed) = model.viewing_worker().map(str::to_string) else {
            return;
        };
        let Some(path) = self.worktrees.get(&viewed).cloned() else {
            return;
        };
        if self.probing.load(Ordering::Acquire)
            || self
                .last_probe
                .is_some_and(|last| last.elapsed() < PROBE_EVERY)
        {
            return;
        }
        self.last_probe = Some(Instant::now());
        self.probing.store(true, Ordering::Release);
        let (probing, slot) = (self.probing.clone(), self.probed.clone());
        std::thread::spawn(move || {
            let (_, paths) =
                davinci_agent::runtime::worker_workspace::probe_changes(&path, PROBE_TIMEOUT);
            if let Ok(mut slot) = slot.lock() {
                *slot = Some((viewed, paths));
            }
            probing.store(false, Ordering::Release);
        });
    }

    /// Send what was typed in a worker's composer. The receipt is the
    /// mailbox's own: nothing is called queued unless the mailbox holds it.
    fn deliver(&mut self, runtime: &RuntimeHandle, model: &mut Model) {
        for out in model.take_worker_outbox() {
            let command_id = Uuid::new_v4();
            let (state, reason, id) = send_to_worker(runtime, &out.agent_id, &out.text, command_id);
            model.set_worker_receipt(&out.agent_id, out.local, id, &state, reason);
            // Accepted but not fully recorded: ask again under the same id, so
            // the answer comes from the mailbox and nothing is sent twice.
            if state == "unknown" {
                self.retries.push(Retry {
                    out,
                    command_id,
                    attempts: 1,
                    at: Instant::now(),
                });
            }
        }
        let due: Vec<Retry> = {
            let (due, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut self.retries)
                .into_iter()
                .partition(|retry| retry.at.elapsed() >= RETRY_AFTER);
            self.retries = wait;
            due
        };
        for mut retry in due {
            let (state, reason, id) = send_to_worker(
                runtime,
                &retry.out.agent_id,
                &retry.out.text,
                retry.command_id,
            );
            model.set_worker_receipt(&retry.out.agent_id, retry.out.local, id, &state, reason);
            retry.attempts += 1;
            retry.at = Instant::now();
            if state == "unknown" && retry.attempts < MAX_ATTEMPTS {
                self.retries.push(retry);
            }
        }
    }

    /// Move queued/delivered receipts along as the worker takes the message.
    fn follow_receipts(&mut self, runtime: &RuntimeHandle, model: &mut Model) {
        let open: Vec<(String, String, String)> = model
            .roster
            .views
            .iter()
            .flat_map(|(agent, view)| {
                view.receipts.iter().filter_map(move |receipt| {
                    let id = receipt.id.clone()?;
                    matches!(receipt.state.as_str(), "queued" | "delivered")
                        .then(|| (agent.clone(), id, receipt.state.clone()))
                })
            })
            .collect();
        for (agent, id, shown) in open {
            let Ok(uuid) = id.parse::<Uuid>() else {
                continue;
            };
            if let Some(now) = runtime.mailbox.get_steering_receipt(&uuid) {
                if now.state != shown {
                    model.advance_worker_receipt(&agent, &id, &now.state, now.reason);
                }
            }
        }
    }
}

fn worker_line(event: &WorkerActivityEvent) -> WorkerLine {
    let failed = matches!(
        event.outcome.as_deref(),
        Some("error" | "failed" | "cancelled")
    );
    let (kind, text) = match event.kind {
        WorkerActivityKind::Started => (WorkerLineKind::Started, event.summary.clone()),
        WorkerActivityKind::ToolStarted | WorkerActivityKind::Message => {
            (WorkerLineKind::Call, event.summary.clone())
        }
        WorkerActivityKind::ToolFinished => (
            if failed {
                WorkerLineKind::Failed
            } else {
                WorkerLineKind::Finished
            },
            event.summary.clone(),
        ),
        WorkerActivityKind::Finished => (
            if failed {
                WorkerLineKind::Failed
            } else {
                WorkerLineKind::Finished
            },
            match &event.outcome {
                Some(outcome) if event.summary.is_empty() => outcome.clone(),
                _ => event.summary.clone(),
            },
        ),
    };
    WorkerLine {
        sequence: event.sequence,
        text,
        kind,
    }
}

/// Journal, then deliver, one steering message. Returns the receipt state,
/// why, and the stable message id when one exists.
pub(crate) fn send_to_worker(
    runtime: &RuntimeHandle,
    agent: &str,
    text: &str,
    id: Uuid,
) -> (String, Option<String>, Option<String>) {
    let Ok(agent_id) = agent.parse::<AgentId>() else {
        return ("rejected".into(), Some("unknown_worker".into()), None);
    };
    let Some(record) = runtime.registry.get(&agent_id) else {
        return ("rejected".into(), Some("unknown_worker".into()), None);
    };
    // A finished worker is never woken by a message.
    if matches!(
        record.state,
        AgentState::Completed | AgentState::Failed | AgentState::Cancelled
    ) {
        return ("rejected".into(), Some(FINISHED_REASON.into()), None);
    }
    steer_command(
        runtime,
        WorkerControlCommand {
            id,
            root_run_id: runtime.run_id,
            agent_id,
            generation: runtime.registry.get_generation(&agent_id),
            task_id: None,
            expected_revision: runtime.registry.get_revision(&agent_id),
            action: WorkerControlAction::Steer {
                message: text.to_string(),
                redirect: false,
            },
        },
    )
}

/// Run one steer command: journaled when the runtime has an operation
/// journal, straight to the mailbox otherwise. The TUI and the RPC both come
/// here, so a message has the same states and the same ids on either side.
/// Returns the receipt state, why, and the stable message id when one exists.
pub(crate) fn steer_command(
    runtime: &RuntimeHandle,
    command: WorkerControlCommand,
) -> (String, Option<String>, Option<String>) {
    let controller = WorkerController::new(runtime.registry.clone());
    let command_id = command.id;
    let message_id = command.id.to_string();
    match runtime.operations.clone() {
        // The journaled path: the intent is written before the mailbox takes
        // the message, and a repeat of the command id answers from the journal.
        Some(operations) => {
            let adapter = ControlOperationAdapter::new(operations);
            match adapter.execute_worker_control(&controller, Some(&runtime.mailbox), command, true)
            {
                Ok(receipt) => match receipt.value {
                    ControlReceiptValue::Steering(steering) => (
                        steering.state.clone(),
                        steering.reason.clone(),
                        Some(steering.message_id.to_string()),
                    ),
                    ControlReceiptValue::Worker(worker) => (
                        if worker.status == ControlStatus::Stale {
                            "stale".into()
                        } else {
                            "rejected".into()
                        },
                        worker
                            .reason
                            .or_else(|| Some(format!("control_{}", worker.status.as_str()))),
                        None,
                    ),
                    ControlReceiptValue::Task(_) => {
                        ("rejected".into(), Some("unexpected_receipt".into()), None)
                    }
                },
                // The mailbox took the message but the journal could not say
                // so: it is accepted, its receipt is incomplete. Keep the id so
                // a retry is answered from the mailbox instead of duplicating.
                Err(ControlOperationError::AcceptedUnrecorded { message_id, reason }) => (
                    "unknown".into(),
                    Some(format!("accepted; receipt incomplete: {reason}")),
                    Some(message_id.to_string()),
                ),
                // A resend of an id the journal cannot finish: the mailbox is
                // the authority on whether it holds the message.
                Err(ControlOperationError::RecoveryRequired) => {
                    match runtime.mailbox.get_steering_receipt(&command_id) {
                        Some(held) if held.state != "rejected" => (
                            held.state,
                            Some("journal incomplete".into()),
                            Some(message_id),
                        ),
                        _ => (
                            "unknown".into(),
                            Some("recovery_required".into()),
                            Some(message_id),
                        ),
                    }
                }
                // Nothing was accepted: the sender is told so.
                Err(error) => (
                    "rejected".into(),
                    Some(format!("not queued: {error}")),
                    None,
                ),
            }
        }
        None => {
            let outcome = controller.execute_steer(&runtime.mailbox, command, true);
            let reason = match &outcome.delivery_error {
                Some(error) => Some(format!("not queued: {error}")),
                None => outcome.reason(),
            };
            (
                outcome.state().to_string(),
                reason,
                outcome
                    .steering
                    .as_ref()
                    .map(|steering| steering.message_id.to_string()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_agent::runtime::operations::{
        CallerType, ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationContext,
        OperationJournal, RootNamespaceId, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
    };
    use davinci_agent::{AgentRecord, RunId, RuntimeBus};
    use davinci_tui::davinci::theme::{ColorDepth, Theme};

    fn record(runtime: &RuntimeHandle, id: AgentId, kind: AgentKind, name: &str) -> AgentRecord {
        AgentRecord {
            id,
            run_id: runtime.run_id,
            parent: (kind != AgentKind::Main).then_some(runtime.agent_id),
            kind,
            name: name.into(),
            provider: String::new(),
            model_id: String::new(),
            cwd: ".".into(),
            state: AgentState::Starting,
            task_id: None,
            worktree: None,
            started_ms: 0,
            updated_ms: 0,
            failure_reason: None,
        }
    }

    /// A lead and one running worker; journaled when `workspace` is given.
    fn fixture(workspace: Option<&std::path::Path>) -> (RuntimeHandle, AgentId) {
        let run_id = RunId::new();
        let lead = AgentId::new();
        let mut runtime = RuntimeHandle::new(run_id, lead, RuntimeBus::new());
        if let Some(workspace) = workspace {
            let identity = JournalIdentity::new(
                JournalId::new(),
                WorkspaceIdentity {
                    id: WorkspaceId::new(),
                    binding_version: 1,
                },
            )
            .unwrap();
            let root = RootNamespaceId::new();
            let context = OperationContext {
                journal_id: identity.journal_id,
                root_namespace_id: root,
                session_id: "roster-host-test".into(),
                runtime_run_id: run_id,
                parent_operation_id: None,
                agent_id: lead,
                worker_id: None,
                task_id: None,
                graph: None,
                workspace: identity.workspace.clone(),
                caller: CallerType::ProviderToolCall,
                wire_tool_call_id: None,
            };
            let journal = Arc::new(
                OperationJournal::open(&workspace.join("journal"), identity, root).unwrap(),
            );
            let operations = ToolOperationRuntime::new(
                journal,
                context,
                ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
                workspace,
            )
            .unwrap();
            runtime = runtime.with_operation_runtime(operations);
        }
        let lead_record = record(&runtime, lead, AgentKind::Main, "lead");
        runtime.registry.register_agent(lead_record).unwrap();
        let worker = AgentId::new();
        let worker_record = record(&runtime, worker, AgentKind::Subagent, "scout");
        runtime.registry.register_agent(worker_record).unwrap();
        runtime
            .registry
            .transition(worker, AgentState::Running)
            .unwrap();
        (runtime, worker)
    }

    fn model() -> Model {
        Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 30, true)
    }

    fn typed(model: &mut Model, worker: AgentId, text: &str) {
        model.focus_agent(&worker.to_string());
        model.composer.set_text(text.to_string());
        assert!(model.submit_to_worker());
    }

    fn receipt_state(model: &Model, worker: AgentId) -> (String, Option<String>) {
        let view = &model.roster.views[&worker.to_string()];
        let receipt = view.receipts.back().expect("a receipt");
        (receipt.state.clone(), receipt.id.clone())
    }

    #[test]
    fn tick_lists_workers_not_the_lead_and_copies_the_viewed_activity() {
        let (runtime, worker) = fixture(None);
        runtime.registry.activity().append(
            worker,
            1,
            WorkerActivityKind::ToolStarted,
            Some("c1"),
            "read src/lib.rs",
            None,
        );
        let mut model = model();
        let mut host = RosterHost::default();
        host.tick(&runtime, &mut model);
        assert_eq!(model.roster.workers.len(), 1, "the lead is the main row");
        assert_eq!(model.roster.workers[0].name, "scout");

        model.focus_agent(&worker.to_string());
        host.tick(&runtime, &mut model);
        let view = &model.roster.views[&worker.to_string()];
        assert_eq!(view.lines.len(), 1);
        assert_eq!(view.lines[0].text, "read src/lib.rs");
        // A second pass adds nothing it already has.
        host.tick(&runtime, &mut model);
        assert_eq!(model.roster.views[&worker.to_string()].lines.len(), 1);
    }

    #[test]
    fn a_message_typed_in_a_worker_view_is_journaled_then_queued() {
        let workspace = tempfile::tempdir().unwrap();
        let (runtime, worker) = fixture(Some(workspace.path()));
        let mut model = model();
        let mut host = RosterHost::default();
        typed(&mut model, worker, "check the parser first");
        assert_eq!(receipt_state(&model, worker).0, "sending");

        host.pump(&runtime, &mut model);
        let (state, id) = receipt_state(&model, worker);
        assert_eq!(state, "queued");
        let id: Uuid = id.expect("a stable id").parse().unwrap();
        assert_eq!(runtime.mailbox.pending_count(&worker), 1);
        assert_eq!(
            runtime.mailbox.get_steering_receipt(&id).unwrap().state,
            "queued"
        );
        // The journal holds this very message id: it was admitted before the
        // mailbox took the message, not only logged afterwards.
        fn contains(dir: &std::path::Path, needle: &str) -> bool {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        contains(&path, needle)
                    } else {
                        std::fs::read(&path)
                            .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
                            .unwrap_or(false)
                    }
                })
        }
        assert!(
            contains(&workspace.path().join("journal"), &id.to_string()),
            "the operation journal does not name message {id}"
        );
    }

    #[test]
    fn the_receipt_follows_the_mailbox_to_applied() {
        let (runtime, worker) = fixture(None);
        let mut model = model();
        let mut host = RosterHost::default();
        typed(&mut model, worker, "use the new schema");
        host.pump(&runtime, &mut model);
        assert_eq!(receipt_state(&model, worker).0, "queued");

        let taken = runtime.mailbox.drain(worker, 10);
        assert_eq!(taken.len(), 1);
        host.pump(&runtime, &mut model);
        assert_eq!(receipt_state(&model, worker).0, "delivered");

        let generation = runtime.registry.get_generation(&worker);
        assert!(runtime
            .mailbox
            .mark_applied(&worker, generation, &taken[0].id));
        host.pump(&runtime, &mut model);
        assert_eq!(receipt_state(&model, worker).0, "applied");
    }

    #[test]
    fn a_late_message_to_a_finished_worker_is_rejected_and_never_resurrects_it() {
        let (runtime, worker) = fixture(None);
        runtime
            .registry
            .transition(worker, AgentState::Completed)
            .unwrap();
        let mut model = model();
        let mut host = RosterHost::default();
        typed(&mut model, worker, "one more thing");
        host.pump(&runtime, &mut model);
        let view = &model.roster.views[&worker.to_string()];
        let receipt = view.receipts.back().unwrap();
        assert_eq!(receipt.state, "rejected");
        assert_eq!(receipt.reason.as_deref(), Some(FINISHED_REASON));
        assert_eq!(runtime.mailbox.pending_count(&worker), 0);
        assert_eq!(
            runtime.registry.get(&worker).unwrap().state,
            AgentState::Completed
        );
    }

    #[test]
    fn the_flag_off_leaves_the_roster_empty_and_sends_nothing() {
        let (runtime, worker) = fixture(None);
        let mut model = model();
        model.roster.enabled = false;
        let mut host = RosterHost::default();
        host.tick(&runtime, &mut model);
        assert!(model.roster.workers.is_empty());
        assert!(!model.roster_visible());
        assert_eq!(runtime.mailbox.pending_count(&worker), 0);
    }

    #[test]
    fn the_rpc_and_the_worker_view_share_one_state_machine() {
        let (runtime, worker) = fixture(None);
        // What the RPC does: its own command through the shared function.
        let command = WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: runtime.run_id,
            agent_id: worker,
            generation: runtime.registry.get_generation(&worker),
            task_id: None,
            expected_revision: runtime.registry.get_revision(&worker),
            action: WorkerControlAction::Steer {
                message: "from sdk".into(),
                redirect: true,
            },
        };
        let (rpc_state, _, rpc_id) = steer_command(&runtime, command);
        // What the view does.
        let (view_state, _, view_id) =
            send_to_worker(&runtime, &worker.to_string(), "from tui", Uuid::new_v4());
        assert_eq!(
            (rpc_state.as_str(), view_state.as_str()),
            ("queued", "queued")
        );
        assert_ne!(rpc_id, view_id);
        // Both messages wait in the one mailbox, in the order they were sent.
        let order: Vec<_> = runtime
            .mailbox
            .peek(&worker)
            .into_iter()
            .map(|m| m.content)
            .collect();
        assert_eq!(order, ["from sdk", "from tui"]);
        // A stale generation is refused the same way on both sides.
        let (stale, reason, id) = steer_command(
            &runtime,
            WorkerControlCommand {
                id: Uuid::new_v4(),
                root_run_id: runtime.run_id,
                agent_id: worker,
                generation: 99,
                task_id: None,
                expected_revision: runtime.registry.get_revision(&worker),
                action: WorkerControlAction::Steer {
                    message: "late".into(),
                    redirect: false,
                },
            },
        );
        assert_ne!(stale, "queued", "{reason:?}");
        assert!(id.is_none());
        assert_eq!(runtime.mailbox.pending_count(&worker), 2);
    }

    struct FullDisk;

    impl davinci_agent::runtime::RuntimeSubscriber for FullDisk {
        fn on_event(
            &self,
            _event: &davinci_agent::runtime::RuntimeEventEnvelope,
        ) -> davinci_agent::runtime::RuntimeDecision {
            davinci_agent::runtime::RuntimeDecision::Continue
        }

        fn record_durably(
            &self,
            _event: &davinci_agent::runtime::RuntimeEventEnvelope,
        ) -> Result<(), String> {
            Err("runtime log append failed: no space left on device".into())
        }
    }

    #[test]
    fn a_failed_log_write_shows_not_queued_and_leaves_nothing_behind() {
        for journaled in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let (runtime, worker) = fixture(journaled.then(|| workspace.path()));
            runtime.bus.subscribe_session(Arc::new(FullDisk));
            let mut model = model();
            let mut host = RosterHost::default();
            typed(&mut model, worker, "important change");
            host.pump(&runtime, &mut model);
            let view = &model.roster.views[&worker.to_string()];
            let receipt = view.receipts.back().unwrap();
            assert_eq!(receipt.state, "rejected", "journaled={journaled}");
            let reason = receipt.reason.clone().unwrap_or_default();
            assert!(reason.starts_with("not queued:"), "{reason}");
            assert!(reason.contains("no space left"), "{reason}");
            assert_eq!(runtime.mailbox.pending_count(&worker), 0);
        }
    }
}
