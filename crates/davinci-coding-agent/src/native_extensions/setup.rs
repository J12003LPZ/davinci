//! `/setup`: prepare the current workspace for the features that need the
//! user to do something before they work — vector memory's embedding server
//! and model, project trust, plugin hook approval, language servers, project
//! instructions and ignoring local state in Git.
//!
//! No TypeScript counterpart. `/setup` checks every area and applies the
//! steps that are local and reversible: it starts an installed Ollama, pulls
//! the configured embedding model in the background, embeds records that have
//! no vector and adds Davinci's local state to `.gitignore`. `/setup check`
//! only reports. Trusting the project runs its hooks, extensions and MCP
//! servers, so it is never automatic: `/setup trust` records that decision.

use super::memory_page;
use super::vector_memory::VectorMemory;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Local state Davinci writes under the workspace that should never be
/// committed: conversation-derived memory records and graph run journals.
pub const IGNORED_STATE: &[&str] = &[".davinci/vector-memory/", ".davinci/graph/"];
/// Records embedded per batch after a pull; the memory lock is released
/// between batches so a turn is never held behind the whole backlog.
const REINDEX_BATCHES: usize = 40;
/// How long `/setup` waits for a freshly started `ollama serve` to answer.
const SERVE_WAIT: Duration = Duration::from_secs(8);
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Report every area and apply the local, reversible steps.
    Apply,
    /// Report only.
    Check,
    /// Record that this project is trusted.
    Trust,
}

pub fn parse_args(args: &str) -> Result<Mode, String> {
    match args.trim() {
        "" | "apply" => Ok(Mode::Apply),
        "check" | "--check" | "status" => Ok(Mode::Check),
        "trust" => Ok(Mode::Trust),
        other => Err(format!(
            "unknown /setup argument `{other}` · use /setup, /setup check or /setup trust"
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    /// Nothing to do.
    Ready,
    /// `/setup` changed something just now.
    Done,
    /// Work continues in the background; run `/setup` again to follow it.
    Running,
    /// Needs the user; `action` says what to run.
    Action,
    /// Not relevant to this workspace or turned off.
    Skipped,
}

impl StepState {
    fn mark(self) -> &'static str {
        match self {
            StepState::Ready => "✓",
            StepState::Done => "✓",
            StepState::Running => "…",
            StepState::Action => "!",
            StepState::Skipped => "-",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: &'static str,
    pub title: &'static str,
    pub state: StepState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl Step {
    fn new(id: &'static str, title: &'static str, state: StepState, detail: String) -> Self {
        Self {
            id,
            title,
            state,
            detail,
            action: None,
        }
    }

    fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    fn line(&self) -> String {
        let mut line = format!("{} {} · {}", self.state.mark(), self.title, self.detail);
        if let Some(action) = &self.action {
            line.push_str(&format!(" → {action}"));
        }
        line
    }
}

/// The answer every mode returns: `lines` for the transcript, `steps` for
/// RPC clients and tests.
pub fn report(mode: Mode, steps: Vec<Step>) -> Value {
    let pending = steps
        .iter()
        .filter(|step| step.state == StepState::Action)
        .count();
    let running = steps.iter().any(|step| step.state == StepState::Running);
    let summary = match (pending, running) {
        (0, false) => "workspace is set up".to_string(),
        (0, true) => "setup continues in the background · run /setup again to follow it".into(),
        (count, _) => format!("{count} step(s) need you"),
    };
    json!({
        "mode": match mode {
            Mode::Apply => "apply",
            Mode::Check => "check",
            Mode::Trust => "trust",
        },
        "summary": summary,
        "lines": steps.iter().map(Step::line).collect::<Vec<_>>(),
        "steps": steps,
    })
}

// ---------------------------------------------------------------------------
// Vector memory
// ---------------------------------------------------------------------------

/// Progress of the one embedding-model pull a process runs at a time. Kept
/// process-wide so `/reload`, which rebuilds the native host, does not lose
/// sight of a pull that is still downloading.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    pub model: String,
    pub url: String,
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub finished: bool,
    pub error: Option<String>,
    pub embedded: usize,
}

fn pulls() -> &'static Mutex<Option<PullProgress>> {
    static PULLS: OnceLock<Mutex<Option<PullProgress>>> = OnceLock::new();
    PULLS.get_or_init(|| Mutex::new(None))
}

pub fn pull_progress() -> Option<PullProgress> {
    pulls().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The pull of `model` from the server at `url`, if one ran in this process.
fn pull_for(url: &str, model: &str) -> Option<PullProgress> {
    pull_progress().filter(|p| p.url == url && p.model == model)
}

/// A pull that succeeded is reported once, by the next `/setup`, then
/// forgotten; the probe says whether the model is installed after that.
fn take_finished_pull(url: &str, model: &str) -> Option<PullProgress> {
    let mut guard = pulls().lock().unwrap_or_else(|e| e.into_inner());
    let matches = guard
        .as_ref()
        .is_some_and(|p| p.url == url && p.model == model && p.finished && p.error.is_none());
    if matches {
        guard.take()
    } else {
        None
    }
}

fn update_pull(change: impl FnOnce(&mut PullProgress)) {
    let mut guard = pulls().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(progress) = guard.as_mut() {
        change(progress);
    }
}

fn pull_line(progress: &PullProgress) -> String {
    if progress.total > 0 {
        let percent = progress.completed.saturating_mul(100) / progress.total.max(1);
        format!(
            "pulling {} · {} · {percent}% of {} MB",
            progress.model,
            progress.status,
            progress.total / 1_000_000
        )
    } else {
        format!("pulling {} · {}", progress.model, progress.status)
    }
}

/// Start `ollama pull` through the HTTP API on a background thread, then
/// embed the records that have no vector. Returns false when a pull for the
/// same model is already running.
pub fn start_pull(memory: Arc<Mutex<VectorMemory>>, url: &str, model: &str) -> bool {
    {
        let mut guard = pulls().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = guard.as_ref() {
            if !existing.finished && existing.model == model {
                return false;
            }
        }
        *guard = Some(PullProgress {
            model: model.to_string(),
            url: url.to_string(),
            status: "starting".into(),
            ..PullProgress::default()
        });
    }
    let url = url.trim_end_matches('/').to_string();
    let model = model.to_string();
    std::thread::spawn(move || {
        let result = pull_model(&url, &model);
        match result {
            Ok(()) => {
                update_pull(|progress| progress.status = "embedding records".into());
                let embedded = embed_backlog(&memory);
                update_pull(|progress| {
                    progress.status = "done".into();
                    progress.embedded = embedded;
                    progress.finished = true;
                });
            }
            Err(error) => update_pull(|progress| {
                progress.status = "failed".into();
                progress.error = Some(error);
                progress.finished = true;
            }),
        }
    });
    true
}

fn pull_model(url: &str, model: &str) -> Result<(), String> {
    // A pull streams progress lines for minutes; only a stalled read fails it.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(120))
        .build();
    let response = agent
        .post(&format!("{url}/api/pull"))
        .send_json(json!({"model": model, "name": model, "stream": true}))
        .map_err(|error| error.to_string())?;
    let reader = BufReader::new(response.into_reader());
    let mut succeeded = false;
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(error) = event["error"].as_str() {
            return Err(error.to_string());
        }
        let status = event["status"].as_str().unwrap_or("").to_string();
        succeeded |= status == "success";
        update_pull(|progress| {
            if !status.is_empty() {
                progress.status = status.clone();
            }
            if let Some(total) = event["total"].as_u64() {
                progress.total = total;
            }
            if let Some(completed) = event["completed"].as_u64() {
                progress.completed = completed;
            }
        });
    }
    if succeeded {
        Ok(())
    } else {
        Err("the pull ended without reporting success".into())
    }
}

/// Embed every record without a vector, one bounded batch per lock.
pub fn embed_backlog(memory: &Arc<Mutex<VectorMemory>>) -> usize {
    let mut total = 0;
    for _ in 0..REINDEX_BATCHES {
        let mut memory = memory.lock().unwrap_or_else(|e| e.into_inner());
        memory.clear_dense_offline();
        let Ok(answer) = memory.reindex() else { break };
        let embedded = answer["reembedded"].as_u64().unwrap_or(0) as usize;
        total += embedded;
        if embedded == 0 || !answer["embeddingError"].is_null() {
            break;
        }
    }
    total
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|directory| {
        let plain = directory.join(program);
        let mut candidates = vec![plain.clone()];
        if cfg!(windows) {
            for extension in ["exe", "cmd", "bat"] {
                candidates.push(plain.with_extension(extension));
            }
        }
        candidates.into_iter().find(|candidate| candidate.is_file())
    })
}

/// Only a local Ollama is ever started; a remote URL is the user's server.
fn is_local_url(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "0.0.0.0")
}

fn spawn_ollama_serve(program: &Path) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    command
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn wait_for_server(memory: &Arc<Mutex<VectorMemory>>) -> memory_page::Probe {
    let started = Instant::now();
    loop {
        let probe = {
            let memory = memory.lock().unwrap_or_else(|e| e.into_inner());
            memory_page::probe(&memory)
        };
        if probe.reachable || started.elapsed() >= SERVE_WAIT {
            return probe;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

pub fn memory_step(memory: &Arc<Mutex<VectorMemory>>, apply: bool) -> Step {
    const ID: &str = "memory";
    const TITLE: &str = "vector memory";
    let (enabled, url, model, dimensions) = {
        let memory = memory.lock().unwrap_or_else(|e| e.into_inner());
        (
            memory.config.enabled,
            memory.config.ollama_url.clone(),
            memory.config.embedding_model.clone(),
            memory.config.embedding_dimensions,
        )
    };
    if !enabled {
        return Step::new(
            ID,
            TITLE,
            StepState::Skipped,
            "disabled by vector-memory.json or PI_MEMORY_ENABLED=0".into(),
        );
    }
    if let Some(progress) = pull_for(&url, &model).filter(|p| !p.finished) {
        return Step::new(ID, TITLE, StepState::Running, pull_line(&progress));
    }

    let mut probe = {
        let memory = memory.lock().unwrap_or_else(|e| e.into_inner());
        memory_page::probe(&memory)
    };
    let mut started_server = false;
    if !probe.reachable {
        let installed = find_on_path("ollama");
        match installed {
            None => {
                return Step::new(
                    ID,
                    TITLE,
                    StepState::Action,
                    format!("no Ollama at {url}; search uses keywords only"),
                )
                .with_action(format!(
                    "install Ollama from https://ollama.com/download, then run /setup (it pulls {model})"
                ));
            }
            Some(program) if apply && is_local_url(&url) => {
                if let Err(error) = spawn_ollama_serve(&program) {
                    return Step::new(
                        ID,
                        TITLE,
                        StepState::Action,
                        format!("could not start {}: {error}", program.display()),
                    )
                    .with_action("start Ollama (`ollama serve`), then run /setup");
                }
                probe = wait_for_server(memory);
                if !probe.reachable {
                    return Step::new(
                        ID,
                        TITLE,
                        StepState::Action,
                        format!("started `ollama serve` but {url} did not answer yet"),
                    )
                    .with_action("run /setup again in a few seconds");
                }
                started_server = true;
            }
            Some(_) => {
                return Step::new(
                    ID,
                    TITLE,
                    StepState::Action,
                    format!("Ollama is installed but {url} does not answer"),
                )
                .with_action("start Ollama (`ollama serve`), then run /setup");
            }
        }
    }
    let server = if started_server {
        "started Ollama"
    } else {
        "Ollama answers"
    };

    if probe.model_installed == Some(false) {
        if let Some(progress) = pull_for(&url, &model).filter(|p| p.error.is_some()) {
            if !apply {
                return Step::new(
                    ID,
                    TITLE,
                    StepState::Action,
                    format!(
                        "pulling {model} failed: {}",
                        progress.error.unwrap_or_default()
                    ),
                )
                .with_action(format!("run /setup to retry, or `ollama pull {model}`"));
            }
        }
        if !apply {
            return Step::new(
                ID,
                TITLE,
                StepState::Action,
                format!("{server}; embedding model {model} is not installed"),
            )
            .with_action(format!("run /setup to pull it, or `ollama pull {model}`"));
        }
        start_pull(Arc::clone(memory), &url, &model);
        let progress = pull_progress().unwrap_or_default();
        return Step::new(ID, TITLE, StepState::Running, pull_line(&progress));
    }

    if !probe.embed_ok {
        return Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!(
                "{server}, but a test embedding with {model} failed: {}",
                probe.embed_error.as_deref().unwrap_or("unknown error")
            ),
        )
        .with_action("check `embeddingModel` in ~/.davinci/agent/vector-memory.json");
    }
    if let Some(actual) = probe
        .embed_dimensions
        .filter(|actual| *actual != dimensions)
    {
        return Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!("{model} returns {actual} dimensions; memory expects {dimensions}"),
        )
        .with_action(format!(
            "set \"embeddingDimensions\": {actual} in ~/.davinci/agent/vector-memory.json"
        ));
    }

    let (records, lag) = {
        let memory = memory.lock().unwrap_or_else(|e| e.into_inner());
        let status = memory.status();
        (
            status["records"].as_u64().unwrap_or(0),
            status["projectionLag"].as_u64().unwrap_or(0),
        )
    };
    let finished_pull = take_finished_pull(&url, &model)
        .map(|_| format!("; pulled {model}"))
        .unwrap_or_default();
    if lag == 0 {
        let state = if started_server || !finished_pull.is_empty() {
            StepState::Done
        } else {
            StepState::Ready
        };
        return Step::new(
            ID,
            TITLE,
            state,
            format!("{server}{finished_pull}; {model} embeds; {records} record(s), all embedded"),
        );
    }
    if !apply {
        return Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!("{server}; {lag} of {records} record(s) have no vector"),
        )
        .with_action("run /setup to embed them");
    }
    let embedded = embed_backlog(memory);
    Step::new(
        ID,
        TITLE,
        StepState::Done,
        format!("{server}{finished_pull}; embedded {embedded} record(s) that had no vector"),
    )
}

// ---------------------------------------------------------------------------
// Git ignore
// ---------------------------------------------------------------------------

fn inside_git_work_tree(cwd: &Path) -> bool {
    cwd.ancestors().any(|dir| dir.join(".git").exists())
}

fn normalized_pattern(line: &str) -> String {
    line.trim()
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

/// Entries of [`IGNORED_STATE`] that `cwd/.gitignore` does not name. A line
/// that ignores the whole `.davinci` directory covers all of them.
pub fn missing_ignores(cwd: &Path) -> Vec<&'static str> {
    let existing = std::fs::read_to_string(cwd.join(".gitignore")).unwrap_or_default();
    let patterns = existing.lines().map(normalized_pattern).collect::<Vec<_>>();
    if patterns
        .iter()
        .any(|pattern| pattern == ".davinci" || pattern == ".davinci/*")
    {
        return Vec::new();
    }
    IGNORED_STATE
        .iter()
        .copied()
        .filter(|entry| !patterns.iter().any(|p| *p == normalized_pattern(entry)))
        .collect()
}

pub fn gitignore_step(cwd: &Path, apply: bool) -> Step {
    const ID: &str = "gitignore";
    const TITLE: &str = "git ignore";
    if !inside_git_work_tree(cwd) {
        return Step::new(ID, TITLE, StepState::Skipped, "not a Git work tree".into());
    }
    let missing = missing_ignores(cwd);
    if missing.is_empty() {
        return Step::new(
            ID,
            TITLE,
            StepState::Ready,
            "local memory and graph state are ignored".into(),
        );
    }
    let listed = missing.join(", ");
    if !apply {
        return Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!("{listed} could be committed by accident"),
        )
        .with_action("run /setup to add them to .gitignore");
    }
    let path = cwd.join(".gitignore");
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("\n# Davinci local state\n");
    for entry in &missing {
        content.push('/');
        content.push_str(entry);
        content.push('\n');
    }
    match std::fs::write(&path, content) {
        Ok(()) => Step::new(
            ID,
            TITLE,
            StepState::Done,
            format!("added {listed} to .gitignore"),
        ),
        Err(error) => Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!("could not write {}: {error}", path.display()),
        )
        .with_action(format!("add {listed} to .gitignore")),
    }
}

// ---------------------------------------------------------------------------
// Project instructions, trust, plugins, language servers
// ---------------------------------------------------------------------------

pub fn instructions_step(cwd: &Path) -> Step {
    const ID: &str = "instructions";
    const TITLE: &str = "project instructions";
    let found = cwd.ancestors().find_map(|dir| {
        INSTRUCTION_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    });
    match found {
        Some(path) => Step::new(
            ID,
            TITLE,
            StepState::Ready,
            crate::trust::display_path(&path).to_string(),
        ),
        None => Step::new(
            ID,
            TITLE,
            StepState::Action,
            "no AGENTS.md or CLAUDE.md for this workspace".into(),
        )
        .with_action("run /init to write AGENTS.md"),
    }
}

pub fn trust_step(agent_dir: &Path, cwd: &Path) -> Step {
    const ID: &str = "trust";
    const TITLE: &str = "project trust";
    if !crate::trust::has_trust_requiring_project_resources(cwd) {
        return Step::new(
            ID,
            TITLE,
            StepState::Skipped,
            "no project settings, hooks, skills, extensions or MCP servers".into(),
        );
    }
    let settings = crate::settings::load_merged_settings(agent_dir, cwd);
    let trusted = crate::trust::resolve_project_trusted(
        agent_dir,
        cwd,
        None,
        settings.default_project_trust.as_deref(),
        &settings.trusted_projects,
    );
    if trusted {
        return Step::new(
            ID,
            TITLE,
            StepState::Ready,
            "trusted; project resources load".into(),
        );
    }
    let stored = crate::trust::ProjectTrustStore::open(agent_dir).try_get(cwd);
    match stored {
        Ok(Some(false)) => Step::new(
            ID,
            TITLE,
            StepState::Skipped,
            "you chose not to trust this project; its .davinci/.pi resources stay off".into(),
        )
        .with_action("/setup trust to change that"),
        _ => Step::new(
            ID,
            TITLE,
            StepState::Action,
            "not trusted, so project settings, hooks, skills and MCP servers are ignored".into(),
        )
        .with_action("review them, then /setup trust"),
    }
}

pub fn trust_project(agent_dir: &Path, cwd: &Path) -> Step {
    match crate::trust::ProjectTrustStore::open(agent_dir).set(cwd, Some(true)) {
        Ok(()) => Step::new(
            "trust",
            "project trust",
            StepState::Done,
            format!("trusted {}", crate::trust::display_path(cwd)),
        )
        .with_action("restart davinci to load the project's resources"),
        Err(error) => Step::new(
            "trust",
            "project trust",
            StepState::Action,
            format!("could not record trust: {error}"),
        ),
    }
}

pub fn plugins_step(agent_dir: &Path) -> Step {
    const ID: &str = "plugins";
    const TITLE: &str = "plugin hooks";
    let active = crate::plugins::active(agent_dir);
    let pending = active
        .plugins
        .iter()
        .filter(|plugin| plugin.plugin.hooks_digest.is_some() && !plugin.hooks_approved())
        .map(|plugin| plugin.plugin.name.clone())
        .collect::<Vec<_>>();
    if pending.is_empty() {
        let detail = if active.plugins.is_empty() {
            "no plugins enabled".to_string()
        } else {
            format!("{} plugin(s), no hooks waiting", active.plugins.len())
        };
        return Step::new(ID, TITLE, StepState::Ready, detail);
    }
    let action = pending
        .iter()
        .map(|name| format!("/plugin approve {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    Step::new(
        ID,
        TITLE,
        StepState::Action,
        format!("hooks of {} wait for approval", pending.join(", ")),
    )
    .with_action(format!("read them with /plugin, then {action}"))
}

struct LanguageCheck {
    language: &'static str,
    markers: &'static [&'static str],
    servers: &'static [&'static str],
    install: &'static str,
}

const LANGUAGES: &[LanguageCheck] = &[
    LanguageCheck {
        language: "Rust",
        markers: &["Cargo.toml"],
        servers: &["rust-analyzer"],
        install: "rustup component add rust-analyzer",
    },
    LanguageCheck {
        language: "TypeScript/JavaScript",
        markers: &["tsconfig.json", "jsconfig.json", "package.json"],
        servers: &["typescript-language-server", "tsgo"],
        install: "npm install -D typescript typescript-language-server",
    },
    LanguageCheck {
        language: "Python",
        markers: &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
        ],
        servers: &["basedpyright-langserver", "pyright-langserver"],
        install: "pip install basedpyright",
    },
];

fn server_installed(cwd: &Path, servers: &[&str]) -> bool {
    servers.iter().any(|server| {
        let local = cwd.join("node_modules").join(".bin").join(server);
        local.is_file()
            || (cfg!(windows) && local.with_extension("cmd").is_file())
            || find_on_path(server).is_some()
    })
}

pub fn language_step(cwd: &Path, enabled: bool) -> Step {
    const ID: &str = "language-servers";
    const TITLE: &str = "language servers";
    if !enabled {
        return Step::new(
            ID,
            TITLE,
            StepState::Skipped,
            "language intelligence is off".into(),
        );
    }
    let detected = LANGUAGES
        .iter()
        .filter(|check| {
            check
                .markers
                .iter()
                .any(|marker| cwd.join(marker).is_file())
        })
        .collect::<Vec<_>>();
    if detected.is_empty() {
        return Step::new(
            ID,
            TITLE,
            StepState::Skipped,
            "no Rust, TypeScript/JavaScript or Python project here".into(),
        );
    }
    let missing = detected
        .iter()
        .filter(|check| !server_installed(cwd, check.servers))
        .collect::<Vec<_>>();
    let found = detected
        .iter()
        .filter(|check| server_installed(cwd, check.servers))
        .map(|check| check.language)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Step::new(
            ID,
            TITLE,
            StepState::Ready,
            format!("found for {}", found.join(", ")),
        );
    }
    let languages = missing
        .iter()
        .map(|check| check.language)
        .collect::<Vec<_>>()
        .join(", ");
    let installs = missing
        .iter()
        .map(|check| format!("`{}`", check.install))
        .collect::<Vec<_>>()
        .join(", ");
    // Davinci never downloads or installs language servers itself.
    Step::new(
        ID,
        TITLE,
        StepState::Action,
        format!("no language server found for {languages}; code navigation falls back to search"),
    )
    .with_action(format!("install with {installs}"))
}

#[cfg(test)]
mod tests {
    use super::super::vector_memory::{MemoryMessage, VectorMemoryConfig};
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn arguments_select_the_mode() {
        assert_eq!(parse_args("").unwrap(), Mode::Apply);
        assert_eq!(parse_args(" check ").unwrap(), Mode::Check);
        assert_eq!(parse_args("trust").unwrap(), Mode::Trust);
        assert!(parse_args("everything").is_err());
    }

    #[test]
    fn check_reports_missing_ignores_and_apply_adds_them_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/").unwrap();

        let checked = gitignore_step(dir.path(), false);
        assert_eq!(checked.state, StepState::Action);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
            "target/"
        );

        let applied = gitignore_step(dir.path(), true);
        assert_eq!(applied.state, StepState::Done, "{}", applied.detail);
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.starts_with("target/\n"));
        assert!(content.contains("/.davinci/vector-memory/"));
        assert!(content.contains("/.davinci/graph/"));

        assert_eq!(gitignore_step(dir.path(), true).state, StepState::Ready);
    }

    #[test]
    fn a_whole_davinci_ignore_covers_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "/.davinci/\n").unwrap();
        assert!(missing_ignores(dir.path()).is_empty());
    }

    #[test]
    fn outside_git_the_ignore_step_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(gitignore_step(dir.path(), true).state, StepState::Skipped);
        assert!(!dir.path().join(".gitignore").exists());
    }

    #[test]
    fn missing_instructions_point_at_init() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a");
        std::fs::create_dir(&nested).unwrap();
        // An AGENTS.md above the temp directory would answer for it, so only
        // a missing file is asserted on.
        let step = instructions_step(&nested);
        if step.state == StepState::Action {
            assert!(step.action.unwrap().contains("/init"));
        }
        std::fs::write(dir.path().join("AGENTS.md"), "# rules").unwrap();
        assert_eq!(instructions_step(&nested).state, StepState::Ready);
    }

    #[test]
    fn untrusted_project_resources_ask_and_trust_records_the_decision() {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join(".davinci")).unwrap();
        std::fs::write(project.path().join(".davinci").join("hooks.json"), "{}").unwrap();

        let step = trust_step(agent.path(), project.path());
        assert_eq!(step.state, StepState::Action, "{}", step.detail);
        assert_eq!(
            trust_project(agent.path(), project.path()).state,
            StepState::Done
        );
        assert_eq!(
            trust_step(agent.path(), project.path()).state,
            StepState::Ready
        );
    }

    #[test]
    fn a_project_without_language_markers_skips_language_servers() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(language_step(dir.path(), true).state, StepState::Skipped);
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        assert_ne!(language_step(dir.path(), true).state, StepState::Skipped);
        assert_eq!(language_step(dir.path(), false).state, StepState::Skipped);
    }

    #[test]
    fn project_local_node_servers_count_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let bin = dir.path().join("node_modules").join(".bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("typescript-language-server"), "").unwrap();
        assert_eq!(language_step(dir.path(), true).state, StepState::Ready);
    }

    #[test]
    fn only_loopback_servers_are_started() {
        assert!(is_local_url("http://127.0.0.1:11434"));
        assert!(is_local_url("http://localhost:11434/"));
        assert!(!is_local_url("http://gpu-box:11434"));
    }

    #[test]
    fn report_counts_steps_that_need_the_user() {
        let answer = report(
            Mode::Check,
            vec![
                Step::new("a", "a", StepState::Ready, "fine".into()),
                Step::new("b", "b", StepState::Action, "broken".into()).with_action("fix it"),
            ],
        );
        assert_eq!(answer["summary"], "1 step(s) need you");
        assert_eq!(answer["lines"][1], "! b · broken → fix it");
        assert_eq!(answer["steps"][1]["action"], "fix it");
    }

    fn memory_with(dir: &Path, url: String) -> Arc<Mutex<VectorMemory>> {
        let config = VectorMemoryConfig {
            ollama_url: url,
            embedding_dimensions: 8,
            request_timeout_seconds: 2,
            ..VectorMemoryConfig::default()
        };
        Arc::new(Mutex::new(VectorMemory::with_config(
            dir.to_path_buf(),
            config,
        )))
    }

    #[test]
    fn a_ready_server_embeds_records_that_have_no_vector() {
        let dir = tempfile::tempdir().unwrap();
        let dead = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        // Index while the server is down so the record has no vector.
        {
            let memory = memory_with(dir.path(), dead);
            memory
                .lock()
                .unwrap()
                .index_messages(&[MemoryMessage {
                    role: "user".into(),
                    content: "Deploy with tools/zephyr-deploy.ps1 on Fridays".into(),
                }])
                .unwrap();
        }
        let memory = memory_with(
            dir.path(),
            super::super::vector_memory::tests::fake_ollama(8),
        );
        let checked = memory_step(&memory, false);
        assert_eq!(checked.state, StepState::Action, "{}", checked.detail);
        let applied = memory_step(&memory, true);
        assert_eq!(applied.state, StepState::Done, "{}", applied.detail);
        assert!(applied.detail.contains("embedded 1"), "{}", applied.detail);
        assert_eq!(memory_step(&memory, false).state, StepState::Ready);
    }

    /// Ollama without the embedding model: `/api/tags` lists nothing and
    /// `/api/pull` streams progress ending in success.
    fn ollama_without_model() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let request = super::super::vector_memory::tests::read_http_request(&mut stream);
                let body = if request.starts_with("GET /api/tags") {
                    json!({"models": []}).to_string()
                } else if request.starts_with("POST /api/pull") {
                    [
                        json!({"status": "pulling manifest"}),
                        json!({"status": "downloading", "total": 100, "completed": 50}),
                        json!({"status": "success"}),
                    ]
                    .iter()
                    .map(|line| line.to_string() + "\n")
                    .collect()
                } else {
                    json!({"error": "model not found"}).to_string()
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
        format!("http://{address}")
    }

    #[test]
    fn a_missing_model_is_pulled_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory_with(dir.path(), ollama_without_model());
        let checked = memory_step(&memory, false);
        assert_eq!(checked.state, StepState::Action);
        assert!(checked
            .action
            .unwrap()
            .contains("ollama pull embeddinggemma"));

        let applied = memory_step(&memory, true);
        assert_eq!(applied.state, StepState::Running, "{}", applied.detail);
        let started = Instant::now();
        while !pull_progress().is_some_and(|p| p.finished) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "pull never finished"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let progress = pull_progress().unwrap();
        assert_eq!(progress.error, None);
        assert_eq!(progress.status, "done");
    }
}
