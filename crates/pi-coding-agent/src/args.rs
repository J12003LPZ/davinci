use pi_protocol::ThinkingLevel;
use pi_tui::TuiMode;
use std::collections::BTreeMap;

pub const APP_NAME: &str = "pi";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    pub unknown_flags: BTreeMap<String, FlagValue>,
    pub diagnostics: Vec<Diagnostic>,
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
