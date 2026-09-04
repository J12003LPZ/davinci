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
        | "tool_search" => ToolClass::Read,
        "write" | "edit" | "notebook_edit" | "apply_patch" => ToolClass::Edit,
        "bash" | "powershell" | "exec_command" | "write_stdin" => ToolClass::Shell,
        "web_fetch" | "web_search" => ToolClass::Network,
        _ => ToolClass::Other,
    }
}

/// `tool` or `tool(pattern)`. The pattern is a glob over the call's subject:
/// the command for a shell tool, the path for a file tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRule {
    pub tool: String,
    pub pattern: Option<String>,
}

impl PermissionRule {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let Some(open) = text.find('(') else {
            if text.chars().any(char::is_whitespace) || text.contains(')') {
                return None;
            }
            return Some(Self {
                tool: text.to_string(),
                pattern: None,
            });
        };
        if !text.ends_with(')') {
            return None;
        }
        let tool = text[..open].trim();
        let pattern = text[open + 1..text.len() - 1].trim();
        if tool.is_empty() || tool.chars().any(char::is_whitespace) {
            return None;
        }
        Some(Self {
            tool: tool.to_string(),
            pattern: (!pattern.is_empty()).then(|| pattern.to_string()),
        })
    }

    pub fn matches(&self, tool: &str, subject: &str) -> bool {
        if !self.tool_matches(tool) {
            return false;
        }
        let Some(pattern) = &self.pattern else {
            return true;
        };
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

    fn tool_matches(&self, tool: &str) -> bool {
        if self.tool == tool {
            return true;
        }
        if self.tool.chars().any(|ch| ch == '*' || ch == '?') {
            glob_matches(&fold(&self.tool), &fold(tool))
        } else {
            false
        }
    }
}

impl std::fmt::Display for PermissionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.pattern {
            Some(pattern) => write!(f, "{}({pattern})", self.tool),
            None => f.write_str(&self.tool),
        }
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
        if let Some(rule) = self
            .deny
            .iter()
            .find(|rule| segments.iter().any(|segment| rule.matches(tool, segment)))
        {
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
                    rule.matches(tool, segment)
                        && (rule.pattern.is_none() || !has_command_substitution(segment))
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
        ToolClass::Other => (String::new(), false),
    }
}

/// The simple commands a shell line chains: split on `&&`, `||`, `;`, `|`,
/// `&` and newlines outside quotes. Redirect duplication (`2>&1`, `&>`) is
/// not a separator. Empty segments are dropped, so `git status && ` is one
/// segment.
pub fn shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut previous = '\0';
    let chars: Vec<char> = command.chars().collect();
    for (index, &ch) in chars.iter().enumerate() {
        let mut separator = false;
        if escaped {
            escaped = false;
        } else if single {
            if ch == '\'' {
                single = false;
            }
        } else if double {
            if ch == '"' {
                double = false;
            } else if ch == '\\' {
                escaped = true;
            }
        } else {
            match ch {
                '\\' => escaped = true,
                '\'' => single = true,
                '"' => double = true,
                ';' | '|' | '\n' => separator = true,
                '&' => {
                    let next = chars.get(index + 1).copied().unwrap_or('\0');
                    separator = !matches!(previous, '>' | '<') && !matches!(next, '>' | '<');
                }
                _ => {}
            }
        }
        if separator {
            let segment = current.trim();
            if !segment.is_empty() {
                segments.push(segment.to_string());
            }
            current.clear();
        } else {
            current.push(ch);
        }
        previous = ch;
    }
    let segment = current.trim();
    if !segment.is_empty() {
        segments.push(segment.to_string());
    }
    segments
}

/// `$(…)`, backticks and process substitution run a program the rule never
/// named.
fn has_command_substitution(segment: &str) -> bool {
    segment.contains("$(")
        || segment.contains('`')
        || segment.contains("<(")
        || segment.contains(">(")
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
        return PermissionRule {
            tool: tool.to_string(),
            pattern: Some(subject.to_string()),
        };
    }
    if tool_class(tool) != ToolClass::Shell || subject.is_empty() {
        return PermissionRule {
            tool: tool.to_string(),
            pattern: None,
        };
    }
    let mut words = subject.split_whitespace();
    let Some(program) = words.next() else {
        return PermissionRule {
            tool: tool.to_string(),
            pattern: None,
        };
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
    PermissionRule {
        tool: tool.to_string(),
        pattern: Some(format!("{prefix} *")),
    }
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
                pattern: None
            })
        );
        assert_eq!(
            PermissionRule::parse(" bash(git *) "),
            Some(PermissionRule {
                tool: "bash".into(),
                pattern: Some("git *".into())
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
}
