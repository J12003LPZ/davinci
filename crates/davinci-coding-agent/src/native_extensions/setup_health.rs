//! Component health: what is actually working, from probes, not from flags.
//!
//! A [`ComponentHealth`] is built from a live check of the Ollama API (model
//! list plus one test embedding) and a read of the local record store. The
//! "dense search is on cooldown" flag in `VectorMemory` is never an input, so a
//! stopped Ollama cannot show as `Ready`.
//!
//! Probes take a config snapshot and never hold the memory lock: a hung
//! server costs the probe its timeout, not every turn that needs memory. They
//! never write anything and never pull a model.

use super::vector_memory::{LocalStoreHealth, VectorMemoryConfig, LOCAL_STORE_UNREADABLE};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub use super::setup_profile::CONFIG_CORRUPT;

/// Error codes, shared with the spec.
pub const INSTALL_REQUIRED: &str = "INSTALL_REQUIRED";
pub const MODEL_MISSING: &str = "MODEL_MISSING";
pub const PROVIDER_OFFLINE: &str = "PROVIDER_OFFLINE";
pub const EMBEDDING_MISMATCH: &str = "EMBEDDING_MISMATCH";
/// Docker is installed but its engine does not answer (or answers too slowly).
pub const DOCKER_UNAVAILABLE: &str = "DOCKER_UNAVAILABLE";
/// Qdrant is chosen, nothing answers, and no Qdrant container exists.
pub const QDRANT_MISSING: &str = "QDRANT_MISSING";
/// A Qdrant container exists that DaVinci did not create and will not touch.
pub const QDRANT_NOT_OWNED: &str = "QDRANT_NOT_OWNED";
pub use super::memory_backends::QDRANT_OFFLINE;
/// Records exist that dense search cannot find yet (repairable).
pub const PROJECTION_STALE: &str = "PROJECTION_STALE";

/// Upper bound for any single probe request, whatever the config says.
const PROBE_TIMEOUT_MAX_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthState {
    /// Switched off on purpose. Nothing is probed, started or reported.
    Disabled,
    /// No setup choice has been made and nothing is configured.
    Unconfigured,
    /// A check is running.
    Checking,
    /// A service or model is being started or downloaded.
    Starting,
    Ready,
    /// Works, but only in part (for example keyword search only).
    Degraded,
    /// Needs the user; `remediation` says what to do.
    ActionRequired,
    /// Was tried, did not come up, and automatic recovery gave up for now.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHealth {
    pub component: &'static str,
    pub state: HealthState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub last_checked: u64,
}

impl ComponentHealth {
    pub fn new(component: &'static str, state: HealthState, detail: impl Into<String>) -> Self {
        Self {
            component,
            state,
            code: None,
            detail: detail.into(),
            remediation: None,
            last_checked: davinci_session::now_ms(),
        }
    }

    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    pub fn is_ready(&self) -> bool {
        self.state == HealthState::Ready
    }
}

/// What to ask the Ollama API. A snapshot of the config, so no lock is held.
#[derive(Debug, Clone)]
pub struct ProbeTarget {
    pub url: String,
    pub model: String,
    pub dimensions: usize,
    pub timeout: Duration,
}

impl ProbeTarget {
    pub fn from_config(config: &VectorMemoryConfig) -> Self {
        let seconds = config
            .request_timeout_seconds
            .clamp(1, PROBE_TIMEOUT_MAX_SECS)
            .min(config.embed_timeout_seconds.max(1));
        Self {
            url: config.ollama_url.trim_end_matches('/').to_string(),
            model: config.embedding_model.clone(),
            dimensions: config.embedding_dimensions,
            timeout: Duration::from_secs(seconds),
        }
    }
}

/// The result of one test embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedOutcome {
    /// Not attempted (server down or model missing).
    NotAttempted,
    Ok {
        dimensions: usize,
        millis: u64,
    },
    /// The model answers with a different vector size than configured.
    Mismatch {
        actual: usize,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaProbe {
    pub reachable: bool,
    pub server_error: Option<String>,
    pub installed_models: Vec<String>,
    /// `None` when the model list could not be read.
    pub model_installed: Option<bool>,
    pub embed: EmbedOutcome,
}

pub fn model_matches(configured: &str, installed: &str) -> bool {
    installed == configured
        || (!configured.contains(':')
            && installed.split(':').next() == Some(configured)
            && installed.ends_with(":latest"))
}

/// Ask Ollama which models it has, then embed one short text with the
/// configured model. Read-only: no pull, no start, no write. Each request is
/// bounded by `target.timeout`.
pub fn probe_ollama(target: &ProbeTarget) -> OllamaProbe {
    let agent = crate::http_client::agent_with_timeout(target.timeout);
    let tags = agent
        .get(&format!("{}/api/tags", target.url))
        .call()
        .map_err(|error| error.to_string())
        .and_then(|mut response| {
            crate::http_client::read_json::<Value>(&mut response).map_err(|error| error.to_string())
        });
    let tags = match tags {
        Ok(tags) => tags,
        Err(error) => {
            return OllamaProbe {
                reachable: false,
                server_error: Some(error),
                installed_models: Vec::new(),
                model_installed: None,
                embed: EmbedOutcome::NotAttempted,
            }
        }
    };
    let installed_models: Vec<String> = tags["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model["name"].as_str().map(str::to_string))
        .collect();
    let model_installed = installed_models
        .iter()
        .any(|name| model_matches(&target.model, name));
    let embed = if model_installed {
        probe_embedding(&agent, target)
    } else {
        EmbedOutcome::NotAttempted
    };
    OllamaProbe {
        reachable: true,
        server_error: None,
        installed_models,
        model_installed: Some(model_installed),
        embed,
    }
}

fn probe_embedding(agent: &ureq::Agent, target: &ProbeTarget) -> EmbedOutcome {
    let started = Instant::now();
    let response = crate::http_client::send_json(
        agent.post(&format!("{}/api/embed", target.url)),
        &json!({"model": target.model, "input": ["DaVinci setup probe"]}),
    )
    .map_err(|error| error.to_string())
    .and_then(|mut response| {
        crate::http_client::read_json::<Value>(&mut response).map_err(|error| error.to_string())
    });
    let value = match response {
        Ok(value) => value,
        Err(error) => return EmbedOutcome::Failed(error),
    };
    let Some(vector) = value["embeddings"][0].as_array() else {
        return EmbedOutcome::Failed("the embedding response has no vector".into());
    };
    if vector.iter().any(|number| number.as_f64().is_none()) {
        return EmbedOutcome::Failed("the embedding vector contains a non-number".into());
    }
    if vector.len() != target.dimensions {
        return EmbedOutcome::Mismatch {
            actual: vector.len(),
        };
    }
    EmbedOutcome::Ok {
        dimensions: vector.len(),
        millis: started.elapsed().as_millis() as u64,
    }
}

/// The local record file, as its own component.
pub fn probe_memory_store(store: &LocalStoreHealth) -> ComponentHealth {
    match store {
        LocalStoreHealth::Ok => ComponentHealth::new("store", HealthState::Ready, "records readable"),
        LocalStoreHealth::Unreadable(error) => {
            ComponentHealth::new("store", HealthState::ActionRequired, error.clone())
                .with_code(LOCAL_STORE_UNREADABLE)
                .with_remediation(
                    "The record file was left untouched. Fix its permissions or encoding, or move it aside, then run /memory-reindex.",
                )
        }
    }
}

/// Facts the health of memory is derived from. Every field is something
/// observed; none is a cooldown flag.
#[derive(Debug, Clone)]
pub struct MemoryFacts<'a> {
    pub enabled: bool,
    /// The config file is unreadable (the feature is held off).
    pub config_error: Option<&'a str>,
    pub store: &'a LocalStoreHealth,
    /// `None` while the check is still running.
    pub probe: Option<&'a OllamaProbe>,
    /// `Some(false)` when no `ollama` executable is known on this machine.
    pub ollama_installed: Option<bool>,
    /// A start or a model download is in flight.
    pub starting: bool,
    pub model: &'a str,
    pub configured_dimensions: usize,
    pub records: usize,
    /// Records without a current vector.
    pub lag: usize,
}

/// Decide the memory component's health. `Ready` needs: readable store, the
/// server answering, the model installed, a test embedding of the configured
/// size, and no stale records.
pub fn assess_memory(facts: &MemoryFacts) -> ComponentHealth {
    const C: &str = "memory";
    if let Some(error) = facts.config_error {
        return ComponentHealth::new(C, HealthState::ActionRequired, error)
            .with_code(CONFIG_CORRUPT)
            .with_remediation("Fix or remove vector-memory.json. Memory stays off until then.");
    }
    if !facts.enabled {
        return ComponentHealth::new(C, HealthState::Disabled, "switched off");
    }
    if facts.store.is_unreadable() {
        let mut health = probe_memory_store(facts.store);
        health.component = C;
        return health;
    }
    let Some(probe) = facts.probe else {
        return ComponentHealth::new(C, HealthState::Checking, "checking the embedding service");
    };
    if !probe.reachable {
        let error = probe.server_error.as_deref().unwrap_or("no answer");
        if facts.starting {
            return ComponentHealth::new(
                C,
                HealthState::Starting,
                "starting the embedding service",
            );
        }
        if facts.ollama_installed == Some(false) {
            return ComponentHealth::new(
                C,
                HealthState::ActionRequired,
                format!("Ollama is not installed; search uses keywords only ({error})"),
            )
            .with_code(INSTALL_REQUIRED)
            .with_remediation("Install Ollama from https://ollama.com/download, then run /setup.");
        }
        return ComponentHealth::new(
            C,
            HealthState::Degraded,
            format!("Ollama does not answer ({error}); search uses keywords only"),
        )
        .with_code(PROVIDER_OFFLINE)
        .with_remediation("Start Ollama, or run /setup.");
    }
    if probe.model_installed == Some(false) {
        if facts.starting {
            return ComponentHealth::new(
                C,
                HealthState::Starting,
                format!("downloading {}", facts.model),
            );
        }
        return ComponentHealth::new(
            C,
            HealthState::ActionRequired,
            format!("embedding model {} is not installed", facts.model),
        )
        .with_code(MODEL_MISSING)
        .with_remediation(format!(
            "Run /setup and approve the download, or `ollama pull {}`.",
            facts.model
        ));
    }
    match &probe.embed {
        EmbedOutcome::Mismatch { actual } => {
            return ComponentHealth::new(
                C,
                HealthState::ActionRequired,
                format!(
                    "{} returns {actual} dimensions; memory is configured for {}",
                    facts.model, facts.configured_dimensions
                ),
            )
            .with_code(EMBEDDING_MISMATCH)
            .with_remediation(format!(
                "Set \"embeddingDimensions\": {actual} in vector-memory.json, then run /setup repair to re-embed."
            ));
        }
        EmbedOutcome::Failed(error) => {
            return ComponentHealth::new(
                C,
                HealthState::Degraded,
                format!("a test embedding with {} failed: {error}", facts.model),
            )
            .with_code(PROVIDER_OFFLINE)
            .with_remediation("Check the model name in vector-memory.json, or run /setup.");
        }
        EmbedOutcome::NotAttempted => {
            return ComponentHealth::new(
                C,
                HealthState::Checking,
                "the test embedding has not run",
            );
        }
        EmbedOutcome::Ok { .. } => {}
    }
    if facts.lag > 0 {
        return ComponentHealth::new(
            C,
            HealthState::Degraded,
            format!(
                "{} of {} record(s) have no current vector",
                facts.lag, facts.records
            ),
        )
        .with_code(PROJECTION_STALE)
        .with_remediation("Run /setup repair to embed them.");
    }
    ComponentHealth::new(
        C,
        HealthState::Ready,
        format!(
            "{} embeds; {} record(s), all embedded",
            facts.model, facts.records
        ),
    )
}
