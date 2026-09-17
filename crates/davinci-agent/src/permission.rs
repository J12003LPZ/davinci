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
                "workspace edits and recognized local checks run; risky or unknown actions ask"
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
    /// Reaches outside the machine (`web_fetch`, `web_search`): allowed in
    /// Plan Mode for research; other prompted modes require approval.
    Network,
    Other,
}

/// Shared sensitive-file classification for auxiliary readers such as plan
/// evidence. Auxiliary readers may be stricter, never less strict, than policy.
pub fn is_sensitive_file_path(path: &str) -> bool {
    permission_risk::is_protected_path(path)
}

pub fn tool_class(tool: &str) -> ToolClass {
    match tool {
        "process_status" | "process_output" | "process_list" => ToolClass::Read,
        "process_start" | "process_write" => ToolClass::Shell,
        "process_stop" => ToolClass::Other,
        "test_related" | "test_impacted" | "test_plan" => ToolClass::Read,
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
    if let Some(tasks) = args.get("tasks").and_then(Value::as_array) {
        if let Some(first_task) = tasks.first() {
            if let Some(val) = first_task.get(param) {
                if !val.is_null() {
                    return Some(val);
                }
            }
        }
    }
    None
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
        let class = self.class_of(tool);
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
        if class == ToolClass::Network && self.mode == PermissionMode::ReadOnly {
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
            if tool == "apply_patch" {
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
                .or_else(|| {
                    args.get("tasks")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.first())
                        .and_then(|t| t.get("isolation"))
                        .and_then(Value::as_str)
                })
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
mod tests {
    use super::*;
    use serde_json::json;

    fn cwd() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:\\work\\proj")
        } else {
            PathBuf::from("/work/proj")
        }
    }

    #[test]
    fn plan_evidence_obeys_explicit_read_denies_even_with_blanket_allow() {
        for mode in PermissionMode::ALL {
            let mut p = policy(mode);
            p.allow.push(PermissionRule::bare("*"));
            p.deny
                .push(PermissionRule::parse("read(private/**)").unwrap());
            assert!(
                is_deny(&verdict(
                    &p,
                    "propose_plan",
                    json!({
                        "expected_revision":0,
                        "evidence":[{"path":"private/notes.txt","finding":"Not authorized"}]
                    })
                )),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn five_named_modes_are_available_in_the_requested_order() {
        let names = [
            "Manual",
            "Accept Edits",
            "Plan Mode",
            "Auto Mode",
            "Always Approve",
        ];
        assert_eq!(PermissionMode::ALL.len(), names.len());
        for (mode, name) in PermissionMode::ALL.iter().zip(names) {
            assert_eq!(PermissionMode::parse(name), Some(*mode), "{name}");
        }
    }

    #[test]
    fn plan_permission_cannot_be_overridden_by_saved_allow_rules() {
        let mut p = policy(PermissionMode::ReadOnly);
        p.allow.push(PermissionRule::bare("*"));
        for (tool, args) in [
            ("write", json!({"path":"src/main.rs", "content":"changed"})),
            ("bash", json!({"command":"rm -rf src"})),
            ("job_kill", json!({"job_id":"1"})),
            (
                "write_stdin",
                json!({"session_id":"1", "chars":"rm -rf src\n"}),
            ),
        ] {
            assert!(
                is_deny(&verdict(&p, tool, args)),
                "Plan Mode allowed {tool}"
            );
        }
    }

    #[test]
    fn auto_escalates_external_destructive_and_sensitive_actions() {
        let p = policy(PermissionMode::Auto);
        for (tool, args) in [
            ("write", json!({"path":"../outside.txt"})),
            ("write", json!({"path":".davinci/settings.json"})),
            ("read", json!({"path":".env"})),
            ("bash", json!({"command":"git push origin main"})),
            ("bash", json!({"command":"rm -rf src"})),
            ("bash", json!({"command":"curl https://example.com"})),
            (
                "bash",
                json!({"command":"python -c 'import os; os.remove(\"a\")'"}),
            ),
            ("unknown_tool", json!({})),
        ] {
            assert!(
                is_ask(&verdict(&p, tool, args.clone())),
                "Auto allowed {tool}: {args}"
            );
        }
        assert_eq!(
            verdict(&p, "write", json!({"path":"src/main.rs"})),
            PermissionVerdict::Allow
        );
        assert_eq!(
            verdict(&p, "bash", json!({"command":"cargo test --offline"})),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn multi_file_patch_cannot_hide_a_sensitive_target() {
        let p = policy(PermissionMode::Edits);
        let patch = "*** Begin Patch\n*** Add File: src/ok.rs\n+ok\n*** Add File: .davinci/settings.json\n+{}\n*** End Patch";
        assert!(is_ask(&verdict(&p, "apply_patch", json!({"input":patch}))));
    }

    #[test]
    fn five_mode_names_and_order_are_public_contract() {
        let names: Vec<_> = PermissionMode::ALL
            .iter()
            .map(|mode| mode.as_str())
            .collect();
        assert_eq!(
            names,
            ["ask", "edits", "read-only", "auto", "always-approve"]
        );
        for name in names {
            assert_eq!(PermissionMode::parse(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn auto_mode_escalates_unknown_destructive_and_external_actions() {
        let p = policy(PermissionMode::Auto);
        for command in [
            "rm -rf build",
            "git push origin main",
            "curl https://example.com/upload",
            "unknown-tool",
            "git status && rm x",
        ] {
            assert!(
                is_ask(&verdict(&p, "bash", json!({"command": command}))),
                "{command}"
            );
        }
        assert!(is_ask(&verdict(
            &p,
            "write",
            json!({"path": "../outside.txt"})
        )));
        assert!(is_ask(&verdict(&p, "custom_tool", json!({}))));
        assert_eq!(
            verdict(&p, "bash", json!({"command": "git status"})),
            PermissionVerdict::Allow
        );
        assert_eq!(
            verdict(&p, "write", json!({"path": "src/lib.rs"})),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn plan_mode_cannot_be_overridden_by_allow_rules() {
        let mut p = policy(PermissionMode::ReadOnly);
        p.allow.push(PermissionRule::bare("*"));
        for (tool, args) in [
            ("write", json!({"path": "x.rs"})),
            ("bash", json!({"command": "rm x"})),
            ("job_kill", json!({})),
            ("agent_message", json!({})),
        ] {
            assert!(is_deny(&verdict(&p, tool, args)), "{tool}");
        }
    }

    #[test]
    fn accept_edits_requires_approval_for_davinci_git_and_secret_paths() {
        let p = policy(PermissionMode::Edits);
        for path in [
            ".davinci/settings.json",
            ".git/config",
            ".env",
            "config/credentials.json",
        ] {
            assert!(
                is_ask(&verdict(&p, "write", json!({"path": path}))),
                "{path}"
            );
        }
        let patch = "*** Begin Patch\n*** Add File: src/a.rs\n+ok\n*** Add File: .davinci/settings.json\n+{}\n*** End Patch";
        assert!(is_ask(&verdict(&p, "apply_patch", json!({"input": patch}))));
    }

    #[test]
    fn mode_labels_cycle_and_legacy_ids_remain_compatible() {
        let ids = ["ask", "edits", "read-only", "auto", "always-approve"];
        for (index, mode) in PermissionMode::ALL.into_iter().enumerate() {
            assert_eq!(mode.as_str(), ids[index]);
            assert_eq!(PermissionMode::parse(mode.label()), Some(mode));
            assert_eq!(mode.next(), PermissionMode::ALL[(index + 1) % ids.len()]);
        }
        for alias in [
            "full-access",
            "danger-full-access",
            "yolo",
            "bypass",
            "autopilot",
        ] {
            assert_eq!(
                PermissionMode::parse(alias),
                Some(PermissionMode::AlwaysApprove)
            );
        }
        assert_eq!(
            PermissionPolicy::default().mode,
            PermissionMode::AlwaysApprove
        );
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);
    }

    #[test]
    fn every_mode_honors_explicit_denies_and_hard_read_boundaries() {
        for mode in PermissionMode::ALL {
            let mut p = policy(mode);
            p.allow.push(PermissionRule::bare("*"));
            p.remember("*");
            p.filesystem_boundary.allow_git_metadata = true;
            p.filesystem_boundary.root = Some(cwd());
            p.deny.push(PermissionRule::parse("write(.git/*)").unwrap());
            assert!(
                is_deny(&verdict(&p, "write", json!({"path":".git/config"}))),
                "{mode:?}"
            );
            p.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);
            assert!(
                is_deny(&verdict(&p, "read", json!({"path":"../outside.txt"}))),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn plan_freeze_precedes_session_grants_and_side_effect_class_hints() {
        let mut p = policy(PermissionMode::ReadOnly);
        p.remember("*");
        // MCP hints cannot reclassify a built-in mutation as a safe read.
        p.mcp_read_only.insert("job_kill".into());
        for tool in [
            "job_kill",
            "agent_message",
            "agent_stop",
            "task_create",
            "task_update",
            "graph_submit",
            "unknown_tool",
        ] {
            assert!(is_deny(&verdict(&p, tool, json!({}))), "{tool}");
        }
        for tool in [
            "read",
            "grep",
            "find",
            "ls",
            "todo",
            "update_plan",
            "job_output",
            "task_list",
            "task_get",
        ] {
            assert_eq!(
                verdict(&p, tool, json!({})),
                PermissionVerdict::Allow,
                "{tool}"
            );
        }
    }

    #[test]
    fn patch_rules_and_risk_checks_inspect_each_target() {
        let patch = "*** Begin Patch\n*** Add File: src/good.rs\n+ok\n*** Add File: private/key.pem\n+not-a-real-key\n*** End Patch";
        let args = json!({"input":patch});
        for mode in [
            PermissionMode::Ask,
            PermissionMode::Edits,
            PermissionMode::Auto,
        ] {
            let mut p = policy(mode);
            p.allow
                .push(PermissionRule::parse("apply_patch(src/*)").unwrap());
            assert!(
                is_ask(&verdict(&p, "apply_patch", args.clone())),
                "{mode:?}"
            );
            p.deny
                .push(PermissionRule::parse("apply_patch(private/*)").unwrap());
            assert!(
                is_deny(&verdict(&p, "apply_patch", args.clone())),
                "{mode:?}"
            );
        }
        for path in [
            ".pi/settings.json",
            ".davinci/settings.json",
            ".git/config",
            ".env.local",
            "../outside.txt",
        ] {
            for action in [
                format!("*** Add File: {path}\n+x"),
                format!("*** Delete File: {path}"),
                format!("*** Update File: {path}\n@@\n-old\n+new"),
            ] {
                let input = format!(
                    "*** Begin Patch\n*** Add File: src/good.rs\n+ok\n{action}\n*** End Patch"
                );
                assert!(
                    is_ask(&verdict(
                        &policy(PermissionMode::Edits),
                        "apply_patch",
                        json!({"input":input})
                    )),
                    "{path}"
                );
            }
        }
    }

    #[test]
    fn auto_shell_matrix_escalates_unknown_syntax_network_and_path_changes() {
        let p = policy(PermissionMode::Auto);
        for command in [
            "git status && cargo test --offline",
            "cargo check --offline",
            "git diff --stat",
            "pwd",
        ] {
            assert_eq!(
                verdict(&p, "bash", json!({"command":command})),
                PermissionVerdict::Allow,
                "{command}"
            );
        }
        for command in [
            "git status && curl https://example.com",
            "git status; rm x",
            "git status $(whoami)",
            "git log `whoami`",
            "diff <(cat a) b",
            "echo ok > out.txt",
            "echo 'unterminated",
            "git status &",
            "git -C ../elsewhere status",
            "cat ../outside.txt",
            "cat /etc/passwd",
            "cat .env",
            "cat config/credentials.json",
            "git show HEAD:.env",
            "cargo test --offline --manifest-path ../Cargo.toml",
            "cargo test --offline --target-dir ../build",
            "cargo test",
            "cargo build",
            "npx jest",
            "npm view package",
            "python -c 'print(1)'",
            "node --test --require=helper.js",
            "find . -delete",
            "git branch new-branch",
        ] {
            assert!(
                is_ask(&verdict(&p, "bash", json!({"command":command}))),
                "{command}"
            );
        }
        assert!(is_ask(&verdict(&p, "exec_command", json!({"cmd":"rm x"}))));
        assert!(is_ask(&verdict(
            &p,
            "bash",
            json!({"command":"git status", "cwd":"../other"})
        )));
        assert!(is_ask(&verdict(
            &p,
            "write_stdin",
            json!({"command":"git status"})
        )));
    }

    #[test]
    fn malformed_shell_chains_are_not_routine_commands() {
        for command in [
            "git status && && git diff",
            "git status || | git diff",
            "git status;;git diff",
        ] {
            assert!(
                is_ask(&verdict(
                    &policy(PermissionMode::Auto),
                    "bash",
                    json!({"command":command})
                )),
                "{command}"
            );
        }
    }

    #[test]
    fn patch_symlinks_and_isolated_boundaries_apply_in_every_mode() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        for mode in PermissionMode::ALL {
            let mut p = policy(mode);
            p.filesystem_boundary.root = Some(root.clone());
            p.filesystem_boundary.enforce_root_for_mutations = true;
            p.allow.push(PermissionRule::bare("*"));
            let patch = "*** Begin Patch\n*** Add File: good.rs\n+ok\n*** Add File: ../outside/bad.rs\n+bad\n*** End Patch";
            assert!(
                is_deny(&p.decide("test", "apply_patch", &json!({"input":patch}), &root)),
                "{mode:?}"
            );
        }
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, root.join("link")).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, root.join("link")).is_ok();
        if linked {
            let patch = "*** Begin Patch\n*** Add File: good.rs\n+ok\n*** Add File: link/new.rs\n+bad\n*** End Patch";
            let p = policy(PermissionMode::AlwaysApprove);
            assert!(is_deny(&p.decide(
                "test",
                "apply_patch",
                &json!({"input":patch}),
                &root
            )));
        }
    }

    #[test]
    fn relative_targets_are_resolved_from_execution_cwd_not_policy_root() {
        let root = cwd();
        let execution_cwd = root.join("nested");
        let mut p = policy(PermissionMode::Edits);
        p.filesystem_boundary.root = Some(root.clone());
        p.deny
            .push(PermissionRule::parse("write(nested/blocked.rs)").unwrap());
        assert!(is_deny(&p.decide(
            "test",
            "write",
            &json!({"path":"blocked.rs"}),
            &execution_cwd
        )));
        assert_eq!(
            project_relative_with_boundary(
                &execution_cwd,
                "../ok.rs",
                Some(&p.filesystem_boundary)
            ),
            ("ok.rs".into(), false)
        );
    }

    #[test]
    fn shell_workdir_changes_cannot_hide_an_outside_operand() {
        let root = cwd();
        let execution_cwd = root.join("nested");
        let mut p = policy(PermissionMode::Auto);
        p.filesystem_boundary.root = Some(root);
        for key in ["cwd", "workdir", "work_dir", "directory"] {
            let mut args = json!({"command":"cat ../outside.txt"});
            args[key] = json!("..");
            assert!(
                is_ask(&p.decide("test", "bash", &args, &execution_cwd)),
                "{key}"
            );
        }
    }

    #[test]
    fn explicit_shell_denies_survive_substitution_and_always_approve() {
        let mut p = policy(PermissionMode::AlwaysApprove);
        p.deny.push(PermissionRule::parse("bash(rm *)").unwrap());
        for command in ["echo ok && rm file", "echo $(rm file)", "echo `rm file`"] {
            assert!(
                is_deny(&verdict(&p, "bash", json!({"command":command}))),
                "{command}"
            );
        }
    }

    #[test]
    fn malformed_patches_never_gain_automatic_edit_permission() {
        for mode in [PermissionMode::Edits, PermissionMode::Auto] {
            for args in [
                json!({}),
                json!({"input":"not a patch"}),
                json!({"input":"*** Begin Patch\n*** End Patch"}),
            ] {
                assert!(
                    is_deny(&verdict(&policy(mode), "apply_patch", args)),
                    "{mode:?}"
                );
            }
        }
    }

    #[test]
    fn rules_parse_bare_tools_and_patterns() {
        assert_eq!(
            PermissionRule::parse("bash"),
            Some(PermissionRule {
                tool: "bash".into(),
                pattern: None,
                specifier: None,
            })
        );
        assert_eq!(
            PermissionRule::parse(" bash(git *) "),
            Some(PermissionRule {
                tool: "bash".into(),
                pattern: Some("git *".into()),
                specifier: Some(RuleSpecifier::Subject("git *".into())),
            })
        );
        assert_eq!(PermissionRule::parse("bash()").unwrap().pattern, None);
        assert_eq!(PermissionRule::parse(""), None);
        assert_eq!(PermissionRule::parse("bash(git"), None);
        assert_eq!(PermissionRule::parse("two words"), None);
        assert_eq!(
            PermissionRule::parse("bash(git *)").unwrap().to_string(),
            "bash(git *)"
        );
    }

    #[test]
    fn globs_match_runs_single_characters_and_slashes() {
        assert!(glob_matches("git *", "git status"));
        assert!(glob_matches("*", "anything at all"));
        assert!(glob_matches("src/**", "src/a/b/c.rs"));
        assert!(glob_matches("src/*.rs", "src/lib.rs"));
        assert!(glob_matches("?at", "cat"));
        assert!(!glob_matches("?at", "at"));
        assert!(!glob_matches("git *", "gitk"));
        assert!(!glob_matches("src/*.rs", "src/lib.ts"));
        assert!(glob_matches("a*b*c", "aXXbYYc"));
        assert!(!glob_matches("a*b*c", "aXXbYY"));
        assert!(glob_matches("", ""));
        assert!(!glob_matches("", "x"));
    }

    #[test]
    fn a_trailing_star_rule_also_means_the_bare_prefix() {
        let rule = PermissionRule::parse("bash(git status *)").unwrap();
        assert!(rule.matches("bash", "git status"));
        assert!(rule.matches("bash", "git status --short"));
        assert!(!rule.matches("bash", "git stash"));
        assert!(!rule.matches("powershell", "git status"));
    }

    #[test]
    fn a_bare_rule_covers_every_call_of_the_tool() {
        let rule = PermissionRule::parse("write").unwrap();
        assert!(rule.matches("write", "src/lib.rs"));
        assert!(rule.matches("write", ""));
        assert!(!rule.matches("edit", "src/lib.rs"));
    }

    #[test]
    fn a_pattern_rule_never_matches_a_tool_without_a_subject() {
        let rule = PermissionRule::parse("vector_search(*)").unwrap();
        assert!(!rule.matches("vector_search", ""));
    }

    #[test]
    fn subjects_are_commands_or_project_relative_paths() {
        let cwd = cwd();
        assert_eq!(
            subject_of("bash", &json!({"command": "  git status \n"}), &cwd),
            ("git status".into(), false)
        );
        assert_eq!(
            subject_of("write", &json!({"path": "src\\lib.rs"}), &cwd),
            ("src/lib.rs".into(), false)
        );
        assert_eq!(
            subject_of(
                "read",
                &json!({"path": cwd.join("a").join("b.txt").to_string_lossy()}),
                &cwd
            ),
            ("a/b.txt".into(), false)
        );
        assert_eq!(
            subject_of("edit", &json!({"path": "src/../../secret"}), &cwd),
            (
                if cfg!(windows) {
                    "C:/work/secret".to_string()
                } else {
                    "/work/secret".to_string()
                },
                true
            )
        );
        assert_eq!(subject_of("ls", &json!({}), &cwd), (".".into(), false));
        assert_eq!(
            subject_of("vector_search", &json!({"q": "x"}), &cwd),
            (String::new(), false)
        );
    }

    #[test]
    fn session_rules_name_the_program_and_its_subcommand() {
        let rule = |command: &str| session_rule_for("bash", command).to_string();
        assert_eq!(rule("git status --short"), "bash(git status *)");
        assert_eq!(rule("cargo test -p davinci-agent"), "bash(cargo test *)");
        assert_eq!(rule("rm -rf build"), "bash(rm *)");
        assert_eq!(rule("./run.sh now"), "bash(./run.sh *)");
        assert_eq!(rule("python script.py"), "bash(python *)");
        assert_eq!(rule("ls && rm x"), "bash(ls *)");
        assert_eq!(rule("make"), "bash(make *)");
        assert_eq!(session_rule_for("write", "src/lib.rs").to_string(), "write");
        assert_eq!(
            session_rule_for("vector_search", "").to_string(),
            "vector_search"
        );
    }

    fn policy(mode: PermissionMode) -> PermissionPolicy {
        PermissionPolicy::new(mode)
    }

    fn verdict(policy: &PermissionPolicy, tool: &str, args: Value) -> PermissionVerdict {
        policy.decide("call_1", tool, &args, &cwd())
    }

    fn is_ask(verdict: &PermissionVerdict) -> bool {
        matches!(verdict, PermissionVerdict::Ask(_))
    }

    fn is_deny(verdict: &PermissionVerdict) -> bool {
        matches!(verdict, PermissionVerdict::Deny { .. })
    }

    #[test]
    fn the_mode_table_decides_what_no_rule_covers() {
        let read = json!({"path": "a.txt"});
        let write = json!({"path": "a.txt", "content": ""});
        let outside = json!({"path": "../elsewhere.txt", "content": ""});
        let shell = json!({"command": "git status"});
        let other = json!({});

        let p = policy(PermissionMode::ReadOnly);
        assert_eq!(verdict(&p, "read", read.clone()), PermissionVerdict::Allow);
        assert!(is_deny(&verdict(&p, "write", write.clone())));
        assert!(is_deny(&verdict(&p, "bash", shell.clone())));
        assert!(is_deny(&verdict(&p, "vector_search", other.clone())));

        let p = policy(PermissionMode::Ask);
        assert_eq!(
            verdict(&p, "grep", json!({"pattern": "x"})),
            PermissionVerdict::Allow
        );
        assert!(is_ask(&verdict(&p, "write", write.clone())));
        assert!(is_ask(&verdict(&p, "bash", shell.clone())));
        assert!(is_ask(&verdict(&p, "vector_search", other.clone())));

        let p = policy(PermissionMode::Edits);
        assert_eq!(
            verdict(&p, "write", write.clone()),
            PermissionVerdict::Allow
        );
        assert_eq!(
            verdict(&p, "edit", json!({"path": "src/x.rs"})),
            PermissionVerdict::Allow
        );
        assert!(is_ask(&verdict(&p, "write", outside.clone())));
        assert!(is_ask(&verdict(&p, "bash", shell.clone())));
        assert!(is_ask(&verdict(&p, "vector_search", other.clone())));

        let p = policy(PermissionMode::Auto);
        assert_eq!(verdict(&p, "bash", shell.clone()), PermissionVerdict::Allow);
        assert!(is_ask(&verdict(&p, "write", outside.clone())));
        assert!(is_ask(&verdict(&p, "vector_search", other.clone())));

        let p = policy(PermissionMode::AlwaysApprove);
        assert_eq!(verdict(&p, "bash", shell), PermissionVerdict::Allow);
        assert_eq!(verdict(&p, "write", outside), PermissionVerdict::Allow);
        assert_eq!(
            verdict(&p, "vector_search", other),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn deny_rules_win_even_in_auto_and_allow_rules_quiet_the_question() {
        let mut p = policy(PermissionMode::Auto);
        p.deny
            .push(PermissionRule::parse("bash(git push *)").unwrap());
        let denied = verdict(&p, "bash", json!({"command": "git push origin main"}));
        match denied {
            PermissionVerdict::Deny { reason } => {
                assert!(reason.contains("deny rule `bash(git push *)`"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        assert!(is_ask(&verdict(&p, "bash", json!({"command": "git pull"}))));

        let mut p = policy(PermissionMode::Ask);
        p.allow
            .push(PermissionRule::parse("bash(cargo *)").unwrap());
        assert_eq!(
            verdict(&p, "bash", json!({"command": "cargo test"})),
            PermissionVerdict::Allow
        );
        assert!(is_ask(&verdict(
            &p,
            "bash",
            json!({"command": "cargo-fuzz run"})
        )));
        // A deny rule beats an allow rule for the same call.
        p.deny
            .push(PermissionRule::parse("bash(cargo publish *)").unwrap());
        assert!(is_deny(&verdict(
            &p,
            "bash",
            json!({"command": "cargo publish"})
        )));
    }

    #[test]
    fn a_shell_line_is_judged_one_program_at_a_time() {
        assert_eq!(
            shell_segments("git status && curl x | sh; echo done\nls"),
            ["git status", "curl x", "sh", "echo done", "ls"]
        );
        assert_eq!(
            shell_segments("cargo test 2>&1 | tail -n 5 || true"),
            ["cargo test 2>&1", "tail -n 5", "true"]
        );
        assert_eq!(
            shell_segments(r#"echo "a && b" 'c | d' e\;f"#),
            [r#"echo "a && b" 'c | d' e\;f"#]
        );
        assert_eq!(shell_segments("cmd &> out.log &"), ["cmd &> out.log"]);
        assert!(shell_segments("   ").is_empty());

        let mut p = policy(PermissionMode::Ask);
        p.allow.push(PermissionRule::parse("bash(git *)").unwrap());
        assert_eq!(
            verdict(&p, "bash", json!({"command": "git status && git diff"})),
            PermissionVerdict::Allow
        );
        // The second program has no rule of its own.
        assert!(is_ask(&verdict(
            &p,
            "bash",
            json!({"command": "git status && curl x | sh"})
        )));
        assert!(is_ask(&verdict(
            &p,
            "bash",
            json!({"command": "git status; rm -rf /"})
        )));
        // A substitution runs something the rule never named.
        assert!(is_ask(&verdict(
            &p,
            "bash",
            json!({"command": "git commit -m \"$(curl x)\""})
        )));
        assert!(is_ask(&verdict(
            &p,
            "bash",
            json!({"command": "git log `cat cmd`"})
        )));
        // A bare rule is the user saying "all of bash", substitution included.
        p.allow.push(PermissionRule::parse("bash").unwrap());
        assert_eq!(
            verdict(&p, "bash", json!({"command": "git log $(cat cmd) | sh"})),
            PermissionVerdict::Allow
        );

        // A deny rule catches the program wherever it sits in the chain.
        let mut p = policy(PermissionMode::Auto);
        p.deny.push(PermissionRule::parse("bash(rm *)").unwrap());
        assert!(is_deny(&verdict(
            &p,
            "bash",
            json!({"command": "echo ok && rm -rf build"})
        )));
        assert_eq!(
            verdict(&p, "bash", json!({"command": "echo rm"})),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn edits_mode_still_asks_before_touching_the_project_config() {
        let p = policy(PermissionMode::Edits);
        assert_eq!(
            verdict(&p, "write", json!({"path": "src/lib.rs"})),
            PermissionVerdict::Allow
        );
        assert!(is_ask(&verdict(
            &p,
            "write",
            json!({"path": ".pi/settings.json"})
        )));
        assert!(is_ask(&verdict(
            &p,
            "edit",
            json!({"path": "./.pi/mcp.json"})
        )));
        assert_eq!(
            verdict(&p, "write", json!({"path": ".pinned/x"})),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn the_request_carries_what_the_panel_and_the_rule_need() {
        let p = policy(PermissionMode::Ask);
        match verdict(&p, "bash", json!({"command": "git status --short"})) {
            PermissionVerdict::Ask(request) => {
                assert_eq!(request.tool_call_id, "call_1");
                assert_eq!(request.summary, "bash · git status --short");
                assert_eq!(request.session_rule, "bash(git status --short)");
                assert_eq!(request.mode, PermissionMode::Ask);
                assert!(!request.outside_project);
            }
            other => panic!("{other:?}"),
        }
        match verdict(&p, "write", json!({"path": "../out.txt", "content": "x"})) {
            PermissionVerdict::Ask(request) => {
                assert!(request.outside_project);
                assert!(request.session_rule.is_empty());
                assert!(!request.allows(ToolApprovalDecision::AllowForSession));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_remembered_rule_quiets_the_next_call_and_is_not_duplicated() {
        let mut p = policy(PermissionMode::Ask);
        let shell = json!({"command": "git status"});
        assert!(is_ask(&verdict(&p, "bash", shell.clone())));
        p.remember("bash(git status *)");
        p.remember("bash(git status *)");
        assert_eq!(p.session_allow.len(), 1);
        assert_eq!(verdict(&p, "bash", shell), PermissionVerdict::Allow);
        assert!(is_ask(&verdict(&p, "bash", json!({"command": "git push"}))));
    }

    #[test]
    fn modes_parse_their_own_names_and_the_codex_sandbox_names() {
        assert_eq!(PermissionMode::parse("ask"), Some(PermissionMode::Ask));
        assert_eq!(
            PermissionMode::parse("READ-ONLY"),
            Some(PermissionMode::ReadOnly)
        );
        assert_eq!(
            PermissionMode::parse("workspace-write"),
            Some(PermissionMode::Edits)
        );
        assert_eq!(
            PermissionMode::parse("full-access"),
            Some(PermissionMode::AlwaysApprove)
        );
        assert_eq!(PermissionMode::parse("nope"), None);
        for mode in PermissionMode::ALL {
            assert_eq!(PermissionMode::parse(mode.as_str()), Some(mode));
        }
    }

    #[test]
    fn the_network_tools_are_asked_about_by_host_and_never_touch_the_workspace() {
        assert_eq!(tool_class("web_fetch"), ToolClass::Network);
        assert_eq!(tool_class("web_search"), ToolClass::Network);
        assert_eq!(tool_class("todo"), ToolClass::Read);
        assert_eq!(tool_class("job_output"), ToolClass::Read);
        assert_eq!(tool_class("notebook_edit"), ToolClass::Edit);
        assert_eq!(tool_class("mcp_read"), ToolClass::Read);
        assert_eq!(tool_class("graph_submit"), ToolClass::Other);
        assert_eq!(tool_class("retrieve_output"), ToolClass::Read);
        assert_eq!(tool_class("graph_run"), ToolClass::Other);
        assert_eq!(tool_class("mcp__memory__echo"), ToolClass::Other);
        assert_eq!(
            host_of("https://user:pw@Docs.rs:443/similar/latest?x=1"),
            "docs.rs"
        );
        assert_eq!(host_of("example.com/page"), "example.com");

        let fetch = json!({"url": "https://docs.rs/similar/latest/"});
        let (subject, outside) = subject_of("web_fetch", &fetch, &cwd());
        assert_eq!(subject, "docs.rs");
        assert!(!outside);
        assert_eq!(
            session_rule_for("web_fetch", &subject).to_string(),
            "web_fetch(docs.rs)"
        );
        assert_eq!(
            session_rule_for("web_search", "rust diff").to_string(),
            "web_search"
        );

        let ask = PermissionPolicy::new(PermissionMode::Ask);
        assert!(matches!(
            ask.decide("c1", "web_fetch", &fetch, &cwd()),
            PermissionVerdict::Ask(request) if request.summary == "web_fetch · docs.rs"
        ));
        let read_only = PermissionPolicy::new(PermissionMode::ReadOnly);
        assert!(matches!(
            read_only.decide("c1", "web_fetch", &fetch, &cwd()),
            PermissionVerdict::Allow
        ));
        let edits = PermissionPolicy::new(PermissionMode::Edits);
        assert!(matches!(
            edits.decide("c1", "web_search", &json!({"query": "x"}), &cwd()),
            PermissionVerdict::Ask(_)
        ));
        let mut denied = PermissionPolicy::new(PermissionMode::Auto);
        denied.deny = vec![PermissionRule::parse("web_fetch(*.internal)").unwrap()];
        assert!(matches!(
            denied.decide(
                "c1",
                "web_fetch",
                &json!({"url": "http://wiki.corp.internal/x"}),
                &cwd()
            ),
            PermissionVerdict::Deny { .. }
        ));
        let mut granted = PermissionPolicy::new(PermissionMode::Ask);
        granted.allow = vec![PermissionRule::parse("web_fetch(docs.rs)").unwrap()];
        assert!(matches!(
            granted.decide("c1", "web_fetch", &fetch, &cwd()),
            PermissionVerdict::Allow
        ));
    }

    #[test]
    fn mcp_read_only_hint_is_read_and_a_deny_glob_still_wins() {
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy.mcp_read_only.insert("mcp__memory__echo".into());
        assert_eq!(policy.class_of("mcp__memory__echo"), ToolClass::Read);
        assert!(matches!(
            policy.decide("c1", "mcp__memory__echo", &json!({}), &cwd()),
            PermissionVerdict::Allow
        ));
        assert!(matches!(
            policy.decide("c1", "mcp__memory__write", &json!({}), &cwd()),
            PermissionVerdict::Ask(_)
        ));
        policy.deny = vec![PermissionRule::parse("mcp__memory__*").unwrap()];
        assert!(matches!(
            policy.decide("c1", "mcp__memory__echo", &json!({}), &cwd()),
            PermissionVerdict::Deny { .. }
        ));
        let read_only = PermissionPolicy::new(PermissionMode::ReadOnly);
        assert!(matches!(
            read_only.decide("c1", "mcp__docs__put", &json!({}), &cwd()),
            PermissionVerdict::Deny { .. }
        ));
        assert!(matches!(
            read_only.decide(
                "c1",
                "mcp_read",
                &json!({"server":"memory","uri":"x"}),
                &cwd()
            ),
            PermissionVerdict::Allow
        ));
    }

    #[test]
    fn plan_mode_freezes_mutations_and_keeps_reads() {
        let policy = PermissionPolicy::new(PermissionMode::ReadOnly);
        assert!(matches!(
            policy.decide("c1", "read", &json!({"path": "a.rs"}), &cwd()),
            PermissionVerdict::Allow
        ));
        assert!(matches!(
            policy.decide("c1", "todo", &json!({"items": []}), &cwd()),
            PermissionVerdict::Allow
        ));
        match policy.decide(
            "c1",
            "write",
            &json!({"path": "a.rs", "content": "x"}),
            &cwd(),
        ) {
            PermissionVerdict::Deny { reason } => {
                assert!(reason.contains("plan mode"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        match policy.decide("c1", "agent", &json!({"prompt": "x"}), &cwd()) {
            PermissionVerdict::Deny { reason } => {
                assert!(reason.contains("plan mode"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn apply_patch_subject_reports_modified_paths_and_detects_outside() {
        let root = cwd();
        let patch = r#"*** Begin Patch
*** Add File: src/inside.rs
+fn inside() {}
*** End Patch"#;
        let (subject, is_out) = subject_of("apply_patch", &json!({"input": patch}), &root);
        assert_eq!(subject, "src/inside.rs");
        assert!(!is_out);

        let patch_outside = r#"*** Begin Patch
*** Add File: ../outside.rs
+fn outside() {}
*** End Patch"#;
        let (_, is_out_bad) = subject_of("apply_patch", &json!({"input": patch_outside}), &root);
        assert!(is_out_bad);
    }

    #[test]
    fn agent_isolation_permission_rules() {
        let root = cwd();
        let wt_call = json!({"prompt": "edit", "isolation": "worktree"});
        let (subject, is_out) = subject_of("agent", &wt_call, &root);
        assert_eq!(subject, "isolation:worktree");
        assert!(!is_out);
        assert_eq!(
            session_rule_for("agent", &subject).to_string(),
            "agent(isolation:worktree)"
        );

        let mut policy = PermissionPolicy::new(PermissionMode::Auto);
        policy.deny = vec![PermissionRule::parse("agent(isolation:worktree)").unwrap()];
        assert!(matches!(
            policy.decide("c1", "agent", &wt_call, &root),
            PermissionVerdict::Deny { .. }
        ));

        let shared_call = json!({"prompt": "search", "isolation": "shared"});
        assert!(matches!(
            policy.decide("c2", "agent", &shared_call, &root),
            PermissionVerdict::Ask(_)
        ));
        policy.mode = PermissionMode::AlwaysApprove;
        assert_eq!(
            policy.decide("c3", "agent", &shared_call, &root),
            PermissionVerdict::Allow
        );
        assert!(is_deny(&policy.decide("c4", "agent", &wt_call, &root)));
    }

    #[test]
    fn deny_parent_checkout_blocks_parent_edits() {
        let parent_dir = cwd();
        let mut policy = PermissionPolicy::new(PermissionMode::Edits);
        policy.deny_parent_checkout(&parent_dir);

        let inside_parent = parent_dir.join("src").join("main.rs");
        let wt_dir = std::env::temp_dir().join("davinci").join("wt-123");
        let call =
            json!({"path": inside_parent.to_string_lossy().to_string(), "content": "mutated"});
        assert!(matches!(
            policy.decide("c1", "write", &call, &wt_dir),
            PermissionVerdict::Deny { .. }
        ));
    }

    #[test]
    fn rules_parse_parameter_syntax() {
        let r1 = PermissionRule::parse_with_diagnostic("Agent(model:anthropic/*)").unwrap();
        assert_eq!(r1.tool, "Agent");
        assert_eq!(
            r1.specifier,
            Some(RuleSpecifier::Parameter {
                param: "model".into(),
                value: "anthropic/*".into(),
            })
        );
        assert_eq!(r1.to_string(), "Agent(model:anthropic/*)");

        let r2 = PermissionRule::parse_with_diagnostic("Agent(isolation:worktree)").unwrap();
        assert_eq!(
            r2.specifier,
            Some(RuleSpecifier::Parameter {
                param: "isolation".into(),
                value: "worktree".into(),
            })
        );

        let r3 = PermissionRule::parse_with_diagnostic("Bash(run_in_background:true)").unwrap();
        assert_eq!(
            r3.specifier,
            Some(RuleSpecifier::Parameter {
                param: "run_in_background".into(),
                value: "true".into(),
            })
        );

        let r4 = PermissionRule::parse_with_diagnostic("Read(./secrets/**)").unwrap();
        assert_eq!(
            r4.specifier,
            Some(RuleSpecifier::Subject("./secrets/**".into()))
        );

        let r5 = PermissionRule::parse_with_diagnostic("WebFetch(domain:example.com)").unwrap();
        assert_eq!(
            r5.specifier,
            Some(RuleSpecifier::Parameter {
                param: "domain".into(),
                value: "example.com".into(),
            })
        );

        // Windows drive letters must not be parsed as parameter rules
        let r6 = PermissionRule::parse_with_diagnostic("Read(C:/secrets/**)").unwrap();
        assert_eq!(
            r6.specifier,
            Some(RuleSpecifier::Subject("C:/secrets/**".into()))
        );
        let r7 = PermissionRule::parse_with_diagnostic("Read(C:\\secrets\\**)").unwrap();
        assert_eq!(
            r7.specifier,
            Some(RuleSpecifier::Subject("C:\\secrets\\**".into()))
        );

        // URLs with schemes must not be parsed as parameter rules
        let r8 = PermissionRule::parse_with_diagnostic("WebFetch(https://example.com/*)").unwrap();
        assert_eq!(
            r8.specifier,
            Some(RuleSpecifier::Subject("https://example.com/*".into()))
        );

        // Host with port must not be parsed as parameter rule
        let r9 = PermissionRule::parse_with_diagnostic("WebFetch(example.com:8080)").unwrap();
        assert_eq!(
            r9.specifier,
            Some(RuleSpecifier::Subject("example.com:8080".into()))
        );

        // Wildcard and bare
        let r10 = PermissionRule::parse_with_diagnostic("Agent(*)").unwrap();
        assert_eq!(r10.specifier, Some(RuleSpecifier::Wildcard));
        assert_eq!(r10.to_string(), "Agent(*)");

        let r11 = PermissionRule::parse_with_diagnostic("Agent").unwrap();
        assert_eq!(r11.specifier, None);
        assert_eq!(r11.to_string(), "Agent");
    }

    #[test]
    fn rules_parse_malformed_diagnostics() {
        assert_eq!(
            PermissionRule::parse_with_diagnostic(""),
            Err(RuleParseError::Empty)
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("   "),
            Err(RuleParseError::Empty)
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("Tool("),
            Err(RuleParseError::MissingClosingParen)
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("Tool(param:)"),
            Err(RuleParseError::EmptyParameterValue)
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("Tool(:value)"),
            Err(RuleParseError::EmptyParameterName)
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("Tool(param:value)extra"),
            Err(RuleParseError::TrailingCharacters("extra".into()))
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("two words"),
            Err(RuleParseError::InvalidToolName("two words".into()))
        );
        assert_eq!(
            PermissionRule::parse_with_diagnostic("Tool(param:value))"),
            Err(RuleParseError::TrailingCharacters(")".into()))
        );
    }

    #[test]
    fn parameter_aware_wildcard_matching() {
        let root = cwd();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy
            .allow
            .push(PermissionRule::parse("Agent(model:*)").unwrap());

        // Model present -> matches
        let call_with_model = json!({"prompt": "hi", "model": "anthropic/claude-3-5-sonnet"});
        assert_eq!(
            policy.decide("c1", "agent", &call_with_model, &root),
            PermissionVerdict::Allow
        );

        // Model omitted -> does not match
        let call_without_model = json!({"prompt": "hi"});
        assert!(matches!(
            policy.decide("c2", "agent", &call_without_model, &root),
            PermissionVerdict::Ask(_)
        ));

        // Agent(*) matches even without model
        let mut wildcard_policy = PermissionPolicy::new(PermissionMode::Ask);
        wildcard_policy
            .allow
            .push(PermissionRule::parse("Agent(*)").unwrap());
        assert_eq!(
            wildcard_policy.decide("c3", "agent", &call_without_model, &root),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn parameter_aware_omitted_parameter() {
        let root = cwd();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy
            .allow
            .push(PermissionRule::parse("Bash(run_in_background:true)").unwrap());

        // Omitted parameter -> does not match
        let call_sync = json!({"command": "cargo test"});
        assert!(matches!(
            policy.decide("c1", "bash", &call_sync, &root),
            PermissionVerdict::Ask(_)
        ));

        // Provided true -> matches
        let call_bg = json!({"command": "cargo test", "run_in_background": true});
        assert_eq!(
            policy.decide("c2", "bash", &call_bg, &root),
            PermissionVerdict::Allow
        );

        // Provided false -> does not match
        let call_not_bg = json!({"command": "cargo test", "run_in_background": false});
        assert!(matches!(
            policy.decide("c3", "bash", &call_not_bg, &root),
            PermissionVerdict::Ask(_)
        ));
    }

    #[test]
    fn parameter_aware_exact_value() {
        let root = cwd();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy
            .allow
            .push(PermissionRule::parse("Agent(isolation:worktree)").unwrap());

        let call_wt = json!({"prompt": "edit", "isolation": "worktree"});
        assert_eq!(
            policy.decide("c1", "agent", &call_wt, &root),
            PermissionVerdict::Allow
        );

        let call_shared = json!({"prompt": "edit", "isolation": "shared"});
        assert!(matches!(
            policy.decide("c2", "agent", &call_shared, &root),
            PermissionVerdict::Ask(_)
        ));
    }

    #[test]
    fn deny_precedence_at_every_layer() {
        let root = cwd();

        // 1. Deny beats allow for matching parameter call
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy
            .allow
            .push(PermissionRule::parse("Agent(model:anthropic/*)").unwrap());
        policy
            .deny
            .push(PermissionRule::parse("Agent(model:anthropic/claude-2)").unwrap());

        let call_allowed = json!({"prompt": "hi", "model": "anthropic/claude-3-5-sonnet"});
        assert_eq!(
            policy.decide("c1", "agent", &call_allowed, &root),
            PermissionVerdict::Allow
        );

        let call_denied = json!({"prompt": "hi", "model": "anthropic/claude-2"});
        assert!(matches!(
            policy.decide("c2", "agent", &call_denied, &root),
            PermissionVerdict::Deny { .. }
        ));

        // 2. Deny beats Auto mode
        let mut auto_policy = PermissionPolicy::new(PermissionMode::Auto);
        auto_policy
            .deny
            .push(PermissionRule::parse("Bash(run_in_background:true)").unwrap());

        let call_bg = json!({"command": "cargo build", "run_in_background": true});
        assert!(matches!(
            auto_policy.decide("c3", "bash", &call_bg, &root),
            PermissionVerdict::Deny { .. }
        ));

        let call_sync = json!({"command": "cargo build --offline"});
        assert_eq!(
            auto_policy.decide("c4", "bash", &call_sync, &root),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn parameter_aware_nested_task_matching() {
        let root = cwd();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy
            .allow
            .push(PermissionRule::parse("Agent(model:anthropic/*)").unwrap());
        policy
            .allow
            .push(PermissionRule::parse("Agent(isolation:worktree)").unwrap());

        // Tasks array element matching
        let call_with_tasks = json!({
            "tasks": [
                {"prompt": "research", "model": "anthropic/claude-3-haiku", "isolation": "worktree"}
            ]
        });
        assert_eq!(
            policy.decide("c1", "agent", &call_with_tasks, &root),
            PermissionVerdict::Allow
        );

        // Direct indexed parameter
        let mut policy2 = PermissionPolicy::new(PermissionMode::Ask);
        policy2
            .allow
            .push(PermissionRule::parse("Agent(tasks[0].model:anthropic/*)").unwrap());
        assert_eq!(
            policy2.decide("c2", "agent", &call_with_tasks, &root),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn dedicated_matchers_for_primary_content_fields() {
        let root = cwd();

        // command matcher
        let mut p1 = PermissionPolicy::new(PermissionMode::Ask);
        p1.allow
            .push(PermissionRule::parse("Bash(command:git status *)").unwrap());
        assert_eq!(
            p1.decide(
                "c1",
                "bash",
                &json!({"command": "git status --short"}),
                &root
            ),
            PermissionVerdict::Allow
        );

        // path matcher
        let mut p2 = PermissionPolicy::new(PermissionMode::Ask);
        p2.allow
            .push(PermissionRule::parse("Read(path:src/**)").unwrap());
        assert_eq!(
            p2.decide("c2", "read", &json!({"path": "src/lib.rs"}), &root),
            PermissionVerdict::Allow
        );

        // domain matcher
        let mut p3 = PermissionPolicy::new(PermissionMode::Ask);
        p3.allow
            .push(PermissionRule::parse("WebFetch(domain:*.example.com)").unwrap());
        assert_eq!(
            p3.decide(
                "c3",
                "web_fetch",
                &json!({"url": "https://api.example.com/data"}),
                &root
            ),
            PermissionVerdict::Allow
        );
    }

    #[test]
    fn visual_snapshot_is_a_network_tool_scoped_by_url_host() {
        assert_eq!(tool_class("visual_snapshot"), ToolClass::Network);
        let root = cwd();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy
            .allow
            .push(PermissionRule::parse("VisualSnapshot(domain:*.example.com)").unwrap());
        assert_eq!(
            policy.decide(
                "visual-1",
                "visual_snapshot",
                &json!({
                    "url": "https://app.example.com",
                    "viewportWidth": 1280,
                    "viewportHeight": 720
                }),
                &root
            ),
            PermissionVerdict::Allow
        );
        assert!(matches!(
            policy.decide(
                "visual-2",
                "visual_snapshot",
                &json!({
                    "url": "https://evil.example.net",
                    "viewportWidth": 1280,
                    "viewportHeight": 720
                }),
                &root
            ),
            PermissionVerdict::Ask(_)
        ));
    }

    #[test]
    fn windows_drive_and_unc_path_containment() {
        let root = PathBuf::from("C:\\work\\proj");

        // Different drive -> outside
        let (sub, outside) = project_relative(&root, "D:\\other\\file.txt");
        assert!(outside);
        assert_eq!(sub, "D:/other/file.txt");

        // Drive prefix without backslash -> outside
        let (_, outside) = project_relative(&root, "D:other\\file.txt");
        assert!(outside);

        // Same drive inside root -> inside
        let (sub, outside) = project_relative(&root, "C:\\work\\proj\\src\\lib.rs");
        assert!(!outside);
        assert_eq!(sub, "src/lib.rs");

        // UNC path -> outside
        let (sub, outside) = project_relative(&root, "\\\\server\\share\\data.txt");
        assert!(outside);
        assert!(sub.contains("server/share/data.txt"));

        // Normalization above drive root cannot escape drive root
        let norm = normalize_lexically(Path::new("C:\\a\\..\\..\\b"));
        assert_eq!(slashes(&norm), "C:/b");
    }

    #[test]
    fn unix_absolute_and_relative_path_containment() {
        let root = PathBuf::from("/work/proj");

        // Root file -> outside
        let (sub, outside) = project_relative(&root, "/etc/passwd");
        assert!(outside);
        assert_eq!(sub, "/etc/passwd");

        // Directory traversal escaping root -> outside
        let (_, outside) = project_relative(&root, "../../secret");
        assert!(outside);

        // Inside root -> inside
        let (sub, outside) = project_relative(&root, "/work/proj/src/main.rs");
        assert!(!outside);
        assert_eq!(sub, "src/main.rs");

        // Popping root cannot pop below root
        let norm = normalize_lexically(Path::new("/a/../.."));
        assert_eq!(norm, PathBuf::from("/"));
    }

    #[test]
    fn untrusted_symlink_escape_detection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let secret_file = outside.join("secret.txt");
        std::fs::write(&secret_file, "classified").unwrap();

        // Create a symlink to outside if the platform/privileges allow
        let symlink_dir = root.join("link_to_outside");
        #[cfg(unix)]
        let symlink_created = std::os::unix::fs::symlink(&outside, &symlink_dir).is_ok();
        #[cfg(windows)]
        let symlink_created = std::os::windows::fs::symlink_dir(&outside, &symlink_dir).is_ok();

        if symlink_created {
            let target = symlink_dir.join("secret.txt");
            let (outside_lexical, symlink_escape) = check_path_boundary(&root, &target);
            assert!(
                symlink_escape || outside_lexical,
                "Symlink escape must be detected"
            );

            let (sub, is_out) = project_relative(&root, target.to_str().unwrap());
            assert!(
                is_out,
                "project_relative must mark symlink escape as outside"
            );
            let _ = sub;
        }

        // Internal normal file
        let internal_file = root.join("normal.txt");
        std::fs::write(&internal_file, "hello").unwrap();
        let (outside_lexical, symlink_escape) = check_path_boundary(&root, &internal_file);
        assert!(!outside_lexical);
        assert!(!symlink_escape);
    }

    #[test]
    fn isolated_agent_worktree_boundary_enforcement() {
        let temp = tempfile::tempdir().unwrap();
        let repo_dir = temp.path().join("main_repo");
        let wt_dir = temp.path().join("worktrees").join("wt-1234");
        std::fs::create_dir_all(repo_dir.join("src")).unwrap();
        std::fs::create_dir_all(wt_dir.join("src")).unwrap();

        let repo_file = repo_dir.join("src").join("lib.rs");
        let wt_file = wt_dir.join("src").join("lib.rs");
        std::fs::write(&repo_file, "fn main_repo() {}").unwrap();
        std::fs::write(&wt_file, "fn worktree() {}").unwrap();

        let mut policy = PermissionPolicy::new(PermissionMode::Auto);
        policy.set_worktree_boundary(&wt_dir, Some(&repo_dir));

        // 1. Mutation inside worktree -> Allowed even with strict boundaries
        let wt_write = json!({"path": "src/lib.rs", "content": "updated"});
        assert_eq!(
            policy.decide("c1", "write", &wt_write, &wt_dir),
            PermissionVerdict::Allow
        );

        // 2. Mutation outside worktree targeting main repo -> Strictly Denied even in Auto mode
        let parent_write = json!({
            "path": repo_file.to_string_lossy().to_string(),
            "content": "illegal modification"
        });
        assert!(matches!(
            policy.decide("c2", "write", &parent_write, &wt_dir),
            PermissionVerdict::Deny { .. }
        ));

        // 3. Mutation targeting arbitrary external dir -> Denied
        let external_file = temp.path().join("external.txt");
        let ext_write = json!({
            "path": external_file.to_string_lossy().to_string(),
            "content": "escaped write"
        });
        assert!(matches!(
            policy.decide("c3", "write", &ext_write, &wt_dir),
            PermissionVerdict::Deny { .. }
        ));
    }

    #[test]
    fn read_outside_root_policy_enforcement() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let outside_dir = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();

        let outside_file = outside_dir.join("config.sys");
        std::fs::write(&outside_file, "secret").unwrap();

        let read_outside = json!({"path": outside_file.to_string_lossy().to_string()});

        // 1. Allow policy
        let mut p_allow = PermissionPolicy::new(PermissionMode::Ask);
        p_allow.set_read_outside_root_policy(ReadOutsideRootPolicy::Allow);
        assert_eq!(
            p_allow.decide("c1", "read", &read_outside, &root),
            PermissionVerdict::Allow
        );

        // 2. Ask policy
        let mut p_ask = PermissionPolicy::new(PermissionMode::Ask);
        p_ask.set_read_outside_root_policy(ReadOutsideRootPolicy::Ask);
        assert!(matches!(
            p_ask.decide("c2", "read", &read_outside, &root),
            PermissionVerdict::Ask(_)
        ));

        // 3. Deny policy
        let mut p_deny = PermissionPolicy::new(PermissionMode::Ask);
        p_deny.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);
        assert!(matches!(
            p_deny.decide("c3", "read", &read_outside, &root),
            PermissionVerdict::Deny { .. }
        ));
    }

    #[test]
    fn git_metadata_access_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        let wt_dir = temp.path().join("worktrees").join("wt-1");
        std::fs::create_dir_all(repo_dir.join(".git").join("worktrees").join("wt-1")).unwrap();
        std::fs::create_dir_all(wt_dir.join("src")).unwrap();

        let mut policy = PermissionPolicy::new(PermissionMode::Edits);
        policy.set_worktree_boundary(&wt_dir, Some(&repo_dir));
        policy.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);

        // Reading git metadata from repo .git -> Allowed despite ReadOutsideRootPolicy::Deny and deny_parent_checkout
        let git_meta_file = repo_dir
            .join(".git")
            .join("worktrees")
            .join("wt-1")
            .join("gitdir");
        std::fs::write(&git_meta_file, "gitdir").unwrap();
        let read_git_meta = json!({"path": git_meta_file.to_string_lossy().to_string()});
        assert_eq!(
            policy.decide("c1", "read", &read_git_meta, &wt_dir),
            PermissionVerdict::Allow
        );

        // Non-git-metadata file in repo -> Denied
        let non_git_file = repo_dir.join("README.md");
        std::fs::write(&non_git_file, "readme").unwrap();
        let read_non_git = json!({"path": non_git_file.to_string_lossy().to_string()});
        assert!(matches!(
            policy.decide("c2", "read", &read_non_git, &wt_dir),
            PermissionVerdict::Deny { .. }
        ));
    }
}
