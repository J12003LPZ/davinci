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
        (
            "init",
            "Analyze this project and create or update AGENTS.md",
            Some("[focus]"),
        ),
        ("settings", "Open settings menu", None),
        ("config", "Open configuration (alias for /settings)", None),
        (
            "model",
            "Select model (opens selector UI)",
            Some("<provider/model>"),
        ),
        (
            "thinking",
            "Set reasoning level",
            Some("<off|minimal|low|medium|high|xhigh|max>"),
        ),
        // The composer rule advertises `/effort` on every screen; it was not a
        // command, so following the hint printed "not a command".
        (
            "effort",
            "Set reasoning effort (alias for /thinking)",
            Some("<off|minimal|low|medium|high|xhigh|max>"),
        ),
        ("tree", "Navigate session tree (switch branches)", None),
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
        ("mcp", "Connected MCP servers, tools and errors", None),
        ("cost", "Tokens and USD spent this session", None),
        ("status", "Model, permission, jobs, MCP, tokens", None),
        (
            "permissions",
            "Choose Manual, Accept Edits, Plan Mode, Auto Mode, or Always Approve",
            Some("[mode]"),
        ),
        (
            "plan",
            "Read-only planning with revisions, edits, and explicit execution handoff",
            Some("[show|diff|edit <id> <text>|reject|accept [mode]]"),
        ),
        (
            "act",
            "Leave Plan Mode without implicitly approving a plan",
            None,
        ),
        ("agents", "List custom agent profiles and status", None),
        (
            "plugin",
            "Browse, install and manage Claude Code / Codex plugins",
            Some("[list|browse|install|import|info|approve|enable|disable|marketplace]"),
        ),
        ("tasks", "Execution checklist and live task board", None),
        (
            "graph",
            "Inspect and control native graph runs",
            Some("[diff|fork|rewind|explain|dry-run|verify|budget|export]"),
        ),
        ("help", "Show all commands and shortcuts", None),
        ("quit", "Quit DaVinci", None),
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
    Reload,
    Import(String),
    Share,
    Settings,
    Hotkeys,
    SessionInfo,
    Mcp,
    ShowCost,
    ShowStatus,
    Agents,
    /// `/plugin …`: the text after the command name.
    Plugin(String),
    Tasks,
}

/// Repository initialization uses the normal agent turn and its permission gates.
fn init_prompt(focus: &str) -> String {
    let mut prompt = String::from(
        "Inspect this project's repository and create a concise AGENTS.md guide for future coding agents. \
         Use the repository root when it is identifiable, otherwise the current working directory. \
         Read existing instructions, README files, build manifests, scripts, CI configuration, and representative source files first. \
         Include verified build, test, lint, and formatting commands; the main architecture and important paths; \
         and repository-specific conventions and pitfalls. Distinguish documented commands from commands you actually ran. \
         Do not invent commands, architecture, or test results. Avoid generic advice and exhaustive file listings. \
         If AGENTS.md already exists, preserve its existing instructions and user edits, adding or correcting only evidence-backed project guidance. \
         The output filename must be AGENTS.md. Do not create or modify CLAUDE.md or other files. \
         Do not include secrets. Respect the current permission mode; if writing is unavailable, provide a draft and explain that it was not saved. \
         Finish by reporting the file path and a brief summary of what changed."
    );
    if !focus.is_empty() {
        prompt.push_str("\n\nAdditional user focus:\n");
        prompt.push_str(focus);
    }
    prompt
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
        "init" => SlashAction::Prompt(init_prompt(args)),
        "compact" => SlashAction::Compact(if args.is_empty() {
            None
        } else {
            Some(args.to_string())
        }),
        "model" if args.is_empty() => SlashAction::OpenModel,
        "model" => SlashAction::SetModel(args.to_string()),
        "thinking" | "effort" => SlashAction::SetThinking(args.to_string()),
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
        "resume" | "sessions" => SlashAction::Resume,
        "tree" => SlashAction::Tree,
        "copy" => SlashAction::Copy,
        "reload" => SlashAction::Reload,
        "import" => SlashAction::Import(args.to_string()),
        "share" => SlashAction::Share,
        "settings" | "config" => SlashAction::Settings,
        "hotkeys" => SlashAction::Hotkeys,
        "session" if args == "info" || args == "stats" => SlashAction::SessionInfo,
        "session" => SlashAction::Resume,
        "mcp" => SlashAction::Mcp,
        "cost" => SlashAction::ShowCost,
        "status" => SlashAction::ShowStatus,
        "agents" => SlashAction::Agents,
        "plugin" | "plugins" => SlashAction::Plugin(args.to_string()),
        "tasks" => SlashAction::Tasks,
        "help" => SlashAction::Status(
            builtin_slash_commands()
                .into_iter()
                .map(|c| format!("/{} — {}", c.name, c.description))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        // TS sends unknown slashes (skills, templates, extension commands) to prompt().
        _ => SlashAction::Prompt(trimmed.to_string()),
    }
}

pub fn invocable_commands(
    extensions: &[(String, String, String)],
    templates: &[davinci_agent::PromptTemplate],
    skills: &[davinci_agent::Skill],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_discoverable_and_expands_into_an_agents_file_task() {
        assert!(builtin_slash_commands()
            .iter()
            .any(|command| command.name == "init"));
        for command in ["/init", "  /init  ", "/init focus on Rust testing"] {
            let SlashAction::Prompt(prompt) = parse_line(command) else {
                panic!("init must run an agent turn")
            };
            assert!(prompt.contains("AGENTS.md"));
            assert!(prompt.contains("existing instructions"));
            assert!(!prompt.starts_with('/'));
            if command.contains("focus on") {
                assert!(prompt.contains("focus on Rust testing"));
            }
        }
        assert_eq!(
            parse_line("/initial"),
            SlashAction::Prompt("/initial".into())
        );
    }

    #[test]
    fn session_aliases_stay_compatible_but_are_hidden_from_the_public_registry() {
        assert_eq!(parse_line("/session"), SlashAction::Resume);
        assert_eq!(parse_line("  /session  "), SlashAction::Resume);
        assert_eq!(parse_line("/sessions"), SlashAction::Resume);
        assert_eq!(parse_line("/resume"), SlashAction::Resume);

        let names = builtin_slash_commands()
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "resume"));
        assert!(names.iter().any(|name| name == "help"));
        assert!(!names.iter().any(|name| name == "session"));
        assert!(!names.iter().any(|name| name == "sessions"));
    }

    #[test]
    fn session_stats_and_info_subcommands_route_to_session_info() {
        assert_eq!(parse_line("/session info"), SlashAction::SessionInfo);
        assert_eq!(parse_line("/session stats"), SlashAction::SessionInfo);
    }

    #[test]
    fn retired_commands_are_not_registered_or_dispatched() {
        let retired = ["llama", "trust", "changelog", "scoped-models"];
        let names = builtin_slash_commands()
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();

        for name in retired {
            assert!(!names.iter().any(|candidate| candidate == name));
            let input = format!("/{name}");
            assert_eq!(parse_line(&input), SlashAction::Prompt(input));
        }
    }

    #[test]
    fn permission_and_plan_commands_are_advertised_for_host_owned_dispatch() {
        let commands = builtin_slash_commands();
        for name in ["permissions", "plan", "act"] {
            assert_eq!(
                commands
                    .iter()
                    .filter(|command| command.name == name)
                    .count(),
                1
            );
            let input = format!("/{name}");
            // Stateful commands are intercepted by the host, not this pure parser.
            assert_eq!(parse_line(&input), SlashAction::Prompt(input));
        }
    }

    #[test]
    fn thinking_command_sets_every_supported_level() {
        let names = builtin_slash_commands()
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "thinking"));

        assert!(names.iter().any(|name| name == "effort"));
        for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
            for command in ["thinking", "effort"] {
                assert_eq!(
                    parse_line(&format!("/{command} {level}")),
                    SlashAction::SetThinking(level.into())
                );
            }
        }
    }

    #[test]
    fn tasks_command_is_advertised_and_parsed() {
        let commands = builtin_slash_commands();
        assert!(commands.iter().any(|c| c.name == "tasks"));
        assert_eq!(parse_line("/tasks"), SlashAction::Tasks);
    }

    #[test]
    fn graph_commands_delegate_to_extension_host_without_internal_lifecycle_leaks() {
        let builtins = builtin_slash_commands();
        let graph = builtins
            .iter()
            .find(|command| command.name == "graph")
            .expect("graph family should be discoverable");
        assert!(graph.description.contains("native graph"));
        assert!(graph
            .argument_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("budget") && hint.contains("export")));
        for internal in ["graph-resume", "graph-status", "graph-view", "graph-abort"] {
            assert!(
                !builtins.iter().any(|c| c.name == internal),
                "internal lifecycle command '{internal}' must not be in builtin_slash_commands"
            );
        }

        // /graph and subcommands route as extension/prompt commands for host dispatch
        assert_eq!(
            parse_line("/graph save security-audit"),
            SlashAction::Prompt("/graph save security-audit".into())
        );
        assert_eq!(
            parse_line("/graph run security-audit"),
            SlashAction::Prompt("/graph run security-audit".into())
        );
        assert_eq!(parse_line("/graph"), SlashAction::Prompt("/graph".into()));
    }
    #[test]
    fn terminal_config_alias_opens_the_existing_settings_without_new_storage() {
        assert_eq!(parse_line("/config"), SlashAction::Settings);
        assert_eq!(parse_line("/settings"), SlashAction::Settings);
        assert_eq!(
            builtin_slash_commands()
                .iter()
                .filter(|c| c.name == "config")
                .count(),
            1
        );
    }
}
