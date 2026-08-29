use std::collections::BTreeMap;

use pi_ai::ThinkingLevel;
use pi_tui::TuiMode;

pub const APP_NAME: &str = "pi";
pub const CONFIG_DIR_NAME: &str = ".pi";
pub const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";
pub const ENV_SESSION_DIR: &str = "PI_CODING_AGENT_SESSION_DIR";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const VALID_THINKING_LEVELS: &[&str] =
    &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

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
    pub print: bool,
    pub export: Option<String>,
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
    pub project_trust_override: Option<bool>,
    pub parallel_run: Option<String>,
    pub diff_jsonl: Option<(String, String)>,
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    pub unknown_flags: BTreeMap<String, FlagValue>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListModels {
    All,
    Search(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagValue {
    Bool(bool),
    String(String),
}

pub fn normalize_session_name(value: &str) -> Option<String> {
    let name = value.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

pub fn parse_args(args: &[String]) -> Args {
    let mut result = Args::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
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
            if let Some(level) = ThinkingLevel::parse(&args[i]) {
                result.thinking = Some(level);
            } else {
                result.diagnostics.push(Diagnostic {
                    kind: "warning",
                    message: format!(
                        "Invalid thinking level \"{}\". Valid values: {}",
                        args[i],
                        VALID_THINKING_LEVELS.join(", ")
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
        } else if (arg == "--extension" || arg == "-e") && i + 1 < args.len() {
            i += 1;
            result.extensions.push(args[i].clone());
        } else if arg == "--no-extensions" || arg == "-ne" {
            result.no_extensions = true;
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
            let theme = args.get(i + 1);
            if theme.is_none() || theme.unwrap().starts_with('-') {
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
                result.list_models = Some(ListModels::Search(args[i].clone()));
            } else {
                result.list_models = Some(ListModels::All);
            }
        } else if arg == "--tui-mode" {
            let mode = args.get(i + 1).map(String::as_str);
            match mode {
                Some("regular") => {
                    i += 1;
                    result.tui_mode = Some(TuiMode::Regular);
                }
                Some("fullscreen") => {
                    i += 1;
                    result.tui_mode = Some(TuiMode::Fullscreen);
                }
                Some(other) if !other.starts_with('-') => {
                    i += 1;
                    result.diagnostics.push(Diagnostic {
                        kind: "error",
                        message: format!(
                            "Invalid TUI mode \"{other}\". Valid values: regular, fullscreen"
                        ),
                    });
                }
                _ => result.diagnostics.push(Diagnostic {
                    kind: "error",
                    message: "--tui-mode requires regular or fullscreen".into(),
                }),
            }
        } else if arg == "--verbose" {
            result.verbose = true;
        } else if arg == "--approve" || arg == "-a" {
            result.project_trust_override = Some(true);
        } else if arg == "--no-approve" || arg == "-na" {
            result.project_trust_override = Some(false);
        } else if arg == "--offline" {
            result.offline = true;
        } else if arg == "--parallel-run" {
            if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                i += 1;
                result.parallel_run = Some(args[i].clone());
            } else {
                result.parallel_run = Some(String::new());
            }
        } else if arg == "--diff-jsonl" && i + 2 < args.len() {
            result.diff_jsonl = Some((args[i + 1].clone(), args[i + 2].clone()));
            i += 2;
        } else if let Some(path) = arg.strip_prefix('@') {
            result.file_args.push(path.to_string());
        } else if let Some(flag) = arg.strip_prefix("--") {
            if let Some((name, value)) = flag.split_once('=') {
                result
                    .unknown_flags
                    .insert(name.to_string(), FlagValue::String(value.to_string()));
            } else {
                let flag_name = flag.to_string();
                if let Some(next) = args.get(i + 1) {
                    if !next.starts_with('-') && !next.starts_with('@') {
                        result
                            .unknown_flags
                            .insert(flag_name, FlagValue::String(next.clone()));
                        i += 1;
                    } else {
                        result
                            .unknown_flags
                            .insert(flag_name, FlagValue::Bool(true));
                    }
                } else {
                    result
                        .unknown_flags
                        .insert(flag_name, FlagValue::Bool(true));
                }
            }
        } else if arg.starts_with('-') && !arg.starts_with("--") {
            result.diagnostics.push(Diagnostic {
                kind: "error",
                message: format!("Unknown option: {arg}"),
            });
        } else if !arg.starts_with('-') {
            result.messages.push(arg.clone());
        }
        i += 1;
    }
    result
}

pub fn print_help() -> String {
    format!(
        "{APP_NAME} - AI coding assistant with read, bash, edit, write tools

Usage:
  {APP_NAME} [options] [--] [@files...] [messages...]

Commands:
  {APP_NAME} install <source> [-l]     Install extension source and add to settings
  {APP_NAME} remove <source> [-l]      Remove extension source from settings
  {APP_NAME} uninstall <source> [-l]   Alias for remove
  {APP_NAME} update [source|self|pi]   Update pi, extensions, or model catalogs
  {APP_NAME} list                      List installed extensions from settings
  {APP_NAME} config [-l]               Open TUI to enable/disable package resources (Tab switches scope)
  {APP_NAME} auth <command>            Print credentials or check provider readiness
  {APP_NAME} <command> --help          Show help for install/remove/uninstall/update/list/config/auth

Options:
  --provider <name>              Provider name (default: google)
  --model <pattern>              Model pattern or ID (supports \"provider/id\" and optional \":<thinking>\")
  --api-key <key>                API key (defaults to env vars)
  --system-prompt <text>         System prompt (default: coding assistant prompt)
  --append-system-prompt <text>  Append text or file contents to the system prompt (can be used multiple times)
  --mode <mode>                  Output mode: text (default), json, or rpc
  --print, -p                    Non-interactive mode: process prompt and exit
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial UUID
  --session-id <id>              Use exact project session ID, creating it if missing
  --fork <path|id>               Fork specific session file or partial UUID into a new session
  --session-dir <dir>            Directory for session storage and lookup
  --no-session                   Don't save session (ephemeral)
  --name, -n <name>              Set session display name
  --models <patterns>            Comma-separated model patterns for Ctrl+P cycling
  --no-tools, -nt                Disable all tools by default (built-in and extension)
  --no-builtin-tools, -nbt       Disable built-in tools by default but keep extension/custom tools enabled
  --tools, -t <tools>            Comma-separated allowlist of tool names to enable
  --exclude-tools, -xt <tools>   Comma-separated denylist of tool names to disable
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh, max
  --extension, -e <path>         Load an extension file (can be used multiple times)
  --no-extensions, -ne           Disable extension discovery (explicit -e paths still work)
  --skill <path>                 Load a skill file or directory (can be used multiple times)
  --no-skills, -ns               Disable skills discovery and loading
  --prompt-template <path>       Load a prompt template file or directory (can be used multiple times)
  --no-prompt-templates, -np     Disable prompt template discovery and loading
  --theme <path>                 Load a theme file or directory (can be used multiple times)
  --use-theme <name[/name]>      Set the initial interactive theme for this run
  --no-themes                    Disable theme discovery and loading
  --no-context-files, -nc        Disable AGENTS.md and CLAUDE.md discovery and loading
  --export <file>                Export session file to HTML and exit
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup (overrides quietStartup setting)
  --tui-mode <mode>              TUI mode: regular (default) or fullscreen
  --approve, -a                  Trust project-local files for this run
  --no-approve, -na              Ignore project-local files for this run
  --offline                      Disable startup network operations (same as PI_OFFLINE=1)
  --parallel-run <ts-pi>         Compare this binary with a TypeScript `pi` (when Node is present)
  --diff-jsonl <a> <b>           Diff two JSONL transcripts and exit
  --                             End option parsing; treat remaining arguments as messages/files
  --help, -h                     Show this help
  --version, -v                  Show version number

Environment Variables:
  {ENV_AGENT_DIR} - Config directory (default: ~/{CONFIG_DIR_NAME}/agent)
  {ENV_SESSION_DIR} - Session storage directory (overridden by --session-dir)
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parses_print_continue_and_unknown_short() {
        let parsed = parse_args(&s(&["-p", "hello", "-c"]));
        assert!(parsed.print);
        assert!(parsed.continue_session);
        assert_eq!(parsed.messages, vec!["hello"]);
        let bad = parse_args(&s(&["-z"]));
        assert_eq!(bad.diagnostics[0].message, "Unknown option: -z");
    }

    #[test]
    fn dash_message_after_print() {
        let parsed = parse_args(&s(&["-p", "--", "- Summarize"]));
        assert_eq!(parsed.messages, vec!["- Summarize"]);
    }
}
