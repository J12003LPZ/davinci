mod args;
mod commands;
mod export;
mod extensions;
mod rpc;
mod settings;
mod slash;

use std::io::{self, BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{
    normalize_session_name, parse_args, print_help, Args, ListModels, Mode, APP_NAME, VERSION,
};
use commands::dispatch_subcommand;
use pi_agent::context::{load_context_files, render_system_prompt};
use pi_agent::{
    run_agent, AgentConfig, AgentEvent, AgentMessage, AllowAllPermissionPolicy, FollowUpQueue,
    SteerQueue, ToolRegistry,
};
use pi_ai::catalog::resolve_model;
use pi_ai::list_models;
use pi_ai::{get_env_api_key, AuthStorage, FileAuthStorage};
use pi_session::{
    append_session_name, clone_session, continue_latest, create_session, default_sessions_root,
    fork_session, resume_by_id_or_path,
};
use pi_tui::component::Component;
use pi_tui::{enter_alt_screen, leave_alt_screen, Editor, Markdown, SelectList, Text, TuiMode};

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match run(&raw, io::stdin().is_terminal(), io::stdout().is_terminal()) {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(raw: &[String], stdin_tty: bool, stdout_tty: bool) -> Result<i32, String> {
    let package = dispatch_subcommand(raw);
    if package.handled {
        print!("{}", package.stdout);
        eprint!("{}", package.stderr);
        return Ok(package.exit_code);
    }

    let parsed = parse_args(raw);
    for diagnostic in &parsed.diagnostics {
        if diagnostic.kind == "error" {
            eprintln!("Error: {}", diagnostic.message);
        } else {
            eprintln!("Warning: {}", diagnostic.message);
        }
    }
    if parsed.diagnostics.iter().any(|d| d.kind == "error") {
        return Ok(1);
    }

    if parsed.version {
        println!("{VERSION}");
        return Ok(0);
    }
    if parsed.help {
        print!("{}", print_help());
        return Ok(0);
    }
    if let Some(export) = &parsed.export {
        let output = parsed.messages.first().map(String::as_str);
        match export::export_from_file(export, output) {
            Ok(path) => {
                println!("Exported to: {}", path.display());
                return Ok(0);
            }
            Err(err) => {
                eprintln!("Error: {err}");
                return Ok(1);
            }
        }
    }
    if let Some(list) = &parsed.list_models {
        return list_models_cmd(list, parsed.provider.as_deref());
    }

    if parsed.mode == Some(Mode::Rpc) && !parsed.file_args.is_empty() {
        eprintln!("Error: @file arguments are not supported in RPC mode");
        return Ok(1);
    }

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let session_dir = parsed
        .session_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(default_sessions_root);

    let mut session = if parsed.no_session {
        None
    } else if parsed.continue_session {
        continue_latest(&session_dir, Some(&cwd.to_string_lossy()))
            .ok()
            .flatten()
    } else if let Some(query) = parsed.session.as_ref().or(parsed.fork.as_ref()) {
        resume_by_id_or_path(&session_dir, query, Some(&cwd.to_string_lossy()))
            .ok()
            .flatten()
    } else if let Some(id) = &parsed.session_id {
        resume_by_id_or_path(&session_dir, id, Some(&cwd.to_string_lossy()))
            .ok()
            .flatten()
            .or_else(|| create_session(&session_dir, &cwd.to_string_lossy(), Some(id)).ok())
    } else if parsed.resume {
        continue_latest(&session_dir, Some(&cwd.to_string_lossy()))
            .ok()
            .flatten()
    } else {
        create_session(&session_dir, &cwd.to_string_lossy(), None).ok()
    };

    if parsed.fork.is_some() {
        if let Some(source) = session.take() {
            session = fork_session(&session_dir, &source, &cwd.to_string_lossy()).ok();
        }
    }

    if let Some(name) = parsed.name.as_deref().and_then(normalize_session_name) {
        if let Some(session) = &session {
            let _ = append_session_name(&session.path, &name);
        }
    } else if parsed.name.is_some() {
        eprintln!("Error: --name requires a non-empty value");
        return Ok(1);
    }

    let app_mode = resolve_mode(&parsed, stdin_tty, stdout_tty);
    let context = load_context_files(&cwd, parsed.no_context_files);
    let system_prompt = render_system_prompt(
        parsed.system_prompt.as_deref(),
        &parsed.append_system_prompt,
        &context,
    );
    let discovered = extensions::discover_extensions(
        &commands::agent_dir(),
        &cwd,
        &parsed.extensions,
        parsed.no_extensions,
    );
    if parsed.verbose {
        let settings = extensions::load_settings_value(&commands::settings_path(false));
        let packages = extensions::settings_packages(&settings);
        if !discovered.is_empty() || !packages.is_empty() {
            eprintln!(
                "extensions: {}",
                discovered
                    .iter()
                    .map(|e| format!("{}={}", e.name, e.path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if !packages.is_empty() {
                eprintln!("packages: {}", packages.join(", "));
            }
        }
    }
    let tools = build_tools(&parsed);
    let model_spec = parsed.model.as_deref().unwrap_or("google/gemini-2.5-flash");
    let (model_id, thinking_from_model) = split_thinking(model_spec);
    let model = resolve_model(model_id);
    let provider = parsed
        .provider
        .clone()
        .or_else(|| model.as_ref().map(|m| m.provider.clone()))
        .unwrap_or_else(|| "google".into());
    let model_id = model
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| model_id.to_string());

    let mut messages = collect_messages(&parsed, &cwd)?;
    let mut steer = SteerQueue::default();
    let mut follow_up = FollowUpQueue::default();
    let mut thinking = parsed
        .thinking
        .map(|t| pi_agent::ThinkingLevel::parse(t.as_str()).unwrap_or(pi_agent::ThinkingLevel::Off))
        .or(thinking_from_model)
        .unwrap_or(pi_agent::ThinkingLevel::Off);

    match app_mode {
        AppMode::Rpc => run_rpc(
            &mut messages,
            &mut steer,
            &mut follow_up,
            &mut thinking,
            &tools,
        ),
        AppMode::Print | AppMode::Json => run_print(
            &parsed,
            &cwd,
            &provider,
            &model_id,
            &system_prompt,
            &tools,
            &messages,
            &mut steer,
            &mut follow_up,
            app_mode == AppMode::Json,
        ),
        AppMode::Interactive => run_interactive(
            &parsed,
            &cwd,
            &provider,
            &model_id,
            &system_prompt,
            &tools,
            &messages,
            session.as_ref().map(|s| s.path.clone()),
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Interactive,
    Print,
    Json,
    Rpc,
}

fn resolve_mode(parsed: &Args, stdin_tty: bool, stdout_tty: bool) -> AppMode {
    if parsed.mode == Some(Mode::Rpc) {
        return AppMode::Rpc;
    }
    if parsed.mode == Some(Mode::Json) {
        return AppMode::Json;
    }
    if parsed.print || !stdin_tty || !stdout_tty {
        return AppMode::Print;
    }
    AppMode::Interactive
}

fn split_thinking(spec: &str) -> (&str, Option<pi_agent::ThinkingLevel>) {
    if let Some((model, level)) = spec.rsplit_once(':') {
        if let Some(parsed) = pi_agent::ThinkingLevel::parse(level) {
            return (model, Some(parsed));
        }
    }
    (spec, None)
}

fn build_tools(parsed: &Args) -> ToolRegistry {
    if parsed.no_tools {
        return ToolRegistry::with_names(&[]);
    }
    let mut tools = if parsed.no_builtin_tools {
        ToolRegistry::with_names(&[])
    } else if parsed.tools.is_empty() {
        ToolRegistry::builtins()
    } else {
        ToolRegistry::with_names(&parsed.tools)
    };
    if !parsed.exclude_tools.is_empty() {
        tools = tools.exclude(&parsed.exclude_tools);
    }
    tools
}

fn collect_messages(parsed: &Args, cwd: &Path) -> Result<Vec<AgentMessage>, String> {
    let mut parts = parsed.messages.clone();
    for file in &parsed.file_args {
        let path = if file.starts_with('/') {
            PathBuf::from(file)
        } else {
            cwd.join(file)
        };
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read @{file}: {e}"))?;
        parts.push(format!("# {file}\n{content}"));
    }
    if parts.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![AgentMessage {
            role: "user".into(),
            content: parts.join("\n\n"),
            images: vec![],
        }])
    }
}

fn list_models_cmd(list: &ListModels, provider: Option<&str>) -> Result<i32, String> {
    let query = match list {
        ListModels::All => "",
        ListModels::Search(q) => q.as_str(),
    }
    .to_ascii_lowercase();
    let mut models = list_models(provider);
    if !query.is_empty() {
        models.retain(|m| {
            m.id.to_ascii_lowercase().contains(&query)
                || m.name.to_ascii_lowercase().contains(&query)
                || m.provider.to_ascii_lowercase().contains(&query)
        });
    }
    for model in models {
        println!("{}/{}  {}", model.provider, model.id, model.name);
    }
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
fn run_print(
    parsed: &Args,
    cwd: &Path,
    provider: &str,
    model_id: &str,
    system_prompt: &str,
    tools: &ToolRegistry,
    messages: &[AgentMessage],
    steer: &mut SteerQueue,
    follow_up: &mut FollowUpQueue,
    json: bool,
) -> Result<i32, String> {
    if messages.is_empty() {
        eprintln!("Error: print mode requires a prompt");
        return Ok(1);
    }
    let config = agent_config(parsed, cwd, provider, model_id, system_prompt);
    let events =
        run_agent(&config, messages, tools, steer, follow_up).map_err(|e| e.to_string())?;
    if json {
        for event in events {
            println!("{}", serde_json::to_string(&event).unwrap());
        }
    } else {
        for event in events {
            if let AgentEvent::Message { message } = event {
                println!("{}", message.content);
            }
        }
    }
    let _ = parsed;
    Ok(0)
}

fn run_rpc(
    messages: &mut Vec<AgentMessage>,
    steer: &mut SteerQueue,
    follow_up: &mut FollowUpQueue,
    thinking: &mut pi_agent::ThinkingLevel,
    tools: &ToolRegistry,
) -> Result<i32, String> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("Invalid RPC JSON: {e}"))?;
        let reply = rpc::handle_rpc(&value, messages, steer, follow_up, thinking, tools);
        println!("{}", reply);
    }
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
fn run_interactive(
    parsed: &Args,
    cwd: &Path,
    provider: &str,
    model_id: &str,
    system_prompt: &str,
    tools: &ToolRegistry,
    initial: &[AgentMessage],
    session_path: Option<PathBuf>,
) -> Result<i32, String> {
    let fullscreen = parsed.tui_mode == Some(TuiMode::Fullscreen);
    let mut stdout = io::stdout();
    if fullscreen {
        enter_alt_screen(&mut stdout).ok();
    }
    println!("{APP_NAME} {VERSION}  {provider}/{model_id}");
    if let Some(path) = &session_path {
        println!("session {}", path.display());
    }
    let commands: Vec<String> = slash::BUILTIN_SLASH_COMMANDS
        .iter()
        .map(|(n, _)| format!("/{n}"))
        .collect();
    let selector = SelectList::new(commands);
    for line in selector.render(80) {
        println!("{line}");
    }
    let _ = tools;
    let editor = Editor::default();
    for line in editor.render(80) {
        let _ = Text::new(line);
    }
    let mut messages = initial.to_vec();
    let mut steer = SteerQueue::default();
    let mut follow = FollowUpQueue::default();
    if !messages.is_empty() {
        print_agent_turn(
            parsed,
            cwd,
            provider,
            model_id,
            system_prompt,
            tools,
            &messages,
            &mut steer,
            &mut follow,
            session_path.as_deref(),
        )?;
    } else {
        println!("Type a prompt, or /help. Ctrl+C to quit.");
    }
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some((cmd, args)) = slash::parse_slash(&line) {
            match cmd {
                "quit" | "exit" => break,
                "help" => {
                    for (name, desc) in slash::BUILTIN_SLASH_COMMANDS {
                        println!("/{name}  {desc}");
                    }
                }
                "clone" => {
                    if let Some(path) = &session_path {
                        if let Ok(info) = pi_session::read_session_info(path) {
                            let _ = clone_session(
                                &default_sessions_root(),
                                &info,
                                &cwd.to_string_lossy(),
                            );
                        }
                    }
                }
                "trust" => {
                    settings::save_trust(&commands::agent_dir(), cwd);
                    println!("Trusted {}", cwd.display());
                }
                "login" => {
                    if let Some(url) = pi_ai::authorize_url(args, "http://127.0.0.1:8765/cb", "pi")
                    {
                        println!("Open: {url}");
                    } else {
                        println!("Usage: /login <provider>");
                    }
                }
                other => println!("/{other} {args}"),
            }
            continue;
        }
        messages.push(AgentMessage {
            role: "user".into(),
            content: line,
            images: vec![],
        });
        print_agent_turn(
            parsed,
            cwd,
            provider,
            model_id,
            system_prompt,
            tools,
            &messages,
            &mut steer,
            &mut follow,
            session_path.as_deref(),
        )?;
    }
    if fullscreen {
        leave_alt_screen(&mut stdout).ok();
    }
    Ok(0)
}

fn resolve_api_key(parsed: &Args, provider: &str) -> Option<String> {
    if let Some(key) = &parsed.api_key {
        return Some(key.clone());
    }
    if let Some(key) = get_env_api_key(provider) {
        return Some(key);
    }
    FileAuthStorage::open(commands::agent_dir().join("auth.json"))
        .ok()
        .and_then(|store| store.read(provider))
        .map(|cred| match cred {
            pi_ai::Credential::ApiKey { ref key, .. } => key.clone(),
            pi_ai::Credential::Oauth { ref access, .. } => access.clone(),
        })
}

fn agent_config(
    parsed: &Args,
    cwd: &Path,
    provider: &str,
    model_id: &str,
    system_prompt: &str,
) -> AgentConfig {
    let fixture = std::env::var("PI_STREAM_FIXTURE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok());
    AgentConfig {
        cwd: cwd.to_path_buf(),
        system_prompt: system_prompt.to_string(),
        model_provider: provider.to_string(),
        model_id: model_id.to_string(),
        api_key: resolve_api_key(parsed, provider),
        allow_network: fixture.is_none() && !parsed.offline,
        auto_retry: true,
        max_retries: 2,
        auto_compact: true,
        context_window: 128_000,
        max_turns: 16,
        fixture,
        permission: Box::new(AllowAllPermissionPolicy),
    }
}

#[allow(clippy::too_many_arguments)]
fn print_agent_turn(
    parsed: &Args,
    cwd: &Path,
    provider: &str,
    model_id: &str,
    system_prompt: &str,
    tools: &ToolRegistry,
    messages: &[AgentMessage],
    steer: &mut SteerQueue,
    follow_up: &mut FollowUpQueue,
    session_path: Option<&Path>,
) -> Result<i32, String> {
    let config = agent_config(parsed, cwd, provider, model_id, system_prompt);
    let events =
        run_agent(&config, messages, tools, steer, follow_up).map_err(|e| e.to_string())?;
    for event in events {
        match event {
            AgentEvent::Message { message } => {
                let md = Markdown::new(&message.content);
                for line in md.render(80) {
                    println!("{line}");
                }
                if let Some(path) = session_path {
                    let _ = pi_session::append_entry(
                        path,
                        &serde_json::json!({"type":"message","role": message.role, "content": message.content}),
                    );
                }
            }
            AgentEvent::ToolStart { name, .. } => println!("▶ {name}"),
            AgentEvent::ToolEnd { name, output, .. } => {
                println!("■ {name}\n{output}");
            }
            AgentEvent::Error { message } => eprintln!("Error: {message}"),
            _ => {}
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_and_version() {
        assert_eq!(run(&["--version".into()], true, true).unwrap(), 0);
        assert_eq!(run(&["--help".into()], true, true).unwrap(), 0);
    }
}
