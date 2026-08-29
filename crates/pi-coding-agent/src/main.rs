mod args;
mod auth_cmd;
mod changelog;
mod export;
mod extension_host;
mod extensions;
mod external_editor;
mod js_host;
mod packages;
mod rpc;
mod self_update;
mod settings;
mod slash;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use pi_agent::{
    default_system_prompt, discover_prompt_templates, discover_skills, load_context_files, Agent,
    AgentEvent,
};
use pi_ai::{
    content_text, find_model, fuzzy_models, live_complete, load_builtin_models,
    resolve_provider_auth, AssistantMessage, AuthStorage, ContentBlock, Credential, CredentialKind,
    StopReason, ToolSpec,
};
use pi_session::{
    default_agent_dir, discover_sessions, latest_session, now_ms, resolve_session_dir,
    resolve_session_ref, JsonlSession, SessionEntry,
};
use pi_tui::{
    builtin_themes, copy_text, detect_terminal_theme, detect_terminal_theme_for_auto, encode_kitty,
    interactive_settings_list, parse_auto_theme, parse_http_idle_timeout, ChatChrome, Component,
    CustomMessage, DoubleEscapeAction, FilterMode, InteractiveSession, Keybindings, MermaidMode,
    ScopedModel, SessionAction, SessionItem, SessionTreeEntry, SlashCommandSpec, ThemeDetection,
    ToolCard, TuiMode, FALLBACK_PREVIEW_LINES, OSC_QUERY_TIMEOUT_MS,
};

use args::{parse_args, print_help, Args, ListModels, Mode, APP_NAME, VERSION};
use auth_cmd::{
    is_auth_command_help, parse_auth_command, print_auth_command_help, validate_auth_command_args,
    AuthCommandKind,
};
use extension_host::{ExtensionEvent, ExtensionHost};
use external_editor::{clipboard_image_png, clipboard_text, ExternalEditor};
use packages::handle_package_command;
use rpc::{handle_rpc, RpcCommand, RpcRuntime};
use settings::{
    default_project_trust_value, is_trusted, load_settings, save_settings, set_enable_analytics,
    settings_path, should_run_first_time_setup, to_interactive_config,
};
use slash::SlashAction;

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match run(raw) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}

fn run(raw: Vec<String>) -> Result<i32, String> {
    if is_package_command(raw.first().map(String::as_str)) {
        let command = raw[0].as_str();
        if raw.iter().any(|a| a == "--help" || a == "-h") {
            println!("{}", print_help());
            return Ok(0);
        }
        let agent_dir = default_agent_dir();
        packages::ensure_agent_dir(&agent_dir)?;
        println!(
            "{}",
            handle_package_command(command, &raw[1..], &agent_dir)?
        );
        return Ok(0);
    }

    if is_auth_command_help(&raw) {
        println!("{}", print_auth_command_help());
        return Ok(0);
    }
    if let Some(command) = parse_auth_command(&raw).map_err(|err| err.0)? {
        return run_auth(command);
    }

    let parsed = parse_args(&raw);
    for diagnostic in &parsed.diagnostics {
        let prefix = if diagnostic.kind == "error" {
            "Error"
        } else {
            "Warning"
        };
        eprintln!("{prefix}: {}", diagnostic.message);
        if diagnostic.kind == "error" {
            return Ok(1);
        }
    }
    if parsed.help {
        println!("{}", print_help());
        return Ok(0);
    }
    if parsed.version {
        println!("{VERSION}");
        return Ok(0);
    }
    if let Some(list) = &parsed.list_models {
        return list_models(list);
    }
    if let Some(export) = &parsed.export {
        return export_session(&parsed, export);
    }

    let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut agent = build_agent(&parsed, &session_dir, &cwd)?;

    if parsed.mode == Some(Mode::Rpc) {
        return run_rpc(&mut agent);
    }

    let stdin_tty = io::stdin().is_terminal();
    let stdout_tty = io::stdout().is_terminal();
    if parsed.print || parsed.mode == Some(Mode::Json) || !stdin_tty || !stdout_tty {
        return run_print(&parsed, &mut agent);
    }
    run_interactive(&parsed, &mut agent)
}

fn is_package_command(command: Option<&str>) -> bool {
    matches!(
        command,
        Some("install" | "remove" | "uninstall" | "update" | "list" | "config")
    )
}

fn build_agent(parsed: &Args, session_dir: &Path, cwd: &Path) -> Result<Agent, String> {
    let mut prompt = parsed
        .system_prompt
        .clone()
        .unwrap_or_else(default_system_prompt);
    for extra in &parsed.append_system_prompt {
        let text = if Path::new(extra).exists() {
            std::fs::read_to_string(extra).map_err(|err| err.to_string())?
        } else {
            extra.clone()
        };
        prompt.push('\n');
        prompt.push_str(&text);
    }
    let mut agent = Agent::new(prompt);
    if let Some(level) = parsed.thinking {
        agent.thinking_level = level;
    }
    if parsed.no_tools || parsed.no_builtin_tools {
        agent.tools.clear();
    }
    if !parsed.tools.is_empty() {
        agent.tools = parsed.tools.clone();
    }
    agent
        .tools
        .retain(|tool| !parsed.exclude_tools.contains(tool));
    if !parsed.no_skills {
        let mut roots: Vec<PathBuf> = parsed.skills.iter().map(PathBuf::from).collect();
        roots.push(cwd.join(".pi").join("skills"));
        agent.skills = discover_skills(&roots);
    }
    if !parsed.no_prompt_templates {
        let mut roots: Vec<PathBuf> = parsed.prompt_templates.iter().map(PathBuf::from).collect();
        roots.push(cwd.join(".pi").join("prompts"));
        agent.templates = discover_prompt_templates(&roots);
    }
    agent.context_files = load_context_files(cwd, !parsed.no_context_files);
    if !parsed.no_session {
        agent.session = Some(resolve_or_create_session(parsed, session_dir, cwd)?);
    }
    let settings = load_settings(&default_agent_dir());
    let _trusted = is_trusted(&settings, cwd, parsed.project_trust_override);
    let mut extensions = settings.extensions.clone();
    extensions.extend(parsed.extensions.clone());
    if !parsed.no_extensions && !extensions.is_empty() {
        let host = ExtensionHost::load(&default_agent_dir(), &extensions);
        let mut names = extensions.clone();
        names.extend(extensions::extension_tool_names(&host.manifests));
        for ext in &host.js {
            names.extend(ext.tools.iter().cloned());
            names.extend(ext.commands.iter().cloned());
            let _ = ext.handlers.as_slice();
        }
        agent.apply_extension_tools(&names);
        let _ = host.describe_js();
    }
    agent.cwd = cwd.to_path_buf();
    let (provider, model_id) = parse_model_ref(
        parsed.provider.as_deref().unwrap_or("google"),
        parsed.model.as_deref(),
    );
    agent.provider = provider;
    agent.model_id = model_id;
    if let Some(key) = &parsed.api_key {
        if let Ok(mut storage) = AuthStorage::create() {
            storage.set_runtime_override(&agent.provider, key);
            let _ = storage.login_api_key(&agent.provider, key);
        }
    }
    Ok(agent)
}

fn resolve_or_create_session(
    parsed: &Args,
    session_dir: &Path,
    cwd: &Path,
) -> Result<JsonlSession, String> {
    if let Some(reference) = parsed.session.as_deref().or(parsed.fork.as_deref()) {
        let summary = resolve_session_ref(session_dir, Some(&cwd.to_string_lossy()), reference)
            .map_err(|err| err.to_string())?;
        let session = JsonlSession::open(&summary.path).map_err(|err| err.to_string())?;
        if parsed.fork.is_some() {
            return session
                .fork(
                    &session.leaf_id.clone().unwrap_or(session.header.id.clone()),
                    session_dir,
                )
                .map_err(|err| err.to_string());
        }
        return Ok(session);
    }
    if let Some(id) = &parsed.session_id {
        if let Ok(summary) = resolve_session_ref(session_dir, Some(&cwd.to_string_lossy()), id) {
            return JsonlSession::open(&summary.path).map_err(|err| err.to_string());
        }
        let mut session = JsonlSession::create(
            session_dir,
            &cwd.to_string_lossy(),
            parsed
                .name
                .as_deref()
                .and_then(args::normalize_session_name)
                .as_deref(),
        )
        .map_err(|err| err.to_string())?;
        session.header.id = id.clone();
        return Ok(session);
    }
    if parsed.continue_session {
        if let Some(summary) = latest_session(session_dir, Some(&cwd.to_string_lossy()))
            .map_err(|err| err.to_string())?
        {
            return JsonlSession::open(&summary.path).map_err(|err| err.to_string());
        }
    }
    if parsed.resume {
        let sessions = discover_sessions(session_dir, Some(&cwd.to_string_lossy()))
            .map_err(|err| err.to_string())?;
        if let Some(summary) = sessions.first() {
            return JsonlSession::open(&summary.path).map_err(|err| err.to_string());
        }
    }
    JsonlSession::create(
        session_dir,
        &cwd.to_string_lossy(),
        parsed
            .name
            .as_deref()
            .and_then(args::normalize_session_name)
            .as_deref(),
    )
    .map_err(|err| err.to_string())
}

fn list_models(list: &ListModels) -> Result<i32, String> {
    let models = load_builtin_models();
    let selected = match list {
        ListModels::All => models.iter().collect(),
        ListModels::Query(query) => fuzzy_models(&models, query),
    };
    for model in selected {
        println!("{}/{}  {}", model.provider, model.id, model.name);
    }
    Ok(0)
}

fn export_session(parsed: &Args, export: &str) -> Result<i32, String> {
    let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let session = if Path::new(export).exists() {
        JsonlSession::open(Path::new(export)).map_err(|err| err.to_string())?
    } else {
        resolve_or_create_session(parsed, &session_dir, &cwd)?
    };
    let output = if let Some(next) = parsed.messages.first() {
        PathBuf::from(next)
    } else {
        PathBuf::from("session.html")
    };
    println!("{}", export::export_html(&session, &output)?);
    Ok(0)
}

fn run_auth(command: auth_cmd::AuthCommand) -> Result<i32, String> {
    let _ = auth_cmd::get_auth_command_usage(command.kind);
    let parsed = auth_cmd::parsed_auth_args(&command);
    let _ = (command.no_refresh, command.min_expiry_ms);
    let (provider, model) =
        validate_auth_command_args(&parsed, command.kind).map_err(|err| err.0)?;
    let provider = provider.or_else(|| {
        model
            .as_ref()
            .and_then(|m| m.split('/').next().map(str::to_string))
    });
    let Some(provider) = provider else {
        return Err("Auth commands require --provider <provider> or --model <model>".into());
    };
    let mut storage = AuthStorage::create().map_err(|err| err.to_string())?;
    let _ = storage.maybe_refresh(
        &provider,
        now_ms(),
        command.min_expiry_ms.unwrap_or(0),
        command.no_refresh,
    );
    let env = std::env::vars().collect();
    let resolved = resolve_provider_auth(&provider, &storage, &env, true);
    match command.kind {
        AuthCommandKind::Check => {
            let status = if resolved.is_some() {
                "ready"
            } else {
                "not_ready"
            };
            if command.json {
                let mut value = serde_json::json!({ "status": status, "provider": provider });
                if command.credentials {
                    if let Some(auth) = &resolved {
                        if let Some(key) = &auth.api_key {
                            value["credentials"] = serde_json::Value::String(key.clone());
                        }
                    }
                }
                println!("{value}");
            } else if command.credentials {
                if let Some(auth) = resolved {
                    println!("{}", auth.api_key.unwrap_or_else(|| status.to_string()));
                } else {
                    println!("{status}");
                }
            } else {
                println!("{status}");
            }
            Ok(if status == "ready" { 0 } else { 1 })
        }
        AuthCommandKind::ApiKey | AuthCommandKind::BearerToken => {
            let auth = resolved.ok_or_else(|| format!("No credential available for {provider}"))?;
            println!(
                "{}",
                auth.api_key
                    .or_else(|| auth.headers.get("Authorization").cloned())
                    .unwrap_or_default()
            );
            Ok(0)
        }
    }
}

fn complete_prompt(parsed: &Args, agent: &mut Agent) -> (String, Vec<AgentEvent>) {
    let offline = parsed.offline
        || matches!(
            std::env::var("PI_OFFLINE").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    let models = load_builtin_models();
    let model = find_model(&models, &agent.provider, &agent.model_id)
        .cloned()
        .or_else(|| {
            models
                .iter()
                .find(|m| m.provider == agent.provider)
                .cloned()
        })
        .or_else(|| models.first().cloned());
    let mut storage = AuthStorage::create().ok();
    if let (Some(storage), Some(key)) = (storage.as_mut(), parsed.api_key.as_deref()) {
        storage.set_runtime_override(&agent.provider, key);
    }
    if let Some(storage) = storage.as_mut() {
        let _ = storage.maybe_refresh(&agent.provider, now_ms(), 0, false);
    }
    let env = std::env::vars().collect();
    let auth = storage
        .as_ref()
        .and_then(|storage| resolve_provider_auth(&agent.provider, storage, &env, true));
    let tools: Vec<ToolSpec> = pi_agent::tool_specs()
        .into_iter()
        .filter(|tool| agent.tools.iter().any(|name| name == &tool.name))
        .map(|tool| ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
        })
        .collect();
    let mut host = ExtensionHost::default();
    host.emit(ExtensionEvent::BeforeAgentStart);
    let events = agent
        .run_loop(|current| {
            let last_user = current
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| content_text(&m.content).len())
                .unwrap_or(0);
            match (offline, model.as_ref(), auth.as_ref()) {
                (false, Some(model), Some(auth)) => live_complete(
                    model,
                    &current.messages,
                    auth,
                    Some(&current.system_prompt),
                    &tools,
                ),
                _ => Ok(AssistantMessage {
                    id: pi_agent::new_message_id(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: format!("(offline) received {last_user} characters"),
                    }],
                    model: format!("{}/{}", current.provider, current.model_id),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                }),
            }
        })
        .unwrap_or_else(|err| {
            vec![AgentEvent::AgentEnd {
                messages: vec![pi_ai::ChatMessage::text(
                    "assistant",
                    format!("Provider error: {err}"),
                )],
            }]
        });
    let reply = agent
        .last_assistant_text()
        .or_else(|| {
            events.iter().rev().find_map(|event| match event {
                AgentEvent::AgentEnd { messages } => messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant")
                    .map(|m| content_text(&m.content)),
                _ => None,
            })
        })
        .unwrap_or_default();
    host.emit(ExtensionEvent::AgentEnd);
    for event in &events {
        if let AgentEvent::ToolExecutionStart {
            tool_name, args, ..
        } = event
        {
            host.emit(ExtensionEvent::ToolCall {
                tool_name: tool_name.clone(),
                args: args.clone(),
            });
            if host.tool_call_blocked() {
                continue;
            }
            if let Some(result) = host.execute_named_tool(tool_name, &agent.cwd) {
                let _ = result;
            }
        }
    }
    let _ = host.kinds();
    let _ = ExtensionHost::js_summary(&crate::js_host::JsExtensionResult::default());
    (reply, events)
}

fn parse_model_ref(provider: &str, model: Option<&str>) -> (String, String) {
    match model {
        Some(value) if value.contains('/') => {
            let (provider, id) = value.split_once('/').unwrap();
            let id = id.split(':').next().unwrap_or(id);
            (provider.to_string(), id.to_string())
        }
        Some(value) => (
            provider.to_string(),
            value.split(':').next().unwrap_or(value).to_string(),
        ),
        None => (provider.to_string(), String::new()),
    }
}

fn run_print(parsed: &Args, agent: &mut Agent) -> Result<i32, String> {
    let mut prompt = parsed.messages.join("\n");
    for file in &parsed.file_args {
        if let Ok(body) = std::fs::read_to_string(file) {
            prompt.push_str("\n\n");
            prompt.push_str(&body);
        }
    }
    if prompt.is_empty() && !io::stdin().is_terminal() {
        prompt = io::read_to_string(io::stdin()).unwrap_or_default();
    }
    if prompt.trim().is_empty() {
        return Ok(0);
    }
    agent.prompt(&prompt);
    let (reply, events) = complete_prompt(parsed, agent);
    if parsed.mode == Some(Mode::Json) {
        for event in events {
            println!(
                "{}",
                serde_json::to_string(&event).map_err(|err| err.to_string())?
            );
        }
    } else {
        println!("{reply}");
    }
    Ok(0)
}

fn run_rpc(agent: &mut Agent) -> Result<i32, String> {
    let session_dir = agent
        .session
        .as_ref()
        .and_then(|session| session.path.parent())
        .map(|path| path.parent().unwrap_or(path).to_path_buf())
        .unwrap_or_else(pi_session::default_session_dir);
    let cwd = agent.cwd.clone();
    let mut runtime = RpcRuntime::new(
        std::mem::replace(agent, Agent::new(default_system_prompt())),
        session_dir,
        cwd,
    );
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|err| err.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let command: RpcCommand = serde_json::from_str(&line).map_err(|err| err.to_string())?;
        let is_prompt = command.kind == "prompt";
        let response = handle_rpc(&mut runtime, command.clone());
        println!(
            "{}",
            serde_json::to_string(&response).map_err(|err| err.to_string())?
        );
        if is_prompt {
            let parsed = Args {
                offline: matches!(
                    std::env::var("PI_OFFLINE").as_deref(),
                    Ok("1") | Ok("true") | Ok("yes")
                ),
                ..Args::default()
            };
            let (_reply, events) = complete_prompt(&parsed, &mut runtime.agent);
            for event in events {
                println!(
                    "{}",
                    serde_json::to_string(&event).map_err(|err| err.to_string())?
                );
            }
        }
        println!(
            "{}",
            serde_json::to_string(&response).map_err(|err| err.to_string())?
        );
        io::stdout().flush().ok();
    }
    *agent = runtime.agent;
    Ok(0)
}

fn run_interactive(parsed: &Args, agent: &mut Agent) -> Result<i32, String> {
    let fullscreen = parsed.tui_mode == Some(TuiMode::Fullscreen);
    let theme = builtin_themes()
        .into_iter()
        .find(|theme| parsed.use_theme.as_deref() == Some(theme.name.as_str()))
        .or_else(|| builtin_themes().into_iter().next())
        .expect("theme");
    let models: Vec<String> = load_builtin_models()
        .into_iter()
        .map(|model| format!("{}/{}", model.provider, model.id))
        .collect();
    let mut session = InteractiveSession::new(theme, format!("{APP_NAME} {VERSION}"), models);
    if let Some(index) = session
        .models
        .iter()
        .position(|item| item == &format!("{}/{}", agent.provider, agent.model_id))
    {
        session.model_index = index;
    }
    session.cwd = agent.cwd.clone();
    session.slash_commands = slash::builtin_slash_commands()
        .into_iter()
        .map(|command| SlashCommandSpec {
            name: command.name,
            description: command.description,
            argument_hint: command.argument_hint,
        })
        .collect();
    session.login_providers = pi_ai::oauth_providers()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let stored = load_settings(&default_agent_dir());
    session.double_escape_action =
        DoubleEscapeAction::parse(stored.double_escape_action.as_deref().unwrap_or("tree"));
    session.autocomplete_max_visible =
        stored.autocomplete_max_visible.unwrap_or(8).clamp(3, 20) as usize;
    session.tree_filter_mode =
        FilterMode::parse(stored.tree_filter_mode.as_deref().unwrap_or("default"));
    session.mermaid_mode =
        MermaidMode::parse(stored.markdown.mermaid.as_deref().unwrap_or("streaming"));
    session.chrome.transcript.mermaid_mode = session.mermaid_mode;
    session.chrome.transcript.hide_thinking_block = stored.hide_thinking_block.unwrap_or(false);
    session.enabled_model_ids = stored.enabled_models.clone();
    session.keybindings = Keybindings::load(&default_agent_dir());
    session.warnings_anthropic_extra_usage = stored.warnings.anthropic_extra_usage.unwrap_or(true);
    if let Some(levels) = stored.model_thinking_levels.clone() {
        session.model_thinking_levels = levels;
    }
    let _ = session.begin_osc_query(OSC_QUERY_TIMEOUT_MS);
    let host = loaded_extension_host(parsed);
    replay_custom_messages(agent, &mut session, &host);
    let _ = FALLBACK_PREVIEW_LINES;
    if should_run_first_time_setup(&settings_path(&default_agent_dir())) {
        session.open_first_time_setup(&detect_terminal_theme(&session.chrome.theme), APP_NAME);
    }
    print!("{}", InteractiveSession::enter_sequences(fullscreen));
    println!("{}", session.chrome.render(session.width).join("\n"));
    if !parsed.messages.is_empty() {
        let prompt = parsed.messages.join("\n");
        if !apply_session_action(parsed, agent, &mut session, SessionAction::Submit(prompt))? {
            print!("{}", InteractiveSession::leave_sequences(fullscreen));
            return Ok(0);
        }
    }
    let result = if io::stdin().is_terminal() {
        run_raw_session(parsed, agent, &mut session)
    } else {
        run_line_session(parsed, agent, &mut session)
    };
    print!("{}", InteractiveSession::leave_sequences(fullscreen));
    result
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> Result<Self, String> {
        crossterm::terminal::enable_raw_mode().map_err(|err| err.to_string())?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn key_event_to_bytes(key: &crossterm::event::KeyEvent) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => "\x03".into(),
            KeyCode::Char('e') | KeyCode::Char('E') => "\x05".into(),
            KeyCode::Char('g') | KeyCode::Char('G') => "\x07".into(),
            KeyCode::Char('p') | KeyCode::Char('P') => "\x10".into(),
            KeyCode::Char('q') | KeyCode::Char('Q') => "\x11".into(),
            KeyCode::Char('r') | KeyCode::Char('R') => "\x12".into(),
            KeyCode::Char('t') | KeyCode::Char('T') => "\x14".into(),
            KeyCode::Char('n') | KeyCode::Char('N') => "\x0e".into(),
            KeyCode::Char('l') | KeyCode::Char('L') => "\x0c".into(),
            KeyCode::Char('d') | KeyCode::Char('D') => "\x04".into(),
            KeyCode::Char('u') | KeyCode::Char('U') => "\x15".into(),
            KeyCode::Char('a') | KeyCode::Char('A') => "\x01".into(),
            KeyCode::Char('o') | KeyCode::Char('O') => "\x0f".into(),
            KeyCode::Char('s') | KeyCode::Char('S') => "\x13".into(),
            KeyCode::Char('v') | KeyCode::Char('V') => "\x16".into(),
            KeyCode::Char('x') | KeyCode::Char('X') => "\x18".into(),
            KeyCode::Left => "\x1b[1;5D".into(),
            KeyCode::Right => "\x1b[1;5C".into(),
            _ => String::new(),
        };
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return match key.code {
            KeyCode::Enter => "\x1b\r".into(),
            KeyCode::Up => "\x1b[1;3A".into(),
            KeyCode::Down => "\x1b[1;3B".into(),
            KeyCode::Left => "\x1b[1;3D".into(),
            KeyCode::Right => "\x1b[1;3C".into(),
            KeyCode::Char('v') | KeyCode::Char('V') => "\x1bv".into(),
            KeyCode::Char('q') | KeyCode::Char('Q') => "\x1bq".into(),
            KeyCode::Char(ch) => format!("\x1b{ch}"),
            _ => String::new(),
        };
    }
    match key.code {
        KeyCode::Esc => "\x1b".into(),
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => "\n".into(),
        KeyCode::Enter => "\r".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::Backspace => "\x7f".into(),
        KeyCode::Up => "\x1b[A".into(),
        KeyCode::Down => "\x1b[B".into(),
        KeyCode::Left => "\x1b[D".into(),
        KeyCode::Right => "\x1b[C".into(),
        KeyCode::PageUp => "\x1b[5~".into(),
        KeyCode::PageDown => "\x1b[6~".into(),
        KeyCode::Char(ch) => ch.to_string(),
        _ => String::new(),
    }
}

fn run_raw_session(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
) -> Result<i32, String> {
    let _raw = RawModeGuard::enter()?;
    loop {
        if let Some(detection) = session.finish_osc_query(std::time::Instant::now()) {
            apply_osc_theme(session, &detection);
        }
        if !crossterm::event::poll(std::time::Duration::from_millis(50))
            .map_err(|err| err.to_string())?
        {
            continue;
        }
        match crossterm::event::read().map_err(|err| err.to_string())? {
            crossterm::event::Event::Key(key) => {
                if key.kind != crossterm::event::KeyEventKind::Press
                    && key.kind != crossterm::event::KeyEventKind::Repeat
                {
                    continue;
                }
                let action = session.handle_bytes(&key_event_to_bytes(&key));
                if !apply_session_action(parsed, agent, session, action)? {
                    break;
                }
            }
            crossterm::event::Event::Mouse(mouse) => {
                session.chrome.apply_mouse(mouse.row, session.width);
            }
            crossterm::event::Event::Paste(text) => {
                session.handle_bytes(&format!("\x1b[200~{text}\x1b[201~"));
            }
            crossterm::event::Event::Resize(cols, _) => {
                session.width = cols as usize;
            }
            _ => {}
        }
        print!("\x1b[H{}", session.render_frame());
        io::stdout().flush().ok();
    }
    Ok(0)
}

fn run_line_session(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
) -> Result<i32, String> {
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush().ok();
        let mut input = String::new();
        if stdin.lock().read_line(&mut input).ok().unwrap_or(0) == 0 {
            break;
        }
        let action = session.handle_line(&input);
        if !apply_session_action(parsed, agent, session, action)? {
            break;
        }
    }
    Ok(0)
}

fn apply_session_action(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    action: SessionAction,
) -> Result<bool, String> {
    match action {
        SessionAction::None | SessionAction::OpenModel | SessionAction::CloseOverlay => Ok(true),
        SessionAction::SelectSession(id) => {
            let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
            let summary =
                resolve_session_ref(&session_dir, Some(&agent.cwd.to_string_lossy()), &id)
                    .map_err(|err| err.to_string())?;
            let next = JsonlSession::open(&summary.path).map_err(|err| err.to_string())?;
            agent.load_from_session(next);
            session.chrome.status = format!(
                "session={}",
                agent
                    .session
                    .as_ref()
                    .map(|s| s.header.id.clone())
                    .unwrap_or(id)
            );
            Ok(true)
        }
        SessionAction::SelectSetting(value) => {
            apply_interactive_setting(session, &value)?;
            Ok(true)
        }
        SessionAction::OpenSettingsSubmenu => Ok(true),
        SessionAction::ApplySetting { id, value } => {
            apply_interactive_setting(session, &format!("{id}={value}"))?;
            Ok(true)
        }
        SessionAction::FollowUp(_) => {
            session.chrome.status = format!("queued follow-up ({})", session.follow_up_queue.len());
            Ok(true)
        }
        SessionAction::Dequeue => {
            session.chrome.status = if session.chrome.editor.buffer.is_empty() {
                "follow-up queue empty".into()
            } else {
                "dequeued follow-up".into()
            };
            Ok(true)
        }
        SessionAction::ExternalEditor => {
            launch_external_editor(session)?;
            Ok(true)
        }
        SessionAction::PasteClipboard => {
            paste_clipboard(session);
            Ok(true)
        }
        SessionAction::RenameSession { id, name } => {
            rename_discovered_session(parsed, agent, session, &id, &name)?;
            Ok(true)
        }
        SessionAction::DeleteSession { id, path } => {
            delete_discovered_session(agent, session, &id, &path)?;
            Ok(true)
        }
        SessionAction::CycleSetting => {
            if let Some(item) = session
                .chrome
                .settings_list
                .as_ref()
                .and_then(|list| list.selected_item())
            {
                apply_interactive_setting(session, &format!("{}={}", item.id, item.current_value))?;
            }
            Ok(true)
        }
        SessionAction::OpenTree => {
            open_session_tree(agent, session);
            Ok(true)
        }
        SessionAction::OpenScopedModels => {
            open_scoped_models(session);
            Ok(true)
        }
        SessionAction::OpenLogin => Ok(true),
        SessionAction::SelectTreeEntry(id) => {
            session.chrome.status = format!("tree={id}");
            Ok(true)
        }
        SessionAction::FirstTimeSubmit {
            theme,
            share_analytics,
        } => {
            apply_first_time_result(session, &theme, share_analytics)?;
            Ok(true)
        }
        SessionAction::FirstTimeSkip => Ok(true),
        SessionAction::LoginCancelled => {
            session.chrome.status = "Login cancelled".into();
            println!("Login cancelled");
            Ok(true)
        }
        SessionAction::LoginSubmit(value) => {
            if let Some(provider) = session
                .chrome
                .status
                .strip_prefix("Login to ")
                .map(str::to_string)
            {
                login_provider(&provider, Some(&value))?;
            }
            Ok(true)
        }
        SessionAction::PersistScopedModels(ids) => {
            session.enabled_model_ids = ids.clone();
            let dir = default_agent_dir();
            let mut stored = load_settings(&dir);
            stored.enabled_models = ids;
            save_settings(&dir, &stored)?;
            Ok(true)
        }
        SessionAction::ChangeScopedModels(ids) => {
            session.enabled_model_ids = ids;
            Ok(true)
        }
        SessionAction::CopyText(text) => {
            if let Some(text) = text {
                let launched = copy_text(&text);
                session.chrome.status = format!("copied {launched}");
                println!("{text}");
            }
            Ok(true)
        }
        SessionAction::TreeLabel { id, label } => {
            if let Some(store) = agent.session.as_mut() {
                store
                    .append_entry(SessionEntry::label_change(&id, label.as_deref()))
                    .map_err(|err| err.to_string())?;
            }
            session.chrome.status = match &label {
                Some(value) => format!("label={id}:{value}"),
                None => format!("label={id}:cleared"),
            };
            Ok(true)
        }
        SessionAction::OpenFork => handle_user_line(parsed, agent, &mut session.chrome, "/fork"),
        SessionAction::RunBash(command) => {
            match crate::js_host::execute_command_tool(&command, &agent.cwd) {
                Ok(out) => {
                    session.chrome.transcript.push("bash", &out);
                    session.chrome.status = "bash done".into();
                    println!("{out}");
                }
                Err(err) => {
                    session.chrome.transcript.push("bash", &err);
                    session.chrome.status = "bash error".into();
                    eprintln!("{err}");
                }
            }
            Ok(true)
        }
        SessionAction::Quit => Ok(false),
        SessionAction::Abort => {
            session.chrome.status = "aborted".into();
            Ok(true)
        }
        SessionAction::CycleModel => {
            if let Some(model) = session.current_model() {
                let (provider, model_id) = parse_model_ref("google", Some(model));
                agent.provider = provider;
                agent.model_id = model_id;
            }
            Ok(true)
        }
        SessionAction::CycleThinking => {
            if let Some(level) = pi_protocol::ThinkingLevel::parse(session.current_thinking()) {
                agent.thinking_level = level;
            }
            Ok(true)
        }
        SessionAction::Clear => Ok(true),
        SessionAction::SelectModel(value) => {
            let (provider, model_id) = parse_model_ref("google", Some(&value));
            agent.provider = provider;
            agent.model_id = model_id;
            session.chrome.status = format!("model={}/{}", agent.provider, agent.model_id);
            Ok(true)
        }
        SessionAction::Submit(text) => match slash::parse_line(&text) {
            SlashAction::Tree => {
                open_session_tree(agent, session);
                Ok(true)
            }
            SlashAction::ScopedModels => {
                open_scoped_models(session);
                Ok(true)
            }
            SlashAction::Login { provider, key } => {
                start_login(session, &provider, key.as_deref())?;
                Ok(true)
            }
            SlashAction::Settings => {
                open_settings_overlay(session);
                Ok(true)
            }
            SlashAction::Resume => {
                open_session_selector(parsed, agent, session)?;
                Ok(true)
            }
            SlashAction::Reload => {
                session.keybindings = Keybindings::load(&default_agent_dir());
                handle_user_line(parsed, agent, &mut session.chrome, &text)
            }
            SlashAction::Import(_) | SlashAction::Share | SlashAction::Changelog => {
                handle_user_line(parsed, agent, &mut session.chrome, &text)
            }
            _ => handle_user_line(parsed, agent, &mut session.chrome, &text),
        },
    }
}

fn handle_user_line(
    parsed: &Args,
    agent: &mut Agent,
    chrome: &mut ChatChrome,
    text: &str,
) -> Result<bool, String> {
    match slash::parse_line(text) {
        SlashAction::Quit => Ok(false),
        SlashAction::Prompt(prompt) => {
            chrome.transcript.push("user", &prompt);
            agent.prompt(&prompt);
            let (reply, events) = complete_prompt(parsed, agent);
            apply_tool_events(chrome, &events);
            let host = loaded_extension_host(parsed);
            let reply = host.transform_markdown(&reply, "assistant", false, 80);
            chrome.transcript.push("assistant", &reply);
            chrome.editor.handle_input("");
            println!("{reply}");
            Ok(true)
        }
        SlashAction::Status(message) => {
            chrome.status = message.clone();
            println!("{message}");
            Ok(true)
        }
        SlashAction::Hotkeys => {
            let keys = pi_tui::get_keybindings()
                .into_iter()
                .map(|b| format!("{}: {}", b.action, b.keys.join(", ")))
                .collect::<Vec<_>>()
                .join("\n");
            chrome.status = keys.clone();
            println!("{keys}");
            Ok(true)
        }
        SlashAction::Settings => {
            let stored = load_settings(&default_agent_dir());
            let list =
                interactive_settings_list(&to_interactive_config(&stored, &chrome.theme.name));
            chrome.settings_list = Some(list);
            chrome.settings_submenu = None;
            chrome.status = "Settings".into();
            if let Some(settings) = &chrome.settings_list {
                println!("{}", settings.render(80).join("\n"));
            }
            Ok(true)
        }
        SlashAction::SessionInfo => {
            let info = format!(
                "session={} messages={}",
                agent
                    .session
                    .as_ref()
                    .map(|s| s.header.id.clone())
                    .unwrap_or_else(|| "(none)".into()),
                agent.messages.len()
            );
            println!("{info}");
            Ok(true)
        }
        SlashAction::NewSession => {
            let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
            let session = JsonlSession::create(&session_dir, &agent.cwd.to_string_lossy(), None)
                .map_err(|err| err.to_string())?;
            agent.messages.clear();
            agent.session = Some(session);
            println!("Started new session");
            Ok(true)
        }
        SlashAction::Compact(instructions) => {
            let result = agent.compact(instructions.as_deref());
            println!("{}", result.summary);
            Ok(true)
        }
        SlashAction::OpenModel => {
            chrome.selector = Some(pi_tui::SelectList::new(
                load_builtin_models()
                    .into_iter()
                    .map(|model| format!("{}/{}", model.provider, model.id))
                    .collect(),
            ));
            chrome.status = "Select model".into();
            println!("{}", chrome.status);
            Ok(true)
        }
        SlashAction::SetModel(value) => {
            let (provider, model_id) = parse_model_ref("google", Some(&value));
            agent.provider = provider;
            agent.model_id = model_id;
            println!("model={}/{}", agent.provider, agent.model_id);
            Ok(true)
        }
        SlashAction::SetThinking(level) => {
            if let Some(level) = pi_protocol::ThinkingLevel::parse(&level) {
                agent.thinking_level = level;
            }
            println!("thinking={}", agent.thinking_level.as_str());
            Ok(true)
        }
        SlashAction::Export(path) => {
            if let Some(session) = &agent.session {
                let output = PathBuf::from(path.unwrap_or_else(|| "session.html".into()));
                println!("{}", export::export_html(session, &output)?);
            }
            Ok(true)
        }
        SlashAction::Login { provider, key } => {
            login_provider(&provider, key.as_deref())?;
            Ok(true)
        }
        SlashAction::Logout { provider } => {
            let mut storage = AuthStorage::create().map_err(|err| err.to_string())?;
            if let Some(provider) = provider {
                storage.remove(&provider).map_err(|e| e.to_string())?;
                println!("removed {provider}");
            }
            Ok(true)
        }
        SlashAction::Name(name) => {
            if let Some(session) = agent.session.as_mut() {
                session.set_name(&name).map_err(|e| e.to_string())?;
            }
            Ok(true)
        }
        SlashAction::Fork => {
            if let Some(session) = &agent.session {
                let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
                let next = session
                    .fork(
                        session.leaf_id.as_deref().unwrap_or(&session.header.id),
                        &session_dir,
                    )
                    .map_err(|e| e.to_string())?;
                agent.load_from_session(next);
                println!("session={}", agent.session.as_ref().unwrap().header.id);
            }
            Ok(true)
        }
        SlashAction::Clone => {
            if let Some(session) = &agent.session {
                let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
                let next = session
                    .clone_session(&session_dir)
                    .map_err(|e| e.to_string())?;
                agent.load_from_session(next);
                println!("session={}", agent.session.as_ref().unwrap().header.id);
            }
            Ok(true)
        }
        SlashAction::ScopedModels => {
            chrome.status = "Model Configuration".into();
            println!("{}", chrome.status);
            Ok(true)
        }
        SlashAction::Resume => {
            let items = discover_session_items(parsed, agent)?;
            let mut selector = pi_tui::SessionSelector::new(items.clone());
            selector.set_cwd(agent.cwd.to_string_lossy().into_owned());
            chrome.session_selector = Some(selector);
            chrome.selector = None;
            chrome.status = "Select session".into();
            for item in items {
                println!(
                    "{}  {}",
                    item.name.as_deref().unwrap_or(&item.id),
                    item.path
                );
            }
            Ok(true)
        }
        SlashAction::Tree => {
            chrome.status = "Session Tree".into();
            println!("{}", chrome.status);
            Ok(true)
        }
        SlashAction::Copy => {
            if let Some(text) = agent.last_assistant_text() {
                println!("{text}");
            }
            Ok(true)
        }
        SlashAction::Trust => {
            let mut settings = load_settings(&default_agent_dir());
            let cwd = agent.cwd.display().to_string();
            if !settings.trusted_projects.contains(&cwd) {
                settings.trusted_projects.push(cwd);
            }
            save_settings(&default_agent_dir(), &settings)?;
            println!("trusted");
            Ok(true)
        }
        SlashAction::Reload => {
            agent.skills = discover_skills(&[agent.cwd.join(".pi").join("skills")]);
            agent.templates = discover_prompt_templates(&[agent.cwd.join(".pi").join("prompts")]);
            agent.context_files = load_context_files(&agent.cwd, true);
            chrome.status = "reloaded".into();
            println!("reloaded");
            Ok(true)
        }
        SlashAction::Import(path) => {
            if path.is_empty() {
                chrome.status = "Usage: /import <path.jsonl>".into();
                println!("{}", chrome.status);
                return Ok(true);
            }
            let expanded = pi_session::expand_tilde(&path);
            let next = JsonlSession::open(&expanded).map_err(|err| err.to_string())?;
            agent.load_from_session(next);
            chrome.status = format!(
                "imported {}",
                agent
                    .session
                    .as_ref()
                    .map(|item| item.header.id.clone())
                    .unwrap_or_default()
            );
            println!("{}", chrome.status);
            Ok(true)
        }
        SlashAction::Share => {
            chrome.status = share_current_session(agent)?;
            println!("{}", chrome.status);
            Ok(true)
        }
        SlashAction::Changelog => {
            let entries = changelog::parse_changelog(&changelog::changelog_path());
            let text = changelog::format_changelog(&entries);
            chrome.transcript.push("changelog", &text);
            chrome.status = "changelog".into();
            println!("{text}");
            Ok(true)
        }
    }
}

fn share_current_session(agent: &Agent) -> Result<String, String> {
    if std::env::var("PI_SHARE_DRY_RUN").is_ok() {
        let viewer = std::env::var("PI_SHARE_VIEWER_URL")
            .unwrap_or_else(|_| "https://pi.dev/session/".into());
        let url = format!("{viewer}dry-run");
        return Ok(format!("Share URL: {url}"));
    }
    if let Ok(url) = std::env::var("PI_SHARE_URL") {
        return Ok(format!("Share URL: {url}"));
    }
    let Some(session) = &agent.session else {
        return Ok("No session to share".into());
    };
    let tmp = std::env::temp_dir().join("pi-share-session.html");
    export::export_html(session, &tmp)?;
    let output = std::process::Command::new("gh")
        .args(["gist", "create", "--public=false"])
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    match output {
        Ok(result) if result.status.success() => {
            let gist_url = String::from_utf8_lossy(&result.stdout).trim().to_string();
            let gist_id = gist_url.rsplit('/').next().unwrap_or_default();
            let viewer = std::env::var("PI_SHARE_VIEWER_URL")
                .unwrap_or_else(|_| "https://pi.dev/session/".into());
            Ok(format!("Share URL: {viewer}{gist_id}\nGist: {gist_url}"))
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            if stderr.contains("not logged in") || stderr.contains("auth") {
                Ok("GitHub CLI is not logged in. Run 'gh auth login' first.".into())
            } else {
                Ok(format!("Failed to create gist: {}", stderr.trim()))
            }
        }
        Err(_) => {
            Ok("GitHub CLI (gh) is not installed. Install it from https://cli.github.com/".into())
        }
    }
}

fn apply_tool_events(chrome: &mut ChatChrome, events: &[AgentEvent]) {
    for event in events {
        match event {
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                chrome
                    .tool_cards
                    .push(ToolCard::start(tool_name, tool_call_id, args.clone()));
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => {
                if let Some(card) = chrome
                    .tool_cards
                    .iter_mut()
                    .find(|card| card.tool_call_id == *tool_call_id)
                {
                    card.update_partial(partial_result);
                }
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                let finished = chrome
                    .tool_cards
                    .iter_mut()
                    .find(|card| card.tool_call_id == *tool_call_id)
                    .map(|card| {
                        card.finish(result, *is_error);
                        (card.image_payloads(), card.format_tool_execution())
                    });
                if let Some((images, formatted)) = finished {
                    let base_id = chrome.tool_cards.len() as u32;
                    for (index, (data, _)) in images.into_iter().enumerate() {
                        chrome.transcript.push(
                            "image",
                            encode_kitty(
                                &data,
                                Some(40),
                                Some(1),
                                Some(base_id + index as u32),
                                false,
                            ),
                        );
                    }
                    chrome.transcript.push("tool", formatted);
                }
            }
            _ => {}
        }
    }
}

fn apply_interactive_setting(session: &mut InteractiveSession, spec: &str) -> Result<(), String> {
    let (id, value) = spec.split_once('=').unwrap_or((spec, ""));
    session.chrome.status = spec.to_string();
    match id {
        "double-escape-action" => {
            session.double_escape_action = DoubleEscapeAction::parse(value);
        }
        "autocomplete-max-visible" => {
            if let Ok(n) = value.parse::<u32>() {
                session.autocomplete_max_visible = n.clamp(3, 20) as usize;
            }
        }
        "theme" => apply_theme_value(session, value),
        "warnings.anthropic-extra-usage" => {
            session.warnings_anthropic_extra_usage = value == "true";
        }
        "model-thinking" => {
            if let Some((key, level)) = value.rsplit_once('=') {
                if level == "__clear__" {
                    session.model_thinking_levels.remove(key);
                } else {
                    session
                        .model_thinking_levels
                        .insert(key.to_string(), level.to_string());
                }
            }
        }
        "tree-filter-mode" => {
            session.tree_filter_mode = FilterMode::parse(value);
        }
        "mermaid-rendering" => {
            session.mermaid_mode = MermaidMode::parse(value);
            session.chrome.transcript.mermaid_mode = session.mermaid_mode;
        }
        "hide-thinking" => {
            session.chrome.transcript.hide_thinking_block = value == "true";
        }
        _ => {}
    }
    let dir = default_agent_dir();
    let mut stored = load_settings(&dir);
    match id {
        "double-escape-action" => stored.double_escape_action = Some(value.to_string()),
        "autocomplete-max-visible" => {
            stored.autocomplete_max_visible = value.parse().ok();
        }
        "theme" => stored.theme = Some(value.to_string()),
        "warnings.anthropic-extra-usage" => {
            stored.warnings.anthropic_extra_usage = Some(value == "true");
        }
        "model-thinking" => {
            if let Some((key, level)) = value.rsplit_once('=') {
                let mut map = stored.model_thinking_levels.take().unwrap_or_default();
                if level == "__clear__" {
                    map.remove(key);
                } else {
                    map.insert(key.to_string(), level.to_string());
                }
                stored.model_thinking_levels = Some(map);
            }
        }
        "quiet-startup" => stored.quiet_startup = value == "true",
        "tree-filter-mode" => stored.tree_filter_mode = Some(value.to_string()),
        "mermaid-rendering" => stored.markdown.mermaid = Some(value.to_string()),
        "enable-analytics" => set_enable_analytics(&mut stored, value == "true"),
        "autocompact" => stored.auto_compact = Some(value == "true"),
        "steering-mode" => stored.steering_mode = Some(value.to_string()),
        "follow-up-mode" => stored.follow_up_mode = Some(value.to_string()),
        "transport" => stored.transport = Some(value.to_string()),
        "http-idle-timeout" => stored.http_idle_timeout_ms = parse_http_idle_timeout(value),
        "hide-thinking" => stored.hide_thinking_block = Some(value == "true"),
        "cache-miss-notices" => stored.show_cache_miss_notices = Some(value == "true"),
        "collapse-changelog" => stored.collapse_changelog = Some(value == "true"),
        "install-telemetry" => stored.enable_install_telemetry = Some(value == "true"),
        "default-project-trust" => {
            stored.default_project_trust = Some(default_project_trust_value(value).into());
        }
        "tui-mode" => stored.tui_mode = Some(value.to_string()),
        "fullscreen-exit-output" => stored.fullscreen_exit_output = Some(value.to_string()),
        "fullscreen-scrollbar" => stored.fullscreen_scrollbar = Some(value.to_string()),
        "fullscreen-copy-on-select" => stored.fullscreen_copy_on_select = Some(value == "true"),
        "show-images" => stored.show_images = Some(value == "true"),
        "image-width-cells" => stored.image_width_cells = value.parse().ok(),
        "auto-resize-images" => stored.auto_resize_images = Some(value == "true"),
        "block-images" => stored.block_images = Some(value == "true"),
        "skill-commands" => stored.enable_skill_commands = Some(value == "true"),
        "show-hardware-cursor" => stored.show_hardware_cursor = Some(value == "true"),
        "editor-padding" => stored.editor_padding_x = value.parse().ok(),
        "output-padding" => stored.output_pad = value.parse().ok(),
        "clear-on-shrink" => stored.clear_on_shrink = Some(value == "true"),
        "terminal-progress" => stored.show_terminal_progress = Some(value == "true"),
        _ => {}
    }
    save_settings(&dir, &stored)?;
    Ok(())
}

fn looks_like_oauth_input(value: &str) -> bool {
    value.starts_with("pi-fixture-")
        || value.contains("://")
        || value.contains("code=")
        || value.contains('#')
}

fn login_provider(provider: &str, key: Option<&str>) -> Result<(), String> {
    let mut storage = AuthStorage::create().map_err(|err| err.to_string())?;
    if let Some(key) = key {
        if looks_like_oauth_input(key) {
            let (code, _) = pi_ai::parse_authorization_input(key);
            let code = code.ok_or_else(|| "Missing authorization code.".to_string())?;
            let pkce = pi_ai::generate_pkce(uuid::Uuid::new_v4().as_bytes());
            let (access, refresh) =
                pi_ai::exchange_authorization_code(provider, &code, Some(&pkce))?;
            storage
                .login_oauth(provider, access, refresh, None)
                .map_err(|err| err.to_string())?;
            println!("stored oauth token for {provider}");
            return Ok(());
        }
        storage
            .login_api_key(provider, key)
            .map_err(|err| err.to_string())?;
        println!("stored api key for {provider}");
        return Ok(());
    }
    if provider.is_empty() {
        println!("Usage: /login <provider> <api-key>");
        return Ok(());
    }
    let id = uuid::Uuid::new_v4();
    let pkce = pi_ai::generate_pkce(id.as_bytes());
    if let Some(request) = pi_ai::authorize_request(provider, &pkce, "pi") {
        println!("{}", request.url);
        println!("{}", request.instructions);
        if let Ok(code) = std::env::var("PI_OAUTH_CODE") {
            let (access, refresh) =
                pi_ai::exchange_authorization_code(provider, &code, request.pkce.as_ref())?;
            storage
                .login_oauth(provider, access, refresh, None)
                .map_err(|err| err.to_string())?;
            println!("stored oauth token for {provider}");
            return Ok(());
        }
        if std::env::var("PI_OAUTH_WAIT").is_ok() {
            if let Some(kind) = pi_ai::CallbackProvider::parse(provider) {
                let host = pi_ai::callback_host();
                let port = std::env::var("PI_OAUTH_CALLBACK_PORT")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let expected = request
                    .state
                    .clone()
                    .unwrap_or_else(|| pkce.verifier.clone());
                let mut server = pi_ai::CallbackServer::bind(&host, port, kind, expected)?;
                println!("Waiting for browser callback on {}", server.redirect_uri()?);
                let response = server.accept_one()?;
                if let Some(code) = response.code {
                    let (access, refresh) =
                        pi_ai::exchange_authorization_code(provider, &code, request.pkce.as_ref())?;
                    storage
                        .login_oauth(provider, access, refresh, None)
                        .map_err(|err| err.to_string())?;
                    println!("stored oauth token for {provider}");
                    return Ok(());
                }
            }
        }
        return Ok(());
    }
    if let (Ok(access), refresh) = (
        std::env::var("PI_OAUTH_ACCESS"),
        std::env::var("PI_OAUTH_REFRESH").ok(),
    ) {
        storage
            .login_oauth(provider, access, refresh, None)
            .map_err(|err| err.to_string())?;
        println!("stored oauth token for {provider}");
        return Ok(());
    }
    println!("Usage: /login <provider> <api-key>");
    Ok(())
}

fn loaded_extension_host(parsed: &Args) -> ExtensionHost {
    let stored = load_settings(&default_agent_dir());
    let mut extensions = stored.extensions.clone();
    extensions.extend(parsed.extensions.clone());
    if parsed.no_extensions {
        ExtensionHost::default()
    } else {
        ExtensionHost::load(&default_agent_dir(), &extensions)
    }
}

fn replay_custom_messages(agent: &Agent, session: &mut InteractiveSession, host: &ExtensionHost) {
    let Some(store) = agent.session.as_ref() else {
        return;
    };
    for entry in &store.entries {
        if entry.entry_type != "custom_message" && entry.entry_type != "custom" {
            continue;
        }
        let custom_type = entry
            .custom_type
            .clone()
            .or_else(|| {
                entry
                    .extra
                    .get("customType")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "custom".into());
        let content = entry
            .message
            .as_ref()
            .map(CustomMessage::text_content)
            .filter(|text| !text.is_empty())
            .or_else(|| {
                entry
                    .extra
                    .get("content")
                    .map(CustomMessage::text_content)
                    .filter(|text| !text.is_empty())
            })
            .unwrap_or_default();
        let lines = if entry.entry_type == "custom" {
            let data = entry
                .extra
                .get("data")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            host.get_entry_renderer(&custom_type)
                .and_then(|_| host.render_custom_entry(&custom_type, &data, false, session.width))
        } else {
            host.get_message_renderer(&custom_type).and_then(|_| {
                host.render_custom_message(&custom_type, &content, false, 1, session.width)
            })
        };
        session
            .chrome
            .transcript
            .push_custom(custom_type, content, lines);
    }
}

fn tree_entry_from_session(entry: &pi_session::SessionEntry) -> SessionTreeEntry {
    let message = entry.message.clone().unwrap_or(serde_json::Value::Null);
    SessionTreeEntry {
        id: entry.id.clone(),
        parent_id: entry.parent_id.clone(),
        entry_type: entry.entry_type.clone(),
        role: message
            .get("role")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        stop_reason: message
            .get("stopReason")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        error_message: message
            .get("errorMessage")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        tool_call_id: message
            .get("toolCallId")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        tool_name: message
            .get("toolName")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        command: message
            .get("command")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        custom_type: entry.custom_type.clone().or_else(|| {
            entry
                .extra
                .get("customType")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        }),
        content: if entry.entry_type == "label" {
            Some(serde_json::json!({
                "targetId": entry.extra.get("targetId").and_then(|value| value.as_str()).unwrap_or("")
            }))
        } else {
            message.get("content").cloned()
        },
        model_id: entry
            .extra
            .get("modelId")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        thinking_level: entry
            .extra
            .get("thinkingLevel")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        label: entry
            .extra
            .get("label")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        name: entry
            .extra
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        tokens_before: entry
            .extra
            .get("tokensBefore")
            .and_then(|value| value.as_u64()),
        summary: entry
            .extra
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }
}

fn apply_theme_value(session: &mut InteractiveSession, value: &str) {
    let resolved = resolve_theme_name(value);
    if let Some(theme) = builtin_themes()
        .into_iter()
        .find(|theme| theme.name == resolved)
    {
        session.chrome.theme = theme;
    }
}

fn resolve_theme_name(value: &str) -> String {
    if let Some((light, dark)) = parse_auto_theme(value) {
        let detected = detect_terminal_theme_for_auto(
            std::env::var("PI_COLOR_SCHEME_REPLY").ok().as_deref(),
            std::env::var("PI_OSC11_REPLY").ok().as_deref(),
            std::env::var("COLORFGBG").ok().as_deref(),
        );
        if detected.theme == "light" {
            light
        } else {
            dark
        }
    } else {
        value.to_string()
    }
}

fn apply_osc_theme(session: &mut InteractiveSession, detection: &ThemeDetection) {
    let stored = load_settings(&default_agent_dir());
    if let Some(setting) = stored.theme.as_deref() {
        if let Some((light, dark)) = parse_auto_theme(setting) {
            let name = if detection.theme == "light" {
                light
            } else {
                dark
            };
            apply_theme_value(session, &name);
            session.chrome.status = format!("theme={} ({})", name, detection.source);
        }
    }
}

fn launch_external_editor(session: &mut InteractiveSession) -> Result<(), String> {
    let stored = load_settings(&default_agent_dir());
    let editor = ExternalEditor::new(
        stored.external_editor.as_deref(),
        &session.chrome.editor.buffer,
    )?;
    session.chrome.status = editor.launch_message();
    println!("{}", session.chrome.status);
    let text = editor.edit()?;
    session.chrome.editor.buffer = text;
    session.chrome.editor.cursor = session.chrome.editor.buffer.len();
    Ok(())
}

fn paste_clipboard(session: &mut InteractiveSession) {
    if let Some(png) = clipboard_image_png() {
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
        session.place_kitty_image(&b64, Some(1));
        session.chrome.status = "pasted image".into();
    } else if let Some(text) = clipboard_text() {
        session.chrome.editor.handle_input(&text);
        session.chrome.status = "pasted clipboard".into();
    } else {
        session.chrome.status = "clipboard empty".into();
    }
}

fn discover_session_items(parsed: &Args, _agent: &Agent) -> Result<Vec<SessionItem>, String> {
    let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
    let sessions = discover_sessions(&session_dir, None).map_err(|err| err.to_string())?;
    Ok(sessions
        .into_iter()
        .map(|summary| SessionItem {
            id: summary.id,
            name: summary.name,
            path: summary.path.display().to_string(),
            cwd: summary.cwd,
            modified_at: summary.modified_at,
            parent_id: summary.parent_session_id,
        })
        .collect())
}

fn open_session_selector(
    parsed: &Args,
    agent: &Agent,
    session: &mut InteractiveSession,
) -> Result<(), String> {
    let items = discover_session_items(parsed, agent)?;
    session.open_session_selector(items);
    if let Some(selector) = &mut session.chrome.session_selector {
        selector.set_cwd(agent.cwd.to_string_lossy().into_owned());
    }
    if let Some(selector) = &session.chrome.session_selector {
        println!("{}", selector.render(80).join("\n"));
    }
    Ok(())
}

fn open_settings_overlay(session: &mut InteractiveSession) {
    let stored = load_settings(&default_agent_dir());
    session.open_settings_list(interactive_settings_list(&to_interactive_config(
        &stored,
        &session.chrome.theme.name,
    )));
    if let Some(settings) = &session.chrome.settings_list {
        println!("{}", settings.render(80).join("\n"));
    }
}

fn rename_discovered_session(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    id: &str,
    name: &str,
) -> Result<(), String> {
    let session_dir = resolve_session_dir(parsed.session_dir.as_deref());
    let summary = resolve_session_ref(&session_dir, Some(&agent.cwd.to_string_lossy()), id)
        .map_err(|err| err.to_string())?;
    let mut store = JsonlSession::open(&summary.path).map_err(|err| err.to_string())?;
    store.set_name(name).map_err(|err| err.to_string())?;
    if agent
        .session
        .as_ref()
        .is_some_and(|current| current.header.id == id)
    {
        if let Some(current) = agent.session.as_mut() {
            current.set_name(name).map_err(|err| err.to_string())?;
        }
    }
    if let Some(selector) = &mut session.chrome.session_selector {
        selector.apply_rename(id, name);
    }
    session.chrome.status = if name.is_empty() {
        format!("session {id} name cleared")
    } else {
        format!("session {id} renamed to {name}")
    };
    Ok(())
}

fn delete_discovered_session(
    agent: &Agent,
    session: &mut InteractiveSession,
    id: &str,
    path: &str,
) -> Result<(), String> {
    if agent
        .session
        .as_ref()
        .is_some_and(|current| current.header.id == id)
    {
        session.chrome.status = "Cannot delete the currently active session".into();
        return Ok(());
    }
    if std::env::var("PI_SESSION_DELETE_DRY_RUN").is_err() {
        let trash = std::process::Command::new("trash").arg(path).status();
        let trashed = trash.map(|status| status.success()).unwrap_or(false);
        if !trashed {
            std::fs::remove_file(path).map_err(|err| err.to_string())?;
        }
        session.chrome.status = if trashed {
            "Session moved to trash".into()
        } else {
            "Session deleted".into()
        };
    } else {
        session.chrome.status = "Session deleted".into();
    }
    if let Some(selector) = &mut session.chrome.session_selector {
        selector.remove(id);
    }
    Ok(())
}

fn open_session_tree(agent: &Agent, session: &mut InteractiveSession) {
    let entries = agent
        .session
        .as_ref()
        .map(|item| {
            item.entries
                .iter()
                .map(tree_entry_from_session)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let roots = pi_tui::build_session_tree(entries);
    let leaf = agent.session.as_ref().and_then(|item| item.leaf_id.clone());
    session.open_tree_overlay(roots, leaf);
    if let Some(tree) = &session.chrome.tree {
        println!("{}", tree.render(80).join("\n"));
    }
}

fn open_scoped_models(session: &mut InteractiveSession) {
    let models = load_builtin_models()
        .into_iter()
        .map(|model| ScopedModel {
            provider: model.provider,
            id: model.id,
            name: model.name,
        })
        .collect();
    session.open_scoped_models(models);
    if let Some(scoped) = &session.chrome.scoped_models {
        println!("{}", scoped.render(80).join("\n"));
    }
}

fn start_login(
    session: &mut InteractiveSession,
    provider: &str,
    key: Option<&str>,
) -> Result<(), String> {
    session.open_login_dialog(provider, None, None);
    if let Some(key) = key {
        login_provider(provider, Some(key))?;
        if let Some(dialog) = &mut session.chrome.login_dialog {
            dialog.show_progress(&format!("stored credentials for {provider}"));
        }
        return Ok(());
    }
    let pkce = pi_ai::generate_pkce(uuid::Uuid::new_v4().as_bytes());
    if let Some(request) = pi_ai::authorize_request(provider, &pkce, "pi") {
        if let Some(dialog) = &mut session.chrome.login_dialog {
            dialog.show_auth(&request.url, Some(request.instructions.as_str()));
            dialog.show_manual_input("Paste the redirect URL or authorization code");
        }
        println!("{}", request.url);
        println!("{}", request.instructions);
    } else if let Some(dialog) = &mut session.chrome.login_dialog {
        dialog.show_manual_input("Enter API key");
        println!("Usage: /login <provider> <api-key>");
    }
    if let Some(dialog) = &session.chrome.login_dialog {
        println!("{}", dialog.render(80).join("\n"));
    }
    Ok(())
}

fn apply_first_time_result(
    session: &mut InteractiveSession,
    theme: &str,
    share_analytics: bool,
) -> Result<(), String> {
    if let Some(found) = builtin_themes().into_iter().find(|item| item.name == theme) {
        session.chrome.theme = found;
    }
    let dir = default_agent_dir();
    let mut stored = load_settings(&dir);
    stored.theme = Some(theme.to_string());
    set_enable_analytics(&mut stored, share_analytics);
    save_settings(&dir, &stored)
}

#[allow(dead_code)]
fn store_api_key(provider: &str, key: &str) -> Result<(), String> {
    let mut storage = AuthStorage::create().map_err(|err| err.to_string())?;
    storage
        .set(
            provider,
            Credential {
                kind: CredentialKind::ApiKey,
                key: Some(key.to_string()),
                access: None,
                refresh: None,
                expires: None,
                env: Default::default(),
            },
        )
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_print_and_thinking_like_ts() {
        let parsed = parse_args(&[
            "-p".into(),
            "hello".into(),
            "--thinking".into(),
            "high".into(),
        ]);
        assert!(parsed.print);
        assert_eq!(parsed.messages, ["hello"]);
        assert_eq!(parsed.thinking, Some(pi_protocol::ThinkingLevel::High));
    }

    #[test]
    fn help_lists_product_commands() {
        let help = print_help();
        for needle in [
            "install", "remove", "update", "list", "config", "auth", "--print", "--resume",
        ] {
            assert!(help.contains(needle), "missing {needle}");
        }
        assert_eq!(
            args::normalize_session_name("  demo  ").as_deref(),
            Some("demo")
        );
        let usage = auth_cmd::get_auth_command_usage(auth_cmd::AuthCommandKind::Check);
        assert!(usage.contains("auth check"));
        let command = auth_cmd::parse_auth_command(&[
            "auth".into(),
            "check".into(),
            "--provider".into(),
            "openai".into(),
            "--no-refresh".into(),
        ])
        .unwrap()
        .unwrap();
        assert!(command.no_refresh);
        assert!(auth_cmd::parsed_auth_args(&command).provider.is_some());
        assert!(command.min_expiry_ms.is_none());
        assert!(matches!(
            slash::parse_line("/quit"),
            slash::SlashAction::Quit
        ));
        assert!(matches!(
            slash::parse_line("hello"),
            slash::SlashAction::Prompt(_)
        ));
        assert!(matches!(
            slash::parse_line("/model"),
            slash::SlashAction::OpenModel
        ));
        assert!(matches!(
            slash::parse_line("/scoped-models"),
            slash::SlashAction::ScopedModels
        ));
        assert!(matches!(
            slash::parse_line("/tree"),
            slash::SlashAction::Tree
        ));
        assert!(matches!(
            slash::parse_line("/import ./foo.jsonl"),
            slash::SlashAction::Import(_)
        ));
        assert!(matches!(
            slash::parse_line("/share"),
            slash::SlashAction::Share
        ));
        assert!(matches!(
            slash::parse_line("/changelog"),
            slash::SlashAction::Changelog
        ));
    }
}
