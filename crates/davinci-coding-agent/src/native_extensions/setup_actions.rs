//! The things `/setup` can do on the user's behalf, one at a time, each only
//! after the user said yes to exactly that.
//!
//! [`approve_action`] records a yes (in `service-consent.json`, plus the
//! approved `ollama` executable for a start). [`execute_approved_action`] does
//! the action and refuses when the yes is not on file. Nothing here runs from
//! a health check, and a project file can neither record nor widen a yes.

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
}

impl SetupAction {
    pub const ALL: [SetupAction; 3] = [
        SetupAction::Install,
        SetupAction::DownloadModel,
        SetupAction::StartService,
    ];

    /// The word used after `/setup approve`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::DownloadModel => "download-model",
            Self::StartService => "start-ollama",
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
            Self::Install => None,
            Self::DownloadModel => Some(ServiceAction::DownloadEmbeddingModel),
            Self::StartService => Some(ServiceAction::StartLocalOllama),
        }
    }
}

/// Record the user's yes for `action`. For a start this also records which
/// `ollama` they approved (path and version), so a different binary later is
/// refused. Returns a line to show.
pub fn approve_action(action: SetupAction, ctx: &SetupContext) -> Result<String, String> {
    let Some(consent) = action.consent() else {
        return Err(format!(
            "{} needs nothing to approve: install Ollama from https://ollama.com/download yourself",
            action.name()
        ));
    };
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

/// Do one approved action. Without the matching yes on file it returns an
/// `Action` step saying how to give it, and changes nothing.
pub fn execute_approved_action(
    action: SetupAction,
    memory: &Arc<Mutex<VectorMemory>>,
    ctx: &SetupContext,
) -> Step {
    const ID: &str = "memory";
    const TITLE: &str = "vector memory";
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
