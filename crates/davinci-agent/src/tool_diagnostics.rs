//! Bounded, content-free diagnostics. These observations never authorize or block a tool.

use crate::{Agent, PermissionVerdict, RunStats, ToolResult};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, path::Path, sync::Mutex};

const MAX_RECENT: usize = 256;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;
type Fingerprint = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    Search,
}

#[derive(Debug)]
struct Observation {
    key: Fingerprint,
    actor: Fingerprint,
    call: Fingerprint,
    output: Fingerprint,
}

#[derive(Debug, Default)]
struct State {
    recent: VecDeque<Observation>,
    comparable: u64,
    unknown: u64,
    reads: u64,
    searches: u64,
    worker_duplicates: u64,
}

/// Shared within one runtime root. Stores hashes only; not a durable evidence store.
#[derive(Debug, Default)]
pub(crate) struct ReadDiagnostics {
    state: Mutex<State>,
    overhead: crate::stats::ElapsedWork,
}

impl ReadDiagnostics {
    pub(crate) fn fold_into(&self, stats: &mut RunStats) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.comparable == 0 && state.unknown == 0 {
            return;
        }
        stats.diagnostic_comparable_operations = Some(state.comparable);
        stats.diagnostic_unknown_operations = Some(state.unknown);
        stats.repeated_reads = Some(state.reads);
        stats.repeated_searches = Some(state.searches);
        stats.worker_duplicate_operations = Some(state.worker_duplicates);
        stats.diagnostics_ms = self.overhead.millis();
    }

    fn observe(
        &self,
        kind: Kind,
        observation: Option<Observation>,
        visible: impl Fn(&Observation) -> bool,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(observation) = observation else {
            state.unknown = state.unknown.saturating_add(1);
            return;
        };
        state.comparable = state.comparable.saturating_add(1);
        let repeated = state.recent.iter().any(|previous| {
            previous.key == observation.key
                && previous.actor == observation.actor
                && previous.output == observation.output
                && visible(previous)
        });
        if repeated {
            let count = if kind == Kind::Read {
                &mut state.reads
            } else {
                &mut state.searches
            };
            *count = count.saturating_add(1);
        }
        if state.recent.iter().any(|previous| {
            previous.key == observation.key
                && previous.actor != observation.actor
                && previous.output == observation.output
        }) {
            state.worker_duplicates = state.worker_duplicates.saturating_add(1);
        }
        if state.recent.len() == MAX_RECENT {
            state.recent.pop_front();
        }
        state.recent.push_back(observation);
    }
}

pub(crate) struct ReadProbe {
    kind: Kind,
    key: Option<Fingerprint>,
}

fn hash(value: impl AsRef<[u8]>) -> Fingerprint {
    Sha256::digest(value.as_ref()).into()
}

impl Agent {
    pub(crate) fn read_diagnostics(&self) -> &ReadDiagnostics {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.read_diagnostics.as_ref())
            .unwrap_or(&self.counters.read_diagnostics)
    }

    pub(crate) fn begin_read_diagnostic(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Option<ReadProbe> {
        let kind = match name {
            "read" => Kind::Read,
            "grep" | "find" | "ls" => Kind::Search,
            _ => return None,
        };
        let _timing = self.read_diagnostics().overhead.start();
        Some(ReadProbe {
            kind,
            key: self.read_diagnostic_key(cwd, name, args),
        })
    }

    fn read_diagnostic_key(&self, cwd: &Path, name: &str, args: &Value) -> Option<Fingerprint> {
        // Directory searches, aliases, large files and unknown revisions remain unknown.
        // Do not scan a repository or read additional targets merely to collect telemetry.
        if !matches!(name, "read" | "grep") {
            return None;
        }
        let path = Path::new(args.get("path")?.as_str()?);
        let root = cwd.canonicalize().ok()?;
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let relative = target.strip_prefix(&root).ok()?;
        let permitted = || {
            let permissions = self
                .permissions
                .lock()
                .map_err(|_| crate::runtime::cache::CacheError::Denied)?;
            if matches!(
                permissions.decide(
                    "read-diagnostic",
                    "read",
                    &serde_json::json!({"path": target}),
                    cwd
                ),
                PermissionVerdict::Allow
            ) {
                Ok(())
            } else {
                Err(crate::runtime::cache::CacheError::Denied)
            }
        };
        let before = self.read_permission_fingerprint()?;
        let source =
            crate::runtime::cache::read_current_file(&root, relative, MAX_SOURCE_BYTES, permitted)
                .ok()?;
        if self.read_permission_fingerprint()? != before {
            return None;
        }
        let contract = self.active_contract();
        let scope = contract.as_ref().map(|contract| {
            (
                contract.revision,
                &contract.writable_paths,
                &contract.protected_paths,
                &contract.artifact_write_roots,
                &contract.external_effects,
            )
        });
        let identity =
            serde_json::to_vec(&(root, name, args, before, source.content_hash, scope)).ok()?;
        Some(hash(identity))
    }

    fn read_permission_fingerprint(&self) -> Option<Fingerprint> {
        let permissions = self.permissions.lock().ok()?;
        let revision = permissions.revision()?;
        // The policy has no wire format; Debug covers all fields and never leaves memory.
        Some(hash(format!("{revision}:{:?}", &*permissions)))
    }

    pub(crate) fn finish_read_diagnostic(
        &self,
        probe: Option<ReadProbe>,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &Value,
        result: &ToolResult,
    ) {
        let Some(probe) = probe else {
            return;
        };
        let diagnostics = self.read_diagnostics();
        let _timing = diagnostics.overhead.start();
        let observation = probe
            .key
            .filter(|key| {
                !result.is_error && Some(*key) == self.read_diagnostic_key(cwd, name, args)
            })
            .map(|key| Observation {
                key,
                actor: hash(
                    self.runtime
                        .as_ref()
                        .map(|rt| rt.agent_id.to_string())
                        .unwrap_or_default(),
                ),
                call: hash(id),
                output: hash(&result.content),
            });
        diagnostics.observe(probe.kind, observation, |previous| {
            // Active context paging may replace a body. Without a current provider-view
            // receipt, conservatively decline the repeated-visible-evidence claim.
            self.context_vm_mode() != crate::runtime::ContextVmMode::Active
                && self.messages.iter().any(|message| {
                    message.role == "toolResult" && message.is_error != Some(true)
                        && !message.extra_bool("excludeFromContext")
                        && message.tool_call_id.as_ref().is_some_and(|id| {
                            !self.pruned_tool_results.contains(id) && hash(id) == previous.call
                        })
                        && matches!(message.content.as_slice(), [davinci_ai::MessageContent::Text { text }] if hash(text) == previous.output)
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Agent, AgentId, PermissionMode, RunId, RuntimeBus, RuntimeHandle};
    use davinci_ai::ChatMessage;
    use serde_json::{json, Value};
    use std::path::Path;

    fn call(agent: &mut Agent, root: &Path, id: &str, name: &str, args: Value) {
        let result = agent.run_prepared_call(root, id, name, &args, 0);
        assert!(!result.is_error, "{}", result.content);
        agent
            .messages
            .push(ChatMessage::tool_result(id, name, result.content, false));
    }

    #[test]
    fn harness_waste_counters_require_matching_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "first\nsecond\n").unwrap();
        let mut agent = Agent::new("diagnostic fixture");
        let read = json!({"path":"a.txt"});
        call(&mut agent, dir.path(), "r1", "read", read.clone());
        call(&mut agent, dir.path(), "r2", "read", read.clone());
        assert_eq!(
            serde_json::to_value(agent.run_stats()).unwrap()["repeatedReads"],
            1
        );
        std::fs::write(dir.path().join("a.txt"), "other\nsecond\n").unwrap();
        call(&mut agent, dir.path(), "r3", "read", read.clone());
        call(
            &mut agent,
            dir.path(),
            "r4",
            "read",
            json!({"path":"a.txt", "offset":2}),
        );
        agent.set_permission_mode(PermissionMode::ReadOnly);
        call(&mut agent, dir.path(), "r5", "read", read.clone());
        agent.messages.clear();
        call(&mut agent, dir.path(), "r6", "read", read.clone());
        agent.pruned_tool_results.insert("r6".into());
        call(&mut agent, dir.path(), "r7", "read", read);
        assert_eq!(
            serde_json::to_value(agent.run_stats()).unwrap()["repeatedReads"],
            1
        );
        let search = json!({"path":"a.txt", "pattern":"second"});
        call(&mut agent, dir.path(), "g1", "grep", search.clone());
        call(&mut agent, dir.path(), "g2", "grep", search);
        call(
            &mut agent,
            dir.path(),
            "g3",
            "grep",
            json!({"path":".", "pattern":"second"}),
        );
        let stats = serde_json::to_value(agent.run_stats()).unwrap();
        assert_eq!(stats["repeatedSearches"], 1);
        assert_eq!(stats["diagnosticUnknownOperations"], 1);
        assert_eq!(stats["diagnosticComparableOperations"], 9);
        assert!(stats["diagnosticsMs"].is_u64());
    }

    #[test]
    fn harness_worker_duplication_is_shared_but_separate_from_visible_repetition() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "same\n").unwrap();
        let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        let mut child = runtime.clone();
        child.parent_agent_id = Some(runtime.agent_id);
        child.agent_id = AgentId::new();
        let mut lead = Agent::new("lead").with_runtime(runtime);
        let mut worker = Agent::new("worker").with_runtime(child);
        call(
            &mut lead,
            dir.path(),
            "lead-read",
            "read",
            json!({"path":"a.txt"}),
        );
        call(
            &mut worker,
            dir.path(),
            "worker-read",
            "read",
            json!({"path":"a.txt"}),
        );
        let stats = serde_json::to_value(lead.run_stats()).unwrap();
        assert_eq!(stats["workerDuplicateOperations"], 1);
        assert_eq!(stats["repeatedReads"], 0);
        assert_eq!(stats["diagnosticComparableOperations"], 2);
        // Merely inspecting counters cannot create another observation.
        assert_eq!(serde_json::to_value(lead.run_stats()).unwrap(), stats);
    }

    #[test]
    fn harness_diagnostics_are_bounded_and_unknown_sources_never_repeat() {
        let diagnostics = ReadDiagnostics::default();
        for value in 0..(MAX_RECENT + 2) {
            diagnostics.observe(
                Kind::Read,
                Some(Observation {
                    key: hash(value.to_le_bytes()),
                    actor: hash("actor"),
                    call: hash(value.to_le_bytes()),
                    output: hash("result"),
                }),
                |_| true,
            );
        }
        let state = diagnostics.state.lock().unwrap();
        assert_eq!(state.recent.len(), MAX_RECENT);
        assert_eq!(
            state.recent.front().unwrap().key,
            hash(2_usize.to_le_bytes())
        );
        drop(state);
        for _ in 0..2 {
            diagnostics.observe(Kind::Read, None, |_| true);
        }
        let mut stats = RunStats::default();
        diagnostics.fold_into(&mut stats);
        assert_eq!(stats.repeated_reads, Some(0));
        assert_eq!(stats.diagnostic_unknown_operations, Some(2));
    }

    #[test]
    fn harness_diagnostics_do_not_read_denied_or_unbounded_sources() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("large.txt"),
            vec![b'x'; MAX_SOURCE_BYTES + 1],
        )
        .unwrap();
        let agent = Agent::new("bounded diagnostic");
        assert!(agent
            .read_diagnostic_key(dir.path(), "read", &json!({"path":"large.txt"}))
            .is_none());
        std::fs::write(dir.path().join("small.txt"), "small").unwrap();
        agent
            .permissions
            .lock()
            .unwrap()
            .deny
            .push(crate::PermissionRule::bare("read"));
        assert!(agent
            .read_diagnostic_key(
                dir.path(),
                "grep",
                &json!({"path":"small.txt", "pattern":"small"})
            )
            .is_none());
        // Unknown/failed observations cannot become a retry veto.
        let result = agent.run_prepared_call(
            dir.path(),
            "allowed-grep",
            "grep",
            &json!({"path":"small.txt", "pattern":"small"}),
            0,
        );
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(agent.run_stats().diagnostic_unknown_operations, Some(1));
    }
}
