mod args;
mod auth_cmd;
mod cache_stats;
mod catalog_refresh;
mod changelog;
mod davinci_session;
mod davinci_sources;
mod davinci_surfaces;
#[cfg(unix)]
mod experimental;
#[cfg(not(unix))]
#[allow(dead_code)]
mod experimental {
    use crate::args::{parse_args, Args};
    pub fn is_experimental_command(_: Option<&str>) -> bool {
        false
    }
    pub fn experimental_features_enabled() -> bool {
        false
    }
    pub fn experimental_tool_sampling() -> Option<serde_json::Value> {
        None
    }
    pub enum ExperimentalCli {
        Pi {
            options: Args,
            listen: Vec<UnixAddress>,
        },
        Server {
            listen: Vec<UnixAddress>,
            auth: Option<ExperimentalAuth>,
        },
        Client {
            connect: Option<UnixAddress>,
            auth: Option<ExperimentalAuth>,
        },
    }
    #[derive(Clone)]
    pub struct UnixAddress {
        pub path: String,
    }
    pub enum ExperimentalAuth {
        Token { token: String },
        File { path: String },
    }
    pub struct ServerCommand {
        pub listen: Vec<UnixAddress>,
        pub auth_token: Option<String>,
    }
    pub struct ClientCommand {
        pub connect: Option<UnixAddress>,
        pub auth_token: Option<String>,
    }
    pub fn resolve_experimental_auth(
        _: Option<ExperimentalAuth>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
    pub fn bind_listen_addresses(_: &[UnixAddress]) -> Result<String, String> {
        Ok(String::new())
    }
    pub fn run_server(_: ServerCommand) -> Result<String, String> {
        Err("Unix server is unavailable on this platform".into())
    }
    pub fn run_client(_: ClientCommand) -> Result<String, String> {
        Err("Unix client is unavailable on this platform".into())
    }
    pub fn parse_experimental_cli(raw: &[String]) -> Result<ExperimentalCli, Vec<String>> {
        Ok(ExperimentalCli::Pi {
            options: parse_args(raw),
            listen: vec![],
        })
    }
}

mod export;
mod extension_host;
mod extensions;
mod external_editor;
mod file_processor;
mod image_convert;
mod js_host;
mod llama;
mod migrations;
mod model_resolver;
mod native_extensions;
mod output;
mod packages;
mod rpc;
mod self_update;
mod settings;
mod shutdown;
mod slash;
mod startup;
mod tools_manager;
mod trust;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// True while the raw-mode TUI owns the screen. Any raw `println!` in that
/// state moves the hardware cursor behind the renderer's back and corrupts
/// the diff-based repaint, so the shadowed macros below reroute output into
/// the transcript instead. Process-wide (not thread-local): the streaming
/// worker thread prints too.
static HOSTED_TUI_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static HOSTED_PENDING_LINES: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());

fn hosted_tui_active() -> bool {
    HOSTED_TUI_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

fn hosted_queue_line(role: &'static str, text: String) {
    if let Ok(mut pending) = HOSTED_PENDING_LINES.lock() {
        pending.push((role, text));
    }
}

/// Say whether a TUI owns the screen. The davinci shell holds the alternate
/// screen too, so without this every stray `println!` in shared code painted
/// straight over its frame.
fn set_hosted_tui_active(active: bool) {
    HOSTED_TUI_ACTIVE.store(active, std::sync::atomic::Ordering::Relaxed);
}

/// The queue `drain_hosted_lines` empties, for a shell that keeps its
/// transcript somewhere other than the legacy chrome.
fn take_hosted_lines() -> Vec<(&'static str, String)> {
    HOSTED_PENDING_LINES
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// Shadow `std::println!`: while the TUI hosts the screen, route the line
/// into the transcript (drained by `sync_hosted_chrome`) instead of stdout.
/// Textual macro scope: every `println!` after this point uses it.
macro_rules! println {
    () => {{
        if !crate::hosted_tui_active() {
            ::std::println!();
        }
    }};
    ($($arg:tt)*) => {{
        let text = ::std::format!($($arg)*);
        if crate::hosted_tui_active() {
            crate::hosted_queue_line("system", text);
        } else {
            ::std::println!("{}", text);
        }
    }};
}

macro_rules! eprintln {
    () => {{
        if !crate::hosted_tui_active() {
            ::std::eprintln!();
        }
    }};
    ($($arg:tt)*) => {{
        let text = ::std::format!($($arg)*);
        if crate::hosted_tui_active() {
            crate::hosted_queue_line("notice", text);
        } else {
            ::std::eprintln!("{}", text);
        }
    }};
}

use pi_agent::{
    default_system_prompt, discover_prompt_templates, discover_skills, env_summarizer,
    load_context_files, Agent, AgentEvent, CompleteOutput, CustomToolExecutor, EventSink,
    SummarizeRequest, SummarizeResponse, Summarizer,
};
use pi_ai::{
    apply_config_auth_with_shell, apply_models_config, check_auth, complete_simple, content_text,
    find_model, format_no_api_key_found_message, format_no_model_selected_message,
    format_no_models_available_message, format_oauth_auth_failed_message, fuzzy_models,
    get_supported_thinking_levels, live_complete_streaming_with_sink, load_builtin_models,
    models_json_path, resolve_provider_auth, snapshot_availability, AssistantMessage, AuthStorage,
    ContentBlock, Credential, CredentialKind, ModelConfig, ModelRuntimeSnapshot, ResolvedAuth,
    StopReason, StreamOptions, ToolSpec, NO_MODELS_AVAILABLE, PROVIDER_SPECS,
};
use pi_coding_agent::interactive_tui::{
    create_interactive_tui, handle_copy_command, remount_chrome_panes, stop_interactive_tui,
    switch_tui_mode, ChromePanes, CopyCommandResult, InteractiveTui, InteractiveTuiOptions,
};
use pi_session::{
    default_agent_dir, discover_sessions, encode_header, latest_session, now_ms,
    resolve_session_dir_from, resolve_session_ref, JsonlSession, SessionEntry,
};
use pi_tui::{
    builtin_themes, collect_name_collisions, copy_text, detect_terminal_theme,
    detect_terminal_theme_for_auto, drain_osc_tty, encode_kitty, format_collision_diagnostic,
    format_context_path, format_display_path, infer_source_info, interactive_settings_list,
    load_themes_from_dir, parse_auto_theme, parse_http_idle_timeout, resolve_git_branch,
    theme_files_from_dir, AuthSelectorMode, AuthSelectorProvider, AutocompleteItem, ChatChrome,
    Component, CustomMessage, DoubleEscapeAction, ExtraAutocompleteProvider, FilterMode,
    InteractiveSession, Keybindings, LiveAutocompleteQuery, LoadedResourceItem, MermaidMode,
    ModelSelectorItem, ScopedModel, SessionAction, SessionItem, SessionTreeEntry, SlashCommandSpec,
    Theme, ThemeDetection, ToolCard, TrustOption, TrustSavedDecision, TrustSelector, TrustUpdate,
    TuiMode, FALLBACK_PREVIEW_LINES, OSC_QUERY_TIMEOUT_MS,
};

use args::{
    format_terminal_title, parse_args, print_help, Args, ListModels, Mode, APP_NAME, VERSION,
};
use auth_cmd::{
    is_auth_command_help, parse_auth_command, print_auth_command_help, validate_auth_command_args,
    AuthCommandKind,
};
use extension_host::{ExtensionEvent, ExtensionHost};
use external_editor::{clipboard_image_png, clipboard_text, ExternalEditor};
use file_processor::{prepare_initial_message, RPC_FILE_ARGS_ERROR};
use native_extensions::{command_specs, native_invocable_commands};
use packages::handle_package_command;
use rpc::{handle_rpc, RpcCommand, RpcRuntime};
use settings::{
    apply_http_proxy_settings, clear_compaction_threshold, default_project_trust_value, is_trusted,
    load_merged_settings, load_merged_settings_with_override, load_settings, save_settings,
    set_compaction_threshold, set_enable_analytics, settings_path, should_run_first_time_setup,
    to_interactive_config,
};
use slash::SlashAction;
use trust::{
    get_project_trust_options, has_trust_requiring_project_resources, ProjectTrustStore,
    ProjectTrustUpdate,
};

const NO_SESSION_SELECTED: &str = "__pi_no_session_selected__";

/// TS `main()`: `--offline` or truthy `PI_OFFLINE` sets `PI_OFFLINE=1` and `PI_SKIP_VERSION_CHECK=1`.
fn apply_offline_mode(raw: &[String]) {
    let from_flag = raw.iter().any(|arg| arg == "--offline");
    if from_flag || tools_manager::is_offline_mode_enabled() {
        std::env::set_var("PI_OFFLINE", "1");
        std::env::set_var("PI_SKIP_VERSION_CHECK", "1");
    }
}

/// `pi --davinci --screen <id>` renders one mockup screen against the fixtures
/// in `docs/ui`, so each can be matched against `Pi TUI Mockups.dc.html` in a
/// real terminal. The live shell is `davinci_session::run`.
fn run_davinci_screens(raw: &[String]) -> Result<i32, String> {
    use pi_tui::davinci;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let screen = raw
        .iter()
        .position(|arg| arg == "--screen")
        .and_then(|index| raw.get(index + 1))
        .cloned()
        .unwrap_or_else(|| "1b".to_string());

    let mut model = davinci::boot(raw, 100, 44);
    davinci::fixtures::dress_screen(&mut model, &screen);
    model.config_path = default_agent_dir()
        .join("config.json")
        .display()
        .to_string();

    // Everything this workspace can already answer comes from the real
    // sources; the plan, the code graph, the recall index and the token budget
    // are still fixtures, and are reported as such.
    let session_dir = pi_session::default_session_dir();
    davinci_sources::dress_from_workspace(&mut model, &cwd, &session_dir);
    if screen == "1a" {
        model.transcript.clear();
    }

    davinci::runtime::run(&mut model, |model, text| {
        model.transcript.push(davinci::model::Entry::Gap);
        model.transcript.push(davinci::model::Entry::prose(&format!(
            "Not wired yet: {text}"
        )));
    })
    .map_err(|err| err.to_string())?;
    Ok(0)
}

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
    apply_offline_mode(&raw);
    // `--davinci --screen <id>` renders a mockup screen against fixtures for
    // comparison with docs/ui. The davinci shell is what interactive pi opens;
    // `--legacy-tui` (or `PI_DAVINCI=0`) asks for the previous chrome, which
    // still owns image display, alt-screen search, mouse selection and the
    // extension dialogs.
    if raw.iter().any(|arg| arg == "--davinci") && raw.iter().any(|arg| arg == "--screen") {
        return run_davinci_screens(&raw);
    }
    if raw.iter().any(|arg| arg == "--legacy-tui") {
        std::env::set_var("PI_DAVINCI", "0");
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    apply_http_proxy_settings(
        load_merged_settings(&default_agent_dir(), &cwd)
            .http_proxy
            .as_deref(),
    );
    tools_manager::prepend_tools_bin_to_path();
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

    let parsed = if experimental::is_experimental_command(raw.first().map(String::as_str))
        || experimental::experimental_features_enabled()
    {
        match experimental::parse_experimental_cli(&raw).map_err(|errors| errors.join("\n"))? {
            experimental::ExperimentalCli::Server { listen, auth } => {
                let message = experimental::run_server(experimental::ServerCommand {
                    listen,
                    auth_token: experimental::resolve_experimental_auth(auth)?,
                })?;
                println!("{message}");
                return Ok(0);
            }
            experimental::ExperimentalCli::Client { connect, auth } => {
                let message = experimental::run_client(experimental::ClientCommand {
                    connect,
                    auth_token: experimental::resolve_experimental_auth(auth)?,
                })?;
                println!("{message}");
                return Ok(0);
            }
            experimental::ExperimentalCli::Pi {
                listen, options, ..
            } => {
                if !listen.is_empty() {
                    let _ = experimental::bind_listen_addresses(&listen)?;
                }
                options
            }
        }
    } else {
        parse_args(&raw)
    };
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
        let host = loaded_extension_host(&parsed);
        write_cli_text(
            &parsed,
            &args::print_help_with_extension_flags(&host.registered_flags()),
        );
        return Ok(0);
    }
    if parsed.version {
        println!("{VERSION}");
        return Ok(0);
    }
    if let Some(list) = &parsed.list_models {
        return list_models(list, should_take_over_stdout(&parsed));
    }
    if let Some(export) = &parsed.export {
        return export_session(&parsed, export);
    }

    let session_dir = resolved_session_dir(&parsed, &cwd);
    let migrations = migrations::maybe_run_startup_migrations(&cwd);
    let mut agent = match build_agent(&parsed, &session_dir, &cwd) {
        Err(err) if err == NO_SESSION_SELECTED => return Ok(0),
        other => other?,
    };

    if parsed.mode == Some(Mode::Rpc) {
        let _ = tools_manager::ensure_managed_tools();
        if !parsed.file_args.is_empty() {
            eprintln!("{RPC_FILE_ARGS_ERROR}");
            return Ok(1);
        }
        return run_rpc(&parsed, &mut agent);
    }

    // Fixture: force the interactive path without a TTY so tests can inspect
    // the rendered chrome (line-session mode).
    let force_interactive = matches!(
        std::env::var("PI_FORCE_INTERACTIVE").as_deref(),
        Ok("1") | Ok("true")
    );
    let stdin_tty = io::stdin().is_terminal() || force_interactive;
    let stdout_tty = io::stdout().is_terminal() || force_interactive;
    if parsed.print || parsed.mode == Some(Mode::Json) || !stdin_tty || !stdout_tty {
        let _ = tools_manager::ensure_managed_tools();
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

/// The system prompt for one request, with the model actually serving it named
/// in the text.
///
/// A documented divergence from vendor `pi`: `buildSystemPrompt`
/// (`vendor/pi/packages/coding-agent/src/core/system-prompt.ts`) carries cwd,
/// tools, context files and skills but no model identity, so "what model are
/// you?" was answered from the model's training prior rather than from the run.
/// The line is appended per request rather than stored on the agent, so a
/// `/model` switch, a thinking-level change, or an extension's `systemPrompt`
/// override can never leave a stale identity behind.
fn system_prompt_with_identity(agent: &Agent) -> String {
    let mut prompt = agent.system_prompt.clone();
    if agent.provider.is_empty() || agent.model_id.is_empty() {
        return prompt;
    }
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(&format!(
        "You are running as {}/{} (thinking: {}).",
        agent.provider,
        agent.model_id,
        agent.thinking_level.as_str()
    ));
    prompt
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
    let settings = load_merged_settings_with_override(
        &default_agent_dir(),
        cwd,
        parsed.project_trust_override,
    );
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
    agent.cwd = cwd.to_path_buf();
    apply_discovered_resources(parsed, &mut agent);
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
    agent.block_images = settings.block_images();
    agent.auto_resize_images = settings.image_auto_resize();
    agent.transport = settings.transport.clone();
    agent.install_telemetry = settings.install_telemetry_enabled();
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
    let mut extensions = if parsed.no_extensions {
        parsed.extensions.clone()
    } else {
        let mut extensions = settings.extensions.clone();
        extensions.extend(parsed.extensions.clone());
        extensions
    };
    for pkg in &settings.packages {
        if !parsed.no_extensions {
            for path in settings::collect_package_resources(pkg, "extensions") {
                extensions.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let mut host = ExtensionHost::load_with_cwd(&default_agent_dir(), &extensions, cwd);
    let mut names = host.native_tool_names();
    // The extension *paths* are not tool names — TS registers only what an
    // extension declares (`resolvedExtensionPaths` never reaches the tool
    // registry). Registering them here put rows like
    // `C:/Users/…/pi-main/packages/…` in the palette's tool corpus.
    names.extend(extensions::extension_tool_names(&host.manifests));
    for ext in &host.js {
        names.extend(ext.tools.iter().cloned());
        names.extend(ext.commands.iter().cloned());
        let _ = ext.handlers.as_slice();
    }
    agent.apply_extension_tools(&names);
    attach_tool_executor(&mut agent, &host);
    host.emit(ExtensionEvent::SessionStart);
    let _ = host.describe_js();
    apply_resolved_models(parsed, &mut agent)?;
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
        maybe_refresh_auth(
            storage,
            &request.provider,
            now_ms(),
            OAUTH_MIN_VALIDITY_MS,
            false,
        );
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
        websocket_connect_timeout_ms: load_settings(&default_agent_dir())
            .websocket_connect_timeout_ms,
        transport: load_settings(&default_agent_dir()).transport.clone(),
        session_id: None,
        cache_retention: Some("none".into()),
        install_telemetry: Some(load_settings(&default_agent_dir()).install_telemetry_enabled()),
        abort_signal: None,
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
        persist_selected_backend(&session, session_dir);
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
        match select_resume_session(parsed, session_dir, cwd)? {
            Some(path) => return JsonlSession::open(&path).map_err(|err| err.to_string()),
            None => {
                println!("\x1b[2mNo session selected\x1b[0m");
                return Err(NO_SESSION_SELECTED.into());
            }
        }
    }
    let session = JsonlSession::create(
        session_dir,
        &cwd.to_string_lossy(),
        parsed
            .name
            .as_deref()
            .and_then(args::normalize_session_name)
            .as_deref(),
    )
    .map_err(|err| err.to_string())?;
    persist_selected_backend(&session, session_dir);
    Ok(session)
}

fn persist_selected_backend(session: &JsonlSession, session_dir: &Path) {
    let backend = std::env::var("PI_SESSION_BACKEND").unwrap_or_else(|_| {
        load_settings(&default_agent_dir())
            .session_backend()
            .to_string()
    });
    if backend != "sqlite" {
        return;
    }
    if let Ok(store) = pi_session_sqlite::SqliteSessionStore::open(&session_dir.join("sessions.db"))
    {
        let _ = store.upsert_session(session);
    }
}

fn available_models(parsed: &Args) -> Vec<pi_ai::Model> {
    load_model_runtime(parsed).available
}

fn has_configured_auth(snapshot: &ModelRuntimeSnapshot, provider: &str) -> bool {
    snapshot
        .configured_providers
        .iter()
        .any(|item| item == provider)
        || snapshot.auth.contains_key(provider)
}

fn session_was_restored(parsed: &Args) -> bool {
    parsed.continue_session
        || parsed.resume
        || parsed.session.is_some()
        || parsed.fork.is_some()
        || parsed.session_id.is_some()
}

fn apply_resolved_models(parsed: &Args, agent: &mut Agent) -> Result<(), String> {
    let snapshot = load_model_runtime(parsed);
    let scoped = if parsed.models.is_empty() {
        model_resolver::ResolveModelScopeResult {
            scoped_models: Vec::new(),
            diagnostics: Vec::new(),
        }
    } else {
        let result = model_resolver::resolve_model_scope_from_models(&parsed.models, &snapshot.all);
        for diagnostic in &result.diagnostics {
            eprintln!("Warning: {}", diagnostic.message);
        }
        result
    };

    if let Some(cli_model) = parsed.model.as_deref() {
        let resolved = model_resolver::resolve_cli_model(
            parsed.provider.as_deref(),
            Some(cli_model),
            parsed.thinking,
            &snapshot.all,
            |provider| has_configured_auth(&snapshot, provider),
        );
        if let Some(error) = resolved.error {
            return Err(error);
        }
        if let Some(warning) = resolved.warning {
            eprintln!("Warning: {warning}");
        }
        if let Some(model) = resolved.model {
            agent.provider = model.provider;
            agent.model_id = model.id;
            agent.context_window = model.context_window;
            if parsed.thinking.is_none() {
                if let Some(level) = resolved.thinking_level {
                    agent.thinking_level = level;
                }
            }
            return Ok(());
        }
    }

    if !scoped.scoped_models.is_empty() && !session_was_restored(parsed) {
        let settings = load_settings(&default_agent_dir());
        let saved = match (&settings.default_provider, &settings.default_model) {
            (Some(provider), Some(id)) => scoped
                .scoped_models
                .iter()
                .find(|item| item.model.provider == *provider && item.model.id == *id),
            _ => None,
        };
        let chosen = saved.unwrap_or(&scoped.scoped_models[0]);
        agent.provider = chosen.model.provider.clone();
        agent.model_id = chosen.model.id.clone();
        agent.context_window = chosen.model.context_window;
        if parsed.thinking.is_none() {
            if let Some(level) = chosen.thinking_level {
                agent.thinking_level = level;
            }
        }
        return Ok(());
    }

    // TS `findInitialModel` steps 3 and 4: the saved default from settings when
    // its provider has auth, then the first available model — preferring each
    // known provider's default id. Without these a plain `pi` fell through to
    // the hardcoded `google` with an empty model id.
    if parsed.provider.is_none() && parsed.model.is_none() {
        let settings = load_settings(&default_agent_dir());
        if let (Some(provider), Some(id)) = (&settings.default_provider, &settings.default_model) {
            if let Some(model) = find_model(&snapshot.available, provider, id) {
                agent.provider = model.provider.clone();
                agent.model_id = model.id.clone();
                agent.context_window = model.context_window;
                if parsed.thinking.is_none() {
                    if let Some(level) = settings
                        .model_thinking_levels
                        .as_ref()
                        .and_then(|levels| levels.get(&format!("{provider}/{id}")))
                        .or(settings.default_thinking_level.as_ref())
                        .and_then(|level| pi_protocol::ThinkingLevel::parse(level))
                    {
                        agent.thinking_level = level;
                    }
                }
                return Ok(());
            }
        }
        let fallback = model_resolver::DEFAULT_MODEL_PER_PROVIDER
            .iter()
            .find_map(|(provider, id)| find_model(&snapshot.available, provider, id))
            .or_else(|| snapshot.available.first());
        if let Some(model) = fallback {
            agent.provider = model.provider.clone();
            agent.model_id = model.id.clone();
            agent.context_window = model.context_window;
            return Ok(());
        }
    }

    let (provider, model_id) = parse_model_ref(
        parsed.provider.as_deref().unwrap_or("google"),
        parsed.model.as_deref(),
    );
    agent.provider = provider;
    agent.model_id = model_id;
    if let Some(model) = find_model(&snapshot.available, &agent.provider, &agent.model_id) {
        agent.context_window = model.context_window;
    }
    Ok(())
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
    let extension_host = loaded_extension_host(parsed);
    extension_host.filter_models(&mut models);
    for provider in extension_host.registered_providers() {
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

fn list_models(list: &ListModels, takeover: bool) -> Result<i32, String> {
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
        write_text(
            takeover,
            &format_no_models_available_message(&coding_agent_docs_dir()),
        );
        return Ok(0);
    }
    let selected = match list {
        ListModels::All => snapshot.available.iter().collect(),
        ListModels::Query(query) => fuzzy_models(&snapshot.available, query),
    };
    if selected.is_empty() {
        if let ListModels::Query(query) = list {
            write_text(takeover, &format!("No models matching \"{query}\""));
            return Ok(0);
        }
    }
    write_text(takeover, &render_models_table(&selected));
    Ok(0)
}

/// TS `takeOverStdout`: non-interactive JSON/RPC/print keep stdout for the protocol.
fn should_take_over_stdout(parsed: &Args) -> bool {
    let plain_metadata =
        !parsed.print && parsed.mode.is_none() && (parsed.help || parsed.list_models.is_some());
    if plain_metadata {
        return false;
    }
    parsed.mode == Some(Mode::Rpc)
        || parsed.mode == Some(Mode::Json)
        || parsed.print
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
}

fn write_cli_text(parsed: &Args, text: &str) {
    write_text(should_take_over_stdout(parsed), text);
}

fn write_text(takeover: bool, text: &str) {
    if takeover {
        eprint!("{text}");
        if !text.ends_with('\n') {
            eprintln!();
        }
    } else {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
    }
}

fn export_session(parsed: &Args, export: &str) -> Result<i32, String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let session_dir = resolved_session_dir(parsed, &cwd);
    let session = if Path::new(export).exists() {
        JsonlSession::open(Path::new(export)).map_err(|err| err.to_string())?
    } else {
        match resolve_or_create_session(parsed, &session_dir, &cwd) {
            Err(err) if err == NO_SESSION_SELECTED => return Ok(0),
            other => other?,
        }
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
    maybe_refresh_auth(
        &mut storage,
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

fn latest_user_prompt(agent: &Agent) -> (String, Vec<pi_ai::MessageContent>) {
    let Some(message) = agent
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
    else {
        return (String::new(), Vec::new());
    };
    let images = message
        .content
        .iter()
        .filter(|block| matches!(block, pi_ai::MessageContent::Image { .. }))
        .cloned()
        .collect();
    (content_text(&message.content), images)
}

fn agent_memory_messages(agent: &Agent) -> Vec<crate::native_extensions::MemoryMessage> {
    agent
        .messages
        .iter()
        .filter_map(|message| {
            let mut content = content_text(&message.content);
            if content.is_empty() && message.role == "bashExecution" {
                content = message
                    .extra
                    .get("output")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            if content.trim().is_empty() {
                None
            } else {
                Some(crate::native_extensions::MemoryMessage {
                    role: message.role.clone(),
                    content,
                })
            }
        })
        .collect()
}

fn complete_prompt(parsed: &Args, agent: &mut Agent) -> (String, Vec<AgentEvent>) {
    complete_prompt_with_host(parsed, agent, None, false)
}

fn complete_prompt_with_host(
    parsed: &Args,
    agent: &mut Agent,
    existing_host: Option<Arc<Mutex<ExtensionHost>>>,
    stream_json: bool,
) -> (String, Vec<AgentEvent>) {
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
        maybe_refresh_auth(
            storage,
            &agent.provider,
            now_ms(),
            OAUTH_MIN_VALIDITY_MS,
            false,
        );
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
    apply_js_oauth_api_key(&agent.provider, storage.as_ref(), &mut auth);
    if auth.as_ref().is_some_and(|item| {
        item.api_key.is_none() && item.headers.is_empty() && item.source == "none"
    }) {
        auth = None;
    }
    // The refresh above already had its chance. A token that is past its
    // expiry now cannot be renewed, and sending it would come back as a bare
    // 401; say which of the two happened instead.
    let expired_oauth = storage
        .as_ref()
        .and_then(|storage| storage.get(&agent.provider))
        .is_some_and(|cred| {
            cred.kind == CredentialKind::Oauth && pi_ai::credential_expires_by(cred, now_ms())
        });
    if expired_oauth {
        auth = None;
    }
    let fresh_host = existing_host.is_none();
    let host = existing_host.unwrap_or_else(|| Arc::new(Mutex::new(loaded_extension_host(parsed))));
    attach_shared_tool_executor(agent, host.clone());
    let native_tool_specs = host
        .lock()
        .map(|host| host.native_tool_specs())
        .unwrap_or_default();
    let tools: Vec<ToolSpec> = pi_agent::tool_specs()
        .into_iter()
        .filter(|tool| agent.tools.iter().any(|name| name == &tool.name))
        .map(|tool| ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
            constrained_sampling: crate::experimental::experimental_tool_sampling(),
        })
        .chain(
            native_tool_specs
                .into_iter()
                .filter(|tool| agent.tools.iter().any(|name| name == &tool.name)),
        )
        .collect();
    agent.clear_ephemeral_context();
    {
        let mut host = host.lock().unwrap_or_else(|err| err.into_inner());
        if fresh_host {
            host.emit(ExtensionEvent::SessionStart);
        }
        host.runtime_active_tools = agent.tools.clone();
        host.runtime_all_tools = agent.tool_registry.clone();
        host.runtime_thinking_level = agent.thinking_level.as_str().to_string();
        host.runtime_flag_values = flag_values_json(parsed);
        agent.reset_system_prompt_to_base();
        host.runtime_system_prompt = agent.system_prompt.clone();
        let (prompt, images) = latest_user_prompt(agent);
        host.emit_before_agent_start(&prompt, &images);
        if let Some(prompt) = host.last_result_system_prompt() {
            agent.system_prompt = prompt.clone();
            host.runtime_system_prompt = prompt;
        }
        for message in host.take_before_agent_start_messages() {
            agent.record_custom_message(&message);
        }
        if let Some(memory) = host.native_memory_inject(&prompt) {
            agent.set_ephemeral_context(vec![pi_ai::ChatMessage::text("custom", memory)]);
        }
        host.emit(ExtensionEvent::AgentStart);
        host.emit(ExtensionEvent::TurnStart);
        host.emit(ExtensionEvent::BeforeProviderRequest {
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
        });
        host.emit(ExtensionEvent::BeforeProviderHeaders {
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
        });
    }
    let js_stream = {
        let host = host.lock().unwrap_or_else(|err| err.into_inner());
        host.js_stream_provider(&agent.provider)
    };
    let hook_host = host.clone();
    let tool_state_cwd = agent.cwd.clone();
    agent.pre_tool = Some(pi_agent::PreToolHook(Arc::new(move |name, args| {
        // A poisoned lock used to bail out of the closure with `None`, which
        // the agent reads as "not blocked": one panic anywhere holding this
        // mutex silently disabled every pre-tool guard, the security scan's
        // included, for the rest of the process. Take the value through the
        // poison instead, as every other site here does.
        let mut host = hook_host.lock().unwrap_or_else(|err| err.into_inner());
        host.emit(ExtensionEvent::ToolCall {
            tool_name: name.to_string(),
            args: args.clone(),
        });
        let state_hash = crate::native_extensions::repo_state_key(&tool_state_cwd);
        if let Some(reason) = host.native_before_tool(name, args, &state_hash) {
            return Some(reason);
        }
        if host.tool_call_blocked() {
            Some(
                host.last_js_result
                    .as_ref()
                    .and_then(|value| value.get("reason"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("blocked by extension")
                    .to_string(),
            )
        } else {
            None
        }
    })));
    let post_host = host.clone();
    agent.post_tool = Some(pi_agent::PostToolHook(Arc::new(
        move |_tool_call_id, _cwd, name, args, result| match post_host.lock() {
            Ok(host) => host.native_after_tool(name, args, result),
            Err(_) => result,
        },
    )));
    if stream_json {
        agent.event_sink = Some(EventSink(Arc::new(|event| {
            if let Ok(value) = to_json_print_event(event) {
                if let Ok(encoded) = serde_json::to_string(&value) {
                    let _ = output::write_raw_stdout_line(&encoded);
                }
            }
        })));
    }
    let events = agent
        .run_loop(|current| {
            let last_user = current
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| content_text(&m.content).len())
                .unwrap_or(0);
            let system = system_prompt_with_identity(current);
            match (offline, model.as_ref(), auth.as_ref(), js_stream.as_ref()) {
                (false, Some(model), _, Some((path, name))) => {
                    crate::js_host::run_js_stream_simple(
                        Path::new(path),
                        name,
                        model,
                        &current.messages_for_provider(),
                        &system,
                    )
                    .map(CompleteOutput::from)
                }
                (false, Some(model), Some(auth), None) => {
                    // Every stream event reaches the sink the moment it is
                    // decoded, as a `MessageUpdate` carrying the partial
                    // message, with a `MessageStart` ahead of the first one.
                    // The loop records them afterwards without resending.
                    let mut started = false;
                    let mut sink = |event: &pi_ai::AssistantMessageEvent| {
                        let partial = Arc::new(pi_ai::assistant_to_chat(event.message()));
                        if !started {
                            started = true;
                            current.emit_live(AgentEvent::MessageStart {
                                message: (*partial).clone(),
                            });
                        }
                        current.emit_live(AgentEvent::MessageUpdate {
                            message: partial,
                            assistant_message_event: event.clone(),
                        });
                    };
                    let result = live_complete_streaming_with_sink(
                        model,
                        &current.messages_for_provider(),
                        auth,
                        Some(&system),
                        &tools,
                        &StreamOptions {
                            thinking_level: Some(current.thinking_level),
                            thinking_budgets: current.thinking_budgets.clone(),
                            timeout_ms: current.provider_timeout_ms,
                            max_retries: current.provider_max_retries,
                            max_retry_delay_ms: Some(current.provider_max_retry_delay_ms),
                            max_tokens: None,
                            websocket_connect_timeout_ms: load_settings(&default_agent_dir())
                                .websocket_connect_timeout_ms,
                            transport: current.transport.clone(),
                            session_id: current
                                .session
                                .as_ref()
                                .map(|session| session.header.id.clone()),
                            cache_retention: None,
                            install_telemetry: Some(current.install_telemetry),
                            abort_signal: current.abort_signal.clone(),
                        },
                        &mut sink,
                    );
                    result.map(|(message, stream_events)| CompleteOutput {
                        message,
                        stream_events: Some(stream_events),
                        streamed_live: started,
                    })
                }
                // Nothing was asked of a provider. Say which of the three
                // reasons it was: an offline run answers with the stub the
                // fixtures expect, but a missing model or a missing credential
                // is a fault, and a reply that only counts the characters it
                // was handed reads like an answer while hiding one.
                (true, ..) => Ok(CompleteOutput::from(AssistantMessage {
                    id: pi_agent::new_message_id(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: format!("(offline) received {last_user} characters"),
                    }],
                    model: format!("{}/{}", current.provider, current.model_id),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })),
                (false, None, ..) => Err(format!(
                    "No model matched {}/{}. Run /model to choose one, or check ~/.pi/agent/models.json.",
                    current.provider, current.model_id
                )),
                (false, Some(_), None, None) if expired_oauth => Err(format!(
                    "The {provider} sign-in expired and could not be refreshed. Run /login {provider}.",
                    provider = current.provider
                )),
                (false, Some(_), None, None) => Err(format!(
                    "No credential for {provider}. Run /login {provider}.",
                    provider = current.provider
                )),
            }
        })
        .unwrap_or_else(|err| {
            // A run that failed outright never reached the sink with an end
            // event; give it one, so JSON and RPC clients see the failure.
            let end = AgentEvent::AgentEnd {
                messages: vec![pi_ai::ChatMessage::text(
                    "assistant",
                    format!("Provider error: {err}"),
                )],
                will_retry: false,
            };
            agent.emit_live(end.clone());
            vec![end]
        });
    agent.event_sink = None;
    // The last assistant message may be a tool call with no text — the
    // reply is then whatever the run ended on, not an empty string that
    // hides a provider error behind "the model returned no text".
    let reply = agent
        .last_assistant_text()
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            events.iter().rev().find_map(|event| match event {
                AgentEvent::AgentEnd { messages, .. } => messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant")
                    .map(|m| content_text(&m.content)),
                _ => None,
            })
        })
        .unwrap_or_default();
    agent.pre_tool = None;
    agent.post_tool = None;
    {
        let mut host = host.lock().unwrap_or_else(|err| err.into_inner());
        host.emit(ExtensionEvent::AfterProviderResponse {
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
        });
        host.emit(ExtensionEvent::TurnEnd);
        host.emit(ExtensionEvent::AgentEnd);
        host.emit(ExtensionEvent::AgentSettled);
        let memory_messages = agent_memory_messages(agent);
        let _ = host.native_index_messages(&memory_messages);
        for event in &events {
            match event {
                AgentEvent::MessageStart { message } => {
                    host.emit(ExtensionEvent::MessageStart {
                        text: content_text(&message.content),
                    });
                }
                AgentEvent::MessageUpdate { message, .. } => {
                    host.emit(ExtensionEvent::MessageUpdate {
                        text: content_text(&message.content),
                    });
                }
                AgentEvent::MessageEnd { message } => {
                    host.emit(ExtensionEvent::MessageEnd {
                        text: content_text(&message.content),
                    });
                }
                AgentEvent::ToolExecutionEnd {
                    tool_name,
                    is_error,
                    ..
                } => {
                    host.emit(ExtensionEvent::ToolExecutionEnd {
                        tool_name: tool_name.clone(),
                        is_error: *is_error,
                    });
                    host.emit(ExtensionEvent::ToolResult {
                        tool_name: tool_name.clone(),
                        is_error: *is_error,
                    });
                }
                AgentEvent::ToolExecutionUpdate { tool_name, .. } => {
                    host.emit(ExtensionEvent::ToolExecutionUpdate {
                        tool_name: tool_name.clone(),
                    });
                }
                AgentEvent::ToolExecutionStart {
                    tool_name, args, ..
                } => {
                    host.emit(ExtensionEvent::ToolExecutionStart {
                        tool_name: tool_name.clone(),
                        args: args.clone(),
                    });
                    if let Some(result) = host.execute_named_tool(tool_name, &agent.cwd) {
                        let _ = result;
                    }
                }
                _ => {}
            }
        }
        // Taken, not cloned: davinci shares one host across every turn, and
        // re-reading the vector re-applied every past call — a fork would
        // fork again on each later turn.
        let session_calls = std::mem::take(&mut host.session_calls);
        apply_session_calls(
            Some(parsed),
            agent,
            SessionCallUi::Silent,
            &session_calls,
            false,
        );
        if session_calls.iter().any(|call| {
            matches!(
                call.get("op").and_then(|value| value.as_str()),
                Some("newSession" | "fork" | "switchSession" | "reload")
            )
        }) {
            rebind_print_extensions(parsed, agent, &mut host);
        }
        // The davinci transcript is the only place those calls can be seen;
        // print mode stays silent about them, as the TS reference is.
        if hosted_tui_active() {
            for call in &session_calls {
                if let Some(line) = session_call_note(call) {
                    println!("{line}");
                }
            }
        }
        let _ = host.kinds();
    }
    let _ = ExtensionHost::js_summary(&crate::js_host::JsExtensionResult::default());
    (reply, events)
}

fn flag_values_json(parsed: &Args) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, value) in &parsed.unknown_flags {
        map.insert(
            name.clone(),
            match value {
                args::FlagValue::Bool(flag) => serde_json::Value::Bool(*flag),
                args::FlagValue::String(text) => serde_json::Value::String(text.clone()),
            },
        );
    }
    serde_json::Value::Object(map)
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
    if let Some(code) = immediate_shutdown_if_fixture(parsed) {
        return Ok(code);
    }
    install_mode_shutdown_watchers(parsed);
    let stdin_content = if !io::stdin().is_terminal() {
        let text = io::read_to_string(io::stdin()).unwrap_or_default();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    } else {
        None
    };
    let settings = load_merged_settings(&default_agent_dir(), &agent.cwd);
    let prepared = prepare_initial_message(
        &parsed.messages,
        &parsed.file_args,
        stdin_content.as_deref(),
        &agent.cwd,
        settings.image_auto_resize(),
    )?;
    let json_mode = parsed.mode == Some(Mode::Json);
    if json_mode {
        if let Some(session) = &agent.session {
            print!("{}", encode_header(&session.header));
        }
    }
    let mut last_reply = String::new();
    let mut all_events = Vec::new();
    if let Some(prompt) = &prepared.text {
        if !prompt.trim().is_empty() || !prepared.images.is_empty() {
            match prepare_user_input(parsed, agent, prompt, &prepared.images, "print", None)? {
                PreparedInput::Handled => {}
                PreparedInput::Ready { text, images } => {
                    agent.prompt_with(&text, &images);
                    let (reply, events) = complete_prompt_with_host(parsed, agent, None, json_mode);
                    last_reply = reply;
                    all_events.extend(events);
                }
            }
        }
    }
    for extra in &prepared.remaining_messages {
        if extra.trim().is_empty() {
            continue;
        }
        match prepare_user_input(parsed, agent, extra, &[], "print", None)? {
            PreparedInput::Handled => {}
            PreparedInput::Ready { text, images } => {
                agent.prompt_with(&text, &images);
                let (reply, events) = complete_prompt_with_host(parsed, agent, None, json_mode);
                last_reply = reply;
                all_events.extend(events);
            }
        }
    }
    let (exit_code, error) = print_text_exit(&all_events);
    // A provider failure that the loop gave up on carries no error stop
    // reason of its own: it is the reply text. Report it as the failure it is.
    let (exit_code, error) = match error {
        None if last_reply.starts_with("Provider error: ") => (1, Some(last_reply.clone())),
        other => (exit_code, other),
    };
    if !json_mode {
        if let Some(error) = error {
            eprintln!("{error}");
        } else if !last_reply.is_empty() {
            println!("{last_reply}");
        }
    }
    loaded_extension_host(parsed).emit(ExtensionEvent::SessionShutdown {
        reason: "quit".into(),
    });
    Ok(exit_code)
}

/// TS `toJsonEvent` — strip cumulative `partial` snapshots from `message_update`.
fn to_json_print_event(event: &AgentEvent) -> Result<serde_json::Value, String> {
    match event {
        AgentEvent::MessageUpdate {
            message,
            assistant_message_event,
        } => {
            if message.role != "assistant" {
                return Err("message_update message is not an assistant message".into());
            }
            // The cumulative `partial` snapshot never reaches the wire, so the
            // delta-bearing variants are built directly; serializing the whole
            // partial message first just to remove it made every stream event
            // O(message length). Done/error still serialize fully.
            use pi_ai::AssistantMessageEvent as Ev;
            let mut assistant = match assistant_message_event {
                Ev::Start { .. } => serde_json::json!({"type": "start"}),
                Ev::TextStart { content_index, .. } => {
                    serde_json::json!({"type": "text_start", "contentIndex": content_index})
                }
                Ev::TextDelta {
                    content_index,
                    delta,
                    ..
                } => serde_json::json!(
                    {"type": "text_delta", "contentIndex": content_index, "delta": delta}
                ),
                Ev::TextEnd {
                    content_index,
                    content,
                    ..
                } => serde_json::json!(
                    {"type": "text_end", "contentIndex": content_index, "content": content}
                ),
                Ev::ThinkingStart { content_index, .. } => {
                    serde_json::json!({"type": "thinking_start", "contentIndex": content_index})
                }
                Ev::ThinkingDelta {
                    content_index,
                    delta,
                    ..
                } => serde_json::json!(
                    {"type": "thinking_delta", "contentIndex": content_index, "delta": delta}
                ),
                Ev::ThinkingEnd {
                    content_index,
                    content,
                    ..
                } => serde_json::json!(
                    {"type": "thinking_end", "contentIndex": content_index, "content": content}
                ),
                Ev::ToolcallStart { content_index, .. } => {
                    serde_json::json!({"type": "toolcall_start", "contentIndex": content_index})
                }
                Ev::ToolcallDelta {
                    content_index,
                    delta,
                    ..
                } => serde_json::json!(
                    {"type": "toolcall_delta", "contentIndex": content_index, "delta": delta}
                ),
                Ev::ToolcallEnd {
                    content_index,
                    tool_call,
                    ..
                } => serde_json::json!(
                    {"type": "toolcall_end", "contentIndex": content_index,
                     "toolCall": serde_json::to_value(tool_call).map_err(|err| err.to_string())?}
                ),
                Ev::Done { .. } | Ev::Error { .. } => {
                    serde_json::to_value(assistant_message_event).map_err(|err| err.to_string())?
                }
            };
            if let Some(object) = assistant.as_object_mut() {
                if object.get("type").and_then(serde_json::Value::as_str) == Some("toolcall_start")
                {
                    if let Some(index) = object
                        .get("contentIndex")
                        .and_then(serde_json::Value::as_u64)
                    {
                        let Some(ContentBlock::ToolCall { id, name, .. }) = assistant_message_event
                            .message()
                            .content
                            .get(index as usize)
                        else {
                            return Err("toolcall_start content at index is not a tool call".into());
                        };
                        object.insert("id".into(), serde_json::Value::String(id.clone()));
                        object.insert("toolName".into(), serde_json::Value::String(name.clone()));
                    }
                }
                object.remove("partial");
            }
            Ok(serde_json::json!({
                "type": "message_update",
                "usage": assistant_message_event.message().usage,
                "assistantMessageEvent": assistant,
            }))
        }
        other => serde_json::to_value(other).map_err(|err| err.to_string()),
    }
}

fn print_text_exit(events: &[AgentEvent]) -> (i32, Option<String>) {
    for event in events.iter().rev() {
        let AgentEvent::MessageUpdate {
            assistant_message_event,
            ..
        } = event
        else {
            continue;
        };
        let message = assistant_message_event.message();
        match message.stop_reason {
            Some(StopReason::Error) | Some(StopReason::Aborted) => {
                let label = if message.stop_reason == Some(StopReason::Aborted) {
                    "aborted"
                } else {
                    "error"
                };
                return (
                    1,
                    Some(
                        message
                            .error_message
                            .clone()
                            .unwrap_or_else(|| format!("Request {label}")),
                    ),
                );
            }
            _ => {}
        }
    }
    (0, None)
}

fn rpc_prompt_auth_error(runtime: &RpcRuntime) -> Option<String> {
    if runtime.agent.model_id.is_empty() {
        return Some(format_no_model_selected_message(&coding_agent_docs_dir()));
    }
    let env: std::collections::HashMap<_, _> = std::env::vars().collect();
    let config = ModelConfig::load(&models_json_path(&default_agent_dir()));
    let Ok(storage) = AuthStorage::create() else {
        return Some(format_no_api_key_found_message(
            &runtime.agent.provider,
            &coding_agent_docs_dir(),
        ));
    };
    if check_auth(&runtime.agent.provider, &config, &storage, &env).is_some() {
        return None;
    }
    if storage
        .get(&runtime.agent.provider)
        .is_some_and(|credential| credential.kind == CredentialKind::Oauth)
    {
        return Some(format_oauth_auth_failed_message(&runtime.agent.provider));
    }
    Some(format_no_api_key_found_message(
        &runtime.agent.provider,
        &coding_agent_docs_dir(),
    ))
}

fn run_rpc(parsed: &Args, agent: &mut Agent) -> Result<i32, String> {
    if let Some(code) = immediate_shutdown_if_fixture(parsed) {
        return Ok(code);
    }
    install_mode_shutdown_watchers(parsed);
    let session_dir = agent
        .session
        .as_ref()
        .and_then(|session| session.path.parent())
        .map(|path| path.parent().unwrap_or(path).to_path_buf())
        .unwrap_or_else(pi_session::default_session_dir);
    let cwd = agent.cwd.clone();
    let mut runtime = RpcRuntime::with_models(
        std::mem::replace(agent, Agent::new(default_system_prompt())),
        session_dir,
        cwd,
        available_models(parsed),
    );
    let host = Arc::new(Mutex::new(loaded_extension_host(parsed)));
    host.lock()
        .unwrap_or_else(|err| err.into_inner())
        .emit(ExtensionEvent::SessionStart);
    {
        let host = host.lock().unwrap_or_else(|err| err.into_inner());
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
        append_native_invocable_commands(&mut runtime.invocable_commands);
    }
    runtime.set_scoped_models(&parsed.models);
    let stored = load_settings(&default_agent_dir());
    runtime.default_thinking_level = stored
        .default_thinking_level
        .as_deref()
        .and_then(pi_protocol::ThinkingLevel::parse);
    if let Some(levels) = stored.model_thinking_levels {
        runtime.model_thinking_levels = levels
            .into_iter()
            .filter_map(|(key, value)| {
                pi_protocol::ThinkingLevel::parse(&value).map(|level| (key, level))
            })
            .collect();
    }
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let rx = Arc::new(Mutex::new(rx));
    let leftover = Arc::new(Mutex::new(std::collections::VecDeque::<String>::new()));
    crate::js_host::install_ui_waiter({
        let leftover = leftover.clone();
        let rx = rx.clone();
        Box::new(move |call| rpc_emit_and_wait_ui(call, &leftover, &rx))
    });
    loop {
        let Some(line) = rpc_next_line(&leftover, &rx) else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let mut command: RpcCommand = serde_json::from_str(&line).map_err(|err| err.to_string())?;
        let is_prompt = command.kind == "prompt";
        if is_prompt {
            if let Some(err) = rpc_prompt_auth_error(&runtime) {
                let response = rpc::fail_response(command.id.clone(), "prompt", err);
                output::write_raw_stdout_line(
                    &serde_json::to_string(&response).map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?;
                continue;
            }
            let images = command
                .images
                .as_deref()
                .map(pi_agent::parse_rpc_images)
                .unwrap_or_default();
            let prepared = prepare_user_input(
                parsed,
                &mut runtime.agent,
                command.message.as_deref().unwrap_or(""),
                &images,
                "rpc",
                None,
            )?;
            match prepared {
                PreparedInput::Handled => {
                    let response = rpc::ok_response(command.id.clone(), "prompt", None);
                    output::write_raw_stdout_line(
                        &serde_json::to_string(&response).map_err(|err| err.to_string())?,
                    )
                    .map_err(|err| err.to_string())?;
                    continue;
                }
                PreparedInput::Ready { text, images } => {
                    command.message = Some(text);
                    command.images = Some(
                        images
                            .into_iter()
                            .filter_map(|block| serde_json::to_value(block).ok())
                            .collect(),
                    );
                }
            }
        }
        if command.kind == "bash" {
            let mut locked = host.lock().unwrap_or_else(|err| err.into_inner());
            locked.emit(ExtensionEvent::UserBash {
                command: command.command.clone().unwrap_or_default(),
                exclude_from_context: command.exclude_from_context.unwrap_or(false),
                cwd: runtime.cwd.display().to_string(),
            });
            emit_extension_ui_requests(&std::mem::take(&mut locked.ui_calls))?;
            if let Some(result) = locked.last_user_bash_result() {
                runtime.agent.record_bash_result(
                    command.command.as_deref().unwrap_or(""),
                    &result,
                    command.exclude_from_context.unwrap_or(false),
                );
                let response = rpc::ok_response(command.id.clone(), "bash", Some(result));
                output::write_raw_stdout_line(
                    &serde_json::to_string(&response).map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?;
                continue;
            }
        }
        let response = handle_rpc(&mut runtime, command.clone());
        if matches!(
            command.kind.as_str(),
            "new_session" | "clone" | "fork" | "switch_session"
        ) && response.success
        {
            let mut locked = host.lock().unwrap_or_else(|err| err.into_inner());
            rebind_print_extensions(parsed, &mut runtime.agent, &mut locked);
            runtime.invocable_commands = slash::invocable_commands(
                &locked
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
            append_native_invocable_commands(&mut runtime.invocable_commands);
        }
        let mut extras = runtime.take_events();
        {
            let mut host = host.lock().unwrap_or_else(|err| err.into_inner());
            emit_extension_ui_requests(&std::mem::take(&mut host.ui_calls))?;
        }
        if is_prompt && response.success && runtime.prompt_needs_turn {
            let prompt_args = Args {
                offline: matches!(
                    std::env::var("PI_OFFLINE").as_deref(),
                    Ok("1") | Ok("true") | Ok("yes")
                ),
                extensions: parsed.extensions.clone(),
                no_extensions: parsed.no_extensions,
                ..Args::default()
            };
            let (_reply, events) = complete_prompt_with_host(
                &prompt_args,
                &mut runtime.agent,
                Some(host.clone()),
                false,
            );
            {
                let mut host = host.lock().unwrap_or_else(|err| err.into_inner());
                let remaining: Vec<_> = std::mem::take(&mut host.ui_calls)
                    .into_iter()
                    .filter(|call| !is_dialog_ui_call(call))
                    .collect();
                emit_extension_ui_requests(&remaining)?;
            }
            for event in events {
                output::write_raw_stdout_line(
                    &serde_json::to_string(&event).map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?;
            }
            extras.push(rpc::RpcSessionEvent::AgentSettled);
        }
        for event in extras {
            output::write_raw_stdout_line(
                &serde_json::to_string(&event).map_err(|err| err.to_string())?,
            )
            .map_err(|err| err.to_string())?;
        }
        output::write_raw_stdout_line(
            &serde_json::to_string(&response).map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())?;
    }
    crate::js_host::clear_ui_waiter();
    emit_session_shutdown(parsed);
    *agent = runtime.agent;
    Ok(0)
}

fn is_dialog_ui_call(call: &serde_json::Value) -> bool {
    matches!(
        call.get("op").and_then(|value| value.as_str()),
        Some("select" | "confirm" | "input" | "editor")
    )
}

fn emit_extension_ui_requests(calls: &[serde_json::Value]) -> Result<(), String> {
    for request in rpc::extension_ui_requests_from_calls(calls) {
        output::write_raw_stdout_line(
            &serde_json::to_string(&request).map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn rpc_next_line(
    leftover: &Mutex<std::collections::VecDeque<String>>,
    rx: &Mutex<std::sync::mpsc::Receiver<String>>,
) -> Option<String> {
    if let Ok(mut queue) = leftover.lock() {
        if let Some(line) = queue.pop_front() {
            return Some(line);
        }
    }
    rx.lock().ok()?.recv().ok()
}

fn rpc_emit_and_wait_ui(
    call: &serde_json::Value,
    leftover: &Mutex<std::collections::VecDeque<String>>,
    rx: &Mutex<std::sync::mpsc::Receiver<String>>,
) -> serde_json::Value {
    let requests = rpc::extension_ui_requests_from_calls(std::slice::from_ref(call));
    let default = if call.get("op").and_then(|value| value.as_str()) == Some("confirm") {
        serde_json::Value::Bool(false)
    } else {
        serde_json::Value::Null
    };
    let Some(request) = requests.first() else {
        return default;
    };
    if let Ok(encoded) = serde_json::to_string(request) {
        println!("{encoded}");
        io::stdout().flush().ok();
    }
    let id = request
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let timeout_ms = call.get("timeout").and_then(|value| value.as_u64());
    let deadline =
        timeout_ms.map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));
    let mut parked = std::collections::VecDeque::new();
    loop {
        if deadline.is_some_and(|end| std::time::Instant::now() >= end) {
            if let Ok(mut queue) = leftover.lock() {
                queue.extend(parked);
            }
            return default;
        }
        let remaining = deadline.map_or(std::time::Duration::from_millis(200), |end| {
            end.saturating_duration_since(std::time::Instant::now())
        });
        let line = leftover
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front())
            .or_else(|| {
                rx.lock()
                    .ok()
                    .and_then(|rx| rx.recv_timeout(remaining).ok())
            });
        let Some(line) = line else {
            continue;
        };
        if let Some(value) = parse_rpc_ui_response(&line, &id) {
            if let Ok(mut queue) = leftover.lock() {
                queue.extend(parked);
            }
            return value;
        }
        parked.push_back(line);
    }
}

fn parse_rpc_ui_response(line: &str, id: &str) -> Option<serde_json::Value> {
    let command: RpcCommand = serde_json::from_str(line).ok()?;
    if command.kind != "extension_ui_response" {
        return None;
    }
    if command.id.as_deref() != Some(id) {
        return None;
    }
    if command.cancelled == Some(true) {
        return Some(serde_json::Value::Null);
    }
    if let Some(value) = command.value {
        return Some(serde_json::Value::String(value));
    }
    if let Some(confirmed) = command.confirmed {
        return Some(serde_json::Value::Bool(confirmed));
    }
    Some(serde_json::Value::Null)
}

fn finish_interactive_tui(
    tui_host: Option<(InteractiveTui, ChromePanes)>,
    session: &InteractiveSession,
    stored: &settings::Settings,
    fullscreen: bool,
) {
    if let Some((tui, panes)) = tui_host {
        stop_interactive_tui(
            tui,
            stored
                .fullscreen_exit_output
                .as_deref()
                .unwrap_or("transcript"),
            InteractiveTuiOptions::with_process_terminal(
                TuiMode::Regular,
                session.chrome.theme.clone(),
                stored.show_hardware_cursor.unwrap_or(false),
                default_agent_dir(),
                stored.fullscreen_copy_on_select.unwrap_or(true),
            ),
            |next| remount_chrome_panes(next, &panes),
        );
    } else {
        print!("{}", InteractiveSession::leave_sequences(fullscreen));
    }
}

fn emit_session_shutdown(parsed: &Args) {
    loaded_extension_host(parsed).emit(ExtensionEvent::SessionShutdown {
        reason: "quit".into(),
    });
    crate::js_host::shutdown_js_pool();
}

fn immediate_shutdown_if_fixture(parsed: &Args) -> Option<i32> {
    let code = shutdown::fixture_shutdown_signal()?;
    emit_session_shutdown(parsed);
    Some(code)
}

fn install_mode_shutdown_watchers(parsed: &Args) {
    let parsed = parsed.clone();
    shutdown::install_shutdown_watchers(move |_| {
        emit_session_shutdown(&parsed);
    });
}

/// UI brand: the identity mark follows the installed binary name, so a copy
/// installed as `davinci` presents as davinci while `pi` stays `pi`.
fn ui_brand() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .filter(|stem| !stem.is_empty() && !stem.eq_ignore_ascii_case("pi-coding-agent"))
        .unwrap_or_else(|| APP_NAME.to_string())
}

fn apply_startup_header(session: &mut InteractiveSession, verbose: bool) {
    if session.quiet_startup {
        session.chrome.startup_header = None;
        return;
    }
    let info = pi_tui::StartupInfo {
        cwd: Some(session.cwd.to_string_lossy().into_owned()),
        branch: session.chrome.footer_branch.clone(),
        model: session.current_model().map(str::to_string),
        session_restored: false,
    };
    session.chrome.startup_header = Some(pi_tui::build_startup_header_with(
        &session.chrome.theme,
        &ui_brand(),
        VERSION,
        &session.keybindings,
        verbose || session.chrome.tools_expanded,
        &info,
    ));
}

fn model_scope_startup_line(session: &InteractiveSession) -> Option<String> {
    if session.quiet_startup {
        return None;
    }
    let ids = session
        .enabled_model_ids
        .as_ref()
        .filter(|ids| !ids.is_empty())?;
    let list = ids
        .iter()
        .map(|id| {
            let model = id.rsplit('/').next().unwrap_or(id);
            match session.scoped_thinking_levels.get(id) {
                Some(level) => format!("{model}:{level}"),
                None => model.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let cycle = session.keybindings.keys_for("app.model.cycleForward");
    let hint = if cycle.is_empty() {
        String::new()
    } else {
        session.chrome.theme.fg(
            "muted",
            &format!(
                " ({} to cycle)",
                pi_tui::format_key_text(&cycle.join("/"), true)
            ),
        )
    };
    Some(
        session
            .chrome
            .theme
            .fg("dim", &format!("Model scope: {list}{hint}")),
    )
}

fn resume_command_for_agent(parsed: &Args, agent: &Agent) -> Option<String> {
    let stdout_tty = io::stdout().is_terminal()
        || matches!(
            std::env::var("PI_RESUME_HINT").as_deref(),
            Ok("1") | Ok("true")
        );
    let session = agent.session.as_ref()?;
    let default_dir = pi_session::resolve_session_dir_from(
        parsed.session_dir.as_deref(),
        load_settings(&default_agent_dir()).session_dir.as_deref(),
    );
    let uses_default = parsed.session_dir.is_none()
        && std::env::var("PI_CODING_AGENT_SESSION_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_none();
    let session_dir = session
        .path
        .parent()
        .and_then(|parent| parent.parent())
        .unwrap_or(&default_dir);
    shutdown::format_resume_command(
        stdout_tty,
        true,
        Some(&session.path),
        Some(&session.header.id),
        Some(&session_dir.display().to_string()),
        uses_default,
    )
}

fn print_resume_hint(parsed: &Args, agent: &Agent, session: &InteractiveSession) {
    if let Some(command) = resume_command_for_agent(parsed, agent) {
        println!(
            "{} {command}",
            session.chrome.theme.fg("dim", "To resume this session:")
        );
    }
}

fn dispose_interactive(
    parsed: &Args,
    agent: &Agent,
    tui_host: Option<(InteractiveTui, ChromePanes)>,
    session: &InteractiveSession,
    stored: &settings::Settings,
    fullscreen: bool,
) {
    finish_interactive_tui(tui_host, session, stored, fullscreen);
    emit_session_shutdown(parsed);
    print_resume_hint(parsed, agent, session);
}

fn tui_options_from_session(
    session: &InteractiveSession,
    stored: &settings::Settings,
    mode: TuiMode,
) -> InteractiveTuiOptions {
    InteractiveTuiOptions::with_process_terminal(
        mode,
        session.chrome.theme.clone(),
        stored.show_hardware_cursor.unwrap_or(false),
        default_agent_dir(),
        stored.fullscreen_copy_on_select.unwrap_or(true),
    )
}

/// Diagnostic for `PI_PERF_LOG`: how long one keystroke spent handling input
/// versus rendering, with the document size that produced it.
fn log_key_timing(
    path: &Path,
    input: std::time::Duration,
    render: std::time::Duration,
    session: &InteractiveSession,
) {
    use std::io::Write;
    let line = format!(
        "input_us={} render_us={} transcript_lines={} width={}
",
        input.as_micros(),
        render.as_micros(),
        session.chrome.transcript.lines.len(),
        session.width,
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn sync_hosted_chrome(tui: &mut InteractiveTui, panes: &ChromePanes, session: &InteractiveSession) {
    sync_hosted_chrome_mut(tui, panes, session)
}

/// Move lines swallowed from `println!`/`eprintln!` while the TUI owned the
/// screen into the transcript. Returns true when anything was drained.
fn drain_hosted_lines(session: &mut InteractiveSession) -> bool {
    let pending = HOSTED_PENDING_LINES
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default();
    if pending.is_empty() {
        return false;
    }
    let status = session.chrome.status.clone();
    let mut drained = false;
    for (role, text) in pending {
        if text.is_empty() || text == status {
            continue;
        }
        session.chrome.transcript.push(role, text);
        drained = true;
    }
    drained
}

fn sync_hosted_chrome_mut(
    tui: &mut InteractiveTui,
    panes: &ChromePanes,
    session: &InteractiveSession,
) {
    panes.sync(
        session.chrome.render_document(session.width),
        session.chrome.render_dock(session.width),
    );
    if let Some(title) = session.terminal_title.as_deref() {
        tui.set_title(title);
    }
    tui.invalidate();
    tui.render_now(false);
}

thread_local! {
    /// Chrome panes of the live raw-mode TUI. Set for the lifetime of
    /// `run_raw_session` so the streaming turn can repaint mid-turn without
    /// threading panes through every submit call site.
    static ACTIVE_PANES: std::cell::RefCell<Option<ChromePanes>> =
        const { std::cell::RefCell::new(None) };
}

fn with_active_panes<T>(f: impl FnOnce(Option<&ChromePanes>) -> T) -> T {
    ACTIVE_PANES.with(|slot| f(slot.borrow().as_ref()))
}

/// Work verb for the Studio working line (design spec §5).
fn working_verb(tool_name: &str, args: &serde_json::Value) -> String {
    let target = |key: &str| {
        args.get(key)
            .and_then(serde_json::Value::as_str)
            .map(|value| {
                let line = value.lines().next().unwrap_or("");
                let mut out: String = line.chars().take(48).collect();
                if line.chars().count() > 48 {
                    out.push('…');
                }
                out
            })
            .unwrap_or_default()
    };
    match tool_name {
        "read" | "ls" => format!("studying {}", target("path")),
        "grep" | "find" => format!("surveying \"{}\"", target("pattern")),
        "bash" | "powershell" => format!("testing {}", target("command")),
        "edit" | "write" => format!("constructing {}", target("path")),
        "memory_search" => format!("recalling {}", target("query")),
        name if name.starts_with("graph") => "tracing the graph".into(),
        _ => format!("measuring {tool_name}"),
    }
}

/// Apply one live agent event to the chrome: transcript text, working verb,
/// and tool cards. Used while the worker streams and for the post-join drain.
fn apply_stream_event(
    session: &mut InteractiveSession,
    render_host: &Arc<Mutex<ExtensionHost>>,
    event: &AgentEvent,
    verb: &mut String,
    pushed_assistant: &mut bool,
) {
    match event {
        AgentEvent::ToolExecutionStart {
            tool_name, args, ..
        } => {
            *verb = working_verb(tool_name, args);
        }
        AgentEvent::MessageStart { message } if message.role == "assistant" => {
            *verb = "composing".into();
        }
        AgentEvent::MessageEnd { message } if message.role == "assistant" => {
            let text = content_text(&message.content);
            if !text.trim().is_empty() {
                let text = render_host
                    .try_lock()
                    .map(|h| h.transform_markdown(&text, "assistant", false, 80))
                    .unwrap_or(text);
                session.chrome.transcript.push("assistant", text);
                *pushed_assistant = true;
            }
        }
        _ => {}
    }
    let host_guard = render_host.try_lock().ok();
    apply_tool_events(
        &mut session.chrome,
        std::slice::from_ref(event),
        host_guard.as_deref(),
        session.width,
    );
}

/// Run one agent turn on a worker thread while this thread keeps the TUI
/// alive: live tool lines, a Studio working line with the 4-frame spinner,
/// Esc/Ctrl+C interrupt, and Enter queueing follow-up prompts.
///
/// Returns `(reply, events, queued_prompts)`.
fn run_streaming_turn(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    tui: &mut InteractiveTui,
    panes: &ChromePanes,
    host: Arc<Mutex<ExtensionHost>>,
) -> (String, Vec<AgentEvent>, Vec<String>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentEvent>();
    let abort = Arc::new(AtomicBool::new(false));
    agent.abort_signal = Some(abort.clone());
    agent.event_sink = Some(EventSink(Arc::new(move |event| {
        let _ = event_tx.send(event.clone());
    })));
    let mut queued = Vec::new();
    let mut pushed_assistant = false;
    let mut verb = String::from("thinking");
    let render_host = host.clone();
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(|| complete_prompt_with_host(parsed, agent, Some(host), false));
        let frames = pi_tui::glyphs::SPINNER_FRAMES;
        let mut frame = 0_usize;
        let mut last_spin = std::time::Instant::now();
        let started = std::time::Instant::now();
        loop {
            let mut dirty = false;
            while let Ok(event) = event_rx.try_recv() {
                apply_stream_event(
                    session,
                    &render_host,
                    &event,
                    &mut verb,
                    &mut pushed_assistant,
                );
                dirty = true;
            }
            dirty |= drain_hosted_lines(session);
            if worker.is_finished() {
                break;
            }
            if last_spin.elapsed() >= std::time::Duration::from_millis(250) {
                frame = (frame + 1) % frames.len();
                last_spin = std::time::Instant::now();
                dirty = true;
            }
            let theme = session.chrome.theme.clone();
            let elapsed = started.elapsed().as_secs();
            let hint = if queued.is_empty() {
                "esc interrupt".to_string()
            } else {
                format!("esc interrupt · {} queued", queued.len())
            };
            session.chrome.working_message = Some(format!(
                "{} {} {}",
                theme.fg("primary", frames[frame]),
                theme.fg("text", &verb),
                theme.fg("dim", &format!("· {elapsed}s · {hint}")),
            ));
            if dirty {
                sync_hosted_chrome(tui, panes, session);
            }
            if crossterm::event::poll(std::time::Duration::from_millis(40)).unwrap_or(false) {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key)) => {
                        if key.kind != crossterm::event::KeyEventKind::Press
                            && key.kind != crossterm::event::KeyEventKind::Repeat
                        {
                            continue;
                        }
                        let bytes = key_event_to_bytes(&key);
                        if bytes == "\x1b" || bytes == "\x03" {
                            abort.store(true, Ordering::Relaxed);
                            session.chrome.status =
                                "interrupting · waiting for the current step".into();
                            sync_hosted_chrome(tui, panes, session);
                            continue;
                        }
                        match session.handle_bytes(&bytes) {
                            pi_tui::SessionAction::Submit(text) => {
                                if !text.trim().is_empty() {
                                    session
                                        .chrome
                                        .transcript
                                        .push("system", format!("queued: {text}"));
                                    queued.push(text);
                                }
                            }
                            pi_tui::SessionAction::Quit => {
                                abort.store(true, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                        sync_hosted_chrome(tui, panes, session);
                    }
                    Ok(crossterm::event::Event::Resize(cols, rows)) => {
                        session.width = cols as usize;
                        tui.set_terminal_size(cols as usize, rows as usize);
                        sync_hosted_chrome(tui, panes, session);
                    }
                    _ => {}
                }
            } else {
                tui.tick(40);
            }
        }
        worker.join()
    });
    agent.abort_signal = None;
    agent.event_sink = None;
    session.chrome.working_message = None;
    session.chrome.status.clear();
    let worker_panicked = outcome.is_err();
    let (reply, events) = outcome.unwrap_or_else(|_| {
        (
            String::new(),
            vec![AgentEvent::AgentEnd {
                messages: Vec::new(),
                will_retry: false,
            }],
        )
    });
    // Drain the events that raced the worker's exit — with a short turn
    // (no tools) every event lands here, including the assistant reply.
    while let Ok(event) = event_rx.try_recv() {
        apply_stream_event(
            session,
            &render_host,
            &event,
            &mut verb,
            &mut pushed_assistant,
        );
    }
    if !pushed_assistant {
        let reply = reply.trim();
        if worker_panicked {
            session
                .chrome
                .transcript
                .push("error", "the turn crashed; session state is preserved");
        } else if agent.aborted || abort.load(std::sync::atomic::Ordering::Relaxed) {
            session.chrome.transcript.push("system", "interrupted");
        } else if reply.is_empty() {
            session
                .chrome
                .transcript
                .push("system", "the model returned no text");
        } else if reply.starts_with("Provider error") {
            session.chrome.transcript.push("error", reply);
        } else {
            let text = render_host
                .try_lock()
                .map(|h| h.transform_markdown(reply, "assistant", false, 80))
                .unwrap_or_else(|_| reply.to_string());
            session.chrome.transcript.push("assistant", text);
        }
    }
    drain_hosted_lines(session);
    refresh_chrome_footer(session, agent);
    sync_hosted_chrome(tui, panes, session);
    (reply, events, queued)
}

fn run_interactive(
    parsed: &Args,
    agent: &mut Agent,
    migrated_auth_providers: &[String],
) -> Result<i32, String> {
    if let Some(code) = immediate_shutdown_if_fixture(parsed) {
        return Ok(code);
    }
    install_mode_shutdown_watchers(parsed);
    // The davinci shell is the interactive default; `PI_DAVINCI=0` and
    // `--legacy-tui` opt back out.
    if !parsed.legacy_tui && std::env::var("PI_DAVINCI").as_deref() != Ok("0") {
        let host = Arc::new(Mutex::new(loaded_extension_host(parsed)));
        let raw: Vec<String> = std::env::args().skip(1).collect();
        return davinci_session::run(parsed, agent, &raw, host, migrated_auth_providers);
    }
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
    {
        // Every catalog model stays discoverable: providers without
        // credentials are listed dimmed with a /login hint.
        let snapshot = load_model_runtime(parsed);
        let available: std::collections::BTreeSet<String> = snapshot
            .available
            .iter()
            .map(|model| format!("{}/{}", model.provider, model.id))
            .collect();
        session.locked_model_items = snapshot
            .all
            .iter()
            .filter(|model| !available.contains(&format!("{}/{}", model.provider, model.id)))
            .map(|model| ModelSelectorItem {
                provider: model.provider.clone(),
                id: model.id.clone(),
                name: model.name.clone(),
            })
            .collect();
    }
    if let Some(index) = session
        .models
        .iter()
        .position(|item| item == &format!("{}/{}", agent.provider, agent.model_id))
    {
        session.model_index = index;
    }
    session.cwd = agent.cwd.clone();
    session.slash_commands = interactive_slash_commands(agent, parsed);
    session.extra_autocomplete = interactive_extra_autocomplete(parsed);
    session.login_providers = interactive_login_providers(parsed);
    let stored = load_merged_settings(&default_agent_dir(), &agent.cwd);
    session.double_escape_action =
        DoubleEscapeAction::parse(stored.double_escape_action.as_deref().unwrap_or("tree"));
    session.autocomplete_max_visible =
        stored.autocomplete_max_visible.unwrap_or(5).clamp(3, 20) as usize;
    session
        .chrome
        .editor
        .set_padding_x(stored.editor_padding_x.unwrap_or(0) as usize);
    session.tree_filter_mode =
        FilterMode::parse(stored.tree_filter_mode.as_deref().unwrap_or("default"));
    session.mermaid_mode =
        MermaidMode::parse(stored.markdown.mermaid.as_deref().unwrap_or("streaming"));
    session.chrome.transcript.mermaid_mode = session.mermaid_mode;
    session.chrome.transcript.hide_thinking_block = stored.hide_thinking_block.unwrap_or(false);
    session.chrome.transcript.code_block_indent = stored.code_block_indent().to_string();
    session.enabled_model_ids = stored.enabled_models.clone();
    if !parsed.models.is_empty() {
        let snapshot = load_model_runtime(parsed);
        let scoped = model_resolver::resolve_model_scope_from_models(&parsed.models, &snapshot.all);
        for diagnostic in &scoped.diagnostics {
            eprintln!("Warning: {}", diagnostic.message);
        }
        if !scoped.scoped_models.is_empty() {
            session.enabled_model_ids = Some(
                scoped
                    .scoped_models
                    .iter()
                    .map(|item| format!("{}/{}", item.model.provider, item.model.id))
                    .collect(),
            );
            session.scoped_thinking_levels = scoped
                .scoped_models
                .iter()
                .filter_map(|item| {
                    item.thinking_level.map(|level| {
                        (
                            format!("{}/{}", item.model.provider, item.model.id),
                            level.as_str().to_string(),
                        )
                    })
                })
                .collect();
            if let Some(index) = session
                .models
                .iter()
                .position(|item| item == &format!("{}/{}", agent.provider, agent.model_id))
            {
                session.model_index = index;
            }
        }
    }
    session.default_model = match (&stored.default_provider, &stored.default_model) {
        (Some(provider), Some(id)) => Some(format!("{provider}/{id}")),
        (None, Some(id)) if id.contains('/') => Some(id.clone()),
        _ => None,
    };
    session.keybindings = Keybindings::load(&default_agent_dir());
    session.quiet_startup = stored.quiet_startup && !parsed.verbose;
    session.chrome.quiet_startup = session.quiet_startup;
    session.show_terminal_progress = stored.show_terminal_progress();
    session.warnings_anthropic_extra_usage = stored.warnings.anthropic_extra_usage.unwrap_or(true);
    session.branch_summary_skip_prompt = stored.branch_summary_skip_prompt();
    session.branch_summary_reserve_tokens = stored.branch_summary_reserve_tokens();
    if let Some(levels) = stored.model_thinking_levels.clone() {
        session.model_thinking_levels = levels;
    }
    sync_session_thinking(&mut session, agent);
    let _ = session.begin_osc_query(OSC_QUERY_TIMEOUT_MS);
    let mut host = loaded_extension_host(parsed);
    host.runtime_flag_values = flag_values_json(parsed);
    host.emit(ExtensionEvent::ResourcesDiscover {
        cwd: agent.cwd.display().to_string(),
        reason: "startup".into(),
    });
    host.emit(ExtensionEvent::SessionStart);
    session.apply_extension_ui_calls(&host.ui_calls);
    apply_extension_shortcuts(parsed, &mut session, agent, &host);
    session.slash_commands = interactive_slash_commands(agent, parsed);
    session.extra_autocomplete = interactive_extra_autocomplete(parsed);
    replay_custom_messages(agent, &mut session, &host);
    let _ = FALLBACK_PREVIEW_LINES;
    session.chrome.status = catalog_refresh::refresh_status_refreshing().into();
    start_catalog_refresh_async(parsed);
    apply_startup_notices(
        &mut session,
        &stored,
        models_json_error,
        migrated_auth_providers,
    );
    show_loaded_resources(&mut session, agent, &host, parsed);
    refresh_chrome_footer(&mut session, agent);
    session.chrome.transcript.agent_label = ui_brand();
    apply_startup_header(&mut session, parsed.verbose);
    if let Some(line) = model_scope_startup_line(&session) {
        println!("{line}");
    }
    apply_project_trust_warning(&mut session, parsed, agent);
    apply_changelog_overlay(&mut session, agent, &stored, &default_agent_dir());
    refresh_chrome_footer(&mut session, agent);
    apply_cache_miss_notices(
        &mut session.chrome,
        agent,
        stored.show_cache_miss_notices.unwrap_or(false),
    );
    if should_run_first_time_setup(&settings_path(&default_agent_dir())) {
        session.open_first_time_setup(&detect_terminal_theme(&session.chrome.theme), APP_NAME);
    }
    let tui_mode = parsed
        .tui_mode
        .or_else(|| TuiMode::parse(stored.tui_mode.as_deref().unwrap_or("regular")))
        .unwrap_or(TuiMode::Regular);
    let fullscreen = tui_mode == TuiMode::Fullscreen;
    let use_tui_host = io::stdin().is_terminal();
    let mut tui_host = if use_tui_host {
        if let Ok((cols, _rows)) = crossterm::terminal::size() {
            session.width = cols as usize;
        }
        let panes = ChromePanes::new(
            session.chrome.render_document(session.width),
            session.chrome.render_dock(session.width),
        );
        let options = InteractiveTuiOptions::with_process_terminal(
            tui_mode,
            session.chrome.theme.clone(),
            stored.show_hardware_cursor.unwrap_or(false),
            default_agent_dir(),
            stored.fullscreen_copy_on_select.unwrap_or(true),
        );
        let mut tui = create_interactive_tui(options);
        tui.set_clear_on_shrink(stored.clear_on_shrink());
        remount_chrome_panes(&mut tui, &panes);
        if let Ok((cols, rows)) = crossterm::terminal::size() {
            tui.set_terminal_size(cols as usize, rows as usize);
        }
        tui.start();
        ensure_interactive_tools(&mut session);
        apply_terminal_title(&mut session, agent, Some(&mut tui));
        Some((tui, panes))
    } else {
        print!("{}", InteractiveSession::enter_sequences(fullscreen));
        println!("{}", session.chrome.render(session.width).join("\n"));
        apply_terminal_title(&mut session, agent, None);
        None
    };
    let prepared = prepare_initial_message(
        &parsed.messages,
        &parsed.file_args,
        None,
        &agent.cwd,
        stored.image_auto_resize(),
    )?;
    if let Some(prompt) = &prepared.text {
        if !submit_user_message(
            parsed,
            agent,
            &mut session,
            prompt,
            &prepared.images,
            tui_host.as_mut().map(|(tui, _)| tui),
        )? {
            dispose_interactive(
                parsed,
                agent,
                tui_host.take(),
                &session,
                &stored,
                fullscreen,
            );
            return Ok(0);
        }
        if let Some((tui, panes)) = &mut tui_host {
            sync_hosted_chrome(tui, panes, &session);
        }
    }
    for extra in &prepared.remaining_messages {
        if !submit_user_message(
            parsed,
            agent,
            &mut session,
            extra,
            &[],
            tui_host.as_mut().map(|(tui, _)| tui),
        )? {
            dispose_interactive(
                parsed,
                agent,
                tui_host.take(),
                &session,
                &stored,
                fullscreen,
            );
            return Ok(0);
        }
        if let Some((tui, panes)) = &mut tui_host {
            sync_hosted_chrome(tui, panes, &session);
        }
    }
    if let Some((tui, panes)) = tui_host.take() {
        run_raw_session(parsed, agent, &mut session, &mut host, tui, panes, &stored)
    } else {
        let result = run_line_session(parsed, agent, &mut session);
        print!("{}", InteractiveSession::leave_sequences(fullscreen));
        emit_session_shutdown(parsed);
        print_resume_hint(parsed, agent, &session);
        result
    }
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
            KeyCode::Char('z') | KeyCode::Char('Z') => "\x1a".into(),
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
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            pi_tui::NATIVE_SHIFT_ENTER_SEQUENCE.into()
        }
        KeyCode::Enter => "\r".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::Backspace => "\x7f".into(),
        KeyCode::Up => "\x1b[A".into(),
        KeyCode::Down => "\x1b[B".into(),
        KeyCode::Left => "\x1b[D".into(),
        KeyCode::Right => "\x1b[C".into(),
        KeyCode::PageUp => "\x1b[5~".into(),
        KeyCode::PageDown => "\x1b[6~".into(),
        KeyCode::Home => "\x1bOH".into(),
        KeyCode::End => "\x1bOF".into(),
        KeyCode::Char(ch) => ch.to_string(),
        _ => String::new(),
    }
}

fn run_raw_session(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    host: &mut ExtensionHost,
    mut tui: InteractiveTui,
    panes: ChromePanes,
    stored: &settings::Settings,
) -> Result<i32, String> {
    let _raw = RawModeGuard::enter()?;
    ACTIVE_PANES.with(|slot| *slot.borrow_mut() = Some(panes.clone()));
    HOSTED_TUI_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
    struct PanesGuard;
    impl Drop for PanesGuard {
        fn drop(&mut self) {
            ACTIVE_PANES.with(|slot| *slot.borrow_mut() = None);
            HOSTED_TUI_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let _panes_guard = PanesGuard;
    let mut stored = stored.clone();
    // Gate reloads on the settings file's mtime; a full read+parse on every
    // input event is wasted work per keystroke (worse on network home dirs).
    // `PI_PERF_LOG=<file>` records per-keystroke input and render timings.
    let perf_log = std::env::var_os("PI_PERF_LOG").map(PathBuf::from);
    let settings_path = default_agent_dir().join("settings.json");
    let mut settings_stamp = std::fs::metadata(&settings_path)
        .and_then(|meta| meta.modified())
        .ok();
    loop {
        if session.osc_query_pending() {
            if let Some(reply) = drain_osc_tty(OSC_QUERY_TIMEOUT_MS) {
                if !tui.handle_host_input(&reply) {
                    let action = session.handle_bytes(&reply);
                    if !apply_session_action(parsed, agent, session, action, Some(&mut tui))? {
                        break;
                    }
                }
                sync_hosted_chrome(&mut tui, &panes, session);
            }
        }
        if let Some(detection) = session.finish_osc_query(std::time::Instant::now()) {
            apply_osc_theme(session, &detection);
            sync_hosted_chrome(&mut tui, &panes, session);
        }
        if !crossterm::event::poll(std::time::Duration::from_millis(50))
            .map_err(|err| err.to_string())?
        {
            tui.tick(50);
            let mut dirty = poll_llama_job(session);
            dirty |= tick_custom_overlay(parsed, session);
            dirty |= poll_catalog_refresh(parsed, session);
            dirty |= drain_hosted_lines(session);
            if dirty {
                sync_hosted_chrome(&mut tui, &panes, session);
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
                let bytes = key_event_to_bytes(&key);
                let perf_started = perf_log.is_some().then(std::time::Instant::now);
                if host.dispatch_terminal_input(&bytes) {
                    session.apply_extension_ui_calls(&host.ui_calls);
                    sync_hosted_chrome(&mut tui, &panes, session);
                    continue;
                }
                if tui.handle_host_input(&bytes) {
                    sync_hosted_chrome(&mut tui, &panes, session);
                    continue;
                }
                let action = session.handle_bytes(&bytes);
                if matches!(action, SessionAction::Suspend) {
                    tui.stop(pi_tui::TuiStopOptions {
                        preserve_screen: true,
                    });
                    apply_suspend(session, true);
                    tui.start();
                    tui.request_render(true);
                    sync_hosted_chrome(&mut tui, &panes, session);
                    continue;
                }
                let perf_after_input = perf_log.is_some().then(std::time::Instant::now);
                if !apply_session_action(parsed, agent, session, action, Some(&mut tui))? {
                    break;
                }
                if let (Some(path), Some(started), Some(after_input)) =
                    (perf_log.as_ref(), perf_started, perf_after_input)
                {
                    let render_started = std::time::Instant::now();
                    sync_hosted_chrome(&mut tui, &panes, session);
                    log_key_timing(
                        path,
                        after_input.duration_since(started),
                        render_started.elapsed(),
                        session,
                    );
                }
            }
            crossterm::event::Event::Mouse(mouse) => {
                if !tui.is_viewport_tui() {
                    session.chrome.apply_mouse(mouse.row, session.width);
                }
            }
            crossterm::event::Event::Paste(text) => {
                let paste = format!("\x1b[200~{text}\x1b[201~");
                if !tui.handle_host_input(&paste) {
                    session.handle_bytes(&paste);
                }
            }
            crossterm::event::Event::Resize(cols, rows) => {
                session.width = cols as usize;
                tui.set_terminal_size(cols as usize, rows as usize);
            }
            _ => {}
        }
        let stamp = std::fs::metadata(&settings_path)
            .and_then(|meta| meta.modified())
            .ok();
        if stamp != settings_stamp {
            settings_stamp = stamp;
            stored = load_settings(&default_agent_dir());
        }
        let desired = TuiMode::parse(stored.tui_mode.as_deref().unwrap_or("regular"))
            .unwrap_or(TuiMode::Regular);
        if desired != tui.product_mode() {
            let (next, switched) = switch_tui_mode(
                tui,
                desired,
                tui_options_from_session(session, &stored, desired),
                true,
            );
            tui = next;
            if switched {
                remount_chrome_panes(&mut tui, &panes);
            }
        }
        if session.show_terminal_progress {
            tui.set_progress(true);
        }
        drain_hosted_lines(session);
        sync_hosted_chrome(&mut tui, &panes, session);
    }
    dispose_interactive(parsed, agent, Some((tui, panes)), session, &stored, false);
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
        if !apply_session_action(parsed, agent, session, action, None)? {
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
    tui: Option<&mut InteractiveTui>,
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
            refresh_chrome_footer(session, agent);
            apply_terminal_title(session, agent, tui);
            Ok(true)
        }
        SessionAction::SelectSetting(value) => {
            apply_interactive_setting(session, &value)?;
            sync_agent_from_settings(agent);
            Ok(true)
        }
        SessionAction::OpenSettingsSubmenu => Ok(true),
        SessionAction::ApplySetting { id, value } => {
            apply_interactive_setting(session, &format!("{id}={value}"))?;
            sync_agent_from_settings(agent);
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
            match host_invoke_shortcut(parsed, agent, session, &path, &key) {
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
            handle_custom_overlay_input(parsed, agent, session, &data);
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
        SessionAction::OpenLogin => {
            show_login_auth_type_selector(session, None);
            Ok(true)
        }
        SessionAction::SelectAuthProvider {
            provider,
            auth_type,
        } => {
            if session.auth_selector_logout {
                session.auth_selector_logout = false;
                handle_logout_command(session, Some(&provider))
            } else {
                start_provider_login(session, &provider, &auth_type, None)?;
                Ok(true)
            }
        }
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
        SessionAction::OpenFork => handle_user_line(parsed, agent, session, "/fork", tui),
        SessionAction::RunBash {
            command,
            exclude_from_context,
        } => {
            let mut host = loaded_extension_host(parsed);
            host.runtime_flag_values = flag_values_json(parsed);
            host.emit(ExtensionEvent::UserBash {
                command: command.clone(),
                exclude_from_context,
                cwd: agent.cwd.display().to_string(),
            });
            session.apply_extension_ui_calls(&host.ui_calls);
            apply_host_session_calls(parsed, agent, session, &host.session_calls, true);
            if let Some(result) = host.last_user_bash_result() {
                agent.record_bash_result(&command, &result, exclude_from_context);
                let output = result
                    .get("output")
                    .or_else(|| result.get("content"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                session.chrome.transcript.push("bash", output);
                session.chrome.status = "bash done".into();
                if !output.is_empty() {
                    println!("{output}");
                }
                return Ok(true);
            }
            match pi_agent::execute_tool(
                &agent.cwd,
                "bash",
                &serde_json::json!({ "command": command }),
            ) {
                Ok(result) => {
                    let value = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
                    agent.record_bash_result(&command, &value, exclude_from_context);
                    session.chrome.transcript.push("bash", &result.content);
                    session.chrome.status = if result.is_error {
                        "bash error".into()
                    } else {
                        "bash done".into()
                    };
                    if result.is_error {
                        eprintln!("{}", result.content);
                    } else {
                        println!("{}", result.content);
                    }
                }
                Err(err) => {
                    session.chrome.transcript.push("bash", err.to_string());
                    session.chrome.status = "bash error".into();
                    eprintln!("{err}");
                }
            }
            Ok(true)
        }
        SessionAction::Suspend => {
            apply_suspend(session, tui.is_some());
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
                apply_model_switch_thinking(agent, session, &provider, &model_id);
                agent.provider = provider;
                agent.model_id = model_id;
                sync_session_thinking(session, agent);
            }
            Ok(true)
        }
        SessionAction::CycleThinking => {
            sync_session_thinking(session, agent);
            if session.supports_thinking {
                if let Some(level) = pi_protocol::ThinkingLevel::parse(session.current_thinking()) {
                    agent.thinking_level = level;
                }
            }
            Ok(true)
        }
        SessionAction::OpenThinking => {
            sync_session_thinking(session, agent);
            session.open_thinking_selector(
                load_settings(&default_agent_dir())
                    .default_thinking_level
                    .as_deref(),
            );
            Ok(true)
        }
        SessionAction::SelectTrust { trusted, updates } => {
            apply_trust_decision(session, trusted, &updates)?;
            Ok(true)
        }
        SessionAction::SelectThinking(level) => apply_thinking_level(agent, session, &level, false),
        SessionAction::SelectThinkingAsDefault(level) => {
            apply_thinking_level(agent, session, &level, true)
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
        SessionAction::ExpandTools => {
            session.chrome.status = format!(
                "Tool output: {}",
                if session.chrome.tools_expanded {
                    "expanded"
                } else {
                    "collapsed"
                }
            );
            Ok(true)
        }
        SessionAction::NewSession => handle_user_line(parsed, agent, session, "/new", tui),
        SessionAction::OpenResume => {
            open_session_selector(parsed, agent, session)?;
            Ok(true)
        }
        SessionAction::Clear => Ok(true),
        SessionAction::SelectModel(value) => {
            if session
                .locked_model_items
                .iter()
                .any(|item| item.key() == value)
            {
                let provider = value.split('/').next().unwrap_or("provider").to_string();
                session.chrome.status = format!("login required · /login {provider}");
                session.chrome.transcript.push(
                    "system",
                    format!("{value} needs credentials: /login {provider}"),
                );
                return Ok(true);
            }
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
                handle_login_command(session, &provider, key.as_deref())?;
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
                handle_user_line(parsed, agent, session, &text, tui)
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
                handle_user_line(parsed, agent, session, &text, tui)
            }
            _ => handle_user_line(parsed, agent, session, &text, tui),
        },
    }
}

fn unknown_thinking_error(search: &str) -> String {
    let levels = pi_protocol::ThinkingLevel::all()
        .iter()
        .map(|level| level.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("Unknown thinking level \"{search}\". Available levels: {levels}.")
}

fn apply_thinking_level(
    agent: &mut Agent,
    session: &mut InteractiveSession,
    level: &str,
    persist: bool,
) -> Result<bool, String> {
    let Some(parsed) = pi_protocol::ThinkingLevel::parse(level) else {
        let message = unknown_thinking_error(level);
        session.chrome.status = message.clone();
        eprintln!("{message}");
        return Ok(true);
    };
    agent.thinking_level = parsed;
    if let Some(index) = session
        .thinking_levels
        .iter()
        .position(|item| item == level)
    {
        session.thinking_index = index;
    }
    if persist {
        let dir = default_agent_dir();
        let mut stored = load_settings(&dir);
        stored.default_thinking_level = Some(level.to_string());
        save_settings(&dir, &stored)?;
        session.chrome.status = format!("Default thinking level: {level}");
    } else {
        session.chrome.status = format!("Thinking level: {level}");
    }
    println!("{}", session.chrome.status);
    Ok(true)
}

enum PreparedInput {
    Handled,
    Ready {
        text: String,
        images: Vec<pi_ai::MessageContent>,
    },
}

/// TS AgentSession.prompt preflight: extension command, input transform/handled, skill/template expand.
fn prepare_user_input(
    parsed: &Args,
    agent: &mut Agent,
    prompt: &str,
    images: &[pi_ai::MessageContent],
    source: &str,
    mut session: Option<&mut InteractiveSession>,
) -> Result<PreparedInput, String> {
    let mut text = prompt.to_string();
    let mut images = images.to_vec();
    if text.starts_with('/') {
        let (name, args) = parse_extension_command(&text);
        if let Some(session) = session.as_mut() {
            if try_extension_slash(parsed, agent, session, &name, &args)? {
                return Ok(PreparedInput::Handled);
            }
        } else {
            let mut host = loaded_extension_host(parsed);
            apply_graph_session_context(parsed, agent, &host);
            if let Some(result) = host.execute_native_command(&name, &args)? {
                println!("/{name}: {}", format_extension_command_result(result));
                return Ok(PreparedInput::Handled);
            }
            if let Some(path) = host
                .js
                .iter()
                .find(|ext| ext.commands.iter().any(|command| command == &name))
                .map(|ext| ext.path.clone())
            {
                host.runtime_flag_values = flag_values_json(parsed);
                let _ = host.invoke_command(&path, &name);
                return Ok(PreparedInput::Handled);
            }
        }
    }
    let mut host = loaded_extension_host(parsed);
    host.runtime_flag_values = flag_values_json(parsed);
    if let Some(session) = session.as_ref() {
        host.editor_text = session.chrome.editor.get_text().to_string();
    }
    let input = host.emit_input(&text, &images, source);
    if let Some(session) = session.as_mut() {
        session.apply_extension_ui_calls(&host.ui_calls);
        apply_host_session_calls(parsed, agent, session, &host.session_calls, true);
    }
    if input.action == "handled" {
        return Ok(PreparedInput::Handled);
    }
    if input.action == "transform" {
        text = input.text;
        images = input.images;
    }
    text = pi_agent::expand_user_text(&text, &agent.skills, &agent.templates);
    Ok(PreparedInput::Ready { text, images })
}

fn submit_user_message(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    prompt: &str,
    images: &[pi_ai::MessageContent],
    mut tui: Option<&mut InteractiveTui>,
) -> Result<bool, String> {
    let prepared = prepare_user_input(parsed, agent, prompt, images, "interactive", Some(session))?;
    let PreparedInput::Ready { text, images } = prepared else {
        return Ok(true);
    };
    session.chrome.transcript.push("user", &text);
    agent.prompt_with(&text, &images);
    // Inside the raw-mode TUI the turn runs on a worker thread so the
    // interface keeps painting (spinner, live tool lines, Esc interrupt).
    let streaming = with_active_panes(|panes| panes.cloned());
    let queued = match (streaming, tui.as_deref_mut()) {
        (Some(panes), Some(tui)) => {
            let host = Arc::new(Mutex::new(loaded_extension_host(parsed)));
            let (_, events, queued) = run_streaming_turn(parsed, agent, session, tui, &panes, host);
            apply_progress_events(session, &events);
            Some(queued)
        }
        _ => None,
    };
    match queued {
        Some(queued) => {
            refresh_chrome_footer(session, agent);
            session.chrome.editor.handle_input("");
            for follow_up in queued {
                if !submit_user_message(
                    parsed,
                    agent,
                    session,
                    &follow_up,
                    &[],
                    tui.as_deref_mut(),
                )? {
                    return Ok(false);
                }
            }
        }
        None => {
            let (reply, events) = complete_prompt(parsed, agent);
            let host = loaded_extension_host(parsed);
            apply_tool_events(&mut session.chrome, &events, Some(&host), session.width);
            apply_progress_events(session, &events);
            let reply = host.transform_markdown(&reply, "assistant", false, 80);
            session.chrome.transcript.push("assistant", &reply);
            apply_cache_miss_notices(
                &mut session.chrome,
                agent,
                load_merged_settings(&default_agent_dir(), &agent.cwd)
                    .show_cache_miss_notices
                    .unwrap_or(false),
            );
            refresh_chrome_footer(session, agent);
            session.chrome.editor.handle_input("");
            println!("{reply}");
        }
    }
    Ok(true)
}

fn handle_user_line(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    text: &str,
    tui: Option<&mut InteractiveTui>,
) -> Result<bool, String> {
    match slash::parse_line(text) {
        SlashAction::Quit => Ok(false),
        SlashAction::Prompt(prompt) => {
            submit_user_message(parsed, agent, session, &prompt, &[], tui)
        }
        SlashAction::OpenThinking => {
            sync_session_thinking(session, agent);
            session.open_thinking_selector(
                load_settings(&default_agent_dir())
                    .default_thinking_level
                    .as_deref(),
            );
            session.chrome.status = "Select thinking level".into();
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::Status(message) => {
            if let Some(name) = message.strip_prefix("Unknown command /") {
                if try_extension_slash(parsed, agent, session, name, "")? {
                    return Ok(true);
                }
            }
            session.chrome.status = message.clone();
            println!("{message}");
            Ok(true)
        }
        SlashAction::Hotkeys => {
            let keys = pi_tui::get_keybindings()
                .into_iter()
                .map(|b| format!("{}: {}", b.action, b.keys.join(", ")))
                .collect::<Vec<_>>()
                .join("\n");
            session.chrome.status = keys.clone();
            println!("{keys}");
            Ok(true)
        }
        SlashAction::Settings => {
            let stored = load_settings(&default_agent_dir());
            let list = interactive_settings_list(&to_interactive_config(
                &stored,
                &session.chrome.theme.name,
            ));
            session.chrome.settings_list = Some(list);
            session.chrome.settings_submenu = None;
            session.chrome.status = "Settings".into();
            if let Some(settings) = &session.chrome.settings_list {
                println!("{}", settings.render(80).join("\n"));
            }
            Ok(true)
        }
        SlashAction::SessionInfo => {
            let models = load_builtin_models();
            let model = find_model(&models, &agent.provider, &agent.model_id);
            let stats = rpc::session_stats_for_agent(agent, model);
            let waste = agent
                .session
                .as_ref()
                .map(|store| cache_stats::compute_cache_waste(&store.entries, &0.3));
            let info = format_session_info(&stats, waste.as_ref());
            session.chrome.status = info.clone();
            println!("{info}");
            Ok(true)
        }
        SlashAction::NewSession => {
            let session_dir = resolved_session_dir(parsed, &agent.cwd);
            let store = JsonlSession::create(&session_dir, &agent.cwd.to_string_lossy(), None)
                .map_err(|err| err.to_string())?;
            agent.messages.clear();
            agent.session = Some(store);
            println!("Started new session");
            refresh_chrome_footer(session, agent);
            apply_terminal_title(session, agent, tui);
            Ok(true)
        }
        SlashAction::Compact(instructions) => {
            let mut host = loaded_extension_host(parsed);
            host.runtime_flag_values = flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionBeforeCompact);
            if host.last_result_cancelled() {
                session.chrome.status = "Compaction cancelled".into();
                println!("{}", session.chrome.status);
                return Ok(true);
            }
            let result = agent.compact(instructions.as_deref());
            if result.compacted {
                host.emit(ExtensionEvent::SessionCompact);
            } else {
                host.emit(ExtensionEvent::SessionCompactFailed {
                    error: result.summary.clone(),
                });
            }
            println!("{}", result.summary);
            Ok(true)
        }
        SlashAction::OpenModel => {
            session.chrome.selector = Some(pi_tui::SelectList::new(
                load_builtin_models()
                    .into_iter()
                    .map(|model| format!("{}/{}", model.provider, model.id))
                    .collect(),
            ));
            session.chrome.status = "Select model".into();
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::SetModel(value) => {
            let (provider, model_id) = parse_model_ref("google", Some(&value));
            agent.provider = provider;
            agent.model_id = model_id;
            loaded_extension_host(parsed).emit(ExtensionEvent::ModelSelect {
                provider: agent.provider.clone(),
                model: agent.model_id.clone(),
            });
            println!("model={}/{}", agent.provider, agent.model_id);
            Ok(true)
        }
        SlashAction::SetThinking(level) => {
            let result = apply_thinking_level(agent, session, &level, false);
            loaded_extension_host(parsed).emit(ExtensionEvent::ThinkingLevelSelect {
                level: level.clone(),
            });
            result
        }
        SlashAction::Export(path) => {
            if let Some(session) = &agent.session {
                let output = PathBuf::from(path.unwrap_or_else(|| "session.html".into()));
                println!("{}", export::export_session(session, &output)?);
            }
            Ok(true)
        }
        SlashAction::Login { provider, key } => {
            handle_login_command(session, &provider, key.as_deref())?;
            Ok(true)
        }
        SlashAction::Logout { provider } => handle_logout_command(session, provider.as_deref()),
        SlashAction::Name(name) => {
            if let Some(store) = agent.session.as_mut() {
                store.set_name(&name).map_err(|e| e.to_string())?;
            }
            refresh_chrome_footer(session, agent);
            apply_terminal_title(session, agent, tui);
            Ok(true)
        }
        SlashAction::Fork => {
            let mut host = loaded_extension_host(parsed);
            host.runtime_flag_values = flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionBeforeFork);
            if host.last_result_cancelled() {
                session.chrome.status = "Fork cancelled".into();
                println!("{}", session.chrome.status);
                return Ok(true);
            }
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
            refresh_chrome_footer(session, agent);
            apply_terminal_title(session, agent, tui);
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
            refresh_chrome_footer(session, agent);
            apply_terminal_title(session, agent, tui);
            Ok(true)
        }
        SlashAction::ScopedModels => {
            session.chrome.status = "Model Configuration".into();
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::Resume => {
            let items = discover_session_items(parsed, agent)?;
            let mut selector = pi_tui::SessionSelector::new(items.clone());
            selector.set_cwd(agent.cwd.to_string_lossy().into_owned());
            session.chrome.session_selector = Some(selector);
            session.chrome.selector = None;
            session.chrome.status = "Select session".into();
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
            let mut host = loaded_extension_host(parsed);
            host.runtime_flag_values = flag_values_json(parsed);
            host.emit(ExtensionEvent::SessionBeforeTree);
            if host.last_result_cancelled() {
                session.chrome.status = "Tree navigation cancelled".into();
                println!("{}", session.chrome.status);
                return Ok(true);
            }
            host.emit(ExtensionEvent::UiPromptStart {
                kind: "tree".into(),
            });
            host.emit(ExtensionEvent::SessionTree);
            host.emit(ExtensionEvent::UiPromptEnd {
                kind: "tree".into(),
            });
            session.chrome.status = "Session Tree".into();
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::Copy => {
            if let Some(tui) = tui {
                match handle_copy_command(tui, agent.last_assistant_text().as_deref(), true, true) {
                    CopyCommandResult::CopiedSelection => {}
                    CopyCommandResult::CopiedAssistant => {
                        if !tui.is_viewport_tui() {
                            session.chrome.status = "Copied last agent message to clipboard".into();
                        }
                    }
                    CopyCommandResult::NoAssistant => {
                        session.chrome.status = "No agent messages to copy yet.".into();
                    }
                    CopyCommandResult::Failed(err) => {
                        session.chrome.status = err;
                    }
                }
            } else if let Some(text) = agent.last_assistant_text() {
                println!("{text}");
            }
            Ok(true)
        }
        SlashAction::Trust => {
            let mut host = loaded_extension_host(parsed);
            host.emit(ExtensionEvent::UiPromptStart {
                kind: "trust".into(),
            });
            host.emit(ExtensionEvent::ProjectTrust {
                path: agent.cwd.display().to_string(),
            });
            open_trust_selector(agent, session);
            Ok(true)
        }
        SlashAction::Reload => {
            reload_interactive_resources(parsed, agent, session);
            session.chrome.status =
                "Reloaded keybindings, extensions, skills, prompts, themes, and context files"
                    .into();
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::Import(path) => {
            if path.is_empty() {
                session.chrome.status = "Usage: /import <path.jsonl>".into();
                println!("{}", session.chrome.status);
                return Ok(true);
            }
            let expanded = pi_session::expand_tilde(&path);
            let next = JsonlSession::open(&expanded).map_err(|err| err.to_string())?;
            agent.load_from_session(next);
            session.chrome.status = format!(
                "imported {}",
                agent
                    .session
                    .as_ref()
                    .map(|item| item.header.id.clone())
                    .unwrap_or_default()
            );
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::Share => {
            session.chrome.status = share_current_session(agent)?;
            println!("{}", session.chrome.status);
            Ok(true)
        }
        SlashAction::Changelog => {
            let entries = changelog::parse_changelog(&changelog::changelog_path());
            let stored = load_merged_settings(&default_agent_dir(), &agent.cwd);
            let text = match stored.last_changelog_version.as_deref() {
                Some(since) => changelog::format_changelog_since(&entries, Some(since)),
                None => changelog::format_changelog(&entries),
            };
            session.chrome.transcript.push("changelog", &text);
            session.chrome.status = "changelog".into();
            println!("{text}");
            Ok(true)
        }
        SlashAction::Llama => {
            session.chrome.status = "llama.cpp is available in interactive mode".into();
            println!("{}", session.chrome.status);
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
    apply_discovered_resources(parsed, agent);
    let mut host = loaded_extension_host(parsed);
    host.runtime_flag_values = flag_values_json(parsed);
    host.emit(ExtensionEvent::SessionStart);
    apply_extension_shortcuts(parsed, session, agent, &host);
    session.slash_commands = interactive_slash_commands(agent, parsed);
    session.extra_autocomplete = interactive_extra_autocomplete(parsed);
    replay_custom_messages(agent, session, &host);
    let current = session.chrome.theme.name.clone();
    apply_theme_value(session, &current);
    show_loaded_resources(session, agent, &host, parsed);
    apply_startup_header(session, parsed.verbose);
}

fn available_themes_with(parsed: Option<&Args>) -> Vec<Theme> {
    let mut themes = load_themes_from_dir(&default_agent_dir().join("themes"));
    if let Some(parsed) = parsed {
        for path in &parsed.themes {
            themes.extend(load_themes_from_dir(Path::new(path)));
        }
    }
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

fn available_themes() -> Vec<Theme> {
    available_themes_with(None)
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

fn apply_suspend(session: &mut InteractiveSession, hosted: bool) {
    if cfg!(windows) {
        session.chrome.status = "Suspend to background is not supported on Windows".into();
        return;
    }
    if std::env::var("PI_SUSPEND_DRY_RUN").is_ok() {
        session.chrome.status = "Suspended".into();
        return;
    }
    if !hosted {
        print!("{}", InteractiveSession::leave_sequences(false));
        let _ = io::stdout().flush();
    }
    let _ = std::process::Command::new("kill")
        .args(["-TSTP", "0"])
        .status();
    if !hosted {
        print!("{}", InteractiveSession::enter_sequences(false));
        let _ = io::stdout().flush();
    }
}

fn apply_tool_events(
    chrome: &mut ChatChrome,
    events: &[AgentEvent],
    host: Option<&ExtensionHost>,
    width: usize,
) {
    for event in events {
        match event {
            AgentEvent::AgentStart => {}
            AgentEvent::AgentEnd { .. } => {}
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let mut card = ToolCard::start(tool_name, tool_call_id, args.clone());
                if let Some(host) = host {
                    if let Some((_, tool)) = host.js_tool(tool_name) {
                        card.render_shell = tool.render_shell.clone();
                    }
                    card.call_lines = host.render_tool_call_lines(tool_name, args, width);
                }
                chrome.tool_cards.push(card);
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
                // The live card becomes a permanent one-line transcript entry
                // (spec §6 ToolCall); keeping the card too would render the
                // whole tool history twice, once inline and once at the dock.
                let finished = chrome
                    .tool_cards
                    .iter()
                    .position(|card| card.tool_call_id == *tool_call_id)
                    .map(|index| {
                        let mut card = chrome.tool_cards.remove(index);
                        card.finish(result, *is_error);
                        if let Some(host) = host {
                            card.result_lines =
                                host.render_tool_result_lines(&card.tool_name, result, width);
                        }
                        card
                    });
                if let Some(card) = finished {
                    let base_id = chrome.tool_cards.len() as u32;
                    for (index, (data, _)) in card.image_payloads().into_iter().enumerate() {
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
                    let mut block = card.summary_block();
                    if !card.result_lines.is_empty() {
                        for line in &card.result_lines {
                            block.push_str("\n  ");
                            block.push_str(line);
                        }
                    } else if chrome.tools_expanded {
                        for line in card.format_tool_execution().lines().take(40) {
                            block.push_str("\n    ");
                            block.push_str(line);
                        }
                    }
                    chrome.transcript.push("tool", block);
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
        "editor-padding" => {
            if let Ok(n) = value.parse::<usize>() {
                session.chrome.editor.set_padding_x(n.min(3));
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
    persist_interactive_setting(spec)
}

/// The stored half of a settings change: write `id=value` into
/// `settings.json`. Shared by the legacy overlay and the davinci sheet, so
/// both write the same keys the same way.
fn persist_interactive_setting(spec: &str) -> Result<(), String> {
    let (id, value) = spec.split_once('=').unwrap_or((spec, ""));
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
        "autocompact-threshold" => {
            if value.eq_ignore_ascii_case("default") || value.is_empty() {
                clear_compaction_threshold(&mut stored);
            } else {
                set_compaction_threshold(&mut stored, value)?;
            }
        }
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

fn sync_agent_from_settings(agent: &mut Agent) {
    let stored = load_settings(&default_agent_dir());
    agent.auto_compaction = stored.compaction_enabled();
    agent.compaction = stored.compaction_settings();
    agent.block_images = stored.block_images();
    agent.auto_resize_images = stored.image_auto_resize();
    agent.transport = stored.transport.clone();
    agent.install_telemetry = stored.install_telemetry_enabled();
    agent.auto_retry = stored.retry_enabled();
}

fn looks_like_oauth_input(value: &str) -> bool {
    value.starts_with("pi-fixture-")
        || value.contains("://")
        || value.contains("code=")
        || value.contains('#')
}

fn login_js_oauth_provider(provider: &str) -> Option<(String, String)> {
    let stored = load_settings(&default_agent_dir());
    ExtensionHost::load(&default_agent_dir(), &stored.extensions).js_oauth_provider(provider)
}

/// TS `DEFAULT_OAUTH_MINIMUM_VALIDITY_MS` (`auth/resolve.ts`): a token with
/// less than five minutes left is renewed before the request rather than
/// after it fails.
const OAUTH_MIN_VALIDITY_MS: u64 = 5 * 60 * 1000;

fn maybe_refresh_auth(
    storage: &mut AuthStorage,
    provider: &str,
    now: u64,
    min_expiry_ms: u64,
    no_refresh: bool,
) {
    if storage
        .maybe_refresh(provider, now, min_expiry_ms, no_refresh)
        .unwrap_or(false)
        || no_refresh
    {
        return;
    }
    let Some((path, name)) = login_js_oauth_provider(provider) else {
        return;
    };
    let Some(cred) = storage.get(provider) else {
        return;
    };
    if cred.kind != CredentialKind::Oauth {
        return;
    }
    if !pi_ai::credential_expires_by(cred, now.saturating_add(min_expiry_ms)) {
        return;
    }
    if let Ok((access, refresh, expires)) = crate::js_host::run_js_oauth_refresh(
        Path::new(&path),
        &name,
        cred.access.as_deref().unwrap_or(""),
        cred.refresh.as_deref(),
        cred.expires,
    ) {
        let _ = storage.login_oauth(provider, access, refresh, expires);
    }
}

fn apply_js_oauth_api_key(
    provider: &str,
    storage: Option<&AuthStorage>,
    auth: &mut Option<ResolvedAuth>,
) {
    let Some(storage) = storage else {
        return;
    };
    let Some((path, name)) = login_js_oauth_provider(provider) else {
        return;
    };
    let Some(cred) = storage.get(provider) else {
        return;
    };
    if cred.kind != CredentialKind::Oauth {
        return;
    }
    let Some(key) = crate::js_host::run_js_oauth_get_api_key(
        Path::new(&path),
        &name,
        cred.access.as_deref().unwrap_or(""),
        cred.refresh.as_deref(),
        cred.expires,
    ) else {
        return;
    };
    if let Some(auth) = auth.as_mut() {
        auth.api_key = Some(key);
    }
}

fn login_provider(provider: &str, key: Option<&str>) -> Result<(), String> {
    login_provider_with_wait(provider, key, false).map(|_| ())
}

/// `Ok(true)` when a credential reached the store. A browser handshake that
/// only printed its URL returns `Ok(false)`: the caller must not announce a
/// sign-in that never happened, which is how `/login` used to report success
/// and leave the next request with no credential at all.
fn login_provider_with_wait(
    provider: &str,
    key: Option<&str>,
    wait_for_oauth_callback: bool,
) -> Result<bool, String> {
    let mut storage = AuthStorage::create().map_err(|err| err.to_string())?;
    if key.is_none() {
        if let Some((path, name)) = login_js_oauth_provider(provider) {
            let (access, refresh, expires) =
                crate::js_host::run_js_oauth_login(Path::new(&path), &name)?;
            storage
                .login_oauth(provider, access, refresh, expires)
                .map_err(|err| err.to_string())?;
            println!("stored oauth token for {provider}");
            return Ok(true);
        }
    }
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
        return Ok(true);
    }
    if let Some(key) = key {
        if looks_like_oauth_input(key) {
            let (code, _) = pi_ai::parse_authorization_input(key);
            let code = code.ok_or_else(|| "Missing authorization code.".to_string())?;
            let pkce = pi_ai::generate_pkce(uuid::Uuid::new_v4().as_bytes());
            let tokens = pi_ai::exchange_authorization_code(provider, &code, Some(&pkce))?;
            return store_oauth_tokens(&mut storage, provider, tokens);
        }
        storage
            .login_api_key(provider, key)
            .map_err(|err| err.to_string())?;
        println!("stored api key for {provider}");
        return Ok(true);
    }
    if provider.is_empty() {
        println!("Usage: /login <provider> <api-key>");
        return Ok(false);
    }
    if let Some(request) = pi_ai::fresh_authorize_request(provider) {
        if let Ok(code) = std::env::var("PI_OAUTH_CODE") {
            let tokens =
                pi_ai::exchange_authorization_code(provider, &code, request.pkce.as_ref())?;
            return store_oauth_tokens(&mut storage, provider, tokens);
        }
        let should_wait = wait_for_oauth_callback || std::env::var("PI_OAUTH_WAIT").is_ok();
        if should_wait {
            if let Some(kind) = pi_ai::CallbackProvider::parse(provider) {
                let host = pi_ai::callback_host();
                let port = std::env::var("PI_OAUTH_CALLBACK_PORT")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| kind.default_port());
                let expected = request
                    .state
                    .clone()
                    .or_else(|| request.pkce.as_ref().map(|pkce| pkce.verifier.clone()))
                    .unwrap_or_default();
                let mut server = pi_ai::CallbackServer::bind(&host, port, kind, expected)?;
                println!("{}", request.url);
                println!("{}", request.instructions);
                println!("Waiting for browser callback on {}", server.redirect_uri()?);
                let response = server.accept_one()?;
                if let Some(code) = response.code {
                    let tokens =
                        pi_ai::exchange_authorization_code(provider, &code, request.pkce.as_ref())?;
                    return store_oauth_tokens(&mut storage, provider, tokens);
                }
                return Err("OAuth callback did not include an authorization code.".into());
            }
        }
        println!("{}", request.url);
        println!("{}", request.instructions);
        return Ok(false);
    }
    if let (Ok(access), refresh) = (
        std::env::var("PI_OAUTH_ACCESS"),
        std::env::var("PI_OAUTH_REFRESH").ok(),
    ) {
        return store_oauth_tokens(
            &mut storage,
            provider,
            pi_ai::OauthTokens {
                access,
                refresh,
                expires: None,
            },
        );
    }
    println!("Usage: /login <provider> <api-key>");
    Ok(false)
}

/// Write an exchanged or refreshed OAuth credential. The expiry is what makes
/// the login survive: with none recorded nothing ever renews the token, so a
/// JWT access token's own `exp` stands in when the provider did not say.
fn store_oauth_tokens(
    storage: &mut AuthStorage,
    provider: &str,
    tokens: pi_ai::OauthTokens,
) -> Result<bool, String> {
    let expires = tokens
        .expires
        .or_else(|| pi_ai::jwt_expiry_ms(&tokens.access));
    storage
        .login_oauth(provider, tokens.access, tokens.refresh, expires)
        .map_err(|err| err.to_string())?;
    println!("stored oauth token for {provider}");
    Ok(true)
}

/// Every provider (OAuth and API-key) completes after `/login `, not just the
/// OAuth subset. Shared by the legacy chrome and the davinci shell so both
/// offer the same list.
fn interactive_login_providers(parsed: &Args) -> Vec<String> {
    let mut ids: Vec<String> = PROVIDER_SPECS
        .iter()
        .map(|spec| spec.id.to_string())
        .chain(std::iter::once(llama::LLAMA_PROVIDER_ID.to_string()))
        .chain(loaded_extension_host(parsed).js_oauth_provider_names())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn interactive_slash_commands(agent: &Agent, parsed: &Args) -> Vec<SlashCommandSpec> {
    let host = loaded_extension_host(parsed);
    let mut commands: Vec<SlashCommandSpec> = slash::builtin_slash_commands()
        .into_iter()
        .map(|command| SlashCommandSpec {
            name: command.name,
            description: command.description,
            argument_hint: command.argument_hint,
            argument_items: Vec::new(),
        })
        .collect();
    let enable_skill_commands = load_settings(&default_agent_dir())
        .enable_skill_commands
        .unwrap_or(true);
    let skills = if enable_skill_commands {
        agent.skills.as_slice()
    } else {
        &[]
    };
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
        skills,
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
            argument_items: Vec::new(),
        });
    }
    for (name, description, argument_hint) in command_specs() {
        if commands.iter().any(|command| command.name == name) {
            continue;
        }
        commands.push(SlashCommandSpec {
            name: name.to_string(),
            description: description.to_string(),
            argument_hint: argument_hint.map(str::to_string),
            argument_items: Vec::new(),
        });
    }
    for command in &mut commands {
        if let Some(detail) = host
            .js
            .iter()
            .flat_map(|ext| ext.command_details.iter())
            .find(|item| item.name == command.name)
        {
            command.argument_items = detail
                .argument_items
                .iter()
                .map(|item| AutocompleteItem {
                    value: item.value.clone(),
                    label: if item.label.is_empty() {
                        item.value.clone()
                    } else {
                        item.label.clone()
                    },
                    description: item.description.clone(),
                })
                .collect();
        }
    }
    commands
}

fn append_native_invocable_commands(commands: &mut Vec<serde_json::Value>) {
    for native in native_invocable_commands() {
        let Some(name) = native.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if commands
            .iter()
            .any(|command| command.get("name").and_then(serde_json::Value::as_str) == Some(name))
        {
            continue;
        }
        commands.push(native);
    }
}

fn interactive_extra_autocomplete(parsed: &Args) -> Vec<ExtraAutocompleteProvider> {
    loaded_extension_host(parsed)
        .js
        .iter()
        .flat_map(|ext| {
            ext.autocomplete_providers
                .iter()
                .map(|provider| {
                    let path = ext.path.clone();
                    ExtraAutocompleteProvider {
                        trigger_characters: provider
                            .trigger_characters
                            .iter()
                            .filter_map(|value| value.chars().next())
                            .collect(),
                        items: provider
                            .items
                            .iter()
                            .map(|item| AutocompleteItem {
                                value: item.value.clone(),
                                label: if item.label.is_empty() {
                                    item.value.clone()
                                } else {
                                    item.label.clone()
                                },
                                description: item.description.clone(),
                            })
                            .collect(),
                        live_query: Some(LiveAutocompleteQuery(std::sync::Arc::new(
                            move |text: &str| {
                                crate::js_host::query_js_autocomplete(Path::new(&path), text)
                                    .into_iter()
                                    .map(|item| AutocompleteItem {
                                        value: item.value.clone(),
                                        label: if item.label.is_empty() {
                                            item.value.clone()
                                        } else {
                                            item.label.clone()
                                        },
                                        description: item.description.clone(),
                                    })
                                    .collect()
                            },
                        ))),
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn format_session_info(
    stats: &serde_json::Value,
    waste: Option<&cache_stats::CacheWasteTotals>,
) -> String {
    let mut info = format!(
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
    );
    if let Some(waste) = waste.filter(|item| item.missed_tokens > 0) {
        let miss_label = if waste.miss_count == 1 {
            "1 miss".into()
        } else {
            format!("{} misses", waste.miss_count)
        };
        let detail = format!("{} tokens, {miss_label}", waste.missed_tokens);
        if waste.missed_cost >= 0.0001 {
            info.push_str(&format!(
                "\nCache Re-billed: ${:.3} ({detail})",
                waste.missed_cost
            ));
        } else {
            info.push_str(&format!("\nCache Re-billed: {detail}"));
        }
    }
    info
}

fn apply_extension_shortcuts(
    parsed: &Args,
    session: &mut InteractiveSession,
    agent: &mut Agent,
    host: &ExtensionHost,
) {
    let (shortcuts, diagnostics) = host.resolve_shortcuts(&session.keybindings);
    session.extension_shortcuts = shortcuts;
    session.apply_extension_ui_calls(&host.ui_calls);
    apply_host_session_calls(parsed, agent, session, &host.session_calls, true);
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
                            None,
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
    agent: &mut Agent,
    session: &mut InteractiveSession,
    path: &str,
    key: &str,
) -> Result<Option<serde_json::Value>, String> {
    let mut host = loaded_extension_host(parsed);
    let result = if host.js.iter().any(|ext| ext.path == path) {
        host.invoke_shortcut(path, key)?
    } else {
        ExtensionHost::default().invoke_shortcut(path, key)?
    };
    session.apply_extension_ui_calls(&host.ui_calls);
    apply_host_session_calls(parsed, agent, session, &host.session_calls, true);
    Ok(result)
}

fn try_extension_slash(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    name: &str,
    args: &str,
) -> Result<bool, String> {
    let mut host = loaded_extension_host(parsed);
    apply_graph_session_context(parsed, agent, &host);
    if let Some(result) = host.execute_native_command(name, args)? {
        push_native_panel(session, name, &result);
        session.chrome.status = format!("/{name}");
        return Ok(true);
    }
    let path = host
        .js
        .iter()
        .find(|ext| ext.commands.iter().any(|command| command == name))
        .map(|ext| ext.path.clone());
    let Some(path) = path else {
        return Ok(false);
    };
    host.runtime_active_tools = agent.tools.clone();
    host.runtime_all_tools = agent.tool_registry.clone();
    host.runtime_thinking_level = agent.thinking_level.as_str().to_string();
    host.runtime_flag_values = flag_values_json(parsed);
    let result = host.invoke_command(&path, name)?;
    session.apply_extension_ui_calls(&host.ui_calls);
    apply_host_session_calls(parsed, agent, session, &host.session_calls, true);
    apply_custom_overlay_result(&mut session.chrome, &path, name, result.as_ref());
    session.chrome.status = format!("/{name}");
    println!("{}", session.chrome.status);
    Ok(true)
}

fn parse_extension_command(input: &str) -> (String, String) {
    let command = input.trim_start().trim_start_matches('/');
    let mut parts = command.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default().to_string();
    let args = parts.next().unwrap_or_default().trim().to_string();
    (name, args)
}

fn format_extension_command_result(result: serde_json::Value) -> String {
    match result {
        serde_json::Value::String(text) => text,
        value => serde_json::to_string(&value).unwrap_or_else(|_| value.to_string()),
    }
}

/// `camelCase` / `snake_case` JSON keys → spaced labels for panel rows.
fn humanize_key(key: &str) -> String {
    let mut out = String::new();
    for ch in key.chars() {
        if ch == '_' || ch == '-' {
            out.push(' ');
        } else if ch.is_uppercase() {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

fn panel_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Bool(true) => "✓".into(),
        serde_json::Value::Bool(false) => "off".into(),
        serde_json::Value::Null => "—".into(),
        other => other.to_string(),
    }
}

/// Instrument name + accent role for a native command (design spec §5).
fn native_instrument(name: &str) -> (&'static str, &'static str, &'static str) {
    if name.starts_with("memory") {
        ("memoria", "VECTOR MEMORY", "secondary")
    } else if name.starts_with("governor") {
        ("mensura", "TOKEN GOVERNOR", "warning")
    } else if name.starts_with("graph") {
        ("grafo", "EXECUTION GRAPH", "primary")
    } else if name.starts_with("sec") {
        ("speculum", "SECURITY SCAN", "error")
    } else {
        ("instrumenta", "", "primary")
    }
}

/// Render a native command result as a framed instrument panel in the
/// transcript instead of a JSON dump on the status line.
fn push_native_panel(session: &mut InteractiveSession, name: &str, result: &serde_json::Value) {
    let theme = session.chrome.theme.clone();
    let width = session.width.clamp(40, 100);
    let (instrument, subtitle, accent) = native_instrument(name);
    let mut body: Vec<String> = Vec::new();
    match result {
        serde_json::Value::Object(map) => {
            // memory-search: hits get score rows, the rest key/value rows.
            if let Some(hits) = map.get("hits").and_then(|value| value.as_array()) {
                let query = map.get("query").and_then(|v| v.as_str()).unwrap_or("");
                body.push(format!(
                    "{} {}",
                    theme.fg("secondary", "⌕"),
                    theme.fg("text", query)
                ));
                if hits.is_empty() {
                    body.push(theme.fg("muted", "no matches above the relevance floor"));
                }
                for hit in hits.iter().take(8) {
                    let score = hit
                        .get("score")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(0.0);
                    let text = hit
                        .get("text")
                        .or_else(|| hit.get("summary"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("");
                    let mut text: String = text.chars().take(width.saturating_sub(12)).collect();
                    if text.is_empty() {
                        text = "(no excerpt)".into();
                    }
                    body.push(format!(
                        "{} {}",
                        theme.fg("primary", &format!("{score:.2}")),
                        theme.fg("muted", &text)
                    ));
                }
            } else {
                for (key, value) in map {
                    if matches!(
                        value,
                        serde_json::Value::Array(_) | serde_json::Value::Object(_)
                    ) {
                        continue;
                    }
                    let text = panel_value_text(value);
                    let styled = match value {
                        serde_json::Value::Bool(true) => theme.fg("success", &text),
                        serde_json::Value::Bool(false) => theme.fg("dim", &text),
                        serde_json::Value::Number(_) => theme.fg("text", &text),
                        _ => theme.fg("muted", &text),
                    };
                    let label = format!("{:<20}", humanize_key(key));
                    body.push(format!("{} {}", theme.fg("muted", &label), styled));
                }
            }
        }
        serde_json::Value::String(text) => {
            for line in text.lines().take(30) {
                body.push(theme.fg("muted", line));
            }
        }
        other => body.push(theme.fg("muted", &other.to_string())),
    }
    if body.is_empty() {
        body.push(theme.fg("dim", "(empty)"));
    }
    let panel = theme.panel(instrument, Some(subtitle), accent, &body, width);
    session.chrome.transcript.push("panel", panel.join("\n"));
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

fn handle_custom_overlay_input(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    data: &str,
) {
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
    host.runtime_active_tools = agent.tools.clone();
    host.runtime_all_tools = agent.tool_registry.clone();
    host.runtime_thinking_level = agent.thinking_level.as_str().to_string();
    host.runtime_flag_values = flag_values_json(parsed);
    match host.invoke_command_with(&path, &name, data, snapshot.as_ref(), session.width) {
        Ok(result) => {
            session.apply_extension_ui_calls(&host.ui_calls);
            apply_host_session_calls(parsed, agent, session, &host.session_calls, true);
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

fn apply_discovered_resources(parsed: &Args, agent: &mut Agent) {
    let settings = load_merged_settings_with_override(
        &default_agent_dir(),
        &agent.cwd,
        parsed.project_trust_override,
    );
    let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
    if !parsed.no_skills {
        let mut roots: Vec<PathBuf> = parsed.skills.iter().map(PathBuf::from).collect();
        roots.push(default_agent_dir().join("skills"));
        if trusted {
            roots.push(agent.cwd.join(".pi").join("skills"));
        }
        if let Some(extra) = &settings.skills {
            roots.extend(extra.iter().map(PathBuf::from));
        }
        for pkg in &settings.packages {
            roots.extend(settings::collect_package_resources(pkg, "skills"));
        }
        agent.skills = discover_skills(&roots);
    }
    if !parsed.no_prompt_templates {
        let mut roots: Vec<PathBuf> = parsed.prompt_templates.iter().map(PathBuf::from).collect();
        roots.push(default_agent_dir().join("prompts"));
        if trusted {
            roots.push(agent.cwd.join(".pi").join("prompts"));
        }
        if let Some(extra) = &settings.prompts {
            roots.extend(extra.iter().map(PathBuf::from));
        }
        for pkg in &settings.packages {
            roots.extend(settings::collect_package_resources(pkg, "prompts"));
        }
        agent.templates = discover_prompt_templates(&roots);
    }
    agent.context_files = load_context_files(&agent.cwd, !parsed.no_context_files);
}

fn rebind_print_extensions(parsed: &Args, agent: &mut Agent, host: &mut ExtensionHost) {
    apply_discovered_resources(parsed, agent);
    *host = loaded_extension_host(parsed);
    host.runtime_flag_values = flag_values_json(parsed);
    let mut names = host.native_tool_names();
    names.extend(extensions::extension_tool_names(&host.manifests));
    for ext in &host.js {
        names.extend(ext.tools.iter().cloned());
        names.extend(ext.commands.iter().cloned());
    }
    agent.apply_extension_tools(&names);
    attach_tool_executor(agent, host);
    host.emit(ExtensionEvent::SessionStart);
}

fn show_loaded_resources(
    session: &mut InteractiveSession,
    agent: &Agent,
    host: &ExtensionHost,
    parsed: &Args,
) {
    let show_listing = !session.quiet_startup;
    let expanded = parsed.verbose || session.chrome.tools_expanded;
    let theme = session.chrome.theme.clone();
    let home = pi_session::home_dir()
        .map(|home| home.display().to_string())
        .unwrap_or_default();
    let cwd = agent.cwd.display().to_string();
    let agent_dir = default_agent_dir().display().to_string();
    let mut loaded = pi_tui::LoadedResources::default();
    if show_listing {
        if !agent.context_files.is_empty() {
            let compact = theme.fg(
                "dim",
                &pi_tui::format_compact_list(
                    agent
                        .context_files
                        .iter()
                        .map(|file| {
                            format_context_path(&file.path.display().to_string(), &cwd, &home)
                        })
                        .collect::<Vec<_>>()
                        .iter()
                        .map(String::as_str),
                    false,
                ),
            );
            let expanded_body = agent
                .context_files
                .iter()
                .map(|file| {
                    theme.fg(
                        "dim",
                        &format!(
                            "  {}",
                            format_display_path(&file.path.display().to_string(), &home)
                        ),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            loaded.add_section(&theme, "Context", &compact, &expanded_body, expanded);
        }
        let skills: Vec<LoadedResourceItem> = agent
            .skills
            .iter()
            .map(|skill| {
                let path = skill.path.display().to_string();
                LoadedResourceItem {
                    compact_label: skill.name.clone(),
                    expanded_label: format_display_path(&path, &home),
                    source: infer_source_info(&path, &cwd, &agent_dir),
                    path,
                }
            })
            .collect();
        loaded.add_named_section(&theme, "Skills", &skills, expanded);
        let prompts: Vec<LoadedResourceItem> = agent
            .templates
            .iter()
            .map(|template| {
                let path = template.path.display().to_string();
                LoadedResourceItem {
                    compact_label: format!("/{}", template.name),
                    expanded_label: format!("/{}", template.name),
                    source: infer_source_info(&path, &cwd, &agent_dir),
                    path,
                }
            })
            .collect();
        loaded.add_named_section(&theme, "Prompts", &prompts, expanded);
        let mut extensions = Vec::new();
        for path in host
            .manifests
            .iter()
            .filter_map(|manifest| manifest.path.clone())
            .chain(host.js.iter().map(|ext| ext.path.clone()))
            .chain(parsed.extensions.iter().cloned())
        {
            if extensions
                .iter()
                .any(|item: &LoadedResourceItem| item.path == path)
            {
                continue;
            }
            let display = format_extension_display_path(&path, &home);
            extensions.push(LoadedResourceItem {
                compact_label: compact_extension_label(&display),
                expanded_label: display,
                source: infer_source_info(&path, &cwd, &agent_dir),
                path,
            });
        }
        loaded.add_named_section(&theme, "Extensions", &extensions, expanded);
        let themes: Vec<LoadedResourceItem> = collect_custom_theme_files(parsed)
            .into_iter()
            .map(|(name, path)| {
                let path = path.display().to_string();
                LoadedResourceItem {
                    compact_label: name,
                    expanded_label: format_display_path(&path, &home),
                    source: infer_source_info(&path, &cwd, &agent_dir),
                    path,
                }
            })
            .collect();
        loaded.add_named_section(&theme, "Themes", &themes, expanded);
    }
    add_resource_collisions(
        &mut loaded,
        &theme,
        "Skill conflicts",
        &agent
            .skills
            .iter()
            .map(|skill| (skill.name.clone(), skill.path.display().to_string()))
            .collect::<Vec<_>>(),
    );
    add_resource_collisions(
        &mut loaded,
        &theme,
        "Prompt conflicts",
        &agent
            .templates
            .iter()
            .map(|template| (template.name.clone(), template.path.display().to_string()))
            .collect::<Vec<_>>(),
    );
    session.chrome.loaded_resources = loaded;
}

fn add_resource_collisions(
    loaded: &mut pi_tui::LoadedResources,
    theme: &Theme,
    title: &str,
    items: &[(String, String)],
) {
    let collisions = collect_name_collisions(items);
    if collisions.is_empty() {
        return;
    }
    let body = collisions
        .iter()
        .map(|(name, winner, losers)| {
            format_collision_diagnostic(theme, name, winner, losers.as_slice())
        })
        .collect::<Vec<_>>()
        .join("\n");
    loaded.add_diagnostic(theme, title, &body);
}

fn format_extension_display_path(path: &str, home: &str) -> String {
    let trimmed = path
        .trim_end_matches("/index.ts")
        .trim_end_matches("/index.js");
    format_display_path(trimmed, home)
}

fn compact_extension_label(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "~")
        .collect();
    if matches!(
        segments.last().copied(),
        Some("index.ts") | Some("index.js")
    ) {
        segments.pop();
    }
    segments.last().copied().unwrap_or(path).to_string()
}

fn collect_custom_theme_files(parsed: &Args) -> Vec<(String, PathBuf)> {
    let mut files = theme_files_from_dir(&default_agent_dir().join("themes"));
    for path in &parsed.themes {
        files.extend(theme_files_from_dir(Path::new(path)));
    }
    let settings = load_settings(&default_agent_dir());
    if let Some(paths) = &settings.themes {
        for path in paths {
            files.extend(theme_files_from_dir(Path::new(path)));
        }
    }
    for pkg in &settings.packages {
        for path in settings::collect_package_resources(pkg, "themes") {
            if path.is_dir() {
                files.extend(theme_files_from_dir(&path));
            } else if let Some(parent) = path.parent() {
                files.extend(theme_files_from_dir(parent));
            }
        }
    }
    files
}

fn attach_tool_executor(agent: &mut Agent, host: &ExtensionHost) {
    let host = host.clone();
    agent.custom_tool_executor = Some(CustomToolExecutor::new(move |cwd, name, args| {
        host.execute_js_or_manifest_tool(cwd, name, args)
    }));
}

fn attach_shared_tool_executor(agent: &mut Agent, host: Arc<Mutex<ExtensionHost>>) {
    agent.custom_tool_executor = Some(CustomToolExecutor::new(move |cwd, name, args| {
        let host = host
            .lock()
            .map_err(|error| pi_agent::ToolError::Failed(error.to_string()))?;
        host.execute_js_or_manifest_tool(cwd, name, args)
    }));
}

/// Hand the graph controller the session's model, thinking level, and trust
/// decision so the workers it spawns inherit them.
fn apply_graph_session_context(parsed: &Args, agent: &Agent, host: &ExtensionHost) {
    let settings = load_merged_settings_with_override(
        &default_agent_dir(),
        &agent.cwd,
        parsed.project_trust_override,
    );
    let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
    let thinking = match agent.thinking_level {
        pi_protocol::ThinkingLevel::Off => None,
        level => Some(level.as_str().to_string()),
    };
    host.set_graph_session_context(
        Some(format!("{}/{}", agent.provider, agent.model_id)),
        thinking,
        trusted,
    );
}

fn loaded_extension_host(parsed: &Args) -> ExtensionHost {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loaded_extension_host_for_cwd(parsed, &cwd)
}

fn loaded_extension_host_for_cwd(parsed: &Args, cwd: &Path) -> ExtensionHost {
    let stored = load_settings(&default_agent_dir());
    let extensions = if parsed.no_extensions {
        parsed.extensions.clone()
    } else {
        let mut extensions = stored.extensions.clone();
        extensions.extend(parsed.extensions.clone());
        extensions
    };
    ExtensionHost::load_with_cwd(&default_agent_dir(), &extensions, cwd)
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
        &session.chrome.editor.get_expanded_text(),
    )?;
    session.chrome.status = editor.launch_message();
    println!("{}", session.chrome.status);
    let text = editor.edit()?;
    session.chrome.editor.set_text(text);
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
        session.chrome.editor.insert_text_at_cursor(&text);
        session.chrome.status = "pasted clipboard".into();
    } else {
        session.chrome.status = "clipboard empty".into();
    }
}

fn open_trust_selector(agent: &Agent, session: &mut InteractiveSession) {
    let cwd = agent.cwd.clone();
    let store = ProjectTrustStore::open(&default_agent_dir());
    let saved = store.get_entry(&cwd).map(|entry| TrustSavedDecision {
        path: entry.path,
        decision: entry.decision,
    });
    let settings = load_settings(&default_agent_dir());
    let project_trusted = is_trusted(&settings, &cwd, None);
    let options = get_project_trust_options(&cwd, false)
        .into_iter()
        .map(|option| TrustOption {
            label: option.label,
            trusted: option.trusted,
            updates: option
                .updates
                .into_iter()
                .map(|update| TrustUpdate {
                    path: update.path,
                    decision: update.decision,
                })
                .collect(),
            saved_path: option.saved_path,
        })
        .collect();
    session.open_trust_selector(
        TrustSelector::new(cwd.display().to_string(), options, saved, project_trusted)
            .with_theme(session.chrome.theme.clone()),
    );
}

fn apply_trust_decision(
    session: &mut InteractiveSession,
    trusted: bool,
    updates: &[TrustUpdate],
) -> Result<(), String> {
    let store = ProjectTrustStore::open(&default_agent_dir());
    let mapped: Vec<ProjectTrustUpdate> = updates
        .iter()
        .map(|update| ProjectTrustUpdate {
            path: update.path.clone(),
            decision: update.decision,
        })
        .collect();
    store.set_many(&mapped)?;
    session.chrome.status = format!(
        "Saved trust decision: {}. Restart pi for this to take effect.",
        if trusted { "trusted" } else { "untrusted" }
    );
    println!("{}", session.chrome.status);
    Ok(())
}

fn apply_project_trust_warning(session: &mut InteractiveSession, parsed: &Args, agent: &Agent) {
    let settings = load_settings(&default_agent_dir());
    if is_trusted(&settings, &agent.cwd, parsed.project_trust_override)
        || !has_trust_requiring_project_resources(&agent.cwd)
    {
        return;
    }
    session.chrome.transcript.push(
        "warning",
        "This project is not trusted. Project .pi resources and packages are ignored. Use /trust to save a trust decision, then restart pi.",
    );
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

fn apply_changelog_overlay(
    session: &mut InteractiveSession,
    agent: &Agent,
    settings: &settings::Settings,
    agent_dir: &Path,
) {
    let has_messages = agent.session.as_ref().is_some_and(|store| {
        store
            .entries
            .iter()
            .any(|entry| entry.entry_type == "message")
    });
    let entries = changelog::parse_changelog(&changelog::changelog_path());
    let display = changelog::changelog_for_display(
        settings.last_changelog_version.as_deref(),
        VERSION,
        &entries,
        has_messages,
    );
    if let Some(version) = &display.persist_version {
        let mut stored = load_settings(agent_dir);
        stored.last_changelog_version = Some(version.clone());
        let _ = save_settings(agent_dir, &stored);
    }
    if display.report_telemetry {
        changelog::report_install_telemetry(VERSION, settings.install_telemetry_enabled());
    }
    if let Some(markdown) = display.markdown {
        let text = changelog::format_startup_changelog(
            &markdown,
            settings.collapse_changelog.unwrap_or(false),
            VERSION,
        );
        session.chrome.transcript.push("changelog", &text);
    }
}

fn refresh_chrome_footer(session: &mut InteractiveSession, agent: &Agent) {
    session.chrome.footer_cwd = Some(agent.cwd.to_string_lossy().into_owned());
    session.chrome.footer_home = pi_session::home_dir().map(|home| home.display().to_string());
    session.chrome.footer_branch = resolve_git_branch(&agent.cwd);
    session.chrome.footer_session_name = agent
        .session
        .as_ref()
        .and_then(|store| store.display_name());
    session.chrome.footer_stats = Some(footer_stats_line(agent));
    session.chrome.footer_model = Some(if agent.model_id.is_empty() {
        agent.provider.clone()
    } else {
        format!("{}/{}", agent.provider, agent.model_id)
    });
    session.chrome.footer_context = Some((
        pi_agent::estimate_context_tokens(&agent.messages),
        agent.context_window,
    ));
    session.chrome.footer_delta = Some(session_delta_stats(agent));
}

/// `Δfiles +added -removed` across this session's write/edit tool calls.
fn session_delta_stats(agent: &Agent) -> (u64, u64, u64) {
    let mut files = std::collections::BTreeSet::new();
    let mut added = 0_u64;
    let mut removed = 0_u64;
    let Some(store) = agent.session.as_ref() else {
        return (0, 0, 0);
    };
    for entry in &store.entries {
        let Some(message) = entry.message.as_ref() else {
            continue;
        };
        let Some(blocks) = message.get("content").and_then(|value| value.as_array()) else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(|value| value.as_str()) != Some("toolCall") {
                continue;
            }
            let name = block.get("name").and_then(|value| value.as_str());
            let args = block
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match name {
                Some("write") => {
                    if let Some(path) = args.get("path").and_then(|value| value.as_str()) {
                        files.insert(path.to_string());
                    }
                    if let Some(content) = args.get("content").and_then(|value| value.as_str()) {
                        added += content.lines().count() as u64;
                    }
                }
                Some("edit") => {
                    if let Some(path) = args.get("path").and_then(|value| value.as_str()) {
                        files.insert(path.to_string());
                    }
                    if let Some(old) = args.get("oldText").and_then(|value| value.as_str()) {
                        removed += old.lines().count() as u64;
                    }
                    if let Some(new) = args.get("newText").and_then(|value| value.as_str()) {
                        added += new.lines().count() as u64;
                    }
                }
                _ => {}
            }
        }
    }
    (files.len() as u64, added, removed)
}

fn apply_terminal_title(
    session: &mut InteractiveSession,
    agent: &Agent,
    tui: Option<&mut InteractiveTui>,
) {
    session.terminal_title = Some(format_terminal_title(
        agent
            .session
            .as_ref()
            .and_then(|store| store.display_name())
            .as_deref(),
        &agent.cwd,
    ));
    if let Some(title) = session.terminal_title.as_deref() {
        if let Some(tui) = tui {
            tui.set_title(title);
        } else if std::io::stdout().is_terminal() || std::env::var("PI_TERMINAL_TITLE").is_ok() {
            print!("\x1b]0;{title}\x07");
            let _ = io::stdout().flush();
        }
    }
}

fn ensure_interactive_tools(session: &mut InteractiveSession) {
    let statuses = tools_manager::ensure_managed_tools();
    for status in statuses {
        show_managed_tool_status(session, &status);
    }
}

fn show_managed_tool_status(session: &mut InteractiveSession, status: &tools_manager::ToolStatus) {
    let (role, text) = match status.kind {
        tools_manager::ToolStatusKind::Warning => {
            ("warning", format!("Warning: {}", status.message))
        }
        tools_manager::ToolStatusKind::Info => ("dim", status.message.clone()),
    };
    session.chrome.transcript.push(role, &text);
}

fn footer_stats_line(agent: &Agent) -> String {
    let mut input = 0_u64;
    let mut output = 0_u64;
    let mut cache_read = 0_u64;
    let mut cache_write = 0_u64;
    if let Some(store) = agent.session.as_ref() {
        for entry in &store.entries {
            if let Some(usage) = cache_stats::assistant_usage_from_entry(entry) {
                input += usage.input;
                cache_read += usage.cache_read;
                cache_write += usage.cache_write;
            }
            if let Some(usage) = entry.extra.get("usage") {
                input += usage
                    .get("input")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                output += usage
                    .get("output")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                cache_read += usage
                    .get("cacheRead")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                cache_write += usage
                    .get("cacheWrite")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
            }
        }
    }
    let mut parts = Vec::new();
    if input > 0 {
        parts.push(format!("↑{}", cache_stats::format_tokens(input as i64)));
    }
    if output > 0 {
        parts.push(format!("↓{}", cache_stats::format_tokens(output as i64)));
    }
    if cache_read > 0 {
        parts.push(format!(
            "R{}",
            cache_stats::format_tokens(cache_read as i64)
        ));
    }
    if cache_write > 0 {
        parts.push(format!(
            "W{}",
            cache_stats::format_tokens(cache_write as i64)
        ));
    }
    if let Some(store) = agent.session.as_ref() {
        let waste = cache_stats::compute_cache_waste(&store.entries, &0.3);
        if waste.missed_tokens > 0 {
            parts.push(format!(
                "miss{}",
                cache_stats::format_tokens(waste.missed_tokens)
            ));
        }
    }
    let model = format!("{}/{}", agent.provider, agent.model_id);
    if parts.is_empty() {
        model
    } else {
        format!("{}  {}", parts.join(" "), model)
    }
}

fn apply_cache_miss_notices(chrome: &mut ChatChrome, agent: &Agent, enabled: bool) {
    if !enabled {
        return;
    }
    let Some(store) = agent.session.as_ref() else {
        return;
    };
    for (_index, miss) in cache_stats::collect_cache_misses(&store.entries, &0.3) {
        if let Some(text) = cache_stats::format_cache_miss_notice(&miss) {
            if !chrome.transcript.lines.iter().any(|line| line.text == text) {
                chrome.transcript.push("notice", &text);
            }
        }
    }
    if let Some((index, usage)) =
        store
            .entries
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, entry)| {
                cache_stats::assistant_usage_from_entry(entry).map(|usage| (index, usage))
            })
    {
        if let Some(miss) = cache_stats::detect_cache_miss(&store.entries[..index], &usage, &0.3) {
            if let Some(text) = cache_stats::format_cache_miss_notice(&miss) {
                if !chrome.transcript.lines.iter().any(|line| line.text == text) {
                    chrome.transcript.push("notice", &text);
                }
            }
        }
    }
    for entry in &store.entries {
        if entry.entry_type != "compaction" && entry.entry_type != "branch_summary" {
            continue;
        }
        let Some(usage) = entry
            .extra
            .get("usage")
            .cloned()
            .and_then(|value| serde_json::from_value::<pi_protocol::Usage>(value).ok())
        else {
            continue;
        };
        let text = cache_stats::format_compaction_cost_notice(&entry.entry_type, &usage);
        if !chrome.transcript.lines.iter().any(|line| line.text == text) {
            chrome.transcript.push("notice", &text);
        }
    }
}

fn apply_progress_events(session: &mut InteractiveSession, events: &[AgentEvent]) {
    for event in events {
        match event {
            AgentEvent::AgentStart => {
                if let Some(sequence) = session.set_progress(true) {
                    print!("{sequence}");
                }
            }
            AgentEvent::AgentEnd { .. } => {
                if let Some(sequence) = session.set_progress(false) {
                    print!("{sequence}");
                }
            }
            _ => {}
        }
    }
}

fn apply_host_session_calls(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    calls: &[serde_json::Value],
    trigger_turns: bool,
) {
    let mut host = loaded_extension_host(parsed);
    host.runtime_flag_values = flag_values_json(parsed);
    let mut allowed = Vec::new();
    for call in calls {
        match call.get("op").and_then(|value| value.as_str()) {
            Some("fork") => {
                host.emit(ExtensionEvent::SessionBeforeFork);
                if host.last_result_cancelled() {
                    continue;
                }
            }
            Some("switchSession") => {
                host.emit(ExtensionEvent::SessionBeforeSwitch);
                if host.last_result_cancelled() {
                    continue;
                }
            }
            Some("navigateTree") => {
                host.emit(ExtensionEvent::SessionBeforeTree);
                if host.last_result_cancelled() {
                    continue;
                }
            }
            Some("reload") => host.emit(ExtensionEvent::SessionShutdown {
                reason: "reload".into(),
            }),
            _ => {}
        }
        allowed.push(call.clone());
    }
    apply_session_calls(
        Some(parsed),
        agent,
        SessionCallUi::Chrome(&mut session.chrome),
        &allowed,
        trigger_turns,
    );
    if calls
        .iter()
        .any(|call| call.get("op").and_then(|value| value.as_str()) == Some("reload"))
    {
        reload_interactive_resources(parsed, agent, session);
    }
    for call in calls {
        if call.get("op").and_then(|value| value.as_str()) != Some("unregisterProvider") {
            continue;
        }
        let Some(name) = call.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        session
            .models
            .retain(|model| !model.starts_with(&format!("{name}/")));
        session.model_items.retain(|item| item.provider != name);
        session.login_providers.retain(|provider| provider != name);
    }
}

/// Where a session call's visible effects land. The state effects are the
/// same everywhere; what differs is who gets told: the legacy chrome takes a
/// status string, the davinci transcript takes a block (design.md §6 — the
/// transcript is the only place a davinci shell can say anything), and the
/// print path says nothing, as the TS reference does.
pub(crate) enum SessionCallUi<'a> {
    Chrome(&'a mut ChatChrome),
    Davinci(&'a mut pi_tui::davinci::model::Model),
    Silent,
}

impl SessionCallUi<'_> {
    /// A transcript line: a user message an extension sent, exec output.
    fn push(&mut self, kind: &str, text: &str) {
        use pi_tui::davinci::model::Entry;
        match self {
            SessionCallUi::Chrome(chrome) => chrome.transcript.push(kind, text),
            SessionCallUi::Davinci(model) => {
                model.transcript.push(Entry::Gap);
                if kind == "user" {
                    model.transcript.push(Entry::user(text));
                } else {
                    model.transcript.push(Entry::prose(text));
                }
            }
            SessionCallUi::Silent => {}
        }
    }

    /// What just happened, in a word or two: `fork=…`, `model=…`.
    fn status(&mut self, text: &str) {
        use pi_tui::davinci::model::Entry;
        use pi_tui::davinci::theme::State;
        match self {
            SessionCallUi::Chrome(chrome) => chrome.status = text.to_string(),
            SessionCallUi::Davinci(model) => {
                model.transcript.push(Entry::Gap);
                model
                    .transcript
                    .push(Entry::tool(State::Done, "instrumenta", text, None));
            }
            SessionCallUi::Silent => {}
        }
    }
}

/// The one-line note a session call earns in a hosted transcript when it was
/// applied where no UI handle was available (the post-turn sweep). Only the
/// calls that change what the user is looking at are worth a line.
fn session_call_note(call: &serde_json::Value) -> Option<String> {
    let field = |key: &str| {
        call.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    match call.get("op").and_then(|value| value.as_str())? {
        "fork" => Some("an extension forked the session".into()),
        "newSession" => Some("an extension started a new session".into()),
        "switchSession" => Some(format!(
            "an extension switched the session to {}",
            field("sessionPath")
        )),
        "reload" => Some("an extension reloaded skills, prompts and context files".into()),
        "setModel" => Some(format!("an extension set the model to {}", field("model"))),
        "setSessionName" => Some(format!("an extension named the session {}", field("name"))),
        _ => None,
    }
}

fn apply_session_calls(
    parsed: Option<&Args>,
    agent: &mut Agent,
    mut ui: SessionCallUi,
    calls: &[serde_json::Value],
    trigger_turns: bool,
) {
    for call in calls {
        match call.get("op").and_then(|value| value.as_str()) {
            Some("sendMessage") | Some("sendUserMessage") => {
                let text = call
                    .get("message")
                    .and_then(|value| value.as_str())
                    .or_else(|| call.get("text").and_then(|value| value.as_str()))
                    .unwrap_or("");
                if text.is_empty() {
                    continue;
                }
                ui.push("user", text);
                let deliver = call
                    .get("options")
                    .and_then(|value| value.get("deliverAs"))
                    .and_then(|value| value.as_str());
                match deliver {
                    Some("steer") => agent.queues.enqueue_steer(text),
                    Some("followUp") => agent.queues.enqueue_follow_up(text),
                    _ => {
                        agent.prompt(text);
                    }
                }
                let should_turn = trigger_turns
                    && call
                        .get("options")
                        .and_then(|value| value.get("triggerTurn"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                if should_turn {
                    if let Some(parsed) = parsed {
                        let _ = complete_prompt(parsed, agent);
                    }
                }
            }
            Some("appendEntry") => {
                if let Some(store) = agent.session.as_mut() {
                    let custom_type = call
                        .get("customType")
                        .and_then(|value| value.as_str())
                        .unwrap_or("custom");
                    let data = call.get("data").cloned().unwrap_or(serde_json::Value::Null);
                    let mut extra = serde_json::Map::new();
                    extra.insert("data".into(), data);
                    let _ = store.append_entry(SessionEntry {
                        id: String::new(),
                        entry_type: "custom".into(),
                        parent_id: None,
                        seq: 0,
                        timestamp: now_ms(),
                        message: None,
                        custom_type: Some(custom_type.to_string()),
                        extra,
                    });
                }
            }
            Some("setLabel") => {
                if let Some(store) = agent.session.as_mut() {
                    let id = call
                        .get("entryId")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let label = call.get("label").and_then(|value| value.as_str());
                    let _ = store.append_entry(SessionEntry::label_change(id, label));
                }
            }
            Some("setSessionName") => {
                if let Some(store) = agent.session.as_mut() {
                    let name = call
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let _ = store.set_name(name);
                }
            }
            Some("exec") => {
                if let Some(stdout) = call.get("stdout").and_then(|value| value.as_str()) {
                    ui.push("exec", stdout);
                    ui.status(&format!(
                        "exec {}",
                        call.get("command")
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                    ));
                } else if let Some(command) = call.get("command").and_then(|value| value.as_str()) {
                    match crate::js_host::execute_command_tool(command, &agent.cwd) {
                        Ok(out) => {
                            ui.push("exec", &out);
                            ui.status(&format!("exec {command}"));
                        }
                        Err(err) => {
                            ui.status(&format!("exec error: {err}"));
                        }
                    }
                }
            }
            Some("newSession") => {
                let parsed = parsed.cloned().unwrap_or_default();
                let session_dir = resolved_session_dir(&parsed, &agent.cwd);
                if let Ok(store) =
                    JsonlSession::create(&session_dir, &agent.cwd.to_string_lossy(), None)
                {
                    agent.messages.clear();
                    agent.session = Some(store);
                    ui.status("newSession");
                }
            }
            Some("fork") => {
                let parsed = parsed.cloned().unwrap_or_default();
                let session_dir = resolved_session_dir(&parsed, &agent.cwd);
                if let Some(store) = agent.session.as_ref() {
                    let entry_id = call
                        .get("entryId")
                        .and_then(|value| value.as_str())
                        .or(store.leaf_id.as_deref())
                        .unwrap_or(&store.header.id)
                        .to_string();
                    if let Ok(next) = store.fork(&entry_id, &session_dir) {
                        agent.load_from_session(next);
                        ui.status(&format!(
                            "fork={}",
                            agent
                                .session
                                .as_ref()
                                .map(|session| session.header.id.clone())
                                .unwrap_or_default()
                        ));
                    }
                }
            }
            Some("setModel") => {
                let model = call.get("model").and_then(|value| value.as_str());
                let provider = call.get("provider").and_then(|value| value.as_str());
                if let Some(model) = model {
                    let (next_provider, model_id) = if let Some(provider) = provider {
                        (provider.to_string(), model.to_string())
                    } else {
                        parse_model_ref("google", Some(model))
                    };
                    agent.provider = next_provider;
                    agent.model_id = model_id;
                    ui.status(&format!("model={}/{}", agent.provider, agent.model_id));
                }
            }
            Some("waitForIdle") => {}
            Some("switchSession") => {
                if let Some(path) = call.get("sessionPath").and_then(|value| value.as_str()) {
                    if let Ok(next) = JsonlSession::open(Path::new(path)) {
                        agent.load_from_session(next);
                        ui.status(&format!("session={}", path));
                    }
                }
            }
            Some("navigateTree") => {
                if let Some(target) = call.get("targetId").and_then(|value| value.as_str()) {
                    let summarize = call
                        .get("options")
                        .and_then(|value| value.get("summarize"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    let _ = agent.navigate_tree_entry(target, summarize, None, false, 16_384);
                    ui.status(&format!("tree={target}"));
                }
            }
            Some("reload") => {
                agent.skills = discover_skills(&[agent.cwd.join(".pi").join("skills")]);
                agent.templates =
                    discover_prompt_templates(&[agent.cwd.join(".pi").join("prompts")]);
                agent.context_files = load_context_files(&agent.cwd, true);
                ui.status(
                    "Reloaded keybindings, extensions, skills, prompts, themes, and context files",
                );
            }
            Some("setActiveTools") => {
                let names: Vec<String> = call
                    .get("toolNames")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                agent.set_active_tools_by_name(&names);
                ui.status(&format!("tools {}", agent.tools.join(",")));
            }
            Some("setThinkingLevel") => {
                if let Some(level) = call
                    .get("level")
                    .and_then(|value| value.as_str())
                    .and_then(pi_protocol::ThinkingLevel::parse)
                {
                    agent.thinking_level = level;
                    ui.status(&format!("thinking {}", level.as_str()));
                }
            }
            _ => {}
        }
    }
}

fn compute_catalog_refresh(parsed: &Args) -> catalog_refresh::CatalogRefreshResult {
    let agent_dir = default_agent_dir();
    let allow_network = std::env::var("PI_OFFLINE").is_err();
    let mut refreshed = catalog_refresh::refresh_model_catalogs(&agent_dir, allow_network, false);
    catalog_refresh::refresh_js_providers(
        &mut refreshed,
        &loaded_extension_host(parsed).js_refresh_providers(),
        allow_network,
        false,
    );
    refreshed
}

/// Receiver for the startup catalog refresh running on a worker thread. The
/// synchronous refresh could stall launch for seconds of sequential HTTP
/// once the cache went stale; the raw loop applies the result when it lands.
static CATALOG_REFRESH_RX: Mutex<
    Option<std::sync::mpsc::Receiver<catalog_refresh::CatalogRefreshResult>>,
> = Mutex::new(None);

fn start_catalog_refresh_async(parsed: &Args) {
    let parsed = parsed.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(compute_catalog_refresh(&parsed));
    });
    if let Ok(mut slot) = CATALOG_REFRESH_RX.lock() {
        *slot = Some(rx);
    }
}

/// Apply a finished background refresh; true when the UI changed.
fn poll_catalog_refresh(parsed: &Args, session: &mut InteractiveSession) -> bool {
    let refreshed = {
        let mut slot = match CATALOG_REFRESH_RX.lock() {
            Ok(slot) => slot,
            Err(_) => return false,
        };
        let Some(rx) = slot.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                *slot = None;
                result
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *slot = None;
                return false;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
        }
    };
    apply_catalog_result(parsed, session, refreshed);
    true
}

fn refresh_interactive_models(parsed: &Args, session: &mut InteractiveSession) {
    session.chrome.status = catalog_refresh::refresh_status_refreshing().into();
    let refreshed = compute_catalog_refresh(parsed);
    apply_catalog_result(parsed, session, refreshed);
}

fn apply_catalog_result(
    parsed: &Args,
    session: &mut InteractiveSession,
    refreshed: catalog_refresh::CatalogRefreshResult,
) {
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
        if context.as_deref() == Some("auth-type") {
            session.login_auth_type_labels = None;
            session.chrome.status.clear();
            return Ok(true);
        }
        session.chrome.status = "extension-select cancelled".into();
        return Ok(true);
    };
    if context.as_deref() == Some("auth-type") {
        handle_auth_type_choice(session, &choice);
        return Ok(true);
    }
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
    agent: &mut Agent,
    session: &mut InteractiveSession,
    id: &str,
    choice: Option<String>,
) -> Result<bool, String> {
    match choice.as_deref() {
        None => {
            open_session_tree(agent, session);
            Ok(true)
        }
        Some("No summary") => apply_tree_navigation(agent, session, id, false, None),
        Some("Summarize") => apply_tree_navigation(agent, session, id, true, None),
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
    agent: &mut Agent,
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
                if let Err(error) =
                    apply_tree_navigation(agent, session, id, true, Some(instructions.as_str()))
                {
                    session.chrome.status = error;
                }
            }
        }
        return;
    }
    session.chrome.status = format!("extension-editor={}", value.unwrap_or_default());
}

fn select_tree_entry(
    agent: &mut Agent,
    session: &mut InteractiveSession,
    id: String,
) -> Result<bool, String> {
    if session.branch_summary_skip_prompt {
        return apply_tree_navigation(agent, session, &id, false, None);
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
    Ok(true)
}

fn apply_tree_navigation(
    agent: &mut Agent,
    session: &mut InteractiveSession,
    target_id: &str,
    summarize: bool,
    custom_instructions: Option<&str>,
) -> Result<bool, String> {
    let reserve = session.branch_summary_reserve_tokens;
    let result =
        agent.navigate_tree_entry(target_id, summarize, custom_instructions, false, reserve)?;
    if result.cancelled {
        session.chrome.status = format!("tree={target_id} cancelled");
        return Ok(true);
    }
    if let Some(text) = result.editor_text {
        session.chrome.editor.set_text(text);
    }
    session.chrome.status = if summarize {
        format!(
            "tree={target_id} summary={}",
            if custom_instructions.is_some() {
                "custom"
            } else {
                "summarize"
            }
        )
    } else {
        format!("tree={target_id} summary=none")
    };
    if let Some(summary) = result.summary {
        session.chrome.transcript.push("assistant", summary);
    }
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
    let allow_network = std::env::var("PI_OFFLINE").is_err();
    let mut refreshed =
        catalog_refresh::refresh_model_catalogs(&default_agent_dir(), allow_network, false);
    let stored = load_settings(&default_agent_dir());
    catalog_refresh::refresh_js_providers(
        &mut refreshed,
        &ExtensionHost::load(&default_agent_dir(), &stored.extensions).js_refresh_providers(),
        allow_network,
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

fn current_runtime_model(agent: &Agent) -> Option<pi_ai::Model> {
    let config = ModelConfig::load(&models_json_path(&default_agent_dir()));
    let models = apply_models_config(&load_builtin_models(), &config)
        .unwrap_or_else(|_| load_builtin_models());
    find_model(&models, &agent.provider, &agent.model_id).cloned()
}

fn apply_model_switch_thinking(
    agent: &mut Agent,
    session: &InteractiveSession,
    provider: &str,
    model_id: &str,
) {
    let key = format!("{provider}/{model_id}");
    let explicit = session
        .scoped_thinking_levels
        .get(&key)
        .and_then(|level| pi_protocol::ThinkingLevel::parse(level));
    let per_model = session
        .model_thinking_levels
        .get(&key)
        .and_then(|level| pi_protocol::ThinkingLevel::parse(level));
    let default_level = load_settings(&default_agent_dir())
        .default_thinking_level
        .as_deref()
        .and_then(pi_protocol::ThinkingLevel::parse);
    agent.thinking_level = model_resolver::thinking_level_for_model_switch(
        explicit,
        per_model,
        default_level,
        agent.thinking_level,
    );
}

fn select_resume_session(
    parsed: &Args,
    session_dir: &Path,
    cwd: &Path,
) -> Result<Option<PathBuf>, String> {
    if let Ok(selected) = std::env::var("PI_RESUME_SESSION") {
        let selected = selected.trim();
        if !selected.is_empty() {
            let path = PathBuf::from(selected);
            if path.exists() {
                return Ok(Some(path));
            }
            if let Ok(summary) =
                resolve_session_ref(session_dir, Some(&cwd.to_string_lossy()), selected)
            {
                return Ok(Some(summary.path));
            }
        }
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Ok(None);
    }
    let mut dummy = Agent::new(default_system_prompt());
    dummy.cwd = cwd.to_path_buf();
    let items = discover_session_items(parsed, &dummy)?;
    if items.is_empty() {
        return Ok(None);
    }
    let theme = builtin_themes().into_iter().next().expect("theme");
    let mut session = InteractiveSession::new(theme, format!("{APP_NAME} {VERSION}"), Vec::new());
    session.cwd = cwd.to_path_buf();
    session.open_session_selector(items.clone());
    if let Some(selector) = &mut session.chrome.session_selector {
        selector.set_cwd(cwd.to_string_lossy().into_owned());
    }
    let _raw = RawModeGuard::enter()?;
    let (mut width, _height) = crossterm::terminal::size().unwrap_or((80, 24));
    session.width = width as usize;
    print!("{}", InteractiveSession::enter_sequences(true));
    io::stdout().flush().ok();
    loop {
        if let Some(selector) = &session.chrome.session_selector {
            print!("\x1b[H\x1b[J{}", selector.render(width as usize).join("\n"));
            io::stdout().flush().ok();
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
                match session.handle_bytes(&key_event_to_bytes(&key)) {
                    SessionAction::SelectSession(id) => {
                        print!("{}", InteractiveSession::leave_sequences(true));
                        if let Some(item) = items.iter().find(|item| item.id == id) {
                            return Ok(Some(PathBuf::from(&item.path)));
                        }
                        if let Ok(summary) =
                            resolve_session_ref(session_dir, Some(&cwd.to_string_lossy()), &id)
                        {
                            return Ok(Some(summary.path));
                        }
                        return Ok(None);
                    }
                    SessionAction::CloseOverlay | SessionAction::Quit | SessionAction::Abort => {
                        print!("{}", InteractiveSession::leave_sequences(true));
                        return Ok(None);
                    }
                    _ => {}
                }
            }
            crossterm::event::Event::Resize(next_width, _) => {
                width = next_width;
                session.width = next_width as usize;
            }
            _ => {}
        }
    }
}

fn sync_session_thinking(session: &mut InteractiveSession, agent: &Agent) {
    if let Some(model) = current_runtime_model(agent) {
        let levels = get_supported_thinking_levels(&model);
        session.set_supported_thinking_levels(
            model.reasoning,
            levels
                .iter()
                .map(|level| level.as_str().to_string())
                .collect(),
        );
    }
}

fn login_provider_options(auth_type: Option<&str>) -> Vec<AuthSelectorProvider> {
    let storage = AuthStorage::create().unwrap_or_else(|_| AuthStorage::in_memory());
    let config = ModelConfig::load(&models_json_path(&default_agent_dir()));
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let mut options = Vec::new();
    for spec in PROVIDER_SPECS {
        let check = check_auth(spec.id, &config, &storage, &env);
        let status_type = check.as_ref().map(|item| item.kind.clone());
        let status_source = check.as_ref().map(|item| item.source.clone());
        if auth_type != Some("api_key") && spec.oauth {
            options.push(AuthSelectorProvider {
                id: spec.id.into(),
                name: spec.name.into(),
                auth_type: "oauth".into(),
                method_name: spec.oauth_name.map(str::to_string),
                status_type: status_type.clone(),
                status_source: status_source.clone(),
            });
        }
        if auth_type != Some("oauth") && !spec.env_vars.is_empty() {
            options.push(AuthSelectorProvider {
                id: spec.id.into(),
                name: spec.name.into(),
                auth_type: "api_key".into(),
                method_name: None,
                status_type,
                status_source,
            });
        }
    }
    options.sort_by(|left, right| left.name.cmp(&right.name));
    options
}

fn logout_provider_options() -> Result<Vec<AuthSelectorProvider>, String> {
    let storage = AuthStorage::create().map_err(|err| err.to_string())?;
    let mut options = Vec::new();
    for id in storage.providers() {
        let Some(credential) = storage.get(&id) else {
            continue;
        };
        let spec = PROVIDER_SPECS.iter().find(|item| item.id == id);
        options.push(AuthSelectorProvider {
            id: id.clone(),
            name: spec.map(|item| item.name.to_string()).unwrap_or(id),
            auth_type: match credential.kind {
                CredentialKind::Oauth => "oauth".into(),
                CredentialKind::ApiKey => "api_key".into(),
            },
            method_name: spec.and_then(|item| item.oauth_name.map(str::to_string)),
            status_type: Some(match credential.kind {
                CredentialKind::Oauth => "oauth".into(),
                CredentialKind::ApiKey => "api_key".into(),
            }),
            status_source: Some("stored credential".into()),
        });
    }
    options.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(options)
}

fn find_login_provider_options(provider_ref: &str) -> Vec<AuthSelectorProvider> {
    let needle = provider_ref.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    login_provider_options(None)
        .into_iter()
        .filter(|provider| {
            provider.id.to_ascii_lowercase() == needle
                || provider.name.to_ascii_lowercase() == needle
        })
        .collect()
}

fn provider_has_interactive_login(provider_id: &str, auth_type: &str) -> bool {
    PROVIDER_SPECS.iter().any(|spec| {
        spec.id == provider_id
            && if auth_type == "oauth" {
                spec.oauth
            } else {
                !spec.env_vars.is_empty()
            }
    })
}

fn show_login_auth_type_selector(
    session: &mut InteractiveSession,
    provider_options: Option<Vec<AuthSelectorProvider>>,
) {
    let oauth = provider_options
        .as_ref()
        .and_then(|options| options.iter().find(|item| item.auth_type == "oauth"));
    let subscription_label = oauth
        .and_then(|item| item.method_name.clone())
        .unwrap_or_else(|| "Sign in with an account".into());
    let api_key_label = "Sign in with an API key".to_string();
    let available: std::collections::BTreeSet<&str> = provider_options
        .as_ref()
        .map(|options| options.iter().map(|item| item.auth_type.as_str()).collect())
        .unwrap_or_else(|| ["oauth", "api_key"].into_iter().collect());
    let mut options = Vec::new();
    if available.contains("oauth") {
        options.push(subscription_label.clone());
    }
    if available.contains("api_key") {
        options.push(api_key_label.clone());
    }
    if options.is_empty() {
        session.chrome.status = "No login methods available.".into();
        println!("{}", session.chrome.status);
        return;
    }
    if let Some(providers) = &provider_options {
        if options.len() == 1 {
            if let Some(provider) = providers.first() {
                let _ = start_provider_login(session, &provider.id, &provider.auth_type, None);
            }
            return;
        }
        session.login_auth_options = providers.clone();
    } else {
        session.login_auth_options.clear();
    }
    session.login_auth_type_labels = Some((subscription_label, api_key_label));
    let title = provider_options
        .as_ref()
        .and_then(|options| options.first())
        .map(|provider| format!("Select authentication method for {}:", provider.name))
        .unwrap_or_else(|| "Select authentication method:".into());
    session.extension_dialog_context = Some("auth-type".into());
    session.open_extension_selector(title, options);
}

fn handle_auth_type_choice(session: &mut InteractiveSession, choice: &str) {
    let labels = session.login_auth_type_labels.take();
    let auth_type = match labels {
        Some((subscription, _)) if choice == subscription => "oauth",
        _ => "api_key",
    };
    let scoped = session.login_auth_options.clone();
    if !scoped.is_empty() {
        if let Some(provider) = scoped.iter().find(|item| item.auth_type == auth_type) {
            let _ = start_provider_login(session, &provider.id, &provider.auth_type, None);
        }
        return;
    }
    show_login_provider_selector(session, Some(auth_type), None);
}

fn show_login_provider_selector(
    session: &mut InteractiveSession,
    auth_type: Option<&str>,
    initial_search: Option<&str>,
) {
    let options = login_provider_options(auth_type);
    if options.is_empty() {
        session.chrome.status = match auth_type {
            Some("oauth") => "No subscription providers available.".into(),
            Some("api_key") => "No API key providers available.".into(),
            _ => "No login providers available.".into(),
        };
        println!("{}", session.chrome.status);
        return;
    }
    session.open_oauth_selector(AuthSelectorMode::Login, options, initial_search);
}

fn handle_login_command(
    session: &mut InteractiveSession,
    provider: &str,
    key: Option<&str>,
) -> Result<(), String> {
    if provider.is_empty() {
        show_login_auth_type_selector(session, None);
        return Ok(());
    }
    let options = find_login_provider_options(provider);
    if options.len() == 1 {
        return start_provider_login(session, &options[0].id, &options[0].auth_type, key);
    }
    if options.len() > 1 {
        let ids: std::collections::BTreeSet<&str> =
            options.iter().map(|item| item.id.as_str()).collect();
        if ids.len() == 1 {
            show_login_auth_type_selector(session, Some(options));
            return Ok(());
        }
    }
    show_login_provider_selector(session, None, Some(provider));
    Ok(())
}

fn handle_logout_command(
    session: &mut InteractiveSession,
    provider: Option<&str>,
) -> Result<bool, String> {
    if let Some(provider) = provider {
        let mut storage = AuthStorage::create().map_err(|err| err.to_string())?;
        storage.remove(provider).map_err(|err| err.to_string())?;
        session.chrome.status = format!("removed {provider}");
        println!("removed {provider}");
        return Ok(true);
    }
    match logout_provider_options() {
        Ok(options) if options.is_empty() => {
            session.chrome.status = "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.".into();
            println!("{}", session.chrome.status);
        }
        Ok(options) => {
            session.open_oauth_selector(AuthSelectorMode::Logout, options, None);
        }
        Err(error) => {
            session.chrome.status = format!("Could not read stored credentials: {error}");
            eprintln!("{}", session.chrome.status);
        }
    }
    Ok(true)
}

fn start_provider_login(
    session: &mut InteractiveSession,
    provider: &str,
    auth_type: &str,
    key: Option<&str>,
) -> Result<(), String> {
    if auth_type == "api_key"
        && key.is_none()
        && !provider_has_interactive_login(provider, auth_type)
    {
        let name = PROVIDER_SPECS
            .iter()
            .find(|spec| spec.id == provider)
            .map(|spec| spec.name)
            .unwrap_or(provider);
        session.open_login_dialog(provider, Some(name), Some(&format!("{name} setup")));
        if let Some(dialog) = &mut session.chrome.login_dialog {
            dialog.show_info(
                &format!("Authentication is configured outside {APP_NAME}."),
                &[],
                true,
            );
        }
        if let Some(dialog) = &session.chrome.login_dialog {
            println!("{}", dialog.render(80).join("\n"));
        }
        return Ok(());
    }
    start_login(session, provider, key)
}

fn start_login(
    session: &mut InteractiveSession,
    provider: &str,
    key: Option<&str>,
) -> Result<(), String> {
    let name = PROVIDER_SPECS
        .iter()
        .find(|spec| spec.id == provider)
        .map(|spec| spec.name);
    session.open_login_dialog(provider, name, None);
    if let Some(key) = key {
        login_provider(provider, Some(key))?;
        if let Some(dialog) = &mut session.chrome.login_dialog {
            dialog.show_progress(&format!("stored credentials for {provider}"));
        }
        return Ok(());
    }
    if let Some(request) = pi_ai::fresh_authorize_request(provider) {
        if let Some(dialog) = &mut session.chrome.login_dialog {
            dialog.show_auth(&request.url, Some(request.instructions.as_str()));
            dialog.show_manual_input("Paste the redirect URL or authorization code");
        }
        println!("{}", request.url);
        println!("{}", request.instructions);
    } else if let Some(dialog) = &mut session.chrome.login_dialog {
        if provider == "amazon-bedrock" {
            dialog.show_details(&[
                "You can also use an AWS profile, IAM keys, or role-based credentials.".into(),
                "See:".into(),
                format!("  {}/docs/providers.md", default_agent_dir().display()),
            ]);
        }
        dialog.show_manual_input("Enter API key");
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

    struct EnvRestore {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvRestore {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn codex_fixture_login_persists_exact_provider_and_resolves_immediately() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().expect("temp auth dir");
        let _agent_dir = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
        let _oauth_code = EnvRestore::set("PI_OAUTH_CODE", "pi-fixture-code");

        assert_eq!(
            login_provider_with_wait("openai-codex", None, false),
            Ok(true)
        );

        let storage = AuthStorage::open(&dir.path().join("auth.json")).expect("persisted auth");
        let credential = storage
            .get("openai-codex")
            .expect("credential stored under exact provider id");
        assert_eq!(credential.kind, CredentialKind::Oauth);
        assert!(storage.get("openai").is_none());

        let resolved = pi_ai::resolve_provider_auth(
            "openai-codex",
            &storage,
            &std::collections::HashMap::new(),
            false,
        )
        .expect("stored codex credential resolves immediately");
        assert_eq!(resolved.source, "OAuth");
        assert!(resolved.api_key.is_some());
    }

    #[test]
    fn rpc_prompt_auth_error_matches_ts_no_model_copy() {
        let runtime = RpcRuntime::new(
            Agent::new(default_system_prompt()),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
        );
        let error = rpc_prompt_auth_error(&runtime).expect("no model");
        assert!(error.starts_with("No model selected."));
        assert!(error.contains("Then use /model to select a model."));
        assert!(error.contains("Use /login to log into a provider via OAuth or API key. See:"));
    }

    #[test]
    fn offline_flag_sets_ts_env_vars() {
        let previous_offline = std::env::var("PI_OFFLINE").ok();
        let previous_skip = std::env::var("PI_SKIP_VERSION_CHECK").ok();
        std::env::remove_var("PI_OFFLINE");
        std::env::remove_var("PI_SKIP_VERSION_CHECK");
        apply_offline_mode(&["--offline".into()]);
        assert_eq!(std::env::var("PI_OFFLINE").as_deref(), Ok("1"));
        assert_eq!(std::env::var("PI_SKIP_VERSION_CHECK").as_deref(), Ok("1"));
        match previous_offline {
            Some(value) => std::env::set_var("PI_OFFLINE", value),
            None => std::env::remove_var("PI_OFFLINE"),
        }
        match previous_skip {
            Some(value) => std::env::set_var("PI_SKIP_VERSION_CHECK", value),
            None => std::env::remove_var("PI_SKIP_VERSION_CHECK"),
        }
    }

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
        assert!(help.contains("Extensions can register additional flags"));
        assert!(help.contains("Examples:"));
        assert!(help.contains("Environment Variables:"));
        assert!(help.contains("PI_CODING_AGENT_DIR"));
        assert!(help.contains("PI_SESSION_DIR"));
        let with_flags =
            args::print_help_with_extension_flags(&[("plan".into(), "/tmp/plan.js".into())]);
        assert!(with_flags.contains("Extension CLI Flags:"));
        assert!(with_flags.contains("--plan"));
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
            slash::parse_line("/thinking"),
            slash::SlashAction::OpenThinking
        ));
        assert!(matches!(
            slash::parse_line("/thinking high"),
            slash::SlashAction::SetThinking(_)
        ));
        assert_eq!(
            unknown_thinking_error("nope"),
            "Unknown thinking level \"nope\". Available levels: off, minimal, low, medium, high, xhigh, max."
        );
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
        assert_eq!(
            slash::parse_line("/review src/index.ts"),
            slash::SlashAction::Prompt("/review src/index.ts".into())
        );
        assert_eq!(
            slash::parse_line("/skill:test explain this"),
            slash::SlashAction::Prompt("/skill:test explain this".into())
        );
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
            thinking_level_map: Default::default(),
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

    #[test]
    fn apply_session_calls_creates_session_switches_model_and_steers() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("x");
        agent.cwd = dir.path().to_path_buf();
        let parsed = Args {
            session_dir: Some(dir.path().join("sessions").display().to_string()),
            ..Args::default()
        };
        let mut chrome = ChatChrome::new(builtin_themes()[0].clone(), "pi");
        apply_session_calls(
            Some(&parsed),
            &mut agent,
            SessionCallUi::Chrome(&mut chrome),
            &[
                serde_json::json!({"op":"setModel","model":"sonnet","provider":"anthropic"}),
                serde_json::json!({"op":"sendUserMessage","text":"hi","options":{"deliverAs":"steer"}}),
                serde_json::json!({"op":"exec","command":"echo","stdout":"ok"}),
                serde_json::json!({"op":"newSession"}),
            ],
            false,
        );
        assert_eq!(agent.provider, "anthropic");
        assert_eq!(agent.model_id, "sonnet");
        assert_eq!(agent.queues.steer.len(), 1);
        assert!(agent.session.is_some());
        assert!(agent.messages.is_empty());
        assert!(chrome
            .transcript
            .lines
            .iter()
            .any(|line| line.role == "exec" && line.text == "ok"));
        assert!(chrome.status.contains("newSession") || chrome.status.contains("model="));
    }

    #[test]
    fn rebind_print_extensions_rediscovers_skills_and_emits_session_start() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let skill_dir = dir.path().join(".pi").join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo\ndescription: demo skill\n---\n# Demo\n",
        )
        .unwrap();
        let mut agent = Agent::new("x");
        agent.cwd = dir.path().to_path_buf();
        let parsed = Args {
            project_trust_override: Some(true),
            ..Args::default()
        };
        let mut host = ExtensionHost::default();
        rebind_print_extensions(&parsed, &mut agent, &mut host);
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
        assert!(
            agent.skills.iter().any(|skill| skill.name == "demo"),
            "print-mode rebind should rediscover project skills: {:?}",
            agent
                .skills
                .iter()
                .map(|skill| &skill.name)
                .collect::<Vec<_>>()
        );
        assert!(host
            .events
            .iter()
            .any(|event| { matches!(event, crate::extension_host::ExtensionEvent::SessionStart) }));
    }

    #[test]
    fn show_loaded_resources_lists_context_skills_and_expands() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let skill_dir = dir.path().join(".pi").join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo\ndescription: demo skill\n---\n# Demo\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# agents\n").unwrap();
        let mut agent = Agent::new("x");
        agent.cwd = dir.path().to_path_buf();
        let parsed = Args {
            project_trust_override: Some(true),
            ..Args::default()
        };
        apply_discovered_resources(&parsed, &mut agent);
        let mut session = InteractiveSession::new(
            builtin_themes()[0].clone(),
            "pi",
            vec!["google/gemini".into()],
        );
        show_loaded_resources(&mut session, &agent, &ExtensionHost::default(), &parsed);
        let collapsed = session.chrome.render_document(80).join("\n");
        assert!(collapsed.contains("[Skills]"), "{collapsed}");
        assert!(collapsed.contains("demo"), "{collapsed}");
        assert!(collapsed.contains("[Context]"), "{collapsed}");
        assert!(collapsed.contains("AGENTS.md"), "{collapsed}");
        session.chrome.set_tools_expanded(true);
        let expanded = session.chrome.render_document(80).join("\n");
        assert!(
            expanded.contains("SKILL.md") || expanded.contains("demo"),
            "{expanded}"
        );
        session.quiet_startup = true;
        show_loaded_resources(&mut session, &agent, &ExtensionHost::default(), &parsed);
        let quiet = session.chrome.render_document(80).join("\n");
        assert!(!quiet.contains("[Skills]"), "{quiet}");
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }

    #[test]
    fn startup_header_and_model_scope_match_ts() {
        let theme = builtin_themes()[0].clone();
        let mut session = InteractiveSession::new(theme, "pi", vec!["anthropic/sonnet".into()]);
        session.enabled_model_ids = Some(vec!["anthropic/sonnet".into()]);
        session
            .scoped_thinking_levels
            .insert("anthropic/sonnet".into(), "high".into());
        apply_startup_header(&mut session, false);
        let collapsed = session.chrome.render_document(80).join("\n");
        assert!(
            collapsed.contains("full startup help and loaded resources"),
            "{collapsed}"
        );
        assert!(collapsed.contains("ctrl+o"), "{collapsed}");
        session.chrome.set_tools_expanded(true);
        let expanded = session.chrome.render_document(80).join("\n");
        assert!(expanded.contains("to expand tools"), "{expanded}");
        let line = model_scope_startup_line(&session).expect("scope");
        assert!(line.contains("Model scope:"), "{line}");
        assert!(line.contains("sonnet:high"), "{line}");
        session.quiet_startup = true;
        apply_startup_header(&mut session, false);
        assert!(session.chrome.startup_header.is_none());
        assert!(model_scope_startup_line(&session).is_none());
    }

    #[test]
    fn streaming_turn_delivers_offline_reply_to_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        let theme = builtin_themes()[0].clone();
        let mut session = InteractiveSession::new(theme, "davinci", vec![]);
        session.width = 100;
        let mut agent = Agent::new("test system prompt");
        agent.cwd = dir.path().to_path_buf();
        agent.provider = "openai-codex".into();
        agent.model_id = "gpt-5.6-sol".into();
        let parsed = Args {
            offline: true,
            no_extensions: true,
            ..Args::default()
        };
        let panes = ChromePanes::new(Vec::new(), Vec::new());
        let options = InteractiveTuiOptions {
            tui_mode: TuiMode::Regular,
            show_hardware_cursor: false,
            log_directory: dir.path().to_path_buf(),
            terminal: Box::new(pi_tui::MemoryTerminal::new(100, 30)),
            theme: builtin_themes()[0].clone(),
            copy_on_select: false,
            open_url: None,
            on_right_click_paste: None,
            copy_selection: None,
        };
        let mut tui = create_interactive_tui(options);
        remount_chrome_panes(&mut tui, &panes);
        ACTIVE_PANES.with(|slot| *slot.borrow_mut() = Some(panes.clone()));
        let ok = submit_user_message(
            &parsed,
            &mut agent,
            &mut session,
            "hello there",
            &[],
            Some(&mut tui),
        )
        .unwrap();
        ACTIVE_PANES.with(|slot| *slot.borrow_mut() = None);
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
        assert!(ok);
        let doc = session.chrome.render_document(100).join("\n");
        let plain = pi_tui::strip_terminal_sequences(&doc);
        assert!(plain.contains("> hello there"), "{plain}");
        assert!(
            plain.contains("(offline) received"),
            "assistant reply missing from transcript: {plain}"
        );
        assert!(session.chrome.working_message.is_none());
    }

    #[test]
    fn keystroke_pipeline_stays_fast_on_long_transcripts() {
        let theme = builtin_themes()[0].clone();
        let mut session =
            InteractiveSession::new(theme, "davinci", vec!["openai-codex/gpt-5.6-sol".into()]);
        session.width = 120;
        for i in 0..400 {
            session
                .chrome
                .transcript
                .push("user", format!("prompt {i}"));
            session.chrome.transcript.push(
                "assistant",
                format!("Answer {i} with some *markdown* and `code` and a\nsecond line."),
            );
            session
                .chrome
                .transcript
                .push("tool", format!("✓ manus · cargo check step {i}  0.3{i}s"));
        }
        session.chrome.footer_cwd = Some("C:\\dev\\davinci-rust".into());
        session.chrome.footer_model = Some("openai-codex/gpt-5.6-sol".into());
        session.chrome.footer_context = Some((47_000, 200_000));
        // Warm the render memo like a running session.
        let _ = session.chrome.render_document(120);
        let started = std::time::Instant::now();
        let keys = 60;
        for i in 0..keys {
            let ch = char::from(b'a' + (i % 26) as u8);
            let _ = session.handle_bytes(&ch.to_string());
            let _ = session.chrome.render_document(120);
            let _ = session.chrome.render_dock(120);
        }
        let per_key = started.elapsed() / keys;
        assert!(
            per_key < std::time::Duration::from_millis(12),
            "keystroke pipeline too slow: {per_key:?} per key"
        );
    }

    #[test]
    fn davinci_frame_composes_transcript_composer_and_status_bar() {
        let theme = builtin_themes()[0].clone();
        let mut session =
            InteractiveSession::new(theme, "davinci", vec!["openai-codex/gpt-5.6-sol".into()]);
        session.width = 100;
        session.chrome.transcript.agent_label = "davinci".into();
        session.chrome.transcript.push("user", "run the tests");
        session
            .chrome
            .transcript
            .push("tool", "✓ manus · cargo test -p pi-agent  1.84s");
        session.chrome.transcript.push(
            "tool",
            "× manus · cargo test -p pi-session  0.42s\n  ! error[E0308] mismatched types · store.rs:118",
        );
        session.chrome.transcript.push(
            "assistant",
            "The failing case builds a path with a forward slash.",
        );
        push_native_panel(
            &mut session,
            "governor-status",
            &serde_json::json!({
                "enabled": true,
                "compressedOutputs": 14,
                "deduplicatedReads": 6,
                "blockedCalls": 0,
            }),
        );
        session.chrome.footer_cwd = Some("C:\\dev\\davinci-rust".into());
        session.chrome.footer_branch = Some("main".into());
        session.chrome.footer_model = Some("openai-codex/gpt-5.6-sol".into());
        session.chrome.footer_context = Some((47_000, 200_000));
        session.chrome.footer_delta = Some((3, 42, 11));
        let document = session.chrome.render_document(100).join("\n");
        let dock = session.chrome.render_dock(100).join("\n");
        println!("─ document ─\n{document}\n─ dock ─\n{dock}");
        let plain_doc = pi_tui::strip_terminal_sequences(&document);
        let plain_dock = pi_tui::strip_terminal_sequences(&dock);
        assert!(plain_doc.contains("> run the tests"), "{plain_doc}");
        assert!(plain_doc.contains("◆ davinci"), "{plain_doc}");
        assert!(plain_doc.contains("✓ manus · cargo test -p pi-agent"));
        assert!(plain_doc.contains("! error[E0308]"), "{plain_doc}");
        assert!(
            plain_doc.contains("MENSURA · TOKEN GOVERNOR"),
            "{plain_doc}"
        );
        assert!(plain_doc.contains("compressed outputs"), "{plain_doc}");
        assert!(plain_dock.contains("›"), "{plain_dock}");
        assert!(plain_dock.contains("enter send"), "{plain_dock}");
        assert!(plain_dock.contains("47k/200k"), "{plain_dock}");
        assert!(plain_dock.contains("Δ3 +42 -11"), "{plain_dock}");
        assert!(plain_dock.contains("main"), "{plain_dock}");
    }

    #[test]
    fn apply_discovered_resources_keeps_cli_skill_paths() {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
        let extra = dir.path().join("extra-skill.md");
        std::fs::write(
            &extra,
            "---\nname: cli-skill\ndescription: from --skill\n---\n# Extra\n",
        )
        .unwrap();
        let mut agent = Agent::new("x");
        agent.cwd = dir.path().to_path_buf();
        let parsed = Args {
            skills: vec![extra.display().to_string()],
            project_trust_override: Some(false),
            ..Args::default()
        };
        apply_discovered_resources(&parsed, &mut agent);
        assert!(
            agent.skills.iter().any(|skill| skill.name == "cli-skill"),
            "CLI --skill paths must survive reload/rebind: {:?}",
            agent
                .skills
                .iter()
                .map(|skill| &skill.name)
                .collect::<Vec<_>>()
        );
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }

    #[test]
    fn suspend_dry_run_sets_status() {
        let theme = builtin_themes().into_iter().next().expect("theme");
        let mut session = InteractiveSession::new(theme, "pi", vec!["google/gemini".into()]);
        std::env::set_var("PI_SUSPEND_DRY_RUN", "1");
        apply_suspend(&mut session, false);
        std::env::remove_var("PI_SUSPEND_DRY_RUN");
        if cfg!(windows) {
            assert_eq!(
                session.chrome.status,
                "Suspend to background is not supported on Windows"
            );
        } else {
            assert_eq!(session.chrome.status, "Suspended");
        }
    }

    #[test]
    fn sqlite_session_backend_upserts_created_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("sessions");
        std::env::set_var("PI_SESSION_BACKEND", "sqlite");
        let session = JsonlSession::create(&session_dir, "/tmp/work", Some("sqlite-demo")).unwrap();
        persist_selected_backend(&session, &session_dir);
        std::env::remove_var("PI_SESSION_BACKEND");
        let store =
            pi_session_sqlite::SqliteSessionStore::open(&session_dir.join("sessions.db")).unwrap();
        let listed = store.list_sessions(None).unwrap();
        assert!(listed.iter().any(|item| item.id == session.header.id));
    }

    fn test_session() -> (Args, Agent, InteractiveSession) {
        let theme = builtin_themes().into_iter().next().expect("theme");
        (
            Args::default(),
            Agent::new("sys"),
            InteractiveSession::new(theme, "pi", vec!["anthropic/claude-fable-5".into()]),
        )
    }

    #[test]
    fn bare_login_opens_auth_type_selector() {
        let (parsed, mut agent, mut session) = test_session();
        handle_user_line(&parsed, &mut agent, &mut session, "/login", None).unwrap();
        let selector = session
            .chrome
            .extension_selector
            .as_ref()
            .expect("auth-type selector");
        assert_eq!(selector.title, "Select authentication method:");
        assert!(selector
            .options
            .iter()
            .any(|item| item == "Sign in with an account"));
        assert!(selector
            .options
            .iter()
            .any(|item| item == "Sign in with an API key"));
    }

    #[test]
    fn login_anthropic_opens_auth_type_when_both_methods_exist() {
        let (parsed, mut agent, mut session) = test_session();
        handle_user_line(&parsed, &mut agent, &mut session, "/login anthropic", None).unwrap();
        let selector = session
            .chrome
            .extension_selector
            .as_ref()
            .expect("provider auth-type");
        assert_eq!(
            selector.title,
            "Select authentication method for Anthropic:"
        );
        assert_eq!(session.login_auth_options.len(), 2);
        assert!(session.login_auth_type_labels.is_some());
    }

    #[test]
    fn login_bedrock_opens_api_key_dialog() {
        let (parsed, mut agent, mut session) = test_session();
        handle_user_line(
            &parsed,
            &mut agent,
            &mut session,
            "/login amazon-bedrock",
            None,
        )
        .unwrap();
        assert!(session.chrome.login_dialog.is_some());
        assert!(session.chrome.extension_selector.is_none());
        let rendered = session
            .chrome
            .login_dialog
            .as_ref()
            .unwrap()
            .render(80)
            .join("\n");
        assert!(rendered.contains("AWS profile") || rendered.contains("Enter API key"));
    }

    #[test]
    fn ambient_login_shows_configured_outside_dialog() {
        let theme = builtin_themes().into_iter().next().expect("theme");
        let mut session = InteractiveSession::new(theme, "pi", vec!["google/gemini".into()]);
        start_provider_login(&mut session, "custom-ambient", "api_key", None).unwrap();
        let dialog = session
            .chrome
            .login_dialog
            .as_ref()
            .expect("ambient dialog");
        let rendered = dialog.render(80).join("\n");
        assert!(rendered.contains(&format!("Authentication is configured outside {APP_NAME}.")));
    }

    #[test]
    fn bare_logout_without_stored_credentials_matches_ts() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        let (parsed, mut agent, mut session) = test_session();
        handle_user_line(&parsed, &mut agent, &mut session, "/logout", None).unwrap();
        std::env::remove_var("PI_CODING_AGENT_DIR");
        assert!(session.chrome.oauth_selector.is_none());
        assert!(session
            .chrome
            .status
            .contains("No stored credentials to remove"));
    }

    #[test]
    fn print_mode_exits_nonzero_on_assistant_error() {
        let message = AssistantMessage {
            id: "m1".into(),
            role: "assistant".into(),
            content: vec![],
            model: "x".into(),
            usage: None,
            stop_reason: Some(StopReason::Error),
            error_message: Some("provider failure".into()),
        };
        let events = vec![AgentEvent::MessageUpdate {
            message: std::sync::Arc::new(pi_ai::ChatMessage::text("assistant", "")),
            assistant_message_event: pi_ai::AssistantMessageEvent::Error {
                reason: StopReason::Error,
                error: message,
            },
        }];
        assert_eq!(
            print_text_exit(&events),
            (1, Some("provider failure".into()))
        );
        let aborted = AssistantMessage {
            id: "m2".into(),
            role: "assistant".into(),
            content: vec![],
            model: "x".into(),
            usage: None,
            stop_reason: Some(StopReason::Aborted),
            error_message: None,
        };
        let events = vec![AgentEvent::MessageUpdate {
            message: std::sync::Arc::new(pi_ai::ChatMessage::text("assistant", "")),
            assistant_message_event: pi_ai::AssistantMessageEvent::Error {
                reason: StopReason::Aborted,
                error: aborted,
            },
        }];
        assert_eq!(
            print_text_exit(&events),
            (1, Some("Request aborted".into()))
        );
    }

    #[test]
    fn print_json_event_strips_partial_and_adds_toolcall_ids() {
        let message = AssistantMessage {
            id: "m1".into(),
            role: "assistant".into(),
            content: vec![ContentBlock::ToolCall {
                id: "call-1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({}),
            }],
            model: "x".into(),
            usage: None,
            stop_reason: None,
            error_message: None,
        };
        let json = to_json_print_event(&AgentEvent::MessageUpdate {
            message: std::sync::Arc::new(pi_ai::ChatMessage::text("assistant", "")),
            assistant_message_event: pi_ai::AssistantMessageEvent::ToolcallStart {
                content_index: 0,
                partial: message,
            },
        })
        .unwrap();
        assert_eq!(json["type"], "message_update");
        assert!(json["assistantMessageEvent"].get("partial").is_none());
        assert_eq!(json["assistantMessageEvent"]["id"], "call-1");
        assert_eq!(json["assistantMessageEvent"]["toolName"], "bash");
        assert_eq!(json["assistantMessageEvent"]["type"], "toolcall_start");
    }

    #[test]
    fn print_json_fast_path_matches_full_serialization_minus_partial() {
        use pi_ai::AssistantMessageEvent as Ev;
        let partial = AssistantMessage {
            id: "m1".into(),
            role: "assistant".into(),
            content: vec![ContentBlock::Text { text: "big".into() }],
            model: "x".into(),
            usage: None,
            stop_reason: None,
            error_message: None,
        };
        let tool_call = ContentBlock::ToolCall {
            id: "call-1".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": "a"}),
        };
        let events = vec![
            Ev::Start {
                partial: partial.clone(),
            },
            Ev::TextStart {
                content_index: 0,
                partial: partial.clone(),
            },
            Ev::TextDelta {
                content_index: 0,
                delta: "d".into(),
                partial: partial.clone(),
            },
            Ev::TextEnd {
                content_index: 0,
                content: "c".into(),
                partial: partial.clone(),
            },
            Ev::ThinkingStart {
                content_index: 1,
                partial: partial.clone(),
            },
            Ev::ThinkingDelta {
                content_index: 1,
                delta: "d".into(),
                partial: partial.clone(),
            },
            Ev::ThinkingEnd {
                content_index: 1,
                content: "c".into(),
                partial: partial.clone(),
            },
            Ev::ToolcallDelta {
                content_index: 2,
                delta: "{".into(),
                partial: partial.clone(),
            },
            Ev::ToolcallEnd {
                content_index: 2,
                tool_call,
                partial: partial.clone(),
            },
        ];
        for event in events {
            let fast = to_json_print_event(&AgentEvent::MessageUpdate {
                message: std::sync::Arc::new(pi_ai::ChatMessage::text("assistant", "")),
                assistant_message_event: event.clone(),
            })
            .unwrap();
            let mut reference = serde_json::to_value(&event).unwrap();
            reference.as_object_mut().unwrap().remove("partial");
            assert_eq!(
                fast["assistantMessageEvent"], reference,
                "fast path diverged for {reference:?}"
            );
        }
    }

    #[test]
    fn json_mode_help_takes_over_stdout() {
        let help = parse_args(&["--help".into()]);
        assert!(!should_take_over_stdout(&help));
        let json_help = parse_args(&["--mode".into(), "json".into(), "--help".into()]);
        assert!(should_take_over_stdout(&json_help));
        let print_help = parse_args(&["-p".into(), "--help".into()]);
        assert!(should_take_over_stdout(&print_help));
    }

    #[test]
    fn apply_resolved_models_fuzzy_and_thinking_suffix() {
        let parsed = Args {
            provider: Some("anthropic".into()),
            model: Some("sonnet:high".into()),
            ..Args::default()
        };
        let mut agent = Agent::new(default_system_prompt());
        apply_resolved_models(&parsed, &mut agent).expect("resolve");
        assert!(agent.model_id.to_ascii_lowercase().contains("sonnet"));
        assert_eq!(agent.thinking_level, pi_protocol::ThinkingLevel::High);
        assert_eq!(agent.provider, "anthropic");
    }

    #[test]
    fn apply_resolved_models_unknown_is_error() {
        let parsed = Args {
            model: Some("definitely-not-a-real-model-xyz".into()),
            ..Args::default()
        };
        let mut agent = Agent::new(default_system_prompt());
        let err = apply_resolved_models(&parsed, &mut agent).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn resume_fixture_opens_selected_session() {
        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), "/tmp/resume-pick", Some("picked")).unwrap();
        std::env::set_var("PI_RESUME_SESSION", session.path.display().to_string());
        let parsed = Args {
            resume: true,
            ..Args::default()
        };
        let selected = select_resume_session(&parsed, dir.path(), Path::new("/tmp/resume-pick"))
            .expect("select");
        std::env::remove_var("PI_RESUME_SESSION");
        assert_eq!(selected.unwrap(), session.path);
    }
}
