//! The things `/setup` can do on the user's behalf, one at a time, each only
//! after the user said yes to exactly that.
//!
//! [`approve_action`] records a yes (in `service-consent.json`, plus the
//! approved `ollama` executable for a start). [`execute_approved_action`] does
//! the action and refuses when the yes is not on file. Nothing here runs from
//! a health check, and a project file can neither record nor widen a yes.

use super::memory_backends::ProjectionRepairReceipt;
use super::service_authority::{ServiceAction, ServiceAuthorization, ServiceConsentStore};
use super::service_supervisor::{approve_installed_ollama, CONSENT_REQUIRED};
use super::setup::{start_pull, step_from_health, SetupContext, Step, StepState};
use super::setup_health::INSTALL_REQUIRED;
use super::setup_profile::load_effective_setup;
use super::vector_memory::{process_env, VectorMemory};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupAction {
    /// Ollama is not on this machine. DaVinci does not install software.
    Install,
    /// Pull the configured embedding model into Ollama.
    DownloadModel,
    /// Start the approved local Ollama.
    StartService,
    /// Re-embed records whose vector is stale. Derived data only; no record
    /// is ever removed, so it needs no approval beyond asking for it.
    RepairProjection,
    /// Start the one Qdrant container the user recorded as DaVinci's.
    StartQdrant,
}

impl SetupAction {
    pub const ALL: [SetupAction; 5] = [
        SetupAction::Install,
        SetupAction::DownloadModel,
        SetupAction::StartService,
        SetupAction::RepairProjection,
        SetupAction::StartQdrant,
    ];

    /// The word used after `/setup approve`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::DownloadModel => "download-model",
            Self::StartService => "start-ollama",
            Self::RepairProjection => "repair",
            Self::StartQdrant => super::docker_discovery::START_QDRANT,
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.name() == word.trim())
    }

    /// The consent this action runs under, if it needs one.
    pub fn consent(self) -> Option<ServiceAction> {
        match self {
            Self::Install | Self::RepairProjection => None,
            Self::DownloadModel => Some(ServiceAction::DownloadEmbeddingModel),
            Self::StartService => Some(ServiceAction::StartLocalOllama),
            Self::StartQdrant => Some(ServiceAction::StartOwnedQdrantContainer),
        }
    }
}

/// Record the user's yes for `action`. For a start this also records which
/// `ollama` they approved (path and version), so a different binary later is
/// refused. Returns a line to show.
pub fn approve_action(action: SetupAction, ctx: &SetupContext) -> Result<String, String> {
    // A feature that is off gets no standing "yes" recorded for it.
    if let Some(off) = memory_off_reason(ctx.agent_dir) {
        return Err(off);
    }
    let Some(consent) = action.consent() else {
        return Err(format!(
            "{} needs nothing to approve: install Ollama from https://ollama.com/download yourself",
            action.name()
        ));
    };
    if action == SetupAction::StartQdrant {
        let effective = load_effective_setup(ctx.agent_dir, &process_env);
        if effective.memory.config.projection.backend
            != super::memory_backends::MemoryProjectionBackend::Qdrant
        {
            return Err(
                "the Qdrant projection is off; /setup qdrant enable turns it on first".into(),
            );
        }
        ServiceConsentStore::open(ctx.agent_dir).set(consent, true)?;
        return Ok(
            "approved starting DaVinci's own Qdrant container (only the one recorded with /setup qdrant own, or labelled by DaVinci)"
                .into(),
        );
    }
    if action == SetupAction::StartService {
        let identity = approve_installed_ollama(ctx.agent_dir, ctx.supervisor.launcher())?;
        ServiceConsentStore::open(ctx.agent_dir).set(consent, true)?;
        return Ok(format!(
            "approved starting {} ({})",
            identity.path, identity.version
        ));
    }
    ServiceConsentStore::open(ctx.agent_dir).set(consent, true)?;
    Ok(format!("approved {}", action.name()))
}

/// Why memory is off, in the user's terms; `None` when it is on.
fn memory_off_reason(agent_dir: &std::path::Path) -> Option<String> {
    let effective = load_effective_setup(agent_dir, &process_env);
    (!effective.memory.enabled).then(|| {
        format!(
            "memory is off: {}; /setup enable memory turns it on",
            effective.off_reason(super::setup_profile::Feature::Memory)
        )
    })
}

/// Do one approved action. Without the matching yes on file it returns an
/// `Action` step saying how to give it, and changes nothing.
pub fn execute_approved_action(
    action: SetupAction,
    memory: &Arc<Mutex<VectorMemory>>,
    ctx: &SetupContext,
) -> Step {
    const ID: &str = "memory";
    const TITLE: &str = "vector memory";
    let live_off = !memory
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .config
        .enabled;
    if let Some(off) = memory_off_reason(ctx.agent_dir)
        .or_else(|| live_off.then(|| "memory is off; /setup enable memory turns it on".to_string()))
    {
        return Step::new(ID, TITLE, StepState::Skipped, off);
    }
    if let Some(consent) = action.consent() {
        if let Err(reason) = ServiceAuthorization::check(ctx.agent_dir, consent) {
            return Step::new(ID, TITLE, StepState::Action, reason)
                .with_code(CONSENT_REQUIRED)
                .with_action(format!("approve it with /setup approve {}", action.name()));
        }
    }
    match action {
        SetupAction::Install => Step::new(
            ID,
            TITLE,
            StepState::Action,
            "Ollama is not installed; DaVinci does not install software".into(),
        )
        .with_code(INSTALL_REQUIRED)
        .with_action("install Ollama from https://ollama.com/download, then run /setup"),
        SetupAction::StartService => {
            let effective = load_effective_setup(ctx.agent_dir, &process_env);
            let health = ctx.supervisor.reconcile(ctx.agent_dir, &effective);
            step_from_health(ID, TITLE, &health)
        }
        SetupAction::RepairProjection => repair_step(memory),
        SetupAction::StartQdrant => {
            let effective = load_effective_setup(ctx.agent_dir, &process_env);
            let health = super::qdrant_supervisor::reconcile_owned_qdrant(
                ctx.agent_dir,
                &effective,
                ctx.docker,
                ctx.containers,
                super::qdrant_supervisor::DEADLINE,
            );
            step_from_health("qdrant", "Qdrant projection", &health)
        }
        SetupAction::DownloadModel => {
            let (url, model) = {
                let memory = memory.lock().unwrap_or_else(|e| e.into_inner());
                (
                    memory.config.ollama_url.clone(),
                    memory.config.embedding_model.clone(),
                )
            };
            if start_pull(Arc::clone(memory), &url, &model) {
                Step::new(
                    ID,
                    TITLE,
                    StepState::Running,
                    format!("downloading {model}; run /setup to follow it"),
                )
            } else {
                Step::new(
                    ID,
                    TITLE,
                    StepState::Running,
                    "a download is already running".into(),
                )
            }
        }
    }
}

/// Run repair batches (the memory lock is released between them) until the
/// records are all embedded, a batch embeds nothing, or something blocks it.
pub fn repair_step(memory: &Arc<Mutex<VectorMemory>>) -> Step {
    use super::setup::{REINDEX_BATCHES, REINDEX_BATCH_SIZE};
    const ID: &str = "memory";
    const TITLE: &str = "vector memory";
    let mut total = 0;
    let mut last = None;
    for _ in 0..REINDEX_BATCHES {
        memory
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear_dense_offline();
        let receipt = VectorMemory::repair_shared(memory, REINDEX_BATCH_SIZE);
        total += receipt.reembedded;
        let stop = receipt.reembedded == 0 || receipt.blocked.is_some() || receipt.error.is_some();
        last = Some(receipt);
        if stop {
            break;
        }
    }
    let receipt = last.unwrap_or_default();
    let remote = remote_catch_up(memory);
    if let Some(code) = receipt.blocked {
        let action = match code {
            "EMBEDDING_MISMATCH" => {
                "set embeddingDimensions to the size the model returns, then run /setup repair"
            }
            "LOCAL_STORE_UNREADABLE" => "fix or move the record file, then run /setup repair",
            _ => "start Ollama (/setup), then run /setup repair",
        };
        return Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!(
                "repair did not start: {} (no record was changed)",
                receipt.error.unwrap_or_default()
            ),
        )
        .with_code(match code {
            "EMBEDDING_MISMATCH" => super::setup_health::EMBEDDING_MISMATCH,
            "LOCAL_STORE_UNREADABLE" => super::vector_memory::LOCAL_STORE_UNREADABLE,
            _ => super::setup_health::PROVIDER_OFFLINE,
        })
        .with_action(action);
    }
    if let Some(error) = receipt.error {
        return Step::new(
            ID,
            TITLE,
            StepState::Action,
            format!("repair stopped after {total} record(s): {error}; no record was changed"),
        )
        .with_action("run /setup repair again");
    }
    if receipt.remaining > 0 {
        return Step::new(
            ID,
            TITLE,
            StepState::Running,
            format!("re-embedded {total} record(s); {} left", receipt.remaining),
        )
        .with_action("run /setup repair again");
    }
    Step::new(
        ID,
        TITLE,
        StepState::Done,
        format!(
            "re-embedded {total} record(s); {} record(s) kept{}",
            receipt.records_after,
            remote_note(remote.as_ref())
        ),
    )
}

/// Bring the optional remote copy up to date, a bounded batch at a time. The
/// network is used on a snapshot, never under the memory lock. `None` in
/// local mode: nothing is contacted.
fn remote_catch_up(memory: &Arc<Mutex<VectorMemory>>) -> Option<ProjectionRepairReceipt> {
    use super::setup::REINDEX_BATCHES;
    let snapshot = memory.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !snapshot.remote_projection_enabled() {
        return None;
    }
    let batch = snapshot.config.projection.batch_size;
    let mut total = ProjectionRepairReceipt::default();
    for _ in 0..REINDEX_BATCHES {
        let pass = snapshot.reconcile_remote(batch);
        total.indexed += pass.indexed;
        total.removed += pass.removed;
        total.lag = pass.lag;
        total.skipped = pass.skipped;
        total.failures += pass.failures;
        total.code = pass.code;
        total.error = pass.error;
        if pass.failures > 0 || pass.lag == 0 || pass.indexed + pass.removed == 0 {
            break;
        }
    }
    Some(total)
}

fn remote_note(remote: Option<&ProjectionRepairReceipt>) -> String {
    match remote {
        None => String::new(),
        Some(receipt) if receipt.is_complete() => {
            format!("; Qdrant copy in sync ({} sent)", receipt.indexed)
        }
        Some(receipt) => format!(
            "; {} Qdrant copy not finished: {} record(s) still to mirror, local search is in use (nothing was lost)",
            receipt.code.unwrap_or("QDRANT_PENDING"),
            receipt.lag
        ),
    }
}
