//! Tool permissions: the mode a run is in, the rules that quiet or refuse a
//! tool, and the question put to the user when neither has an answer.
//!
//! No TypeScript counterpart. Vendor `pi` runs every tool once a project is
//! trusted; this is a documented divergence, designed in
//! `docs/superpowers/specs/2026-09-01-trust-and-control-design.md`. The loop
//! (`turn.rs`) asks the policy before every tool call, after the extension
//! `tool_call` hook and before the tool runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "permission_risk.rs"]
mod permission_risk;

/// How much a run may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Read tools only; everything else is refused without a question.
    ReadOnly,
    /// Read tools run; edits, shell commands and unknown tools ask.
    #[default]
    Ask,
    /// Edits inside the project run; shell commands and unknown tools ask.
    Edits,
    /// Ordinary workspace edits and recognized local checks run; boundary cases ask.
    Auto,
    /// No harness approval prompts. Explicit denies and isolation still apply.
    AlwaysApprove,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 5] = [
        PermissionMode::Ask,
        PermissionMode::Edits,
        PermissionMode::ReadOnly,
        PermissionMode::Auto,
        PermissionMode::AlwaysApprove,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Manual",
            Self::Edits => "Accept Edits",
            Self::ReadOnly => "Plan Mode",
            Self::Auto => "Auto Mode",
            Self::AlwaysApprove => "Always Approve",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Ask => Self::Edits,
            Self::Edits => Self::ReadOnly,
            Self::ReadOnly => Self::Auto,
            Self::Auto => Self::AlwaysApprove,
            Self::AlwaysApprove => Self::Ask,
        }
    }

    /// User-facing names and legacy aliases. These select approval policy,
    /// not an operating-system sandbox.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "plan" | "plan mode" | "plan-mode" | "read-only" | "readonly" | "read_only" => {
                Some(Self::ReadOnly)
            }
            "manual" | "ask" | "default" => Some(Self::Ask),
            "edits" | "accept edits" | "accept-edits" | "workspace-write" => Some(Self::Edits),
            "auto" | "auto mode" | "auto-mode" => Some(Self::Auto),
            "always-approve" | "always approve" | "autopilot" | "full-access"
            | "danger-full-access" | "bypass" | "yolo" => Some(Self::AlwaysApprove),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::Edits => "edits",
            Self::Auto => "auto",
            Self::AlwaysApprove => "always-approve",
        }
    }

    /// One line for `/permissions` and the help text.
    pub fn describe(self) -> &'static str {
        match self {
            Self::ReadOnly => "read tools only; edits and shell commands are refused",
            Self::Ask => "read tools run; edits and shell commands ask",
            Self::Edits => "edits inside the project run; shell commands ask",
            Self::Auto => {
                "workspace edits and recognized local checks run; risky or unknown actions ask. Tests it runs execute code it wrote."
            }
            Self::AlwaysApprove => {
                "WARNING: no harness approval prompts; explicit denies and isolation still apply"
            }
        }
    }
}

/// What kind of thing a tool does, for the mode table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolClass {
    Read,
    Edit,
    Shell,
    /// Reaches outside the machine (`web_fetch`, `web_search`,
    /// `visual_snapshot`). Plan Mode allows `web_search` without a prompt;
    /// fetching a URL asks in every prompted mode unless a rule allows it.
    Network,
    Other,
}

/// Shared sensitive-file classification for auxiliary readers such as plan
/// evidence. Auxiliary readers may be stricter, never less strict, than policy.
pub fn is_sensitive_file_path(path: &str) -> bool {
    permission_risk::is_protected_path(path)
}

pub(crate) fn is_secret_file_path(path: &str) -> bool {
    permission_risk::is_secret_path(path)
}

pub fn tool_class(tool: &str) -> ToolClass {
    match tool {
        "patch_preview" | "patch_status" => ToolClass::Read,
        "patch_apply" | "patch_rollback" => ToolClass::Edit,
        "process_status" | "process_output" | "process_list" => ToolClass::Read,
        "process_start" | "process_write" => ToolClass::Shell,
        "process_stop" => ToolClass::Other,
        "test_related" | "test_impacted" | "test_plan" => ToolClass::Read,
        "package_info" | "package_exports" | "package_symbol" | "package_dependents"
        | "package_why" => ToolClass::Read,
        "workspace_packages" | "build_targets" | "build_dependencies" | "build_affected"
        | "build_command" => ToolClass::Read,
        "git_symbol_history"
        | "git_related_commits"
        | "git_changed_symbols"
        | "git_branch_diff"
        | "git_blame_symbol"
        | "git_commit_context"
        | "git_conflict_explain" => ToolClass::Read,
        "impact_analyze" | "verification_plan" | "workspace_checkpoint" | "workspace_diff" => {
            ToolClass::Read
        }
        "workspace_restore" => ToolClass::Edit,
        "repo_map"
        | "symbol_search"
        | "file_symbols"
        | "file_dependencies"
        | "symbol_relationships"
        | "related_files"
        | "code_query" => ToolClass::Read,
        "lsp_definition"
        | "lsp_references"
        | "lsp_hover"
        | "lsp_document_symbols"
        | "lsp_workspace_symbols"
        | "lsp_implementations"
        | "lsp_type_definition"
        | "lsp_diagnostics" => ToolClass::Read,
        // Reading a job's output or keeping the ledger changes nothing the
        // user would want to be asked about.
        // A batch is judged operation by operation; the wrapper itself
        // changes nothing.
        // `memory_search` and `retrieve_output` read the memory index and
        // governor's store. Artifact submission is a mutation, not a read.
        "read" | "grep" | "find" | "ls" | "job_output" | "todo" | "mcp_read" | "batch"
        | "memory_search" | "retrieve_output" | "update_plan" | "tool_search" | "agent_status"
        | "task_list" | "task_get" | "workflow_status" | "propose_plan" => ToolClass::Read,
        // Process control, messages to workers, and task dispatch can cause effects.
        "job_kill" | "agent_message" | "agent_stop" | "task_create" | "task_update"
        | "graph_submit" => ToolClass::Other,
        "write" | "edit" | "notebook_edit" | "apply_patch" => ToolClass::Edit,
        "bash" | "powershell" | "exec_command" | "write_stdin" => ToolClass::Shell,
        "web_fetch" | "web_search" | "visual_snapshot" => ToolClass::Network,
        _ => ToolClass::Other,
    }
}

/// Whether a call is intrinsically read-only or a recognized local check.
/// This is an additional capability ceiling, not a permission grant: the
/// ordinary policy still decides whether an otherwise-safe call may run.
pub(crate) fn read_only_capability_allows(
    tool: &str,
    args: &Value,
    command: &str,
    cwd: &Path,
) -> bool {
    match tool_class(tool) {
        ToolClass::Read => !matches!(tool, "batch" | "propose_plan" | "todo" | "update_plan"),
        ToolClass::Shell => permission_risk::routine_local_shell(
            tool,
            args,
            command,
            cwd,
            &FilesystemBoundaryPolicy::default(),
        ),
        ToolClass::Edit | ToolClass::Network | ToolClass::Other => false,
    }
}

/// AST specifier for a permission rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSpecifier {
    Wildcard,
    Subject(String),
    Parameter { param: String, value: String },
}

impl std::fmt::Display for RuleSpecifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wildcard => write!(f, "*"),
            Self::Subject(subject) => write!(f, "{subject}"),
            Self::Parameter { param, value } => write!(f, "{param}:{value}"),
        }
    }
}

/// Diagnostic errors when parsing a permission rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleParseError {
    Empty,
    InvalidToolName(String),
    MissingClosingParen,
    TrailingCharacters(String),
    EmptyParameterName,
    EmptyParameterValue,
    InvalidParameterSyntax(String),
    MalformedSpecifier(String),
}

impl std::fmt::Display for RuleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "permission rule is empty"),
            Self::InvalidToolName(tool) => write!(f, "invalid tool name `{tool}`"),
            Self::MissingClosingParen => write!(f, "missing closing parenthesis"),
            Self::TrailingCharacters(s) => write!(f, "unexpected trailing characters `{s}`"),
            Self::EmptyParameterName => write!(f, "parameter name cannot be empty"),
            Self::EmptyParameterValue => write!(f, "parameter value cannot be empty"),
            Self::InvalidParameterSyntax(s) => write!(f, "invalid parameter syntax `{s}`"),
            Self::MalformedSpecifier(s) => write!(f, "malformed specifier `{s}`"),
        }
    }
}

impl std::error::Error for RuleParseError {}

/// `tool` or `tool(pattern)`. The pattern is a glob over the call's subject:
/// the command for a shell tool, the path for a file tool, or a parameter rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRule {
    pub tool: String,
    pub pattern: Option<String>,
    pub specifier: Option<RuleSpecifier>,
}

impl PermissionRule {
    pub fn bare(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            pattern: None,
            specifier: None,
        }
    }

    pub fn subject(tool: impl Into<String>, subject: impl Into<String>) -> Self {
        let s = subject.into();
        Self {
            tool: tool.into(),
            pattern: Some(s.clone()),
            specifier: Some(RuleSpecifier::Subject(s)),
        }
    }

    pub fn wildcard(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            pattern: Some("*".to_string()),
            specifier: Some(RuleSpecifier::Wildcard),
        }
    }

    pub fn parameter(
        tool: impl Into<String>,
        param: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let p = param.into();
        let v = value.into();
        Self {
            tool: tool.into(),
            pattern: Some(format!("{p}:{v}")),
            specifier: Some(RuleSpecifier::Parameter { param: p, value: v }),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::parse_with_diagnostic(text).ok()
    }

    pub fn parse_with_diagnostic(text: &str) -> Result<Self, RuleParseError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(RuleParseError::Empty);
        }

        let Some(open) = text.find('(') else {
            if text.chars().any(char::is_whitespace) || text.contains(')') {
                return Err(RuleParseError::InvalidToolName(text.to_string()));
            }
            return Ok(Self {
                tool: text.to_string(),
                pattern: None,
                specifier: None,
            });
        };

        let tool = text[..open].trim();
        if tool.is_empty() || tool.chars().any(char::is_whitespace) {
            return Err(RuleParseError::InvalidToolName(tool.to_string()));
        }

        let mut depth = 0usize;
        let mut close_opt = None;
        for (i, ch) in text[open..].char_indices() {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
                if depth == 0 {
                    close_opt = Some(open + i);
                    break;
                }
            }
        }

        let Some(close) = close_opt else {
            return Err(RuleParseError::MissingClosingParen);
        };

        let trailing = text[close + 1..].trim();
        if !trailing.is_empty() {
            return Err(RuleParseError::TrailingCharacters(trailing.to_string()));
        }

        let inner = text[open + 1..close].trim();
        if inner.is_empty() {
            return Ok(Self {
                tool: tool.to_string(),
                pattern: None,
                specifier: None,
            });
        }

        let specifier = parse_specifier(inner)?;
        let pattern = specifier.as_ref().map(|s| s.to_string());
        Ok(Self {
            tool: tool.to_string(),
            pattern,
            specifier,
        })
    }

    pub fn matches(&self, tool: &str, subject: &str) -> bool {
        self.matches_call(tool, &Value::Null, subject)
    }

    pub fn matches_call(&self, tool: &str, args: &Value, subject: &str) -> bool {
        if !self.tool_matches(tool) {
            return false;
        }
        let Some(specifier) = &self.specifier else {
            if let Some(pattern) = &self.pattern {
                return self.matches_subject(tool, pattern, subject);
            }
            return true;
        };
        match specifier {
            RuleSpecifier::Wildcard => !subject.is_empty() || !args.is_null(),
            RuleSpecifier::Subject(pattern) => self.matches_subject(tool, pattern, subject),
            RuleSpecifier::Parameter { param, value } => {
                self.matches_parameter(tool, args, subject, param, value)
            }
        }
    }

    fn matches_subject(&self, _tool: &str, pattern: &str, subject: &str) -> bool {
        if subject.is_empty() {
            return false;
        }
        // `git status *` also means plain `git status`: the trailing `*`
        // says "and whatever follows", including nothing.
        if let Some(prefix) = pattern.strip_suffix(" *") {
            if fold(subject) == fold(prefix) {
                return true;
            }
        }
        glob_matches(&fold(pattern), &fold(subject))
    }

    fn matches_parameter(
        &self,
        tool: &str,
        args: &Value,
        subject: &str,
        param: &str,
        value: &str,
    ) -> bool {
        // Safety rule:
        // Never permit generic parameter matching for primary content fields whose interpretation
        // is command/path-specific; keep dedicated matchers for shell command, filesystem path,
        // and URL domain.
        match param {
            "command" => self.matches_subject(tool, value, subject),
            "path" => self.matches_subject(tool, value, subject),
            "domain" => {
                let host = if tool_class(tool) == ToolClass::Network
                    && matches!(tool, "web_fetch" | "visual_snapshot")
                {
                    subject.to_string()
                } else if let Some(url) = args.get("url").and_then(Value::as_str) {
                    host_of(url)
                } else {
                    subject.to_string()
                };
                if host.is_empty() {
                    return false;
                }
                glob_matches(&fold(value), &fold(&host))
            }
            _ => {
                if let Some(arg_val) = get_param_value(args, param) {
                    let actual_str = value_to_string(arg_val);
                    glob_matches(&fold(value), &fold(&actual_str))
                } else if let Some(subject_val) = subject.strip_prefix(&format!("{param}:")) {
                    glob_matches(&fold(value), &fold(subject_val))
                } else {
                    // Omitted parameter in a call does NOT match a parameter-specific rule.
                    false
                }
            }
        }
    }

    pub fn tool_matches(&self, tool: &str) -> bool {
        let rule_tool = normalize_tool_name(&self.tool);
        let target_tool = normalize_tool_name(tool);
        if rule_tool == target_tool {
            return true;
        }
        if rule_tool.chars().any(|ch| ch == '*' || ch == '?') {
            glob_matches(&rule_tool, &target_tool)
        } else {
            false
        }
    }
}

impl std::fmt::Display for PermissionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.specifier {
            Some(spec) => write!(f, "{}({spec})", self.tool),
            None => match &self.pattern {
                Some(pattern) => write!(f, "{}({pattern})", self.tool),
                None => f.write_str(&self.tool),
            },
        }
    }
}

pub(crate) fn normalize_tool_name(tool: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = tool.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 && chars[i - 1] != '_' && !chars[i - 1].is_ascii_uppercase() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_windows_drive_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

fn is_url_with_scheme(s: &str) -> bool {
    if let Some(pos) = s.find("://") {
        let scheme = &s[..pos];
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    } else {
        false
    }
}

fn is_valid_param_name(param: &str) -> bool {
    if param.is_empty() {
        return false;
    }
    if param
        .chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\\' || c == ':')
    {
        return false;
    }
    let first = param.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    // A dot is only valid if it's an indexed field access like "tasks[0].model"
    if param.contains('.') && !param.contains('[') {
        return false;
    }
    if param.contains('[') || param.contains('.') {
        for part in param.split('.') {
            if let Some(b_start) = part.find('[') {
                if !part.ends_with(']') {
                    return false;
                }
                let field = &part[..b_start];
                if !field.is_empty()
                    && !field
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return false;
                }
                let index_str = &part[b_start + 1..part.len() - 1];
                if index_str.is_empty() || !index_str.chars().all(|c| c.is_ascii_digit()) {
                    return false;
                }
            } else if !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return false;
            }
        }
        true
    } else {
        param
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }
}

fn parse_specifier(inner: &str) -> Result<Option<RuleSpecifier>, RuleParseError> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed == "*" {
        return Ok(Some(RuleSpecifier::Wildcard));
    }

    if is_windows_drive_path(trimmed) || is_url_with_scheme(trimmed) {
        return Ok(Some(RuleSpecifier::Subject(trimmed.to_string())));
    }

    if let Some(colon_pos) = trimmed.find(':') {
        let param = trimmed[..colon_pos].trim();
        let value = trimmed[colon_pos + 1..].trim();

        if colon_pos == 0 || param.is_empty() {
            return Err(RuleParseError::EmptyParameterName);
        }

        if is_valid_param_name(param) {
            if value.is_empty() {
                return Err(RuleParseError::EmptyParameterValue);
            }
            return Ok(Some(RuleSpecifier::Parameter {
                param: param.to_string(),
                value: value.to_string(),
            }));
        }
    }

    Ok(Some(RuleSpecifier::Subject(trimmed.to_string())))
}

fn get_param_value<'a>(args: &'a Value, param: &str) -> Option<&'a Value> {
    if args.is_null() {
        return None;
    }
    if let Some(val) = args.get(param) {
        if !val.is_null() {
            return Some(val);
        }
    }
    if param.contains('[') || param.contains('.') {
        if let Some(val) = resolve_json_path(args, param) {
            if !val.is_null() {
                return Some(val);
            }
        }
    }
    None
}

/// `Some(tasks)` when `args` contains a non-empty batch and no ambiguous
/// top-level fields. Callers deny malformed batches that still carry `tasks`.
fn split_agent_tasks(args: &Value) -> Option<Vec<Value>> {
    let object = args.as_object()?;
    let tasks = object.get("tasks")?.as_array()?;
    if object.len() != 1 || tasks.is_empty() {
        return None;
    }
    Some(tasks.clone())
}

fn resolve_json_path<'a>(mut current: &'a Value, path: &str) -> Option<&'a Value> {
    for part in path.split('.') {
        if let Some(bracket_start) = part.find('[') {
            if !part.ends_with(']') {
                return None;
            }
            let field = &part[..bracket_start];
            let index_str = &part[bracket_start + 1..part.len() - 1];
            let index: usize = index_str.parse().ok()?;
            if !field.is_empty() {
                current = current.get(field)?;
            }
            current = current.as_array()?.get(index)?;
        } else {
            current = current.get(part)?;
        }
    }
    Some(current)
}

fn value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// File systems on Windows do not care about case; neither do rules there.
fn fold(text: &str) -> String {
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text.to_string()
    }
}

/// `*` and `**` match any run of characters, `/` included; `?` matches one.
/// Iterative with one backtrack point, which is all a single-star pattern
/// needs and what keeps a pathological pattern from going exponential.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '*' {
            while p < pattern.len() && pattern[p] == '*' {
                p += 1;
            }
            star = Some((p, t));
            continue;
        }
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
            continue;
        }
        match star {
            Some((star_p, star_t)) => {
                p = star_p;
                t = star_t + 1;
                star = Some((star_p, t));
            }
            None => return false,
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// A tool call the policy could not decide on its own, shaped for a host to
/// put to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolApprovalRequest {
    /// Choices issued by policy. Hosts may remove unsupported choices, never add grants.
    pub legal_choices: Vec<crate::approval::ApprovalChoice>,
    pub tool_call_id: String,
    pub tool: String,
    pub args: Value,
    /// What a rule would be matched against: the command, or the path.
    pub subject: String,
    /// One line naming the call: `bash · git status`, `write · src/lib.rs`.
    pub summary: String,
    /// What "allow for this session" / "always allow" would add.
    pub session_rule: String,
    /// A file tool whose target is not under the project.
    pub outside_project: bool,
    pub mode: PermissionMode,
}

impl ToolApprovalRequest {
    pub fn allows(&self, decision: ToolApprovalDecision) -> bool {
        use crate::approval::GrantScope;
        let scope = match decision {
            ToolApprovalDecision::AllowOnce => GrantScope::Once,
            ToolApprovalDecision::AllowForSession => GrantScope::Session,
            ToolApprovalDecision::AllowAlways => GrantScope::Project,
            ToolApprovalDecision::Deny => GrantScope::Deny,
        };
        self.legal_choices
            .iter()
            .any(|choice| choice.scope == scope)
    }

    /// Compatibility choices supported by the current four-decision host bridge.
    pub fn host_choices(&self, project_available: bool) -> Vec<ToolApprovalDecision> {
        use ToolApprovalDecision::*;
        [AllowOnce, AllowForSession, AllowAlways, Deny]
            .into_iter()
            .filter(|choice| self.allows(*choice) && (*choice != AllowAlways || project_available))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolApprovalDecision {
    AllowOnce,
    /// The gate appends the request's `session_rule` for the rest of the run.
    AllowForSession,
    /// The host has persisted the rule; the gate also appends it, so the next
    /// call in this run is quiet without a re-read.
    AllowAlways,
    Deny,
}

/// What the host answers an `Ask` with. Blocks the tool thread until the
/// user has spoken.
#[derive(Clone)]
pub struct ToolApprover(
    pub Arc<dyn Fn(&ToolApprovalRequest) -> ToolApprovalDecision + Send + Sync>,
);

impl std::fmt::Debug for ToolApprover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ToolApprover(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionVerdict {
    Allow,
    Deny { reason: String },
    Ask(ToolApprovalRequest),
}

/// Policy governing read operations targeting paths outside the project/worktree root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ReadOutsideRootPolicy {
    /// Reading outside root is permitted (standard default behavior).
    #[default]
    Allow,
    /// Reading outside root prompts for user approval.
    Ask,
    /// Reading outside root is strictly denied.
    Deny,
}

/// Filesystem boundary policy enforcing root containment for isolated agents
/// while optionally allowing git metadata access and configuring read policies.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilesystemBoundaryPolicy {
    /// Explicit root directory boundary (e.g. workspace or isolated worktree root).
    pub root: Option<PathBuf>,
    /// Git repository root (e.g. main repo when in an isolated worktree).
    pub repo_root: Option<PathBuf>,
    /// Policy governing reads targeting paths outside `root`.
    pub read_outside_root: ReadOutsideRootPolicy,
    /// If true, any mutation outside `root` is denied immediately without prompting.
    pub enforce_root_for_mutations: bool,
    /// If true, git metadata operations (such as under `repo_root/.git`) are permitted.
    pub allow_git_metadata: bool,
}

/// The mode plus every rule in force. `allow` and `deny` come from settings;
/// `session_allow` is what the user granted for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPolicy {
    /// Supplied by trusted host configuration, never by tool arguments.
    pub project_trusted: bool,
    pub mode: PermissionMode,
    pub allow: Vec<PermissionRule>,
    pub deny: Vec<PermissionRule>,
    pub session_allow: Vec<PermissionRule>,
    /// MCP tools whose server marked `readOnlyHint`. `mcp_read` is Read by name.
    pub mcp_read_only: BTreeSet<String>,
    /// Filesystem boundary enforcement configuration.
    pub filesystem_boundary: FilesystemBoundaryPolicy,
}

impl Default for PermissionPolicy {
    /// The library default is what vendor `pi` does: every tool runs. The
    /// CLI installs the configured policy (`ask` unless told otherwise) in
    /// `build_agent`; embedders who want the gate set a mode.
    fn default() -> Self {
        Self {
            project_trusted: false,
            mode: PermissionMode::AlwaysApprove,
            allow: Vec::new(),
            deny: Vec::new(),
            session_allow: Vec::new(),
            mcp_read_only: BTreeSet::new(),
            filesystem_boundary: FilesystemBoundaryPolicy::default(),
        }
    }
}

impl PermissionPolicy {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    pub fn with_filesystem_boundary(mut self, boundary: FilesystemBoundaryPolicy) -> Self {
        self.filesystem_boundary = boundary;
        self
    }

    pub fn set_filesystem_boundary(&mut self, boundary: FilesystemBoundaryPolicy) {
        self.filesystem_boundary = boundary;
    }

    /// Sets an isolated worktree boundary for this policy.
    /// Mutations outside `worktree_root` are strictly forbidden,
    /// and git metadata access to `repo_root` (or parent `.git`) is preserved.
    pub fn set_worktree_boundary(&mut self, worktree_root: &Path, repo_root: Option<&Path>) {
        self.filesystem_boundary.root = Some(worktree_root.to_path_buf());
        self.filesystem_boundary.repo_root = repo_root.map(|p| p.to_path_buf());
        self.filesystem_boundary.enforce_root_for_mutations = true;
        self.filesystem_boundary.allow_git_metadata = true;
        // The structural boundary is distinct from explicit deny rules.
        // Injecting broad parent denies then exempting metadata would allow
        // unrelated user deny rules to be bypassed.
    }

    pub fn set_read_outside_root_policy(&mut self, policy: ReadOutsideRootPolicy) {
        self.filesystem_boundary.read_outside_root = policy;
    }

    pub fn is_git_metadata_call(&self, tool: &str, args: &Value, cwd: &Path) -> bool {
        if !self.filesystem_boundary.allow_git_metadata {
            return false;
        }
        let root = self.filesystem_boundary.root.as_deref().unwrap_or(cwd);
        let repo_root = self.filesystem_boundary.repo_root.as_deref();

        if let Some(raw_path) = args.get("path").and_then(Value::as_str) {
            let given = Path::new(raw_path);
            let joined = if given.is_absolute() {
                given.to_path_buf()
            } else {
                cwd.join(given)
            };
            return is_git_metadata_path(&joined, repo_root, Some(root));
        }

        if tool == "apply_patch" {
            if let Some(input) = args.get("input").and_then(Value::as_str) {
                if let Ok(parsed) = crate::apply_patch::parse_codex_patch(input) {
                    return parsed.actions.iter().all(|a| {
                        let p = match a {
                            crate::apply_patch::FileAction::Add { path, .. } => path,
                            crate::apply_patch::FileAction::Update { path, .. } => path,
                            crate::apply_patch::FileAction::Delete { path } => path,
                        };
                        is_git_metadata_path(Path::new(p), repo_root, Some(root))
                    });
                }
            }
        }

        false
    }

    /// Add deny rules preventing any mutation or tool execution targeting the parent checkout.
    pub fn deny_parent_checkout(&mut self, parent_cwd: &Path) {
        let parent_str = slashes(&normalize_lexically(parent_cwd));
        self.deny
            .push(PermissionRule::subject("*", format!("{parent_str}/**")));
        self.deny
            .push(PermissionRule::subject("*", format!("{parent_str}/*")));
        self.deny.push(PermissionRule::subject("*", parent_str));
    }

    /// Hard constraints (deny rules, Plan Mode, filesystem boundaries) precede
    /// every approval shortcut. Automatic grants are conservative; unknown
    /// actions ask unless the user explicitly selected Always Approve.
    pub fn decide(
        &self,
        tool_call_id: &str,
        tool: &str,
        args: &Value,
        cwd: &Path,
    ) -> PermissionVerdict {
        if tool == "agent" {
            if let Some(tasks) = split_agent_tasks(args) {
                return self.decide_agent_batch(tool_call_id, &tasks, cwd);
            }
            if args.get("tasks").is_some() {
                return PermissionVerdict::Deny {
                    reason: "agent: pass per-task fields inside `tasks`, not beside it".into(),
                };
            }
        }
        // Capturing evidence is a compound read. It must obey the same rules
        // as the read tool, including named denies, even when propose_plan is
        // generally permitted or Always Approve is selected.
        let mut evidence_approval = None;
        if tool == "propose_plan" {
            if let Some(evidence) = args.get("evidence").and_then(Value::as_array) {
                if evidence.len() > 32 {
                    return PermissionVerdict::Deny {
                        reason: "Plan evidence is limited to 32 files".into(),
                    };
                }
                for item in evidence {
                    let Some(path) = item.get("path").and_then(Value::as_str) else {
                        return PermissionVerdict::Deny {
                            reason: "Every plan evidence item needs a path".into(),
                        };
                    };
                    match self.decide(tool_call_id, "read", &serde_json::json!({"path":path}), cwd)
                    {
                        PermissionVerdict::Deny { reason } => {
                            return PermissionVerdict::Deny { reason }
                        }
                        PermissionVerdict::Ask(mut request) => {
                            request.tool = tool.to_string();
                            request.args = args.clone();
                            request.summary = crate::approval::display_text(&format!(
                                "Capture plan evidence: {}",
                                request.subject
                            ));
                            request.session_rule.clear();
                            request.legal_choices =
                                crate::approval::offer_scopes(true, false, false);
                            evidence_approval = Some(request);
                        }
                        PermissionVerdict::Allow => {}
                    }
                }
            }
        }
        // Transaction previews and recovery capture source bytes even when the
        // outer tool is an edit. Approval is for the complete original call,
        // so its one-shot permit also binds these compound reads.
        if matches!(
            tool,
            "write"
                | "edit"
                | "notebook_edit"
                | "apply_patch"
                | "patch_preview"
                | "patch_apply"
                | "patch_status"
                | "patch_rollback"
        ) {
            let mut source_paths = crate::runtime::contracts::extract_tool_targets(tool, args);
            if tool == "patch_status"
                && args.get("observe_commit").and_then(Value::as_bool) == Some(true)
            {
                // Git may resolve packed objects, linked-worktree metadata and
                // config includes. A directory-level grant cannot prove that
                // none of those internal reads intersects a scoped read deny.
                if self.deny.iter().any(|rule| rule.tool_matches("read")) {
                    return PermissionVerdict::Deny {
                        reason: "Git commit observation requires unrestricted metadata reads; a read deny prevents proving its internal Git reads are authorized".into(),
                    };
                }
                source_paths.push(".git".into());
            }
            for path in source_paths {
                match self.decide(tool_call_id, "read", &serde_json::json!({"path":path}), cwd) {
                    PermissionVerdict::Deny { reason } => {
                        return PermissionVerdict::Deny { reason }
                    }
                    PermissionVerdict::Ask(mut request) => {
                        request.tool = tool.to_owned();
                        request.args = args.clone();
                        request.summary = crate::approval::display_text(&format!(
                            "Read transaction source for {tool}: {}",
                            request.subject
                        ));
                        request.session_rule.clear();
                        request.legal_choices = crate::approval::offer_scopes(true, false, false);
                        evidence_approval = Some(request);
                    }
                    PermissionVerdict::Allow => {}
                }
            }
        }
        let targets =
            match permission_risk::file_targets(tool, args, cwd, &self.filesystem_boundary) {
                Ok(targets) => targets,
                Err(reason) => {
                    return PermissionVerdict::Deny {
                        reason: format!(
                            "Permission denied: cannot validate `{tool}` targets: {reason}."
                        ),
                    }
                }
            };
        let (subject, outside_project) = if targets.is_empty() {
            subject_of_with_boundary(tool, args, cwd, Some(&self.filesystem_boundary))
        } else {
            (
                targets
                    .iter()
                    .map(|target| target.subject.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                targets.iter().any(|target| target.outside),
            )
        };
        let effective_root = self.filesystem_boundary.root.as_deref().unwrap_or(cwd);
        let class = if self.mode == PermissionMode::ReadOnly
            && tool == "agent"
            && self.agent_call_is_read_only(args)
        {
            ToolClass::Read
        } else {
            self.class_of(tool)
        };
        // Git metadata is a narrow read-boundary exception, never a deny-rule
        // exception or permission to write outside an isolated root.
        let is_git_meta = class == ToolClass::Read && self.is_git_metadata_call(tool, args, cwd);
        let subjects = if !targets.is_empty() {
            targets
                .iter()
                .map(|target| target.subject.clone())
                .collect::<Vec<_>>()
        } else if class == ToolClass::Shell && tool != "process_start" {
            let segments = shell_segments(&subject);
            if segments.is_empty() {
                vec![subject.clone()]
            } else {
                segments
            }
        } else {
            vec![subject.clone()]
        };

        if let Some(rule) = self.deny.iter().find(|rule| {
            subjects
                .iter()
                .any(|part| rule.matches_call(tool, args, part))
        }) {
            return PermissionVerdict::Deny {
                reason: format!(
                    "Permission denied: `{}` matches the deny rule `{rule}`.",
                    summary_of(tool, &subject)
                ),
            };
        }
        // The structured process route must not sidestep an existing shell
        // deny. Only deny rules cross this boundary; a Bash allow is not a
        // grant to start persistent processes or inject stdin into one.
        if tool == "process_start" {
            let shell_names = ["bash", "powershell", "exec_command"];
            let shell_args = serde_json::json!({"command": subject, "cmd": subject});
            let shell_denies = self
                .deny
                .iter()
                .filter(|rule| shell_names.iter().any(|name| rule.tool_matches(name)))
                .collect::<Vec<_>>();
            if shell_denies.iter().any(|rule| {
                shell_names
                    .iter()
                    .any(|name| rule.matches_call(name, &shell_args, &subject))
            }) {
                return PermissionVerdict::Deny {
                    reason: "Permission denied: process command matches an active shell deny rule."
                        .into(),
                };
            }
            let report = crate::shell_policy::analyze_command(&subject);
            if !shell_denies.is_empty()
                && (report.has_nested_shell || report.has_substitution || report.has_unknown_syntax)
            {
                return PermissionVerdict::Deny { reason: "Permission denied: process arguments cannot be proven to satisfy active shell deny rules.".into() };
            }
        }
        // A deny rule naming shell commands cannot be proven satisfied by
        // examining only the outer program of a substitution. Fail closed.
        if class == ToolClass::Shell
            && has_command_substitution(&subject)
            && self.deny.iter().any(|rule| rule.tool_matches(tool))
        {
            return PermissionVerdict::Deny {
                reason: "Permission denied: command substitution cannot be checked against the active shell deny rules.".into(),
            };
        }

        if self.mode == PermissionMode::ReadOnly
            && !matches!(class, ToolClass::Read | ToolClass::Network)
        {
            return PermissionVerdict::Deny {
                reason: format!(
                    "{} (`{}`).",
                    crate::PLAN_MODE_DENIAL,
                    summary_of(tool, &subject)
                ),
            };
        }
        if class == ToolClass::Edit {
            if self.filesystem_boundary.enforce_root_for_mutations && outside_project {
                return PermissionVerdict::Deny {
                    reason: format!(
                        "Permission denied: `{}` attempts to mutate outside isolated root `{}`.",
                        summary_of(tool, &subject),
                        effective_root.display()
                    ),
                };
            }
            if targets.iter().any(|target| target.symlink_escape) {
                return PermissionVerdict::Deny {
                    reason: format!("Permission denied: `{}` contains an untrusted symlink escape outside `{}`.", summary_of(tool, &subject), effective_root.display()),
                };
            }
        }
        if class == ToolClass::Read
            && outside_project
            && !is_git_meta
            && self.filesystem_boundary.read_outside_root == ReadOutsideRootPolicy::Deny
        {
            return PermissionVerdict::Deny {
                reason: format!(
                    "Permission denied: `{}` reads outside root `{}`.",
                    summary_of(tool, &subject),
                    effective_root.display()
                ),
            };
        }
        let secret = targets.iter().any(|target| target.secret);
        if self.mode == PermissionMode::ReadOnly && secret {
            return PermissionVerdict::Deny {
                reason: "plan mode: credential access requires an explicit execution mode".into(),
            };
        }
        if self.mode == PermissionMode::AlwaysApprove {
            return PermissionVerdict::Allow;
        }

        if let Some(request) = evidence_approval {
            return PermissionVerdict::Ask(request);
        }

        // Every patch target and every shell segment needs its own grant.
        // A subject pattern cannot authorize hidden command substitutions.
        if subjects.iter().all(|part| {
            self.allow
                .iter()
                .chain(self.session_allow.iter())
                .any(|rule| {
                    rule.matches_call(tool, args, part)
                        && (class != ToolClass::Shell
                            || (rule.specifier.is_none() && rule.pattern.is_none())
                            || !has_command_substitution(part))
                })
        }) {
            return PermissionVerdict::Allow;
        }

        let read_needs_approval = outside_project
            && !is_git_meta
            && (self.filesystem_boundary.read_outside_root == ReadOutsideRootPolicy::Ask
                || self.mode == PermissionMode::Auto);
        if class == ToolClass::Read && !secret && !read_needs_approval {
            return PermissionVerdict::Allow;
        }
        if class == ToolClass::Network
            && self.mode == PermissionMode::ReadOnly
            && tool == "web_search"
        {
            return PermissionVerdict::Allow;
        }
        if matches!(self.mode, PermissionMode::Edits | PermissionMode::Auto)
            && class == ToolClass::Edit
            && !targets.is_empty()
            && targets
                .iter()
                .all(|target| !target.outside && !target.protected && !target.destructive)
        {
            return PermissionVerdict::Allow;
        }
        if self.mode == PermissionMode::Auto
            && class == ToolClass::Shell
            && permission_risk::routine_local_shell(
                tool,
                args,
                &subject,
                cwd,
                &self.filesystem_boundary,
            )
        {
            return PermissionVerdict::Allow;
        }
        let safe_file = matches!(
            tool,
            "write" | "edit" | "notebook_edit" | "read" | "grep" | "find" | "ls"
        ) && targets.len() == 1
            && targets.iter().all(|target| {
                !target.outside
                    && !target.protected
                    && !target.secret
                    && !target.destructive
                    && !target.symlink_escape
            });
        let safe_shell = class == ToolClass::Shell
            // Allow rules are matched against individual parsed segments.
            // Persist only a subject that will match that same representation.
            && subjects.as_slice() == [subject.clone()]
            && permission_risk::routine_local_shell(
                tool,
                args,
                &subject,
                cwd,
                &self.filesystem_boundary,
            );
        let rule = PermissionRule::subject(normalize_tool_name(tool), &subject);
        let narrow = !subject.is_empty()
            && subject.len() <= 512
            && !subject
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '*' | '?'))
            && PermissionRule::parse(&rule.to_string()).as_ref() == Some(&rule);
        let persistent = (safe_file || safe_shell) && narrow;
        PermissionVerdict::Ask(ToolApprovalRequest {
            legal_choices: crate::approval::offer_scopes(
                !persistent,
                persistent,
                persistent && self.project_trusted,
            ),
            tool_call_id: tool_call_id.to_string(),
            tool: tool.to_string(),
            args: args.clone(),
            summary: crate::approval::display_text(&summary_of(tool, &subject)),
            session_rule: if persistent {
                rule.to_string()
            } else {
                String::new()
            },
            subject,
            outside_project,
            mode: self.mode,
        })
    }

    /// Deny if any task is denied, ask if at least one task needs approval,
    /// and allow only when every task is allowed.
    fn decide_agent_batch(
        &self,
        tool_call_id: &str,
        tasks: &[Value],
        cwd: &Path,
    ) -> PermissionVerdict {
        let mut first_ask = None;
        for task in tasks {
            match self.decide(tool_call_id, "agent", task, cwd) {
                PermissionVerdict::Allow => {}
                deny @ PermissionVerdict::Deny { .. } => return deny,
                PermissionVerdict::Ask(request) => {
                    first_ask.get_or_insert(request);
                }
            }
        }
        match first_ask {
            Some(mut request) => {
                let task_rows = tasks
                    .iter()
                    .enumerate()
                    .map(|(index, task)| {
                        let prompt = task
                            .get("prompt")
                            .and_then(Value::as_str)
                            .unwrap_or("<missing prompt>")
                            .trim();
                        let (subject, _) = subject_of_with_boundary(
                            "agent",
                            task,
                            cwd,
                            Some(&self.filesystem_boundary),
                        );
                        format!("{}. {} [{}]", index + 1, prompt, subject)
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                request.args = serde_json::json!({"tasks": tasks});
                request.subject = format!("{} agent tasks", tasks.len());
                request.summary = crate::approval::display_text(&format!(
                    "Agent batch: {task_rows}"
                ));
                // A mixed batch can carry different prompts, tool sets, and
                // isolation modes. A durable grant derived from one member is
                // not a safe authorization for the whole batch.
                request.session_rule.clear();
                request.legal_choices = crate::approval::offer_scopes(true, false, false);
                PermissionVerdict::Ask(request)
            }
            None => PermissionVerdict::Allow,
        }
    }

    fn agent_call_is_read_only(&self, args: &Value) -> bool {
        if args.get("isolation").and_then(Value::as_str) == Some("worktree") {
            return false;
        }
        let Some(tools) = args.get("tools") else {
            return true;
        };
        let Some(tools) = tools.as_array() else {
            return false;
        };
        tools.iter().all(|tool| {
            tool.as_str().is_some_and(|name| {
                name != "agent"
                    && matches!(self.class_of(name), ToolClass::Read | ToolClass::Network)
            })
        })
    }

    pub fn class_of(&self, tool: &str) -> ToolClass {
        if tool.starts_with("mcp__") && self.mcp_read_only.contains(tool) {
            ToolClass::Read
        } else {
            tool_class(tool)
        }
    }

    /// Add a rule the user granted for the rest of the run.
    pub fn remember(&mut self, rule: &str) {
        if let Some(rule) = PermissionRule::parse(rule) {
            if !self.session_allow.contains(&rule) {
                self.session_allow.push(rule);
            }
        }
    }
}

/// What a rule is matched against, and whether a file target lies outside
/// the project.
pub fn subject_of(tool: &str, args: &Value, cwd: &Path) -> (String, bool) {
    subject_of_with_boundary(tool, args, cwd, None)
}

pub fn subject_of_with_boundary(
    tool: &str,
    args: &Value,
    cwd: &Path,
    boundary: Option<&FilesystemBoundaryPolicy>,
) -> (String, bool) {
    if tool == "process_start" {
        let executable = args
            .get("executable")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let argv = args
            .get("argv")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return (crate::shell_policy::argv_subject(executable, &argv), false);
    }
    if matches!(
        tool,
        "process_write" | "process_status" | "process_output" | "process_stop"
    ) {
        return (
            format!(
                "process:{}",
                args.get("id").and_then(Value::as_u64).unwrap_or(0)
            ),
            false,
        );
    }
    if tool == "process_list" {
        return ("owned processes".into(), false);
    }
    match tool_class(tool) {
        ToolClass::Shell => (
            args.get("command")
                .or_else(|| args.get("cmd"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            false,
        ),
        ToolClass::Read | ToolClass::Edit => {
            if matches!(tool, "apply_patch" | "patch_preview") {
                if let Some(input) = args.get("input").and_then(Value::as_str) {
                    if let Ok(parsed) = crate::apply_patch::parse_codex_patch(input) {
                        let mut outside = false;
                        let mut paths = Vec::new();
                        for action in parsed.actions {
                            let p = match action {
                                crate::apply_patch::FileAction::Add { path, .. } => path,
                                crate::apply_patch::FileAction::Update { path, .. } => path,
                                crate::apply_patch::FileAction::Delete { path } => path,
                            };
                            let (rel, is_out) = project_relative_with_boundary(cwd, &p, boundary);
                            if is_out {
                                outside = true;
                            }
                            paths.push(rel);
                        }
                        return (paths.join(", "), outside);
                    }
                }
            }
            if matches!(tool, "patch_apply" | "patch_status" | "patch_rollback") {
                let targets = crate::runtime::contracts::extract_tool_targets(tool, args);
                let subjects: Vec<_> = targets
                    .iter()
                    .map(|p| project_relative_with_boundary(cwd, p, boundary))
                    .collect();
                return (
                    subjects
                        .iter()
                        .map(|(p, _)| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    subjects.iter().any(|(_, outside)| *outside),
                );
            }
            let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
            project_relative_with_boundary(cwd, raw, boundary)
        }
        // A fetch is judged by where it goes, a search by what it asks.
        ToolClass::Network if matches!(tool, "web_fetch" | "visual_snapshot") => (
            host_of(args.get("url").and_then(Value::as_str).unwrap_or_default()),
            false,
        ),
        ToolClass::Network => (
            args.get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            false,
        ),
        _ if tool == "agent" => {
            let isolation = args
                .get("isolation")
                .and_then(Value::as_str)
                .unwrap_or("shared");
            (format!("isolation:{isolation}"), false)
        }
        ToolClass::Other => (String::new(), false),
    }
}

/// The simple commands a shell line chains: split on `&&`, `||`, `;`, `|`,
/// `&` and newlines outside quotes. Redirect duplication (`2>&1`, `&>`) is
/// not a separator. Empty segments are dropped, so `git status && ` is one
/// segment.
pub fn shell_segments(command: &str) -> Vec<String> {
    crate::shell_policy::split_shell_segments(command)
}

/// `$(…)`, backticks and process substitution run a program the rule never
/// named.
fn has_command_substitution(segment: &str) -> bool {
    crate::shell_policy::has_command_substitution(segment)
}

/// `bash · git status`, `write · src/lib.rs`, or the bare tool name.
/// `docs.rs` from `https://docs.rs/similar/latest/`: the host a fetch rule
/// names. Lower-cased, port and credentials dropped, scheme optional.
pub fn host_of(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    host.trim().to_ascii_lowercase()
}

pub fn summary_of(tool: &str, subject: &str) -> String {
    if subject.is_empty() {
        tool.to_string()
    } else {
        format!("{tool} · {subject}")
    }
}

/// Check if a path starts with a Windows drive prefix (`C:`, `D:`, etc.).
pub fn has_windows_drive_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_windows_unc_path(s: &str) -> bool {
    s.starts_with(r"\\") || s.starts_with("//")
}

/// Normalize path syntax independently from the host operating system.
/// Both slash styles are treated as separators so persisted Windows paths
/// have the same identity when evaluated on Linux/macOS runners.
pub(crate) fn normalize_portable_path_text(raw: &str) -> String {
    let trailing_separator = raw.ends_with('/') || raw.ends_with('\\');
    let normalized = raw.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let (prefix, rest, absolute) = if normalized.starts_with("//") {
        ("//".to_string(), normalized.trim_start_matches('/'), true)
    } else if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let after = &normalized[2..];
        if after.starts_with('/') {
            (format!("{drive}:/"), after.trim_start_matches('/'), true)
        } else {
            (format!("{drive}:"), after, false)
        }
    } else if normalized.starts_with('/') {
        ("/".to_string(), normalized.trim_start_matches('/'), true)
    } else {
        (String::new(), normalized.as_str(), false)
    };

    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }

    let body = parts.join("/");
    let mut result = match prefix.as_str() {
        "//" => format!("//{body}"),
        "/" => format!("/{body}"),
        _ if prefix.ends_with('/') => format!("{prefix}{body}"),
        _ => format!("{prefix}{body}"),
    };
    if trailing_separator && !result.is_empty() && !result.ends_with('/') {
        result.push('/');
    }
    result
}

/// Strip Windows extended verbatim path prefixes (`\\?\` and `\\?\UNC\`) for uniform path matching.
pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// Check if a path points to git metadata (e.g. within a `.git` directory or pointer).
pub fn is_git_metadata_path(path: &Path, repo_root: Option<&Path>, root: Option<&Path>) -> bool {
    let normalized = strip_verbatim_prefix(&normalize_lexically(path));

    if let Some(repo) = repo_root {
        let repo_git = strip_verbatim_prefix(&normalize_lexically(&repo.join(".git")));
        if normalized.starts_with(&repo_git) || normalized == repo_git {
            return true;
        }
    }

    if let Some(r) = root {
        let root_git = strip_verbatim_prefix(&normalize_lexically(&r.join(".git")));
        if normalized.starts_with(&root_git) || normalized == root_git {
            return true;
        }
    }

    // A `.git` component in an unrelated checkout is not our metadata.
    repo_root.is_none()
        && root.is_none()
        && normalized.components().any(|c| c.as_os_str() == ".git")
}

/// Check whether `target` escapes `root` either lexically or via symlinks.
/// Returns `(outside_lexical, symlink_escape)`.
pub fn check_path_boundary(root: &Path, target: &Path) -> (bool, bool) {
    let norm_root = strip_verbatim_prefix(&normalize_lexically(root));
    let norm_target = strip_verbatim_prefix(&normalize_lexically(target));

    // 1. Lexical check
    let outside_lexical = !norm_target.starts_with(&norm_root);

    if outside_lexical {
        return (true, false);
    }

    // 2. Symlink escape check
    let mut symlink_escape = false;

    if let (Ok(canon_root), Ok(canon_target)) = (root.canonicalize(), target.canonicalize()) {
        let clean_root = strip_verbatim_prefix(&canon_root);
        let clean_target = strip_verbatim_prefix(&canon_target);
        if !clean_target.starts_with(&clean_root) {
            symlink_escape = true;
        }
    } else if let Ok(canon_root) = root.canonicalize() {
        let clean_root = strip_verbatim_prefix(&canon_root);
        if let Ok(meta) = target.symlink_metadata() {
            if meta.file_type().is_symlink() {
                if let Ok(link) = target.read_link() {
                    let resolved = if link.is_absolute() {
                        link
                    } else if let Some(p) = target.parent() {
                        p.join(link)
                    } else {
                        link
                    };
                    let clean_resolved = strip_verbatim_prefix(&normalize_lexically(&resolved));
                    if !clean_resolved.starts_with(&norm_root) {
                        symlink_escape = true;
                    }
                }
            }
        }

        let mut ancestor = target;
        while let Some(parent) = ancestor.parent() {
            if parent.exists() {
                if let Ok(parent_canon) = parent.canonicalize() {
                    let clean_parent = strip_verbatim_prefix(&parent_canon);
                    if !clean_parent.starts_with(&clean_root) {
                        symlink_escape = true;
                    }
                }
                break;
            }
            ancestor = parent;
        }
    }

    if !symlink_escape {
        if let Ok(rel) = norm_target.strip_prefix(&norm_root) {
            let mut current = norm_root.clone();
            for component in rel.components() {
                current.push(component);
                if let Ok(meta) = current.symlink_metadata() {
                    if meta.file_type().is_symlink() {
                        if let Ok(link) = current.read_link() {
                            let resolved = if link.is_absolute() {
                                link
                            } else if let Some(p) = current.parent() {
                                p.join(link)
                            } else {
                                link
                            };
                            let clean_resolved =
                                strip_verbatim_prefix(&normalize_lexically(&resolved));
                            if !clean_resolved.starts_with(&norm_root) {
                                symlink_escape = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    (outside_lexical, symlink_escape)
}

/// Convenience helper: returns true if `target` is either outside `root` or an untrusted symlink escape.
pub fn is_outside_or_symlink_escape(root: &Path, target: &Path) -> (bool, bool) {
    check_path_boundary(root, target)
}

/// Returns true if target escapes root via symlinks.
pub fn is_symlink_escape(root: &Path, target: &Path) -> bool {
    check_path_boundary(root, target).1
}

/// A path as a rule sees it: forward slashes, relative to the project when
/// it is inside it. The second value says when it is not.
pub fn project_relative(cwd: &Path, raw: &str) -> (String, bool) {
    project_relative_with_boundary(cwd, raw, None)
}

pub fn project_relative_with_boundary(
    cwd: &Path,
    raw: &str,
    boundary: Option<&FilesystemBoundaryPolicy>,
) -> (String, bool) {
    let effective_root = boundary.and_then(|b| b.root.as_deref()).unwrap_or(cwd);
    let raw_trimmed = raw.trim();

    let is_diff_drive = if has_windows_drive_prefix(raw_trimmed) {
        let drive_char = raw_trimmed.chars().next().map(|c| c.to_ascii_uppercase());
        let root_drive_char = effective_root
            .to_string_lossy()
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase());
        drive_char != root_drive_char
    } else {
        false
    };

    let given = Path::new(raw_trimmed);
    let joined = if given.is_absolute()
        || has_windows_drive_prefix(raw_trimmed)
        || is_windows_unc_path(raw_trimmed)
    {
        PathBuf::from(raw_trimmed)
    } else {
        // The executor resolves relative paths from cwd, not the policy root.
        cwd.join(given)
    };

    let (outside_lexical, symlink_escape) = check_path_boundary(effective_root, &joined);
    let outside = is_diff_drive || outside_lexical || symlink_escape;

    let full = normalize_lexically(&joined);
    let root = normalize_lexically(effective_root);
    match full.strip_prefix(&root) {
        Ok(rest) if !outside => {
            let text = slashes(rest);
            (if text.is_empty() { ".".into() } else { text }, false)
        }
        _ => (slashes(&full), outside),
    }
}

/// Resolve `.` and `..` without touching the file system: the target may
/// not exist yet, and a rule is about where it would be.
pub fn normalize_lexically(path: &Path) -> PathBuf {
    PathBuf::from(normalize_portable_path_text(&path.to_string_lossy()))
}

pub fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The rule "allow for this session" adds: the program and its first
/// subcommand for a shell call (`git status *`, `cargo test *`, `rm *`), the
/// bare tool for everything else.
pub fn session_rule_for(tool: &str, subject: &str) -> PermissionRule {
    // A fetch grant covers the host, not the one page.
    if tool == "web_fetch" && !subject.is_empty() {
        return PermissionRule::subject(tool, subject);
    }
    if tool == "agent" && !subject.is_empty() {
        let specifier = if let Some(val) = subject.strip_prefix("isolation:") {
            Some(RuleSpecifier::Parameter {
                param: "isolation".to_string(),
                value: val.to_string(),
            })
        } else {
            Some(RuleSpecifier::Subject(subject.to_string()))
        };
        return PermissionRule {
            tool: tool.to_string(),
            pattern: Some(subject.to_string()),
            specifier,
        };
    }
    if tool_class(tool) != ToolClass::Shell || subject.is_empty() {
        return PermissionRule::bare(tool);
    }
    let mut words = subject.split_whitespace();
    let Some(program) = words.next() else {
        return PermissionRule::bare(tool);
    };
    let mut prefix = program.to_string();
    // A script or a path has no subcommands: `./run.sh now` is one program.
    let bare_program = !program.contains('/') && !program.contains('\\') && !program.contains('.');
    if let Some(first) = words.next().filter(|_| bare_program) {
        let is_flag = first.starts_with('-');
        let is_path = first.contains('/') || first.contains('\\') || first.contains('.');
        let is_operator = matches!(first, "&&" | "||" | "|" | ";" | ">" | ">>" | "<");
        if !is_flag && !is_path && !is_operator {
            prefix.push(' ');
            prefix.push_str(first);
        }
    }
    PermissionRule::subject(tool, format!("{prefix} *"))
}

#[cfg(test)]
#[path = "permission_tests.rs"]
mod tests;
