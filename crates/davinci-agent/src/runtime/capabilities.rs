//! Universal runtime capability registry for tools across built-ins, JS plugins, native extensions, and MCP.
//!
//! Provides a unified schema and capability ledger so subagents, graph workers, and workflows
//! inspect verified tool metadata, enforce least-privilege scoping deterministically, and compute
//! provider-safe prompt-cache keys using capability schema/version hashes.

use std::collections::HashMap;
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

/// A unified capability record representing an executable tool in the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapability {
    pub name: String,
    pub source: CapabilitySource,
    pub tool_class: ToolClass,
    pub read_only: bool,
    pub schema_hash: String,
    pub version: Option<String>,
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
        Self {
            name: name.into(),
            source,
            tool_class,
            read_only,
            schema_hash: compute_schema_hash(schema),
            version,
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
        Self {
            name: name.into(),
            source,
            tool_class,
            read_only,
            schema_hash: schema_hash.into(),
            version,
        }
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
            let schema_hash = compute_schema_hash(&tool.parameters);
            RuntimeCapability {
                name: tool.name,
                source: CapabilitySource::Builtin,
                tool_class: class,
                read_only,
                schema_hash,
                version: Some(format!("builtin-{}", env!("CARGO_PKG_VERSION"))),
            }
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
}
