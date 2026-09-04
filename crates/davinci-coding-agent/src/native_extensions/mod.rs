//! Native Rust ports of the bundled pi extensions.

pub mod graph;
mod security_scan;
mod token_governor;
pub mod vector_memory;

pub use graph::*;
pub use security_scan::*;
pub use token_governor::*;
pub use vector_memory::*;

use davinci_agent::{ToolError, ToolResult};
use serde_json::{json, Value};
use std::path::Path;

pub const NATIVE_TOOLS: &[&str] = &[
    "memory_search",
    "retrieve_output",
    "graph_status",
    "graph_run",
    "sec_scan_start",
    "sec_scan_context",
    "sec_scan_progress",
    "sec_scan_draft",
    "sec_scan_complete",
    "sec_scan_cancel",
    "sec_candidates_record",
    "sec_candidates_list",
    "sec_candidates_validate",
    "sec_candidates_attack_path",
    "sec_scope_files",
    "sec_policy_resolve",
    "sec_tracking_validate",
    "sec_deep_scan",
];

pub const NATIVE_COMMANDS: &[&str] = &[
    "memory-status",
    "memory-search",
    "memory-reindex",
    "memory-clear",
    "governor-status",
    "governor-reset",
    "graph",
    "graph-resume",
    "graph-status",
    "graph-view",
    "graph-abort",
    "sec-status",
    "sec-report",
    "sec-abort",
];

/// Metadata shared by the interactive and RPC command discovery surfaces.
/// Native commands are Rust implementations, but they should look like any
/// other invocable pi command to clients and autocomplete.
pub fn command_specs() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    vec![
        (
            "memory-status",
            "Show vector-memory health and record counts.",
            None,
        ),
        (
            "memory-search",
            "Search durable vector and lexical memory.",
            Some("<query>"),
        ),
        (
            "memory-reindex",
            "Reload the local vector-memory index.",
            None,
        ),
        ("memory-clear", "Clear local vector-memory records.", None),
        (
            "governor-status",
            "Show token-governor compression and deduplication counters.",
            None,
        ),
        (
            "governor-reset",
            "Reset token-governor session state.",
            None,
        ),
        (
            "graph",
            "Run a coding task as an execution graph of isolated workers.",
            Some("<goal> [--simple|--complex] [--dry-run]"),
        ),
        (
            "graph-resume",
            "Resume an unfinished graph run, reusing its completed workers.",
            Some("[runId]"),
        ),
        ("graph-status", "Show active and recent graph runs.", None),
        (
            "graph-view",
            "Tail a graph worker's live transcript.",
            Some("[taskId]"),
        ),
        ("graph-abort", "Abort the active graph-engineer run.", None),
        ("sec-status", "Show the active security scan status.", None),
        ("sec-report", "Show the active security scan report.", None),
        ("sec-abort", "Cancel the active security scan.", None),
    ]
}

pub fn native_invocable_commands() -> Vec<Value> {
    command_specs()
        .into_iter()
        .map(|(name, description, argument_hint)| {
            let mut command = json!({
                "name": name,
                "description": description,
                "source": "native",
                "sourceInfo": {"runtime": "rust"},
            });
            if let Some(argument_hint) = argument_hint {
                command["argumentHint"] = Value::String(argument_hint.to_string());
            }
            command
        })
        .collect()
}

/// Set inside a graph worker child process, absent in an ordinary session.
/// It gates the `graph_submit` tool and the per-role tool and shell policy.
pub fn graph_worker_context() -> Option<GraphWorkerContext> {
    GraphWorkerContext::from_env()
}

#[derive(Debug, Clone, Default)]
pub struct NativeExtensionHost {
    pub governor: TokenGovernor,
    pub memory: VectorMemory,
    pub graph: GraphController,
    pub security: SecurityScanController,
}

impl NativeExtensionHost {
    pub fn new_with_agent_dir(
        session_key: impl Into<String>,
        cwd: &Path,
        agent_dir: Option<&Path>,
    ) -> Self {
        let session_key = session_key.into();
        let governor_config = agent_dir
            .map(|dir| TokenGovernorConfig::from_file(&dir.join("token-governor.json")))
            .unwrap_or_else(TokenGovernorConfig::from_env);
        let memory_config = agent_dir
            .map(|dir| VectorMemoryConfig::from_file(&dir.join("vector-memory.json")))
            .unwrap_or_else(VectorMemoryConfig::from_env);
        let governor = TokenGovernor::new(session_key.clone(), governor_config);
        // Only the product host sweeps: other sessions' stored outputs past
        // the retention window go, never the live session's.
        let _ = governor.sweep_stale_outputs();
        Self {
            governor,
            memory: VectorMemory::with_config(cwd.to_path_buf(), memory_config),
            graph: GraphController::new(cwd.to_path_buf()),
            security: SecurityScanController::new(cwd.to_path_buf()),
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = NATIVE_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        if graph_worker_context().is_some() {
            names.push(GRAPH_SUBMIT_TOOL.to_string());
        }
        names
    }

    /// `state_hash` is only evaluated for the tools whose ledger needs it
    /// (it runs git), so ordinary reads and edits never pay for it.
    pub fn before_tool(
        &mut self,
        name: &str,
        args: &Value,
        state_hash: impl FnOnce() -> String,
    ) -> Option<String> {
        // Inside a graph worker, least privilege is enforced here as well as by
        // the child's --tools allowlist: "bash" is one tool whose danger lives
        // in its command text.
        if let Some(context) = graph_worker_context() {
            if let Some(reason) = context.block_reason(name, args) {
                return Some(reason);
            }
        }
        self.governor.before_tool(name, args, state_hash)
    }

    pub fn after_tool(&mut self, name: &str, args: &Value, result: ToolResult) -> ToolResult {
        self.governor.after_tool(name, args, result)
    }

    pub fn memory_inject(&self, query: &str) -> Option<String> {
        self.memory.inject(query)
    }

    pub fn memory_index_messages(
        &mut self,
        messages: &[MemoryMessage],
    ) -> Result<usize, ToolError> {
        self.memory.index_messages(messages)
    }

    pub fn session_start(&mut self) {
        self.governor.session_start();
        self.memory.session_start();
    }

    pub fn session_compact(&mut self) {
        self.governor.session_compact();
        self.memory.session_compact();
    }

    /// A background graph run must not outlive the session that started it.
    pub fn session_shutdown(&mut self) {
        graph::abort_all_runs();
    }

    pub fn execute_tool(
        &mut self,
        _cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        match name {
            "memory_search" => self.memory.search_tool(args),
            "retrieve_output" => self.governor.retrieve(args),
            name if name.starts_with("sec_") => self.security.execute_tool(name, args),
            "graph_status" | "graph_run" | GRAPH_SUBMIT_TOOL => self.graph.execute_tool(name, args),
            _ => Err(ToolError::Unknown(name.to_string())),
        }
    }

    pub fn command(&mut self, name: &str, args: &str) -> Result<Option<Value>, String> {
        match name {
            "memory-status" => Ok(Some(self.memory.status())),
            "memory-search" => Ok(Some(self.memory.search_text(args))),
            "memory-reindex" => Ok(Some(self.memory.reindex().map_err(|err| err.to_string())?)),
            "memory-clear" => Ok(Some(self.memory.clear().map_err(|err| err.to_string())?)),
            "governor-status" => Ok(Some(self.governor.status())),
            "governor-reset" => {
                self.governor.reset();
                Ok(Some(self.governor.status()))
            }
            name if name.starts_with("graph") => self.graph.command(name, args),
            name if name.starts_with("sec-") => self.security.command(name, args),
            _ => Ok(None),
        }
    }

    pub fn describe_tool(name: &str) -> Option<davinci_ai::ToolSpec> {
        let (description, parameters) = match name {
            "memory_search" => (
                "Search durable vector and lexical memory for supporting context.",
                json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":20}},"required":["query"]}),
            ),
            "retrieve_output" => (
                "Retrieve a lossless full or ranged tool output saved by token governor.",
                json!({"type":"object","properties":{"id":{"type":"string","pattern":"^out-[0-9a-f]{12}$"},"startLine":{"type":"integer","minimum":1},"endLine":{"type":"integer","minimum":1},"grep":{"type":"string"}},"required":["id"]}),
            ),
            "graph_status" => (
                "Inspect the active and recent graph runs in this project.",
                json!({"type":"object","properties":{"runId":{"type":"string"}}}),
            ),
            "graph_run" => (
                "Solve a coding task as an execution graph of isolated, least-privileged worker processes (classify, research, plan, implement, verify, review) and return the outcome. Runs to completion, which can take a long time.",
                json!({"type":"object","properties":{"goal":{"type":"string","minLength":1},"mode":{"type":"string","enum":["simple","complex"]},"dryRun":{"type":"boolean"}},"required":["goal"]}),
            ),
            name if name.starts_with("sec_") => security_scan::tool_spec(name),
            // Only a graph worker child sees this tool; it is the worker's one
            // exit door and its schema names the artifact that node owes.
            GRAPH_SUBMIT_TOOL => {
                let context = graph_worker_context()?;
                let (description, parameters) = context.tool_spec();
                return Some(davinci_ai::ToolSpec {
                    name: name.to_string(),
                    description,
                    parameters,
                    constrained_sampling: None,
                });
            }
            _ => return None,
        };
        Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            constrained_sampling: None,
        })
    }

    pub fn tool_specs() -> Vec<davinci_ai::ToolSpec> {
        NATIVE_TOOLS
            .iter()
            .copied()
            .chain(graph_worker_context().map(|_| GRAPH_SUBMIT_TOOL))
            .filter_map(Self::describe_tool)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// PI_GRAPH_* is process-global, so the tests that toggle it run one at a time.
    static GRAPH_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct WorkerEnv;

    impl WorkerEnv {
        fn set(artifact_path: &Path) -> Self {
            std::env::set_var("PI_GRAPH_ROLE", "reviewer");
            std::env::set_var("PI_GRAPH_EXPECT", "review");
            std::env::set_var("PI_GRAPH_ARTIFACT_PATH", artifact_path);
            std::env::set_var("PI_GRAPH_EXTRA_TOOLS", "read,graph_submit");
            Self
        }
    }

    impl Drop for WorkerEnv {
        fn drop(&mut self) {
            for key in [
                "PI_GRAPH_ROLE",
                "PI_GRAPH_EXPECT",
                "PI_GRAPH_ARTIFACT_PATH",
                "PI_GRAPH_EXTRA_TOOLS",
            ] {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn graph_submit_exists_only_inside_a_worker_process() {
        let _lock = GRAPH_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("artifact.json");

        let host = NativeExtensionHost::default();
        assert!(!host
            .tool_names()
            .iter()
            .any(|name| name == GRAPH_SUBMIT_TOOL));
        assert!(!NativeExtensionHost::tool_specs()
            .iter()
            .any(|spec| spec.name == GRAPH_SUBMIT_TOOL));

        let _env = WorkerEnv::set(&artifact);
        let host = NativeExtensionHost::default();
        assert!(host
            .tool_names()
            .iter()
            .any(|name| name == GRAPH_SUBMIT_TOOL));
        let spec = NativeExtensionHost::tool_specs()
            .into_iter()
            .find(|spec| spec.name == GRAPH_SUBMIT_TOOL)
            .expect("graph_submit is offered to a worker");
        assert!(spec.description.contains("final review artifact"));
    }

    #[test]
    fn a_worker_submits_and_is_policed_through_the_native_host() {
        let _lock = GRAPH_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _submit = graph::worker_hooks::submit_test_guard();
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("artifact.json");
        let _env = WorkerEnv::set(&artifact);
        let mut host = NativeExtensionHost::default();

        // The reviewer may run tests but never mutate the repository.
        assert_eq!(
            host.before_tool("bash", &json!({"command": "cargo test"}), || "hash".into()),
            None
        );
        assert!(host
            .before_tool("write", &json!({"path": "src/lib.rs"}), || "hash".into())
            .is_some());
        assert!(host
            .before_tool("bash", &json!({"command": "rm -rf src"}), || "hash".into())
            .is_some());

        let result = host
            .execute_tool(
                dir.path(),
                GRAPH_SUBMIT_TOOL,
                &json!({"artifact": {"verdict": "approve", "issues": [], "notes": "ok"}}),
            )
            .expect("submit accepted");
        assert!(!result.is_error);
        assert!(artifact.is_file(), "the artifact reached disk");

        // Once the artifact is in, the node is done: no more work is accepted.
        assert!(host
            .before_tool("bash", &json!({"command": "cargo test"}), || "hash".into())
            .is_some());
    }

    #[test]
    fn every_native_command_has_discoverable_metadata() {
        let specs = command_specs();
        assert_eq!(specs.len(), NATIVE_COMMANDS.len());
        for (name, description, _) in specs {
            assert!(NATIVE_COMMANDS.contains(&name));
            assert!(!description.is_empty());
        }
    }

    #[test]
    fn native_invocable_commands_identify_rust_runtime() {
        let commands = native_invocable_commands();
        assert_eq!(commands.len(), NATIVE_COMMANDS.len());
        assert!(commands.iter().any(|command| {
            command["name"] == "memory-search"
                && command["source"] == "native"
                && command["argumentHint"] == "<query>"
        }));
    }
}
