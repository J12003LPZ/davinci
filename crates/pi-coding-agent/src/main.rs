mod args;
mod auth_cmd;
mod export;
mod packages;
mod rpc;
mod settings;
mod slash;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use pi_agent::{
    default_system_prompt, discover_prompt_templates, discover_skills, load_context_files, Agent,
};
use pi_ai::{
    content_text, find_model, fuzzy_models, live_complete, load_builtin_models,
    resolve_provider_auth, AuthStorage, Credential, CredentialKind,
};
use pi_session::{
    default_agent_dir, discover_sessions, latest_session, resolve_session_dir, resolve_session_ref,
    JsonlSession,
};
use pi_tui::{builtin_themes, Component, Editor, TuiMode};

use args::{parse_args, print_help, Args, ListModels, Mode, APP_NAME, VERSION};
use auth_cmd::{
    is_auth_command_help, parse_auth_command, print_auth_command_help, validate_auth_command_args,
    AuthCommandKind,
};
use packages::handle_package_command;
use rpc::{handle_rpc, RpcCommand};
use settings::{is_trusted, load_settings};

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
    if !parsed.extensions.is_empty() {
        agent.apply_extension_tools(&parsed.extensions);
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
    let storage = AuthStorage::create().map_err(|err| err.to_string())?;
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

fn complete_prompt(parsed: &Args, agent: &mut Agent) -> String {
    let offline = parsed.offline
        || matches!(
            std::env::var("PI_OFFLINE").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    let models = load_builtin_models();
    let (provider, model_id) = parse_model_ref(
        parsed.provider.as_deref().unwrap_or("google"),
        parsed.model.as_deref(),
    );
    let model = find_model(&models, &provider, &model_id)
        .cloned()
        .or_else(|| models.iter().find(|m| m.provider == provider).cloned())
        .or_else(|| models.first().cloned());
    let storage = AuthStorage::create().ok();
    let env = std::env::vars().collect();
    let auth = storage
        .as_ref()
        .and_then(|storage| resolve_provider_auth(&provider, storage, &env, true));
    let last_user = agent
        .messages
        .last()
        .map(|m| content_text(&m.content).len())
        .unwrap_or(0);
    let reply = match (offline, model, auth) {
        (false, Some(model), Some(auth)) => {
            match live_complete(&model, &agent.messages, &auth, Some(&agent.system_prompt)) {
                Ok(message) => message
                    .content
                    .into_iter()
                    .filter_map(|block| match block {
                        pi_ai::ContentBlock::Text { text } => Some(text),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
                Err(err) => format!("Provider error: {err}"),
            }
        }
        _ => format!("(offline) received {last_user} characters"),
    };
    agent.record_assistant(&reply);
    reply
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
    let reply = complete_prompt(parsed, agent);
    if parsed.mode == Some(Mode::Json) {
        println!(
            "{}",
            serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": reply,
            })
        );
    } else {
        println!("{reply}");
    }
    Ok(0)
}

fn run_rpc(agent: &mut Agent) -> Result<i32, String> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|err| err.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let command: RpcCommand = serde_json::from_str(&line).map_err(|err| err.to_string())?;
        let response = handle_rpc(agent, command);
        println!(
            "{}",
            serde_json::to_string(&response).map_err(|err| err.to_string())?
        );
        io::stdout().flush().ok();
    }
    Ok(0)
}

fn run_interactive(parsed: &Args, agent: &mut Agent) -> Result<i32, String> {
    if parsed.tui_mode == Some(TuiMode::Fullscreen) {
        print!("{}", pi_tui::ALT_BUFFER_ENTER);
    }
    let theme = parsed
        .use_theme
        .clone()
        .or_else(|| builtin_themes().into_iter().next().map(|t| t.name));
    println!(
        "{APP_NAME} {VERSION}  theme={}",
        theme.unwrap_or_else(|| "dark".into())
    );
    if !parsed.messages.is_empty() {
        let prompt = parsed.messages.join("\n");
        agent.prompt(&prompt);
        let reply = complete_prompt(parsed, agent);
        println!("{reply}");
    }
    let mut editor = Editor::new();
    print!("> ");
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).ok().unwrap_or(0) > 0 {
        editor.handle_input(input.trim_end());
        let text = editor.submit();
        if text == "/quit" {
            if parsed.tui_mode == Some(TuiMode::Fullscreen) {
                print!("{}", pi_tui::ALT_BUFFER_LEAVE);
            }
            return Ok(0);
        }
        if !text.is_empty() {
            agent.prompt(&text);
            let reply = complete_prompt(parsed, agent);
            println!("{reply}");
        }
    }
    if parsed.tui_mode == Some(TuiMode::Fullscreen) {
        print!("{}", pi_tui::ALT_BUFFER_LEAVE);
    }
    Ok(0)
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
    }
}
