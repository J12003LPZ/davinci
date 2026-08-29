mod args;
mod auth_cmd;
mod catalog_refresh;
mod changelog;
mod experimental;
mod export;
mod extension_host;
mod extensions;
mod external_editor;
mod image_convert;
mod js_host;
mod llama;
mod migrations;
mod packages;
mod rpc;
mod self_update;
mod settings;
mod slash;
mod startup;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use pi_agent::{
    default_system_prompt, discover_prompt_templates, discover_skills, env_summarizer,
    load_context_files, Agent, AgentEvent, CustomToolExecutor, SummarizeRequest, SummarizeResponse,
    Summarizer,
};
use pi_ai::{
    apply_config_auth_with_shell, apply_models_config, complete_simple, content_text, find_model,
    format_no_models_available_message, fuzzy_models, live_complete_with, load_builtin_models,
    models_json_path, resolve_provider_auth, snapshot_availability, AssistantMessage, AuthStorage,
    ContentBlock, Credential, CredentialKind, ModelConfig, ModelRuntimeSnapshot, ResolvedAuth,
    StopReason, StreamOptions, ToolSpec, NO_MODELS_AVAILABLE,
};
use pi_session::{
    default_agent_dir, discover_sessions, latest_session, now_ms, resolve_session_dir_from,
    resolve_session_ref, JsonlSession, SessionEntry,
};
use pi_tui::{
    builtin_themes, copy_text, detect_terminal_theme, detect_terminal_theme_for_auto,
    drain_osc_tty, encode_kitty, interactive_settings_list, load_themes_from_dir, parse_auto_theme,
    parse_http_idle_timeout, ChatChrome, Component, CustomMessage, DoubleEscapeAction, FilterMode,
    InteractiveSession, Keybindings, MermaidMode, ModelSelectorItem, ScopedModel, SessionAction,
    SessionItem, SessionTreeEntry, SlashCommandSpec, Theme, ThemeDetection, ToolCard, TuiMode,
    FALLBACK_PREVIEW_LINES, OSC_QUERY_TIMEOUT_MS,
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
    apply_http_proxy_settings, default_project_trust_value, is_trusted, load_merged_settings,
    load_settings, save_settings, set_enable_analytics, settings_path, should_run_first_time_setup,
    to_interactive_config,
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
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    apply_http_proxy_settings(
        load_merged_settings(&default_agent_dir(), &cwd)
            .http_proxy
            .as_deref(),
    );
    if experimental::is_experimental_command(raw.first().map(String::as_str)) {
        let command = raw[0].as_str();
        if raw.iter().any(|a| a == "--help" || a == "-h") {
            println!("{}", print_help());
            return Ok(0);
        }
        let message = match command {
            "server" => experimental::run_server(experimental::parse_server_command(&raw[1..])?)?,
            "client" => experimental::run_client(experimental::parse_client_command(&raw[1..])?)?,
            _ => return Err(format!("Unknown command {command}")),
        };
        println!("{message}");
        return Ok(0);
    }

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

    let session_dir = resolved_session_dir(&parsed, &cwd);
    let migrations = migrations::maybe_run_startup_migrations(&cwd);
    let mut agent = build_agent(&parsed, &session_dir, &cwd)?;

    if parsed.mode == Some(Mode::Rpc) {
        return run_rpc(&mut agent);
    }

    let stdin_tty = io::stdin().is_terminal();
    let stdout_tty = io::stdout().is_terminal();
    if parsed.print || parsed.mode == Some(Mode::Json) || !stdin_tty || !stdout_tty {
        return run_print(&parsed, &mut agent);
    }
    if !migrations.deprecation_warnings.is_empty() {
        migrations::show_deprecation_warnings(&migrations.deprecation_warnings);
    }
    run_interactive(&parsed, &mut agent, &migrations.migrated_auth_providers)
}

fn resolved_session_dir(parsed: &Args, cwd: &Path) -> PathBuf {
    let settings = load_merged_settings(&default_agent_dir(), cwd);
    resolve_session_dir_from(
        parsed.session_dir.as_deref(),
        settings.session_dir_normalized().as_deref(),
    )
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
    let settings = load_merged_settings(&default_agent_dir(), cwd);
    apply_http_proxy_settings(settings.http_proxy.as_deref());
    if let Some(level) = parsed.thinking {
        agent.thinking_level = level;
    } else if let Some(level) = settings
        .default_thinking_level
        .as_deref()
        .and_then(pi_protocol::ThinkingLevel::parse)
    {
        agent.thinking_level = level;
    }
    if parsed.no_tools || parsed.no_builtin_tools {
        agent.tools.clear();
    } else if parsed.tools.is_empty() {
        if let Some(tools) = &settings.default_tools {
            agent.tools = tools.clone();
        }
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
        if let Some(extra) = &settings.skills {
            roots.extend(extra.iter().map(PathBuf::from));
        }
        agent.skills = discover_skills(&roots);
    }
    if !parsed.no_prompt_templates {
        let mut roots: Vec<PathBuf> = parsed.prompt_templates.iter().map(PathBuf::from).collect();
        roots.push(cwd.join(".pi").join("prompts"));
        if let Some(extra) = &settings.prompts {
            roots.extend(extra.iter().map(PathBuf::from));
        }
        agent.templates = discover_prompt_templates(&roots);
    }
    agent.context_files = load_context_files(cwd, !parsed.no_context_files);
    if !parsed.no_session {
        agent.session = Some(resolve_or_create_session(parsed, session_dir, cwd)?);
    }
    agent.auto_compaction = settings.compaction_enabled();
    agent.compaction = settings.compaction_settings();
    agent.auto_retry = settings.retry_enabled();
    agent.retry_attempts = settings.retry_max_retries();
    agent.retry_base_delay_ms = settings.retry_base_delay_ms();
    agent.provider_timeout_ms = settings.provider_timeout_ms();
    agent.provider_max_retries = settings.provider_max_retries();
    agent.provider_max_retry_delay_ms = settings.provider_max_retry_delay_ms();
    agent.thinking_budgets = settings.thinking_budgets.clone();
    if let Some(ms) = settings.websocket_connect_timeout_ms {
        std::env::set_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS", ms.to_string());
    }
    let (images, true_color, hyperlinks) = settings.terminal_capability_overrides();
    if let Some(kind) = images {
        std::env::set_var("PI_TERMINAL_IMAGES", kind);
    }
    if let Some(value) = true_color {
        std::env::set_var("PI_TERMINAL_TRUECOLOR", if value { "1" } else { "0" });
    }
    if let Some(value) = hyperlinks {
        std::env::set_var("PI_TERMINAL_HYPERLINKS", if value { "1" } else { "0" });
    }
    if let Some(path) = settings.shell_path.as_deref() {
        std::env::set_var("PI_SHELL", path);
    }
    if let Some(prefix) = settings.shell_command_prefix.as_deref() {
        std::env::set_var("PI_SHELL_COMMAND_PREFIX", prefix);
    }
    let _trusted = is_trusted(&settings, cwd, parsed.project_trust_override);
    let mut extensions = settings.extensions.clone();
    extensions.extend(parsed.extensions.clone());
    for pkg in &settings.packages {
        if !parsed.no_extensions {
            for path in settings::collect_package_resources(pkg, "extensions") {
                extensions.push(path.to_string_lossy().into_owned());
            }
        }
        if !parsed.no_skills {
            agent
                .skills
                .extend(discover_skills(&settings::collect_package_resources(
                    pkg, "skills",
                )));
        }
        if !parsed.no_prompt_templates {
            agent.templates.extend(discover_prompt_templates(
                &settings::collect_package_resources(pkg, "prompts"),
            ));
        }
    }
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
        attach_tool_executor(&mut agent, &host);
        let _ = host.describe_js();
    }
    agent.cwd = cwd.to_path_buf();
    let (provider, model_id) = parse_model_ref(
        parsed.provider.as_deref().unwrap_or("google"),
        parsed.model.as_deref(),
    );
    agent.provider = provider;
    agent.model_id = model_id;
    if let Some(model) = find_model(&available_models(parsed), &agent.provider, &agent.model_id) {
        agent.context_window = model.context_window;
    }
    if let Some(key) = &parsed.api_key {
        if let Ok(mut storage) = AuthStorage::create() {
            storage.set_runtime_override(&agent.provider, key);
            let _ = storage.login_api_key(&agent.provider, key);
        }
    }
    agent.summarizer = Some(live_compaction_summarizer(parsed, &agent));
    Ok(agent)
}

fn live_compaction_summarizer(parsed: &Args, agent: &Agent) -> Summarizer {
    let parsed = parsed.clone();
    let timeout_ms = agent.provider_timeout_ms;
    let max_retries = agent.provider_max_retries;
    let max_retry_delay_ms = Some(agent.provider_max_retry_delay_ms);
    let thinking_level = agent.thinking_level;
    let thinking_budgets = agent.thinking_budgets.clone();
    Summarizer::new(move |request| {
        if let Some(env) = env_summarizer() {
            return env.summarize(request);
        }
        complete_simple_summarization(
            &parsed,
            request,
            timeout_ms,
            max_retries,
            max_retry_delay_ms,
            thinking_level,
            thinking_budgets.clone(),
        )
    })
}

fn complete_simple_summarization(
    parsed: &Args,
    request: &SummarizeRequest,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
    max_retry_delay_ms: Option<u64>,
    thinking_level: pi_protocol::ThinkingLevel,
    thinking_budgets: Option<pi_ai::ThinkingBudgets>,
) -> Result<SummarizeResponse, String> {
    let offline = parsed.offline
        || matches!(
            std::env::var("PI_OFFLINE").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    if offline {
        return Err("Summarization failed: offline".into());
    }
    let models = available_models(parsed);
    let model = find_model(&models, &request.provider, &request.model_id)
        .cloned()
        .or_else(|| {
            models
                .iter()
                .find(|item| item.provider == request.provider)
                .cloned()
        })
        .or_else(|| models.first().cloned())
        .ok_or_else(|| "Summarization failed: no model available".to_string())?;
    let mut storage = AuthStorage::create().ok();
    if let (Some(storage), Some(key)) = (storage.as_mut(), parsed.api_key.as_deref()) {
        storage.set_runtime_override(&request.provider, key);
    }
    if let Some(storage) = storage.as_mut() {
        let _ = storage.maybe_refresh(&request.provider, now_ms(), 0, false);
    }
    let env = std::env::vars().collect();
    let mut auth = storage
        .as_ref()
        .and_then(|storage| resolve_provider_auth(&request.provider, storage, &env, true))
        .unwrap_or(ResolvedAuth {
            api_key: None,
            headers: Default::default(),
            source: "none".into(),
        });
    let config = ModelConfig::load(&models_json_path(&default_agent_dir()));
    let shell_path = load_settings(&default_agent_dir()).shell_path;
    apply_config_auth_with_shell(
        &mut auth,
        &config,
        &request.provider,
        Some(&model),
        &env,
        shell_path.as_deref(),
    );
    if auth.api_key.is_none() && auth.headers.is_empty() && auth.source == "none" {
        return Err("Summarization failed: no credentials".into());
    }
    let options = StreamOptions {
        thinking_level: if model.reasoning && thinking_level != pi_protocol::ThinkingLevel::Off {
            Some(thinking_level)
        } else {
            None
        },
        thinking_budgets,
        timeout_ms,
        max_retries,
        max_retry_delay_ms,
        max_tokens: Some(request.max_tokens),
    };
    let response = complete_simple(
        &model,
        &request.prompt,
        Some(&request.system),
        &auth,
        &options,
    )?;
    Ok(SummarizeResponse {
        text: response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        usage: response.usage.unwrap_or_default(),
        stop_reason: response.stop_reason,
        error_message: response.error_message,
        has_tool_call: response
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. })),
    })
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

fn available_models(parsed: &Args) -> Vec<pi_ai::Model> {
    load_model_runtime(parsed).available
}

fn load_available_models(parsed: &Args) -> (Vec<pi_ai::Model>, Option<String>) {
    let snapshot = load_model_runtime(parsed);
    if snapshot.all.is_empty() {
        return (
            Vec::new(),
            Some(
                snapshot
                    .get_error()
                    .unwrap_or_else(|| NO_MODELS_AVAILABLE.into()),
            ),
        );
    }
    let error = snapshot.get_error();
    (snapshot.available, error)
}

fn load_model_runtime(parsed: &Args) -> ModelRuntimeSnapshot {
    let mut models = load_builtin_models();
    let agent_dir = default_agent_dir();
    let store = pi_ai::load_models_store(&agent_dir);
    for entry in store.providers.values() {
        models = pi_ai::merge_models(&models, &entry.models);
    }
    let config = ModelConfig::load(&models_json_path(&agent_dir));
    let mut composition_errors = std::collections::BTreeMap::new();
    models = match apply_models_config(&models, &config) {
        Ok(applied) => applied,
        Err(err) => {
            let provider = err
                .strip_prefix("Provider ")
                .and_then(|rest| rest.split(':').next())
                .unwrap_or("models.json")
                .trim()
                .to_string();
            composition_errors.insert(provider, err);
            models
        }
    };
    for provider in loaded_extension_host(parsed).registered_providers() {
        models.extend(pi_ai::models_from_provider_config(
            &provider.name,
            &provider.config,
        ));
        match apply_models_config(&models, &config) {
            Ok(applied) => models = applied,
            Err(err) => {
                composition_errors.insert(provider.name.clone(), err);
            }
        }
    }
    let auth_path = agent_dir.join("auth.json");
    let mut storage = AuthStorage::open(&auth_path).unwrap_or_else(|_| AuthStorage::in_memory());
    if let Some(key) = parsed.api_key.as_deref() {
        let provider = parsed
            .provider
            .clone()
            .or_else(|| {
                parsed
                    .model
                    .as_deref()
                    .and_then(|value| value.split('/').next().map(str::to_string))
            })
            .unwrap_or_default();
        if !provider.is_empty() {
            storage.set_runtime_override(&provider, key);
        }
    }
    let env = std::env::vars().collect();
    snapshot_availability(models, &config, &storage, &env, composition_errors, None)
}

fn coding_agent_docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pi/packages/coding-agent/docs")
}

fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let millions = count as f64 / 1_000_000.0;
        if millions.fract() == 0.0 {
            format!("{}M", millions as u64)
        } else {
            format!("{millions:.1}M")
        }
    } else if count >= 1_000 {
        let thousands = count as f64 / 1_000.0;
        if thousands.fract() == 0.0 {
            format!("{}K", thousands as u64)
        } else {
            format!("{thousands:.1}K")
        }
    } else {
        count.to_string()
    }
}

fn render_models_table(models: &[&pi_ai::Model]) -> String {
    let mut rows: Vec<(String, String, String, String, String, String)> = models
        .iter()
        .map(|model| {
            (
                model.provider.clone(),
                model.id.clone(),
                format_token_count(model.context_window),
                format_token_count(model.max_tokens),
                if model.reasoning { "yes" } else { "no" }.into(),
                if model.input.iter().any(|item| item == "image") {
                    "yes"
                } else {
                    "no"
                }
                .into(),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let headers = (
        "provider".to_string(),
        "model".to_string(),
        "context".to_string(),
        "max-out".to_string(),
        "thinking".to_string(),
        "images".to_string(),
    );
    let widths = [
        rows.iter()
            .map(|row| row.0.len())
            .max()
            .unwrap_or(0)
            .max(headers.0.len()),
        rows.iter()
            .map(|row| row.1.len())
            .max()
            .unwrap_or(0)
            .max(headers.1.len()),
        rows.iter()
            .map(|row| row.2.len())
            .max()
            .unwrap_or(0)
            .max(headers.2.len()),
        rows.iter()
            .map(|row| row.3.len())
            .max()
            .unwrap_or(0)
            .max(headers.3.len()),
        rows.iter()
            .map(|row| row.4.len())
            .max()
            .unwrap_or(0)
            .max(headers.4.len()),
        rows.iter()
            .map(|row| row.5.len())
            .max()
            .unwrap_or(0)
            .max(headers.5.len()),
    ];
    let mut lines = vec![format!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {:<w5$}",
        headers.0,
        headers.1,
        headers.2,
        headers.3,
        headers.4,
        headers.5,
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4],
        w5 = widths[5],
    )];
    for row in rows {
        lines.push(format!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {:<w5$}",
            row.0,
            row.1,
            row.2,
            row.3,
            row.4,
            row.5,
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
            w4 = widths[4],
            w5 = widths[5],
        ));
    }
    lines.join("\n")
}

fn list_models(list: &ListModels) -> Result<i32, String> {
    let snapshot = load_model_runtime(&Args::default());
    if let Some(error) = snapshot.get_error() {
        if snapshot.all.is_empty() {
            eprintln!("{error}");
            return Ok(1);
        }
        eprintln!("Warning: errors loading models.json:\n{error}");
    }
    if snapshot.available.is_empty() {
        if snapshot.all.is_empty() {
            eprintln!("{NO_MODELS_AVAILABLE}");
            return Ok(1);
        }
        println!(
            "{}",
            format_no_models_available_message(&coding_agent_docs_dir())
        );
        return Ok(0);
    }
    let selected = match list {
        ListModels::All => snapshot.available.iter().collect(),
        ListModels::Query(query) => fuzzy_models(&snapshot.available, query),
    };
    if selected.is_empty() {
        if let ListModels::Query(query) = list {
            println!("No models matching \"{query}\"");
            return Ok(0);
        }
    }
    println!("{}", render_models_table(&selected));
    Ok(0)
}

fn export_session(parsed: &Args, export: &str) -> Result<i32, String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let session_dir = resolved_session_dir(parsed, &cwd);
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
    println!("{}", export::export_session(&session, &output)?);
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
    let mut resolved = resolve_provider_auth(&provider, &storage, &env, true);
    if let Some(auth) = resolved.as_mut() {
        let config = ModelConfig::load(&models_json_path(&default_agent_dir()));
        let shell_path = load_settings(&default_agent_dir()).shell_path;
        apply_config_auth_with_shell(auth, &config, &provider, None, &env, shell_path.as_deref());
    }
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
    let models = available_models(parsed);
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
    let mut auth = storage
        .as_ref()
        .and_then(|storage| resolve_provider_auth(&agent.provider, storage, &env, true))
        .or_else(|| {
            Some(ResolvedAuth {
                api_key: None,
                headers: Default::default(),
                source: "none".into(),
            })
        });
    if let Some(auth) = auth.as_mut() {
        let config = ModelConfig::load(&models_json_path(&default_agent_dir()));
        let model = find_model(&models, &agent.provider, &agent.model_id);
        let shell_path = load_settings(&default_agent_dir()).shell_path;
        apply_config_auth_with_shell(
            auth,
            &config,
            &agent.provider,
            model,
            &env,
            shell_path.as_deref(),
        );
    }
    if auth.as_ref().is_some_and(|item| {
        item.api_key.is_none() && item.headers.is_empty() && item.source == "none"
    }) {
        auth = None;
    }
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
                (false, Some(model), Some(auth)) => live_complete_with(
                    model,
                    &current.messages,
                    auth,
                    Some(&current.system_prompt),
                    &tools,
                    &StreamOptions {
                        thinking_level: Some(current.thinking_level),
                        thinking_budgets: current.thinking_budgets.clone(),
                        timeout_ms: current.provider_timeout_ms,
                        max_retries: current.provider_max_retries,
                        max_retry_delay_ms: Some(current.provider_max_retry_delay_ms),
                        max_tokens: None,
                    },
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
    let host = loaded_extension_host(&Args::default());
    for provider in host.registered_providers() {
        runtime.models.extend(pi_ai::models_from_provider_config(
            &provider.name,
            &provider.config,
        ));
    }
    runtime.invocable_commands = slash::invocable_commands(
        &host
            .js
            .iter()
            .flat_map(|ext| {
                ext.commands
                    .iter()
                    .map(|name| (name.clone(), String::new(), ext.path.clone()))
            })
            .collect::<Vec<_>>(),
        &runtime.agent.templates,
        &runtime.agent.skills,
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
        for request in rpc::extension_ui_requests_from_calls(&host.ui_calls) {
            println!(
                "{}",
                serde_json::to_string(&request).map_err(|err| err.to_string())?
            );
        }
        if is_prompt {
            let parsed = Args {
                offline: matches!(
                    std::env::var("PI_OFFLINE").as_deref(),
                    Ok("1") | Ok("true") | Ok("yes")
                ),
                ..Args::default()
            };
            let (_reply, events) = complete_prompt(&parsed, &mut runtime.agent);
            for request in rpc::extension_ui_requests_from_calls(&host.ui_calls) {
                println!(
                    "{}",
                    serde_json::to_string(&request).map_err(|err| err.to_string())?
                );
            }
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

fn run_interactive(
    parsed: &Args,
    agent: &mut Agent,
    migrated_auth_providers: &[String],
) -> Result<i32, String> {
    let fullscreen = parsed.tui_mode == Some(TuiMode::Fullscreen);
    let theme = builtin_themes()
        .into_iter()
        .find(|theme| parsed.use_theme.as_deref() == Some(theme.name.as_str()))
        .or_else(|| builtin_themes().into_iter().next())
        .expect("theme");
    let (runtime_models, models_json_error) = load_available_models(parsed);
    let models: Vec<String> = runtime_models
        .iter()
        .map(|model| format!("{}/{}", model.provider, model.id))
        .collect();
    let mut session = InteractiveSession::new(theme, format!("{APP_NAME} {VERSION}"), models);
    session.model_items = runtime_models
        .iter()
        .map(|model| ModelSelectorItem {
            provider: model.provider.clone(),
            id: model.id.clone(),
            name: model.name.clone(),
        })
        .collect();
    if let Some(index) = session
        .models
        .iter()
        .position(|item| item == &format!("{}/{}", agent.provider, agent.model_id))
    {
        session.model_index = index;
    }
    session.cwd = agent.cwd.clone();
    session.slash_commands = interactive_slash_commands(agent, parsed);
    session.login_providers = pi_ai::oauth_providers()
        .iter()
        .map(|name| (*name).to_string())
        .chain(std::iter::once(llama::LLAMA_PROVIDER_ID.to_string()))
        .collect();
    let stored = load_merged_settings(&default_agent_dir(), &agent.cwd);
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
    session.default_model = match (&stored.default_provider, &stored.default_model) {
        (Some(provider), Some(id)) => Some(format!("{provider}/{id}")),
        (None, Some(id)) if id.contains('/') => Some(id.clone()),
        _ => None,
    };
    session.keybindings = Keybindings::load(&default_agent_dir());
    session.warnings_anthropic_extra_usage = stored.warnings.anthropic_extra_usage.unwrap_or(true);
    session.branch_summary_skip_prompt = stored.branch_summary_skip_prompt();
    session.branch_summary_reserve_tokens = stored.branch_summary_reserve_tokens();
    let _indent = stored.code_block_indent();
    if let Some(levels) = stored.model_thinking_levels.clone() {
        session.model_thinking_levels = levels;
    }
    let _ = session.begin_osc_query(OSC_QUERY_TIMEOUT_MS);
    let host = loaded_extension_host(parsed);
    apply_extension_shortcuts(&mut session, &host);
    session.slash_commands = interactive_slash_commands(agent, parsed);
    replay_custom_messages(agent, &mut session, &host);
    let _ = FALLBACK_PREVIEW_LINES;
    refresh_interactive_models(parsed, &mut session);
    apply_startup_notices(
        &mut session,
        &stored,
        models_json_error,
        migrated_auth_providers,
    );
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
        if session.osc_query_pending() {
            if let Some(reply) = drain_osc_tty(OSC_QUERY_TIMEOUT_MS) {
                let action = session.handle_bytes(&reply);
                if !apply_session_action(parsed, agent, session, action)? {
                    break;
                }
            }
        }
        if let Some(detection) = session.finish_osc_query(std::time::Instant::now()) {
            apply_osc_theme(session, &detection);
        }
        if !crossterm::event::poll(std::time::Duration::from_millis(50))
            .map_err(|err| err.to_string())?
        {
            let mut dirty = poll_llama_job(session);
            dirty |= tick_custom_overlay(parsed, session);
            if dirty {
                print!("\x1b[H{}", session.render_frame());
                io::stdout().flush().ok();
            }
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
        SessionAction::None | SessionAction::CloseOverlay => Ok(true),
        SessionAction::OpenModel => {
            refresh_interactive_models(parsed, session);
            Ok(true)
        }
        SessionAction::ExtensionProgressCancel => {
            handle_llama_progress_cancel(session);
            Ok(true)
        }
        SessionAction::SelectSession(id) => {
            let session_dir = resolved_session_dir(parsed, &agent.cwd);
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
        SessionAction::ExtensionShortcut { key, path } => {
            match host_invoke_shortcut(parsed, &path, &key) {
                Ok(Some(value)) => {
                    session.chrome.status = format!("shortcut={key} {value}");
                }
                Ok(None) => {
                    session.chrome.status = format!("shortcut={key}");
                }
                Err(err) => {
                    session.chrome.status = format!("Shortcut handler error: {err}");
                }
            }
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
        SessionAction::ExtensionSelect(choice) => {
            handle_extension_select(parsed, agent, session, choice)
        }
        SessionAction::ExtensionInput(value) => {
            handle_extension_input(session, value);
            Ok(true)
        }
        SessionAction::ExtensionEditor(value) => {
            handle_branch_summary_editor(agent, session, value);
            Ok(true)
        }
        SessionAction::ExtensionConfirm(value) => {
            handle_extension_confirm(session, value);
            Ok(true)
        }
        SessionAction::CustomEditorInput(data) => {
            handle_custom_editor_input(parsed, agent, session, &data)
        }
        SessionAction::CustomOverlayInput(data) => {
            handle_custom_overlay_input(parsed, session, &data);
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
        SessionAction::SelectTreeEntry(id) => select_tree_entry(agent, session, id),
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
        SessionAction::CycleModel | SessionAction::CycleModelBackward => {
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
        SessionAction::ToggleHideThinking => {
            apply_interactive_setting(
                session,
                &format!(
                    "hide-thinking={}",
                    session.chrome.transcript.hide_thinking_block
                ),
            )?;
            Ok(true)
        }
        SessionAction::ExpandTools => Ok(true),
        SessionAction::NewSession => handle_user_line(parsed, agent, &mut session.chrome, "/new"),
        SessionAction::OpenResume => {
            open_session_selector(parsed, agent, session)?;
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
        SessionAction::SelectModelAsDefault(value) => {
            let (provider, model_id) = parse_model_ref("google", Some(&value));
            agent.provider = provider;
            agent.model_id = model_id;
            session.default_model = Some(format!("{}/{}", agent.provider, agent.model_id));
            let dir = default_agent_dir();
            let mut stored = load_settings(&dir);
            stored.default_provider = Some(agent.provider.clone());
            stored.default_model = Some(agent.model_id.clone());
            save_settings(&dir, &stored)?;
            session.chrome.status = format!("default={}/{}", agent.provider, agent.model_id);
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
                reload_interactive_resources(parsed, agent, session);
                handle_user_line(parsed, agent, &mut session.chrome, &text)
            }
            SlashAction::Llama => {
                open_llama_ui(session)?;
                Ok(true)
            }
            SlashAction::Hotkeys => {
                let mut keys = pi_tui::get_keybindings()
                    .into_iter()
                    .map(|b| format!("{}: {}", b.action, b.keys.join(", ")))
                    .collect::<Vec<_>>();
                keys.extend(
                    session
                        .extension_shortcuts
                        .iter()
                        .map(|(key, path)| format!("extension.{key}: {key} ({path})")),
                );
                let keys = keys.join("\n");
                session.chrome.status = keys.clone();
                println!("{keys}");
                Ok(true)
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
            if let Some(name) = message.strip_prefix("Unknown command /") {
                if try_extension_slash(parsed, chrome, name)? {
                    return Ok(true);
                }
            }
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
            let models = load_builtin_models();
            let model = find_model(&models, &agent.provider, &agent.model_id);
            let stats = rpc::session_stats_for_agent(agent, model);
            let info = format_session_info(&stats);
            chrome.status = info.clone();
            println!("{info}");
            Ok(true)
        }
        SlashAction::NewSession => {
            let session_dir = resolved_session_dir(parsed, &agent.cwd);
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
                println!("{}", export::export_session(session, &output)?);
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
                let session_dir = resolved_session_dir(parsed, &agent.cwd);
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
                let session_dir = resolved_session_dir(parsed, &agent.cwd);
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
            let _host = loaded_extension_host(parsed);
            chrome.status =
                "Reloaded keybindings, extensions, skills, prompts, themes, and context files"
                    .into();
            println!("{}", chrome.status);
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
            let stored = load_merged_settings(&default_agent_dir(), &agent.cwd);
            let text = match stored.last_changelog_version.as_deref() {
                Some(since) => changelog::format_changelog_since(&entries, Some(since)),
                None => changelog::format_changelog(&entries),
            };
            chrome.transcript.push("changelog", &text);
            chrome.status = "changelog".into();
            println!("{text}");
            Ok(true)
        }
        SlashAction::Llama => {
            chrome.status = "llama.cpp is available in interactive mode".into();
            println!("{}", chrome.status);
            Ok(true)
        }
    }
}

const DEFAULT_RADIUS_GATEWAY: &str = "https://radius.pi.dev";

fn reload_interactive_resources(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
) {
    session.keybindings = Keybindings::load(&default_agent_dir());
    agent.skills = discover_skills(&[agent.cwd.join(".pi").join("skills")]);
    agent.templates = discover_prompt_templates(&[agent.cwd.join(".pi").join("prompts")]);
    agent.context_files = load_context_files(&agent.cwd, true);
    let host = loaded_extension_host(parsed);
    apply_extension_shortcuts(session, &host);
    session.slash_commands = interactive_slash_commands(agent, parsed);
    replay_custom_messages(agent, session, &host);
    let current = session.chrome.theme.name.clone();
    apply_theme_value(session, &current);
}

fn available_themes() -> Vec<Theme> {
    let mut themes = load_themes_from_dir(&default_agent_dir().join("themes"));
    let settings = load_settings(&default_agent_dir());
    if let Some(paths) = &settings.themes {
        for path in paths {
            themes.extend(load_themes_from_dir(Path::new(path)));
        }
    }
    for pkg in &settings.packages {
        for path in settings::collect_package_resources(pkg, "themes") {
            if path.is_dir() {
                themes.extend(load_themes_from_dir(&path));
            } else if let Some(parent) = path.parent() {
                themes.extend(load_themes_from_dir(parent));
            }
        }
    }
    themes
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
    if let Some(result) = try_share_via_radius(agent) {
        return result;
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

fn radius_share_token() -> Option<String> {
    if let Ok(token) = std::env::var("PI_RADIUS_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }
    let storage = AuthStorage::create().ok()?;
    let cred = storage.get("radius")?;
    cred.access
        .clone()
        .or_else(|| cred.key.clone())
        .filter(|token| !token.is_empty())
}

fn try_share_via_radius(agent: &Agent) -> Option<Result<String, String>> {
    let token = radius_share_token()?;
    if let Ok(url) = std::env::var("PI_RADIUS_ARTIFACT_URL") {
        if !url.is_empty() {
            return Some(Ok(format!("Share URL: {url}")));
        }
    }
    if let Ok(reply) = std::env::var("PI_RADIUS_ARTIFACT_REPLY") {
        return Some(parse_radius_artifact_reply(&reply));
    }
    Some(upload_radius_artifact(agent, &token))
}

fn parse_radius_artifact_reply(reply: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(reply)
        .map_err(|err| format!("Failed to upload Radius artifact: {err}"))?;
    if let Some(url) = json
        .pointer("/artifact/canonical_url")
        .and_then(|value| value.as_str())
    {
        return Ok(format!("Share URL: {url}"));
    }
    let error = json
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown error");
    Err(format!("Failed to upload Radius artifact: {error}"))
}

fn upload_radius_artifact(agent: &Agent, token: &str) -> Result<String, String> {
    let session = agent
        .session
        .as_ref()
        .ok_or_else(|| "No session to share".to_string())?;
    let body = std::fs::read(&session.path).map_err(|err| err.to_string())?;
    let url =
        format!("{DEFAULT_RADIUS_GATEWAY}/v1/artifacts?visibility=organization&title=Pi%20session");
    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/x-ndjson")
        .send_bytes(&body)
        .map_err(|err| format!("Failed to upload Radius artifact: {err}"))?;
    let text = response
        .into_string()
        .map_err(|err| format!("Failed to upload Radius artifact: {err}"))?;
    parse_radius_artifact_reply(&text)
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
            AgentEvent::AutoRetryStart {
                attempt,
                max_attempts,
                delay_ms,
                error_message,
            } => {
                chrome.status =
                    format!("Retrying ({attempt}/{max_attempts}) in {delay_ms}ms: {error_message}");
            }
            AgentEvent::AutoRetryEnd {
                success,
                attempt,
                final_error,
            } => {
                if *success {
                    chrome.status.clear();
                } else {
                    chrome.status = format!(
                        "Retry failed after {attempt} attempts: {}",
                        final_error.as_deref().unwrap_or("Unknown error")
                    );
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
    if provider == llama::LLAMA_PROVIDER_ID {
        let mut env = std::collections::HashMap::new();
        let (api_key, url) = match key {
            Some(value) if value.starts_with("http://") || value.starts_with("https://") => {
                (None, value.to_string())
            }
            Some(value) => (
                Some(value.to_string()),
                std::env::var("LLAMA_BASE_URL")
                    .unwrap_or_else(|_| llama::DEFAULT_LLAMA_SERVER_URL.into()),
            ),
            None => (
                None,
                std::env::var("LLAMA_BASE_URL")
                    .unwrap_or_else(|_| llama::DEFAULT_LLAMA_SERVER_URL.into()),
            ),
        };
        let url = llama::normalize_llama_server_url(&url)?;
        env.insert("LLAMA_BASE_URL".into(), url.clone());
        storage
            .set(
                provider,
                Credential {
                    kind: CredentialKind::ApiKey,
                    key: api_key,
                    access: None,
                    refresh: None,
                    expires: None,
                    env,
                    available_model_ids: Vec::new(),
                },
            )
            .map_err(|err| err.to_string())?;
        println!("stored llama.cpp server {url}");
        return Ok(());
    }
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

fn interactive_slash_commands(agent: &Agent, parsed: &Args) -> Vec<SlashCommandSpec> {
    let host = loaded_extension_host(parsed);
    let mut commands: Vec<SlashCommandSpec> = slash::builtin_slash_commands()
        .into_iter()
        .map(|command| SlashCommandSpec {
            name: command.name,
            description: command.description,
            argument_hint: command.argument_hint,
        })
        .collect();
    for spec in slash::invocable_commands(
        &host
            .js
            .iter()
            .flat_map(|ext| {
                ext.commands
                    .iter()
                    .map(|name| (name.clone(), String::new(), ext.path.clone()))
            })
            .collect::<Vec<_>>(),
        &agent.templates,
        &agent.skills,
    ) {
        let Some(name) = spec.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        if commands.iter().any(|command| command.name == name) {
            continue;
        }
        commands.push(SlashCommandSpec {
            name: name.to_string(),
            description: spec
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            argument_hint: None,
        });
    }
    commands
}

fn format_session_info(stats: &serde_json::Value) -> String {
    format!(
        "Session Info\n\nFile: {}\nID: {}\n\nMessages\nTotal: {}\nUser: {}\nAssistant: {}\nTools: {} calls, {} results\n\nTokens\nInput: {}\nOutput: {}\nCache read: {}\nCache write: {}\nTotal: {}\nCost: {}",
        stats.get("sessionFile").and_then(|value| value.as_str()).unwrap_or("In-memory"),
        stats.get("sessionId").and_then(|value| value.as_str()).unwrap_or(""),
        stats.get("totalMessages").and_then(|value| value.as_u64()).unwrap_or(0),
        stats.get("userMessages").and_then(|value| value.as_u64()).unwrap_or(0),
        stats.get("assistantMessages").and_then(|value| value.as_u64()).unwrap_or(0),
        stats.get("toolCalls").and_then(|value| value.as_u64()).unwrap_or(0),
        stats.get("toolResults").and_then(|value| value.as_u64()).unwrap_or(0),
        stats["tokens"]["input"].as_u64().unwrap_or(0),
        stats["tokens"]["output"].as_u64().unwrap_or(0),
        stats["tokens"]["cacheRead"].as_u64().unwrap_or(0),
        stats["tokens"]["cacheWrite"].as_u64().unwrap_or(0),
        stats["tokens"]["total"].as_u64().unwrap_or(0),
        stats.get("cost").and_then(|value| value.as_f64()).unwrap_or(0.0),
    )
}

fn apply_extension_shortcuts(session: &mut InteractiveSession, host: &ExtensionHost) {
    let (shortcuts, diagnostics) = host.resolve_shortcuts(&session.keybindings);
    session.extension_shortcuts = shortcuts;
    session.apply_extension_ui_calls(&host.ui_calls);
    activate_custom_editor(session, host);
    if let Some(warning) = diagnostics.first() {
        session.chrome.status = warning.clone();
    }
}

fn activate_custom_editor(session: &mut InteractiveSession, host: &ExtensionHost) {
    for path in host
        .editor_modules
        .iter()
        .chain(host.js.iter().map(|ext| &ext.path))
    {
        if let Ok(result) = host.editor_input(
            path,
            "",
            session.custom_editor_snapshot.as_ref(),
            session.width,
        ) {
            if result.get("enabled").and_then(|value| value.as_bool()) != Some(true) {
                continue;
            }
            session.custom_editor_path = Some(path.clone());
            if let Some(snapshot) = result.get("snapshot").cloned() {
                session.custom_editor_snapshot = Some(snapshot);
            }
            if let Some(lines) = result.get("lines").and_then(|value| value.as_array()) {
                session.chrome.custom_editor_lines = Some(
                    lines
                        .iter()
                        .filter_map(|line| line.as_str().map(str::to_string))
                        .collect(),
                );
            }
            return;
        }
    }
}

fn apply_editor_host_result(session: &mut InteractiveSession, result: &serde_json::Value) {
    if result.get("enabled").and_then(|value| value.as_bool()) == Some(false) {
        session.custom_editor_path = None;
        session.custom_editor_snapshot = None;
        session.chrome.custom_editor_lines = None;
        return;
    }
    if let Some(snapshot) = result.get("snapshot").cloned() {
        session.custom_editor_snapshot = Some(snapshot);
    }
    if let Some(lines) = result.get("lines").and_then(|value| value.as_array()) {
        session.chrome.custom_editor_lines = Some(
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect(),
        );
    }
}

fn handle_custom_editor_input(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    data: &str,
) -> Result<bool, String> {
    let Some(path) = session.custom_editor_path.clone() else {
        return Ok(true);
    };
    let host = loaded_extension_host(parsed);
    match host.editor_input(
        &path,
        data,
        session.custom_editor_snapshot.as_ref(),
        session.width,
    ) {
        Ok(result) => {
            apply_editor_host_result(session, &result);
            match result.get("action").and_then(|value| value.as_str()) {
                Some("submit") => {
                    let text = result
                        .get("text")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        return apply_session_action(
                            parsed,
                            agent,
                            session,
                            SessionAction::Submit(text),
                        );
                    }
                }
                Some("abort") => {
                    session.chrome.status = "aborted".into();
                }
                Some("quit") => return Ok(false),
                _ => {}
            }
        }
        Err(err) => {
            session.chrome.status = format!("Editor host error: {err}");
        }
    }
    Ok(true)
}

fn host_invoke_shortcut(
    parsed: &Args,
    path: &str,
    key: &str,
) -> Result<Option<serde_json::Value>, String> {
    let mut host = loaded_extension_host(parsed);
    if host.js.iter().any(|ext| ext.path == path) {
        return host.invoke_shortcut(path, key);
    }
    ExtensionHost::default().invoke_shortcut(path, key)
}

fn try_extension_slash(parsed: &Args, chrome: &mut ChatChrome, name: &str) -> Result<bool, String> {
    let mut host = loaded_extension_host(parsed);
    let path = host
        .js
        .iter()
        .find(|ext| ext.commands.iter().any(|command| command == name))
        .map(|ext| ext.path.clone());
    let Some(path) = path else {
        return Ok(false);
    };
    let result = host.invoke_command(&path, name)?;
    for call in &host.ui_calls {
        chrome.apply_ui_call(call);
    }
    apply_custom_overlay_result(chrome, &path, name, result.as_ref());
    chrome.status = format!("/{name}");
    println!("{}", chrome.status);
    Ok(true)
}

fn apply_custom_overlay_result(
    chrome: &mut ChatChrome,
    path: &str,
    name: &str,
    result: Option<&serde_json::Value>,
) {
    let Some(result) = result else {
        return;
    };
    if result.get("pending").and_then(|value| value.as_bool()) != Some(true) {
        chrome.custom_overlay_path = None;
        chrome.custom_overlay_command = None;
        chrome.custom_overlay_snapshot = None;
        chrome.custom_overlay_lines = None;
        chrome.custom_overlay_composite = false;
        chrome.custom_overlay_options = None;
        return;
    }
    chrome.custom_overlay_path = Some(path.to_string());
    chrome.custom_overlay_command = Some(name.to_string());
    chrome.custom_overlay_snapshot = result.get("snapshot").cloned();
    chrome.custom_overlay_lines =
        result
            .get("lines")
            .and_then(|value| value.as_array())
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| line.as_str().map(str::to_string))
                    .collect()
            });
    chrome.custom_overlay_composite = result
        .get("overlay")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    chrome.custom_overlay_options = result
        .get("overlayOptions")
        .map(pi_tui::overlay_options_from_json);
}

fn handle_custom_overlay_input(parsed: &Args, session: &mut InteractiveSession, data: &str) {
    let Some(path) = session.chrome.custom_overlay_path.clone() else {
        return;
    };
    let name = session
        .chrome
        .custom_overlay_command
        .clone()
        .unwrap_or_default();
    let snapshot = session.chrome.custom_overlay_snapshot.clone();
    let mut host = loaded_extension_host(parsed);
    match host.invoke_command_with(&path, &name, data, snapshot.as_ref(), session.width) {
        Ok(result) => {
            apply_custom_overlay_result(&mut session.chrome, &path, &name, result.as_ref());
            if session.chrome.custom_overlay_lines.is_none() {
                if let Some(value) = result {
                    session.chrome.status = format!("custom={}", value);
                }
            }
        }
        Err(err) => {
            session.close_overlays();
            session.chrome.status = format!("Custom UI error: {err}");
        }
    }
}

fn attach_tool_executor(agent: &mut Agent, host: &ExtensionHost) {
    let host = host.clone();
    agent.custom_tool_executor = Some(CustomToolExecutor::new(move |cwd, name, args| {
        host.execute_js_or_manifest_tool(cwd, name, args)
    }));
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
    if let Some(theme) = available_themes()
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
        let (display, status) =
            match crate::image_convert::resize_image_in_process(&png, "image/png") {
                Some(resized) => {
                    let status = format!(
                        "pasted image {}x{} (from {}x{}, {})",
                        resized.width,
                        resized.height,
                        resized.original_width,
                        resized.original_height,
                        resized.mime_type
                    );
                    let display = if resized.was_resized && resized.mime_type == "image/png" {
                        resized.bytes
                    } else {
                        png
                    };
                    (display, status)
                }
                None => (png, "pasted image".into()),
            };
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, display);
        session.place_kitty_image(&b64, Some(1));
        session.chrome.status = status;
    } else if let Some(text) = clipboard_text() {
        session.chrome.editor.handle_input(&text);
        session.chrome.status = "pasted clipboard".into();
    } else {
        session.chrome.status = "clipboard empty".into();
    }
}

fn apply_startup_notices(
    session: &mut InteractiveSession,
    settings: &settings::Settings,
    models_json_error: Option<String>,
    migrated_auth_providers: &[String],
) {
    let notices = startup::collect_startup_notices(
        VERSION,
        settings,
        models_json_error,
        migrated_auth_providers.to_vec(),
    );
    for (kind, line) in startup::format_notices(&notices) {
        session.chrome.transcript.push(&kind, &line);
    }
}

fn refresh_interactive_models(parsed: &Args, session: &mut InteractiveSession) {
    session.chrome.status = catalog_refresh::refresh_status_refreshing().into();
    let agent_dir = default_agent_dir();
    let allow_network = std::env::var("PI_OFFLINE").is_err();
    let refreshed = catalog_refresh::refresh_model_catalogs(&agent_dir, allow_network, false);
    let snapshot = load_model_runtime(parsed);
    let mut models: Vec<String> = snapshot
        .available
        .iter()
        .map(|model| format!("{}/{}", model.provider, model.id))
        .collect();
    for model in &refreshed.models {
        let label = format!("{}/{}", model.provider, model.id);
        if snapshot
            .configured_providers
            .iter()
            .any(|provider| provider == &model.provider)
            && !models.contains(&label)
        {
            models.push(label);
        }
    }
    session.models = models;
    session.model_items = snapshot
        .available
        .iter()
        .map(|model| ModelSelectorItem {
            provider: model.provider.clone(),
            id: model.id.clone(),
            name: model.name.clone(),
        })
        .collect();
    if session.model_items.is_empty() {
        session.model_items = session
            .models
            .iter()
            .map(|key| ModelSelectorItem::from_key(key))
            .collect();
    }
    if session.chrome.selector.is_some() {
        session.chrome.selector = Some(pi_tui::SelectList::new(session.models.clone()));
    }
    if session.chrome.model_selector.is_some() {
        let scoped = session
            .enabled_model_ids
            .as_ref()
            .map(|ids| {
                session
                    .model_items
                    .iter()
                    .filter(|item| ids.iter().any(|id| id == &item.key()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let items = session.model_items.clone();
        let current = session.current_model().map(str::to_string);
        let default_model = session.default_model.clone();
        let success = refreshed.status == catalog_refresh::refresh_status_ok()
            || refreshed.status == "Model catalogs refreshed.";
        let status = refreshed.status.clone();
        let error = snapshot.get_error();
        if let Some(selector) = &mut session.chrome.model_selector {
            selector.reload(items, current, default_model, scoped);
            selector.set_refresh_status(Some(status), success);
            if let Some(error) = error {
                selector.error_message = Some(error);
            }
        }
    }
    if let Some(scoped) = &mut session.chrome.scoped_models {
        scoped.refresh_status = Some(refreshed.status.clone());
    }
    session.chrome.status = refreshed.status;
}

fn open_llama_ui(session: &mut InteractiveSession) -> Result<(), String> {
    let storage = AuthStorage::create().map_err(|err| err.to_string())?;
    let cred = storage.get(llama::LLAMA_PROVIDER_ID);
    let url = llama::resolve_server_url(
        &cred.map(|item| item.env.clone()).unwrap_or_default(),
        cred.and_then(|item| item.key.as_deref()),
    );
    let Ok(url) = llama::normalize_llama_server_url(&url) else {
        session.chrome.status = format!(
            "Configure llama.cpp with /login {}",
            llama::LLAMA_PROVIDER_ID
        );
        return Ok(());
    };
    show_llama_catalog(session, &url)
}

fn show_llama_catalog(session: &mut InteractiveSession, url: &str) -> Result<(), String> {
    let catalog = match llama::list_models(url) {
        Ok(catalog) => catalog,
        Err(err) => {
            session.extension_dialog_context = Some(format!("llama-retry:{url}"));
            session.open_extension_selector(
                llama::connection_retry_title(url, &err),
                vec!["Retry".into(), "Close".into()],
            );
            return Ok(());
        }
    };
    let autoload = llama::router_autoload(url, &catalog);
    let selectable = llama::selectable_models(&catalog, url, autoload)?;
    let inference = llama::llama_inference_url(url)?;
    let mut options: Vec<String> = catalog.iter().map(llama::catalog_option_label).collect();
    options.push("Download model…".into());
    options.push("Close".into());
    session.extension_dialog_context = Some(format!("llama:{url}"));
    session.chrome.status = format!("{} selectable via {inference}", selectable.len());
    session.open_extension_selector(format!("llama.cpp models\n{url}"), options);
    if let Some(selector) = &session.chrome.extension_selector {
        println!("{}", selector.render(80).join("\n"));
    }
    Ok(())
}

fn handle_extension_select(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    choice: Option<String>,
) -> Result<bool, String> {
    let context = session.extension_dialog_context.take();
    if let Some(id) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("branch-summary:"))
    {
        return handle_branch_summary_choice(parsed, agent, session, id, choice);
    }
    let Some(choice) = choice else {
        session.chrome.status = "extension-select cancelled".into();
        return Ok(true);
    };
    if let Some(url) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("llama-retry:"))
    {
        if choice == "Retry" {
            show_llama_catalog(session, url)?;
            return Ok(true);
        }
        session.chrome.status = "llama closed".into();
        return Ok(true);
    }
    if let Some(target) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("llama-hf-pick:"))
    {
        let id = choice
            .split_once(" · ")
            .map(|(id, _)| id)
            .unwrap_or(choice.as_str());
        begin_llama_download(session, target, id);
        return Ok(true);
    }
    if let Some(rest) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("llama-hf-gated:"))
    {
        if let Some((model, target)) = rest.split_once('@') {
            if choice == "Continue" {
                continue_llama_download(session, target, model, None, true);
            } else {
                session.chrome.status = "llama download cancelled".into();
                let _ = show_llama_catalog(session, target);
            }
        }
        return Ok(true);
    }
    if let Some(rest) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("llama-hf-quant:"))
    {
        if let Some((model, target)) = rest.split_once('@') {
            let quant = choice
                .split_once(" · ")
                .map(|(name, _)| name)
                .unwrap_or(choice.as_str());
            finish_llama_download(session, target, &format!("{model}:{quant}"));
        }
        return Ok(true);
    }
    if let Some(rest) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("llama-load:"))
    {
        if let Some((model, target)) = rest.split_once('@') {
            match choice.as_str() {
                "Cancel" => {
                    session.chrome.status = "llama load cancelled".into();
                    let _ = show_llama_catalog(session, target);
                }
                "Unload all and load" => {
                    let catalog = llama::list_models(target).unwrap_or_default();
                    let restore: Vec<String> = llama::loaded_models(&catalog, Some(model))
                        .into_iter()
                        .map(|item| item.id.clone())
                        .collect();
                    for id in &restore {
                        let _ = llama::unload_and_wait(target, id);
                    }
                    start_llama_load(session, target, model, restore)?;
                }
                _ => start_llama_load(session, target, model, Vec::new())?,
            }
        }
        return Ok(true);
    }
    if let Some(url) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("llama:"))
    {
        if choice == "Close" {
            session.chrome.status = "llama closed".into();
            return Ok(true);
        }
        if choice == "Download model…" || choice == "Download model" {
            session.extension_dialog_context = Some(format!("llama-download:{url}"));
            session
                .open_extension_input("Download model", "Model name or owner/repository[:quant]");
            return Ok(true);
        }
        let catalog = llama::list_models(url).unwrap_or_default();
        let autoload = llama::router_autoload(url, &catalog);
        if let Some(model) = catalog
            .iter()
            .find(|model| llama::catalog_option_label(model) == choice)
        {
            if llama::model_is_loaded(model) {
                session.extension_dialog_context = Some(format!("llama-unload:{}@{url}", model.id));
                session.open_extension_confirm("Unload model?", &model.id);
                return Ok(true);
            }
            if llama::model_is_selectable(model, autoload) || model.status.value == "unloaded" {
                let loaded = llama::loaded_models(&catalog, Some(&model.id));
                if !loaded.is_empty() {
                    session.extension_dialog_context =
                        Some(format!("llama-load:{}@{url}", model.id));
                    let title = if loaded.len() == 1 {
                        "1 model is loaded".into()
                    } else {
                        format!("{} models are loaded", loaded.len())
                    };
                    session.open_extension_selector(
                        title,
                        vec![
                            "Unload all and load".into(),
                            "Keep loaded and load".into(),
                            "Cancel".into(),
                        ],
                    );
                    return Ok(true);
                }
                start_llama_load(session, url, &model.id, Vec::new())?;
                return Ok(true);
            }
            session.chrome.status = format!("{} is {}", model.id, model.status.value);
            return Ok(true);
        }
        session.chrome.status = format!("llama={choice}");
        return Ok(true);
    }
    session.chrome.status = format!("extension-select={choice}");
    Ok(true)
}

fn refresh_llama_models(session: &mut InteractiveSession, url: &str) {
    let catalog = llama::list_models(url).unwrap_or_default();
    if let Ok(models) = llama::selectable_models(&catalog, url, true) {
        for model in models {
            let label = format!("{}/{}", model.provider, model.id);
            if !session.models.contains(&label) {
                session.models.push(label);
            }
        }
    }
}

struct LlamaJob {
    kind: &'static str,
    url: String,
    model: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: std::sync::Arc<std::sync::Mutex<llama::LlamaProgress>>,
    result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
    restore: Vec<String>,
}

fn llama_job_slot() -> &'static std::sync::Mutex<Option<LlamaJob>> {
    static SLOT: std::sync::Mutex<Option<LlamaJob>> = std::sync::Mutex::new(None);
    &SLOT
}

fn start_llama_load(
    session: &mut InteractiveSession,
    url: &str,
    model: &str,
    restore: Vec<String>,
) -> Result<(), String> {
    start_llama_op(session, "load", url, model, restore)
}

fn start_llama_download_op(
    session: &mut InteractiveSession,
    url: &str,
    model: &str,
) -> Result<(), String> {
    start_llama_op(session, "download", url, model, Vec::new())
}

fn start_llama_op(
    session: &mut InteractiveSession,
    kind: &'static str,
    url: &str,
    model: &str,
    restore: Vec<String>,
) -> Result<(), String> {
    let title = if kind == "download" {
        "Downloading model"
    } else {
        "Loading model"
    };
    session.extension_dialog_context = Some(format!("llama-{kind}:{model}@{url}"));
    session.open_extension_progress(title, model, "Starting…");
    if llama::fixture_wait_mode() {
        let _ = llama::watch_events(url);
        let status = run_llama_op_blocking(kind, url, model, None, |progress| {
            session.update_extension_progress(
                progress.message.clone(),
                progress.ratio,
                progress.detail.clone(),
            );
            if let Some(ratio) = progress.ratio {
                session.chrome.status = llama::progress_bar(ratio);
            }
        })?;
        finish_llama_op(session, kind, url, model, Ok(status), restore);
        return Ok(());
    }
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let progress = std::sync::Arc::new(std::sync::Mutex::new(llama::LlamaProgress {
        message: "Starting…".into(),
        ratio: None,
        detail: None,
    }));
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let url_owned = url.to_string();
    let model_owned = model.to_string();
    let cancel_t = cancel.clone();
    let progress_t = progress.clone();
    let result_t = result.clone();
    std::thread::spawn(move || {
        let outcome =
            run_llama_op_blocking(kind, &url_owned, &model_owned, Some(&cancel_t), |update| {
                if let Ok(mut guard) = progress_t.lock() {
                    *guard = update;
                }
            });
        if let Ok(mut guard) = result_t.lock() {
            *guard = Some(outcome);
        }
    });
    if let Ok(mut slot) = llama_job_slot().lock() {
        *slot = Some(LlamaJob {
            kind,
            url: url.to_string(),
            model: model.to_string(),
            cancel,
            progress,
            result,
            restore,
        });
    }
    Ok(())
}

fn run_llama_op_blocking(
    kind: &str,
    url: &str,
    model: &str,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    mut on_progress: impl FnMut(llama::LlamaProgress),
) -> Result<String, String> {
    if kind == "download" {
        if cancel.is_none() {
            llama::download_and_wait(url, model, on_progress)?;
        } else {
            llama::download_and_wait_with_cancel(url, model, on_progress, cancel)?;
        }
        return Ok(format!("Downloaded {model}"));
    }
    let mut status = format!("Load started for {model}");
    let loaded = if cancel.is_none() {
        llama::load_and_wait(url, model, |progress| {
            status = format!("{} {model}", progress.message);
            on_progress(progress);
        })?
    } else {
        llama::load_and_wait_with_cancel(
            url,
            model,
            |progress| {
                status = format!("{} {model}", progress.message);
                on_progress(progress);
            },
            cancel,
        )?
    };
    Ok(if loaded.status.value == "loaded" {
        format!("Loaded {}", loaded.id)
    } else {
        status
    })
}

fn finish_llama_op(
    session: &mut InteractiveSession,
    kind: &str,
    url: &str,
    model: &str,
    status: Result<String, String>,
    restore: Vec<String>,
) {
    session.chrome.extension_progress = None;
    match status {
        Ok(message) => {
            session.chrome.status = message;
            refresh_llama_models(session, url);
        }
        Err(err) if err == "Cancelled" => {
            session.chrome.status = format!("{kind} cancelled");
            if !restore.is_empty() {
                session.chrome.status = "Restoring previously loaded models".into();
                for id in restore {
                    let _ = llama::load_and_wait(url, &id, |_| {});
                }
            }
        }
        Err(err) => {
            if !llama::is_connection_error(&err) {
                session.chrome.status = err;
            } else {
                session.chrome.status = llama::connection_error_message(&err);
            }
            if !restore.is_empty() {
                for id in restore {
                    let _ = llama::load_and_wait(url, &id, |_| {});
                }
            }
        }
    }
    let _ = model;
    let _ = show_llama_catalog(session, url);
}

fn poll_llama_job(session: &mut InteractiveSession) -> bool {
    let mut finished = None;
    if let Ok(slot) = llama_job_slot().lock() {
        if let Some(job) = slot.as_ref() {
            if let Ok(progress) = job.progress.lock() {
                session.update_extension_progress(
                    progress.message.clone(),
                    progress.ratio,
                    progress.detail.clone(),
                );
            }
            if let Ok(result) = job.result.lock() {
                if let Some(done) = result.clone() {
                    finished = Some((
                        job.kind,
                        job.url.clone(),
                        job.model.clone(),
                        done,
                        job.restore.clone(),
                    ));
                }
            }
        }
    }
    if let Some((kind, url, model, status, restore)) = finished {
        if let Ok(mut slot) = llama_job_slot().lock() {
            *slot = None;
        }
        finish_llama_op(session, kind, &url, &model, status, restore);
        return true;
    }
    session.chrome.extension_progress.is_some()
}

fn handle_llama_progress_cancel(session: &mut InteractiveSession) {
    let context = session.extension_dialog_context.clone().unwrap_or_default();
    if let Some(rest) = context.strip_prefix("llama-load:") {
        if let Some((model, _)) = rest.split_once('@') {
            session.extension_dialog_context = Some(format!("llama-stop-load:{rest}"));
            session.open_extension_confirm("Stop loading?", model);
            return;
        }
    }
    if let Some(rest) = context.strip_prefix("llama-download:") {
        if let Some((model, _)) = rest.split_once('@') {
            session.extension_dialog_context = Some(format!("llama-stop-download:{rest}"));
            session.open_extension_confirm("Stop download?", model);
            return;
        }
    }
    session.chrome.status = "llama cancelled".into();
}

fn cancel_llama_job(url: &str, model: &str) {
    if let Ok(slot) = llama_job_slot().lock() {
        if let Some(job) = slot.as_ref() {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let _ = llama::unload_model(url, model);
}

fn tick_custom_overlay(parsed: &Args, session: &mut InteractiveSession) -> bool {
    let Some(path) = session.chrome.custom_overlay_path.clone() else {
        return false;
    };
    let Some(command) = session.chrome.custom_overlay_command.clone() else {
        return false;
    };
    let snapshot = session.chrome.custom_overlay_snapshot.clone();
    let mut host = loaded_extension_host(parsed);
    if let Ok(Some(result)) =
        host.invoke_custom_tick(&path, &command, snapshot.as_ref(), session.width)
    {
        if let Some(lines) = result.get("lines").and_then(|value| value.as_array()) {
            session.chrome.custom_overlay_lines = Some(
                lines
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect(),
            );
        }
        if let Some(snapshot) = result.get("snapshot") {
            session.chrome.custom_overlay_snapshot = Some(snapshot.clone());
        }
        return result
            .get("wantsTick")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
    }
    false
}

fn begin_llama_download(session: &mut InteractiveSession, url: &str, value: &str) {
    let (repository, quantization) = llama::parse_hugging_face_model(value);
    if !repository.contains('/') && repository.len() >= 2 {
        match llama::search_hugging_face(&repository) {
            Ok(results) if !results.is_empty() => {
                session.extension_dialog_context = Some(format!("llama-hf-pick:{url}"));
                session.open_extension_selector(
                    "Hugging Face models",
                    results
                        .iter()
                        .map(llama::hugging_face_search_label)
                        .collect(),
                );
            }
            Ok(_) => session.chrome.status = "No Hugging Face models matched".into(),
            Err(err) => session.chrome.status = err,
        }
        return;
    }
    continue_llama_download(session, url, &repository, quantization.as_deref(), false);
}

fn continue_llama_download(
    session: &mut InteractiveSession,
    url: &str,
    repository: &str,
    quantization: Option<&str>,
    skip_gated: bool,
) {
    match llama::hugging_face_details(repository) {
        Ok(details) => {
            if !skip_gated && details.gated != llama::HuggingFaceGated::False {
                let approval = match details.gated {
                    llama::HuggingFaceGated::Manual => "Manual approval is required",
                    _ => "Accept the access terms",
                };
                session.extension_dialog_context =
                    Some(format!("llama-hf-gated:{}@{url}", details.id));
                session.open_extension_selector(
                    format!(
                        "Hugging Face access required\n{}\n\n{approval} at:\nhttps://huggingface.co/{}",
                        details.id, details.id
                    ),
                    vec!["Continue".into(), "Back".into()],
                );
                return;
            }
            if quantization.is_none() && !details.quantizations.is_empty() {
                session.extension_dialog_context =
                    Some(format!("llama-hf-quant:{}@{url}", details.id));
                session.open_extension_selector(
                    format!("Select quantization\n{}", details.id),
                    details
                        .quantizations
                        .iter()
                        .map(llama::quantization_option_label)
                        .collect(),
                );
                return;
            }
            let model = match quantization {
                Some(name) => format!("{}:{name}", details.id),
                None => details.id,
            };
            finish_llama_download(session, url, &model);
        }
        Err(_) => {
            let model = match quantization {
                Some(name) => format!("{repository}:{name}"),
                None => repository.to_string(),
            };
            finish_llama_download(session, url, &model);
        }
    }
}

fn finish_llama_download(session: &mut InteractiveSession, url: &str, model: &str) {
    let token = llama::find_hugging_face_token();
    session.chrome.status = format!("hf={}", token.as_deref().unwrap_or("missing"));
    let _ = start_llama_download_op(session, url, model);
}

fn handle_extension_input(session: &mut InteractiveSession, value: Option<String>) {
    let context = session.extension_dialog_context.take();
    let Some(value) = value else {
        session.chrome.status = "extension-input cancelled".into();
        return;
    };
    if let Some(url) = context
        .as_deref()
        .and_then(|item| item.strip_prefix("llama-download:"))
    {
        begin_llama_download(session, url, &value);
        return;
    }
    session.chrome.status = format!("extension-input={value}");
}

fn handle_extension_confirm(session: &mut InteractiveSession, confirmed: bool) {
    let context = session.extension_dialog_context.take();
    if let Some(rest) = context
        .as_deref()
        .and_then(|item| item.strip_prefix("llama-stop-load:"))
    {
        if let Some((model, url)) = rest.split_once('@') {
            if confirmed {
                cancel_llama_job(url, model);
                session.chrome.status = "llama load cancelled".into();
            } else {
                session.extension_dialog_context = Some(format!("llama-load:{model}@{url}"));
                session.chrome.status = "Loading model".into();
            }
        }
        return;
    }
    if let Some(rest) = context
        .as_deref()
        .and_then(|item| item.strip_prefix("llama-stop-download:"))
    {
        if let Some((model, url)) = rest.split_once('@') {
            if confirmed {
                cancel_llama_job(url, model);
                session.chrome.status = "llama download cancelled".into();
            } else {
                session.extension_dialog_context = Some(format!("llama-download:{model}@{url}"));
                session.chrome.status = "Downloading model".into();
            }
        }
        return;
    }
    if let Some(rest) = context
        .as_deref()
        .and_then(|item| item.strip_prefix("llama-unload:"))
    {
        if confirmed {
            if let Some((model, url)) = rest.split_once('@') {
                let _ = llama::unload_and_wait(url, model);
                session.chrome.status = format!("Unloaded {model}");
                let _ = show_llama_catalog(session, url);
            } else {
                session.chrome.status = format!("Unloaded {rest}");
            }
        } else if let Some((_, url)) = rest.split_once('@') {
            session.chrome.status = "llama unload cancelled".into();
            let _ = show_llama_catalog(session, url);
        } else {
            session.chrome.status = "llama unload cancelled".into();
        }
        return;
    }
    session.chrome.status = format!("extension-confirm={confirmed}");
}

fn discover_session_items(parsed: &Args, agent: &Agent) -> Result<Vec<SessionItem>, String> {
    let session_dir = resolved_session_dir(parsed, &agent.cwd);
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
            all_messages_text: summary.all_messages_text,
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
    let session_dir = resolved_session_dir(parsed, &agent.cwd);
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

fn handle_branch_summary_choice(
    _parsed: &Args,
    agent: &Agent,
    session: &mut InteractiveSession,
    id: &str,
    choice: Option<String>,
) -> Result<bool, String> {
    match choice.as_deref() {
        None => {
            open_session_tree(agent, session);
            Ok(true)
        }
        Some("No summary") => {
            session.chrome.status = format!("tree={id} summary=none");
            Ok(true)
        }
        Some("Summarize") => {
            session.chrome.status = format!(
                "tree={id} summary=summarize reserve={}",
                session.branch_summary_reserve_tokens
            );
            Ok(true)
        }
        Some("Summarize with custom prompt") => {
            session.extension_dialog_context = Some(format!("branch-summary-custom:{id}"));
            session.open_extension_editor("Custom summarization instructions", "");
            Ok(true)
        }
        Some(other) => {
            session.chrome.status = format!("tree={id} summary={other}");
            Ok(true)
        }
    }
}

fn handle_branch_summary_editor(
    _agent: &Agent,
    session: &mut InteractiveSession,
    value: Option<String>,
) {
    let context = session.extension_dialog_context.take();
    if let Some(id) = context
        .as_deref()
        .and_then(|value| value.strip_prefix("branch-summary-custom:"))
    {
        match value {
            None => {
                session.extension_dialog_context = Some(format!("branch-summary:{id}"));
                session.open_extension_selector(
                    "Summarize branch?",
                    vec![
                        "No summary".into(),
                        "Summarize".into(),
                        "Summarize with custom prompt".into(),
                    ],
                );
            }
            Some(instructions) => {
                session.chrome.status = format!("tree={id} summary=custom");
                let _ = instructions;
            }
        }
        return;
    }
    session.chrome.status = format!("extension-editor={}", value.unwrap_or_default());
}

fn select_tree_entry(
    agent: &Agent,
    session: &mut InteractiveSession,
    id: String,
) -> Result<bool, String> {
    if session.branch_summary_skip_prompt {
        session.chrome.status = format!("tree={id} summary=none");
        return Ok(true);
    }
    session.extension_dialog_context = Some(format!("branch-summary:{id}"));
    session.open_extension_selector(
        "Summarize branch?",
        vec![
            "No summary".into(),
            "Summarize".into(),
            "Summarize with custom prompt".into(),
        ],
    );
    let _ = session.branch_summary_reserve_tokens;
    let _ = agent.compaction.reserve_tokens;
    Ok(true)
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
    session.chrome.status = catalog_refresh::refresh_status_refreshing().into();
    let refreshed = catalog_refresh::refresh_model_catalogs(
        &default_agent_dir(),
        std::env::var("PI_OFFLINE").is_err(),
        false,
    );
    let models = refreshed
        .models
        .into_iter()
        .map(|model| ScopedModel {
            provider: model.provider,
            id: model.id,
            name: model.name,
        })
        .collect();
    session.open_scoped_models(models);
    if let Some(scoped) = &mut session.chrome.scoped_models {
        scoped.refresh_status = Some(refreshed.status.clone());
        println!("{}", scoped.render(80).join("\n"));
    }
    session.chrome.status = refreshed.status;
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
    if let Some(found) = available_themes()
        .into_iter()
        .find(|item| item.name == theme)
    {
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
                available_model_ids: Vec::new(),
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
            "install", "remove", "update", "list", "config", "auth", "server", "client", "--print",
            "--resume",
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
        assert!(matches!(
            slash::parse_line("/llama"),
            slash::SlashAction::Llama
        ));
        assert!(slash::builtin_slash_commands()
            .iter()
            .any(|command| command.name == "llama"
                && command.description == "Manage llama.cpp router models"));
    }

    #[test]
    fn radius_share_uses_fixture_url() {
        std::env::set_var("PI_RADIUS_TOKEN", "fixture-token");
        std::env::set_var("PI_RADIUS_ARTIFACT_URL", "https://example.test/session/abc");
        std::env::remove_var("PI_SHARE_DRY_RUN");
        std::env::remove_var("PI_SHARE_URL");
        let agent = Agent::new("sys");
        let shared = share_current_session(&agent).expect("share");
        assert_eq!(shared, "Share URL: https://example.test/session/abc");
        std::env::set_var(
            "PI_RADIUS_ARTIFACT_REPLY",
            r#"{"artifact":{"canonical_url":"https://radius.example/a"}}"#,
        );
        std::env::remove_var("PI_RADIUS_ARTIFACT_URL");
        let shared = share_current_session(&agent).expect("share reply");
        assert_eq!(shared, "Share URL: https://radius.example/a");
        std::env::remove_var("PI_RADIUS_TOKEN");
        std::env::remove_var("PI_RADIUS_ARTIFACT_REPLY");
    }

    #[test]
    fn list_models_table_matches_ts_columns() {
        assert_eq!(format_token_count(200_000), "200K");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
        assert_eq!(format_token_count(500), "500");
        let model = pi_ai::Model {
            id: "sonnet".into(),
            name: "Sonnet".into(),
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            base_url: None,
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            cost: pi_ai::ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 200_000,
            max_tokens: 16_384,
            compat: serde_json::json!(null),
            headers: Default::default(),
        };
        let table = render_models_table(&[&model]);
        let header = table.lines().next().unwrap();
        assert!(header.contains("provider"));
        assert!(header.contains("model"));
        assert!(header.contains("context"));
        assert!(header.contains("max-out"));
        assert!(header.contains("thinking"));
        assert!(header.contains("images"));
        let row = table.lines().nth(1).unwrap();
        assert!(row.contains("anthropic"));
        assert!(row.contains("sonnet"));
        assert!(row.contains("200K"));
        assert!(row.contains("16.4K") || row.contains("16384") || row.contains("16K"));
        assert!(row.contains("yes"));
    }
}
