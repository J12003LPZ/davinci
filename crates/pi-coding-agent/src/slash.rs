use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    #[serde(rename = "argumentHint", skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub source: String,
}

pub fn builtin_slash_commands() -> Vec<SlashCommand> {
    [
        ("settings", "Open settings menu", None),
        (
            "model",
            "Select model (opens selector UI)",
            Some("<provider/model>"),
        ),
        ("tree", "Navigate session tree (switch branches)", None),
        ("thinking", "Set thinking level", Some("<level>")),
        (
            "scoped-models",
            "Enable/disable models for Ctrl+P cycling",
            None,
        ),
        (
            "export",
            "Export session (HTML default, or specify path: .html/.jsonl)",
            None,
        ),
        (
            "import",
            "Import and resume a session from a JSONL file",
            None,
        ),
        ("share", "Share session as a secret GitHub gist", None),
        ("copy", "Copy last agent message to clipboard", None),
        ("name", "Set session display name", None),
        ("session", "Show session info and stats", None),
        ("changelog", "Show changelog entries", None),
        ("hotkeys", "Show all keyboard shortcuts", None),
        (
            "fork",
            "Create a new fork from a previous user message",
            None,
        ),
        (
            "clone",
            "Duplicate the current session at the current position",
            None,
        ),
        (
            "trust",
            "Save project trust decision for future sessions",
            None,
        ),
        (
            "login",
            "Configure provider authentication",
            Some("<provider>"),
        ),
        ("logout", "Remove provider authentication", None),
        ("new", "Start a new session", None),
        ("compact", "Manually compact the session context", None),
        ("resume", "Resume a different session", None),
        (
            "reload",
            "Reload keybindings, extensions, skills, prompts, themes, and context files",
            None,
        ),
        ("llama", "Manage llama.cpp router models", None),
        ("quit", "Quit pi", None),
    ]
    .into_iter()
    .map(|(name, description, hint)| SlashCommand {
        name: name.into(),
        description: description.into(),
        argument_hint: hint.map(str::to_string),
        source: "builtin".into(),
    })
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashAction {
    Quit,
    Status(String),
    Prompt(String),
    NewSession,
    Compact(Option<String>),
    OpenModel,
    OpenThinking,
    SetModel(String),
    SetThinking(String),
    Export(Option<String>),
    Login {
        provider: String,
        key: Option<String>,
    },
    Logout {
        provider: Option<String>,
    },
    Name(String),
    Fork,
    Clone,
    Resume,
    Tree,
    Copy,
    Trust,
    Reload,
    Import(String),
    Share,
    Changelog,
    Settings,
    Hotkeys,
    SessionInfo,
    ScopedModels,
    Llama,
}

pub fn parse_line(line: &str) -> SlashAction {
    let trimmed = line.trim();
    if !trimmed.starts_with('/') {
        return SlashAction::Prompt(trimmed.to_string());
    }
    let rest = trimmed.trim_start_matches('/');
    let (name, args) = rest
        .split_once(char::is_whitespace)
        .map(|(name, args)| (name, args.trim()))
        .unwrap_or((rest, ""));
    match name {
        "quit" | "exit" | "q" => SlashAction::Quit,
        "new" => SlashAction::NewSession,
        "compact" => SlashAction::Compact(if args.is_empty() {
            None
        } else {
            Some(args.to_string())
        }),
        "model" if args.is_empty() => SlashAction::OpenModel,
        "model" => SlashAction::SetModel(args.to_string()),
        "thinking" if args.is_empty() => SlashAction::OpenThinking,
        "thinking" => SlashAction::SetThinking(args.to_string()),
        "export" => SlashAction::Export(if args.is_empty() {
            None
        } else {
            Some(args.to_string())
        }),
        "login" => {
            let mut parts = args.split_whitespace();
            let provider = parts.next().unwrap_or("").to_string();
            let key = parts.next().map(str::to_string);
            SlashAction::Login { provider, key }
        }
        "logout" => SlashAction::Logout {
            provider: if args.is_empty() {
                None
            } else {
                Some(args.to_string())
            },
        },
        "name" => SlashAction::Name(args.to_string()),
        "fork" => SlashAction::Fork,
        "clone" => SlashAction::Clone,
        "resume" => SlashAction::Resume,
        "tree" => SlashAction::Tree,
        "copy" => SlashAction::Copy,
        "trust" => SlashAction::Trust,
        "reload" => SlashAction::Reload,
        "import" => SlashAction::Import(args.to_string()),
        "share" => SlashAction::Share,
        "changelog" => SlashAction::Changelog,
        "settings" => SlashAction::Settings,
        "scoped-models" => SlashAction::ScopedModels,
        "hotkeys" => SlashAction::Hotkeys,
        "session" => SlashAction::SessionInfo,
        "llama" => SlashAction::Llama,
        "help" => SlashAction::Status(
            builtin_slash_commands()
                .into_iter()
                .map(|c| format!("/{} — {}", c.name, c.description))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        other => SlashAction::Status(format!("Unknown command /{other}")),
    }
}

pub fn invocable_commands(
    extensions: &[(String, String, String)],
    templates: &[pi_agent::PromptTemplate],
    skills: &[pi_agent::Skill],
) -> Vec<serde_json::Value> {
    let mut commands = Vec::new();
    for (name, description, path) in extensions {
        commands.push(serde_json::json!({
            "name": name,
            "description": description,
            "source": "extension",
            "sourceInfo": { "path": path }
        }));
    }
    for template in templates {
        let description = template
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .to_string();
        commands.push(serde_json::json!({
            "name": template.name,
            "description": description,
            "source": "prompt",
            "sourceInfo": { "path": template.path.display().to_string() }
        }));
    }
    for skill in skills {
        commands.push(serde_json::json!({
            "name": format!("skill:{}", skill.name),
            "description": skill.description,
            "source": "skill",
            "sourceInfo": { "path": skill.path.display().to_string() }
        }));
    }
    commands
}
