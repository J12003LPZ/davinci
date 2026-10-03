use davinci_agent::PermissionMode;
use davinci_protocol::ThinkingLevel;
use davinci_tui::TuiMode;
use std::collections::BTreeMap;

pub const APP_NAME: &str = "davinci";
pub const APP_TITLE: &str = "DaVinci";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Return the first argument that is not the global offline switch.  The
/// maintenance dispatcher and the normal CLI use the same definition so a
/// literal `--` remains a prompt boundary rather than a subcommand marker.
pub fn first_non_offline_argument(args: &[String]) -> Option<usize> {
    args.iter().position(|argument| argument != "--offline")
}

/// TS `InteractiveMode.updateTerminalTitle`.
pub fn format_terminal_title(session_name: Option<&str>, cwd: &std::path::Path) -> String {
    let cwd_basename = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| cwd.to_string_lossy().into_owned());
    match session_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{APP_TITLE} - {name} - {cwd_basename}"),
        None => format!("{APP_TITLE} - {cwd_basename}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Json,
    Rpc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
    pub thinking: Option<ThinkingLevel>,
    pub continue_session: bool,
    pub resume: bool,
    pub help: bool,
    pub version: bool,
    pub mode: Option<Mode>,
    pub name: Option<String>,
    pub no_session: bool,
    pub session: Option<String>,
    pub session_id: Option<String>,
    pub fork: Option<String>,
    pub session_dir: Option<String>,
    pub models: Vec<String>,
    pub tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub extensions: Vec<String>,
    pub no_extensions: bool,
    pub no_mcp: bool,
    pub print: bool,
    pub export: Option<String>,
    /// Hidden maintainer command: write a live Codex backend capability report.
    pub codex_probe: Option<String>,
    pub no_skills: bool,
    pub skills: Vec<String>,
    pub prompt_templates: Vec<String>,
    pub no_prompt_templates: bool,
    pub themes: Vec<String>,
    pub use_theme: Option<String>,
    pub no_themes: bool,
    pub no_context_files: bool,
    pub list_models: Option<ListModels>,
    pub offline: bool,
    pub tui_mode: Option<TuiMode>,
    pub verbose: bool,
    /// `--legacy-tui`: open the previous chrome instead of the davinci shell.
    pub legacy_tui: bool,
    pub project_trust_override: Option<bool>,
    /// `--permission-mode <mode>` or its historical `--sandbox <preset>` alias.
    pub permission_mode: Option<PermissionMode>,
    /// Separate OS execution policy override. This never changes approval mode.
    pub execution_sandbox_mode: Option<String>,
    /// `--prompt-profile <stable|preview|legacy-v1>`
    pub prompt_profile: Option<davinci_agent::PromptProfile>,
    /// `--cd, -C <dir>`: a Davinci addition for Codex `exec` parity. `run`
    /// applies it through [`take_cd_flag`] before anything reads the working
    /// directory, so this field is only set when `parse_args` sees the raw flag.
    pub cd: Option<String>,
    /// `--output-last-message, -o <file>`: a Davinci addition for Codex `exec`
    /// parity. Print and json runs write the final reply text there.
    pub output_last_message: Option<String>,
    /// `--output-schema <file>`: a Davinci addition for Codex `exec` parity.
    /// `run` loads the JSON schema the final answer of a print or json run
    /// must match; this is the path as given.
    pub output_schema: Option<String>,
    /// Host-approved finite root limits, shared with worker processes.
    pub root_budget: Option<String>,
    /// `--approval-policy <abort|deny-continue>`: what print and json runs do
    /// when a call needs approval nobody can give.
    pub approval_policy: ApprovalPolicy,
    /// `--fail-on-denied`: exit 3 when a deny-continue run denied anything.
    pub fail_on_denied: bool,
    /// `--add-dir <dir>` (repeatable): extra writable roots beside the
    /// workspace. A Davinci addition for Codex parity. Each value is
    /// validated and canonicalized at parse time against the working
    /// directory, which `--cd` has already applied.
    pub add_dirs: Vec<std::path::PathBuf>,
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    pub unknown_flags: BTreeMap<String, FlagValue>,
    pub diagnostics: Vec<Diagnostic>,
}

/// What a non-interactive run does at a permission `Ask`. No TS counterpart;
/// `deny-continue` mirrors Codex `exec -a never`, where the refusal goes back
/// to the model as a tool error and the run carries on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Stop at the first `Ask`, report `approval_required` and exit 1.
    #[default]
    Abort,
    /// Deny the call, tell the model why, and keep going.
    DenyContinue,
}

impl ApprovalPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "abort" => Some(Self::Abort),
            "deny-continue" => Some(Self::DenyContinue),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListModels {
    All,
    Query(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagValue {
    Bool(bool),
    String(String),
}

pub fn is_valid_thinking_level(level: &str) -> Option<ThinkingLevel> {
    ThinkingLevel::parse(level)
}

pub fn normalize_session_name(value: &str) -> Option<String> {
    let name = value.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

const CD_FLAG: &str = "--cd";
const OUTPUT_LAST_MESSAGE_FLAG: &str = "--output-last-message";
const OUTPUT_SCHEMA_FLAG: &str = "--output-schema";
const ADD_DIR_FLAG: &str = "--add-dir";

/// The long name of the Davinci path flag `arg` spells, in any of its forms:
/// long, short, or long with `=value`. Neither flag exists in TS pi; both
/// mirror Codex `exec` (`-C/--cd`, `-o/--output-last-message`).
fn path_flag_name(arg: &str) -> Option<&'static str> {
    [(CD_FLAG, "-C"), (OUTPUT_LAST_MESSAGE_FLAG, "-o")]
        .into_iter()
        .find(|(long, short)| {
            arg == *long
                || arg == *short
                || arg
                    .strip_prefix(long)
                    .is_some_and(|rest| rest.starts_with('='))
        })
        .map(|(long, _)| long)
}

/// The value a path flag carries: the text after `=`, or the next argument
/// unless that is another option. Returns how many extra arguments it used.
fn split_path_flag<'a>(arg: &'a str, next: Option<&'a String>) -> (Option<&'a str>, usize) {
    match arg.split_once('=') {
        Some((_, value)) => (Some(value), 0),
        None => match next {
            Some(value) if !value.starts_with('-') => (Some(value.as_str()), 1),
            _ => (None, 0),
        },
    }
}

/// A missing or empty value is an error, so `-o -p task` never writes a file
/// named `-p`. A dash-led path is still reachable as `--cd=-dir`.
fn path_flag_value(name: &str, value: Option<&str>) -> Result<String, String> {
    match value {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!(
            "{name} requires {}",
            if name == CD_FLAG || name == ADD_DIR_FLAG {
                "a directory"
            } else {
                "a file path"
            }
        )),
    }
}

/// Removes every `--cd` / `-C` before a literal `--` and returns the other
/// arguments with the last directory named. `run` calls this first, so the
/// subcommands (`plugin`, `install`, `inspect`) and everything that reads the
/// working directory see the same arguments and the same cwd. A `-C` that is
/// really the value of another flag (`--name -C`) is read as `--cd` here.
pub fn take_cd_flag(raw: Vec<String>) -> Result<(Vec<String>, Option<String>), String> {
    let mut rest = Vec::with_capacity(raw.len());
    let mut cd = None;
    let mut args = raw.into_iter().peekable();
    while let Some(arg) = args.next() {
        if arg == "--" {
            rest.push(arg);
            rest.extend(args);
            break;
        }
        if path_flag_name(&arg) != Some(CD_FLAG) {
            rest.push(arg);
            continue;
        }
        let (value, consumed) = split_path_flag(&arg, args.peek());
        cd = Some(path_flag_value(CD_FLAG, value)?);
        if consumed == 1 {
            args.next();
        }
    }
    Ok((rest, cd))
}

pub fn parse_args(args: &[String]) -> Args {
    let mut result = Args::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            for positional in &args[i + 1..] {
                if let Some(path) = positional.strip_prefix('@') {
                    result.file_args.push(path.to_string());
                } else {
                    result.messages.push(positional.clone());
                }
            }
            break;
        } else if arg == "--help" || arg == "-h" {
            result.help = true;
        } else if arg == "--version" || arg == "-v" {
            result.version = true;
        } else if arg == "--mode" && i + 1 < args.len() {
            i += 1;
            result.mode = match args[i].as_str() {
                "text" => Some(Mode::Text),
                "json" => Some(Mode::Json),
                "rpc" => Some(Mode::Rpc),
                _ => result.mode,
            };
        } else if arg == "--continue" || arg == "-c" {
            result.continue_session = true;
        } else if arg == "--resume" || arg == "-r" {
            result.resume = true;
        } else if arg == "--provider" && i + 1 < args.len() {
            i += 1;
            result.provider = Some(args[i].clone());
        } else if arg == "--model" && i + 1 < args.len() {
            i += 1;
            result.model = Some(args[i].clone());
        } else if arg == "--api-key" && i + 1 < args.len() {
            i += 1;
            result.api_key = Some(args[i].clone());
        } else if arg == "--system-prompt" && i + 1 < args.len() {
            i += 1;
            result.system_prompt = Some(args[i].clone());
        } else if arg == "--append-system-prompt" && i + 1 < args.len() {
            i += 1;
            result.append_system_prompt.push(args[i].clone());
        } else if arg == "--prompt-profile" && i + 1 < args.len() {
            i += 1;
            match crate::prompt_host::parse_prompt_profile(&args[i]) {
                Ok(profile) => result.prompt_profile = Some(profile),
                Err(message) => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message,
                }),
            }
        } else if let Some(val) = arg.strip_prefix("--prompt-profile=") {
            match crate::prompt_host::parse_prompt_profile(val) {
                Ok(profile) => result.prompt_profile = Some(profile),
                Err(message) => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message,
                }),
            }
        } else if arg == "--name" || arg == "-n" {
            if i + 1 < args.len() {
                i += 1;
                result.name = Some(args[i].clone());
            } else {
                result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message: "--name requires a value".into(),
                });
            }
        } else if arg == "--no-session" {
            result.no_session = true;
        } else if arg == "--session" && i + 1 < args.len() {
            i += 1;
            result.session = Some(args[i].clone());
        } else if arg == "--session-id" && i + 1 < args.len() {
            i += 1;
            result.session_id = Some(args[i].clone());
        } else if arg == "--fork" && i + 1 < args.len() {
            i += 1;
            result.fork = Some(args[i].clone());
        } else if arg == "--session-dir" && i + 1 < args.len() {
            i += 1;
            result.session_dir = Some(args[i].clone());
        } else if arg == "--models" && i + 1 < args.len() {
            i += 1;
            result.models = args[i].split(',').map(|s| s.trim().to_string()).collect();
        } else if arg == "--no-tools" || arg == "-nt" {
            result.no_tools = true;
        } else if arg == "--no-builtin-tools" || arg == "-nbt" {
            result.no_builtin_tools = true;
        } else if (arg == "--tools" || arg == "-t") && i + 1 < args.len() {
            i += 1;
            result.tools = args[i]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if (arg == "--exclude-tools" || arg == "-xt") && i + 1 < args.len() {
            i += 1;
            result.exclude_tools = args[i]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if arg == "--thinking" && i + 1 < args.len() {
            i += 1;
            if let Some(level) = is_valid_thinking_level(&args[i]) {
                result.thinking = Some(level);
            } else {
                result.diagnostics.push(Diagnostic {
                    kind: "warning",
                    message: format!(
                        "Invalid thinking level \"{}\". Valid values: off, minimal, low, medium, high, xhigh, max",
                        args[i]
                    ),
                });
            }
        } else if arg == "--print" || arg == "-p" {
            result.print = true;
            if let Some(next) = args.get(i + 1) {
                if !next.starts_with('@') && (!next.starts_with('-') || next.starts_with("---")) {
                    result.messages.push(next.clone());
                    i += 1;
                }
            }
        } else if arg == "--export" && i + 1 < args.len() {
            i += 1;
            result.export = Some(args[i].clone());
        } else if arg == "--codex-probe" && i + 1 < args.len() {
            i += 1;
            result.codex_probe = Some(args[i].clone());
        } else if (arg == "--extension" || arg == "-e") && i + 1 < args.len() {
            i += 1;
            result.extensions.push(args[i].clone());
        } else if arg == "--no-extensions" || arg == "-ne" {
            result.no_extensions = true;
        } else if arg == "--no-mcp" {
            result.no_mcp = true;
        } else if arg == "--skill" && i + 1 < args.len() {
            i += 1;
            result.skills.push(args[i].clone());
        } else if arg == "--prompt-template" && i + 1 < args.len() {
            i += 1;
            result.prompt_templates.push(args[i].clone());
        } else if arg == "--theme" && i + 1 < args.len() {
            i += 1;
            result.themes.push(args[i].clone());
        } else if arg == "--use-theme" {
            let theme_name = args.get(i + 1);
            if theme_name.is_none() || theme_name.unwrap().starts_with('-') {
                result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message: "--use-theme requires a theme name".into(),
                });
            } else {
                i += 1;
                result.use_theme = Some(args[i].clone());
            }
        } else if arg == "--no-skills" || arg == "-ns" {
            result.no_skills = true;
        } else if arg == "--no-prompt-templates" || arg == "-np" {
            result.no_prompt_templates = true;
        } else if arg == "--no-themes" {
            result.no_themes = true;
        } else if arg == "--no-context-files" || arg == "-nc" {
            result.no_context_files = true;
        } else if arg == "--list-models" {
            if i + 1 < args.len() && !args[i + 1].starts_with('-') && !args[i + 1].starts_with('@')
            {
                i += 1;
                result.list_models = Some(ListModels::Query(args[i].clone()));
            } else {
                result.list_models = Some(ListModels::All);
            }
        } else if arg == "--tui-mode" {
            let mode = args.get(i + 1).map(String::as_str);
            match mode {
                Some("regular") | Some("fullscreen") => {
                    i += 1;
                    result.tui_mode = TuiMode::parse(&args[i]);
                }
                Some(value) if value.starts_with('-') || value.is_empty() => {
                    result.diagnostics.push(Diagnostic {
                        kind: "error",
                        message: "--tui-mode requires regular or fullscreen".into(),
                    });
                }
                None => {
                    result.diagnostics.push(Diagnostic {
                        kind: "error",
                        message: "--tui-mode requires regular or fullscreen".into(),
                    });
                }
                Some(value) => {
                    i += 1;
                    result.diagnostics.push(Diagnostic {
                        kind: "error",
                        message: format!(
                            "Invalid TUI mode \"{value}\". Valid values: regular, fullscreen"
                        ),
                    });
                }
            }
        } else if arg == "--verbose" {
            result.verbose = true;
        } else if arg == "--legacy-tui" || arg == "--davinci" {
            // Boolean, and declared here so neither swallows the message that
            // follows it. `--davinci` is now the default and kept only for
            // `--davinci --screen <id>`; `--legacy-tui` asks for the old
            // chrome. Both are read from the raw argv in `main`.
            result.legacy_tui = arg == "--legacy-tui";
        } else if arg == "--execution-sandbox" {
            match args.get(i + 1) {
                None => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message: "--execution-sandbox requires one of: none, restricted, workspace-write, full-access".into(),
                }),
                Some(value) => {
                    i += 1;
                    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                        "none" | "no-execution" | "restricted" | "read-only"
                        | "workspace-write" | "workspace" | "full-access" | "full" => {
                            result.execution_sandbox_mode = Some(value.clone());
                        }
                        _ => result.diagnostics.push(Diagnostic {
                            kind: "error",
                            message: format!(
                                "Invalid execution sandbox mode \"{value}\". Valid values: none, restricted, workspace-write, full-access"
                            ),
                        }),
                    }
                }
            }
        } else if arg == "--permission-mode" || arg == "--sandbox" {
            // Two spellings of one flag: ours, and the Codex CLI's sandbox
            // presets (`read-only`, `workspace-write`, `full-access`), which
            // `PermissionMode::parse` maps. Neither is an OS sandbox.
            match args.get(i + 1) {
                None => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message: format!(
                        "{arg} requires one of: manual, accept-edits, plan-mode, auto, always-approve{}",
                        if arg == "--sandbox" {
                            " (or read-only, workspace-write, full-access)"
                        } else {
                            ""
                        }
                    ),
                }),
                Some(value) => {
                    i += 1;
                    match PermissionMode::parse(value) {
                        Some(mode) => result.permission_mode = Some(mode),
                        None => result.diagnostics.push(Diagnostic {
                            kind: "error",
                            message: format!(
                                "Invalid permission mode \"{value}\". Valid values: manual, accept-edits, plan-mode, auto, always-approve"
                            ),
                        }),
                    }
                }
            }
        } else if arg == "--approval-policy" || arg.starts_with("--approval-policy=") {
            let value = match arg.split_once('=') {
                Some((_, value)) => Some(value),
                None => match args.get(i + 1) {
                    Some(next) if !next.starts_with('-') => {
                        i += 1;
                        Some(next.as_str())
                    }
                    _ => None,
                },
            };
            match value.and_then(ApprovalPolicy::parse) {
                Some(policy) => result.approval_policy = policy,
                None => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message: match value {
                        Some(value) => format!(
                            "Invalid approval policy \"{value}\". Valid values: abort, deny-continue"
                        ),
                        None => "--approval-policy requires abort or deny-continue".into(),
                    },
                }),
            }
        } else if arg == "--fail-on-denied" {
            result.fail_on_denied = true;
        } else if arg == "--root-budget" || arg.starts_with("--root-budget=") {
            let (value, consumed) = split_path_flag(arg, args.get(i + 1));
            i += consumed;
            match path_flag_value("--root-budget", value) {
                Ok(value) => result.root_budget = Some(value),
                Err(message) => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message,
                }),
            }
        } else if arg == OUTPUT_SCHEMA_FLAG
            || arg
                .strip_prefix(OUTPUT_SCHEMA_FLAG)
                .is_some_and(|rest| rest.starts_with('='))
        {
            // Codex has no short form for this one.
            let (value, consumed) = split_path_flag(arg, args.get(i + 1));
            i += consumed;
            match path_flag_value(OUTPUT_SCHEMA_FLAG, value) {
                Ok(value) => result.output_schema = Some(value),
                Err(message) => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message,
                }),
            }
        } else if arg == ADD_DIR_FLAG
            || arg
                .strip_prefix(ADD_DIR_FLAG)
                .is_some_and(|rest| rest.starts_with('='))
        {
            // Parsed natively, so an extension flag of the same name never
            // receives it.
            let (value, consumed) = split_path_flag(arg, args.get(i + 1));
            i += consumed;
            let checked = path_flag_value(ADD_DIR_FLAG, value).and_then(|value| {
                let cwd =
                    std::env::current_dir().map_err(|err| format!("{ADD_DIR_FLAG}: {err}"))?;
                davinci_agent::validate_extra_root(
                    &value,
                    &cwd,
                    davinci_session::home_dir().as_deref(),
                )
                .map_err(|reason| format!("{ADD_DIR_FLAG}: {reason}"))
            });
            match checked {
                Ok(root) if !result.add_dirs.contains(&root) => result.add_dirs.push(root),
                Ok(_) => {}
                Err(message) => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message,
                }),
            }
        } else if let Some(name) = path_flag_name(arg) {
            let (value, consumed) = split_path_flag(arg, args.get(i + 1));
            i += consumed;
            match path_flag_value(name, value) {
                Ok(value) if name == CD_FLAG => result.cd = Some(value),
                Ok(value) => result.output_last_message = Some(value),
                Err(message) => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message,
                }),
            }
        } else if arg == "--approve" || arg == "-a" {
            result.project_trust_override = Some(true);
        } else if arg == "--no-approve" || arg == "-na" {
            result.project_trust_override = Some(false);
        } else if arg == "--offline" {
            result.offline = true;
        } else if let Some(path) = arg.strip_prefix('@') {
            result.file_args.push(path.to_string());
        } else if let Some(flag) = arg.strip_prefix("--") {
            if let Some((name, value)) = flag.split_once('=') {
                result
                    .unknown_flags
                    .insert(name.to_string(), FlagValue::String(value.to_string()));
            } else if let Some(next) = args.get(i + 1) {
                if !next.starts_with('-') && !next.starts_with('@') {
                    result
                        .unknown_flags
                        .insert(flag.to_string(), FlagValue::String(next.clone()));
                    i += 1;
                } else {
                    result
                        .unknown_flags
                        .insert(flag.to_string(), FlagValue::Bool(true));
                }
            } else {
                result
                    .unknown_flags
                    .insert(flag.to_string(), FlagValue::Bool(true));
            }
        } else if arg.starts_with('-') && !arg.starts_with("--") {
            result.diagnostics.push(Diagnostic {
                kind: "error",
                message: format!("Unknown option: {arg}"),
            });
        } else if !arg.starts_with('-') {
            result.messages.push(arg.to_string());
        }
        i += 1;
    }
    result
}

pub fn print_help() -> String {
    include_str!("help.txt").to_string()
}

/// Append dynamically registered extension flags, matching TS `printHelp(extensionFlags)`.
pub fn print_help_with_extension_flags(flags: &[(String, String)]) -> String {
    let mut help = print_help();
    if flags.is_empty() {
        return help;
    }
    if !help.ends_with('\n') {
        help.push('\n');
    }
    help.push_str("\nExtension CLI Flags:\n");
    for (name, path) in flags {
        help.push_str(&format!("  --{name:<24} Registered by {path}\n"));
    }
    help
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn no_mcp_is_a_boolean_native_flag() {
        let parsed = parse_args(&["--no-mcp".into(), "explain this".into()]);
        assert!(parsed.unknown_flags.is_empty());
        assert!(parsed.no_mcp);
        assert_eq!(parsed.messages, vec!["explain this"]);
    }

    #[test]
    fn help_uses_davinci_branding_and_explains_native_tools() {
        let help = print_help();
        assert!(help.starts_with("davinci -"));
        assert!(help.contains("--no-mcp"));
        assert!(help.contains("Native intelligence"));
        assert!(!help
            .lines()
            .any(|line| line.trim_start().starts_with("pi ")));
    }

    #[test]
    fn the_ui_flags_are_boolean_and_never_swallow_the_message_after_them() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());

        let legacy = args(&["--legacy-tui", "explain the runtime"]);
        assert!(legacy.legacy_tui);
        assert_eq!(legacy.messages, vec!["explain the runtime".to_string()]);
        assert!(legacy.unknown_flags.is_empty());

        // `--davinci` is the default now and survives only for the fixture
        // renderer, but it must not eat a message either.
        let davinci = args(&["--davinci", "explain the runtime"]);
        assert!(!davinci.legacy_tui);
        assert_eq!(davinci.messages, vec!["explain the runtime".to_string()]);

        assert!(!args(&["explain the runtime"]).legacy_tui);
    }

    #[test]
    fn execution_sandbox_flag_is_separate_from_permission_mode() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let parsed = args(&[
            "--permission-mode",
            "manual",
            "--execution-sandbox",
            "workspace-write",
        ]);
        assert_eq!(parsed.permission_mode, Some(PermissionMode::Ask));
        assert_eq!(
            parsed.execution_sandbox_mode.as_deref(),
            Some("workspace-write")
        );

        let invalid = args(&["--execution-sandbox", "magic"]);
        assert!(invalid.execution_sandbox_mode.is_none());
        assert!(invalid
            .diagnostics
            .iter()
            .any(|item| item.message.contains("Invalid execution sandbox mode")));
    }

    #[test]
    fn early_sandbox_activation_uses_the_same_last_flags_and_trust_override() {
        let parsed = parse_args(&[
            "--permission-mode".into(),
            "ask".into(),
            "--sandbox".into(),
            "auto".into(),
            "--execution-sandbox".into(),
            "restricted".into(),
            "--execution-sandbox".into(),
            "workspace-write".into(),
            "--no-approve".into(),
            "--approve".into(),
            "--help".into(),
        ]);
        assert_eq!(parsed.permission_mode, Some(PermissionMode::Auto));
        assert_eq!(
            parsed.execution_sandbox_mode.as_deref(),
            Some("workspace-write")
        );
        assert_eq!(parsed.project_trust_override, Some(true));
    }

    #[test]
    fn permission_mode_and_its_sandbox_alias_parse_the_same_modes() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(
            args(&["--permission-mode", "edits"]).permission_mode,
            Some(PermissionMode::Edits)
        );
        assert_eq!(
            args(&["--sandbox", "workspace-write", "hello"]).permission_mode,
            Some(PermissionMode::Edits)
        );
        assert_eq!(
            args(&["--sandbox", "full-access"]).permission_mode,
            Some(PermissionMode::AlwaysApprove)
        );
        assert_eq!(
            args(&["--sandbox", "workspace-write", "hello"]).messages,
            ["hello"]
        );

        let bad = args(&["--permission-mode", "sometimes"]);
        assert_eq!(bad.permission_mode, None);
        assert!(bad
            .diagnostics
            .iter()
            .any(|d| d.kind == "error"
                && d.message.contains("Invalid permission mode \"sometimes\"")));
        let missing = args(&["--sandbox"]);
        assert!(missing
            .diagnostics
            .iter()
            .any(|d| d.message.contains("--sandbox requires")));
    }

    #[test]
    fn codex_exec_path_flags_parse_long_short_and_equals_forms() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());

        let long = args(&[
            "--output-last-message",
            "out.txt",
            "--cd",
            "repo",
            "-p",
            "go",
        ]);
        assert_eq!(long.output_last_message.as_deref(), Some("out.txt"));
        assert_eq!(long.cd.as_deref(), Some("repo"));
        assert_eq!(long.messages, ["go"]);
        assert!(long.diagnostics.is_empty() && long.unknown_flags.is_empty());

        let short = args(&["-o", "out.txt", "-C", "repo", "go"]);
        assert_eq!(short.output_last_message.as_deref(), Some("out.txt"));
        assert_eq!(short.cd.as_deref(), Some("repo"));
        assert_eq!(short.messages, ["go"]);

        let equals = args(&["--output-last-message=a b.txt", "--cd=-odd"]);
        assert_eq!(equals.output_last_message.as_deref(), Some("a b.txt"));
        assert_eq!(equals.cd.as_deref(), Some("-odd"));
        assert!(equals.unknown_flags.is_empty());

        // After `--` they are message text, like every other flag.
        let literal = args(&["--", "-o", "-C"]);
        assert_eq!(literal.output_last_message, None);
        assert_eq!(literal.messages, ["-o", "-C"]);
    }

    #[test]
    fn codex_exec_path_flags_without_a_value_are_errors() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let error = |parsed: &Args| {
            parsed
                .diagnostics
                .iter()
                .find(|d| d.kind == "error")
                .map(|d| d.message.clone())
        };

        for missing in [
            args(&["-o"]),
            args(&["--output-last-message"]),
            args(&["--output-last-message="]),
            args(&["-o", "-p", "task"]),
        ] {
            assert_eq!(missing.output_last_message, None);
            assert_eq!(
                error(&missing).as_deref(),
                Some("--output-last-message requires a file path")
            );
        }
        // `-o -p task` must not swallow the print flag or its message.
        let swallowed = args(&["-o", "-p", "task"]);
        assert!(swallowed.print);
        assert_eq!(swallowed.messages, ["task"]);

        for missing in [args(&["-C"]), args(&["--cd"]), args(&["--cd", "--print"])] {
            assert_eq!(missing.cd, None);
            assert_eq!(
                error(&missing).as_deref(),
                Some("--cd requires a directory")
            );
        }
    }

    #[test]
    fn take_cd_flag_strips_the_flag_so_subcommands_never_see_it() {
        let raw = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        assert_eq!(
            take_cd_flag(raw(&["-C", "repo", "plugin", "list"])).unwrap(),
            (raw(&["plugin", "list"]), Some("repo".to_string()))
        );
        assert_eq!(
            take_cd_flag(raw(&["-p", "--cd=one", "go", "--cd", "two"])).unwrap(),
            (raw(&["-p", "go"]), Some("two".to_string()))
        );
        assert_eq!(
            take_cd_flag(raw(&["-p", "go", "--", "-C", "x"])).unwrap(),
            (raw(&["-p", "go", "--", "-C", "x"]), None)
        );
        assert_eq!(
            take_cd_flag(raw(&["-p", "go"])).unwrap(),
            (raw(&["-p", "go"]), None)
        );
        assert_eq!(
            take_cd_flag(raw(&["-p", "go", "-C"])).unwrap_err(),
            "--cd requires a directory"
        );
        assert_eq!(
            take_cd_flag(raw(&["--cd="])).unwrap_err(),
            "--cd requires a directory"
        );
    }

    #[test]
    fn approval_policy_parses_both_values_and_rejects_the_rest() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let error = |parsed: &Args| {
            parsed
                .diagnostics
                .iter()
                .find(|d| d.kind == "error")
                .map(|d| d.message.clone())
        };

        let default = args(&["-p", "go"]);
        assert_eq!(default.approval_policy, ApprovalPolicy::Abort);
        assert!(!default.fail_on_denied);

        let spaced = args(&[
            "--approval-policy",
            "deny-continue",
            "--fail-on-denied",
            "go",
        ]);
        assert_eq!(spaced.approval_policy, ApprovalPolicy::DenyContinue);
        assert!(spaced.fail_on_denied);
        assert_eq!(spaced.messages, ["go"]);
        assert!(spaced.diagnostics.is_empty() && spaced.unknown_flags.is_empty());

        let equals = args(&["--approval-policy=abort", "go"]);
        assert_eq!(equals.approval_policy, ApprovalPolicy::Abort);
        assert_eq!(equals.messages, ["go"]);

        let bad = args(&["--approval-policy", "never", "go"]);
        assert_eq!(bad.approval_policy, ApprovalPolicy::Abort);
        assert_eq!(
            error(&bad).as_deref(),
            Some("Invalid approval policy \"never\". Valid values: abort, deny-continue")
        );
        assert_eq!(bad.messages, ["go"]);

        // A missing value never swallows the next flag.
        for missing in [
            args(&["--approval-policy"]),
            args(&["--approval-policy="]),
            args(&["--approval-policy", "-p", "go"]),
        ] {
            assert!(error(&missing).is_some_and(|message| message.contains("abort")));
        }
        let before_print = args(&["--approval-policy", "-p", "go"]);
        assert!(before_print.print);
        assert_eq!(before_print.messages, ["go"]);
    }

    #[test]
    fn add_dir_repeats_and_validates_each_directory() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let temp = tempfile::tempdir().unwrap();
        let base = davinci_agent::strip_verbatim_prefix(&temp.path().canonicalize().unwrap());
        let [one, two] = ["one", "two"].map(|name| {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        });
        let one_text = one.to_string_lossy().into_owned();
        let two_text = two.to_string_lossy().into_owned();
        let parsed = args(&[
            "--add-dir",
            &one_text,
            &format!("--add-dir={two_text}"),
            "--add-dir",
            &one_text,
            "go",
        ]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert_eq!(parsed.add_dirs, [one, two]);
        assert_eq!(parsed.messages, ["go"]);
        assert!(
            !parsed.unknown_flags.contains_key("add-dir"),
            "extension passthrough must not see --add-dir"
        );

        let missing = base.join("missing").to_string_lossy().into_owned();
        let parsed = args(&["--add-dir", &missing]);
        assert!(parsed.add_dirs.is_empty());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].kind, "error");
        assert!(
            parsed.diagnostics[0].message.starts_with("--add-dir: ")
                && parsed.diagnostics[0].message.contains("does not exist"),
            "{:?}",
            parsed.diagnostics
        );

        for bare in [args(&["--add-dir"]), args(&["--add-dir", "--print"])] {
            assert_eq!(
                bare.diagnostics[0].message,
                "--add-dir requires a directory"
            );
        }
        assert!(args(&["--", "--add-dir", "x"]).add_dirs.is_empty());
    }

    #[test]
    fn output_schema_takes_a_path_in_spaced_and_equals_forms() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());

        let spaced = args(&["--output-schema", "schema.json", "-p", "go"]);
        assert_eq!(spaced.output_schema.as_deref(), Some("schema.json"));
        assert_eq!(spaced.messages, ["go"]);
        assert!(spaced.diagnostics.is_empty() && spaced.unknown_flags.is_empty());

        let equals = args(&["--output-schema=a b.json", "go"]);
        assert_eq!(equals.output_schema.as_deref(), Some("a b.json"));
        assert_eq!(equals.messages, ["go"]);

        assert_eq!(args(&["-p", "go"]).output_schema, None);
        assert_eq!(args(&["--", "--output-schema", "x"]).output_schema, None);

        for missing in [
            args(&["--output-schema"]),
            args(&["--output-schema="]),
            args(&["--output-schema", "-p", "go"]),
        ] {
            assert_eq!(missing.output_schema, None);
            assert!(missing
                .diagnostics
                .iter()
                .any(|d| d.kind == "error" && d.message == "--output-schema requires a file path"));
        }
        // A missing value never swallows the print flag or its message.
        let before_print = args(&["--output-schema", "-p", "go"]);
        assert!(before_print.print);
        assert_eq!(before_print.messages, ["go"]);
    }

    #[test]
    fn help_lists_the_codex_exec_path_flags() {
        let help = print_help();
        assert!(help.contains("--output-schema <file>"));
        assert!(help.contains("--output-last-message, -o <file>"));
        assert!(help.contains("--cd, -C <dir>"));
        assert!(help.contains("--approval-policy <abort|deny-continue>"));
        assert!(help.contains("--fail-on-denied"));
    }

    #[test]
    fn terminal_title_uses_davinci_name_and_project() {
        assert_eq!(
            format_terminal_title(None, Path::new("/tmp/project")),
            "DaVinci - project"
        );
        assert_eq!(
            format_terminal_title(Some("demo"), Path::new("/tmp/project")),
            "DaVinci - demo - project"
        );
        assert_eq!(
            format_terminal_title(Some("  "), Path::new("/tmp/project")),
            "DaVinci - project"
        );
    }

    #[test]
    fn prompt_profile_flag_parsing() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());

        assert_eq!(args(&[]).prompt_profile, None);
        assert_eq!(
            args(&["--prompt-profile", "stable"]).prompt_profile,
            Some(davinci_agent::PromptProfile::Stable)
        );
        assert_eq!(
            args(&["--prompt-profile", "preview"]).prompt_profile,
            Some(davinci_agent::PromptProfile::Preview)
        );
        assert_eq!(
            args(&["--prompt-profile", "legacy-v1"]).prompt_profile,
            Some(davinci_agent::PromptProfile::LegacyV1)
        );
        assert_eq!(
            args(&["--prompt-profile=preview"]).prompt_profile,
            Some(davinci_agent::PromptProfile::Preview)
        );

        let bad = args(&["--prompt-profile", "unknown"]);
        assert_eq!(bad.prompt_profile, None);
        assert!(bad.diagnostics.iter().any(|d| d.kind == "error"
            && d.message
                == "Invalid prompt profile 'unknown'. Valid profiles: stable, preview, legacy-v1"));
    }
}
