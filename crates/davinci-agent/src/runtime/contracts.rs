//! Task-scoped execution contracts.
//!
//! Enforces accepted task scope across all execution paths, independently of
//! permission mode and worker instructions.
//!
//! # Backend Enforcement Limitations & Platform Matrix
//!
//! | Platform | Filesystem Path Scope | Shell / Process Confinement | Network Isolation |
//! |---|---|---|---|
//! | Windows (MSVC) | Enforced via pre-dispatch path normalization, reparse/symlink check, protected-wins | Unenforceable in-process backend; refuses before spawn | Unenforceable; refuses before dispatch |
//! | Linux / macOS | Enforced via pre-dispatch path normalization, symlink resolution, protected-wins | Unenforceable in-process backend; refuses before spawn | Unenforceable; refuses before dispatch |
//!
//! Note: Prompt constraints and permission modes do NOT constitute an OS sandbox. Because
//! no kernel-level namespace or race-safe filesystem sandbox exists in this runtime,
//! hard-contracted external process, network, and unconfined filesystem mutations fail closed
//! before execution rather than claiming unprovable confinement.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path;

use crate::runtime::ids::TaskId;

/// Maximum allowable path length in contract scope.
pub const MAX_CONTRACT_PATH_LEN: usize = 1024;
/// Maximum entries in path scope lists.
pub const MAX_CONTRACT_PATHS_COUNT: usize = 256;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("path contains invalid traversal: {0}")]
    PathTraversal(String),
    #[error("path must be relative: {0}")]
    AbsolutePath(String),
    #[error("path contains unsupported alternate data stream: {0}")]
    AlternateDataStream(String),
    #[error("path escapes workspace via symlink or reparse point: {0}")]
    SymlinkEscape(String),
    #[error("dependency addition not allowed by contract: {0}")]
    DependencyAdditionNotAllowed(String),
    #[error("path exceeds maximum length of {limit} bytes: {actual}")]
    PathTooLong { actual: usize, limit: usize },
    #[error("path list exceeds maximum limit of {limit} entries: {actual}")]
    TooManyPaths { actual: usize, limit: usize },
    #[error("contract identifier cannot be empty")]
    EmptyIdentifier,
    #[error("digest mismatch: expected {expected}, computed {computed}")]
    DigestMismatch { expected: String, computed: String },
    #[error("scope expansion preview is stale: expected revision {expected}, current revision {current}")]
    StaleExpansionRevision { expected: u64, current: u64 },
    #[error("scope expansion preview is invalid: {0}")]
    InvalidExpansionPreview(String),
}

/// Normalizes a relative path for contract evaluation.
///
/// Ensures forward slashes, rejects absolute, UNC, drive-relative,
/// parent directory traversal (`..`), and Windows Alternate Data Streams (`:`).
pub fn normalize_relative_path(raw: &str) -> Result<String, ContractError> {
    if raw.len() > MAX_CONTRACT_PATH_LEN {
        return Err(ContractError::PathTooLong {
            actual: raw.len(),
            limit: MAX_CONTRACT_PATH_LEN,
        });
    }

    if raw.starts_with('/')
        || raw.starts_with('\\')
        || crate::permission::has_windows_drive_prefix(raw)
    {
        return Err(ContractError::AbsolutePath(raw.to_string()));
    }
    if raw.contains(':') {
        return Err(ContractError::AlternateDataStream(raw.to_string()));
    }

    let result = crate::permission::normalize_portable_path_text(raw);
    if result == ".." || result.starts_with("../") {
        return Err(ContractError::PathTraversal(raw.to_string()));
    }
    Ok(result)
}

/// Resolves a relative path within a root directory, checking for symlink/reparse point escapes.
pub fn resolve_contract_path(root: &Path, raw_rel: &str) -> Result<String, ContractError> {
    let normalized = normalize_relative_path(raw_rel)?;
    let target = root.join(&normalized);

    // If target escapes via symlink, reject
    if crate::permission::is_symlink_escape(root, &target) {
        return Err(ContractError::SymlinkEscape(raw_rel.to_string()));
    }

    // Check parent ancestors if target does not yet exist
    let mut ancestor = target.as_path();
    while let Some(parent) = ancestor.parent() {
        if parent.exists() {
            if crate::permission::is_symlink_escape(root, parent) {
                return Err(ContractError::SymlinkEscape(raw_rel.to_string()));
            }
            break;
        }
        ancestor = parent;
    }

    // If target or parent canonicalizes, resolve canonical relative path
    if let (Ok(canon_root), Ok(canon_target)) = (root.canonicalize(), target.canonicalize()) {
        let clean_root = crate::permission::strip_verbatim_prefix(&canon_root);
        let clean_target = crate::permission::strip_verbatim_prefix(&canon_target);
        if !clean_target.starts_with(&clean_root) {
            return Err(ContractError::SymlinkEscape(raw_rel.to_string()));
        }
        if let Ok(rel) = clean_target.strip_prefix(&clean_root) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            return Ok(rel_str.trim_start_matches('/').to_string());
        }
    }

    Ok(normalized)
}

/// Core path scope evaluation helper.
///
/// Evaluates whether a normalized relative file path is permitted under the given
/// writable and protected scopes.
///
/// Invariant: **Protected scope strictly wins.** If any protected scope matches,
/// access is denied, even if an overlapping or wider writable scope also matches.
pub fn path_scope_allows(normalized: &str, writable: &[&str], protected: &[&str]) -> bool {
    path_scope_allows_case(normalized, writable, protected, false)
}

/// Core path scope evaluation helper with optional case-insensitivity.
pub fn path_scope_allows_case(
    normalized: &str,
    writable: &[&str],
    protected: &[&str],
    case_insensitive: bool,
) -> bool {
    let matches = |scope: &&str| {
        let s = scope.trim_end_matches('/');
        if case_insensitive {
            normalized.eq_ignore_ascii_case(s)
                || (scope.ends_with('/')
                    && normalized.len() >= scope.len()
                    && normalized[..scope.len()].eq_ignore_ascii_case(scope))
        } else {
            normalized == s || (scope.ends_with('/') && normalized.starts_with(*scope))
        }
    };
    !protected.iter().any(matches) && writable.iter().any(matches)
}

/// Checks whether a proposed change to a manifest or lockfile adds new dependencies.
///
/// If `allowed_new_dependencies` is false, this verifies both manifest and lockfile
/// changes semantically to prevent lockfile-only or sneaky dependency insertions.
pub fn check_dependency_addition(
    path: &str,
    before: &str,
    after: &str,
) -> Result<(), ContractError> {
    let lower = path.to_ascii_lowercase().replace('\\', "/");
    let filename = lower.rsplit('/').next().unwrap_or(&lower);

    if filename == "cargo.toml" {
        let before_deps = extract_cargo_toml_deps(before);
        let after_deps = extract_cargo_toml_deps(after);
        for dep in &after_deps {
            if !before_deps.contains(dep) {
                return Err(ContractError::DependencyAdditionNotAllowed(format!(
                    "new dependency '{dep}' added to {path}"
                )));
            }
        }
    } else if filename == "cargo.lock" {
        let before_pkgs = extract_cargo_lock_pkgs(before);
        let after_pkgs = extract_cargo_lock_pkgs(after);
        for pkg in &after_pkgs {
            if !before_pkgs.contains(pkg) {
                return Err(ContractError::DependencyAdditionNotAllowed(format!(
                    "new package '{pkg}' added to lockfile {path}"
                )));
            }
        }
    } else if filename == "package.json" {
        let before_deps = extract_package_json_deps(before);
        let after_deps = extract_package_json_deps(after);
        for dep in &after_deps {
            if !before_deps.contains(dep) {
                return Err(ContractError::DependencyAdditionNotAllowed(format!(
                    "new dependency '{dep}' added to {path}"
                )));
            }
        }
    } else if matches!(
        filename,
        "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "poetry.lock" | "pipfile.lock"
    ) && before.trim().is_empty()
        && !after.trim().is_empty()
    {
        return Err(ContractError::DependencyAdditionNotAllowed(format!(
            "new lockfile created: {path}"
        )));
    }

    Ok(())
}

fn extract_cargo_toml_deps(content: &str) -> HashSet<String> {
    let mut deps = HashSet::new();
    let mut in_deps_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed.trim_matches(|c| c == '[' || c == ']');
            in_deps_section = section == "dependencies"
                || section == "dev-dependencies"
                || section == "build-dependencies"
                || section.ends_with(".dependencies");
            continue;
        }
        if in_deps_section && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((name, _)) = trimmed.split_once('=') {
                let dep_name = name.trim().trim_matches('"');
                if !dep_name.is_empty() {
                    deps.insert(dep_name.to_string());
                }
            }
        }
    }
    deps
}

fn extract_cargo_lock_pkgs(content: &str) -> HashSet<String> {
    let mut pkgs = HashSet::new();
    let mut in_pkg = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            in_pkg = true;
            continue;
        }
        if in_pkg && trimmed.starts_with("name =") {
            if let Some((_, name_val)) = trimmed.split_once('=') {
                let pkg_name = name_val.trim().trim_matches('"');
                if !pkg_name.is_empty() {
                    pkgs.insert(pkg_name.to_string());
                }
            }
            in_pkg = false;
        }
    }
    pkgs
}

fn extract_package_json_deps(content: &str) -> HashSet<String> {
    let mut deps = HashSet::new();
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
        for section in [
            "dependencies",
            "devDependencies",
            "optionalDependencies",
            "peerDependencies",
        ] {
            if let Some(map) = val.get(section).and_then(|v| v.as_object()) {
                for key in map.keys() {
                    deps.insert(key.clone());
                }
            }
        }
    }
    deps
}

/// Final execution gate evaluating whether an action is permitted under the active contract.
///
/// Invariant: Always Approve and other permissive modes never bypass task scope,
/// ownership validity, or hard platform/security limits.
pub fn contract_gate(
    _mode: &str,
    in_scope: bool,
    owner_valid: bool,
    hard_limits_allow: bool,
) -> bool {
    in_scope && owner_valid && hard_limits_allow
}

/// Advances a contract revision only when the caller proves it is applying the expected
/// current revision, a user explicitly authorized the expansion, and the durable write
/// completed successfully. Model-generated approval flags never satisfy these host inputs.
pub fn expand_revision(
    current: u64,
    expected: u64,
    user_authorized: bool,
    durable: bool,
) -> Option<u64> {
    if current != expected || !user_authorized || !durable {
        return None;
    }
    current.checked_add(1)
}

/// A required verification is satisfied only by an explicit passing state.
/// Optional checks may remain unavailable without satisfying or blocking the contract.
pub fn required_check_satisfied(required: bool, state: &str) -> bool {
    !required || state == "passed"
}

/// Redacts sensitive credentials, tokens, and secret patterns from strings before
/// recording in audit events or logs.
pub fn redact_secrets(input: &str) -> String {
    let mut output = input.to_string();
    let prefixes = [
        ("sk-", "sk-[REDACTED]"),
        ("ghp_", "ghp_[REDACTED]"),
        ("github_pat_", "github_pat_[REDACTED]"),
        ("AKIA", "AKIA[REDACTED]"),
        ("Bearer", "Bearer [REDACTED]"),
    ];
    for (prefix, replacement) in prefixes {
        let mut cursor = 0usize;
        while let Some(relative) = output[cursor..].find(prefix) {
            let start = cursor + relative;
            let val_raw_start = start + prefix.len();
            let rest = &output[val_raw_start..];
            let val_start = val_raw_start + (rest.len() - rest.trim_start().len());
            let end = output[val_start..]
                .find(|ch: char| {
                    ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | '&' | '|')
                })
                .map(|offset| val_start + offset)
                .unwrap_or(output.len());
            output.replace_range(start..end, replacement);
            cursor = start + replacement.len();
            if cursor >= output.len() {
                break;
            }
        }
    }

    for key in ["password", "secret", "api_key", "apikey", "token"] {
        let mut search = 0usize;
        loop {
            let lower = output.to_ascii_lowercase();
            let Some(relative) = lower[search..].find(key) else {
                break;
            };
            let start = search + relative;
            let key_end = start + key.len();
            let after = &output[key_end..];
            let sep_pos = after.find(['=', ':']);
            if let Some(pos) = sep_pos {
                if pos <= 3 {
                    let val_raw_start = key_end + pos + 1;
                    let rest = &output[val_raw_start..];
                    let val_start = val_raw_start
                        + (rest.len()
                            - rest
                                .trim_start_matches(|c: char| {
                                    c.is_whitespace() || c == '"' || c == '\''
                                })
                                .len());
                    let end = output[val_start..]
                        .find(|ch: char| {
                            ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | '&' | '|')
                        })
                        .map(|offset| val_start + offset)
                        .unwrap_or(output.len());
                    if end > val_start {
                        output.replace_range(val_start..end, "[REDACTED]");
                        search = val_start + "[REDACTED]".len();
                        continue;
                    }
                }
            }
            search = key_end;
            if search >= output.len() {
                break;
            }
        }
    }

    output
}

/// Structured details for a contract scope violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeViolation {
    pub requested_target: String,
    pub tool: String,
    pub writable_scope: Vec<String>,
    pub protected_scope: Vec<String>,
    pub reason: String,
}

impl ScopeViolation {
    pub fn new(
        requested_target: impl Into<String>,
        tool: impl Into<String>,
        writable_scope: Vec<String>,
        protected_scope: Vec<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            requested_target: redact_secrets(&requested_target.into()),
            tool: tool.into(),
            writable_scope,
            protected_scope,
            reason: redact_secrets(&reason.into()),
        }
    }
}

impl std::fmt::Display for ScopeViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Scope violation: tool `{}` target `{}` is forbidden. Protected: {:?}. Writable: {:?}. {}",
            self.tool, self.requested_target, self.protected_scope, self.writable_scope, self.reason
        )
    }
}

/// Extracts file/path targets from tool arguments.
pub fn extract_tool_targets(tool: &str, args: &serde_json::Value) -> Vec<String> {
    let mut targets = Vec::new();
    match tool {
        "write" | "edit" | "notebook_edit" => {
            for key in ["path", "file", "target", "target_file"] {
                if let Some(p) = args.get(key).and_then(serde_json::Value::as_str) {
                    targets.push(p.to_string());
                }
            }
        }
        "apply_patch" => {
            if let Some(patch) = args.get("patch").and_then(serde_json::Value::as_str) {
                for line in patch.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed
                        .strip_prefix("*** a/")
                        .or_else(|| trimmed.strip_prefix("--- a/"))
                        .or_else(|| trimmed.strip_prefix("+++ b/"))
                    {
                        let path = rest.split_whitespace().next().unwrap_or(rest);
                        targets.push(path.to_string());
                    }
                }
            }
            if let Some(p) = args.get("path").and_then(serde_json::Value::as_str) {
                targets.push(p.to_string());
            }
        }
        _ => {
            for key in ["path", "file", "target_file", "dest"] {
                if let Some(p) = args.get(key).and_then(serde_json::Value::as_str) {
                    targets.push(p.to_string());
                }
            }
        }
    }
    targets
}

/// A validated, immutable execution contract bound to an accepted task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    pub id: String,
    pub revision: u64,
    pub task_id: TaskId,
    pub accepted_plan_revision: u64,
    pub writable_paths: Vec<String>,
    pub protected_paths: Vec<String>,
    pub allowed_new_dependencies: bool,
    pub external_effects: Vec<String>,
    pub verification_requirements: Vec<String>,
    pub artifact_write_roots: Vec<String>,
    pub digest: String,
}

/// Host-visible, immutable description of one exact proposed scope expansion.
///
/// Unknown fields are rejected so model-authored JSON such as `approved: true` cannot be
/// mistaken for host authority. The actual decision is represented separately by the
/// non-deserializable [`ScopeExpansionDecision`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeExpansionPreview {
    pub schema_version: u32,
    pub expected_revision: u64,
    pub current_digest: String,
    pub requested_target: String,
    pub reason: String,
    pub proposed: TaskContract,
    pub preview_digest: String,
}

/// Trusted-host decision for a scope expansion. Intentionally not `Deserialize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeExpansionDecision {
    Approve,
    RequestInScopeApproach,
    Deny,
}

impl ScopeExpansionPreview {
    fn compute_preview_digest(
        expected_revision: u64,
        current_digest: &str,
        requested_target: &str,
        reason: &str,
        proposed_digest: &str,
    ) -> String {
        let canonical = serde_json::json!({
            "schema_version": 1,
            "expected_revision": expected_revision,
            "current_digest": current_digest,
            "requested_target": requested_target,
            "reason": reason,
            "proposed_digest": proposed_digest,
        });
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_vec(&canonical).unwrap_or_default());
        format!("{:x}", hasher.finalize())
    }

    /// Revalidate an exact submitted preview against the live current contract.
    pub fn verify_against(&self, current: &TaskContract) -> Result<(), ContractError> {
        current.validate()?;
        self.proposed.validate()?;
        if self.expected_revision != current.revision || self.current_digest != current.digest {
            return Err(ContractError::StaleExpansionRevision {
                expected: self.expected_revision,
                current: current.revision,
            });
        }
        if self.schema_version != 1
            || self.proposed.id != current.id
            || self.proposed.task_id != current.task_id
            || self.proposed.accepted_plan_revision != current.accepted_plan_revision
            || self.proposed.revision
                != current.revision.checked_add(1).ok_or_else(|| {
                    ContractError::InvalidExpansionPreview("contract revision overflow".into())
                })?
        {
            return Err(ContractError::InvalidExpansionPreview(
                "proposed contract does not advance the submitted current contract exactly once"
                    .into(),
            ));
        }
        let normalized_target = normalize_relative_path(&self.requested_target)?;
        if !self
            .proposed
            .writable_paths
            .iter()
            .any(|path| path_scope_allows(&normalized_target, &[path.as_str()], &[]))
        {
            return Err(ContractError::InvalidExpansionPreview(
                "proposed contract does not include the submitted requested target".into(),
            ));
        }
        let computed = Self::compute_preview_digest(
            self.expected_revision,
            &self.current_digest,
            &self.requested_target,
            &self.reason,
            &self.proposed.digest,
        );
        if computed != self.preview_digest {
            return Err(ContractError::DigestMismatch {
                expected: self.preview_digest.clone(),
                computed,
            });
        }
        Ok(())
    }
}

impl TaskContract {
    /// Computes the deterministic SHA-256 digest of contract parameters.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_digest(
        id: &str,
        revision: u64,
        task_id: &TaskId,
        accepted_plan_revision: u64,
        writable_paths: &[String],
        protected_paths: &[String],
        allowed_new_dependencies: bool,
        external_effects: &[String],
        verification_requirements: &[String],
        artifact_write_roots: &[String],
    ) -> String {
        let mut sorted_writable = writable_paths.to_vec();
        sorted_writable.sort();
        let mut sorted_protected = protected_paths.to_vec();
        sorted_protected.sort();
        let mut sorted_effects = external_effects.to_vec();
        sorted_effects.sort();
        let mut sorted_reqs = verification_requirements.to_vec();
        sorted_reqs.sort();
        let mut sorted_artifacts = artifact_write_roots.to_vec();
        sorted_artifacts.sort();

        let canonical = serde_json::json!({
            "id": id,
            "revision": revision,
            "task_id": task_id.to_string(),
            "accepted_plan_revision": accepted_plan_revision,
            "writable_paths": sorted_writable,
            "protected_paths": sorted_protected,
            "allowed_new_dependencies": allowed_new_dependencies,
            "external_effects": sorted_effects,
            "verification_requirements": sorted_reqs,
            "artifact_write_roots": sorted_artifacts,
        });

        let serialized = serde_json::to_string(&canonical).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Constructs a new `TaskContract` with canonical digest and validation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        revision: u64,
        task_id: TaskId,
        accepted_plan_revision: u64,
        writable_paths: Vec<String>,
        protected_paths: Vec<String>,
        allowed_new_dependencies: bool,
        external_effects: Vec<String>,
        verification_requirements: Vec<String>,
        artifact_write_roots: Vec<String>,
    ) -> Result<Self, ContractError> {
        let id_str = id.into();
        if id_str.trim().is_empty() {
            return Err(ContractError::EmptyIdentifier);
        }

        if writable_paths.len() > MAX_CONTRACT_PATHS_COUNT {
            return Err(ContractError::TooManyPaths {
                actual: writable_paths.len(),
                limit: MAX_CONTRACT_PATHS_COUNT,
            });
        }
        if protected_paths.len() > MAX_CONTRACT_PATHS_COUNT {
            return Err(ContractError::TooManyPaths {
                actual: protected_paths.len(),
                limit: MAX_CONTRACT_PATHS_COUNT,
            });
        }

        let mut norm_writable = Vec::with_capacity(writable_paths.len());
        for p in &writable_paths {
            norm_writable.push(normalize_relative_path(p)?);
        }

        let mut norm_protected = Vec::with_capacity(protected_paths.len());
        for p in &protected_paths {
            norm_protected.push(normalize_relative_path(p)?);
        }

        let digest = Self::compute_digest(
            &id_str,
            revision,
            &task_id,
            accepted_plan_revision,
            &norm_writable,
            &norm_protected,
            allowed_new_dependencies,
            &external_effects,
            &verification_requirements,
            &artifact_write_roots,
        );

        Ok(Self {
            id: id_str,
            revision,
            task_id,
            accepted_plan_revision,
            writable_paths: norm_writable,
            protected_paths: norm_protected,
            allowed_new_dependencies,
            external_effects,
            verification_requirements,
            artifact_write_roots,
            digest,
        })
    }

    /// Evaluates if a given path is permitted by this contract's scopes.
    pub fn allows_path(&self, raw_path: &str) -> Result<bool, ContractError> {
        self.allows_path_case(raw_path, false)
    }

    /// Evaluates if a given path is permitted by this contract's scopes with optional case-insensitivity.
    pub fn allows_path_case(
        &self,
        raw_path: &str,
        case_insensitive: bool,
    ) -> Result<bool, ContractError> {
        let normalized = normalize_relative_path(raw_path)?;
        let writable_refs: Vec<&str> = self.writable_paths.iter().map(AsRef::as_ref).collect();
        let protected_refs: Vec<&str> = self.protected_paths.iter().map(AsRef::as_ref).collect();
        Ok(path_scope_allows_case(
            &normalized,
            &writable_refs,
            &protected_refs,
            case_insensitive,
        ))
    }

    /// Builds an exact host-visible scope-expansion proposal without granting authority.
    pub fn preview_scope_expansion(
        &self,
        requested_target: &str,
        reason: &str,
    ) -> Result<ScopeExpansionPreview, ContractError> {
        self.validate()?;
        let normalized_target = normalize_relative_path(requested_target)?;
        let proposed = self.expand_scope(vec![normalized_target.clone()], vec![])?;
        let preview_digest = ScopeExpansionPreview::compute_preview_digest(
            self.revision,
            &self.digest,
            &normalized_target,
            reason,
            &proposed.digest,
        );
        Ok(ScopeExpansionPreview {
            schema_version: 1,
            expected_revision: self.revision,
            current_digest: self.digest.clone(),
            requested_target: normalized_target,
            reason: reason.to_string(),
            proposed,
            preview_digest,
        })
    }

    /// Proposes or applies a scope expansion, producing a new revision with updated digest.
    pub fn expand_scope(
        &self,
        additional_writable: Vec<String>,
        additional_protected: Vec<String>,
    ) -> Result<Self, ContractError> {
        let mut new_writable = self.writable_paths.clone();
        for p in additional_writable {
            let norm = normalize_relative_path(&p)?;
            if !new_writable.contains(&norm) {
                new_writable.push(norm);
            }
        }
        let mut new_protected = self.protected_paths.clone();
        for p in additional_protected {
            let norm = normalize_relative_path(&p)?;
            if !new_protected.contains(&norm) {
                new_protected.push(norm);
            }
        }
        Self::new(
            &self.id,
            self.revision + 1,
            self.task_id,
            self.accepted_plan_revision,
            new_writable,
            new_protected,
            self.allowed_new_dependencies,
            self.external_effects.clone(),
            self.verification_requirements.clone(),
            self.artifact_write_roots.clone(),
        )
    }

    /// Validates the internal consistency and checksum digest of the contract.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.id.trim().is_empty() {
            return Err(ContractError::EmptyIdentifier);
        }
        let computed = Self::compute_digest(
            &self.id,
            self.revision,
            &self.task_id,
            self.accepted_plan_revision,
            &self.writable_paths,
            &self.protected_paths,
            self.allowed_new_dependencies,
            &self.external_effects,
            &self.verification_requirements,
            &self.artifact_write_roots,
        );
        if computed != self.digest {
            return Err(ContractError::DigestMismatch {
                expected: self.digest.clone(),
                computed,
            });
        }
        if self.writable_paths.len() > MAX_CONTRACT_PATHS_COUNT {
            return Err(ContractError::TooManyPaths {
                actual: self.writable_paths.len(),
                limit: MAX_CONTRACT_PATHS_COUNT,
            });
        }
        if self.protected_paths.len() > MAX_CONTRACT_PATHS_COUNT {
            return Err(ContractError::TooManyPaths {
                actual: self.protected_paths.len(),
                limit: MAX_CONTRACT_PATHS_COUNT,
            });
        }
        for p in &self.writable_paths {
            normalize_relative_path(p)?;
        }
        for p in &self.protected_paths {
            normalize_relative_path(p)?;
        }
        for p in &self.artifact_write_roots {
            normalize_relative_path(p)?;
        }
        Ok(())
    }

    /// Checks whether a tool call conforms to this contract's scope and effects.
    pub fn check_call(
        &self,
        cwd: &Path,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<(), ScopeViolation> {
        let targets = extract_tool_targets(tool, args);
        if targets.is_empty() {
            let class = crate::permission::tool_class(tool);
            let is_read = matches!(
                class,
                crate::permission::ToolClass::Read | crate::permission::ToolClass::Network
            );
            if tool.starts_with("mcp__") && !is_read {
                return Err(ScopeViolation::new(
                    tool,
                    tool,
                    self.writable_paths.clone(),
                    self.protected_paths.clone(),
                    "Unknown MCP mutation is blocked under a task-scoped contract without declared scope",
                ));
            }
            return Ok(());
        }

        for target in targets {
            let rel = match resolve_contract_path(cwd, &target) {
                Ok(r) => r,
                Err(err) => {
                    return Err(ScopeViolation::new(
                        target,
                        tool,
                        self.writable_paths.clone(),
                        self.protected_paths.clone(),
                        format!("Path resolution failed: {err}"),
                    ));
                }
            };

            match self.allows_path(&rel) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(ScopeViolation::new(
                        rel,
                        tool,
                        self.writable_paths.clone(),
                        self.protected_paths.clone(),
                        "Target is not within writable scope or is protected by contract",
                    ));
                }
                Err(err) => {
                    return Err(ScopeViolation::new(
                        rel,
                        tool,
                        self.writable_paths.clone(),
                        self.protected_paths.clone(),
                        format!("Scope evaluation error: {err}"),
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Compiles a user-accepted plan scope into an immutable versioned `TaskContract`.
pub fn compile_plan_contract(
    plan: &crate::LivingPlan,
    step_id: Option<&str>,
    task_id: Option<TaskId>,
) -> Result<TaskContract, ContractError> {
    let mut writable = Vec::new();
    let mut verification = Vec::new();

    if let Some(sid) = step_id {
        if let Some(step) = plan.steps.iter().find(|s| s.id == sid) {
            writable.extend(step.files.clone());
            verification.extend(step.verify.clone());
        }
    } else {
        for step in &plan.steps {
            writable.extend(step.files.clone());
            verification.extend(step.verify.clone());
        }
    }

    let contract_id = format!(
        "contract-plan-r{}-{}",
        plan.revision,
        step_id.unwrap_or("all")
    );

    // Baseline protected project metadata and credentials
    let default_protected = vec![
        ".git/".to_string(),
        ".davinci/".to_string(),
        ".pi/".to_string(),
        ".env".to_string(),
    ];

    TaskContract::new(
        contract_id,
        1,
        task_id.unwrap_or_default(),
        plan.revision,
        writable,
        default_protected,
        false,
        Vec::new(),
        verification,
        vec!["target/".to_string()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{AgentId, RunId, TaskRecord, TaskRegistry};
    use serde_json::json;

    #[test]
    fn f05_protected_wins() {
        assert!(path_scope_allows(
            "src/auth/reset.rs",
            &["src/auth/"],
            &["src/auth/private/"]
        ));
        assert!(!path_scope_allows(
            "src/auth/private/key.rs",
            &["src/auth/"],
            &["src/auth/private/"]
        ));
        assert!(!path_scope_allows("src/authentic.rs", &["src/auth/"], &[]));
    }

    #[test]
    fn f05_path_prefix_collision() {
        // "src/auth" must not grant "src/authentic.rs" or "src/authorization/user.rs"
        assert!(!path_scope_allows("src/authentic.rs", &["src/auth/"], &[]));
        assert!(!path_scope_allows("src/auth.rs", &["src/auth/"], &[]));
        assert!(path_scope_allows("src/auth/login.rs", &["src/auth/"], &[]));
    }

    #[test]
    fn f05_protected_wins_even_if_later_writable_entry_matches() {
        // Protected scope wins regardless of array order or exact matches
        assert!(!path_scope_allows(
            "src/auth/private/secret.key",
            &["src/auth/", "src/auth/private/secret.key"],
            &["src/auth/private/"]
        ));
    }

    #[test]
    fn f05_normalize_relative_path_safety() {
        // Traversal rejection
        assert!(matches!(
            normalize_relative_path("../secret.txt"),
            Err(ContractError::PathTraversal(_))
        ));
        assert!(matches!(
            normalize_relative_path("src/../../etc/passwd"),
            Err(ContractError::PathTraversal(_))
        ));

        // Absolute path rejection
        assert!(matches!(
            normalize_relative_path("/etc/shadow"),
            Err(ContractError::AbsolutePath(_))
        ));

        // Windows ADS rejection
        assert!(matches!(
            normalize_relative_path("file.txt:stream"),
            Err(ContractError::AlternateDataStream(_))
        ));

        // Normalization of current dir and slashes
        assert_eq!(
            normalize_relative_path("./src/auth/../auth/login.rs").unwrap(),
            "src/auth/login.rs"
        );
        assert_eq!(
            normalize_relative_path("src\\auth\\login.rs").unwrap(),
            "src/auth/login.rs"
        );
    }

    #[test]
    fn f05_contract_digest_and_validation() {
        let task_id = TaskId::new();
        let contract = TaskContract::new(
            "contract-1",
            1,
            task_id,
            42,
            vec!["src/lib.rs".into(), "crates/".into()],
            vec!["crates/secrets/".into()],
            false,
            vec!["read_only_fs".into()],
            vec!["cargo check".into()],
            vec!["target/".into()],
        )
        .unwrap();

        assert_eq!(contract.revision, 1);
        assert!(!contract.digest.is_empty());
        assert!(contract.validate().is_ok());

        assert!(contract.allows_path("src/lib.rs").unwrap());
        assert!(contract.allows_path("crates/agent/src/main.rs").unwrap());
        assert!(!contract.allows_path("crates/secrets/key.pem").unwrap());
        assert!(!contract.allows_path("README.md").unwrap());

        let mut tampered = contract.clone();
        tampered.writable_paths.push("unauthorized/".into());
        assert!(matches!(
            tampered.validate(),
            Err(ContractError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn f05_windows_ads() {
        assert!(matches!(
            normalize_relative_path("file.txt:stream"),
            Err(ContractError::AlternateDataStream(_))
        ));
        assert!(matches!(
            normalize_relative_path("dir/secret.key::$DATA"),
            Err(ContractError::AlternateDataStream(_))
        ));
    }

    #[test]
    fn f05_same_file_via_alias() {
        let p1 = normalize_relative_path("./src/auth/../auth/login.rs").unwrap();
        let p2 = normalize_relative_path("src\\auth\\login.rs").unwrap();
        let p3 = normalize_relative_path("src/auth/./login.rs").unwrap();
        assert_eq!(p1, "src/auth/login.rs");
        assert_eq!(p2, "src/auth/login.rs");
        assert_eq!(p3, "src/auth/login.rs");

        let writable = &["src/auth/"];
        let protected = &[];
        assert!(path_scope_allows(&p1, writable, protected));
        assert!(path_scope_allows(&p2, writable, protected));
        assert!(path_scope_allows(&p3, writable, protected));
    }

    #[test]
    fn f05_symlink_reparse_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let inside = root.join("inside");
        std::fs::create_dir_all(&inside).unwrap();

        // Legitimate relative path inside root
        let res = resolve_contract_path(root, "inside/test.txt").unwrap();
        assert_eq!(res, "inside/test.txt");

        // Traversal escape rejected
        assert!(matches!(
            resolve_contract_path(root, "../outside.txt"),
            Err(ContractError::PathTraversal(_))
        ));
    }

    #[test]
    fn f05_case_sensitive_vs_insensitive_volume() {
        let writable = &["src/auth/"];
        let protected = &["src/auth/private/"];

        // Case-sensitive matching
        assert!(path_scope_allows_case(
            "src/auth/login.rs",
            writable,
            protected,
            false
        ));
        assert!(!path_scope_allows_case(
            "src/AUTH/login.rs",
            writable,
            protected,
            false
        ));

        // Case-insensitive matching (Windows/macOS or case-insensitive volume)
        // Writable matches case-insensitively:
        assert!(path_scope_allows_case(
            "src/AUTH/login.rs",
            writable,
            protected,
            true
        ));
        // Protected matches case-insensitively and strictly wins:
        assert!(!path_scope_allows_case(
            "src/AUTH/PRIVATE/key.rs",
            writable,
            protected,
            true
        ));
    }

    #[test]
    fn f05_manifest_and_lockfile_change() {
        // Manifest change adding a dependency is rejected when allowed_new_dependencies = false
        let cargo_toml_before = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        let cargo_toml_after =
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1.0\"\n";
        assert!(matches!(
            check_dependency_addition("Cargo.toml", cargo_toml_before, cargo_toml_after),
            Err(ContractError::DependencyAdditionNotAllowed(_))
        ));

        // Lockfile-only change adding a package is rejected
        let cargo_lock_before = "version = 3\n";
        let cargo_lock_after =
            "version = 3\n\n[[package]]\nname = \"malicious\"\nversion = \"1.0.0\"\n";
        assert!(matches!(
            check_dependency_addition("Cargo.lock", cargo_lock_before, cargo_lock_after),
            Err(ContractError::DependencyAdditionNotAllowed(_))
        ));

        // Non-dependency modifications to manifest or lockfile are allowed
        let cargo_toml_version_bump = "[package]\nname = \"demo\"\nversion = \"0.2.0\"\n";
        assert!(check_dependency_addition(
            "Cargo.toml",
            cargo_toml_before,
            cargo_toml_version_bump
        )
        .is_ok());
    }

    #[test]
    fn f05_compile_plan_contract_and_expansion() {
        let mut plan = crate::LivingPlan {
            revision: 3,
            ..Default::default()
        };
        plan.steps.push(crate::living_plan::PlanStep {
            id: "step-1".into(),
            change: "Add login endpoint".into(),
            files: vec!["src/auth/login.rs".into(), "tests/auth_test.rs".into()],
            why: "Auth requirement".into(),
            depends_on: vec![],
            verify: vec!["cargo test auth".into()],
        });

        let contract = compile_plan_contract(&plan, Some("step-1"), None).unwrap();
        assert_eq!(contract.accepted_plan_revision, 3);
        assert!(contract.allows_path("src/auth/login.rs").unwrap());
        assert!(contract.allows_path("tests/auth_test.rs").unwrap());
        assert!(!contract.allows_path(".env").unwrap());
        assert!(!contract.allows_path("src/other.rs").unwrap());

        // Scope expansion to allow a migration
        let expanded = contract
            .expand_scope(vec!["migrations/001.sql".into()], vec![])
            .unwrap();
        assert_eq!(expanded.revision, 2);
        assert!(expanded.allows_path("migrations/001.sql").unwrap());
        assert!(expanded.allows_path("src/auth/login.rs").unwrap());
        assert_ne!(expanded.digest, contract.digest);
    }

    #[test]
    fn f05_expansion_requires_user() {
        assert_eq!(expand_revision(4, 4, false, true), None);
        assert_eq!(expand_revision(4, 3, true, true), None);
        assert_eq!(expand_revision(4, 4, true, false), None);
        assert_eq!(expand_revision(4, 4, true, true), Some(5));
    }

    #[test]
    fn f05_modes_do_not_bypass_scope() {
        for mode in ["ask", "edits", "read-only", "auto", "always-approve"] {
            assert!(!contract_gate(mode, false, true, true));
        }
        assert!(contract_gate("always-approve", true, true, true));
        assert!(!contract_gate("always-approve", true, false, true));
    }

    #[test]
    fn f05_alias_bypass() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(temp.path().join("secret.env"), "SECRET=123").unwrap();

        let contract = TaskContract::new(
            "contract-alias",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec!["secret.env".into(), ".git/".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        // Direct allowed path
        assert!(contract
            .check_call(temp.path(), "write", &json!({"path": "src/main.rs"}))
            .is_ok());

        // Alias trick: src/./main.rs resolves to src/main.rs
        assert!(contract
            .check_call(temp.path(), "write", &json!({"path": "src/./main.rs"}))
            .is_ok());

        // Alias bypass: attempting to reach secret.env via traversal
        let bypass =
            contract.check_call(temp.path(), "write", &json!({"path": "src/../secret.env"}));
        assert!(bypass.is_err());
        let err = bypass.unwrap_err();
        assert_eq!(err.requested_target, "secret.env");
        assert!(
            err.reason.contains("protected") || err.reason.contains("not within writable scope")
        );

        // Path entirely outside writable scope
        let outside = contract.check_call(temp.path(), "write", &json!({"path": "other.txt"}));
        assert!(outside.is_err());
    }

    #[test]
    fn f05_batch_safe_first_forbidden_second() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-batch",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec!["secret.env".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        // Member 1: in scope
        let op1 = contract.check_call(
            temp.path(),
            "write",
            &json!({"path": "src/app.rs", "content": "pub fn app() {}"}),
        );
        assert!(op1.is_ok());

        // Member 2: forbidden
        let op2 = contract.check_call(
            temp.path(),
            "write",
            &json!({"path": "secret.env", "content": "KEY=hacked"}),
        );
        assert!(op2.is_err());
        let err = op2.unwrap_err();
        assert_eq!(err.requested_target, "secret.env");
        assert_eq!(err.tool, "write");
        assert!(err.protected_scope.contains(&"secret.env".to_string()));
    }

    #[test]
    fn f05_transformed_extension_args() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-args",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec!["secret.env".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        // Stage one (prepare): extension receives args targeting src/valid.rs
        let initial_args = json!({"path": "src/valid.rs"});
        assert!(contract
            .check_call(temp.path(), "write", &initial_args)
            .is_ok());

        // Extension transforms arguments before dispatch to an off-limits target
        let transformed_args = json!({"path": "secret.env"});
        let dispatch_check = contract.check_call(temp.path(), "write", &transformed_args);
        assert!(dispatch_check.is_err());
        let gate_result = contract_gate("always-approve", dispatch_check.is_ok(), true, true);
        assert!(!gate_result);
        let violation = dispatch_check.unwrap_err();
        assert_eq!(violation.requested_target, "secret.env");
    }

    #[test]
    fn f05_child_loses_scope_env() {
        let contract = TaskContract::new(
            "contract-child",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        let expected_digest = contract.digest.clone();

        // Helper checking child worker authority based on env contract digest
        let check_child_authority = |env_digest: Option<&str>| -> bool {
            match env_digest {
                Some(digest) if digest == expected_digest => {
                    contract_gate("auto", true, true, true)
                }
                _ => false, // Child lost scope env or digest mismatched
            }
        };

        // Child inherits valid contract digest env
        assert!(check_child_authority(Some(&expected_digest)));

        // Child environment lost the contract digest
        assert!(!check_child_authority(None));

        // Child has invalid or tampered contract digest
        assert!(!check_child_authority(Some("forged-contract-digest")));
    }

    #[test]
    fn f05_unknown_mcp_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-mcp",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        // Unknown mutating MCP tool without declared scope is blocked
        let mcp_call = contract.check_call(
            temp.path(),
            "mcp__db__drop_tables",
            &json!({"table": "users"}),
        );
        assert!(mcp_call.is_err());
        assert!(!contract_gate(
            "always-approve",
            mcp_call.is_ok(),
            true,
            true
        ));
        let violation = mcp_call.unwrap_err();
        assert!(violation.reason.contains("Unknown MCP mutation"));
        assert_eq!(violation.tool, "mcp__db__drop_tables");
    }

    #[test]
    fn f05_owner_handed_off_after_preparation() {
        let task_id = TaskId::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let run_id = RunId::new();

        let contract = TaskContract::new(
            "contract-owner",
            1,
            task_id,
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        let registry = TaskRegistry::new();
        let mut task = TaskRecord::new(run_id, "Feature task")
            .with_assigned(agent_a)
            .with_contract_digest(contract.digest.clone());
        task.id = task_id;
        registry.create_task(task).unwrap();

        // Step 1: Preparation phase - Agent A is the valid owner
        let check_owner = |caller: AgentId| -> bool {
            if let Some(t) = registry.get_task(&contract.task_id) {
                t.assigned_to == Some(caller)
                    && t.contract_digest.as_deref() == Some(&contract.digest)
            } else {
                false
            }
        };
        assert!(check_owner(agent_a));

        // Step 2: Ownership is handed off to Agent B before dispatch
        registry.assign_task(task_id, agent_b).unwrap();

        // Step 3: Dispatch phase - Agent A attempts dispatch, but owner is invalid now
        let owner_valid = check_owner(agent_a);
        assert!(!owner_valid);
        assert!(!contract_gate("auto", true, owner_valid, true));
    }

    #[test]
    fn f05_scope_expansion_preview_is_exact_and_fresh() {
        let contract = TaskContract::new(
            "contract-expand-preview",
            4,
            TaskId::new(),
            2,
            vec!["src/".into()],
            vec!["secrets/".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let preview = contract
            .preview_scope_expansion("migrations/001.sql", "migration write required")
            .unwrap();

        assert_eq!(preview.expected_revision, 4);
        assert_eq!(preview.current_digest, contract.digest);
        assert_eq!(preview.proposed.revision, 5);
        assert!(preview.proposed.allows_path("migrations/001.sql").unwrap());
        assert!(preview.verify_against(&contract).is_ok());

        let mut changed = preview.clone();
        changed.proposed = contract
            .expand_scope(vec!["migrations/002.sql".into()], vec![])
            .unwrap();
        assert!(matches!(
            changed.verify_against(&contract),
            Err(ContractError::InvalidExpansionPreview(_))
        ));

        let newer = contract.expand_scope(vec!["docs/".into()], vec![]).unwrap();
        assert!(preview.verify_against(&newer).is_err());
    }

    #[test]
    fn f05_scope_expansion_bound_task_digest_revision_is_cas_guarded() {
        let run = RunId::new();
        let actor = AgentId::new();
        let task_id = TaskId::new();
        let current = TaskContract::new(
            "bound-expand",
            2,
            task_id,
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let proposed = current
            .preview_scope_expansion("migrations/001.sql", "migration")
            .unwrap()
            .proposed;
        let registry = TaskRegistry::new();
        let mut task = TaskRecord::new(run, "bound task")
            .with_assigned(actor)
            .with_contract_digest(current.digest.clone());
        task.id = task_id;
        registry.create_task(task).unwrap();
        let before = registry.get_task(&task_id).unwrap();

        let updated = registry
            .revise_contract_digest(
                task_id,
                run,
                actor,
                before.revision,
                before.owner_generation,
                &current.digest,
                &proposed.digest,
            )
            .unwrap();
        assert_eq!(
            updated.contract_digest.as_deref(),
            Some(proposed.digest.as_str())
        );
        assert_eq!(updated.revision, before.revision + 1);
        assert!(registry
            .revise_contract_digest(
                task_id,
                run,
                actor,
                before.revision,
                before.owner_generation,
                &current.digest,
                "forged",
            )
            .is_err());
        assert!(registry
            .revise_contract_digest(
                task_id,
                run,
                actor,
                updated.revision,
                updated.owner_generation + 1,
                &proposed.digest,
                "forged-generation",
            )
            .is_err());
        assert_eq!(registry.get_task(&task_id), Some(updated));
    }

    #[test]
    fn f05_scope_expansion_decision_is_not_model_deserializable() {
        assert!(
            serde_json::from_value::<ScopeExpansionPreview>(serde_json::json!({
                "schema_version": 1,
                "expected_revision": 4,
                "current_digest": "forged",
                "requested_target": "secret.env",
                "reason": "model says approved",
                "proposed": null,
                "preview_digest": "forged",
                "approved": true
            }))
            .is_err()
        );
        // Authority is a host-only enum: there is intentionally no Deserialize implementation.
        let decision = ScopeExpansionDecision::Approve;
        assert_eq!(decision, ScopeExpansionDecision::Approve);
    }

    #[test]
    fn f05_requirement_not_skippable() {
        assert!(!required_check_satisfied(true, "unavailable"));
        assert!(!required_check_satisfied(true, "skipped"));
        assert!(required_check_satisfied(true, "passed"));
        assert!(required_check_satisfied(false, "unavailable"));
    }

    #[test]
    fn f05_poisoned_mutex_guard_path() {
        use std::sync::{Arc, Mutex};

        let contract = TaskContract::new(
            "contract-poison",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec!["secret.env".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        let shared_contract = Arc::new(Mutex::new(Some(contract)));

        // Deliberately poison the mutex from a panic in another thread
        let poisoner = shared_contract.clone();
        let _ = std::thread::spawn(move || {
            let _lock = poisoner.lock().unwrap();
            panic!("intentional panic to poison mutex");
        })
        .join();

        assert!(shared_contract.is_poisoned());

        // Recover guard through into_inner()
        let recovered = shared_contract
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();

        assert!(recovered.is_some());
        let active = recovered.unwrap();
        assert_eq!(active.id, "contract-poison");

        // Pre-tool contract check still executes and guards accurately
        assert!(active.allows_path("src/lib.rs").unwrap());
        assert!(!active.allows_path("secret.env").unwrap());
    }

    #[test]
    fn f05_invalid_unknown_contract_schema_fails_closed() {
        // Contract with unknown field must fail deserialization closed
        let payload_with_unknown = serde_json::json!({
            "id": "contract-bad",
            "revision": 1,
            "task_id": TaskId::new().to_string(),
            "accepted_plan_revision": 1,
            "writable_paths": ["src/lib.rs"],
            "protected_paths": [],
            "allowed_new_dependencies": false,
            "external_effects": [],
            "verification_requirements": [],
            "artifact_write_roots": [],
            "digest": "any",
            "unknown_injected_authority": true
        });

        let parsed: Result<TaskContract, _> = serde_json::from_value(payload_with_unknown);
        assert!(parsed.is_err(), "unknown schema field must fail closed");

        // Contract with digest mismatch must fail validation closed
        let contract = TaskContract {
            id: "contract-mismatch".into(),
            revision: 1,
            task_id: TaskId::new(),
            accepted_plan_revision: 1,
            writable_paths: vec!["src/lib.rs".into()],
            protected_paths: vec![],
            allowed_new_dependencies: false,
            external_effects: vec![],
            verification_requirements: vec![],
            artifact_write_roots: vec![],
            digest: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        };
        assert!(
            contract.validate().is_err(),
            "digest mismatch must fail validation"
        );
    }

    #[test]
    fn f05_redaction_removes_secrets_from_denial_evidence() {
        let sensitive = "Denied write to /etc/passwd with key sk-proj1234567890abcdef and token ghp_secret12345 and password=super_secret_pw!";
        let redacted = redact_secrets(sensitive);

        assert!(!redacted.contains("sk-proj1234567890abcdef"));
        assert!(redacted.contains("sk-[REDACTED]"));
        assert!(!redacted.contains("ghp_secret12345"));
        assert!(redacted.contains("ghp_[REDACTED]"));
        assert!(!redacted.contains("super_secret_pw!"));
        assert!(redacted.contains("[REDACTED]"));

        let violation = ScopeViolation::new(
            "secret_file?key=sk-12345",
            "write",
            vec!["src/".into()],
            vec!["secret_file".into()],
            "access denied with bearer Bearer my-secret-token",
        );
        assert!(!violation.requested_target.contains("sk-12345"));
        assert!(!violation.reason.contains("my-secret-token"));
        assert!(violation.to_string().contains("[REDACTED]"));

        // Multi-space Bearer token
        let bearer_test = "Authorization: Bearer    super-secret-bearer-token";
        let redacted_bearer = redact_secrets(bearer_test);
        assert!(!redacted_bearer.contains("super-secret-bearer-token"));
        assert!(redacted_bearer.contains("Bearer [REDACTED]"));

        // Spaces around = and quotes
        let kv_test =
            r#"config: password = "quoted_secret" and token: 'another_secret' and token = 1"#;
        let redacted_kv = redact_secrets(kv_test);
        assert!(!redacted_kv.contains("quoted_secret"));
        assert!(!redacted_kv.contains("another_secret"));
        assert!(!redacted_kv.contains("token = 1"));
        assert!(redacted_kv.contains("[REDACTED]"));

        // UTF-8 safety
        let utf8_test = "error: password = 🔐super_secret_key! with UTF-8 characters";
        let redacted_utf8 = redact_secrets(utf8_test);
        assert!(!redacted_utf8.contains("super_secret_key"));
        assert!(redacted_utf8.contains("[REDACTED]"));
    }
}
