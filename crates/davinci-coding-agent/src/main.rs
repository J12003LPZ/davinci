use davinci_coding_agent::agent_profiles;
use davinci_coding_agent::args;
mod auth_cmd;
#[cfg(test)]
mod browser_integration_tests;
mod cache_stats;
mod catalog_refresh;
mod changelog;
use davinci_coding_agent::completion_delivery;
mod codex_probe;
mod davinci_interactive;
mod davinci_sources;
mod davinci_surfaces;
#[cfg(all(unix, feature = "experimental-ipc"))]
mod experimental;
#[cfg(test)]
mod process_manager_integration_tests;
#[cfg(test)]
mod test_impact_integration_tests;
mod voice_input;
mod voice_models;
#[cfg(not(all(unix, feature = "experimental-ipc")))]
#[allow(dead_code)]
mod experimental {
    use crate::args::{parse_args, Args};
    pub fn is_experimental_command(command: Option<&str>) -> bool {
        matches!(command, Some("server") | Some("client"))
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
        Err("experimental IPC is not available in this build".into())
    }
    pub fn run_client(_: ClientCommand) -> Result<String, String> {
        Err("experimental IPC is not available in this build".into())
    }
    pub fn parse_experimental_cli(raw: &[String]) -> Result<ExperimentalCli, Vec<String>> {
        match raw.first().map(String::as_str) {
            Some("server") => Ok(ExperimentalCli::Server {
                listen: vec![],
                auth: None,
            }),
            Some("client") => Ok(ExperimentalCli::Client {
                connect: None,
                auth: None,
            }),
            _ => Ok(ExperimentalCli::Pi {
                options: parse_args(raw),
                listen: vec![],
            }),
        }
    }
}

mod export;
mod extension_host;
use davinci_coding_agent::interaction_testing;
mod extensions;
mod external_editor;
mod file_processor;
use davinci_coding_agent::hooks;
mod image_convert;
mod js_host;
mod llama;
mod mcp;
mod migrations;
mod model_resolver;
use davinci_coding_agent::native_extensions;
use davinci_coding_agent::native_tools;
mod output;
use davinci_coding_agent::package_source;
mod packages;
use davinci_coding_agent::permissions;
use davinci_coding_agent::project_config;
mod rpc;
use davinci_coding_agent::runtime_host;
mod self_update;
use davinci_coding_agent::settings;
use davinci_coding_agent::semantic;
mod shutdown;
mod slash;
mod startup;
mod tools_manager;
use davinci_coding_agent::trust;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use davinci_coding_agent::prompt_host;

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

use davinci_agent::{
    discover_prompt_templates, discover_skills, env_summarizer, load_context_files, Agent,
    AgentEvent, CompleteOutput, CustomToolExecutor, EventSink, SummarizeRequest, SummarizeResponse,
    Summarizer,
};
use davinci_ai::{
    apply_config_auth_with_shell, apply_models_config, check_auth, complete_simple, content_text,
    find_model, format_no_api_key_found_message, format_no_model_selected_message,
    format_no_models_available_message, format_oauth_auth_failed_message, fuzzy_models,
    get_supported_thinking_levels, live_complete_streaming_with_sink_envelope, load_builtin_models,
    models_json_path, resolve_provider_auth, snapshot_availability, AssistantMessage, AuthStorage,
    ContentBlock, Credential, CredentialKind, ModelConfig, ModelRuntimeSnapshot, ResolvedAuth,
    StopReason, StreamOptions, ToolSpec, NO_MODELS_AVAILABLE, PROVIDER_SPECS,
};
use davinci_coding_agent::interactive_tui::{
    create_interactive_tui, handle_copy_command, remount_chrome_panes, stop_interactive_tui,
    switch_tui_mode, ChromePanes, CopyCommandResult, InteractiveTui, InteractiveTuiOptions,
};
use davinci_session::{
    default_agent_dir, discover_sessions, encode_header, latest_session, now_ms,
    resolve_session_dir_from, resolve_session_ref, JsonlSession, SessionEntry,
};
use davinci_tui::{
    builtin_themes, collect_name_collisions, copy_text, detect_terminal_theme,
    detect_terminal_theme_for_auto, drain_osc_tty, encode_kitty, format_collision_diagnostic,
    format_context_path, format_display_path, infer_source_info, interactive_settings_list,
    load_themes_from_dir, parse_auto_theme, parse_http_idle_timeout, resolve_git_branch,
    theme_files_from_dir, AuthSelectorMode, AuthSelectorProvider, AutocompleteItem, ChatChrome,
    Component, CustomMessage, DoubleEscapeAction, ExtraAutocompleteProvider, FilterMode,
    InteractiveSession, Keybindings, LiveAutocompleteQuery, LoadedResourceItem, MermaidMode,
    ModelSelectorItem, ScopedModel, SessionAction, SessionItem, SessionTreeEntry, SlashCommandSpec,
    Theme, ThemeDetection, ToolCard, TrustUpdate, TuiMode, FALLBACK_PREVIEW_LINES,
    OSC_QUERY_TIMEOUT_MS,
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
use trust::{has_trust_requiring_project_resources, ProjectTrustStore, ProjectTrustUpdate};

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
/// real terminal. The live shell is `davinci_interactive::run`.
fn run_davinci_screens(raw: &[String]) -> Result<i32, String> {
    use davinci_tui::davinci;

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
    let session_dir = davinci_session::default_session_dir();
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
    if raw.as_slice() == ["--internal-process-supervisor"]
        && std::env::var("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").as_deref() == Ok("1")
    {
        davinci_agent::jobs::supervisor::run();
    }
    match run(raw) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}

/// Appends `<ms since run start>  <stage>` to the file named by
/// `DAVINCI_STARTUP_TRACE`, so a slow start can be split into stages without a
/// profiler. Off, it costs one environment lookup per stage.
pub(crate) fn startup_mark(stage: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let Some(path) = std::env::var_os("DAVINCI_STARTUP_TRACE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{:>7} ms  {stage}", start.elapsed().as_millis());
    }
}

fn run(raw: Vec<String>) -> Result<i32, String> {
    startup_mark("start");
    apply_offline_mode(&raw);
    if let Some(result) = davinci_coding_agent::runtime_inspect::try_run(&raw) {
        return result;
    }
    let command_start = args::first_non_offline_argument(&raw).unwrap_or(0);
    if raw.get(command_start).map(String::as_str) == Some("voice") {
        return voice_models::cli(&raw[command_start + 1..]);
    }
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
    tools_manager::prepend_tools_bin_to_path();
    if is_package_command(raw.first().map(String::as_str)) {
        let command = raw[0].as_str();
        if raw.iter().any(|a| a == "--help" || a == "-h") {
            println!("{}", print_help());
            return Ok(0);
        }
        let agent_dir = default_agent_dir();
        packages::ensure_agent_dir(&agent_dir)?;
        let user_settings = load_settings(&agent_dir);
        apply_http_proxy_settings(user_settings.http_proxy.as_deref());
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
    if let Some(path) = parsed.codex_probe.as_deref() {
        codex_probe::run(&parsed, std::path::Path::new(path))?;
        return Ok(0);
    }

    let session_dir = resolved_session_dir(&parsed, &cwd);
    let migrations = migrations::maybe_run_startup_migrations(&cwd);
    let mut agent = match build_agent(&parsed, &session_dir, &cwd) {
        Err(err) if err == NO_SESSION_SELECTED => return Ok(0),
        other => other?,
    };
    startup_mark("agent built");
    // Evidence older than a week is nobody's: a `read` of its path would
    // have happened in the session that wrote it.
    if let Some(store) = &agent.evidence {
        let _ = store.sweep(davinci_agent::EVIDENCE_TTL);
    }

    if parsed.mode == Some(Mode::Rpc) {
        let _ = tools_manager::ensure_managed_tools();
        if !parsed.file_args.is_empty() {
            eprintln!("{RPC_FILE_ARGS_ERROR}");
            return Ok(1);
        }
        let code = run_rpc(&parsed, &mut agent);
        run_stop_hooks_for(&parsed, &agent.cwd);
        return code;
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
        let code = run_print(&parsed, &mut agent);
        run_stop_hooks_for(&parsed, &agent.cwd);
        return code;
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

/// The request-local identity for the model actually serving the turn.
///
/// A documented divergence from vendor `pi`: `buildSystemPrompt`
/// (`vendor/pi/packages/coding-agent/src/core/system-prompt.ts`) carries cwd,
/// tools, context files and skills but no model identity, so "what model are
/// you?" was answered from the model's training prior rather than from the run.
/// The suffix is refreshed for every run and appended to the current turn
/// prompt at dispatch, so prompt preparation and extension overrides cannot
/// leave the provider request with a stale prompt body.
fn provider_identity(agent: &Agent) -> Option<String> {
    if agent.provider.is_empty() || agent.model_id.is_empty() {
        return None;
    }
    Some(format!(
        "You are running as {}/{} (thinking: {}).",
        agent.provider,
        agent.model_id,
        agent.thinking_level.as_str()
    ))
}

fn synchronize_provider_system_prompt(agent: &mut Agent) {
    let suffix = provider_identity(agent);
    agent.set_provider_system_prompt_suffix(suffix);
}

/// The reply an offline run gives. `PI_OFFLINE_TOOL_CALL` is a fixture —
/// `{"name": "bash", "arguments": {"command": "git status"}}` — that makes the
/// first reply of a turn that tool call, so the tool path and the permission
/// question in front of it can be driven without a provider; the reply that
/// follows the tool result is the usual character-count stub. An array supplies
/// up to 32 sequential calls, advancing only after successful tool results.
fn offline_stub_message(current: &Agent, last_user: usize) -> AssistantMessage {
    let scripted = std::env::var("PI_OFFLINE_TOOL_CALL")
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|fixture| {
            let is_prompt = |message: &davinci_ai::ChatMessage| {
                message.role == "user" && !message.extra.contains_key("davinciCapabilityReminder")
            };
            let Some(calls) = fixture.as_array() else {
                return current
                    .messages
                    .last()
                    .is_some_and(is_prompt)
                    .then_some(fixture);
            };
            if calls.len() > 32 {
                return None;
            }
            let start = current.messages.iter().rposition(is_prompt)?;
            let turn = &current.messages[start..];
            let last = turn
                .iter()
                .rev()
                .find(|message| !message.extra.contains_key("davinciCapabilityReminder"))?;
            if !is_prompt(last) && last.role != "toolResult" {
                return None;
            }
            if turn
                .iter()
                .any(|message| message.role == "toolResult" && message.is_error == Some(true))
            {
                return None;
            }
            let completed = turn
                .iter()
                .filter(|message| message.role == "toolResult")
                .count();
            calls.get(completed).cloned()
        });
    let (content, stop_reason) = match scripted {
        Some(call) => (
            vec![ContentBlock::ToolCall {
                id: format!("call_{}", davinci_agent::new_message_id()),
                name: call
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("bash")
                    .to_string(),
                arguments: call
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            }],
            StopReason::ToolUse,
        ),
        None => (
            vec![ContentBlock::Text {
                text: format!("(offline) received {last_user} characters"),
            }],
            StopReason::Stop,
        ),
    };
    AssistantMessage {
        id: davinci_agent::new_message_id(),
        role: "assistant".into(),
        content,
        model: format!("{}/{}", current.provider, current.model_id),
        usage: None,
        stop_reason: Some(stop_reason),
        error_message: None,
    }
}

fn build_agent(parsed: &Args, session_dir: &Path, cwd: &Path) -> Result<Agent, String> {
    let graph_worker = crate::native_extensions::graph_worker_context();
    if std::env::var_os("PI_GRAPH_ROLE").is_some() && graph_worker.is_none() {
        return Err("invalid Graph worker context; refusing ordinary-session fallback".into());
    }
    let worker_runtime = crate::native_extensions::graph::worker_sessions::runtime_from_env(
        parsed,
        cwd,
        graph_worker.as_ref(),
    )?;
    if worker_runtime.is_some() && parsed.no_session {
        return Err("bound Graph worker cannot disable its conversation".into());
    }
    let settings = load_merged_settings_with_override(
        &default_agent_dir(),
        cwd,
        parsed.project_trust_override,
    );
    // set_var is only sound before other threads exist. These values are
    // resolved from trust-aware merged settings before MCP or extension
    // runners can spawn any background work.
    apply_http_proxy_settings(settings.http_proxy.as_deref());
    crate::settings::apply_web_search_settings(settings.web_search.as_ref());
    if let Some(ms) = settings.websocket_connect_timeout_ms {
        std::env::set_var("PI_WEBSOCKET_CONNECT_TIMEOUT_MS", ms.to_string());
    }
    if let Some(value) = settings.openai_verbosity.as_deref() {
        std::env::set_var("DAVINCI_OPENAI_VERBOSITY", value);
    }
    if let Some(value) = settings.reasoning_summary.as_deref() {
        std::env::set_var("DAVINCI_REASONING_SUMMARY", value);
    }
    if let Some(value) = settings.graph_economy_model.as_deref() {
        std::env::set_var("DAVINCI_GRAPH_ECONOMY_MODEL", value);
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
    let env_profile = std::env::var("DAVINCI_PROMPT_PROFILE")
        .ok()
        .or_else(|| std::env::var("PI_PROMPT_PROFILE").ok());
    let profile = crate::settings::resolve_prompt_profile(
        parsed.prompt_profile,
        settings.prompt_profile.as_deref(),
        env_profile.as_deref(),
    )?
    .profile;

    let mut agent = if let Some(custom) = &parsed.system_prompt {
        let mut session = davinci_agent::PromptSessionState::custom(custom);
        for extra in &parsed.append_system_prompt {
            let text = if Path::new(extra).exists() {
                std::fs::read_to_string(extra).map_err(|err| err.to_string())?
            } else {
                extra.clone()
            };
            session.append(text);
        }
        let ctx = davinci_agent::prompt::PromptContext {
            provider: parsed.provider.as_deref().unwrap_or(""),
            model_id: parsed.model.as_deref().unwrap_or(""),
            permission_mode: parsed
                .permission_mode
                .unwrap_or(davinci_agent::PermissionMode::Ask),
            plan_active: false,
        };
        let comp = session.render_and_record(&ctx);
        let mut agent = Agent::new(&comp.text);
        agent.prompt_manifest = Some(comp.manifest);
        agent.prompt_session = session;
        agent
    } else {
        let mut session = davinci_agent::PromptSessionState::builtin(profile);
        for extra in &parsed.append_system_prompt {
            let text = if Path::new(extra).exists() {
                std::fs::read_to_string(extra).map_err(|err| err.to_string())?
            } else {
                extra.clone()
            };
            session.append(text);
        }
        let ctx = davinci_agent::prompt::PromptContext {
            provider: parsed.provider.as_deref().unwrap_or(""),
            model_id: parsed.model.as_deref().unwrap_or(""),
            permission_mode: parsed
                .permission_mode
                .unwrap_or(davinci_agent::PermissionMode::Ask),
            plan_active: false,
        };
        let comp = session.render_and_record(&ctx);
        let mut agent = Agent::new_builtin(profile);
        agent.system_prompt = comp.text;
        agent.prompt_manifest = Some(comp.manifest);
        agent.prompt_session = session;
        agent
    };
    if let Some(level) = parsed.thinking {
        agent.thinking_level = level;
    } else if let Some(level) = settings
        .default_thinking_level
        .as_deref()
        .and_then(davinci_protocol::ThinkingLevel::parse)
    {
        agent.thinking_level = level;
    }
    let explicit_tool_selection = parsed.no_tools
        || parsed.no_builtin_tools
        || !parsed.tools.is_empty()
        || settings.default_tools.is_some();
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
    if settings.decision_intelligence_enabled() {
        match AuthStorage::create()
            .map_err(|error| error.to_string())
            .and_then(|auth| {
                davinci_coding_agent::decision_providers::typesafe::TypeSafeProvider::from_auth(
                    &auth,
                )
                .map_err(|error| error.to_string())
            }) {
            Ok(Some(provider)) => {
                let runtime = Arc::new(davinci_agent::decision::DecisionRuntime::new(provider));
                runtime.enable();
                agent.set_decision_runtime(runtime);
            }
            Ok(None) => eprintln!(
                "TypeSafe decision intelligence is enabled but has no credential; remaining off"
            ),
            Err(error) => eprintln!("TypeSafe decision intelligence unavailable: {error}"),
        }
    }
    // The product default is `ask`; the library default (every tool runs)
    // is only for embedders. Who answers an ask is the mode's business:
    // davinci, RPC and the legacy chrome each install an approver, and a
    // `--print` run fails closed.
    agent.permissions = Arc::new(davinci_agent::PermissionState::new(
        permissions::policy_for(
            &default_agent_dir(),
            cwd,
            parsed.project_trust_override,
            parsed.permission_mode,
        ),
    ));
    agent.tool_context.cache = davinci_agent::runtime::cache::CacheRuntime::shared(
        settings.cache.clone().unwrap_or_default(),
        default_agent_dir(),
    );
    agent.tool_context.transactions_disabled = settings
        .editing_transactions
        .as_ref()
        .is_some_and(|config| !config.enabled);
    if agent.tool_context.transactions_disabled {
        agent
            .tools
            .retain(|name| !davinci_agent::runtime::transactions::is_tool(name));
        agent
            .tool_registry
            .retain(|name| !davinci_agent::runtime::transactions::is_tool(name));
    }
    agent.tool_context.semantic = Some(Arc::new(
        davinci_coding_agent::semantic::NativeSemanticService::with_permissions_and_cache(
            agent.permissions.clone(),
            agent.tool_context.cache.clone(),
        ),
    ));
    agent.tool_context.foreground_supervisor = std::env::current_exe().ok().map(|executable| {
        davinci_agent::jobs::supervisor::SupervisorCommand {
            executable,
            argv: vec!["--internal-process-supervisor".into()],
        }
    });
    if settings
        .process_manager
        .as_ref()
        .is_none_or(|config| config.enabled)
    {
        let manager = std::env::current_exe()
            .map_err(|_| "process supervisor host unavailable".to_string())
            .and_then(|executable| {
                davinci_agent::process_manager::ProcessManager::new(
                    cwd,
                    agent.tool_context.jobs.clone(),
                    agent.permissions.clone(),
                    davinci_agent::jobs::supervisor::SupervisorCommand {
                        executable,
                        argv: vec!["--internal-process-supervisor".into()],
                    },
                )
            });
        match manager {
            Ok(manager) => {
                agent.tool_context.processes = Some(manager.with_counters(agent.counters.clone()))
            }
            Err(error) => eprintln!("Managed processes unavailable: {error}"),
        }
    }
    if agent.tool_context.processes.is_none() {
        agent
            .tools
            .retain(|name| !davinci_agent::tools::is_managed_process_tool(name));
        agent
            .tool_registry
            .retain(|name| !davinci_agent::tools::is_managed_process_tool(name));
    }
    let trusted = is_trusted(&settings, cwd, parsed.project_trust_override);
    if !parsed.no_mcp {
        agent.attach_mcp(davinci_agent::McpRegistry::connect(
            &mcp::load(&default_agent_dir(), cwd, trusted),
            cwd,
        ));
    }
    // Overflowing batch output is kept where the model can `read` it
    // back, under the agent dir so a fixture dir keeps tests contained.
    agent.evidence = Some(davinci_agent::EvidenceStore::new(
        default_agent_dir().join("evidence"),
    ));
    let parsed_for_worker = parsed.clone();
    let cwd_for_worker = cwd.to_path_buf();
    let mcp_for_worker = agent.tool_context.mcp.clone();
    agent.subagent_runner = Some(davinci_agent::SubagentRunner::new(move |req| {
        if let Ok(fix) = std::env::var("PI_SUBAGENT_FIXTURE") {
            let path = std::path::Path::new(&fix);
            if path.is_file() {
                return std::fs::read_to_string(path).map_err(|err| err.to_string());
            }
            return Ok(fix);
        }
        run_nested_subagent(&parsed_for_worker, &cwd_for_worker, &mcp_for_worker, req)
    }));
    apply_discovered_resources(parsed, &mut agent);
    if let Some(runtime) = worker_runtime {
        agent.set_runtime(runtime);
    }
    if !parsed.no_session {
        agent.load_from_session(resolve_or_create_session(parsed, session_dir, cwd)?)?;
        // A resumed session opens on the ledger it closed on, in every
        // mode; davinci re-reads it to draw the rows.
        agent.restore_todos();
    }
    startup_mark("session opened");
    agent.auto_compaction = settings.compaction_enabled();
    agent.compaction = settings.compaction_settings();
    agent.auto_retry = settings.retry_enabled();
    agent.retry_attempts = settings.retry_max_retries();
    agent.retry_base_delay_ms = settings.retry_base_delay_ms();
    apply_max_model_turns(&mut agent, &settings);
    agent.provider_timeout_ms = settings.provider_timeout_ms();
    agent.provider_max_retries = settings.provider_max_retries();
    agent.provider_max_retry_delay_ms = settings.provider_max_retry_delay_ms();
    agent.thinking_budgets = settings.thinking_budgets.clone();
    agent.block_images = settings.block_images();
    agent.auto_resize_images = settings.image_auto_resize();
    agent.transport = settings.transport.clone();
    agent.install_telemetry = settings.install_telemetry_enabled();
    let mut extensions = if parsed.no_extensions {
        parsed.extensions.clone()
    } else {
        let mut extensions = settings.extensions.clone();
        extensions.extend(parsed.extensions.clone());
        extensions
    };
    for pkg in &settings.packages {
        if !parsed.no_extensions {
            for path in settings::collect_package_resources(pkg, "extensions", &default_agent_dir(), cwd) {
                extensions.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let mut host = ExtensionHost::load_with_cwd(&default_agent_dir(), &extensions, cwd);
    startup_mark("extensions loaded");
    let native_names = host.native_tool_names();
    let mut names = native_names.clone();
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
    sync_visual_verification_availability(&mut agent, &host);
    if let Some(graph_worker) = &graph_worker {
        agent.tool_context.transaction_owner.graph_node = Some(graph_worker.node_id.clone());
        if let Some(id) = std::env::var_os("DAVINCI_AGENT_ID") {
            agent.tool_context.transaction_owner.agent_id = id
                .to_str()
                .ok_or("invalid Graph worker agent identity")?
                .parse()
                .map_err(|_| "invalid Graph worker agent identity")?;
        }
        // The parent-owned graph contract is the authorization surface. The
        // initial provider projection is a separate, smaller view: a worker
        // may discover any authorized deferred schema through tool_search,
        // but it must not receive that schema in its first request.
        let authorized_tools: Vec<String> = graph_worker
            .allowed_tools
            .iter()
            .filter(|tool| !parsed.exclude_tools.contains(tool))
            .filter(|tool| {
                !agent.tool_context.transactions_disabled
                    || !davinci_agent::runtime::transactions::is_tool(tool)
            })
            .cloned()
            .collect();
        let initial_source: Vec<String> = match std::env::var("PI_GRAPH_INITIAL_TOOLS") {
            Ok(raw) => raw
                .split(',')
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .map(str::to_string)
                .collect(),
            Err(_) => parsed.tools.clone(),
        };
        let initial_tools: Vec<String> = initial_source
            .into_iter()
            .filter(|tool| authorized_tools.contains(tool))
            .collect();
        agent.tools = authorized_tools;
        agent.sync_tool_authorization();
        *agent
            .tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            davinci_agent::runtime::ToolExposureState::new(initial_tools);
    } else if explicit_tool_selection {
        agent.expose_active_tools();
    } else {
        agent.sync_tool_authorization();
    }
    if let Some(coord) = davinci_agent::runtime::task_transport::TaskCoordinatorClient::from_env() {
        agent.tool_context.task_coordinator = Some(coord);
    }
    if let Ok(raw_contract) = std::env::var("DAVINCI_TASK_CONTRACT_JSON") {
        let contract: davinci_agent::runtime::TaskContract = serde_json::from_str(&raw_contract)
            .map_err(|err| format!("invalid inherited task contract: {err}"))?;
        contract
            .validate()
            .map_err(|err| format!("invalid inherited task contract: {err}"))?;
        agent.set_active_contract(contract);
    }
    attach_tool_executor(&mut agent, &host);
    host.emit(ExtensionEvent::SessionStart);
    let _ = host.describe_js();
    apply_resolved_models(parsed, &mut agent)?;
    startup_mark("models resolved");
    let ctx = davinci_agent::prompt::PromptContext {
        provider: &agent.provider,
        model_id: &agent.model_id,
        permission_mode: agent.permission_mode(),
        plan_active: agent.is_plan_mode(),
    };
    let comp = agent.prompt_session.render_and_record(&ctx);
    agent.system_prompt = comp.text;
    agent.prompt_manifest = Some(comp.manifest);
    if agent.prompt_session.is_builtin() {
        agent.persist_prompt_session()?;
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

pub(crate) fn resolve_model_and_auth(
    parsed: &Args,
    provider: &str,
    model_id: &str,
) -> Result<(davinci_ai::Model, ResolvedAuth), String> {
    let offline = parsed.offline
        || matches!(
            std::env::var("PI_OFFLINE").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    if offline {
        return Err("Provider request failed: offline".into());
    }

    let models = available_models(parsed);
    let model = find_model(&models, provider, model_id)
        .cloned()
        .or_else(|| {
            models
                .iter()
                .find(|item| item.provider == provider && item.id == model_id)
                .cloned()
        })
        .ok_or_else(|| format!("No model available for {provider}/{model_id}"))?;

    let mut storage = AuthStorage::create().ok();
    if let (Some(storage), Some(key)) = (storage.as_mut(), parsed.api_key.as_deref()) {
        storage.set_runtime_override(provider, key);
    }
    ensure_supported_stored_oauth(storage.as_ref(), provider, parsed.api_key.as_deref())?;
    if let Some(storage) = storage.as_mut() {
        maybe_refresh_auth(storage, provider, now_ms(), OAUTH_MIN_VALIDITY_MS, false);
    }

    let env = std::env::vars().collect();
    let mut auth = storage
        .as_ref()
        .and_then(|storage| resolve_provider_auth(provider, storage, &env, true))
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
        provider,
        Some(&model),
        &env,
        shell_path.as_deref(),
    );
    if auth.api_key.is_none() && auth.headers.is_empty() && auth.source == "none" {
        return Err(format!("No credentials available for {provider}"));
    }
    Ok((model, auth))
}

fn complete_simple_summarization(
    parsed: &Args,
    request: &SummarizeRequest,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
    max_retry_delay_ms: Option<u64>,
    thinking_level: davinci_protocol::ThinkingLevel,
    thinking_budgets: Option<davinci_ai::ThinkingBudgets>,
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
    ensure_supported_stored_oauth(
        storage.as_ref(),
        &request.provider,
        parsed.api_key.as_deref(),
    )
    .map_err(|error| format!("Summarization failed: {error}"))?;
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
        thinking_level: if model.reasoning && thinking_level != davinci_protocol::ThinkingLevel::Off
        {
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
        cache_key: None,
        cache_retention: Some("none".into()),
        native_responses_resume: None,
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
        let created = davinci_session::JsonlSessionRepo::new(session_dir)
            .create(davinci_session::JsonlCreateOptions {
                id: Some(id.clone()),
                cwd: cwd.to_string_lossy().into_owned(),
                parent_session_id: None,
                metadata: parsed
                    .name
                    .as_deref()
                    .and_then(args::normalize_session_name)
                    .map(|name| serde_json::json!({ "name": name })),
            })
            .map_err(|err| err.to_string())?;
        let session = JsonlSession::open(&created.info.path).map_err(|err| err.to_string())?;
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
    Ok(session)
}

fn available_models(parsed: &Args) -> Vec<davinci_ai::Model> {
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
                        .and_then(|level| davinci_protocol::ThinkingLevel::parse(level))
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

fn load_available_models(parsed: &Args) -> (Vec<davinci_ai::Model>, Option<String>) {
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

/// Everything `build_model_runtime` reads, reduced to a comparable value. The
/// environment is kept only as a hash so no credential is held in the cache.
#[derive(Debug, Clone, PartialEq)]
struct ModelRuntimeKey {
    args: String,
    cwd: PathBuf,
    env_hash: u64,
    files: Vec<(PathBuf, Option<(std::time::SystemTime, u64)>)>,
}

impl ModelRuntimeKey {
    fn current(parsed: &Args) -> Self {
        use std::hash::{Hash, Hasher};
        let agent_dir = default_agent_dir();
        let mut env: Vec<(String, String)> = std::env::vars().collect();
        env.sort();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        env.hash(&mut hasher);
        let stamp = |path: PathBuf| {
            let meta = std::fs::metadata(&path).ok();
            let stamp = meta.and_then(|meta| Some((meta.modified().ok()?, meta.len())));
            (path, stamp)
        };
        Self {
            args: format!(
                "{:?}|{:?}|{:?}|{}|{:?}",
                parsed.api_key,
                parsed.provider,
                parsed.model,
                parsed.no_extensions,
                parsed.extensions
            ),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env_hash: hasher.finish(),
            files: vec![
                stamp(davinci_ai::models_store_path(&agent_dir)),
                stamp(models_json_path(&agent_dir)),
                stamp(crate::settings::settings_path(&agent_dir)),
                stamp(davinci_ai::default_auth_path()),
                stamp(agent_dir.join("extensions")),
            ],
        }
    }

    /// The same question asked of possibly different file contents: an entry
    /// for these inputs is stale once any stamp or the environment moves.
    fn same_inputs(&self, other: &Self) -> bool {
        self.args == other.args
            && self.cwd == other.cwd
            && self
                .files
                .iter()
                .map(|(path, _)| path)
                .eq(other.files.iter().map(|(path, _)| path))
    }
}

/// Recent model runtime snapshots and the inputs each was built from.
///
/// Building one loads a throwaway extension host (the native extensions plus a
/// Node process per JavaScript extension) and re-parses the model catalog, and
/// one interactive start asked for it several times. A snapshot is reused only
/// while every input it read is unchanged; `/reload` clears the cache so edited
/// extension sources are read again, as they are at startup. A rebuild replaces
/// the stale entry for the same inputs, so an ordinary session keeps one entry.
static MODEL_RUNTIME_CACHE: Mutex<Vec<(ModelRuntimeKey, ModelRuntimeSnapshot)>> =
    Mutex::new(Vec::new());
const MODEL_RUNTIME_CACHE_ENTRIES: usize = 4;

pub(crate) fn clear_model_runtime_cache() {
    MODEL_RUNTIME_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

fn load_model_runtime(parsed: &Args) -> ModelRuntimeSnapshot {
    let key = ModelRuntimeKey::current(parsed);
    if let Some((_, snapshot)) = MODEL_RUNTIME_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|(cached, _)| *cached == key)
    {
        return snapshot.clone();
    }
    let snapshot = build_model_runtime(parsed);
    let mut cache = MODEL_RUNTIME_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|(cached, _)| !cached.same_inputs(&key));
    if cache.len() >= MODEL_RUNTIME_CACHE_ENTRIES {
        cache.remove(0);
    }
    cache.push((key, snapshot.clone()));
    snapshot
}

fn build_model_runtime(parsed: &Args) -> ModelRuntimeSnapshot {
    let mut models = load_builtin_models();
    let agent_dir = default_agent_dir();
    let store = davinci_ai::load_models_store(&agent_dir);
    for entry in store.providers.values() {
        models = davinci_ai::merge_models(&models, &entry.models);
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
        models.extend(davinci_ai::models_from_provider_config(
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
    let mut storage = AuthStorage::create().unwrap_or_else(|_| AuthStorage::in_memory());
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
    let davinci_docs = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/davinci/packages/coding-agent/docs");
    if davinci_docs.exists() {
        return davinci_docs;
    }
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

fn render_models_table(models: &[&davinci_ai::Model]) -> String {
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

fn latest_user_prompt(agent: &Agent) -> (String, Vec<davinci_ai::MessageContent>) {
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
        .filter(|block| matches!(block, davinci_ai::MessageContent::Image { .. }))
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

fn apply_max_model_turns(agent: &mut Agent, settings: &settings::Settings) {
    if let Some(max_model_turns) = settings.max_model_turns() {
        agent.max_model_turns = Some(max_model_turns);
    }
}

fn new_worker_agent(system_prompt: impl Into<String>) -> Agent {
    let mut agent = Agent::new(system_prompt);
    agent.max_model_turns = Some(60);
    agent
}

fn run_nested_subagent(
    parsed: &Args,
    cwd: &Path,
    mcp: &davinci_agent::McpRegistry,
    req: &davinci_agent::SubagentRequest,
) -> Result<String, String> {
    let settings = load_merged_settings(&default_agent_dir(), cwd);
    let trusted = is_trusted(&settings, cwd, parsed.project_trust_override);
    let profile = if let Some(agent_name) = &req.agent {
        let profiles = agent_profiles::discover_agent_profiles(cwd, None, trusted);
        let found = profiles.into_iter().find(|p| p.name == *agent_name);
        match found {
            Some(p) => Some(p),
            None => return Err(format!("Agent profile '{agent_name}' not found")),
        }
    } else {
        None
    };

    if let Some(profile) = &profile {
        let parent_mode = req
            .parent_permission_mode
            .unwrap_or(davinci_agent::PermissionMode::ReadOnly);
        agent_profiles::validate_profile_containment(profile, parent_mode)?;
    }

    let system_prompt = if let Some(profile) = &profile {
        format!(
            "{}\n{}",
            profile.system_prompt,
            davinci_agent::TOOL_USE_STRATEGY
        )
    } else {
        format!(
            "You are a scoped worker. Answer the prompt using only the tools you have. Do not call agent. \
             Reply with the answer itself — findings, file paths with line numbers, short quotes — not a narrative of what you did.\n{}",
            davinci_agent::TOOL_USE_STRATEGY
        )
    };
    let mut child = new_worker_agent(system_prompt);
    let effective_cwd = if let Some(wt) = &req.worktree_path {
        wt.as_path()
    } else {
        cwd
    };
    child.cwd = effective_cwd.to_path_buf();

    let mut tools = if let Some(profile) = &profile {
        if !profile.tools.is_empty() {
            profile.tools.clone()
        } else {
            req.tools.clone()
        }
    } else {
        req.tools.clone()
    };
    crate::native_extensions::token_governor::ensure_governor_recovery_tool(&mut tools);
    child.tools = tools.clone();
    child.tool_registry = tools;
    child.expose_active_tools();
    child.session = None;
    if let Some(runtime) = &req.runtime {
        if req.runtime_agent_id != Some(runtime.agent_id) || runtime.parent_agent_id.is_none() {
            return Err("worker runtime identity does not match its host request".into());
        }
        child.set_runtime(runtime.clone());
    } else if child.tools.iter().any(|tool| {
        matches!(
            tool.as_str(),
            "task_create" | "task_update" | "task_list" | "task_get"
        )
    }) {
        return Err("worker task tools require a parent coordinator".into());
    }
    // A worker's output is bounded by the parent; its own overflow has
    // nowhere useful to go.
    child.evidence = None;
    let child_mode = if let Some(profile) = &profile {
        davinci_agent::PermissionMode::parse(&profile.permission_mode)
            .unwrap_or(davinci_agent::PermissionMode::ReadOnly)
    } else if req.worktree_path.is_some() {
        davinci_agent::PermissionMode::Edits
    } else {
        davinci_agent::PermissionMode::ReadOnly
    };
    let mut policy = davinci_agent::PermissionPolicy::new(child_mode);
    if let Some(wt) = &req.worktree_path {
        policy.set_worktree_boundary(wt, Some(cwd));
    }
    child.permissions = Arc::new(davinci_agent::PermissionState::new(policy));
    child.approver = None;
    child.approval_responder = None;
    // `mcp_read` and read-only MCP tools need the parent's connections.
    child.tool_context.mcp = mcp.clone();
    child.abort_signal = req.abort.clone();
    apply_resolved_models(parsed, &mut child)?;

    let model_req = req.model_override.as_deref().or_else(|| {
        profile.as_ref().and_then(|p| {
            if p.model != "inherit" && !p.model.is_empty() {
                Some(p.model.as_str())
            } else {
                None
            }
        })
    });
    if let Some(target) = model_req {
        let parts: Vec<&str> = target.splitn(2, '/').collect();
        let (prov, mid) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            ("", parts[0])
        };
        let snapshot = load_model_runtime(parsed);
        if let Some(model) = davinci_ai::find_model(&snapshot.all, prov, mid) {
            child.provider = model.provider.clone();
            child.model_id = model.id.clone();
            child.context_window = model.context_window;
        }
    } else if let (Some(provider), Some(model_id)) = (&req.provider, &req.model_id) {
        if !model_id.is_empty() && (provider != &child.provider || model_id != &child.model_id) {
            let snapshot = load_model_runtime(parsed);
            if let Some(model) = davinci_ai::find_model(&snapshot.all, provider, model_id) {
                child.provider = model.provider.clone();
                child.model_id = model.id.clone();
                child.context_window = model.context_window;
            }
        }
    }

    if let Some(profile) = &profile {
        if let Some(max_tokens) = profile.max_context_tokens {
            child.context_window = max_tokens as u64;
        }
    }
    child.prompt(&req.prompt);
    let (text, events) = complete_prompt(parsed, &mut child);
    // A provider failure is prose too; report it as the failure it is
    // instead of handing the parent an "answer".
    let (code, error) = print_text_exit(&events);
    if code != 0 {
        return Err(error.unwrap_or_else(|| "subagent failed".into()));
    }
    if text.trim().is_empty() {
        Err("subagent returned no text".into())
    } else {
        Ok(text)
    }
}

fn complete_prompt(parsed: &Args, agent: &mut Agent) -> (String, Vec<AgentEvent>) {
    complete_prompt_with_host(parsed, agent, None, false)
}

fn provider_tools(agent: &Agent) -> Vec<ToolSpec> {
    agent
        .provider_tool_specs()
        .into_iter()
        .map(|tool| ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
            constrained_sampling: crate::experimental::experimental_tool_sampling(),
        })
        .collect()
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
            cred.kind == CredentialKind::Oauth && davinci_ai::credential_expires_by(cred, now_ms())
        });
    if expired_oauth {
        auth = None;
    }
    let fresh_host = existing_host.is_none();
    let host = existing_host.unwrap_or_else(|| Arc::new(Mutex::new(loaded_extension_host(parsed))));
    attach_shared_tool_executor(agent, host.clone());
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
            // An extension's prompt replaces the base, not the mode: plan
            // mode keeps its appendix.
            if agent.is_plan_mode()
                && agent.turn_context_placement()
                    == davinci_agent::turn_context::TurnContextPlacement::SystemPrompt
                && !agent
                    .system_prompt
                    .contains(davinci_agent::PLAN_MODE_APPENDIX)
            {
                agent.system_prompt.push_str("\n\n");
                agent
                    .system_prompt
                    .push_str(davinci_agent::PLAN_MODE_APPENDIX);
            }
            host.runtime_system_prompt = agent.system_prompt.clone();
        }
        for message in host.take_before_agent_start_messages() {
            agent.record_custom_message(&message);
        }
        let suppress_memory = std::env::var_os("PI_GRAPH_SUPPRESS_MEMORY_INJECT").is_some()
            || std::env::var_os("PI_GRAPH_ROLE").is_some();
        let memory = if suppress_memory {
            None
        } else {
            host.native_memory_inject(&prompt)
        };
        match agent.turn_context_placement() {
            davinci_agent::turn_context::TurnContextPlacement::Appended => {
                agent.freeze_tools_for_cache();
                agent.commit_turn_context(memory);
            }
            davinci_agent::turn_context::TurnContextPlacement::SystemPrompt => {
                if let Some(memory) = memory {
                    agent.set_ephemeral_context(vec![davinci_ai::ChatMessage::text(
                        "custom", memory,
                    )]);
                }
            }
        }
        host.native_cancel_learning_review();
        host.emit(ExtensionEvent::AgentStart);
        host.emit(ExtensionEvent::TurnStart);
    }
    let js_stream = {
        let host = host.lock().unwrap_or_else(|err| err.into_inner());
        host.js_stream_provider(&agent.provider)
    };
    let hook_host = host.clone();
    let settings = load_merged_settings(&default_agent_dir(), &agent.cwd);
    let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
    let user_hooks = hooks::load(&default_agent_dir(), &agent.cwd, trusted);

    let hook_policy = settings.hook_policy.clone().unwrap_or_default();
    let runtime_bus = davinci_agent::RuntimeBus::new();
    runtime_bus.subscribe(Arc::new(
        runtime_host::HooksRuntimeSubscriber::new_with_config(
            user_hooks.clone(),
            hook_policy,
            agent.cwd.clone(),
            default_agent_dir(),
        ),
    ));
    runtime_bus.subscribe(Arc::new(runtime_host::CompactionRuntimeSubscriber::new(
        hook_host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .native
            .clone(),
    )));
    let mut runtime_handle = davinci_agent::RuntimeHandle::new(
        davinci_agent::RunId::new(),
        // A Graph process owns one agent across all of its turns. Preserve the
        // parent's identity (or the process-local fallback for legacy launchers).
        // Generating a new identity here breaks transaction ownership on turn two.
        if agent.tool_context.transaction_owner.graph_node.is_some() {
            agent.tool_context.transaction_owner.agent_id
        } else {
            davinci_agent::AgentId::new()
        },
        runtime_bus.clone(),
    )
    .with_cache(agent.tool_context.cache.clone());
    let wt_mgr =
        davinci_agent::WorktreeManager::new(&agent.cwd, default_agent_dir().join("worktrees"))
            .with_bus(runtime_bus.clone());
    runtime_handle = runtime_handle.with_worktree_manager(wt_mgr);
    runtime_handle = runtime_handle.with_project_trusted(trusted);
    if agent.session.is_none() {
        if let Some(worker) = agent
            .runtime
            .as_ref()
            .filter(|runtime| runtime.parent_agent_id.is_some())
        {
            runtime_handle = runtime_handle.with_worker_state_from(worker);
        }
    }
    let wf_store = davinci_agent::WorkflowStateStore::with_options(
        davinci_agent::runtime::workflow::state::DEFAULT_MAX_INLINE_ARTIFACT_BYTES,
        default_agent_dir().join("workflow_artifacts"),
    );
    runtime_handle = match runtime_host::configure_session_workflow(
        runtime_handle,
        agent.session.as_ref(),
        agent.runtime_for_session(),
        wf_store,
        agent.subagent_runner.clone(),
    )
    .and_then(|runtime| {
        runtime_host::attach_operation_runtime(runtime, &agent.cwd, agent.session.as_ref())
    }) {
        Ok(runtime) => runtime,
        Err(error) => {
            let reply = format!("Runtime recovery required: {error}");
            let end = AgentEvent::AgentEnd {
                messages: vec![davinci_ai::ChatMessage::text("assistant", &reply)],
                will_retry: false,
            };
            if stream_json {
                if let Ok(value) = to_json_print_event(&end) {
                    if let Ok(encoded) = serde_json::to_string(&value) {
                        let _ = output::write_raw_stdout_line(&encoded);
                    }
                }
            } else {
                agent.emit_live(end.clone());
            }
            let mut host = host.lock().unwrap_or_else(|error| error.into_inner());
            host.emit(ExtensionEvent::TurnEnd);
            host.emit(ExtensionEvent::AgentEnd);
            host.emit(ExtensionEvent::AgentSettled);
            return (reply, vec![end]);
        }
    };
    {
        let mut host = host.lock().unwrap_or_else(|error| error.into_inner());
        host.emit(ExtensionEvent::BeforeProviderRequest {
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
        });
        host.emit(ExtensionEvent::BeforeProviderHeaders {
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
        });
    }
    host.lock()
        .unwrap_or_else(|error| error.into_inner())
        .register_with(&runtime_handle.capability_registry);
    agent.set_runtime(runtime_handle);
    // Print/RPC turns need the same native permission, model and runtime
    // bindings as the interactive shell before a discovered tool executes.
    apply_graph_session_context(
        parsed,
        agent,
        &host.lock().unwrap_or_else(|error| error.into_inner()),
    );
    bind_test_impact_context(
        agent,
        &host.lock().unwrap_or_else(|error| error.into_inner()),
    );

    let pre_hooks = user_hooks.clone();
    agent.pre_tool = Some(davinci_agent::PreToolHook(Arc::new(move |name, args| {
        if std::env::var("DAVINCI_RUNTIME_HOOKS_V2").as_deref() == Ok("0") {
            if let Some(reason) = hooks::run_pre_tool(&pre_hooks, name, args) {
                return Some(reason);
            }
        }
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
        // Git HEAD/status cannot prove content freshness in dirty, ignored or
        // out-of-repository search targets. Do not suppress on that weak key.
        if let Some(reason) = host.native_before_tool(name, args, String::new) {
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
    let post_hooks = user_hooks;
    let session_path = agent.session.as_ref().map(|session| session.path.clone());
    agent.post_tool = Some(davinci_agent::PostToolHook(Arc::new(
        move |tool_call_id, _cwd, name, args, result| {
            // A call the gate refused never ran: no post hook, and the row
            // says `denied` so the ledger does not count it as a tool run.
            let denied = result
                .details
                .as_ref()
                .and_then(|details| details.get("denied"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if !denied && std::env::var("DAVINCI_RUNTIME_HOOKS_V2").as_deref() == Ok("0") {
                hooks::run_post_tool(&post_hooks, name, args, &result.content);
            }
            hooks::append_event(
                session_path.as_ref(),
                if denied { "denied" } else { "tool" },
                name,
                Some(tool_call_id),
                Some(!result.is_error),
            );
            match post_host.lock() {
                Ok(host) => host.native_after_tool(name, args, result),
                Err(_) => result,
            }
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
    synchronize_provider_system_prompt(agent);
    agent.set_provider_context_overhead_tokens(Some(
        serde_json::to_vec(&provider_tools(agent))
            .expect("tool schemas are JSON")
            .len() as u64
            + 128,
    ));
    agent.set_provider_output_limit(model.as_ref().map(|m| m.max_tokens));
    let mut context_visibility = (agent.stats.pruned_results, agent.stats.compactions);
    // Session calls settle after the loop. Hold the final terminal event until
    // their result is known so streaming clients receive one final outcome.
    let terminal_sink = agent.event_sink.clone();
    if let Some(sink) = terminal_sink.clone() {
        agent.event_sink = Some(EventSink(Arc::new(move |event| {
            if !matches!(
                event,
                AgentEvent::AgentEnd {
                    will_retry: false,
                    ..
                }
            ) {
                (sink.0)(event);
            }
        })));
    }
    let mut loop_failure = None;
    let mut conversation_failure = None;
    let mut events = agent
        .run_loop(|current| {
            let visibility = (current.stats.pruned_results, current.stats.compactions);
            if visibility != context_visibility {
                host.lock().unwrap_or_else(|error| error.into_inner()).native_context_pruned();
                context_visibility = visibility;
            }
            let last_user = current
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| content_text(&m.content).len())
                .unwrap_or(0);
            let system = current.provider_system_prompt();
            match (offline, model.as_ref(), auth.as_ref(), js_stream.as_ref()) {
                (false, Some(model), _, Some((path, name))) => {
                    crate::js_host::run_js_stream_simple(
                        Path::new(path),
                        name,
                        model,
                        &current.messages_for_provider(),
                        &system,
                        current.context_vm_provider_output_limit(),
                    )
                    .map(CompleteOutput::from)
                }
                (false, Some(model), Some(auth), None) => {
                    // Every stream event reaches the sink the moment it is
                    // decoded, as a `MessageUpdate` carrying the partial
                    // message, with a `MessageStart` ahead of the first one.
                    // The loop records them afterwards without resending.
                    let mut started = false;
                    let mut sink = |event: &davinci_ai::AssistantMessageEvent| {
                        let partial = Arc::new(davinci_ai::assistant_to_chat(event.message()));
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
                    let provider_messages = current.messages_for_provider();
                    let result = live_complete_streaming_with_sink_envelope(
                        model,
                        &provider_messages,
                        auth,
                        Some(&system),
                        &provider_tools(current),
                        &StreamOptions {
                            thinking_level: Some(current.thinking_level),
                            thinking_budgets: current.thinking_budgets.clone(),
                            timeout_ms: current.provider_timeout_ms,
                            max_retries: current.provider_max_retries,
                            max_retry_delay_ms: Some(current.provider_max_retry_delay_ms),
                            max_tokens: current.context_vm_provider_output_limit(),
                            websocket_connect_timeout_ms: load_settings(&default_agent_dir())
                                .websocket_connect_timeout_ms,
                            transport: current.transport.clone(),
                            session_id: current
                                .session
                                .as_ref()
                                .map(|session| session.header.id.clone()),
                            cache_key: std::env::var("PI_GRAPH_CACHE_KEY")
                                .ok()
                                .filter(|s| !s.is_empty())
                                .or_else(|| Some(davinci_agent::CacheIdentity {
                                    provider: model.provider.clone(),
                                    model_id: model.id.clone(),
                                    system_prompt_hash: davinci_agent::hash_system_prompt_with_manifest(&system, current.prompt_manifest.as_ref()),
                                    tool_schema_hash: current.provider_tool_schema_identity(),
                                    permission_surface_hash: davinci_agent::hash_tool_names(&current.visible_tool_names().iter().map(String::as_str).collect::<Vec<_>>()),
                                    context_item_hashes: current
                                        .context_vm_cache_affinity()
                                        .into_iter()
                                        .collect(),
                                    agent_profile_hash: None,
                                    contract_hash: None,
                                    role: Some("root".into()),
                                }.cache_key())),
                            cache_retention: None,
                            native_responses_resume:
                                current.native_responses_resume_record(),
                            install_telemetry: Some(current.install_telemetry),
                            abort_signal: current.abort_signal.clone(),
                        },
                        &mut sink,
                    );
                    result.map(|envelope| {
                        let native_responses_resume = envelope.native_responses.map(|turn| {
                            let mut resume_projection = provider_messages.clone();
                            resume_projection
                                .push(davinci_ai::assistant_to_chat(&envelope.message));
                            davinci_ai::NativeResponsesResumeRecord {
                                turn,
                                resume_provider_message_count: resume_projection.len(),
                                resume_provider_messages_fingerprint:
                                    davinci_ai::provider_messages_fingerprint(
                                        &resume_projection,
                                    ),
                            }
                        });
                        CompleteOutput {
                            message: envelope.message,
                            stream_events: Some(envelope.stream_events),
                            native_responses_resume,
                            streamed_live: started,
                        }
                    })
                }
                // Nothing was asked of a provider. Say which of the three
                // reasons it was: an offline run answers with the stub the
                // fixtures expect, but a missing model or a missing credential
                // is a fault, and a reply that only counts the characters it
                // was handed reads like an answer while hiding one.
                (true, ..) => Ok(CompleteOutput::from(offline_stub_message(
                    current, last_user,
                ))),
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
            conversation_failure = agent.ensure_session_persistence().err();
            let reply = conversation_failure.clone()
                .unwrap_or_else(|| format!("Provider error: {err}"));
            loop_failure = Some(reply.clone());
            let mut message = davinci_ai::ChatMessage::text("assistant", &reply);
            if conversation_failure.is_some() {
                message.extra.insert("sessionPersistenceError".into(), serde_json::json!(reply));
            }
            let end = AgentEvent::AgentEnd {
                messages: vec![message],
                will_retry: false,
            };
            agent.emit_live(end.clone());
            vec![end]
        });
    agent.event_sink = None;
    // The last assistant message may be a tool call with no text — the
    // reply is then whatever the run ended on, not an empty string that
    // hides a provider error behind "the model returned no text".
    let mut reply = loop_failure
        .or_else(|| {
            agent
                .last_assistant_text()
                .filter(|text| !text.trim().is_empty())
        })
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
    let mut session_failure = None;
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

        let settings = load_merged_settings(&default_agent_dir(), &agent.cwd);
        let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
        host.set_project_trusted(trusted);

        let mut commands_ran = 0u32;
        let mut all_commands_passed = true;
        let mut graph_run_id = None;
        for ev in &events {
            if let AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                details,
                ..
            } = ev
            {
                if tool_name == "bash"
                    || tool_name == "powershell"
                    || tool_name == "execute_command"
                {
                    commands_ran += 1;
                    if *is_error {
                        all_commands_passed = false;
                    }
                } else if tool_name == "graph_run" {
                    if let Some(details) = details {
                        if let Some(graph) = details.get("graph") {
                            if let Some(id) = graph.get("runId").and_then(|v| v.as_str()) {
                                graph_run_id = Some(id.to_string());
                            }
                            if let Some(v) = graph.get("verification") {
                                if let Ok(vr) = serde_json::from_value::<
                                    crate::native_extensions::graph::types::VerificationResult,
                                >(v.clone())
                                {
                                    for cmd in &vr.commands {
                                        if !cmd.skipped {
                                            commands_ran += 1;
                                            if cmd.exit_code != 0 {
                                                all_commands_passed = false;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let verification = crate::native_extensions::learning::VerificationEvidence {
            graph_run_id,
            commands_ran,
            passed: all_commands_passed && commands_ran > 0,
            user_accepted: false,
            user_corrected: false,
            permission_denied: false,
        };

        let skill_outcome = if verification.passed && verification.commands_ran > 0 {
            crate::native_extensions::learning::SkillOutcome::VerifiedSuccess
        } else if !verification.passed && verification.commands_ran > 0 {
            crate::native_extensions::learning::SkillOutcome::VerifiedFailure
        } else {
            crate::native_extensions::learning::SkillOutcome::Neutral
        };

        let (latest_user_text, _) = latest_user_prompt(agent);
        for skill in &agent.skills {
            let tag = format!("<skill name=\"{}\"", skill.name);
            if latest_user_text.contains(&tag) {
                host.native_record_skill_outcome_for_skill(&skill.name, &skill.body, skill_outcome);
            }
        }

        let learning_evidence = crate::native_extensions::learning::build_learning_evidence(
            crate::native_extensions::learning::BuildEvidenceInput {
                session_id: agent
                    .session
                    .as_ref()
                    .map(|s| s.path.display().to_string())
                    .unwrap_or_else(|| "default-session".to_string()),
                repo_id: crate::native_extensions::vector_memory::repo_id(&agent.cwd),
                turn: agent.messages.len() as u64,
                messages: &memory_messages,
                events: &events,
                run_stats: agent.run_stats(),
                verification,
            },
        );
        let _ = host.native_review_settled_turn(learning_evidence);
        if fresh_host {
            for notice in host.drain_learning_notifications() {
                println!("{notice}");
            }
        }
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
        let failures = apply_session_calls(
            Some(parsed),
            agent,
            SessionCallUi::Silent,
            &session_calls,
            false,
        );
        if !failures.is_empty() {
            session_failure = Some(
                failures
                    .iter()
                    .map(|(_, error)| error.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        let session_calls: Vec<_> = session_calls
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !failures.iter().any(|(failed, _)| failed == index))
            .map(|(_, call)| call)
            .collect();
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
    conversation_failure =
        conversation_failure.or_else(|| agent.ensure_session_persistence().err());
    if let Some(error) = conversation_failure.clone().or(session_failure) {
        reply = error;
        let mut message = davinci_ai::ChatMessage::text("assistant", &reply);
        if conversation_failure.is_some() {
            message
                .extra
                .insert("sessionPersistenceError".into(), serde_json::json!(reply));
        }
        let end = AgentEvent::AgentEnd {
            messages: vec![message],
            will_retry: false,
        };
        if let Some(previous) = events.iter_mut().rev().find(|event| {
            matches!(
                event,
                AgentEvent::AgentEnd {
                    will_retry: false,
                    ..
                }
            )
        }) {
            *previous = end;
        } else {
            events.push(end);
        }
    }
    if let (Some(sink), Some(end)) = (
        &terminal_sink,
        events.iter().rev().find(|event| {
            matches!(
                event,
                AgentEvent::AgentEnd {
                    will_retry: false,
                    ..
                }
            )
        }),
    ) {
        (sink.0)(end);
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

/// Print cannot collect consent. Stop at the first policy-owned challenge and
/// report it without creating grants or leaving a sticky cancellation signal.
fn with_print_approval<T>(
    agent: &mut Agent,
    configuration_path: &Path,
    run: impl FnOnce(&mut Agent) -> T,
) -> (T, Option<serde_json::Value>) {
    let required = Arc::new(Mutex::new(None));
    let captured = required.clone();
    let path = configuration_path.to_string_lossy().into_owned();
    let abort = Arc::new(std::sync::atomic::AtomicBool::new(agent.abort_requested()));
    let cancelled = abort.clone();
    let previous_abort = agent.abort_signal.replace(abort);
    let previous_approver = agent.approver.take();
    let previous_responder = agent.approval_responder.replace(
        davinci_agent::approval::ApprovalResponder(Arc::new(move |_, challenge| {
            let mut report = captured.lock().unwrap_or_else(|err| err.into_inner());
            if report.is_none() {
                *report = Some(serde_json::json!({
                    "type": "approval_required",
                    "tool_call_id": challenge.call_id,
                    "action": challenge.action_label,
                    "target": native_extensions::vector_memory::redact_secrets(&challenge.display_target),
                    "permission_mode": challenge.mode,
                    "configuration_path": path,
                    "guidance": "Use an interactive host to approve this action, or configure a narrow permissions.allow rule. Explicit deny rules still take precedence.",
                }));
            }
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            davinci_agent::approval::ApprovalReply::from_legacy(
                challenge, davinci_agent::ToolApprovalDecision::Deny,
            )
        })),
    );
    let result = run(agent);
    agent.approval_responder = previous_responder;
    agent.approver = previous_approver;
    agent.abort_signal = previous_abort;
    let report = required
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take();
    (result, report)
}

fn scope_expansion_preview_from_events(
    agent: &Agent,
    events: &[AgentEvent],
) -> Option<davinci_agent::runtime::contracts::ScopeExpansionPreview> {
    let contract = agent.active_contract()?;
    events.iter().find_map(|event| {
        let AgentEvent::ToolExecutionEnd {
            is_error: true,
            details: Some(details),
            ..
        } = event
        else {
            return None;
        };
        let violation: davinci_agent::runtime::contracts::ScopeViolation =
            serde_json::from_value(details.get("scope_violation")?.clone()).ok()?;
        contract
            .preview_scope_expansion(&violation.requested_target, &violation.reason)
            .ok()
    })
}

fn scope_expansion_required_report(
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
) -> serde_json::Value {
    serde_json::json!({
        "type": "scope_expansion_required",
        "preview": preview,
        "guidance": "Scope cannot be expanded by the model, permission mode, print mode, or an unsupported RPC client. Use the interactive scope-expansion decision flow.",
    })
}

fn blocking_host_report(
    agent: &Agent,
    events: &[AgentEvent],
    approval_required: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    scope_expansion_preview_from_events(agent, events)
        .map(|preview| scope_expansion_required_report(&preview))
        .or(approval_required)
}

fn rpc_scope_expansion_result(
    id: Option<String>,
    preview: &davinci_agent::runtime::contracts::ScopeExpansionPreview,
) -> (serde_json::Value, rpc::RpcResponse) {
    (
        scope_expansion_required_report(preview),
        rpc::fail_response(id, "prompt", "scope_expansion_required".into()),
    )
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
        write_prompt_manifest_json_event(agent)?;
    }
    let mut last_reply = String::new();
    let mut approval_required = None;
    let configuration_path = settings::settings_path(&default_agent_dir());
    let mut all_events = Vec::new();
    if let Some(prompt) = &prepared.text {
        if !prompt.trim().is_empty() || !prepared.images.is_empty() {
            match prepare_user_input(parsed, agent, prompt, &prepared.images, "print", None)? {
                PreparedInput::Completed(code) => return Ok(code),
                PreparedInput::Handled => {}
                PreparedInput::Ready { text, images } => {
                    agent.prompt_user_with(&text, &images);
                    if json_mode {
                        write_prompt_manifest_json_event(agent)?;
                    }
                    let ((reply, events), required) =
                        with_print_approval(agent, &configuration_path, |agent| {
                            complete_prompt_with_host(parsed, agent, None, json_mode)
                        });
                    approval_required = blocking_host_report(agent, &events, required);
                    last_reply = reply;
                    all_events.extend(events);
                }
            }
        }
    }
    for extra in &prepared.remaining_messages {
        if approval_required.is_some()
            || agent.ensure_session_persistence().is_err()
            || last_reply.starts_with("Runtime recovery required: ")
        {
            break;
        }
        if extra.trim().is_empty() {
            continue;
        }
        match prepare_user_input(parsed, agent, extra, &[], "print", None)? {
            PreparedInput::Completed(code) => return Ok(code),
            PreparedInput::Handled => {}
            PreparedInput::Ready { text, images } => {
                agent.prompt_user_with(&text, &images);
                if json_mode {
                    write_prompt_manifest_json_event(agent)?;
                }
                let ((reply, events), required) =
                    with_print_approval(agent, &configuration_path, |agent| {
                        complete_prompt_with_host(parsed, agent, None, json_mode)
                    });
                approval_required = blocking_host_report(agent, &events, required);
                last_reply = reply;
                all_events.extend(events);
            }
        }
    }
    let (exit_code, error) = print_text_exit(&all_events);
    // A provider failure that the loop gave up on carries no error stop
    // reason of its own: it is the reply text. Report it as the failure it is.
    let (exit_code, error) = match error {
        None if last_reply.starts_with("Provider error: ")
            || last_reply.starts_with("Runtime recovery required: ") =>
        {
            (1, Some(last_reply.clone()))
        }
        other => (exit_code, other),
    };
    let exit_code = if let Some(required) = approval_required {
        let encoded = serde_json::to_string(&required).map_err(|err| err.to_string())?;
        output::write_raw_stdout_line(&encoded).map_err(|err| err.to_string())?;
        1
    } else {
        if !json_mode {
            if let Some(error) = error {
                eprintln!("{error}");
            } else if !last_reply.is_empty() {
                println!("{last_reply}");
            }
        }
        exit_code
    };
    loaded_extension_host(parsed).emit(ExtensionEvent::SessionShutdown {
        reason: "quit".into(),
    });
    Ok(exit_code)
}

fn prompt_manifest_json_event(agent: &Agent) -> Option<serde_json::Value> {
    let manifest = agent.prompt_manifest.as_ref()?;
    let mut event = serde_json::json!({
        "type": "prompt_manifest",
        "promptManifest": manifest,
    });
    if let Some(candidate_id) = agent.prompt_session.candidate_id.as_deref() {
        event["candidateId"] = serde_json::Value::String(candidate_id.to_string());
    }
    Some(event)
}

fn write_prompt_manifest_json_event(agent: &Agent) -> Result<(), String> {
    if let Some(event) = prompt_manifest_json_event(agent) {
        let encoded = serde_json::to_string(&event).map_err(|err| err.to_string())?;
        output::write_raw_stdout_line(&encoded).map_err(|err| err.to_string())?;
    }
    Ok(())
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
            use davinci_ai::AssistantMessageEvent as Ev;
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
        if let AgentEvent::AgentEnd { messages, .. } = event {
            if let Some(error) = messages.iter().find_map(|message| {
                message
                    .extra
                    .get("sessionPersistenceError")
                    .and_then(serde_json::Value::as_str)
            }) {
                return (1, Some(error.to_string()));
            }
        }
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
    run_rpc_with_host(
        parsed,
        agent,
        Arc::new(Mutex::new(loaded_extension_host(parsed))),
    )
}

fn run_rpc_with_host(
    parsed: &Args,
    agent: &mut Agent,
    host: Arc<Mutex<ExtensionHost>>,
) -> Result<i32, String> {
    let session_dir = agent
        .session
        .as_ref()
        .and_then(|session| session.path.parent())
        .map(|path| path.parent().unwrap_or(path).to_path_buf())
        .unwrap_or_else(davinci_session::default_session_dir);
    let cwd = agent.cwd.clone();
    let mut runtime = RpcRuntime::with_models(
        std::mem::replace(
            agent,
            Agent::new_builtin(davinci_agent::PromptProfile::Stable),
        ),
        session_dir,
        cwd,
        available_models(parsed),
    );
    host.lock()
        .unwrap_or_else(|err| err.into_inner())
        .emit(ExtensionEvent::SessionStart);
    {
        let host = host.lock().unwrap_or_else(|err| err.into_inner());
        for provider in host.registered_providers() {
            runtime
                .models
                .extend(davinci_ai::models_from_provider_config(
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
        .and_then(davinci_protocol::ThinkingLevel::parse);
    if let Some(levels) = stored.model_thinking_levels {
        runtime.model_thinking_levels = levels
            .into_iter()
            .filter_map(|(key, value)| {
                davinci_protocol::ThinkingLevel::parse(&value).map(|level| (key, level))
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
    let ui_abort = Arc::new(Mutex::new(None));
    crate::js_host::install_ui_waiter({
        let ui_abort = ui_abort.clone();
        let leftover = leftover.clone();
        let rx = rx.clone();
        Box::new(move |call| rpc_emit_and_wait_ui(call, &leftover, &rx, &ui_abort))
    });
    // A tool call the policy cannot decide is put to the client as the
    // `select` request it already renders for extensions; the answer text is
    // the decision. The prompt turn runs on this thread, so the waiter is
    // the one just installed. Anything but a listed answer denies.
    {
        let cwd = runtime.agent.cwd.clone();
        let trusted = settings::is_trusted(
            &load_merged_settings(&default_agent_dir(), &cwd),
            &cwd,
            parsed.project_trust_override,
        );
        let policy = runtime.agent.permissions.clone();
        runtime.agent.approver = None;
        runtime.agent.approval_responder = Some(davinci_agent::approval::ApprovalResponder(
            Arc::new(move |request, challenge| {
                rpc_resolve_challenge(
                    request,
                    challenge,
                    trusted,
                    &cwd,
                    &policy,
                    crate::js_host::dispatch_ui_waiter,
                )
            }),
        ));
        runtime.agent.tool_context.decision_responder =
            Some(davinci_agent::DecisionResponder::new(move |request| {
                crate::rpc::rpc_resolve_decision(&request, crate::js_host::dispatch_ui_waiter)
            }));
    }
    loop {
        let Some(line) = rpc_next_line(&leftover, &rx) else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let mut command: RpcCommand = match serde_json::from_str(&line) {
            Ok(command) => command,
            Err(err) => {
                let response =
                    rpc::fail_response(None, "parse", format!("parse error: {err}"));
                output::write_raw_stdout_line(
                    &serde_json::to_string(&response).map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?;
                continue;
            }
        };
        if command.kind == "verify_browser" {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct BrowserVerificationRequest {
                browser_id: uuid::Uuid,
                dom_contains: String,
                accessibility_contains: Option<String>,
            }
            let result = (|| {
                if command
                    .value
                    .as_ref()
                    .is_some_and(|value| value.len() > 12 * 1024)
                {
                    return Err("browser verification request exceeds limit".into());
                }
                let request: BrowserVerificationRequest = serde_json::from_str(
                    command
                        .value
                        .as_deref()
                        .ok_or("browser verification request required")?,
                )
                .map_err(|_| "invalid browser verification request")?;
                let controller = host
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .native
                    .lock()
                    .map_err(|_| "native host unavailable")?
                    .browser
                    .clone();
                let receipt = controller.verify_host(
                    &runtime.cwd,
                    request.browser_id,
                    crate::native_extensions::browser::BrowserAssertionSpec {
                        dom_contains: request.dom_contains,
                        accessibility_contains: request.accessibility_contains,
                    },
                    &runtime.agent.tool_context,
                )?;
                serde_json::to_value(receipt)
                    .map_err(|_| "browser verification receipt unavailable".into())
            })();
            let response = match result {
                Ok(data) => rpc::ok_response(command.id.clone(), &command.kind, Some(data)),
                Err(error) => rpc::fail_response(command.id.clone(), &command.kind, error),
            };
            output::write_raw_stdout_line(
                &serde_json::to_string(&response).map_err(|err| err.to_string())?,
            )
            .map_err(|err| err.to_string())?;
            continue;
        }
        if command.kind == "get_browser_artifact" {
            let result = (|| {
                if command
                    .value
                    .as_ref()
                    .is_some_and(|value| value.len() > 12 * 1024)
                {
                    return Err("browser artifact request exceeds limit".into());
                }
                let args: serde_json::Value = serde_json::from_str(
                    command
                        .value
                        .as_deref()
                        .ok_or("browser artifact request required")?,
                )
                .map_err(|_| "invalid browser artifact request")?;
                let controller = host
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .native
                    .lock()
                    .map_err(|_| "native host unavailable")?
                    .browser
                    .clone();
                controller.retrieve_artifact(&runtime.cwd, &args, &runtime.agent.tool_context)
            })();
            let response = match result {
                Ok(data) => rpc::ok_response(command.id.clone(), &command.kind, Some(data)),
                Err(error) => rpc::fail_response(command.id.clone(), &command.kind, error),
            };
            output::write_raw_stdout_line(
                &serde_json::to_string(&response).map_err(|err| err.to_string())?,
            )
            .map_err(|err| err.to_string())?;
            continue;
        }
        let is_prompt = command.kind == "prompt";
        if is_prompt {
            let message = command.message.as_deref().unwrap_or("");
            if message.trim_start().starts_with('/') {
                let (name, args) = parse_extension_command(message);
                if matches!(
                    name.as_str(),
                    "security-scan" | "sec-resume" | "sec-status" | "sec-report" | "sec-abort"
                ) {
                    let locked = host.lock().unwrap_or_else(|e| e.into_inner());
                    locked
                        .native
                        .lock()
                        .map_err(|_| "native host lock poisoned")?
                        .security
                        .set_review_storage(default_agent_dir());
                    let result = (|| {
                        if matches!(name.as_str(), "security-scan" | "sec-resume") {
                            configure_security_review(parsed, &runtime.agent, &locked)?;
                        }
                        locked.execute_native_command(&name, &args)
                    })();
                    let response = match result {
                        Ok(value) => rpc::ok_response(command.id.clone(), "prompt", value),
                        Err(error) => rpc::fail_response(command.id.clone(), "prompt", error),
                    };
                    output::write_raw_stdout_line(
                        &serde_json::to_string(&response).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    continue;
                }
            }
        }
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
                .map(davinci_agent::parse_rpc_images)
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
                PreparedInput::Handled | PreparedInput::Completed(_) => {
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
        let mut response = handle_rpc(&mut runtime, command.clone());
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
            let signal = Arc::new(std::sync::atomic::AtomicBool::new(
                runtime.agent.abort_requested(),
            ));
            let previous_abort = runtime.agent.abort_signal.replace(signal.clone());
            *ui_abort.lock().unwrap_or_else(|err| err.into_inner()) = Some(signal);
            let remote = runtime.agent.remote_queue();
            let skills = runtime.agent.skills.clone();
            let templates = runtime.agent.templates.clone();
            let (_reply, events) = std::thread::scope(|scope| {
                let stop = std::sync::atomic::AtomicBool::new(false);
                let watcher = scope.spawn(|| {
                    rpc_watch_during_turn(
                        &leftover,
                        &rx,
                        &ui_abort,
                        &remote,
                        &skills,
                        &templates,
                        &stop,
                    )
                });
                let result = complete_prompt_with_host(
                    &prompt_args,
                    &mut runtime.agent,
                    Some(host.clone()),
                    false,
                );
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = watcher.join();
                result
            });
            *ui_abort.lock().unwrap_or_else(|err| err.into_inner()) = None;
            runtime.agent.abort_signal = previous_abort;
            if let Some(preview) = scope_expansion_preview_from_events(&runtime.agent, &events) {
                let (report, failed) = rpc_scope_expansion_result(command.id.clone(), &preview);
                output::write_raw_stdout_line(
                    &serde_json::to_string(&report).map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?;
                response = failed;
            }
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
    {
        let locked = host.lock().unwrap_or_else(|e| e.into_inner());
        let _ = locked.execute_native_command("sec-abort", "");
    }
    emit_session_shutdown(parsed);
    *agent = runtime.agent;
    Ok(0)
}

const RPC_APPROVAL_ONCE: &str = "allow once";
const RPC_APPROVAL_SESSION: &str = "allow for this session";
const RPC_APPROVAL_ALWAYS: &str = "always allow in this project";
const RPC_APPROVAL_DENY: &str = "deny";

/// The `select` an RPC client is shown for a permission question. The
/// "always" row is offered only when the project is trusted, for the same
/// reason davinci omits it: an untrusted project's settings are never read.
fn rpc_approval_call(
    request: &davinci_agent::ToolApprovalRequest,
    trusted: bool,
) -> serde_json::Value {
    let options = request
        .host_choices(trusted)
        .into_iter()
        .map(|choice| match choice {
            davinci_agent::ToolApprovalDecision::AllowOnce => RPC_APPROVAL_ONCE,
            davinci_agent::ToolApprovalDecision::AllowForSession => RPC_APPROVAL_SESSION,
            davinci_agent::ToolApprovalDecision::AllowAlways => RPC_APPROVAL_ALWAYS,
            davinci_agent::ToolApprovalDecision::Deny => RPC_APPROVAL_DENY,
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "op": "select",
        "title": format!("Allow {}?", request.summary),
        "options": options,
    })
}

/// The client's answer as a decision. Text is matched exactly; a cancelled
/// or unoffered answer is a refusal.
fn rpc_approval_decision(
    answer: &serde_json::Value,
    trusted: bool,
) -> davinci_agent::ToolApprovalDecision {
    use davinci_agent::ToolApprovalDecision::*;
    match answer.as_str().map(str::trim) {
        Some(RPC_APPROVAL_ONCE) => AllowOnce,
        Some(RPC_APPROVAL_SESSION) => AllowForSession,
        Some(RPC_APPROVAL_ALWAYS) if trusted => AllowAlways,
        _ => Deny,
    }
}

fn rpc_resolve_approval(
    request: &davinci_agent::ToolApprovalRequest,
    trusted: bool,
    cwd: &Path,
    mut ask: impl FnMut(&serde_json::Value) -> serde_json::Value,
) -> davinci_agent::ToolApprovalDecision {
    let decision = rpc_approval_decision(&ask(&rpc_approval_call(request, trusted)), trusted);
    if !request.allows(decision) {
        return davinci_agent::ToolApprovalDecision::Deny;
    }
    if decision != davinci_agent::ToolApprovalDecision::AllowAlways
        || permissions::remember_project_rule(cwd, &request.session_rule).is_ok()
    {
        return decision;
    }
    let mut retry = rpc_approval_call(request, false);
    retry["title"] = serde_json::json!(format!(
        "Project permission could not be saved. Choose another scope. Allow {}?",
        request.summary
    ));
    let decision = rpc_approval_decision(&ask(&retry), false);
    if request.allows(decision) {
        decision
    } else {
        davinci_agent::ToolApprovalDecision::Deny
    }
}

/// Keep the existing select wire format while binding its synchronous reply to
/// the engine-issued challenge. Never persist an answer already known stale.
fn rpc_resolve_challenge(
    request: &davinci_agent::ToolApprovalRequest,
    challenge: &davinci_agent::approval::ApprovalChallenge,
    trusted: bool,
    cwd: &Path,
    permissions: &davinci_agent::PermissionState,
    mut ask: impl FnMut(&serde_json::Value) -> serde_json::Value,
) -> davinci_agent::approval::ApprovalReply {
    let started = std::time::Instant::now();
    let available = std::time::Duration::from_millis(
        challenge
            .expires_at_ms
            .saturating_sub(davinci_session::now_ms()),
    );
    let fresh = || {
        if davinci_session::now_ms() >= challenge.expires_at_ms || started.elapsed() >= available {
            return false;
        }
        let policy = permissions.lock().unwrap_or_else(|err| err.into_inner());
        policy.revision() == Some(challenge.policy_revision)
            && matches!(policy.decide(&request.tool_call_id, &request.tool, &request.args, cwd), davinci_agent::PermissionVerdict::Ask(current) if current == *request)
    };
    let mut ask_fresh = |call: &serde_json::Value| {
        if !fresh() {
            return serde_json::Value::Null;
        }
        let mut bounded = call.clone();
        bounded["timeout"] =
            serde_json::json!(available.saturating_sub(started.elapsed()).as_millis() as u64);
        let answer = ask(&bounded);
        if fresh() {
            answer
        } else {
            serde_json::Value::Null
        }
    };
    let offers_instructions = [&request.legal_choices, &challenge.legal_choices]
        .iter()
        .all(|choices| {
            choices.iter().any(|choice| {
                choice.id == "deny_with_instructions"
                    && choice.scope == davinci_agent::approval::GrantScope::DenyWithInstructions
            })
        });
    let mut instructions = None;
    let decision = rpc_resolve_approval(request, trusted, cwd, |call| {
        let mut call = call.clone();
        if offers_instructions {
            if let Some(options) = call["options"].as_array_mut() {
                options.push(serde_json::json!("deny with instructions"));
            }
        }
        let answer = ask_fresh(&call);
        if !offers_instructions || answer.as_str() != Some("deny with instructions") {
            return answer;
        }
        let input = ask_fresh(&serde_json::json!({
            "op": "input", "title": "Deny this call: what should the model do instead?",
            "placeholder": "Instructions (up to 4096 bytes)",
        }));
        if let Some(text) = input
            .as_str()
            .filter(|text| !text.trim().is_empty() && text.len() <= 4096)
        {
            let confirmed = ask_fresh(&serde_json::json!({
                "op": "confirm", "title": "Deny this call with these instructions?", "message": text,
            }));
            if confirmed.as_bool() == Some(true) {
                instructions = Some(text.to_string());
            }
        }
        serde_json::Value::Null
    });
    let mut reply = davinci_agent::approval::ApprovalReply::from_legacy(challenge, decision);
    if instructions.is_some() {
        reply.choice_id = "deny_with_instructions".into();
        reply.instructions = instructions;
    }
    reply
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

fn rpc_write_ok(id: Option<String>, kind: &str) {
    if let Ok(encoded) = serde_json::to_string(&rpc::ok_response(id, kind, None)) {
        let _ = output::write_raw_stdout_line(&encoded);
    }
}

fn rpc_watch_during_turn(
    leftover: &Mutex<std::collections::VecDeque<String>>,
    rx: &Mutex<std::sync::mpsc::Receiver<String>>,
    active: &RpcUiAbort,
    remote: &davinci_agent::RemoteQueue,
    skills: &[davinci_agent::Skill],
    templates: &[davinci_agent::PromptTemplate],
    stop: &std::sync::atomic::AtomicBool,
) {
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        let line = match rx.lock() {
            Ok(receiver) => match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            },
            Err(_) => return,
        };
        let parsed = serde_json::from_str::<RpcCommand>(&line).ok();
        let handled = match parsed {
            Some(command) if command.kind == "abort" => {
                if let Some(signal) = active
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .as_ref()
                {
                    signal.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                rpc_write_ok(command.id, "abort");
                true
            }
            Some(command) if command.kind == "prompt" => {
                if let (Some(behavior), Some(message)) = (
                    command.streaming_behavior.as_deref(),
                    command.message.as_deref(),
                ) {
                    if matches!(behavior, "steer" | "followUp") {
                        let text = davinci_agent::expand_user_text(message, skills, templates);
                        let images = command
                            .images
                            .as_deref()
                            .map(davinci_agent::parse_rpc_images)
                            .unwrap_or_default();
                        if behavior == "steer" {
                            remote.push_steer(text, images);
                        } else {
                            remote.push_follow_up(text, images);
                        }
                        rpc_write_ok(command.id, "prompt");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if !handled {
            leftover
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push_back(line);
        }
    }
}

type RpcUiAbort = Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>;

fn rpc_with_ui_abort<T>(
    agent: &mut Agent,
    active: &RpcUiAbort,
    run: impl FnOnce(&mut Agent) -> T,
) -> T {
    let signal = Arc::new(std::sync::atomic::AtomicBool::new(agent.abort_requested()));
    let previous = agent.abort_signal.replace(signal.clone());
    *active.lock().unwrap_or_else(|err| err.into_inner()) = Some(signal);
    let result = run(agent);
    *active.lock().unwrap_or_else(|err| err.into_inner()) = None;
    agent.abort_signal = previous;
    result
}

fn rpc_emit_and_wait_ui(
    call: &serde_json::Value,
    leftover: &Mutex<std::collections::VecDeque<String>>,
    rx: &Mutex<std::sync::mpsc::Receiver<String>>,
    active: &RpcUiAbort,
) -> serde_json::Value {
    let abort = active.lock().unwrap_or_else(|err| err.into_inner()).clone();
    let requests = rpc::extension_ui_requests_from_calls(std::slice::from_ref(call));
    let default = if call.get("op").and_then(|value| value.as_str()) == Some("confirm") {
        serde_json::Value::Bool(false)
    } else {
        serde_json::Value::Null
    };
    if abort
        .as_ref()
        .is_some_and(|signal| signal.load(std::sync::atomic::Ordering::Relaxed))
    {
        return default;
    }
    let Some(request) = requests.first() else {
        return default;
    };
    let Ok(encoded) = serde_json::to_string(request) else {
        return default;
    };
    {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        if writeln!(out, "{encoded}")
            .and_then(|_| out.flush())
            .is_err()
        {
            return default;
        }
    }
    let id = request
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let timeout_ms = call.get("timeout").and_then(|value| value.as_u64());
    let (answer, aborted) =
        rpc_wait_ui_response(&id, default, timeout_ms, leftover, rx, abort.is_some());
    if let (Some(command), Some(abort)) = (aborted, abort) {
        abort.store(true, std::sync::atomic::Ordering::Relaxed);
        let response = rpc::ok_response(command.id, "abort", None);
        let Ok(encoded) = serde_json::to_string(&response) else {
            return answer;
        };
        let stdout = io::stdout();
        let mut out = stdout.lock();
        if writeln!(out, "{encoded}")
            .and_then(|_| out.flush())
            .is_err()
        {
            return answer;
        }
    }
    answer
}

fn rpc_wait_ui_response(
    id: &str,
    default: serde_json::Value,
    timeout_ms: Option<u64>,
    leftover: &Mutex<std::collections::VecDeque<String>>,
    rx: &Mutex<std::sync::mpsc::Receiver<String>>,
    intercept_abort: bool,
) -> (serde_json::Value, Option<RpcCommand>) {
    let deadline = match timeout_ms {
        Some(ms) => {
            match std::time::Instant::now().checked_add(std::time::Duration::from_millis(ms)) {
                Some(end) => Some(end),
                None => return (default, None),
            }
        }
        None => None,
    };
    let mut parked = std::collections::VecDeque::new();
    let result = loop {
        if deadline.is_some_and(|end| std::time::Instant::now() >= end) {
            break (default, None);
        }
        let remaining = deadline.map_or(std::time::Duration::from_millis(200), |end| {
            end.saturating_duration_since(std::time::Instant::now())
        });
        let queued = leftover
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .pop_front();
        let line = match queued {
            Some(line) => line,
            None => match rx
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .recv_timeout(remaining)
            {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break (default, None),
            },
        };
        if let Some(value) = parse_rpc_ui_response(&line, id) {
            break (value, None);
        }
        if let Ok(command) = serde_json::from_str::<RpcCommand>(&line) {
            if intercept_abort && command.kind == "abort" {
                break (default, Some(command));
            }
        }
        parked.push_back(line);
    };
    let mut queue = leftover.lock().unwrap_or_else(|err| err.into_inner());
    parked.append(&mut queue);
    *queue = parked;
    result
}

fn parse_rpc_ui_response(line: &str, id: &str) -> Option<serde_json::Value> {
    let command: RpcCommand = serde_json::from_str(line).ok()?;
    if command.kind != "extension_ui_response" && command.kind != "decision_response" {
        return None;
    }
    if command.id.as_deref() != Some(id) {
        return None;
    }
    if command.kind == "decision_response" {
        return serde_json::from_str(line).ok();
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

/// The user's `stop` hooks, once per run end: `/quit`, the quit chord, the
/// end of a `--print` or RPC run, and a signal exit.
fn run_stop_hooks_for(parsed: &Args, cwd: &Path) {
    // Once per process: `/quit` returns through the mode's exit, and a
    // fixture signal exit returns through it too.
    static RAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RAN.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let settings = load_merged_settings(&default_agent_dir(), cwd);
    let trusted = is_trusted(&settings, cwd, parsed.project_trust_override);
    hooks::run_stop(&hooks::load(&default_agent_dir(), cwd, trusted));
}

fn emit_session_shutdown(parsed: &Args) {
    loaded_extension_host(parsed).emit(ExtensionEvent::SessionShutdown {
        reason: "quit".into(),
    });
    crate::js_host::shutdown_js_pool();
    // `process::exit` follows: nothing below runs a destructor, so the
    // children are reaped here.
    davinci_agent::jobs::kill_every_job();
    davinci_agent::McpRegistry::shutdown_all();
    let cwd = std::env::current_dir().unwrap_or_default();
    run_stop_hooks_for(parsed, &cwd);
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
    let info = davinci_tui::StartupInfo {
        cwd: Some(session.cwd.to_string_lossy().into_owned()),
        branch: session.chrome.footer_branch.clone(),
        model: session.current_model().map(str::to_string),
        session_restored: false,
    };
    session.chrome.startup_header = Some(davinci_tui::build_startup_header_with(
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
                davinci_tui::format_key_text(&cycle.join("/"), true)
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
    let default_dir = davinci_session::resolve_session_dir_from(
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
    // The legacy chrome answers a permission question with its extension
    // confirm dialog: yes allows once, no denies. No session or project rows;
    // this chrome is the fallback surface, davinci the product.
    let (approval_tx, approval_rx) = std::sync::mpsc::channel::<(
        davinci_agent::ToolApprovalRequest,
        std::sync::mpsc::Sender<davinci_agent::ToolApprovalDecision>,
    )>();
    agent.approver = Some(davinci_agent::ToolApprover(Arc::new(move |request| {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if approval_tx.send((request.clone(), reply_tx)).is_err() {
            return davinci_agent::ToolApprovalDecision::Deny;
        }
        reply_rx
            .recv()
            .unwrap_or(davinci_agent::ToolApprovalDecision::Deny)
    })));
    let mut approval: Option<std::sync::mpsc::Sender<davinci_agent::ToolApprovalDecision>> = None;
    let (decision_tx, decision_rx) = std::sync::mpsc::channel::<(
        davinci_agent::DecisionHostRequest,
        std::sync::mpsc::Sender<davinci_agent::DecisionHostResponse>,
    )>();
    agent.tool_context.decision_responder =
        Some(davinci_agent::DecisionResponder::new(move |request| {
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            if decision_tx.send((request, reply_tx)).is_err() {
                return davinci_agent::DecisionHostResponse::Unavailable;
            }
            reply_rx
                .recv()
                .unwrap_or(davinci_agent::DecisionHostResponse::Cancelled)
        }));
    enum LegacyDecisionState {
        Selecting {
            request: davinci_agent::DecisionHostRequest,
            reply: std::sync::mpsc::Sender<davinci_agent::DecisionHostResponse>,
        },
        Inputting {
            reply: std::sync::mpsc::Sender<davinci_agent::DecisionHostResponse>,
        },
    }
    let mut legacy_decision: Option<LegacyDecisionState> = None;
    session.running = true;
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(|| complete_prompt_with_host(parsed, agent, Some(host), false));
        let frames = davinci_tui::glyphs::SPINNER_FRAMES;
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
            if approval.is_none() && legacy_decision.is_none() {
                if let Ok((request, reply)) = approval_rx.try_recv() {
                    session.open_extension_confirm(
                        format!("Allow {}?", request.summary),
                        format!(
                            "permission mode {} · y runs this call once · n tells the model no",
                            request.mode.as_str()
                        ),
                    );
                    approval = Some(reply);
                    dirty = true;
                } else if let Ok((request, reply)) = decision_rx.try_recv() {
                    let mut options: Vec<String> = request
                        .question
                        .options
                        .iter()
                        .map(|o| o.label.clone())
                        .collect();
                    if request.question.allow_custom {
                        options.push("Custom response".into());
                    }
                    options.push("Defer decision".into());
                    options.push("Cancel".into());
                    session.open_extension_selector(
                        format!("{}: {}", request.question.title, request.question.question),
                        options,
                    );
                    legacy_decision = Some(LegacyDecisionState::Selecting { request, reply });
                    dirty = true;
                }
            }
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
                            // A question left open under an interrupt is a no.
                            if let Some(reply) = approval.take() {
                                let _ = reply.send(davinci_agent::ToolApprovalDecision::Deny);
                                session.close_overlays();
                            }
                            if let Some(state) = legacy_decision.take() {
                                match state {
                                    LegacyDecisionState::Selecting { reply, .. }
                                    | LegacyDecisionState::Inputting { reply } => {
                                        let _ = reply
                                            .send(davinci_agent::DecisionHostResponse::Cancelled);
                                    }
                                }
                                session.close_overlays();
                            }
                            session.chrome.status =
                                "interrupting · waiting for the current step".into();
                            sync_hosted_chrome(tui, panes, session);
                            continue;
                        }
                        match session.handle_bytes(&bytes) {
                            davinci_tui::SessionAction::ExtensionConfirm(confirmed)
                                if approval.is_some() =>
                            {
                                let reply = approval.take().expect("checked above");
                                let _ = reply.send(if confirmed {
                                    davinci_agent::ToolApprovalDecision::AllowOnce
                                } else {
                                    davinci_agent::ToolApprovalDecision::Deny
                                });
                                session.chrome.status.clear();
                            }
                            davinci_tui::SessionAction::ExtensionSelect(selected)
                                if legacy_decision.is_some() =>
                            {
                                if let Some(LegacyDecisionState::Selecting { request, reply }) =
                                    legacy_decision.take()
                                {
                                    match selected.as_deref() {
                                        Some("Cancel") | None => {
                                            let _ = reply.send(
                                                davinci_agent::DecisionHostResponse::Cancelled,
                                            );
                                            session.close_overlays();
                                            session.chrome.status.clear();
                                        }
                                        Some("Defer decision") => {
                                            let _ = reply.send(
                                                davinci_agent::DecisionHostResponse::Reply(
                                                    davinci_agent::DecisionHostReply {
                                                        action:
                                                            davinci_agent::decisions::HostDecisionAction::Defer,
                                                        host_event_id: format!(
                                                            "legacy_{}",
                                                            davinci_session::now_ms()
                                                        ),
                                                        answered_at_ms: davinci_session::now_ms(),
                                                    },
                                                ),
                                            );
                                            session.close_overlays();
                                            session.chrome.status.clear();
                                        }
                                        Some("Custom response")
                                            if request.question.allow_custom =>
                                        {
                                            session.open_extension_input(
                                                "Enter custom response",
                                                "Custom answer...",
                                            );
                                            legacy_decision =
                                                Some(LegacyDecisionState::Inputting { reply });
                                        }
                                        Some(choice_str) => {
                                            if let Some(opt) =
                                                request.question.options.iter().find(|o| {
                                                    o.label == choice_str || o.id == choice_str
                                                })
                                            {
                                                let _ = reply.send(
                                                    davinci_agent::DecisionHostResponse::Reply(
                                                        davinci_agent::DecisionHostReply {
                                                            action:
                                                                davinci_agent::decisions::HostDecisionAction::AnswerChoice(
                                                                    opt.id.clone(),
                                                                ),
                                                            host_event_id: format!(
                                                                "legacy_{}",
                                                                davinci_session::now_ms()
                                                            ),
                                                            answered_at_ms: davinci_session::now_ms(
                                                            ),
                                                        },
                                                    ),
                                                );
                                            } else {
                                                let _ = reply.send(
                                                    davinci_agent::DecisionHostResponse::Cancelled,
                                                );
                                            }
                                            session.close_overlays();
                                            session.chrome.status.clear();
                                        }
                                    }
                                }
                            }
                            davinci_tui::SessionAction::ExtensionInput(text)
                                if legacy_decision.is_some() =>
                            {
                                if let Some(LegacyDecisionState::Inputting { reply }) =
                                    legacy_decision.take()
                                {
                                    if let Some(custom) =
                                        text.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
                                    {
                                        let _ = reply.send(
                                            davinci_agent::DecisionHostResponse::Reply(
                                                davinci_agent::DecisionHostReply {
                                                    action:
                                                        davinci_agent::decisions::HostDecisionAction::AnswerCustom(
                                                            custom,
                                                        ),
                                                    host_event_id: format!(
                                                        "legacy_{}",
                                                        davinci_session::now_ms()
                                                    ),
                                                    answered_at_ms: davinci_session::now_ms(),
                                                },
                                            ),
                                        );
                                    } else {
                                        let _ = reply
                                            .send(davinci_agent::DecisionHostResponse::Cancelled);
                                    }
                                    session.close_overlays();
                                    session.chrome.status.clear();
                                }
                            }
                            davinci_tui::SessionAction::Submit(text) => {
                                if !text.trim().is_empty() {
                                    session
                                        .chrome
                                        .transcript
                                        .push("system", format!("queued: {text}"));
                                    queued.push(text);
                                }
                            }
                            davinci_tui::SessionAction::Quit => {
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
    agent.approver = None;
    agent.approval_responder = None;
    agent.tool_context.decision_responder = None;
    if approval.take().is_some() || legacy_decision.take().is_some() {
        session.close_overlays();
    }
    session.running = false;
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
        startup_mark("interactive extensions loaded");
        let raw: Vec<String> = std::env::args().skip(1).collect();
        return davinci_interactive::run(parsed, agent, &raw, host, migrated_auth_providers);
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
    let prepared = prepare_initial_message(
        &parsed.messages,
        &parsed.file_args,
        None,
        &agent.cwd,
        stored.image_auto_resize(),
    )?;
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
            davinci_tui::NATIVE_SHIFT_ENTER_SEQUENCE.into()
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
                    tui.stop(davinci_tui::TuiStopOptions {
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
            agent.load_from_session(next)?;
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
            match davinci_agent::execute_tool(
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
        SessionAction::CyclePermissionMode => {
            let mode = agent.cycle_permission_mode();
            session.chrome.status = format!("{} · {}", mode.label(), mode.describe());
            Ok(true)
        }
        SessionAction::CycleThinking => {
            sync_session_thinking(session, agent);
            if session.supports_thinking {
                if let Some(level) =
                    davinci_protocol::ThinkingLevel::parse(session.current_thinking())
                {
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
            SlashAction::Hotkeys => {
                let mut keys = davinci_tui::get_keybindings()
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
            SlashAction::Import(_) | SlashAction::Share => {
                handle_user_line(parsed, agent, session, &text, tui)
            }
            _ => handle_user_line(parsed, agent, session, &text, tui),
        },
    }
}

fn unknown_thinking_error(search: &str) -> String {
    let levels = davinci_protocol::ThinkingLevel::all()
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
    let Some(parsed) = davinci_protocol::ThinkingLevel::parse(level) else {
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
    Completed(i32),
    Ready {
        text: String,
        images: Vec<davinci_ai::MessageContent>,
    },
}

/// TS AgentSession.prompt preflight: extension command, input transform/handled, skill/template expand.
fn prepare_user_input(
    parsed: &Args,
    agent: &mut Agent,
    prompt: &str,
    images: &[davinci_ai::MessageContent],
    source: &str,
    mut session: Option<&mut InteractiveSession>,
) -> Result<PreparedInput, String> {
    let mut text = prompt.to_string();
    let mut images = images.to_vec();
    if let Some(result) = permissions::handle_mode_command(agent, &text) {
        let message = result?;
        if parsed.mode == Some(Mode::Json) {
            println!(
                "{}",
                serde_json::json!({"type":"mode_change", "mode":agent.permission_mode().as_str(), "message":message})
            );
        } else {
            println!("{message}");
        }
        return Ok(PreparedInput::Handled);
    }
    if text.starts_with('/') {
        let (name, args) = parse_extension_command(&text);
        if name == "learn" {
            let req = crate::native_extensions::learning::parse_learn_args(&args)?;
            text = crate::native_extensions::learning::build_learn_prompt(&req);
        } else if let Some(session) = session.as_mut() {
            if try_extension_slash(parsed, agent, session, &name, &args)? {
                return Ok(PreparedInput::Handled);
            }
        } else {
            let mut host = loaded_extension_host(parsed);
            apply_graph_session_context(parsed, agent, &host);
            if matches!(name.as_str(), "security-scan" | "sec-resume") {
                let request = if name == "security-scan" {
                    native_extensions::security_scan::command::ScanCommand::parse(&args)
                } else {
                    native_extensions::security_scan::command::ScanCommand::parse("")
                };
                let format = request
                    .as_ref()
                    .map(|request| request.format)
                    .unwrap_or(native_extensions::security_scan::command::ReportFormat::Terminal);
                let outcome = (|| {
                    request?;
                    configure_security_review(parsed, agent, &host)?;
                    let interrupt =
                        native_extensions::security_scan::interrupt::Interrupt::install()?;
                    let result = host
                        .execute_native_command(&name, &args)?
                        .ok_or("security command unavailable")?;
                    let security = host
                        .native
                        .lock()
                        .map_err(|_| "native host lock poisoned")?
                        .security
                        .clone();
                    security.wait_for_review_interruptible(&interrupt);
                    Ok::<_, String>(
                        host.execute_native_command("sec-report", "")?
                            .unwrap_or(result),
                    )
                })();
                let report = outcome.unwrap_or_else(|error| serde_json::json!({"schemaVersion":2,"status":"failed","coverageComplete":false,"limitations":[error]}));
                if parsed.mode == Some(Mode::Json) {
                    println!(
                        "{}",
                        serde_json::json!({"type":"security_scan_report","report":report})
                    );
                } else {
                    println!(
                        "{}",
                        native_extensions::security_scan::report::render(&report, format)?
                    );
                }
                return Ok(PreparedInput::Completed(
                    native_extensions::security_scan::report::exit_code(&report),
                ));
            }
            if name == "sec-report" {
                let report = host.execute_native_command(&name, &args)
                    .and_then(|value| value.ok_or_else(|| "security report unavailable".to_string()))
                    .unwrap_or_else(|error| serde_json::json!({"schemaVersion":2,"status":"failed","coverageComplete":false,"limitations":[error]}));
                if parsed.mode == Some(Mode::Json) {
                    println!(
                        "{}",
                        serde_json::json!({"type":"security_scan_report","report":report})
                    );
                } else if report["schemaVersion"] == 2 {
                    println!(
                        "{}",
                        native_extensions::security_scan::report::render(
                            &report,
                            native_extensions::security_scan::command::ReportFormat::Terminal
                        )?
                    );
                } else {
                    println!(
                        "/{name}: {}",
                        format_extension_command_result(report.clone())
                    );
                }
                return Ok(PreparedInput::Completed(
                    native_extensions::security_scan::report::exit_code(&report),
                ));
            }
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
    text = davinci_agent::expand_user_text(&text, &agent.skills, &agent.templates);
    Ok(PreparedInput::Ready { text, images })
}

fn submit_user_message(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    prompt: &str,
    images: &[davinci_ai::MessageContent],
    mut tui: Option<&mut InteractiveTui>,
) -> Result<bool, String> {
    let prepared = prepare_user_input(parsed, agent, prompt, images, "interactive", Some(session))?;
    let PreparedInput::Ready { text, images } = prepared else {
        return Ok(true);
    };
    session.chrome.transcript.push("user", &text);
    agent.prompt_user_with(&text, &images);
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
        SlashAction::Quit => {
            run_stop_hooks_for(parsed, &agent.cwd);
            Ok(false)
        }
        SlashAction::Prompt(prompt) => {
            submit_user_message(parsed, agent, session, &prompt, &[], tui)
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
            let keys = davinci_tui::get_keybindings()
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
            agent.load_from_session(store)?;
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
            session.chrome.selector = Some(davinci_tui::SelectList::new(
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
                agent.load_from_session(next)?;
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
                agent.load_from_session(next)?;
                println!("session={}", agent.session.as_ref().unwrap().header.id);
            }
            refresh_chrome_footer(session, agent);
            apply_terminal_title(session, agent, tui);
            Ok(true)
        }
        SlashAction::Resume => {
            let items = discover_session_items(parsed, agent)?;
            let mut selector = davinci_tui::SessionSelector::new(items.clone());
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
            let expanded = davinci_session::expand_tilde(&path);
            let next = JsonlSession::open(&expanded).map_err(|err| err.to_string())?;
            agent.load_from_session(next)?;
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
        SlashAction::ShowCost => {
            let text = format_session_cost(parsed, agent);
            session.chrome.transcript.push("cost", &text);
            session.chrome.status = "cost".into();
            println!("{text}");
            Ok(true)
        }
        SlashAction::ShowStatus => {
            let text = format_session_status(parsed, agent);
            session.chrome.transcript.push("status", &text);
            session.chrome.status = "status".into();
            println!("{text}");
            Ok(true)
        }
        SlashAction::Mcp => {
            let rows = agent.tool_context.mcp.rows();
            let text = if rows.is_empty() {
                format!(
                    "no MCP servers connected — config: {}",
                    default_agent_dir().join("mcp.json").display()
                )
            } else {
                rows.into_iter()
                    .map(|row| {
                        let error = row.error.map(|err| format!(" · {err}")).unwrap_or_default();
                        format!(
                            "{}  {}  {}  {} tools{error}",
                            row.status, row.transport, row.name, row.tools
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            session.chrome.transcript.push("mcp", &text);
            session.chrome.status = "mcp".into();
            println!("{text}");
            Ok(true)
        }
        SlashAction::Agents => {
            let settings = load_merged_settings(&default_agent_dir(), &agent.cwd);
            let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
            let text = agent_profiles::format_agent_profiles_status(&agent.cwd, None, trusted);
            session.chrome.transcript.push("agents", &text);
            session.chrome.status = "agents".into();
            println!("{text}");
            Ok(true)
        }
        SlashAction::Tasks => {
            let tasks = if let Some(runtime) = &agent.runtime {
                davinci_surfaces::task_board(runtime)
            } else {
                Vec::new()
            };
            let text = if tasks.is_empty() {
                "no tasks tracked".into()
            } else {
                tasks
                    .into_iter()
                    .map(|t| {
                        format!(
                            "{}  [{}]  {}  (owner: {})",
                            t.id,
                            t.status,
                            t.title,
                            t.owner.as_deref().unwrap_or("unassigned")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            session.chrome.transcript.push("tasks", &text);
            session.chrome.status = "tasks".into();
            println!("{text}");
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
    clear_model_runtime_cache();
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
        for path in settings::collect_package_resources(
            pkg,
            "themes",
            &default_agent_dir(),
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
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
        "decision-intelligence" => {
            stored.decision_intelligence = Some(crate::settings::DecisionIntelligenceSettings {
                enabled: value == "on",
            });
        }
        "transport" => stored.transport = Some(value.to_string()),
        "http-idle-timeout" => stored.http_idle_timeout_ms = parse_http_idle_timeout(value),
        "hide-thinking" => stored.hide_thinking_block = Some(value == "true"),
        "show-tool-output" => stored.show_tool_output = Some(value == "true"),
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
    let stored = load_merged_settings(&default_agent_dir(), &agent.cwd);
    agent.auto_compaction = stored.compaction_enabled();
    agent.compaction = stored.compaction_settings();
    agent.block_images = stored.block_images();
    agent.auto_resize_images = stored.image_auto_resize();
    agent.transport = stored.transport.clone();
    agent.install_telemetry = stored.install_telemetry_enabled();
    agent.auto_retry = stored.retry_enabled();
    if !stored.decision_intelligence_enabled() {
        agent.disable_decision_runtime();
    }
}

fn fixtures_enabled() -> bool {
    cfg!(any(test, feature = "test-fixtures"))
}

fn looks_like_oauth_input(value: &str) -> bool {
    (fixtures_enabled() && value.starts_with("pi-fixture-"))
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
    if provider == "anthropic" {
        return;
    }
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
    if !davinci_ai::credential_expires_by(cred, now.saturating_add(min_expiry_ms)) {
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
    if provider == "anthropic" {
        return;
    }
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

fn ensure_supported_stored_oauth(
    storage: Option<&AuthStorage>,
    provider: &str,
    explicit_api_key: Option<&str>,
) -> Result<(), String> {
    if explicit_api_key.is_none()
        && provider == "anthropic"
        && storage
            .and_then(|storage| storage.get(provider))
            .is_some_and(|credential| credential.kind == CredentialKind::Oauth)
    {
        return Err(davinci_ai::ANTHROPIC_OAUTH_UNSUPPORTED_MESSAGE.into());
    }
    Ok(())
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
    if provider == "anthropic" && key.is_none() {
        println!("Anthropic login uses an API key: /login anthropic sk-ant-...");
        return Ok(false);
    }
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
        if provider == "anthropic" && looks_like_oauth_input(key) {
            return Err(davinci_ai::ANTHROPIC_OAUTH_UNSUPPORTED_MESSAGE.into());
        }
        if looks_like_oauth_input(key) {
            let (code, pasted_state) = davinci_ai::parse_authorization_input(key);
            let code = code.ok_or_else(|| "Missing authorization code.".to_string())?;
            let pending = davinci_ai::take_pending_login(&default_agent_dir(), provider)
                .ok_or_else(|| {
                    format!(
                        "No login in progress for {provider}. Run /login {provider} first, then paste the redirect URL."
                    )
                })?;
            if let (Some(expected), Some(got)) =
                (pending.state.as_deref(), pasted_state.as_deref())
            {
                if expected != got {
                    return Err(
                        "The pasted login does not match the one started here (state mismatch)."
                            .into(),
                    );
                }
            }
            let tokens = davinci_ai::exchange_authorization_code(
                provider,
                &code,
                pending.pkce.as_ref(),
                pending.state.as_deref(),
            )?;
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
    if let Some(request) = davinci_ai::fresh_authorize_request(provider) {
        if let Ok(code) = std::env::var("PI_OAUTH_CODE") {
            let tokens = davinci_ai::exchange_authorization_code(
                provider,
                &code,
                request.pkce.as_ref(),
                request.state.as_deref(),
            )?;
            return store_oauth_tokens(&mut storage, provider, tokens);
        }
        let should_wait = wait_for_oauth_callback || std::env::var("PI_OAUTH_WAIT").is_ok();
        if should_wait {
            if let Some(kind) = davinci_ai::CallbackProvider::parse(provider) {
                let host = davinci_ai::callback_host();
                let port = std::env::var("PI_OAUTH_CALLBACK_PORT")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| kind.default_port());
                let expected = request
                    .state
                    .clone()
                    .or_else(|| request.pkce.as_ref().map(|pkce| pkce.verifier.clone()))
                    .unwrap_or_default();
                let mut server = davinci_ai::CallbackServer::bind(&host, port, kind, expected)?;
                println!("{}", request.url);
                println!("{}", request.instructions);
                println!("Waiting for browser callback on {}", server.redirect_uri()?);
                let response = server.accept_until(
                    std::time::Instant::now() + std::time::Duration::from_secs(300),
                )?;
                if let Some(code) = response.code {
                    let tokens = davinci_ai::exchange_authorization_code(
                        provider,
                        &code,
                        request.pkce.as_ref(),
                        request.state.as_deref(),
                    )?;
                    return store_oauth_tokens(&mut storage, provider, tokens);
                }
                return Err("OAuth callback did not include an authorization code.".into());
            }
        }
        davinci_ai::save_pending_login(&default_agent_dir(), provider, &request)?;
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
            davinci_ai::OauthTokens {
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
    tokens: davinci_ai::OauthTokens,
) -> Result<bool, String> {
    let expires = tokens
        .expires
        .or_else(|| davinci_ai::jwt_expiry_ms(&tokens.access));
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

pub fn format_session_cost(parsed: &Args, agent: &Agent) -> String {
    let models = available_models(parsed);
    let found = models
        .iter()
        .find(|item| item.provider == agent.provider && item.id == agent.model_id);
    let stats = rpc::session_stats_for_agent(agent, found);
    format!(
        "input {} · output {} · cache read {} · cache write {} · total {} · ${:.4}",
        stats["tokens"]["input"].as_u64().unwrap_or(0),
        stats["tokens"]["output"].as_u64().unwrap_or(0),
        stats["tokens"]["cacheRead"].as_u64().unwrap_or(0),
        stats["tokens"]["cacheWrite"].as_u64().unwrap_or(0),
        stats["tokens"]["total"].as_u64().unwrap_or(0),
        stats
            .get("cost")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
    )
}

pub fn format_session_status(parsed: &Args, agent: &Agent) -> String {
    let mode = agent
        .permissions
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .mode
        .as_str();
    let plan = if agent.is_plan_mode() { "plan" } else { "act" };
    let jobs = agent
        .tool_context
        .jobs
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .running();
    let mcp = agent.tool_context.mcp.rows().len();
    let mut text = format!(
        "{}/{} · {mode} · {plan} · {} jobs · {mcp} mcp · {}",
        agent.provider,
        agent.model_id,
        jobs,
        format_session_cost(parsed, agent),
    );
    if let Some(manifest) = &agent.prompt_manifest {
        let hash_prefix = if manifest.stable_sha256.len() >= 8 {
            &manifest.stable_sha256[..8]
        } else {
            &manifest.stable_sha256
        };
        let candidate_suffix = if let Some(candidate) = &agent.prompt_session.candidate_id {
            format!(" [{candidate}]")
        } else {
            String::new()
        };
        let model_policy_suffix = if manifest.model_policy == "default" {
            String::new()
        } else {
            format!(
                " · {} v{}",
                manifest.model_policy, manifest.model_policy_version
            )
        };
        text.push_str(&format!(
            " · prompt: {} v{}{candidate_suffix}{model_policy_suffix} · {hash_prefix}",
            manifest.profile, manifest.profile_version
        ));
        if let Some(diag) = &agent.prompt_session.transition_diagnostic {
            text.push_str(&format!(" · transition: {diag}"));
        }
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Some(run) = crate::native_extensions::graph::active_run(&cwd).and_then(|r| r.snapshot())
    {
        let compact = run.ecosystem_stats.render_compact_lines();
        if !compact.is_empty() {
            text.push('\n');
            text.push_str(&compact.join("\n"));
        }
    }
    let behavior_runs = davinci_telemetry::get_behavior_telemetry();
    if !behavior_runs.is_empty() {
        let profile_filter = agent
            .prompt_manifest
            .as_ref()
            .map(|m| m.profile.as_str())
            .unwrap_or("stable");
        let report =
            davinci_telemetry::LocalBehaviorReport::from_runs(profile_filter, &behavior_runs);
        text.push_str("\n\n");
        text.push_str(&report.format_report());
    }
    text
}

fn format_session_info(
    stats: &serde_json::Value,
    waste: Option<&cache_stats::CacheWasteTotals>,
) -> String {
    let mut info = format!(
        "Session Info\n\nFile: {}\nID: {}\n\nMessages\nTotal: {}\nUser: {}\nAssistant: {}\nTools: {} calls, {} results\n\nTokens\nInput: {}\nOutput: {}\nCache read: {}\nCache write: {}\nTotal: {}\nCost: {}{}",
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
        format_runtime_stats(stats.get("runtime")),
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

/// The harness counters of this run (`davinci_agent::RunStats`), as a block for
/// `/status`: turns against tool calls, batch width, time split between
/// the model and the tools, and what pruning saved.
fn format_runtime_stats(runtime: Option<&serde_json::Value>) -> String {
    let Some(runtime) = runtime else {
        return String::new();
    };
    let get = |key: &str| {
        runtime
            .get(key)
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
    };
    if get("modelTurns") == 0 {
        return String::new();
    }
    let batches = get("toolBatches");
    let mean_width = if batches == 0 {
        0.0
    } else {
        get("toolCalls") as f64 / batches as f64
    };
    format!(
        "\n\nRuntime (this run)\nModel turns: {}\nTool calls: {} in {} batches (mean width {:.1}, max {}, {} parallel groups)\nBatch operations: {}\nWorkers: {}\nTime: model {:.1}s, tools {:.1}s\nPeak context: {} tokens\nPruned: {} results, {} chars\nCompactions: {}\nEvidence files: {}",
        get("modelTurns"),
        get("toolCalls"),
        batches,
        mean_width,
        get("maxBatchWidth"),
        get("parallelGroups"),
        get("batchOperations"),
        get("subagents"),
        get("modelWallMs") as f64 / 1000.0,
        get("toolWallMs") as f64 / 1000.0,
        get("peakContextTokens"),
        get("prunedResults"),
        get("prunedChars"),
        get("compactions"),
        get("evidenceFiles"),
    )
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

thread_local! {
    // The legacy chrome creates an extension host per command. Retain only its
    // scanner controller, keyed by the actual session, between those calls.
    static LEGACY_SECURITY: std::cell::RefCell<Option<(String, native_extensions::SecurityScanController)>> = const { std::cell::RefCell::new(None) };
}

fn try_extension_slash(
    parsed: &Args,
    agent: &mut Agent,
    session: &mut InteractiveSession,
    name: &str,
    args: &str,
) -> Result<bool, String> {
    let mut host = loaded_extension_host(parsed);
    let security_command = matches!(
        name,
        "security-scan" | "sec-resume" | "sec-status" | "sec-report" | "sec-abort"
    );
    let session_key = agent
        .session
        .as_ref()
        .map(|session| session.header.id.clone())
        .unwrap_or_else(|| agent.cwd.to_string_lossy().into_owned());
    if security_command {
        LEGACY_SECURITY.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some((key, controller)) = slot.as_mut() {
                if key == &session_key {
                    host.native
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .security = controller.clone();
                } else {
                    let _ = controller.command("sec-abort", "");
                }
            }
        });
    }
    apply_graph_session_context(parsed, agent, &host);
    if matches!(name, "security-scan" | "sec-resume") {
        configure_security_review(parsed, agent, &host)?;
    }
    let result = host.execute_native_command(name, args);
    if security_command {
        let controller = host
            .native
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .security
            .clone();
        LEGACY_SECURITY.with(|slot| *slot.borrow_mut() = Some((session_key, controller)));
    }
    if let Some(result) = result? {
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
        .map(davinci_tui::overlay_options_from_json);
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
            roots.extend(project_config::all(&agent.cwd, "skills"));
        }
        if let Some(extra) = &settings.skills {
            roots.extend(extra.iter().map(PathBuf::from));
        }
        for pkg in &settings.packages {
            roots.extend(settings::collect_package_resources(pkg, "skills", &default_agent_dir(), &agent.cwd));
        }
        agent.skills = discover_skills(&roots);
    }
    if !parsed.no_prompt_templates {
        let mut roots: Vec<PathBuf> = parsed.prompt_templates.iter().map(PathBuf::from).collect();
        roots.push(default_agent_dir().join("prompts"));
        if trusted {
            roots.extend(project_config::all(&agent.cwd, "prompts"));
        }
        if let Some(extra) = &settings.prompts {
            roots.extend(extra.iter().map(PathBuf::from));
        }
        for pkg in &settings.packages {
            roots.extend(settings::collect_package_resources(pkg, "prompts", &default_agent_dir(), &agent.cwd));
        }
        agent.templates = discover_prompt_templates(&roots);
    }
    agent.context_files = load_context_files(&agent.cwd, !parsed.no_context_files);
}

fn sync_visual_verification_availability(agent: &mut Agent, host: &ExtensionHost) {
    agent.set_visual_verification_available(host.visual_verification_available());
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
    sync_visual_verification_availability(agent, host);
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
    let home = davinci_session::home_dir()
        .map(|home| home.display().to_string())
        .unwrap_or_default();
    let cwd = agent.cwd.display().to_string();
    let agent_dir = default_agent_dir().display().to_string();
    let mut loaded = davinci_tui::LoadedResources::default();
    if show_listing {
        if !agent.context_files.is_empty() {
            let compact = theme.fg(
                "dim",
                &davinci_tui::format_compact_list(
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
    loaded: &mut davinci_tui::LoadedResources,
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
        for path in settings::collect_package_resources(
            pkg,
            "themes",
            &default_agent_dir(),
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
            if path.is_dir() {
                files.extend(theme_files_from_dir(&path));
            } else if let Some(parent) = path.parent() {
                files.extend(theme_files_from_dir(parent));
            }
        }
    }
    files
}

fn bind_test_impact_context(agent: &Agent, host: &ExtensionHost) {
    let native = host
        .native
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    native
        .test_impact
        .set_permissions(agent.permissions.clone());
    native
        .test_impact
        .set_cancellation(agent.abort_signal.clone());
    native
        .verification_planner
        .set_permissions(agent.permissions.clone());
    native
        .verification_planner
        .set_cancellation(agent.abort_signal.clone());
    native
        .workspace_snapshot
        .set_permissions(agent.permissions.clone());
    native
        .workspace_snapshot
        .set_cancellation(agent.abort_signal.clone());
}

fn attach_tool_executor(agent: &mut Agent, host: &ExtensionHost) {
    bind_test_impact_context(agent, host);
    let host = host.clone();
    agent.custom_tool_executor = Some(CustomToolExecutor::new_with_context(
        move |cwd, name, args, context| {
            host.execute_js_or_manifest_tool_with_context(cwd, name, args, context)
        },
    ));
}

fn attach_shared_tool_executor(agent: &mut Agent, host: Arc<Mutex<ExtensionHost>>) {
    bind_test_impact_context(
        agent,
        &host.lock().unwrap_or_else(|error| error.into_inner()),
    );
    agent.custom_tool_executor = Some(CustomToolExecutor::new_with_context(
        move |cwd, name, args, context| {
            let host = host
                .lock()
                .map_err(|error| davinci_agent::ToolError::Failed(error.to_string()))?
                .clone();
            host.execute_js_or_manifest_tool_with_context(cwd, name, args, context)
        },
    ));
}

/// Hand the graph controller the session's model, thinking level, and trust
/// decision so the workers it spawns inherit them.
fn configure_security_review(
    parsed: &Args,
    agent: &Agent,
    host: &ExtensionHost,
) -> Result<(), String> {
    use native_extensions::security_scan::worker::SecurityWorkerRunner;
    if parsed.offline
        || ["DAVINCI_OFFLINE", "PI_OFFLINE", "PI_DISABLE_NETWORK"]
            .iter()
            .any(|name| matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "yes")))
    {
        if fixtures_enabled() {
            if let Ok(path) = std::env::var("PI_SECURITY_SCAN_FIXTURE") {
                let runner = SecurityWorkerRunner::from_offline_fixture(Path::new(&path))?;
                let config = crate::settings::load_security_scan_config(
                    &default_agent_dir(),
                    &agent.cwd,
                    false,
                )?;
                let mut native = host
                    .native
                    .lock()
                    .map_err(|_| "native host lock poisoned")?;
                native.security.configure_review(runner, config);
                native.security.set_review_storage(default_agent_dir());
                davinci_agent::runtime::capacity::bind_shared_directory(
                    default_agent_dir().join("capacity"),
                );
                return Ok(());
            }
        }
        return Err(
            "offline mode forbids security provider requests; no model review was started".into(),
        );
    }
    if davinci_ai::trace::enabled() {
        return Err("disable AI wire tracing before reviewing private source".into());
    }
    let settings = load_merged_settings_with_override(
        &default_agent_dir(),
        &agent.cwd,
        parsed.project_trust_override,
    );
    let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
    let config =
        crate::settings::load_security_scan_config(&default_agent_dir(), &agent.cwd, trusted)?;
    let models = available_models(parsed);
    let model = find_model(&models, &agent.provider, &agent.model_id)
        .cloned()
        .ok_or("selected security review model is unavailable")?;
    let mut storage = AuthStorage::create().map_err(|_| "cannot load provider authorization")?;
    if let Some(key) = parsed.api_key.as_deref() {
        storage.set_runtime_override(&agent.provider, key);
    }
    let env = std::env::vars().collect();
    let auth = resolve_provider_auth(&agent.provider, &storage, &env, true)
        .ok_or("selected security review provider has no existing authorization")?;
    let supported = davinci_ai::get_supported_thinking_levels(&model);
    let thinking = native_extensions::security_scan::worker::effective_thinking_level(
        agent.thinking_level,
        &supported,
    )?;
    let provenance = serde_json::json!({"provider":model.provider,"modelId":model.id,"requestedThinkingLevel":agent.thinking_level.as_str(),"effectiveThinkingLevel":thinking.as_str(),"qualityEvaluation":"unmeasured"});
    let runner = SecurityWorkerRunner::new(move |request| {
        let options = davinci_ai::StreamOptions {
            thinking_level: Some(thinking),
            timeout_ms: Some(120_000),
            max_retries: Some(0),
            max_tokens: Some(request.max_output_tokens),
            install_telemetry: Some(false),
            abort_signal: Some(request.run.abort_signal()),
            session_id: Some(request.run.status().scan_id),
            ..Default::default()
        };
        davinci_ai::live_complete_streaming_with_sink(
            &model,
            request.messages,
            &auth,
            Some(request.system),
            request.tools,
            &options,
            &mut |_| {},
        )
        .map(|(response, _)| response)
        .map_err(|_| "security provider request failed".into())
    })
    .with_provenance(provenance);
    let mut native = host
        .native
        .lock()
        .map_err(|_| "native host lock poisoned")?;
    native.security.configure_review(runner, config);
    native.security.set_review_storage(default_agent_dir());
    davinci_agent::runtime::capacity::bind_shared_directory(default_agent_dir().join("capacity"));
    Ok(())
}

fn apply_graph_session_context(parsed: &Args, agent: &Agent, host: &ExtensionHost) {
    if let Ok(mut native) = host.native.lock() {
        native
            .test_impact
            .set_permissions(agent.permissions.clone());
        native
            .language_intelligence
            .set_permissions(Some(agent.permissions.clone()));
        native
            .verification_planner
            .set_permissions(agent.permissions.clone());
        native
            .verification_planner
            .set_cancellation(agent.abort_signal.clone());
        native
            .workspace_snapshot
            .set_permissions(agent.permissions.clone());
        native
            .workspace_snapshot
            .set_cancellation(agent.abort_signal.clone());
        native.security.set_review_storage(default_agent_dir());
        native
            .graph
            .set_runtime(agent.runtime_for_session().cloned());
        native
            .graph
            .set_permissions(Some(agent.permissions.clone()));
        native.graph.set_task_contract(agent.active_contract());
        native.graph.processes = agent.tool_context.processes.clone();
        native.graph.browser = agent
            .tool_context
            .foreground_supervisor
            .clone()
            .map(|supervisor| native_extensions::browser::BrowserWorkerHost {
                controller: native.browser.clone(),
                supervisor,
            });
    }
    let settings = load_merged_settings_with_override(
        &default_agent_dir(),
        &agent.cwd,
        parsed.project_trust_override,
    );
    let trusted = is_trusted(&settings, &agent.cwd, parsed.project_trust_override);
    let thinking = match agent.thinking_level {
        davinci_protocol::ThinkingLevel::Off => None,
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

fn tree_entry_from_session(entry: &davinci_session::SessionEntry) -> SessionTreeEntry {
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
    session.chrome.footer_home = davinci_session::home_dir().map(|home| home.display().to_string());
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
        davinci_agent::estimate_context_tokens(&agent.messages),
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
            .and_then(|value| serde_json::from_value::<davinci_protocol::Usage>(value).ok())
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
    Davinci(&'a mut davinci_tui::davinci::model::Model),
    Silent,
}

impl SessionCallUi<'_> {
    /// A transcript line: a user message an extension sent, exec output.
    fn push(&mut self, kind: &str, text: &str) {
        use davinci_tui::davinci::model::Entry;
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
        use davinci_tui::davinci::model::Entry;
        use davinci_tui::davinci::theme::State;
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
) -> Vec<(usize, String)> {
    let mut failures = Vec::new();
    for (index, call) in calls.iter().enumerate() {
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
            Some(op @ ("newSession" | "fork" | "switchSession")) => {
                let parsed = parsed.cloned().unwrap_or_default();
                let session_dir = resolved_session_dir(&parsed, &agent.cwd);
                let next = match op {
                    "newSession" => {
                        JsonlSession::create(&session_dir, &agent.cwd.to_string_lossy(), None)
                            .map_err(|error| error.to_string())
                    }
                    "fork" => agent
                        .session
                        .as_ref()
                        .ok_or_else(|| "No session to fork".to_string())
                        .and_then(|store| {
                            let entry_id = call
                                .get("entryId")
                                .and_then(|value| value.as_str())
                                .or(store.leaf_id.as_deref())
                                .unwrap_or(&store.header.id);
                            store
                                .fork(entry_id, &session_dir)
                                .map_err(|error| error.to_string())
                        }),
                    _ => call
                        .get("sessionPath")
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| "No session path to switch to".to_string())
                        .and_then(|path| {
                            JsonlSession::open(Path::new(path)).map_err(|error| error.to_string())
                        }),
                };
                if let Err(error) = next.and_then(|next| agent.load_from_session(next)) {
                    let error = if error.starts_with("Runtime recovery required:") {
                        error
                    } else {
                        format!("Runtime recovery required: {error}")
                    };
                    ui.status(&error);
                    failures.push((index, error));
                    failures.extend((index + 1..calls.len()).map(|index| {
                        (
                            index,
                            "Session call skipped after failed session activation".into(),
                        )
                    }));
                    break;
                }
                match op {
                    "newSession" => ui.status("newSession"),
                    "fork" => ui.status(&format!(
                        "fork={}",
                        agent.session.as_ref().unwrap().header.id
                    )),
                    _ => {
                        ui.status(&format!(
                            "session={}",
                            agent.session.as_ref().unwrap().path.display()
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
                // Match startup's trust-gated root assembly so reload neither
                // loads untrusted project resources nor drops user and CLI roots.
                let fallback;
                let args = match parsed {
                    Some(args) => args,
                    None => {
                        fallback = Args::default();
                        &fallback
                    }
                };
                apply_discovered_resources(args, agent);
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
                    .and_then(davinci_protocol::ThinkingLevel::parse)
                {
                    agent.thinking_level = level;
                    ui.status(&format!("thinking {}", level.as_str()));
                }
            }
            _ => {}
        }
    }
    failures
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
        session.chrome.selector = Some(davinci_tui::SelectList::new(session.models.clone()));
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
    let roots = davinci_tui::build_session_tree(entries);
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

fn current_runtime_model(agent: &Agent) -> Option<davinci_ai::Model> {
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
        .and_then(|level| davinci_protocol::ThinkingLevel::parse(level));
    let per_model = session
        .model_thinking_levels
        .get(&key)
        .and_then(|level| davinci_protocol::ThinkingLevel::parse(level));
    let default_level = load_settings(&default_agent_dir())
        .default_thinking_level
        .as_deref()
        .and_then(davinci_protocol::ThinkingLevel::parse);
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
    let mut dummy = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
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
    if let Some(request) = davinci_ai::fresh_authorize_request(provider) {
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
#[path = "main_tests.rs"]
mod tests;
