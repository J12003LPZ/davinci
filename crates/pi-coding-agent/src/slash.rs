pub const BUILTIN_SLASH_COMMANDS: &[(&str, &str)] = &[
    ("settings", "Open settings menu"),
    ("model", "Select model (opens selector UI)"),
    ("tree", "Navigate session tree (switch branches)"),
    ("thinking", "Set thinking level"),
    ("scoped-models", "Enable/disable models for Ctrl+P cycling"),
    (
        "export",
        "Export session (HTML default, or specify path: .html/.jsonl)",
    ),
    ("import", "Import and resume a session from a JSONL file"),
    ("share", "Share session as a secret GitHub gist"),
    ("copy", "Copy last agent message to clipboard"),
    ("name", "Set session display name"),
    ("session", "Show session info and stats"),
    ("changelog", "Show changelog entries"),
    ("hotkeys", "Show all keyboard shortcuts"),
    ("fork", "Create a new fork from a previous user message"),
    (
        "clone",
        "Duplicate the current session at the current position",
    ),
    ("trust", "Save project trust decision for future sessions"),
    ("login", "Configure provider authentication"),
    ("logout", "Remove provider authentication"),
    ("new", "Start a new session"),
    ("compact", "Manually compact the session context"),
    ("resume", "Resume a different session"),
    (
        "reload",
        "Reload keybindings, extensions, skills, prompts, themes, and context files",
    ),
    ("quit", "Quit pi"),
];

#[allow(dead_code)]
pub fn is_slash_command(input: &str) -> bool {
    input.starts_with('/')
}

pub fn parse_slash(input: &str) -> Option<(&str, &str)> {
    let rest = input.strip_prefix('/')?;
    let (name, args) = rest.split_once(' ').unwrap_or((rest, ""));
    Some((name, args.trim()))
}

pub fn is_builtin_slash(name: &str) -> bool {
    matches!(name, "help" | "exit")
        || BUILTIN_SLASH_COMMANDS
            .iter()
            .any(|(command, _)| *command == name)
}
