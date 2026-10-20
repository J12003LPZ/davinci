//! Host-owned consent for starting or installing local services.
//!
//! Trusting a project folder lets DaVinci load that folder's configuration. It
//! never starts a server, downloads a model or installs software. Those need a
//! separate, durable consent that lives in the user's own agent directory and
//! that no project file can write or widen.
//!
//! Phase 1 ships the store and the check. `/setup`'s spawn of `ollama serve`
//! and its model pull are not routed through [`ServiceAuthorization::check`]
//! yet: that is the seam phase 2's service supervisor plugs into (see
//! `setup::memory_step`, where `spawn_ollama_serve` and `start_pull` run).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::settings::with_settings_lock;

/// File name inside the agent directory.
pub const CONSENT_FILE: &str = "service-consent.json";

/// What a service step may do on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    /// Run `ollama serve` for the configured local server.
    StartLocalOllama,
    /// Pull the embedding model into Ollama.
    DownloadEmbeddingModel,
    /// Start a Qdrant container DaVinci created and owns.
    StartOwnedQdrantContainer,
    /// Install third-party software.
    InstallSoftware,
}

impl ServiceAction {
    pub const ALL: [ServiceAction; 4] = [
        ServiceAction::StartLocalOllama,
        ServiceAction::DownloadEmbeddingModel,
        ServiceAction::StartOwnedQdrantContainer,
        ServiceAction::InstallSoftware,
    ];

    /// The key this consent has in `service-consent.json`.
    pub fn key(self) -> &'static str {
        match self {
            Self::StartLocalOllama => "allow_start_local_ollama",
            Self::DownloadEmbeddingModel => "allow_download_embedding_model",
            Self::StartOwnedQdrantContainer => "allow_start_owned_qdrant_container",
            Self::InstallSoftware => "allow_install_software",
        }
    }
}

/// The four consents. Everything defaults to refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServiceConsent {
    pub allow_start_local_ollama: bool,
    pub allow_download_embedding_model: bool,
    pub allow_start_owned_qdrant_container: bool,
    pub allow_install_software: bool,
}

impl ServiceConsent {
    pub fn allows(&self, action: ServiceAction) -> bool {
        match action {
            ServiceAction::StartLocalOllama => self.allow_start_local_ollama,
            ServiceAction::DownloadEmbeddingModel => self.allow_download_embedding_model,
            ServiceAction::StartOwnedQdrantContainer => self.allow_start_owned_qdrant_container,
            ServiceAction::InstallSoftware => self.allow_install_software,
        }
    }

    fn set(&mut self, action: ServiceAction, allowed: bool) {
        match action {
            ServiceAction::StartLocalOllama => self.allow_start_local_ollama = allowed,
            ServiceAction::DownloadEmbeddingModel => self.allow_download_embedding_model = allowed,
            ServiceAction::StartOwnedQdrantContainer => {
                self.allow_start_owned_qdrant_container = allowed
            }
            ServiceAction::InstallSoftware => self.allow_install_software = allowed,
        }
    }
}

/// The consent file in one agent directory. The agent directory is the user's
/// global data, never a project path.
#[derive(Debug, Clone)]
pub struct ServiceConsentStore {
    path: PathBuf,
}

impl ServiceConsentStore {
    pub fn open(agent_dir: &Path) -> Self {
        Self {
            path: agent_dir.join(CONSENT_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A missing file is "nothing consented". A file that cannot be read or
    /// does not match the schema is an error, never a default: the caller must
    /// treat it as refusal.
    pub fn load(&self) -> Result<ServiceConsent, String> {
        read_consent(&self.path)
    }

    /// Record one consent, or withdraw it. Written atomically under a lock;
    /// a damaged file is reported and left alone rather than overwritten.
    pub fn set(&self, action: ServiceAction, allowed: bool) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        with_settings_lock(&self.path, || {
            let mut consent = read_consent(&self.path)?;
            consent.set(action, allowed);
            let mut body = serde_json::Map::new();
            body.insert("schemaVersion".into(), json!(1));
            for each in ServiceAction::ALL {
                body.insert(each.key().into(), json!(consent.allows(each)));
            }
            let text = format!(
                "{}\n",
                serde_json::to_string_pretty(&Value::Object(body)).map_err(|e| e.to_string())?
            );
            davinci_sys::fs::atomic_write(&self.path, text.as_bytes()).map_err(|e| e.to_string())
        })
    }
}

fn read_consent(path: &Path) -> Result<ServiceConsent, String> {
    if !path.exists() {
        return Ok(ServiceConsent::default());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("Failed to read service consent {}: {err}", path.display()))?;
    let invalid = |why: String| format!("Invalid service consent {}: {why}", path.display());
    let parsed: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
        .map_err(|err| invalid(err.to_string()))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| invalid("expected an object".into()))?;
    let mut consent = ServiceConsent::default();
    for (key, value) in object {
        if key == "schemaVersion" {
            if value.as_u64() != Some(1) {
                return Err(invalid("unsupported schemaVersion".into()));
            }
            continue;
        }
        let Some(action) = ServiceAction::ALL.iter().find(|a| a.key() == key) else {
            return Err(invalid(format!("unknown key {key:?}")));
        };
        let Some(allowed) = value.as_bool() else {
            return Err(invalid(format!("{key} must be true or false")));
        };
        consent.set(*action, allowed);
    }
    Ok(consent)
}

/// The one question a service step asks before it acts.
pub struct ServiceAuthorization;

impl ServiceAuthorization {
    /// `Ok` only when the user's own consent file allows `action`. A missing
    /// consent, a withdrawn one and an unreadable file all refuse; nothing a
    /// project contains, and no folder-trust decision, is consulted.
    pub fn check(agent_dir: &Path, action: ServiceAction) -> Result<(), String> {
        let consent = ServiceConsentStore::open(agent_dir).load()?;
        if consent.allows(action) {
            Ok(())
        } else {
            Err(format!(
                "{} is not allowed: set {} in {}",
                action.key(),
                action.key(),
                agent_dir.join(CONSENT_FILE).display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_consented_by_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ServiceConsentStore::open(dir.path()).load().unwrap(),
            ServiceConsent::default()
        );
        for action in ServiceAction::ALL {
            assert!(ServiceAuthorization::check(dir.path(), action).is_err());
        }
    }

    #[test]
    fn consents_are_independent_durable_and_revocable() {
        let dir = tempfile::tempdir().unwrap();
        let store = ServiceConsentStore::open(dir.path());
        store.set(ServiceAction::StartLocalOllama, true).unwrap();
        assert!(ServiceAuthorization::check(dir.path(), ServiceAction::StartLocalOllama).is_ok());
        for other in [
            ServiceAction::DownloadEmbeddingModel,
            ServiceAction::StartOwnedQdrantContainer,
            ServiceAction::InstallSoftware,
        ] {
            assert!(ServiceAuthorization::check(dir.path(), other).is_err());
        }
        // A new handle (a relaunch) sees it; revoking removes it.
        assert!(
            ServiceConsentStore::open(dir.path())
                .load()
                .unwrap()
                .allow_start_local_ollama
        );
        store.set(ServiceAction::StartLocalOllama, false).unwrap();
        assert!(ServiceAuthorization::check(dir.path(), ServiceAction::StartLocalOllama).is_err());
    }

    #[test]
    fn a_damaged_file_refuses_everything_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONSENT_FILE);
        for bad in [
            "{ nope",
            "[]",
            r#"{"allow_install_software":"yes"}"#,
            r#"{"allow_everything":true}"#,
            r#"{"schemaVersion":9}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            for action in ServiceAction::ALL {
                let error = ServiceAuthorization::check(dir.path(), action).unwrap_err();
                assert!(error.contains(CONSENT_FILE), "{error}");
            }
            assert!(ServiceConsentStore::open(dir.path())
                .set(ServiceAction::InstallSoftware, true)
                .is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        }
    }
}
