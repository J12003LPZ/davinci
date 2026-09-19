//! Universal runtime capability registry for tools across built-ins, JS plugins, native extensions, and MCP.
//!
//! Provides a unified schema and capability ledger so subagents, graph workers, and workflows
//! inspect verified tool metadata, enforce least-privilege scoping deterministically, and compute
//! provider-safe prompt-cache keys using capability schema/version hashes.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::permission::{tool_class, ToolClass};

/// The origin of a tool capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    Builtin,
    JsExtension,
    NativeExtension,
    Mcp,
}

impl std::fmt::Display for CapabilitySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilitySource::Builtin => write!(f, "builtin"),
            CapabilitySource::JsExtension => write!(f, "js_extension"),
            CapabilitySource::NativeExtension => write!(f, "native_extension"),
            CapabilitySource::Mcp => write!(f, "mcp"),
        }
    }
}

/// Whether calls may share a scheduler lane with adjacent calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyPolicy {
    ParallelSafe,
    SerialBarrier,
}

impl Default for ConcurrencyPolicy {
    fn default() -> Self {
        Self::SerialBarrier
    }
}

/// Whether a terminal result may be replayed automatically after recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayPolicy {
    SafeToReplay,
    ReconcileBeforeReplay,
    NeverAutoReplay,
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self::NeverAutoReplay
    }
}

/// Whether a tool result may enter the Governor's compressible output path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputPolicy {
    Normal,
    Compressible,
    LosslessRequired,
}

impl Default for OutputPolicy {
    fn default() -> Self {
        Self::Normal
    }
}

/// Return conservative execution properties for a capability at registration
/// time. Unknown names remain serial, non-replayable, and uncompressed.
pub fn output_policy_for_tool(name: &str, tool_class: ToolClass) -> OutputPolicy {
    if matches!(
        name,
        "read"
            | "edit"
            | "write"
            | "notebook_edit"
            | "batch"
            | "todo"
            | "agent"
            | "retrieve_output"
            | "memory_search"
            | "graph_submit"
    ) {
        return OutputPolicy::LosslessRequired;
    }
    if matches!(
        tool_class,
        ToolClass::Read | ToolClass::Edit | ToolClass::Shell | ToolClass::Network
    ) {
        OutputPolicy::Compressible
    } else {
        OutputPolicy::Normal
    }
}

/// Return conservative execution properties for a capability at registration
/// time. Unknown names remain serial and non-replayable; output policy is
/// derived from the same lossless contract used by the Token Governor.
pub fn default_execution_policies(
    name: &str,
    _source: CapabilitySource,
    read_only: bool,
    tool_class: ToolClass,
) -> (ConcurrencyPolicy, ReplayPolicy, OutputPolicy) {
    let output_policy = output_policy_for_tool(name, tool_class);
    let (concurrency, replay) = if name == "agent" {
        (
            ConcurrencyPolicy::ParallelSafe,
            ReplayPolicy::NeverAutoReplay,
        )
    } else if matches!(name, "web_search" | "web_fetch") {
        (
            ConcurrencyPolicy::ParallelSafe,
            ReplayPolicy::ReconcileBeforeReplay,
        )
    } else if name == "tool_search" {
        (ConcurrencyPolicy::ParallelSafe, ReplayPolicy::SafeToReplay)
    } else if matches!(name, "process_status" | "process_output" | "process_list") {
        (
            ConcurrencyPolicy::ParallelSafe,
            ReplayPolicy::ReconcileBeforeReplay,
        )
    } else if matches!(
        name,
        "bash"
            | "powershell"
            | "exec_command"
            | "write_stdin"
            | "write"
            | "edit"
            | "apply_patch"
            | "notebook_edit"
            | "todo"
            | "update_plan"
            | "batch"
            | "propose_plan"
            | "ask_user_question"
            | "job_kill"
            | "agent_message"
            | "agent_stop"
            | "task_create"
            | "task_update"
    ) {
        (
            ConcurrencyPolicy::SerialBarrier,
            if matches!(name, "write" | "edit" | "apply_patch" | "notebook_edit") {
                ReplayPolicy::ReconcileBeforeReplay
            } else {
                ReplayPolicy::NeverAutoReplay
            },
        )
    } else if matches!(
        name,
        "read" | "grep" | "find" | "ls" | "mcp_read" | "job_output"
    ) || (read_only && matches!(tool_class, ToolClass::Read | ToolClass::Network))
    {
        (ConcurrencyPolicy::ParallelSafe, ReplayPolicy::SafeToReplay)
    } else {
        (
            ConcurrencyPolicy::SerialBarrier,
            ReplayPolicy::NeverAutoReplay,
        )
    };
    (concurrency, replay, output_policy)
}

/// Conservative fallback used when a runtime registry has no record for a
/// tool. It intentionally does not infer safety from an untrusted name.
pub fn conservative_replay_policy(name: &str) -> ReplayPolicy {
    default_execution_policies(
        name,
        CapabilitySource::NativeExtension,
        false,
        ToolClass::Other,
    )
    .1
}

/// Declared execution effect of a tool capability.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredEffect {
    FileSystemRead,
    FileSystemWrite,
    ProcessExecution,
    NetworkAccess,
    ExternalServicePublish,
    McpRead,
    McpMutation,
    HostInteraction,
    Other(String),
}

impl std::fmt::Display for DeclaredEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileSystemRead => write!(f, "filesystem_read"),
            Self::FileSystemWrite => write!(f, "filesystem_write"),
            Self::ProcessExecution => write!(f, "process_execution"),
            Self::NetworkAccess => write!(f, "network_access"),
            Self::ExternalServicePublish => write!(f, "external_service_publish"),
            Self::McpRead => write!(f, "mcp_read"),
            Self::McpMutation => write!(f, "mcp_mutation"),
            Self::HostInteraction => write!(f, "host_interaction"),
            Self::Other(s) => write!(f, "other({s})"),
        }
    }
}

/// Computes the default declared effects for a builtin or registered tool.
pub fn default_declared_effects(name: &str, class: ToolClass) -> Vec<DeclaredEffect> {
    match name {
        "read" | "grep" | "find" | "ls" => vec![DeclaredEffect::FileSystemRead],
        "write" | "edit" | "notebook_edit" | "apply_patch" => {
            vec![DeclaredEffect::FileSystemWrite]
        }
        "bash" | "powershell" | "exec_command" | "write_stdin" | "process_start"
        | "process_write" | "process_stop" => {
            vec![DeclaredEffect::ProcessExecution]
        }
        "web_fetch" | "web_search" => vec![DeclaredEffect::NetworkAccess],
        "mcp_read" => vec![DeclaredEffect::McpRead],
        "ask_user_question" => vec![DeclaredEffect::HostInteraction],
        _ => match class {
            ToolClass::Read => vec![DeclaredEffect::FileSystemRead],
            ToolClass::Edit => vec![DeclaredEffect::FileSystemWrite],
            ToolClass::Shell => vec![DeclaredEffect::ProcessExecution],
            ToolClass::Network => vec![DeclaredEffect::NetworkAccess],
            ToolClass::Other => vec![DeclaredEffect::Other("other".into())],
        },
    }
}

/// A canonical representation of an action prepared for execution,
/// validated against contracts and runtime capability ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedAction {
    pub tool: String,
    pub targets: Vec<String>,
    pub declared_effects: Vec<DeclaredEffect>,
    #[serde(default)]
    pub contract_digest: Option<String>,
    #[serde(default)]
    pub owner_generation: Option<u64>,
    #[serde(default)]
    pub source_manifest: Option<String>,
}

impl PreparedAction {
    pub fn new(
        tool: impl Into<String>,
        targets: Vec<String>,
        declared_effects: Vec<DeclaredEffect>,
    ) -> Self {
        Self {
            tool: tool.into(),
            targets,
            declared_effects,
            contract_digest: None,
            owner_generation: None,
            source_manifest: None,
        }
    }

    pub fn with_contract(
        mut self,
        digest: impl Into<String>,
        owner_generation: Option<u64>,
    ) -> Self {
        self.contract_digest = Some(digest.into());
        self.owner_generation = owner_generation;
        self
    }

    pub fn with_source_manifest(mut self, manifest: impl Into<String>) -> Self {
        self.source_manifest = Some(manifest.into());
        self
    }
}

/// A unified capability record representing an executable tool in the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapability {
    pub name: String,
    pub source: CapabilitySource,
    pub tool_class: ToolClass,
    pub read_only: bool,
    pub schema_hash: String,
    pub version: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
    #[serde(default)]
    pub declared_effects: Vec<DeclaredEffect>,
    #[serde(default)]
    pub concurrency_policy: ConcurrencyPolicy,
    #[serde(default)]
    pub replay_policy: ReplayPolicy,
    #[serde(default)]
    pub output_policy: OutputPolicy,
}

impl RuntimeCapability {
    pub fn new(
        name: impl Into<String>,
        source: CapabilitySource,
        tool_class: ToolClass,
        read_only: bool,
        schema: &serde_json::Value,
        version: Option<String>,
    ) -> Self {
        let name_str = name.into();
        let declared_effects = default_declared_effects(&name_str, tool_class);
        let (concurrency_policy, replay_policy, output_policy) =
            default_execution_policies(&name_str, source, read_only, tool_class);
        Self {
            name: name_str,
            source,
            tool_class,
            read_only,
            schema_hash: compute_schema_hash(schema),
            version,
            description: String::new(),
            schema: Some(schema.clone()),
            declared_effects,
            concurrency_policy,
            replay_policy,
            output_policy,
        }
    }

    pub fn with_raw_hash(
        name: impl Into<String>,
        source: CapabilitySource,
        tool_class: ToolClass,
        read_only: bool,
        schema_hash: impl Into<String>,
        version: Option<String>,
    ) -> Self {
        let name_str = name.into();
        let declared_effects = default_declared_effects(&name_str, tool_class);
        let (concurrency_policy, replay_policy, output_policy) =
            default_execution_policies(&name_str, source, read_only, tool_class);
        Self {
            name: name_str,
            source,
            tool_class,
            read_only,
            schema_hash: schema_hash.into(),
            version,
            description: String::new(),
            schema: None,
            declared_effects,
            concurrency_policy,
            replay_policy,
            output_policy,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_declared_effects(mut self, effects: Vec<DeclaredEffect>) -> Self {
        self.declared_effects = effects;
        self
    }
}

/// Provider-facing visibility for schemas that are authorized but initially deferred.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExposureState {
    visible: BTreeSet<String>,
}

impl ToolExposureState {
    pub fn new<I, S>(initial: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            visible: initial.into_iter().map(Into::into).collect(),
        }
    }

    pub fn is_visible(&self, name: &str) -> bool {
        self.visible.contains(name)
    }

    pub fn activate_authorized(&mut self, name: &str, authorized: bool) -> bool {
        if authorized {
            self.visible.insert(name.to_string())
        } else {
            self.visible.remove(name);
            false
        }
    }

    pub fn retain_authorized(&mut self, authorized: &BTreeSet<String>) {
        self.visible.retain(|name| authorized.contains(name));
    }

    pub fn visible_names(&self) -> &BTreeSet<String> {
        &self.visible
    }
}

/// Compute a deterministic SHA-256 hash of a tool schema definition.
pub fn compute_schema_hash(schema: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    let json_bytes = serde_json::to_vec(schema).unwrap_or_default();
    hasher.update(&json_bytes);
    format!("{:x}", hasher.finalize())
}

/// Returns builtin tool capabilities for all builtins in `tool_specs()`.
pub fn builtin_capabilities() -> Vec<RuntimeCapability> {
    let specs = crate::tools::tool_specs();
    specs
        .into_iter()
        .map(|tool| {
            let class = tool_class(&tool.name);
            let read_only = matches!(class, ToolClass::Read | ToolClass::Network);
            RuntimeCapability::new(
                tool.name,
                CapabilitySource::Builtin,
                class,
                read_only,
                &tool.parameters,
                Some(format!("builtin-{}", env!("CARGO_PKG_VERSION"))),
            )
            .with_description(tool.description)
        })
        .collect()
}

/// Thread-safe registry containing all active runtime capabilities.
#[derive(Debug, Clone)]
pub struct RuntimeCapabilityRegistry {
    capabilities: Arc<RwLock<HashMap<String, RuntimeCapability>>>,
}

impl Default for RuntimeCapabilityRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl RuntimeCapabilityRegistry {
    /// Create an empty capability registry.
    pub fn new() -> Self {
        Self {
            capabilities: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a capability registry pre-populated with all builtin tools.
    pub fn with_builtins() -> Self {
        let registry = Self::new();
        registry.register_all(builtin_capabilities());
        registry
    }

    /// Register a single capability.
    pub fn register(&self, capability: RuntimeCapability) {
        let mut caps = self
            .capabilities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        caps.insert(capability.name.clone(), capability);
    }

    /// Register multiple capabilities.
    pub fn register_all(&self, capabilities: impl IntoIterator<Item = RuntimeCapability>) {
        let mut caps = self
            .capabilities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for cap in capabilities {
            caps.insert(cap.name.clone(), cap);
        }
    }

    /// Unregister a capability by name.
    pub fn unregister(&self, name: &str) -> Option<RuntimeCapability> {
        self.capabilities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(name)
    }

    /// Retrieve a capability by name.
    pub fn get(&self, name: &str) -> Option<RuntimeCapability> {
        self.capabilities
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(name)
            .cloned()
    }

    /// Check if a capability is known to be read-only.
    /// Invariant: Unknown capabilities remain mutation-capable by default (returns false).
    pub fn is_read_only(&self, name: &str) -> bool {
        self.get(name).map(|c| c.read_only).unwrap_or(false)
    }

    /// Check if a capability is mutating.
    /// Invariant: Unknown capabilities remain mutation-capable by default (returns true).
    pub fn is_mutating(&self, name: &str) -> bool {
        !self.is_read_only(name)
    }

    /// List all registered capabilities sorted by name.
    pub fn list(&self) -> Vec<RuntimeCapability> {
        let caps = self
            .capabilities
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut list: Vec<_> = caps.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Scope tools based on an optional allowlist and whether mutation is allowed.
    /// Unknown capabilities are treated as mutating by default.
    pub fn scope_tools(
        &self,
        requested: Option<&[String]>,
        available: &[String],
        allow_mutation: bool,
    ) -> Vec<String> {
        let mut result = Vec::new();
        for tool in available {
            if let Some(req) = requested {
                if !req.contains(tool) {
                    continue;
                }
            }
            if !allow_mutation && self.is_mutating(tool) {
                continue;
            }
            result.push(tool.clone());
        }
        result
    }

    /// Compute a stable hash representing schemas and versions of the specified tools.
    pub fn hash_tool_capabilities(&self, tool_names: &[impl AsRef<str>]) -> String {
        let mut sorted: Vec<&str> = tool_names.iter().map(|s| s.as_ref()).collect();
        sorted.sort();
        sorted.dedup();

        let mut hasher = Sha256::new();
        hasher.update(b"capabilities_v1\n");
        for name in sorted {
            hasher.update(name.as_bytes());
            hasher.update(b":");
            if let Some(cap) = self.get(name) {
                hasher.update(cap.schema_hash.as_bytes());
                hasher.update(b":");
                if let Some(ver) = &cap.version {
                    hasher.update(ver.as_bytes());
                }
            } else {
                hasher.update(b"unknown_mutation");
            }
            hasher.update(b"\n");
        }
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_builtin_capabilities_registered() {
        let registry = RuntimeCapabilityRegistry::with_builtins();
        assert!(registry.get("read").is_some());
        assert!(registry.get("write").is_some());
        assert!(registry.is_read_only("read"));
        assert!(!registry.is_read_only("write"));
        assert!(registry.is_mutating("write"));
        assert!(!registry.is_mutating("read"));

        // Unknown capability defaults to mutating
        assert!(!registry.is_read_only("unregistered_custom_tool"));
        assert!(registry.is_mutating("unregistered_custom_tool"));
        let ask = registry.get("ask_user_question").unwrap();
        assert_eq!(ask.declared_effects, vec![DeclaredEffect::HostInteraction]);
    }

    #[test]
    fn test_tool_scoping_with_registry() {
        let registry = RuntimeCapabilityRegistry::with_builtins();
        let available = vec![
            "read".to_string(),
            "write".to_string(),
            "grep".to_string(),
            "custom_mutation".to_string(),
        ];

        // 1. All tools requested, mutations allowed
        let scoped = registry.scope_tools(None, &available, true);
        assert_eq!(scoped, available);

        // 2. Read-only scoping (mutation disallowed)
        let scoped_ro = registry.scope_tools(None, &available, false);
        assert_eq!(scoped_ro, vec!["read".to_string(), "grep".to_string()]);

        // 3. Requested filter + read-only
        let requested = vec!["read".to_string(), "write".to_string()];
        let scoped_filtered = registry.scope_tools(Some(&requested), &available, false);
        assert_eq!(scoped_filtered, vec!["read".to_string()]);
    }

    #[test]
    fn test_hash_tool_capabilities_stability_and_version() {
        let registry = RuntimeCapabilityRegistry::new();
        let cap1 = RuntimeCapability::new(
            "test_tool",
            CapabilitySource::Mcp,
            ToolClass::Read,
            true,
            &json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            Some("1.0.0".into()),
        );
        registry.register(cap1);

        let hash1 = registry.hash_tool_capabilities(&["test_tool"]);

        // Same tool, same schema/version -> identical hash
        let hash2 = registry.hash_tool_capabilities(&["test_tool"]);
        assert_eq!(hash1, hash2);

        // Update version -> hash changes
        let cap1_v2 = RuntimeCapability::new(
            "test_tool",
            CapabilitySource::Mcp,
            ToolClass::Read,
            true,
            &json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            Some("2.0.0".into()),
        );
        registry.register(cap1_v2);
        let hash3 = registry.hash_tool_capabilities(&["test_tool"]);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn representative_tools_have_consistent_execution_metadata() {
        let registry = RuntimeCapabilityRegistry::with_builtins();
        registry.register(RuntimeCapability::new(
            "mcp__memory__echo",
            CapabilitySource::Mcp,
            ToolClass::Read,
            true,
            &json!({"type": "object"}),
            None,
        ));

        let cases = [
            (
                "read",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::SafeToReplay,
                OutputPolicy::LosslessRequired,
            ),
            (
                "edit",
                ConcurrencyPolicy::SerialBarrier,
                ReplayPolicy::ReconcileBeforeReplay,
                OutputPolicy::LosslessRequired,
            ),
            (
                "exec_command",
                ConcurrencyPolicy::SerialBarrier,
                ReplayPolicy::NeverAutoReplay,
                OutputPolicy::Compressible,
            ),
            (
                "web_search",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::ReconcileBeforeReplay,
                OutputPolicy::Compressible,
            ),
            (
                "tool_search",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::SafeToReplay,
                OutputPolicy::Compressible,
            ),
            (
                "agent",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::NeverAutoReplay,
                OutputPolicy::LosslessRequired,
            ),
            (
                "mcp__memory__echo",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::SafeToReplay,
                OutputPolicy::Compressible,
            ),
        ];

        for (name, concurrency, replay, output) in cases {
            let capability = registry.get(name).unwrap();
            assert_eq!(capability.concurrency_policy, concurrency, "{name}");
            assert_eq!(capability.replay_policy, replay, "{name}");
            assert_eq!(capability.output_policy, output, "{name}");
        }
    }

    #[test]
    fn unknown_tool_defaults_to_serial_nonreplayable_mutation() {
        let capability = RuntimeCapability::with_raw_hash(
            "unknown_tool",
            CapabilitySource::NativeExtension,
            ToolClass::Other,
            false,
            "unknown",
            None,
        );

        assert!(!capability.read_only);
        assert_eq!(
            capability.concurrency_policy,
            ConcurrencyPolicy::SerialBarrier
        );
        assert_eq!(capability.replay_policy, ReplayPolicy::NeverAutoReplay);
        assert_eq!(capability.output_policy, OutputPolicy::Normal);
    }
}
