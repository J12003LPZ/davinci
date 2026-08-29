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

pub fn rpc_commands() -> Vec<serde_json::Value> {
    builtin_slash_commands()
        .into_iter()
        .map(|command| {
            serde_json::json!({
                "name": command.name,
                "description": command.description,
                "source": "prompt",
                "sourceInfo": { "path": "core/slash-commands.ts" }
            })
        })
        .collect()
}
