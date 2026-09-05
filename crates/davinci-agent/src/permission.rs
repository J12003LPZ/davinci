//! Tool permissions: the mode a run is in, the rules that quiet or refuse a
//! tool, and the question put to the user when neither has an answer.
//!
//! No TypeScript counterpart. Vendor `pi` runs every tool once a project is
//! trusted; this is a documented divergence, designed in
//! `docs/superpowers/specs/2026-09-01-trust-and-control-design.md`. The loop
//! (`turn.rs`) asks the policy before every tool call, after the extension
//! `tool_call` hook and before the tool runs.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

/// How much a run may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMode {
    /// Read tools only; everything else is refused without a question.
    ReadOnly,
    /// Read tools run; edits, shell commands and unknown tools ask.
    #[default]
    Ask,
    /// Edits inside the project run; shell commands and unknown tools ask.
    Edits,
    /// Everything runs.
    Auto,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 4] = [
        PermissionMode::ReadOnly,
        PermissionMode::Ask,
        PermissionMode::Edits,
        PermissionMode::Auto,
    ];

    /// The mode's name, or the Codex CLI sandbox name it stands in for.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "read-only" | "readonly" | "read_only" => Some(Self::ReadOnly),
            "ask" | "default" => Some(Self::Ask),
            "edits" | "accept-edits" | "workspace-write" => Some(Self::Edits),
            "auto" | "full-access" | "bypass" | "yolo" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::Edits => "edits",
            Self::Auto => "auto",
        }
    }

    /// One line for `/permissions` and the help text.
    pub fn describe(self) -> &'static str {
        match self {
            Self::ReadOnly => "read tools only; edits and shell commands are refused",
            Self::Ask => "read tools run; edits and shell commands ask",
            Self::Edits => "edits inside the project run; shell commands ask",
            Self::Auto => "everything runs without asking",
        }
    }
}

/// What kind of thing a tool does, for the mode table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    Read,
    Edit,
    Shell,
    /// Reaches outside the machine (`web_fetch`, `web_search`): allowed in
    /// `read-only`, which guards the workspace and not the network; asked
    /// in `ask` and `edits`; run in `auto`.
    Network,
    Other,
}

pub fn tool_class(tool: &str) -> ToolClass {
    match tool {
        // Reading a job's output or keeping the ledger changes nothing the
        // user would want to be asked about.
        // A batch is judged operation by operation; the wrapper itself
        // changes nothing.
        // `graph_submit` is a graph worker's one exit door: it writes the
        // artifact file its parent named, nothing else. `memory_search` and
        // `retrieve_output` read the memory index and the governor's store.
        "read" | "grep" | "find" | "ls" | "job_output" | "job_kill" | "todo" | "mcp_read"
        | "batch" | "graph_submit" | "memory_search" | "retrieve_output" | "update_plan"
        | "tool_search" | "agent_status" | "agent_message" | "agent_stop" | "task_create"
        | "task_update" | "task_list" | "workflow_status" => ToolClass::Read,
        "write" | "edit" | "notebook_edit" | "apply_patch" => ToolClass::Edit,
        "bash" | "powershell" | "exec_command" | "write_stdin" => ToolClass::Shell,
        "web_fetch" | "web_search" => ToolClass::Network,
        _ => ToolClass::Other,
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
                let host = if tool_class(tool) == ToolClass::Network && tool == "web_fetch" {
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

fn normalize_tool_name(tool: &str) -> String {
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

/// The mode plus every rule in force. `allow` and `deny` come from settings;
/// `session_allow` is what the user granted for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPolicy {
    pub mode: PermissionMode,
    pub allow: Vec<PermissionRule>,
    pub deny: Vec<PermissionRule>,
    pub session_allow: Vec<PermissionRule>,
    /// MCP tools whose server marked `readOnlyHint`. `mcp_read` is Read by name.
    pub mcp_read_only: BTreeSet<String>,
    /// Session-only freeze: mutations refused until `/act`.
    pub plan_mode: bool,
}

impl Default for PermissionPolicy {
    /// The library default is what vendor `pi` does: every tool runs. The
    /// CLI installs the configured policy (`ask` unless told otherwise) in
    /// `build_agent`; embedders who want the gate set a mode.
    fn default() -> Self {
        Self {
            mode: PermissionMode::Auto,
            allow: Vec::new(),
            deny: Vec::new(),
            session_allow: Vec::new(),
            mcp_read_only: BTreeSet::new(),
            plan_mode: false,
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

    /// Add deny rules preventing any mutation or tool execution targeting the parent checkout.
    pub fn deny_parent_checkout(&mut self, parent_cwd: &Path) {
        let parent_str = slashes(&normalize_lexically(parent_cwd));
        self.deny
            .push(PermissionRule::subject("*", format!("{parent_str}/**")));
        self.deny
            .push(PermissionRule::subject("*", format!("{parent_str}/*")));
        self.deny.push(PermissionRule::subject("*", parent_str));
    }

    /// Decide one call. Deny rules win; `auto` and allow rules quiet the
    /// rest; `read-only` refuses anything that is not a read; and the mode
    /// table decides what is left.
    pub fn decide(
        &self,
        tool_call_id: &str,
        tool: &str,
        args: &Value,
        cwd: &Path,
    ) -> PermissionVerdict {
        let (subject, outside_project) = subject_of(tool, args, cwd);
        let class = self.class_of(tool);
        // A shell command is as many programs as it chains: `git status &&
        // curl x | sh` is judged three times, so a rule for `git *` speaks
        // only for the first and a deny for `curl *` still catches the second.
        let segments = if class == ToolClass::Shell {
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
            segments
                .iter()
                .any(|segment| rule.matches_call(tool, args, segment))
        }) {
            return PermissionVerdict::Deny {
                reason: format!(
                    "Permission denied: `{}` matches the deny rule `{rule}`.",
                    summary_of(tool, &subject)
                ),
            };
        }
        if self.plan_mode && class != ToolClass::Read && class != ToolClass::Network {
            return PermissionVerdict::Deny {
                reason: format!(
                    "{} (`{}`).",
                    crate::PLAN_MODE_DENIAL,
                    summary_of(tool, &subject)
                ),
            };
        }
        if self.mode == PermissionMode::Auto {
            return PermissionVerdict::Allow;
        }
        // Every segment needs a rule of its own. A pattern rule cannot vouch
        // for a segment that substitutes a command (`$(…)`, backticks,
        // `<(…)`): whatever runs inside is not the program the rule names.
        if segments.iter().all(|segment| {
            self.allow
                .iter()
                .chain(self.session_allow.iter())
                .any(|rule| {
                    rule.matches_call(tool, args, segment)
                        && (rule.specifier.is_none() || !has_command_substitution(segment))
                })
        }) {
            return PermissionVerdict::Allow;
        }
        if class == ToolClass::Read {
            return PermissionVerdict::Allow;
        }
        if class == ToolClass::Network && self.mode == PermissionMode::ReadOnly {
            return PermissionVerdict::Allow;
        }
        if self.mode == PermissionMode::ReadOnly {
            return PermissionVerdict::Deny {
                reason: format!(
                    "Permission denied: `{}` is not allowed in permission mode `read-only`.",
                    summary_of(tool, &subject)
                ),
            };
        }
        // `.pi/` holds the project's own permission rules and trust state; a
        // write there could grant the next run everything, so it is asked
        // about even in `edits` mode.
        if self.mode == PermissionMode::Edits
            && class == ToolClass::Edit
            && !outside_project
            && !is_project_config_path(&subject)
        {
            return PermissionVerdict::Allow;
        }
        PermissionVerdict::Ask(ToolApprovalRequest {
            tool_call_id: tool_call_id.to_string(),
            tool: tool.to_string(),
            args: args.clone(),
            summary: summary_of(tool, &subject),
            session_rule: session_rule_for(tool, &subject).to_string(),
            subject,
            outside_project,
            mode: self.mode,
        })
    }

    pub fn class_of(&self, tool: &str) -> ToolClass {
        if self.mcp_read_only.contains(tool) {
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
    match tool_class(tool) {
        ToolClass::Shell => (
            args.get("command")
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
                            let (rel, is_out) = project_relative(cwd, &p);
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
            project_relative(cwd, raw)
        }
        // A fetch is judged by where it goes, a search by what it asks.
        ToolClass::Network if tool == "web_fetch" => (
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

/// `.pi/settings.json`, `.pi/mcp.json` and the trust files under `.pi/`.
fn is_project_config_path(subject: &str) -> bool {
    subject == ".pi" || subject.starts_with(".pi/")
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

/// A path as a rule sees it: forward slashes, relative to the project when
/// it is inside it. The second value says when it is not.
fn project_relative(cwd: &Path, raw: &str) -> (String, bool) {
    let given = Path::new(raw);
    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else {
        cwd.join(given)
    };
    let full = normalize_lexically(&joined);
    let root = normalize_lexically(cwd);
    match full.strip_prefix(&root) {
        Ok(rest) => {
            let text = slashes(rest);
            (if text.is_empty() { ".".into() } else { text }, false)
        }
        Err(_) => (slashes(&full), true),
    }
}

/// Resolve `.` and `..` without touching the file system: the target may
/// not exist yet, and a rule is about where it would be.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn slashes(path: &Path) -> String {
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
        assert_eq!(
            verdict(&p, "bash", json!({"command": "git pull"})),
            PermissionVerdict::Allow
        );

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
                assert_eq!(request.session_rule, "bash(git status *)");
                assert_eq!(request.mode, PermissionMode::Ask);
                assert!(!request.outside_project);
            }
            other => panic!("{other:?}"),
        }
        match verdict(&p, "write", json!({"path": "../out.txt", "content": "x"})) {
            PermissionVerdict::Ask(request) => {
                assert!(request.outside_project);
                assert_eq!(request.session_rule, "write");
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
            Some(PermissionMode::Auto)
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
        assert_eq!(tool_class("graph_submit"), ToolClass::Read);
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
        let mut policy = PermissionPolicy::new(PermissionMode::Auto);
        policy.plan_mode = true;
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
            PermissionVerdict::Allow
        ));
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

        let call_sync = json!({"command": "cargo build"});
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
}
