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

/// Droid-compatible script aliases, in preference order per alias. The alias
/// only ever names a capability that is already admitted: it never adds one.
const SCRIPT_ALIASES: &[(&str, &[&str])] = &[
    ("Read", &["read"]),
    ("Grep", &["grep"]),
    ("Glob", &["find"]),
    ("LS", &["ls"]),
    ("Execute", &["exec_command", "bash", "powershell"]),
    ("ApplyPatch", &["apply_patch"]),
    ("Edit", &["edit"]),
    ("Write", &["write"]),
];

/// Resolve a Droid-compatible `tools.<Alias>` name to the canonical capability
/// the run already admitted. Aliasing never creates authority.
pub fn resolve_script_alias(
    name: &str,
    mode: CodeModeMode,
    registry: &CapabilityPolicy,
) -> Result<String, CodeModeError> {
    if matches!(mode, CodeModeMode::Off) {
        return Err(CodeModeError::new(
            "DENIED",
            "script aliases require an enabled Codemode mode",
        ));
    }
    let alias = name.strip_prefix("tools.").ok_or_else(|| {
        CodeModeError::new("DENIED", "script alias must use the tools.<Name> spelling")
    })?;
    if registry
        .script_alias_collisions()
        .iter()
        .any(|collision| collision == alias)
    {
        return Err(CodeModeError::new(
            "DENIED",
            "script alias collides with a discovered tool",
        ));
    }
    registry
        .aliases
        .get(&format!("tools.{alias}"))
        .cloned()
        .ok_or_else(|| CodeModeError::new("DENIED", "unsupported script alias"))
}

/// Whether a declared effect class may be dispatched at all under `mode`.
/// Aliasing and script arguments never widen this ceiling.
pub fn effect_is_admitted(mode: &CodeModeMode, effect: &DeclaredEffect) -> bool {
    match mode {
        CodeModeMode::Off => false,
        CodeModeMode::ReadOnly => matches!(
            effect,
            DeclaredEffect::FileSystemRead
                | DeclaredEffect::NetworkAccess
                | DeclaredEffect::McpRead
        ),
        // Controlled admits the same ceiling `CapabilityPolicy::new` admits:
        // host interaction and unknown effects stay out, everything the
        // ordinary authorizer guards may run.
        CodeModeMode::Controlled => !matches!(
            effect,
            DeclaredEffect::HostInteraction | DeclaredEffect::Other(_)
        ),
    }
}

/// Explicit host opt-ins for tools a Script may otherwise never reach. Both
/// default off: a script can never widen its own catalog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScriptToolGates {
    pub allow_todo: bool,
    pub allow_subagents: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptSubagentPolicy {
    pub allow: bool,
    pub depth: u32,
    pub max_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSubagentRequest {
    pub description: String,
}

/// Admit one child agent from inside a Script. Off unless the host enabled it,
/// bounded by depth and the parent's remaining budget.
pub fn admit_script_subagent(
    request: &ScriptSubagentRequest,
    policy: &ScriptSubagentPolicy,
    budget_remaining: usize,
) -> Result<(), CodeModeError> {
    if !policy.allow {
        return Err(CodeModeError::new(
            "DENIED",
            "subagents inside a Script require explicit feature approval",
        ));
    }
    if request.description.trim().is_empty() || request.description.len() > 4096 {
        return Err(CodeModeError::new(
            "INVALID_INPUT",
            "subagent brief must be non-empty and bounded",
        ));
    }
    if policy.depth >= policy.max_depth {
        return Err(CodeModeError::new("DENIED", "subagent depth limit reached"));
    }
    if budget_remaining == 0 {
        return Err(CodeModeError::new(
            "LIMIT_EXCEEDED",
            "parent worker budget is exhausted",
        ));
    }
    Ok(())
}

/// Tools excluded from every Script catalog unless an explicit host gate opens
/// them. `todo` stays out by default: a script must not gain bookkeeping
/// authority implicitly.
fn is_excluded(name: &str, gates: ScriptToolGates) -> bool {
    EXCLUDED.contains(&name) && !(name == "todo" && gates.allow_todo)
}

pub struct CapabilityPolicy {
    capabilities: BTreeMap<String, RuntimeCapability>,
    /// Admissible tools excluded because their JS aliases collide.
    collisions: BTreeSet<String>,
    /// Frozen `alias -> canonical` snapshot for this Script's lifetime.
    aliases: BTreeMap<String, String>,
    /// Aliases withheld because a discovered tool already owns the name.
    alias_collisions: BTreeSet<String>,
}

impl CapabilityPolicy {
    pub fn new(
        mode: CodeModeMode,
        capabilities: Vec<RuntimeCapability>,
    ) -> Result<Self, CodeModeError> {
        Self::new_with_gates(mode, capabilities, ScriptToolGates::default())
    }

    pub fn new_with_gates(
        mode: CodeModeMode,
        capabilities: Vec<RuntimeCapability>,
        gates: ScriptToolGates,
    ) -> Result<Self, CodeModeError> {
        let mut admitted = BTreeMap::new();
        let mut aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for capability in capabilities {
            if matches!(mode, CodeModeMode::Off)
                || is_excluded(&capability.name, gates)
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
            if alias.is_empty() || is_excluded(&alias, gates) {
                continue;
            }
            if capability.name.len() > 1024 || admitted.contains_key(&capability.name) {
                return Err(CodeModeError::new(
                    "UNAVAILABLE",
                    "ambiguous capability identity",
                ));
            }
            aliases
                .entry(alias)
                .or_default()
                .push(capability.name.clone());
            admitted.insert(capability.name.clone(), capability);
        }
        // Tools whose JS aliases collide are unreachable by name, so exclude
        // every member of the collision. The rest of the catalog stays usable,
        // and the exclusion is reported rather than silent.
        let mut collisions = BTreeSet::new();
        for names in aliases.values().filter(|names| names.len() > 1) {
            for name in names {
                admitted.remove(name);
                collisions.insert(name.clone());
            }
        }
        if admitted.len() > 1024 {
            return Err(CodeModeError::new(
                "LIMIT_EXCEEDED",
                "capability catalog exceeds limit",
            ));
        }
        // Snapshot the alias binding once, from the admitted catalog. A name a
        // discovered tool already uses is ambiguous and stays unbound.
        let mut script_aliases = BTreeMap::new();
        let mut alias_collisions = BTreeSet::new();
        let todo_alias: &[(&str, &[&str])] = if gates.allow_todo {
            &[("TodoWrite", &["todo"])]
        } else {
            &[]
        };
        for (alias, candidates) in SCRIPT_ALIASES.iter().chain(todo_alias.iter()) {
            if aliases.contains_key(*alias) {
                alias_collisions.insert((*alias).to_string());
                continue;
            }
            if let Some(canonical) = candidates
                .iter()
                .find(|candidate| admitted.contains_key(**candidate))
            {
                script_aliases.insert(format!("tools.{alias}"), (*canonical).to_string());
            }
        }
        Ok(Self {
            capabilities: admitted,
            collisions,
            aliases: script_aliases,
            alias_collisions,
        })
    }

    pub fn tools(&self) -> Vec<&RuntimeCapability> {
        self.capabilities.values().collect()
    }

    /// Frozen alias snapshot for this Script, `tools.<Alias> -> canonical`.
    pub fn script_aliases(&self) -> BTreeMap<String, String> {
        self.aliases.clone()
    }

    /// Canonical names of tools the given script alias resolves to.
    pub fn resolve_alias(&self, alias: &str) -> Result<&RuntimeCapability, CodeModeError> {
        if self.alias_collisions.contains(alias) {
            return Err(CodeModeError::new(
                "DENIED",
                "script alias collides with a discovered tool",
            ));
        }
        let canonical = self
            .aliases
            .get(alias)
            .ok_or_else(|| CodeModeError::new("DENIED", "unsupported script alias"))?;
        self.resolve(canonical)
    }

    /// Canonical names excluded because their script aliases collide.
    pub fn alias_collisions(&self) -> Vec<String> {
        self.collisions.iter().cloned().collect()
    }

    /// Droid alias names withheld because a discovered tool owns the name.
    pub fn script_alias_collisions(&self) -> Vec<String> {
        self.alias_collisions.iter().cloned().collect()
    }

    pub fn resolve(&self, canonical_name: &str) -> Result<&RuntimeCapability, CodeModeError> {
        if self.collisions.contains(canonical_name) {
            return Err(CodeModeError::new(
                "DENIED",
                "capability script name collides with another tool",
            ));
        }
        self.capabilities
            .get(canonical_name)
            .ok_or_else(|| CodeModeError::new("DENIED", "capability is outside the run ceiling"))
    }
}
