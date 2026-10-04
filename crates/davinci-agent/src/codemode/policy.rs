use super::{CodeModeError, CodeModeMode};
use crate::runtime::{DeclaredEffect, RuntimeCapability};
use std::collections::{BTreeMap, BTreeSet};

/// One Rust-owned exclusion registry; discovery and dispatch share this ceiling.
const EXCLUDED: &[&str] = &[
    "codemode",
    "batch",
    "agent",
    "graph_submit",
    "graph_status",
    "workflow_start",
    "workflow_resume",
    "workflow_run",
    "agent_message",
    "agent_stop",
    "task_create",
    "task_update",
    "session_fork",
    "session_switch",
    "session_compact",
    "config",
    "auth",
    "constructor",
    "__proto__",
    "prototype",
    "graph_run",
    "graph_verify",
    "tool_search",
    "todo",
];

pub fn js_name(name: &str) -> String {
    let alias: String = name
        .chars()
        .enumerate()
        .map(|(index, c)| {
            if c.is_ascii_alphabetic() || c == '_' || c == '$' || (index > 0 && c.is_ascii_digit())
            {
                c
            } else {
                '_'
            }
        })
        .collect();
    if alias.is_empty() {
        "_".into()
    } else {
        alias
    }
}

pub struct CapabilityPolicy {
    capabilities: BTreeMap<String, RuntimeCapability>,
}

impl CapabilityPolicy {
    pub fn new(
        mode: CodeModeMode,
        capabilities: Vec<RuntimeCapability>,
    ) -> Result<Self, CodeModeError> {
        let mut admitted = BTreeMap::new();
        let mut aliases = BTreeSet::new();
        for capability in capabilities {
            if matches!(mode, CodeModeMode::Off)
                || EXCLUDED.contains(&capability.name.as_str())
                || (matches!(mode, CodeModeMode::ReadOnly) && !capability.read_only)
            {
                continue;
            }
            if capability.declared_effects.iter().any(|effect| {
                matches!(
                    effect,
                    DeclaredEffect::HostInteraction | DeclaredEffect::Other(_)
                )
            }) {
                continue;
            }
            if matches!(mode, CodeModeMode::ReadOnly)
                && capability.declared_effects.iter().any(|effect| {
                    matches!(
                        effect,
                        DeclaredEffect::FileSystemWrite
                            | DeclaredEffect::ProcessExecution
                            | DeclaredEffect::ExternalServicePublish
                            | DeclaredEffect::McpMutation
                    )
                })
            {
                continue;
            }
            let alias = js_name(&capability.name);
            if alias.is_empty() || EXCLUDED.contains(&alias.as_str()) {
                continue;
            }
            if capability.name.len() > 1024
                || !aliases.insert(alias)
                || admitted.contains_key(&capability.name)
            {
                return Err(CodeModeError::new(
                    "UNAVAILABLE",
                    "ambiguous capability identity",
                ));
            }
            admitted.insert(capability.name.clone(), capability);
        }
        if admitted.len() > 1024 {
            return Err(CodeModeError::new(
                "LIMIT_EXCEEDED",
                "capability catalog exceeds limit",
            ));
        }
        Ok(Self {
            capabilities: admitted,
        })
    }

    pub fn tools(&self) -> Vec<&RuntimeCapability> {
        self.capabilities.values().collect()
    }

    pub fn resolve(&self, canonical_name: &str) -> Result<&RuntimeCapability, CodeModeError> {
        self.capabilities
            .get(canonical_name)
            .ok_or_else(|| CodeModeError::new("DENIED", "capability is outside the run ceiling"))
    }
}
