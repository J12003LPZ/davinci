//! Native Rust ports of the bundled pi extensions.

mod graph;
mod security_scan;
mod token_governor;
mod vector_memory;

pub use graph::*;
pub use security_scan::*;
pub use token_governor::*;
pub use vector_memory::*;

use pi_agent::{ToolError, ToolResult};
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
            "Start a bounded graph-engineer run.",
            Some("<goal>"),
        ),
        (
            "graph-resume",
            "Resume the latest persisted graph run.",
            None,
        ),
        ("graph-status", "Show graph-engineer run status.", None),
        (
            "graph-view",
            "Show graph-engineer artifacts and task state.",
            None,
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

#[derive(Debug, Clone, Default)]
pub struct NativeExtensionHost {
    pub governor: TokenGovernor,
    pub memory: VectorMemory,
    pub graph: GraphController,
    pub security: SecurityScanController,
}

impl NativeExtensionHost {
    pub fn new(session_key: impl Into<String>, cwd: &Path) -> Self {
        Self::new_with_agent_dir(session_key, cwd, None)
    }

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
        Self {
            governor: TokenGovernor::new(session_key.clone(), governor_config),
            memory: VectorMemory::with_config(cwd.to_path_buf(), memory_config),
            graph: GraphController::new(cwd.to_path_buf()),
            security: SecurityScanController::new(cwd.to_path_buf()),
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        NATIVE_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    }

    pub fn command_names(&self) -> Vec<String> {
        NATIVE_COMMANDS
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    }

    pub fn before_tool(&mut self, name: &str, args: &Value, state_hash: &str) -> Option<String> {
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
            "graph_status" | "graph_run" => self.graph.execute_tool(name, args),
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
                self.governor.session_start();
                Ok(Some(self.governor.status()))
            }
            name if name.starts_with("graph") => self.graph.command(name, args),
            name if name.starts_with("sec-") => self.security.command(name, args),
            _ => Ok(None),
        }
    }

    pub fn describe_tool(name: &str) -> Option<pi_ai::ToolSpec> {
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
                "Inspect the current graph run status.",
                json!({"type":"object","properties":{"runId":{"type":"string"}}}),
            ),
            "graph_run" => (
                "Create or resume a bounded, dependency-validated graph run.",
                json!({"type":"object","properties":{"goal":{"type":"string","minLength":1},"mode":{"type":"string","enum":["simple","complex"]},"dryRun":{"type":"boolean"}},"required":["goal"]}),
            ),
            name if name.starts_with("sec_") => security_scan::tool_spec(name),
            _ => return None,
        };
        Some(pi_ai::ToolSpec {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            constrained_sampling: None,
        })
    }

    pub fn tool_specs() -> Vec<pi_ai::ToolSpec> {
        NATIVE_TOOLS
            .iter()
            .filter_map(|name| Self::describe_tool(name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
