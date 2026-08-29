mod ansi_to_html;
mod args;
mod changelog;
mod clipboard;
mod commands;
mod config_selector;
mod event_bus;
mod export;
mod extension_ui;
mod extensions;
mod interactive;
mod package_manager;
mod rpc;
mod self_update;
mod session_runtime;
mod settings;
mod share;
mod slash;
mod theme;
mod tool_html;

use std::io::{self, BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{normalize_session_name, parse_args, print_help, Args, ListModels, Mode, VERSION};
use commands::dispatch_subcommand;
use event_bus::EventBus;
use interactive::{run_interactive, InteractiveMode};
use pi_agent::context::{load_context_files, render_system_prompt};
use pi_agent::{
    discover_default_skill_dirs, load_skills, AgentMessage, FollowUpQueue, SteerQueue,
    ThinkingLevel, ToolRegistry,
};
use pi_ai::catalog::resolve_model;
use pi_ai::list_models;
use pi_ai::{get_env_api_key, AuthStorage, FileAuthStorage};
use pi_session::{
    append_session_name, continue_latest, create_session, default_sessions_root, fork_session,
    resume_by_id_or_path,
};
use pi_telemetry::{InMemoryTelemetryContext, SpanOptions};
use pi_tui::TuiMode;
use session_runtime::{to_json_event, SessionRuntime};

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
    let telemetry = InMemoryTelemetryContext::new();
    let enable_telemetry = std::env::var("PI_TELEMETRY").is_ok();
    telemetry.start_span(
        SpanOptions {
            name: "pi.startup".into(),
            attributes: None,
        },
        |span| {
            span.add_event("argv", None);
            if enable_telemetry {
                span.add_event("telemetry_enabled", None);
            }
        },
    );

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
        let exported = match parsed.use_theme.as_deref() {
            Some(theme) => export::export_from_file_with_theme(export, output, theme),
            None => export::export_from_file(export, output),
        };
        match exported {
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
    let project_trusted = crate::package_manager::project_is_trusted(
        parsed.project_trust_override,
        crate::package_manager::ProjectTrustMode::Full,
    );
    if parsed.project_trust_override != Some(false)
        && !project_trusted
        && settings::has_trust_requiring_project_resources(&cwd)
    {
        eprintln!(
            "Warning: project {} is not trusted. Use --approve or /trust.",
            cwd.display()
        );
    }

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
            .or_else(|| {
                resume_by_id_or_path(&session_dir, query, None)
                    .ok()
                    .flatten()
            })
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
    let mut skill_paths = if project_trusted {
        discover_default_skill_dirs(&cwd, &commands::agent_dir())
    } else {
        vec![commands::agent_dir().join("skills")]
    };
    skill_paths.extend(parsed.skills.iter().map(PathBuf::from));
    let skills = load_skills(&skill_paths, parsed.no_skills);
    let mut system_prompt = system_prompt;
    if !skills.is_empty() {
        system_prompt.push_str("\n\n# Skills\n");
        for skill in &skills {
            system_prompt.push_str(&format!("## {}\n{}\n", skill.name, skill.body));
        }
    }

    let discovered = extensions::discover_extensions(
        &commands::agent_dir(),
        &cwd,
        &parsed.extensions,
        parsed.no_extensions,
        parsed.project_trust_override,
    );
    let bus = EventBus::new();
    extensions::attach_extensions(&bus, &discovered);
    if parsed.verbose {
        eprintln!("event-bus channels: {}", bus.channel_count());
    }
    bus.emit(
        "session_start",
        serde_json::json!({"cwd": cwd.display().to_string()}),
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

    let messages = collect_messages(&parsed, &cwd)?;
    let thinking = parsed
        .thinking
        .map(|t| ThinkingLevel::parse(t.as_str()).unwrap_or(ThinkingLevel::Off))
        .or(thinking_from_model)
        .unwrap_or(ThinkingLevel::Off);

    let fixture = std::env::var("PI_STREAM_FIXTURE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let offline = parsed.offline || std::env::var("PI_OFFLINE").is_ok();

    let mut runtime = SessionRuntime {
        cwd: cwd.clone(),
        provider: provider.clone(),
        model_id: model_id.clone(),
        system_prompt,
        messages,
        steer: SteerQueue::default(),
        follow_up: FollowUpQueue::default(),
        thinking,
        tools,
        session_path: session.as_ref().map(|s| s.path.clone()),
        session_id: session
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "ephemeral".into()),
        session_name: session.as_ref().and_then(|s| s.name.clone()),
        scoped_models: parsed.models.clone(),
        auto_compact: true,
        auto_retry: true,
        is_streaming: false,
        is_compacting: false,
        aborted: false,
        api_key: resolve_api_key(&parsed, &provider),
        allow_network: fixture.is_none() && !offline,
        fixture,
        bus,
        max_turns: 16,
        context_window: 128_000,
        ui: crate::extension_ui::ExtensionUiHost::default(),
        extensions: discovered.clone(),
        registry: crate::extensions::ExtensionRegistry::default(),
        theme: parsed.use_theme.clone().unwrap_or_else(|| "dark".into()),
        flag_values: Default::default(),
        pending_custom_lines: Vec::new(),
        pending_next_turn: Vec::new(),
        pending_custom_messages: Vec::new(),
        pending_trigger_turn: false,
        running_turn: false,
        last_extension_turn_events: Vec::new(),
    };
    runtime.bind_extensions();
    if let Err(errors) = runtime.apply_cli_flags(&parsed.unknown_flags) {
        for error in errors {
            eprintln!("Error: {error}");
        }
        return Ok(1);
    }

    match app_mode {
        AppMode::Rpc => run_rpc(&mut runtime),
        AppMode::Print | AppMode::Json => {
            run_print(&parsed, &mut runtime, app_mode == AppMode::Json)
        }
        AppMode::Interactive => {
            let fullscreen = parsed.tui_mode == Some(TuiMode::Fullscreen);
            let mode = InteractiveMode::new(
                runtime,
                parsed.tui_mode.unwrap_or(TuiMode::Regular),
                discovered,
            );
            run_interactive(mode, fullscreen)
        }
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

fn split_thinking(spec: &str) -> (&str, Option<ThinkingLevel>) {
    if let Some((model, level)) = spec.rsplit_once(':') {
        if let Some(parsed) = ThinkingLevel::parse(level) {
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

fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn collect_messages(parsed: &Args, cwd: &Path) -> Result<Vec<AgentMessage>, String> {
    let mut parts = parsed.messages.clone();
    let mut images = Vec::new();
    for file in &parsed.file_args {
        let path = if file.starts_with('/') {
            PathBuf::from(file)
        } else {
            cwd.join(file)
        };
        if is_image_path(&path) {
            let bytes = std::fs::read(&path).map_err(|e| format!("Failed to read @{file}: {e}"))?;
            let b64 = {
                const TABLE: &[u8] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                let mut out = String::new();
                for chunk in bytes.chunks(3) {
                    let a = chunk[0] as usize;
                    let b = chunk.get(1).copied().unwrap_or(0) as usize;
                    let c = chunk.get(2).copied().unwrap_or(0) as usize;
                    out.push(TABLE[a >> 2] as char);
                    out.push(TABLE[((a & 3) << 4) | (b >> 4)] as char);
                    if chunk.len() > 1 {
                        out.push(TABLE[((b & 15) << 2) | (c >> 6)] as char);
                    } else {
                        out.push('=');
                    }
                    if chunk.len() > 2 {
                        out.push(TABLE[c & 63] as char);
                    } else {
                        out.push('=');
                    }
                }
                out
            };
            images.push(serde_json::json!({
                "type": "image",
                "path": path.display().to_string(),
                "mimeType": match path.extension().and_then(|e| e.to_str()) {
                    Some("png") => "image/png",
                    Some("gif") => "image/gif",
                    Some("webp") => "image/webp",
                    _ => "image/jpeg",
                },
                "data": b64,
            }));
        } else {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read @{file}: {e}"))?;
            parts.push(format!("# {file}\n{content}"));
        }
    }
    if parts.is_empty() && images.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![AgentMessage {
            role: "user".into(),
            content: parts.join("\n\n"),
            images,
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

fn run_print(parsed: &Args, runtime: &mut SessionRuntime, json: bool) -> Result<i32, String> {
    if runtime.messages.is_empty() {
        eprintln!("Error: print mode requires a prompt");
        return Ok(1);
    }
    let extra = parsed.messages.clone();
    let first = runtime.messages.clone();
    runtime.messages.clear();
    for message in first {
        let events = runtime.prompt(&message.content, message.images)?;
        emit_print_events(&events, json);
    }
    for extra_msg in extra.iter().skip(1) {
        let events = runtime.prompt(extra_msg, vec![])?;
        emit_print_events(&events, json);
    }
    Ok(0)
}

fn emit_print_events(events: &[pi_agent::AgentEvent], json: bool) {
    if json {
        for event in events {
            println!("{}", to_json_event(event));
        }
    } else {
        for event in events {
            if let pi_agent::AgentEvent::Message { message } = event {
                println!("{}", message.content);
            }
        }
    }
}

fn run_rpc(runtime: &mut SessionRuntime) -> Result<i32, String> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("Invalid RPC JSON: {e}"))?;
        let (reply, events) = rpc::handle_rpc(&value, runtime);
        println!("{reply}");
        for event in events {
            println!("{event}");
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_and_version() {
        assert_eq!(run(&["--version".into()], true, true).unwrap(), 0);
        assert_eq!(run(&["--help".into()], true, true).unwrap(), 0);
    }

    #[test]
    fn fork_conflict_is_error() {
        assert_eq!(
            run(
                &[
                    "--fork".into(),
                    "abc".into(),
                    "--session".into(),
                    "abc".into()
                ],
                true,
                true
            )
            .unwrap(),
            1
        );
    }
}
