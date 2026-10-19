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
use super::service_authority::{ServiceAction, ServiceAuthorization};
use super::service_supervisor::find_in_paths;
#[cfg(test)]
use super::service_supervisor::is_local_url;
use super::service_supervisor::{system_supervisor, ManagedServiceSupervisor};
use super::setup_health::{ComponentHealth, HealthState};
use super::setup_profile::load_effective_setup;
use super::token_governor::{GovernorHealth, GovernorState};
use super::vector_memory::process_env;
use super::vector_memory::VectorMemory;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Local state Davinci writes under the workspace that should never be
/// committed: learned durable memory records and graph run journals.
pub const IGNORED_STATE: &[&str] = &[".davinci/vector-memory/", ".davinci/graph/"];
/// Records embedded per batch after a pull; the memory lock is released
/// between batches so a turn is never held behind the whole backlog.
pub(crate) const REINDEX_BATCHES: usize = 40;
/// Records re-embedded per locked batch.
pub(crate) const REINDEX_BATCH_SIZE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Report every area and apply the local, reversible steps.
    Apply,
    /// Report only. Changes nothing and starts nothing.
    Check,
    /// `Check`, answered as JSON.
    CheckJson,
    /// Re-embed records whose vector is stale. Never deletes a record.
    Repair,
    /// Record that this project is trusted.
    Trust,
    /// Record the user's yes to one action and do just that action.
    Approve(super::setup_actions::SetupAction),
}

pub fn parse_args(args: &str) -> Result<Mode, String> {
    let args = args.trim();
    if let Some(rest) = args.strip_prefix("approve") {
        let word = rest.trim();
        return super::setup_actions::SetupAction::parse(word)
            .map(Mode::Approve)
            .ok_or_else(|| {
                format!(
                    "unknown action `{word}` · use /setup approve start-ollama or /setup approve download-model"
                )
            });
    }
    match args {
        "" | "apply" => Ok(Mode::Apply),
        "check" | "--check" | "status" => Ok(Mode::Check),
        "check --json" | "status --json" | "--json" => Ok(Mode::CheckJson),
        "repair" => Ok(Mode::Repair),
        "trust" => Ok(Mode::Trust),
        other => Err(format!(
            "unknown /setup argument `{other}` · use /setup, /setup check [--json], /setup repair, /setup approve <action> or /setup trust"
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
    /// Stable error code (`MODEL_MISSING`, `LOCAL_STORE_UNREADABLE`, ...) when
    /// the step needs the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

impl Step {
    pub(crate) fn new(
        id: &'static str,
        title: &'static str,
        state: StepState,
        detail: String,
    ) -> Self {
        Self {
            id,
            title,
            state,
            detail,
            action: None,
            code: None,
        }
    }

    pub(crate) fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub(crate) fn with_action(mut self, action: impl Into<String>) -> Self {
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
    report_full(mode, steps, Vec::new(), Vec::new())
}

/// [`report`] with the component health and the configuration issues the
/// steps were derived from. `/setup check --json` prints this object.
pub fn report_full(
    mode: Mode,
    steps: Vec<Step>,
    health: Vec<ComponentHealth>,
    issues: Vec<super::setup_profile::Issue>,
) -> Value {
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
    let lines: Vec<String> = if mode == Mode::CheckJson {
        let body = json!({
            "summary": summary,
            "steps": steps,
            "health": health,
            "issues": issues,
        });
        serde_json::to_string_pretty(&body)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    } else {
        steps.iter().map(Step::line).collect()
    };
    json!({
        "mode": match mode {
            Mode::Apply => "apply",
            Mode::Check | Mode::CheckJson => "check",
            Mode::Repair => "repair",
            Mode::Trust => "trust",
            Mode::Approve(_) => "approve",
        },
        "summary": summary,
        "lines": lines,
        "steps": steps,
        "health": health,
        "issues": issues,
    })
}

/// The governor's health in the same shape as the memory component's.
pub fn governor_health(health: &GovernorHealth, effective_enabled: bool) -> ComponentHealth {
    if !effective_enabled || health.state == GovernorState::Disabled {
        return ComponentHealth::new("governor", HealthState::Disabled, "token governor is off");
    }
    match health.state {
        GovernorState::Degraded => ComponentHealth::new(
            "governor",
            HealthState::Degraded,
            format!(
                "cannot keep full tool outputs at {}: {}; {} output(s) stayed uncompressed",
                health.store.display(),
                health.last_error.as_deref().unwrap_or("store not writable"),
                health.store_failures
            ),
        )
        .with_code("GOVERNOR_STORE_UNWRITABLE")
        .with_remediation("make the store folder writable or set storeDir in token-governor.json"),
        _ => ComponentHealth::new("governor", HealthState::Ready, "token governor is on"),
    }
}

/// The memory component's health, from live probes and the store, without
/// holding the memory lock across the network and without changing anything.
pub fn memory_health(
    memory: &Arc<Mutex<VectorMemory>>,
    effective: &super::setup_profile::EffectiveSetupConfig,
    ctx: &SetupContext,
) -> ComponentHealth {
    use super::setup_health::{assess_memory, probe_ollama, MemoryFacts, ProbeTarget};
    use super::vector_memory::ConfigFileState;
    let (config, store, records, lag) = {
        let memory = memory.lock().unwrap_or_else(|e| e.into_inner());
        let status = memory.status();
        (
            memory.config.clone(),
            memory.store_health(),
            status["records"].as_u64().unwrap_or(0) as usize,
            status["projectionLag"].as_u64().unwrap_or(0) as usize,
        )
    };
    let config_error = match &effective.memory.file {
        ConfigFileState::Corrupt(error) => Some(error.clone()),
        _ => None,
    };
    let enabled = effective.memory.enabled && config.enabled;
    // A feature that is off, or whose config is unreadable, is not probed.
    let probe = (enabled && config_error.is_none() && !store.is_unreadable())
        .then(|| probe_ollama(&ProbeTarget::from_config(&config)));
    let installed = probe
        .as_ref()
        .filter(|probe| !probe.reachable)
        .map(|_| ctx.supervisor.launcher().find().is_some());
    let starting = pull_progress().is_some_and(|progress| !progress.finished);
    let facts = MemoryFacts {
        enabled,
        config_error: config_error.as_deref(),
        store: &store,
        probe: probe.as_ref(),
        ollama_installed: installed,
        starting,
        model: &config.embedding_model,
        configured_dimensions: config.embedding_dimensions,
        records,
        lag,
    };
    assess_memory(&facts)
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

/// Set by [`cancel_pull`]; the running pull checks it between progress lines.
static PULL_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the running model download to stop. The partial download stays in
/// Ollama's own store; nothing of ours is deleted. Returns false when no
/// download is running.
pub fn cancel_pull() -> bool {
    let running = pull_progress().is_some_and(|progress| !progress.finished);
    if running {
        PULL_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    running
}

/// Forget a failed pull so the next explicit `/setup` may try again. A failed
/// or interrupted download is reported to the user first; it is never started
/// again silently.
fn take_failed_pull(url: &str, model: &str) -> Option<PullProgress> {
    let mut guard = pulls().lock().unwrap_or_else(|e| e.into_inner());
    let matches = guard
        .as_ref()
        .is_some_and(|p| p.url == url && p.model == model && p.finished && p.error.is_some());
    if matches {
        guard.take()
    } else {
        None
    }
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

fn reserve_pull(slot: &mut Option<PullProgress>, url: &str, model: &str) -> bool {
    if slot.as_ref().is_some_and(|existing| !existing.finished) {
        return false;
    }
    PULL_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    *slot = Some(PullProgress {
        model: model.to_string(),
        url: url.to_string(),
        status: "starting".into(),
        ..PullProgress::default()
    });
    true
}

/// Start `ollama pull` through the HTTP API on a background thread, then
/// embed the records that have no vector. Returns false when a pull for the
/// server or model is already running. Its progress slot belongs to that worker
/// until it finishes, including its embedding pass.
pub fn start_pull(memory: Arc<Mutex<VectorMemory>>, url: &str, model: &str) -> bool {
    {
        let mut guard = pulls().lock().unwrap_or_else(|e| e.into_inner());
        if !reserve_pull(&mut guard, url, model) {
            return false;
        }
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
        if PULL_CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("cancelled; the partial download stays in Ollama's store".into());
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
        // The stream ended without a verdict: the outcome is unknown. The
        // next check asks Ollama what it has instead of assuming either way.
        Err("the download ended without a result; Ollama may still hold a partial copy".into())
    }
}

/// Embed every record without a vector, one bounded batch per lock.
pub fn embed_backlog(memory: &Arc<Mutex<VectorMemory>>) -> usize {
    let mut total = 0;
    for _ in 0..REINDEX_BATCHES {
        let receipt = {
            let mut memory = memory.lock().unwrap_or_else(|e| e.into_inner());
            memory.clear_dense_offline();
            memory.repair_projection(REINDEX_BATCH_SIZE)
        };
        total += receipt.reembedded;
        if receipt.reembedded == 0 || receipt.blocked.is_some() || receipt.error.is_some() {
            break;
        }
    }
    total
}

/// Probe the embedding service from a config snapshot: the memory lock is
/// released before the network is touched, so a slow or hung server never
/// holds up a turn that needs memory.
fn probe_unlocked(memory: &Arc<Mutex<VectorMemory>>) -> memory_page::Probe {
    let config = memory
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .config
        .clone();
    memory_page::probe_config(&config)
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    find_in_paths(program, &std::env::var_os("PATH")?)
}

/// What a setup step may use: the host's agent directory (where consent and
/// the profile live) and the service supervisor.
pub struct SetupContext<'a> {
    pub agent_dir: &'a Path,
    pub supervisor: &'a ManagedServiceSupervisor,
}

impl<'a> SetupContext<'a> {
    /// The real machine: this process's supervisor.
    pub fn system(agent_dir: &'a Path) -> Self {
        Self {
            agent_dir,
            supervisor: system_supervisor(),
        }
    }
}

pub(crate) fn step_from_health(
    id: &'static str,
    title: &'static str,
    health: &ComponentHealth,
) -> Step {
    let state = match health.state {
        HealthState::Ready => StepState::Ready,
        HealthState::Disabled => StepState::Skipped,
        HealthState::Checking | HealthState::Starting => StepState::Running,
        _ => StepState::Action,
    };
    let mut step = Step::new(id, title, state, health.detail.clone());
    if let Some(code) = health.code {
        step = step.with_code(code);
    }
    if let Some(remediation) = &health.remediation {
        step = step.with_action(remediation.clone());
    }
    step
}

pub fn memory_step(memory: &Arc<Mutex<VectorMemory>>, apply: bool, ctx: &SetupContext) -> Step {
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
    if let super::vector_memory::LocalStoreHealth::Unreadable(error) = memory
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .store_health()
    {
        return Step::new(ID, TITLE, StepState::Action, error)
            .with_code(super::vector_memory::LOCAL_STORE_UNREADABLE)
            .with_action(
                "the record file was left untouched; fix its permissions or encoding, or move it aside, then run /memory-reindex",
            );
    }

    let mut probe = probe_unlocked(memory);
    let mut started_server = false;
    if !probe.reachable {
        if !apply {
            // Read-only: say what is wrong without starting anything.
            let installed = ctx.supervisor.launcher().find();
            return match installed {
                None => Step::new(
                    ID,
                    TITLE,
                    StepState::Action,
                    format!("no Ollama at {url}; search uses keywords only"),
                )
                .with_code(super::setup_health::INSTALL_REQUIRED)
                .with_action(format!(
                    "install Ollama from https://ollama.com/download, then run /setup (it pulls {model})"
                )),
                Some(_) => Step::new(
                    ID,
                    TITLE,
                    StepState::Action,
                    format!("Ollama is installed but {url} does not answer"),
                )
                .with_code(super::setup_health::PROVIDER_OFFLINE)
                .with_action("run /setup to start it (it asks once), or `ollama serve`"),
            };
        }
        // The supervisor starts it only with the user's consent and only the
        // Ollama the user approved; see `service_supervisor`.
        let effective = load_effective_setup(ctx.agent_dir, &process_env);
        let health = ctx.supervisor.reconcile(ctx.agent_dir, &effective);
        if health.state != HealthState::Ready {
            return step_from_health(ID, TITLE, &health);
        }
        probe = probe_unlocked(memory);
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
    let server = if started_server {
        "started Ollama"
    } else {
        "Ollama answers"
    };

    if probe.model_installed == Some(false) {
        if let Some(progress) = pull_for(&url, &model).filter(|p| p.error.is_some()) {
            // A failed or interrupted download is reported, once, and never
            // restarted by itself: the next explicit /setup may try again.
            if apply {
                take_failed_pull(&url, &model);
            }
            return Step::new(
                ID,
                TITLE,
                StepState::Action,
                format!(
                    "downloading {model} did not finish: {}",
                    progress.error.unwrap_or_default()
                ),
            )
            .with_code(super::setup_health::MODEL_MISSING)
            .with_action(format!("run /setup to try again, or `ollama pull {model}`"));
        }
        if !apply {
            return Step::new(
                ID,
                TITLE,
                StepState::Action,
                format!("{server}; embedding model {model} is not installed"),
            )
            .with_code(super::setup_health::MODEL_MISSING)
            .with_action(format!(
                "run /setup to download it (it asks first), or `ollama pull {model}`"
            ));
        }
        // Downloading is the user's call, made once and recorded.
        if let Err(reason) =
            ServiceAuthorization::check(ctx.agent_dir, ServiceAction::DownloadEmbeddingModel)
        {
            return Step::new(
                ID,
                TITLE,
                StepState::Action,
                format!(
                    "{server}; embedding model {model} is not installed and downloading it needs your approval ({reason})"
                ),
            )
            .with_code(super::service_supervisor::CONSENT_REQUIRED)
            .with_action(format!(
                "approve the download of {model} with /setup approve download-model (Ollama reports the size once it starts), then run /setup"
            ));
        }
        start_pull(Arc::clone(memory), &url, &model);
        let progress = pull_progress().unwrap_or_default();
        return Step::new(ID, TITLE, StepState::Running, pull_line(&progress));
    }

    // A model that answers with the wrong vector size is not "a failed
    // embedding": say what to change.
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
        .with_code(super::setup_health::EMBEDDING_MISMATCH)
        .with_action(format!(
            "set \"embeddingDimensions\": {actual} in ~/.davinci/agent/vector-memory.json, then run /setup repair"
        ));
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

/// Entries of [`IGNORED_STATE`] that `cwd/.gitignore` does not name. A line
/// that ignores the whole `.davinci` directory covers all of them.
pub fn missing_ignores(cwd: &Path) -> Vec<&'static str> {
    let existing = std::fs::read_to_string(cwd.join(".gitignore")).unwrap_or_default();
    missing_ignore_patterns(cwd, &existing)
}

fn missing_ignore_patterns(cwd: &Path, existing: &str) -> Vec<&'static str> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(cwd);
    for line in existing.lines() {
        if builder.add_line(None, line).is_err() {
            return IGNORED_STATE.to_vec();
        }
    }
    let Ok(patterns) = builder.build() else {
        return IGNORED_STATE.to_vec();
    };
    IGNORED_STATE
        .iter()
        .copied()
        .filter(|entry| {
            !patterns
                // `is_dir` carries the directory marker. A trailing slash in
                // the candidate itself prevents anchored matches on Unix.
                .matched_path_or_any_parents(cwd.join(entry.trim_end_matches('/')), true)
                .is_ignore()
        })
        .collect()
}

pub fn gitignore_step(cwd: &Path, apply: bool) -> Step {
    const ID: &str = "gitignore";
    const TITLE: &str = "git ignore";
    if !inside_git_work_tree(cwd) {
        return Step::new(ID, TITLE, StepState::Skipped, "not a Git work tree".into());
    }
    let path = cwd.join(".gitignore");
    let mut content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Step::new(
                ID,
                TITLE,
                StepState::Action,
                format!("could not read {}: {error}", path.display()),
            )
            .with_action("check .gitignore permissions and UTF-8 encoding");
        }
    };
    let missing = missing_ignore_patterns(cwd, &content);
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
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("\n# Davinci local state\n");
    for entry in &missing {
        content.push('/');
        content.push_str(entry);
        content.push('\n');
    }
    match davinci_sys::fs::atomic_write(&path, content.as_bytes()) {
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
    // The files the agent loads, so the report and the session agree: inside
    // a checkout, from its root down to `cwd`; outside one, none.
    let found = davinci_agent::load_context_files(cwd, true)
        .into_iter()
        .next()
        .map(|file| file.path);
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
            "no AGENTS.md for this workspace".into(),
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
        // The store may hold a decision we cannot read. Say so with the
        // reader's own message (it names the file) instead of the generic
        // "not trusted" text, and do not offer a grant that cannot be saved.
        Err(error) => Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!("the trust store cannot be read, so this project stays untrusted: {error}"),
        )
        .with_action("fix or remove that file, then /setup trust"),
        Ok(_) => Step::new(
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
        .with_action("/reload loads its skills and prompts; restart davinci for MCP servers, hooks and extensions"),
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
        .map(|name| format!("/plugins approve {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    Step::new(
        ID,
        TITLE,
        StepState::Action,
        format!("hooks of {} wait for approval", pending.join(", ")),
    )
    .with_action(format!("read them with /plugins, then {action}"))
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
    use std::time::Instant;

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
    fn another_model_cannot_replace_an_active_pull() {
        let mut slot = None;
        assert!(reserve_pull(&mut slot, "http://first:11434", "first-model"));
        assert!(!reserve_pull(
            &mut slot,
            "http://second:11434",
            "second-model"
        ));
        assert_eq!(slot.as_ref().unwrap().model, "first-model");
        assert_eq!(slot.as_ref().unwrap().url, "http://first:11434");
        slot.as_mut().unwrap().finished = true;
        assert!(reserve_pull(
            &mut slot,
            "http://second:11434",
            "second-model"
        ));
    }

    #[test]
    fn negated_gitignore_patterns_require_a_later_ignore() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".gitignore"),
            ".davinci/*\n!/.davinci/vector-memory/\n",
        )
        .unwrap();
        assert!(missing_ignores(dir.path()).contains(&".davinci/vector-memory/"));
        assert_eq!(gitignore_step(dir.path(), true).state, StepState::Done);
        assert!(missing_ignores(dir.path()).is_empty());
    }

    #[test]
    fn apply_preserves_an_unreadable_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let path = dir.path().join(".gitignore");
        let original = b"target/\n\xffprivate-key\n";
        std::fs::write(&path, original).unwrap();
        let step = gitignore_step(dir.path(), true);
        assert_eq!(step.state, StepState::Action);
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert!(step.detail.contains("could not read"), "{}", step.detail);
    }

    #[test]
    fn local_ollama_url_uses_the_parsed_host() {
        for url in [
            "http://[::1]:11434",
            "http://LOCALHOST:11434",
            "http://127.0.0.1:11434",
        ] {
            assert!(is_local_url(url), "{url}");
        }
        // A synthetic port in userinfo must not disguise a remote host as local.
        let misleading_userinfo = format!("http://localhost:{}@remote.example", 11434);
        for url in [
            misleading_userinfo.as_str(),
            "http://localhost.example",
            "file://localhost/x",
            "localhost:11434",
        ] {
            assert!(!is_local_url(url), "{url}");
        }
    }

    #[test]
    fn a_whole_davinci_ignore_covers_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "/.davinci/\n").unwrap();
        assert!(missing_ignores(dir.path()).is_empty());
    }

    #[test]
    fn anchored_state_directories_are_recognized_before_they_exist() {
        let dir = tempfile::tempdir().unwrap();
        let content = "/.davinci/vector-memory/\n/.davinci/graph/\n";
        assert!(!dir.path().join(".davinci").exists());
        assert!(missing_ignore_patterns(dir.path(), content).is_empty());
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
        let repo = dir.path().join("repo");
        let nested = repo.join("a");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir(&nested).unwrap();
        // An AGENTS.md beside the checkout is not the checkout's.
        std::fs::write(dir.path().join("AGENTS.md"), "@CLAUDE.md").unwrap();
        let step = instructions_step(&nested);
        assert_eq!(step.state, StepState::Action);
        assert!(step.action.unwrap().contains("/init"));
        std::fs::write(repo.join("AGENTS.md"), "# rules").unwrap();
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
    fn an_unreadable_trust_store_is_reported_not_swallowed() {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join(".davinci")).unwrap();
        std::fs::write(project.path().join(".davinci").join("hooks.json"), "{}").unwrap();
        std::fs::write(agent.path().join("trust.json"), "{ not json").unwrap();

        let step = trust_step(agent.path(), project.path());
        assert_eq!(step.state, StepState::Action, "{}", step.detail);
        assert!(step.detail.contains("trust.json"), "{}", step.detail);
        assert!(step.detail.contains("cannot be read"), "{}", step.detail);
        // The damaged file is left for the user to inspect.
        assert_eq!(
            std::fs::read_to_string(agent.path().join("trust.json")).unwrap(),
            "{ not json"
        );
        // Granting trust cannot paper over it either.
        let granted = trust_project(agent.path(), project.path());
        assert_eq!(granted.state, StepState::Action);
        assert!(granted.detail.contains("trust.json"), "{}", granted.detail);
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

    /// A machine with no `ollama` at all, and a throwaway agent directory.
    struct NoOllama;
    impl super::super::service_supervisor::OllamaLauncher for NoOllama {
        fn find(&self) -> Option<PathBuf> {
            None
        }
        fn version(&self, _: &Path) -> Option<String> {
            None
        }
        fn spawn(
            &self,
            _: &Path,
            _: &Path,
        ) -> Result<Box<dyn super::super::service_supervisor::OwnedProcess>, String> {
            Err("no process may be started in this test".into())
        }
    }

    struct TestCtx {
        agent: tempfile::TempDir,
        supervisor: ManagedServiceSupervisor,
    }

    impl TestCtx {
        fn new() -> Self {
            Self {
                agent: tempfile::tempdir().unwrap(),
                supervisor: ManagedServiceSupervisor::new(Box::new(NoOllama)),
            }
        }
        fn ctx(&self) -> SetupContext<'_> {
            SetupContext {
                agent_dir: self.agent.path(),
                supervisor: &self.supervisor,
            }
        }
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
    fn a_model_with_another_vector_size_names_the_dimension_fix() {
        let tc = TestCtx::new();
        let dir = tempfile::tempdir().unwrap();
        // The server answers with 12 dimensions; memory expects 8.
        let memory = memory_with(
            dir.path(),
            super::super::vector_memory::tests::fake_ollama(12),
        );
        let step = memory_step(&memory, false, &tc.ctx());
        assert_eq!(step.state, StepState::Action, "{}", step.detail);
        assert_eq!(step.code, Some("EMBEDDING_MISMATCH"));
        assert!(step.detail.contains("12 dimensions"), "{}", step.detail);
        assert!(
            step.action
                .as_deref()
                .unwrap_or("")
                .contains("\"embeddingDimensions\": 12"),
            "{:?}",
            step.action
        );
    }

    #[test]
    fn an_unreadable_record_file_is_an_action_not_a_green_step() {
        let tc = TestCtx::new();
        let dir = tempfile::tempdir().unwrap();
        let memory = memory_with(
            dir.path(),
            super::super::vector_memory::tests::fake_ollama(8),
        );
        let path = memory.lock().unwrap().local_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"\xff\xfe").unwrap();
        // Rebuild so the constructor sees the bad file.
        let rebuilt = {
            let config = memory.lock().unwrap().config.clone();
            Arc::new(Mutex::new(VectorMemory::with_config(
                dir.path().to_path_buf(),
                config,
            )))
        };
        let step = memory_step(&rebuilt, true, &tc.ctx());
        assert_eq!(step.state, StepState::Action);
        assert_eq!(step.code, Some("LOCAL_STORE_UNREADABLE"));
        assert_eq!(std::fs::read(&path).unwrap(), b"\xff\xfe");
    }

    #[test]
    fn a_ready_server_embeds_records_that_have_no_vector() {
        let tc = TestCtx::new();
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
        let checked = memory_step(&memory, false, &tc.ctx());
        assert_eq!(checked.state, StepState::Action, "{}", checked.detail);
        let applied = memory_step(&memory, true, &tc.ctx());
        assert_eq!(applied.state, StepState::Done, "{}", applied.detail);
        assert!(applied.detail.contains("embedded 1"), "{}", applied.detail);
        assert_eq!(
            memory_step(&memory, false, &tc.ctx()).state,
            StepState::Ready
        );
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

    /// An Ollama stand-in that records every request line it is sent.
    fn recording_ollama(requests: Arc<Mutex<Vec<String>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let request = super::super::vector_memory::tests::read_http_request(&mut stream);
                requests
                    .lock()
                    .unwrap()
                    .push(request.lines().next().unwrap_or_default().to_string());
                let body = json!({"models": []}).to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: {}
Connection: close

{}",
                    body.len(),
                    body
                );
            }
        });
        format!("http://{address}")
    }

    #[test]
    fn folder_trust_does_not_grant_service_start() {
        let tc = TestCtx::new();
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join(".davinci")).unwrap();
        std::fs::write(project.path().join(".davinci").join("hooks.json"), "{}").unwrap();

        let requests = Arc::new(Mutex::new(Vec::new()));
        let url = recording_ollama(Arc::clone(&requests));
        let memory_dir = tempfile::tempdir().unwrap();
        let memory = memory_with(memory_dir.path(), url);

        // Granting trust, and reading the trust state back, touch no service.
        assert_eq!(
            trust_project(agent.path(), project.path()).state,
            StepState::Done
        );
        assert_eq!(
            trust_step(agent.path(), project.path()).state,
            StepState::Ready
        );
        assert!(
            requests.lock().unwrap().is_empty(),
            "trusting a folder reached the embedding service: {:?}",
            requests.lock().unwrap()
        );

        // With the folder trusted, a check still only observes: it probes the
        // server (model list, one test embedding) and neither pulls a model
        // nor starts a server.
        let checked = memory_step(&memory, false, &tc.ctx());
        assert_eq!(checked.state, StepState::Action, "{}", checked.detail);
        let seen = requests.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.contains("/api/pull")),
            "{seen:?}"
        );
    }

    #[test]
    fn a_relative_path_entry_cannot_supply_the_executable_setup_starts() {
        // `Cargo.toml` sits in the test's working directory (the crate root),
        // which is what `.` and an empty entry mean to a launched process.
        let relative =
            std::env::join_paths([std::path::Path::new("."), std::path::Path::new("")]).unwrap();
        assert_eq!(find_in_paths("Cargo.toml", &relative), None);
        let absolute = std::env::join_paths([
            std::path::PathBuf::from("."),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        ])
        .unwrap();
        assert_eq!(
            find_in_paths("Cargo.toml", &absolute),
            Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        );
    }

    #[test]
    fn a_missing_model_is_pulled_in_the_background() {
        let tc = TestCtx::new();
        let dir = tempfile::tempdir().unwrap();
        let memory = memory_with(dir.path(), ollama_without_model());
        let checked = memory_step(&memory, false, &tc.ctx());
        assert_eq!(checked.state, StepState::Action);
        assert!(checked
            .action
            .unwrap()
            .contains("ollama pull embeddinggemma"));

        // Without the user's yes the download does not start.
        let refused = memory_step(&memory, true, &tc.ctx());
        assert_eq!(refused.state, StepState::Action, "{}", refused.detail);
        assert_eq!(refused.code, Some("CONSENT_REQUIRED"));
        super::super::service_authority::ServiceConsentStore::open(tc.agent.path())
            .set(ServiceAction::DownloadEmbeddingModel, true)
            .unwrap();

        let applied = memory_step(&memory, true, &tc.ctx());
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
