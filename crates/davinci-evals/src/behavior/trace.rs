//! Normalized behavioral trace capturing agent execution.

use davinci_agent::prompt::manifest::PromptManifest;
use davinci_agent::AgentEvent;
use davinci_ai::{content_text, ChatMessage, MessageContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BehaviorTrace {
    pub scenario_id: String,
    pub prompt_manifest: Option<PromptManifest>,
    pub events: Vec<BehaviorEvent>,
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub file_diffs: Vec<FileDiff>,
    pub verification: Vec<VerificationEvent>,
    pub stats: BehaviorStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BehaviorEvent {
    Search {
        tool: String,
        query: String,
    },
    Read {
        path: String,
        start: Option<u64>,
        end: Option<u64>,
    },
    Edit {
        path: String,
    },
    Shell {
        command_class: String,
        exit_code: Option<i32>,
    },
    SubagentSpawn {
        count: usize,
    },
    PermissionAsked {
        tool: String,
    },
    PermissionDenied {
        tool: String,
    },
    MessageLifecycle {
        phase: String,
    },
    PlanEvent {
        kind: String,
    },
    CapabilityIdentity {
        capability: String,
    },
    PromptProfile {
        profile: String,
    },
    VerificationClaim {
        claim: String,
    },
    FinalResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationEvent {
    pub kind: String,
    pub command: String,
    pub passed: bool,
    /// Number of edits the trace had recorded when this check finished. A check
    /// older than the last edit says nothing about the final state. `None`
    /// (traces saved before this field) is treated as current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "BehaviorStatsWire", into = "BehaviorStatsWire")]
pub struct BehaviorStats {
    pub model_turns: u64,
    pub tool_calls: u64,
    /// `None` means the run never reported the count; `Some(0)` is a measured zero.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub wall_ms: u64,
    pub permission_prompts: u64,
}

/// On-disk shape. `usage_markers` is written by every current trace. A trace
/// without it predates optional usage, when `0` was the placeholder for "never
/// reported", so a zero there is read back as unknown.
#[derive(Serialize, Deserialize)]
struct BehaviorStatsWire {
    model_turns: u64,
    tool_calls: u64,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    wall_ms: u64,
    permission_prompts: u64,
    #[serde(default)]
    usage_markers: bool,
}

impl From<BehaviorStats> for BehaviorStatsWire {
    fn from(stats: BehaviorStats) -> Self {
        Self {
            model_turns: stats.model_turns,
            tool_calls: stats.tool_calls,
            input_tokens: stats.input_tokens,
            output_tokens: stats.output_tokens,
            wall_ms: stats.wall_ms,
            permission_prompts: stats.permission_prompts,
            usage_markers: true,
        }
    }
}

impl From<BehaviorStatsWire> for BehaviorStats {
    fn from(wire: BehaviorStatsWire) -> Self {
        let legacy_zero_is_unknown =
            |count: Option<u64>| count.filter(|count| wire.usage_markers || *count != 0);
        Self {
            model_turns: wire.model_turns,
            tool_calls: wire.tool_calls,
            input_tokens: legacy_zero_is_unknown(wire.input_tokens),
            output_tokens: legacy_zero_is_unknown(wire.output_tokens),
            wall_ms: wire.wall_ms,
            permission_prompts: wire.permission_prompts,
        }
    }
}

/// Class of a shell command: `dev_server`, `test`, `build`, `lint`, `git`
/// (read-only git), `search` (read-only file readers and finders) or `other`.
/// `other` is also what every command we cannot prove harmless is: the
/// staleness rule treats it as a possible edit. Classification is by program
/// and subcommand, never by substring, so `cat tests/data.txt` is not a test.
pub fn classify_shell_command(command: &str) -> &'static str {
    let lower = command.trim().to_lowercase();
    let mut classes: Vec<&'static str> = Vec::new();
    for segment in split_segments(&lower) {
        match classify_segment(&segment) {
            "neutral" => {}
            class => classes.push(class),
        }
    }
    let is_check = |class: &&str| matches!(*class, "test" | "build" | "lint");
    let is_read_only = |class: &&str| matches!(*class, "git" | "search");
    if classes.is_empty() {
        return "other";
    }
    if classes.contains(&"dev_server") {
        return if classes
            .iter()
            .all(|c| *c == "dev_server" || is_read_only(c))
        {
            "dev_server"
        } else {
            "other"
        };
    }
    // Read-only segments (`| tail`, `git diff`) do not change what a check is.
    let rest: Vec<&'static str> = classes
        .iter()
        .copied()
        .filter(|c| !is_read_only(c))
        .collect();
    if rest.is_empty() {
        return classes[0];
    }
    if rest.iter().all(is_check) {
        for class in ["test", "build", "lint"] {
            if rest.contains(&class) {
                return class;
            }
        }
    }
    "other"
}

/// Split on `&&`, `||`, `;`, `|` and newlines. A segment that redirects output
/// to a file is not read-only whatever its program: it is marked with a `>` so
/// `classify_segment` can refuse it (a `2>&1` merge does not write a file).
fn split_segments(command: &str) -> Vec<String> {
    let cleaned = command.replace("2>&1", " ").replace("1>&2", " ");
    cleaned
        .split(['\n', ';', '|', '&'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .map(|segment| {
            // Command substitution can run anything.
            if segment.contains("$(") || segment.contains('`') {
                format!("{segment} >")
            } else {
                segment
            }
        })
        .collect()
}

fn classify_segment(segment: &str) -> &'static str {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let Some(first) = tokens.first() else {
        return "neutral";
    };
    let program = first
        .trim_matches(['"', '\''])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd");
    let args = &tokens[1..];
    // A redirect writes a file, unless the program is a check we only observe.
    let redirects = segment.contains('>');
    let arg_is = |index: usize, name: &str| args.get(index).is_some_and(|arg| *arg == name);
    let class = match program {
        "cd" | "pushd" | "popd" | "set-location" | "sl" => return "neutral",
        "vite" => "dev_server",
        "next" if arg_is(0, "dev") => "dev_server",
        "npm" | "pnpm" | "yarn" | "bun" => match args {
            [first, ..] if matches!(*first, "dev" | "start") => "dev_server",
            [run, name, ..] if *run == "run" && *name == "dev" => "dev_server",
            [first, ..] if matches!(*first, "test" | "t") => "test",
            [run, name, ..] if *run == "run" && matches!(*name, "test" | "test:unit") => "test",
            [run, name, ..] if *run == "run" && *name == "build" => "build",
            [first, ..] if *first == "build" => "build",
            _ => "other",
        },
        "cargo" => {
            let sub = args
                .iter()
                .find(|arg| !arg.starts_with('+') && !arg.starts_with('-'))
                .copied();
            match sub {
                Some("test") | Some("nextest") => "test",
                Some("run") => "dev_server",
                Some("build") | Some("check") => "build",
                Some("clippy") => "lint",
                _ => "other",
            }
        }
        "pytest" | "py.test" | "vitest" | "jest" | "ctest" => "test",
        "node" if args.contains(&"--test") => "test",
        "deno" if arg_is(0, "test") => "test",
        // Runners that only launch another program: judge that program.
        "npx" | "pnpx" | "bunx" | "uvx" => {
            let launched: Vec<&str> = args
                .iter()
                .copied()
                .skip_while(|arg| arg.starts_with('-'))
                .collect();
            return classify_segment(&launched.join(" "));
        }
        "uv" | "poetry" if arg_is(0, "run") => {
            return classify_segment(&args[1..].join(" "));
        }
        "python" | "python3" | "py" => {
            if arg_is(0, "-m") && matches!(args.get(1), Some(&"pytest") | Some(&"unittest")) {
                "test"
            } else {
                "other"
            }
        }
        "go" => match args.first() {
            Some(&"test") => "test",
            Some(&"build") => "build",
            _ => "other",
        },
        "dotnet" => match args.first() {
            Some(&"test") => "test",
            Some(&"build") => "build",
            _ => "other",
        },
        "make" => match args.first() {
            Some(&"test") | Some(&"check") => "test",
            _ => "build",
        },
        "rustc" | "gcc" | "clang" => "build",
        "eslint" | "tsc" | "flake8" | "ruff" | "mypy" => "lint",
        "git" => {
            let mut rest = args.iter();
            let mut sub = None;
            while let Some(arg) = rest.next() {
                match *arg {
                    "-c" | "-C" => {
                        rest.next();
                    }
                    flag if flag.starts_with('-') => {}
                    name => {
                        sub = Some(name);
                        break;
                    }
                }
            }
            let writes_file = args.iter().any(|arg| arg.starts_with("--output"));
            if matches!(sub, Some("status" | "diff" | "log" | "show" | "blame")) && !writes_file {
                "git"
            } else {
                "other"
            }
        }
        "cat" | "type" | "head" | "tail" | "wc" | "ls" | "dir" | "get-content" | "gc"
        | "select-string" | "sls" | "get-childitem" | "gci" | "rg" | "grep" => "search",
        "find" => {
            let mutating = [
                "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprintf", "-fls",
            ];
            if args.iter().any(|arg| mutating.contains(arg)) {
                "other"
            } else {
                "search"
            }
        }
        _ => "other",
    };
    if redirects && matches!(class, "search" | "git") {
        return "other";
    }
    class
}

pub fn detect_verification_claims(text: &str) -> Vec<String> {
    let mut claims = Vec::new();
    let lower = text.to_lowercase();
    let patterns = [
        "all tests pass",
        "tests pass",
        "tests are passing",
        "test suite passed",
        "verified that",
        "verification passed",
        "build succeeded",
        "build passes",
        "all checks pass",
        "successfully verified",
    ];

    for pat in &patterns {
        if lower.contains(pat) {
            claims.push(pat.to_string());
        }
    }
    claims
}

impl BehaviorTrace {
    /// Edits recorded so far; the generation a new check belongs to. A shell
    /// call that is neither a check nor a read-only search (`sed -i`, `git
    /// checkout`, a generator) may have changed files without an `Edit` event,
    /// so it counts too: a later passing check is needed after it.
    pub fn mutation_count(&self) -> u64 {
        self.events
            .iter()
            .filter(|event| match event {
                BehaviorEvent::Edit { .. } => true,
                BehaviorEvent::Shell { command_class, .. } => !matches!(
                    command_class.as_str(),
                    "test" | "build" | "lint" | "search" | "git"
                ),
                _ => false,
            })
            .count() as u64
    }

    /// True only when the latest check of `kind` (every kind, when `None`) that
    /// is not older than the last edit passed. A pass before the final edit, or
    /// a pass followed by a failure, is not verification of the final state.
    pub fn verified_after_last_mutation(&self, kind: Option<&str>) -> bool {
        let final_generation = self.mutation_count();
        let mut latest: std::collections::BTreeMap<&str, bool> = std::collections::BTreeMap::new();
        for check in &self.verification {
            let current = check
                .mutation_generation
                .is_none_or(|generation| generation >= final_generation);
            if current && kind.is_none_or(|kind| check.kind == kind) {
                latest.insert(check.kind.as_str(), check.passed);
            }
        }
        !latest.is_empty() && latest.values().all(|passed| *passed)
    }

    /// Strip credential-shaped values from every free-text field before the
    /// trace leaves the process. Structure, counts and exit codes are kept.
    pub fn redact_credentials(&mut self) {
        use super::process::redact_credentials as redact;
        for event in &mut self.events {
            match event {
                BehaviorEvent::Search { query, .. } => *query = redact(query),
                BehaviorEvent::VerificationClaim { claim } => *claim = redact(claim),
                _ => {}
            }
        }
        for item in &mut self.verification {
            item.command = redact(&item.command);
        }
        for file in &mut self.file_diffs {
            file.diff = redact(&file.diff);
        }
    }

    pub fn new(scenario_id: impl Into<String>, prompt_manifest: Option<PromptManifest>) -> Self {
        Self {
            scenario_id: scenario_id.into(),
            prompt_manifest,
            events: Vec::new(),
            files_changed: Vec::new(),
            file_diffs: Vec::new(),
            verification: Vec::new(),
            stats: BehaviorStats::default(),
        }
    }

    pub fn from_agent_events(
        scenario_id: &str,
        manifest: Option<PromptManifest>,
        events: &[AgentEvent],
        initial_stats: Option<BehaviorStats>,
    ) -> Self {
        let mut trace = Self::new(scenario_id, manifest);
        if let Some(stats) = initial_stats {
            trace.stats = stats;
        }

        let mut pending_shells: HashMap<String, PendingShell> = HashMap::new();
        let mut pending_edits: HashMap<String, PendingEdit> = HashMap::new();

        for event in events {
            match event {
                AgentEvent::TurnStart => {
                    trace.stats.model_turns += 1;
                }
                AgentEvent::ToolExecutionStart {
                    tool_call_id,
                    tool_name,
                    args,
                } => {
                    trace.stats.tool_calls += 1;
                    if let Some((capability, operation)) = native_capability(tool_name) {
                        trace.events.push(BehaviorEvent::CapabilityIdentity {
                            capability: capability.to_string(),
                        });
                        trace.events.push(BehaviorEvent::CapabilityIdentity {
                            capability: operation.to_string(),
                        });
                    }
                    match tool_name.as_str() {
                        "read" | "read_file" | "mcp_read" => {
                            let path = args
                                .get("path")
                                .or_else(|| args.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let start = args
                                .get("start")
                                .or_else(|| args.get("offset"))
                                .and_then(|v| v.as_u64());
                            let end = args
                                .get("end")
                                .or_else(|| args.get("limit"))
                                .and_then(|v| v.as_u64());
                            trace.events.push(BehaviorEvent::Read { path, start, end });
                        }
                        "grep" | "find" | "web_search" | "code_search" => {
                            let query = args
                                .get("query")
                                .or_else(|| args.get("pattern"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            trace.events.push(BehaviorEvent::Search {
                                tool: tool_name.clone(),
                                query,
                            });
                        }
                        "edit" | "write" | "apply_patch" | "notebook_edit" => {
                            let patch = (tool_name == "apply_patch")
                                .then(|| args.get("input").and_then(Value::as_str))
                                .flatten()
                                .map(str::to_owned);
                            let mut paths = patch.as_deref().map(patch_paths).unwrap_or_default();
                            if paths.is_empty() {
                                if let Some(path) = args
                                    .get("path")
                                    .or_else(|| args.get("file_path"))
                                    .and_then(|v| v.as_str())
                                    .filter(|path| !path.is_empty())
                                {
                                    paths.push(path.to_owned());
                                }
                            }
                            for path in &paths {
                                if !trace.files_changed.contains(path) {
                                    trace.files_changed.push(path.clone());
                                }
                                trace
                                    .events
                                    .push(BehaviorEvent::Edit { path: path.clone() });
                            }
                            if paths.is_empty() {
                                trace.events.push(BehaviorEvent::Edit {
                                    path: String::new(),
                                });
                            }
                            pending_edits
                                .insert(tool_call_id.clone(), PendingEdit { paths, patch });
                        }
                        "bash" | "powershell" | "exec_command" => {
                            let cmd = args
                                .get("command")
                                .or_else(|| args.get("cmd"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let class = classify_shell_command(cmd);
                            pending_shells.insert(
                                tool_call_id.clone(),
                                PendingShell {
                                    class,
                                    command: cmd.to_string(),
                                },
                            );
                        }
                        "agent" => {
                            trace.events.push(BehaviorEvent::SubagentSpawn { count: 1 });
                        }
                        _ => {}
                    }
                }
                AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name,
                    result,
                    is_error,
                    details,
                    ..
                } => {
                    if tool_name == "bash"
                        || tool_name == "powershell"
                        || tool_name == "exec_command"
                    {
                        let pending = pending_shells.remove(tool_call_id);
                        let class = pending.as_ref().map(|shell| shell.class).unwrap_or("other");
                        let command = pending
                            .as_ref()
                            .map(|shell| shell.command.clone())
                            .unwrap_or_default();
                        let exit_code = if *is_error {
                            result
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .map(|c| c as i32)
                                .or(Some(1))
                        } else {
                            result
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .map(|c| c as i32)
                                .or(Some(0))
                        };

                        trace.events.push(BehaviorEvent::Shell {
                            command_class: class.to_string(),
                            exit_code,
                        });

                        if class == "test" || class == "build" || class == "lint" {
                            let generation = trace.mutation_count();
                            trace.verification.push(VerificationEvent {
                                kind: class.to_string(),
                                command,
                                passed: exit_code == Some(0),
                                mutation_generation: Some(generation),
                            });
                        }
                    }

                    if is_edit_tool(tool_name) {
                        let pending = pending_edits.remove(tool_call_id);
                        if !*is_error {
                            let diff = details
                                .as_ref()
                                .and_then(|details| details.get("diff"))
                                .and_then(Value::as_str)
                                .filter(|diff| !diff.is_empty())
                                .map(str::to_owned)
                                .or_else(|| pending.as_ref().and_then(|edit| edit.patch.clone()));
                            if let Some(diff) = diff {
                                let paths = details
                                    .as_ref()
                                    .and_then(|details| details.get("paths"))
                                    .and_then(Value::as_array)
                                    .map(|paths| {
                                        paths
                                            .iter()
                                            .filter_map(Value::as_str)
                                            .map(str::to_owned)
                                            .collect::<Vec<_>>()
                                    })
                                    .filter(|paths| !paths.is_empty())
                                    .or_else(|| {
                                        details
                                            .as_ref()
                                            .and_then(|details| details.get("path"))
                                            .and_then(Value::as_str)
                                            .filter(|path| !path.is_empty())
                                            .map(|path| vec![path.to_owned()])
                                    })
                                    .or_else(|| pending.as_ref().map(|edit| edit.paths.clone()))
                                    .filter(|paths| !paths.is_empty())
                                    .unwrap_or_else(|| vec![String::new()]);
                                trace
                                    .file_diffs
                                    .extend(paths.into_iter().map(|path| FileDiff {
                                        path,
                                        diff: diff.clone(),
                                    }));
                            }
                        }
                    }

                    if *is_error {
                        let err_str = result.to_string().to_lowercase();
                        if err_str.contains("permission") || err_str.contains("denied") {
                            trace.events.push(BehaviorEvent::PermissionDenied {
                                tool: tool_name.clone(),
                            });
                        }
                    }
                }
                AgentEvent::MessageStart { .. } | AgentEvent::MessageUpdate { .. } => {}
                AgentEvent::MessageEnd { message } | AgentEvent::TurnEnd { message, .. } => {
                    record_assistant_message(&mut trace, message);
                }
                AgentEvent::AgentEnd { messages, .. } => {
                    for message in messages {
                        record_assistant_message(&mut trace, message);
                    }
                }
                _ => {}
            }
        }

        trace
    }
}

fn record_assistant_message(trace: &mut BehaviorTrace, message: &ChatMessage) {
    if message.role != "assistant" {
        return;
    }

    let text = content_text(&message.content);
    for claim in detect_verification_claims(&text) {
        trace
            .events
            .push(BehaviorEvent::VerificationClaim { claim });
    }
    let has_tool_calls = message
        .content
        .iter()
        .any(|block| matches!(block, MessageContent::ToolCall { .. }));
    if !has_tool_calls && !text.is_empty() {
        trace.events.push(BehaviorEvent::FinalResponse);
    }
}

#[derive(Debug, Clone)]
struct PendingShell {
    class: &'static str,
    command: String,
}

#[derive(Debug, Clone)]
struct PendingEdit {
    paths: Vec<String>,
    patch: Option<String>,
}

fn patch_paths(patch: &str) -> Vec<String> {
    davinci_agent::apply_patch::parse_codex_patch(patch)
        .map(|parsed| {
            parsed
                .actions
                .into_iter()
                .map(|action| match action {
                    davinci_agent::apply_patch::FileAction::Add { path, .. }
                    | davinci_agent::apply_patch::FileAction::Delete { path }
                    | davinci_agent::apply_patch::FileAction::Update { path, .. } => path,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_edit_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "edit" | "write" | "apply_patch" | "notebook_edit"
    )
}

fn native_capability(tool_name: &str) -> Option<(&'static str, &str)> {
    let capability = if matches!(
        tool_name,
        "repo_map"
            | "symbol_search"
            | "file_symbols"
            | "file_dependencies"
            | "symbol_relationships"
            | "related_files"
            | "code_query"
    ) {
        "repo_intelligence"
    } else if tool_name.starts_with("lsp_") {
        "language_intelligence"
    } else if tool_name.starts_with("package_") {
        "package_intelligence"
    } else if matches!(
        tool_name,
        "workspace_packages"
            | "build_targets"
            | "build_dependencies"
            | "build_affected"
            | "build_command"
    ) {
        "build_intelligence"
    } else if tool_name.starts_with("git_") {
        "git_intelligence"
    } else if tool_name.starts_with("test_") {
        "test_impact"
    } else if tool_name == "impact_analyze" {
        "change_impact"
    } else if tool_name == "verification_plan" {
        "verification_planner"
    } else if matches!(
        tool_name,
        "workspace_checkpoint" | "workspace_diff" | "workspace_restore"
    ) {
        "workspace_snapshots"
    } else if tool_name.starts_with("browser_") {
        "browser_verification"
    } else if matches!(tool_name, "process_start" | "process_write") {
        "process_manager"
    } else {
        return None;
    };
    Some((capability, tool_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::ChatMessage;
    use serde_json::json;

    #[test]
    fn normalize_agent_events_into_behavior_trace() {
        let events = vec![
            AgentEvent::TurnStart,
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c1".into(),
                tool_name: "grep".into(),
                args: json!({ "query": "fn process" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c1".into(),
                tool_name: "grep".into(),
                result: json!({ "matches": ["src/lib.rs:10"] }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c2".into(),
                tool_name: "read".into(),
                args: json!({ "path": "src/lib.rs", "start": 1, "end": 20 }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c2".into(),
                tool_name: "read".into(),
                result: json!({ "content": "fn process() {}" }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c3".into(),
                tool_name: "edit".into(),
                args: json!({ "path": "src/lib.rs" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c3".into(),
                tool_name: "edit".into(),
                result: json!({ "success": true }),
                is_error: false,
                details: Some(json!({ "diff": "+return Err(error)" })),
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "c4".into(),
                tool_name: "bash".into(),
                args: json!({ "command": "cargo test" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "c4".into(),
                tool_name: "bash".into(),
                result: json!({ "exit_code": 0 }),
                is_error: false,
                details: None,
            },
            AgentEvent::MessageEnd {
                message: ChatMessage::text(
                    "assistant",
                    "I finished the changes and all tests pass.",
                ),
            },
        ];

        let trace = BehaviorTrace::from_agent_events("scenario-1", None, &events, None);

        assert_eq!(trace.scenario_id, "scenario-1");
        assert_eq!(trace.files_changed, vec!["src/lib.rs"]);
        assert_eq!(trace.verification.len(), 1);
        assert_eq!(trace.verification[0].kind, "test");
        assert_eq!(trace.verification[0].command, "cargo test");
        assert!(trace.verification[0].passed);
        assert_eq!(
            trace.file_diffs,
            vec![FileDiff {
                path: "src/lib.rs".into(),
                diff: "+return Err(error)".into(),
            }]
        );

        assert_eq!(trace.stats.model_turns, 1);
        assert_eq!(trace.stats.tool_calls, 4);

        assert_eq!(
            trace.events,
            vec![
                BehaviorEvent::Search {
                    tool: "grep".into(),
                    query: "fn process".into(),
                },
                BehaviorEvent::Read {
                    path: "src/lib.rs".into(),
                    start: Some(1),
                    end: Some(20),
                },
                BehaviorEvent::Edit {
                    path: "src/lib.rs".into(),
                },
                BehaviorEvent::Shell {
                    command_class: "test".into(),
                    exit_code: Some(0),
                },
                BehaviorEvent::VerificationClaim {
                    claim: "all tests pass".into(),
                },
                BehaviorEvent::VerificationClaim {
                    claim: "tests pass".into(),
                },
                BehaviorEvent::FinalResponse,
            ]
        );
    }

    #[test]
    fn classifier_uses_program_and_subcommand_not_substrings() {
        // Reading a path that happens to contain "test" is not running tests.
        assert_eq!(classify_shell_command("cat tests/data.txt"), "search");
        assert_eq!(classify_shell_command("ls tests"), "search");
        assert_eq!(classify_shell_command("git log -- test_utils.rs"), "git");
        assert_eq!(classify_shell_command("curl http://x/test"), "other");
        assert_eq!(classify_shell_command("echo run the tests"), "other");
        // Real runners, by program and subcommand.
        for command in [
            "cargo test",
            "cargo +nightly test --lib",
            "cargo nextest run",
            "pytest -q",
            "python -m pytest tests",
            "python3 -m unittest",
            "npm test",
            "npm run test",
            "pnpm test",
            "yarn test",
            "go test ./...",
            "dotnet test",
            "make test",
            "vitest run",
            "jest",
            "node --test src/login.test.ts",
            "npx --yes vitest run",
            "uv run pytest -q",
            "cd crates/a && cargo test --quiet",
            "cargo test 2>&1 | tail -20",
            "C:\\tools\\cargo.exe test",
        ] {
            assert_eq!(classify_shell_command(command), "test", "{command}");
        }
        assert_eq!(classify_shell_command("go build ./..."), "build");
        assert_eq!(classify_shell_command("cargo build && cargo test"), "test");
        // A check chained after something we cannot vouch for is not a check.
        assert_eq!(
            classify_shell_command("sed -i s/a/b/ f && cargo test"),
            "other"
        );
        assert_eq!(classify_shell_command("cargo test; rm -rf target"), "other");
    }

    #[test]
    fn read_only_shell_calls_are_classified_read_only() {
        for command in [
            "git diff",
            "git status --short",
            "git --no-pager log -5",
            "git -C sub show HEAD",
            "git blame src/a.rs",
            "cat src/a.rs",
            "type src\\a.rs",
            "head -20 a",
            "tail -f a",
            "wc -l a",
            "ls -la",
            "dir",
            "Get-Content a.txt",
            "Select-String -Path a -Pattern x",
            "Get-ChildItem -Recurse",
            "rg foo",
            "grep -rn foo .",
            "find . -name '*.rs'",
            "git diff | wc -l",
        ] {
            assert!(
                matches!(classify_shell_command(command), "git" | "search"),
                "{command} -> {}",
                classify_shell_command(command)
            );
        }
        // Anything that can write stays a possible edit.
        for command in [
            "sed -i s/a/b/ f",
            "git checkout -- f",
            "git commit -am x",
            "git apply p.diff",
            "git diff --output=out.patch",
            "find . -name '*.o' -delete",
            "find . -exec rm {} ;",
            "cat a > b",
            "Get-Content a | Set-Content b",
            "grep foo . > out",
            "cat $(rm f)",
            "python gen.py",
        ] {
            assert_eq!(classify_shell_command(command), "other", "{command}");
        }
    }

    fn shell(command: &str) -> BehaviorEvent {
        BehaviorEvent::Shell {
            command_class: classify_shell_command(command).into(),
            exit_code: Some(0),
        }
    }

    fn passing_check(generation: u64) -> VerificationEvent {
        VerificationEvent {
            kind: "test".into(),
            command: "cargo test".into(),
            passed: true,
            mutation_generation: Some(generation),
        }
    }

    #[test]
    fn read_only_shell_calls_after_a_passing_check_keep_it_verified() {
        for reader in [
            "git diff",
            "Get-Content src/a.rs",
            "cat src/a.rs",
            "rg foo",
            "git status",
        ] {
            let mut trace = BehaviorTrace::new("s", None);
            trace.events.push(BehaviorEvent::Edit {
                path: "a.rs".into(),
            });
            trace.events.push(shell("cargo test"));
            trace
                .verification
                .push(passing_check(trace.mutation_count()));
            trace.events.push(shell(reader));
            assert!(trace.verified_after_last_mutation(None), "{reader}");
        }
    }

    #[test]
    fn a_possible_edit_after_a_passing_check_makes_it_stale() {
        for writer in [
            "sed -i s/a/b/ src/a.rs",
            "git checkout -- src/a.rs",
            "python gen.py",
            "cat a > b",
        ] {
            let mut trace = BehaviorTrace::new("s", None);
            trace.events.push(BehaviorEvent::Edit {
                path: "a.rs".into(),
            });
            trace.events.push(shell("cargo test"));
            trace
                .verification
                .push(passing_check(trace.mutation_count()));
            trace.events.push(shell(writer));
            assert!(!trace.verified_after_last_mutation(None), "{writer}");
        }
    }

    #[test]
    fn classifies_commands_accurately() {
        assert_eq!(classify_shell_command("cargo test --lib"), "test");
        assert_eq!(classify_shell_command("pytest -v tests/"), "test");
        assert_eq!(classify_shell_command("npm test"), "test");
        assert_eq!(classify_shell_command("cargo build --release"), "build");
        assert_eq!(
            classify_shell_command("cargo clippy -- -D warnings"),
            "lint"
        );
        assert_eq!(classify_shell_command("git status"), "git");
        assert_eq!(classify_shell_command("ls -la"), "search");
        assert_eq!(classify_shell_command("curl http://example.com"), "other");
    }

    #[test]
    fn correlates_interleaved_shell_results_by_tool_call_id() {
        let events = vec![
            AgentEvent::ToolExecutionStart {
                tool_call_id: "test".into(),
                tool_name: "bash".into(),
                args: json!({ "command": "cargo test --test regression" }),
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id: "lint".into(),
                tool_name: "powershell".into(),
                args: json!({ "command": "cargo clippy" }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "lint".into(),
                tool_name: "powershell".into(),
                result: json!({ "exit_code": 1 }),
                is_error: false,
                details: None,
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "test".into(),
                tool_name: "bash".into(),
                result: json!({ "exit_code": 0 }),
                is_error: false,
                details: None,
            },
        ];

        let trace = BehaviorTrace::from_agent_events("scenario-1", None, &events, None);

        assert_eq!(trace.verification[0].command, "cargo clippy");
        assert!(!trace.verification[0].passed);
        assert_eq!(
            trace.verification[1].command,
            "cargo test --test regression"
        );
        assert!(trace.verification[1].passed);
    }

    #[test]
    fn captures_successful_apply_patch_diff_without_result_details() {
        let patch = "*** Begin Patch\n*** Add File: src/service.rs\n+return Ok(())\n*** End Patch";
        let events = vec![
            AgentEvent::ToolExecutionStart {
                tool_call_id: "patch".into(),
                tool_name: "apply_patch".into(),
                args: json!({ "input": patch }),
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: "patch".into(),
                tool_name: "apply_patch".into(),
                result: json!({ "success": true }),
                is_error: false,
                details: None,
            },
        ];

        let trace = BehaviorTrace::from_agent_events("scenario-1", None, &events, None);
        assert_eq!(trace.files_changed, vec!["src/service.rs"]);
        assert_eq!(
            trace.file_diffs,
            vec![FileDiff {
                path: "src/service.rs".into(),
                diff: patch.into(),
            }]
        );
    }
}
