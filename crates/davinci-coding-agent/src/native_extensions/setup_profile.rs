//! The host-owned setup profile and the effective-config resolver.
//!
//! The profile (`setup-profile.json` in the agent directory, never in a
//! project) records the choices `/setup` makes: the memory backend, whether
//! the local service starts by itself, whether safe repair runs, and an
//! explicit "switched off" for memory and the governor. It does not copy the
//! settings that already live in `vector-memory.json` and `token-governor.json`;
//! it references those files, and [`load_effective_setup`] layers the three
//! (profile, config file, environment) into what actually applies and says
//! where each answer came from.
//!
//! Rules this module keeps:
//! - A profile or config file that exists but cannot be parsed is never reset
//!   to defaults. It becomes an [`Issue`] and the feature stays off.
//! - An explicit "off" in the profile is sticky. The environment cannot turn
//!   it back on; the conflict is reported instead.
//! - Writes are atomic, and the previous file is kept as `.bak`.

use super::memory_backends::MemoryProjectionBackend;
use super::token_governor::TokenGovernorConfig;
use super::vector_memory::{ConfigFileState, VectorMemoryConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROFILE_FILE: &str = "setup-profile.json";
pub const MEMORY_CONFIG_FILE: &str = "vector-memory.json";
pub const GOVERNOR_CONFIG_FILE: &str = "token-governor.json";
pub const PROFILE_SCHEMA: u32 = 1;

/// Stable codes for configuration that needs the user.
pub const CONFIG_CORRUPT: &str = "CONFIG_CORRUPT";
pub const PROFILE_CORRUPT: &str = "PROFILE_CORRUPT";
pub const PROFILE_NEWER: &str = "PROFILE_NEWER";
pub const ENV_CONFLICT: &str = "ENV_CONFLICT";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryChoices {
    /// `Some(false)` is the user's explicit "off"; `None` means no choice yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Which projection backs recall. The JSONL store is authoritative either
    /// way; `Qdrant` adds a rebuildable remote copy and is chosen explicitly.
    #[serde(default)]
    pub backend: MemoryProjectionBackend,
    /// Start an approved local Ollama by itself when it is down.
    #[serde(default = "yes")]
    pub autostart: bool,
    /// Re-embed stale vectors by itself. Never deletes records.
    #[serde(default = "yes")]
    pub safe_repair: bool,
    /// Qdrant is an optional projection, never a dependency. These apply only
    /// when `backend` is `Qdrant`.
    #[serde(default)]
    pub qdrant_setup: QdrantChoices,
}

/// What the user chose for Qdrant. Every field is off or empty until `/setup`
/// records a choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QdrantChoices {
    /// Overrides `qdrantUrl` in `vector-memory.json` when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The user agreed that a non-loopback Qdrant may receive vectors.
    #[serde(default)]
    pub allow_remote: bool,
}

impl Default for MemoryChoices {
    fn default() -> Self {
        Self {
            enabled: None,
            backend: MemoryProjectionBackend::Local,
            autostart: true,
            safe_repair: true,
            qdrant_setup: QdrantChoices::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernorChoices {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// The Ollama executable the user approved, recorded so the supervisor starts
/// only that one (see `service_supervisor`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaIdentity {
    pub path: String,
    pub version: String,
}

/// The existing config files this profile sits on top of. They keep every
/// detail setting; the profile only points at them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRefs {
    pub memory: String,
    pub governor: String,
}

impl Default for ConfigRefs {
    fn default() -> Self {
        Self {
            memory: MEMORY_CONFIG_FILE.into(),
            governor: GOVERNOR_CONFIG_FILE.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupProfile {
    #[serde(default = "schema")]
    pub schema: u32,
    #[serde(default)]
    pub memory: MemoryChoices,
    #[serde(default)]
    pub governor: GovernorChoices,
    #[serde(default)]
    pub config_files: ConfigRefs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama: Option<OllamaIdentity>,
}

impl Default for SetupProfile {
    fn default() -> Self {
        Self {
            schema: PROFILE_SCHEMA,
            memory: MemoryChoices::default(),
            governor: GovernorChoices::default(),
            config_files: ConfigRefs::default(),
            ollama: None,
        }
    }
}

fn yes() -> bool {
    true
}
fn schema() -> u32 {
    PROFILE_SCHEMA
}

/// What reading the profile found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileState {
    Missing,
    Present,
    /// Exists but cannot be read or parsed. Not reset; not overwritten.
    Corrupt(String),
    /// Written by a newer DaVinci. Read-only here.
    Newer(u32),
}

impl SetupProfile {
    pub fn path(agent_dir: &Path) -> PathBuf {
        agent_dir.join(PROFILE_FILE)
    }

    /// The profile and what was found. On `Corrupt` / `Newer` the returned
    /// profile is the defaults, for display only: `save` refuses to write
    /// over those states.
    pub fn load(agent_dir: &Path) -> (Self, ProfileState) {
        let path = Self::path(agent_dir);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), ProfileState::Missing)
            }
            Err(error) => return (Self::default(), ProfileState::Corrupt(error.to_string())),
        };
        // Invalid UTF-8 is corrupt, never repaired: a lossy decode would make
        // the next save rewrite the user's bytes.
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return (
                Self::default(),
                ProfileState::Corrupt("not valid UTF-8".into()),
            );
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        match serde_json::from_str::<Self>(text) {
            Ok(profile) if profile.schema > PROFILE_SCHEMA => {
                let newer = profile.schema;
                (Self::default(), ProfileState::Newer(newer))
            }
            Ok(profile) => (profile, ProfileState::Present),
            Err(error) => (Self::default(), ProfileState::Corrupt(error.to_string())),
        }
    }

    /// Write the profile atomically, keeping the previous file as `.bak`.
    /// Refuses to replace a profile that is corrupt or from a newer version:
    /// that is the user's to fix, not ours to reset.
    pub fn save(&self, agent_dir: &Path) -> Result<(), String> {
        let (_, state) = Self::load(agent_dir);
        match state {
            ProfileState::Corrupt(error) => {
                return Err(format!(
                    "{PROFILE_CORRUPT}: {} cannot be read ({error}); fix or move it, then run /setup again.",
                    Self::path(agent_dir).display()
                ));
            }
            ProfileState::Newer(version) => {
                return Err(format!(
                    "{PROFILE_NEWER}: {} is schema {version}, newer than this DaVinci understands ({PROFILE_SCHEMA}).",
                    Self::path(agent_dir).display()
                ));
            }
            ProfileState::Missing | ProfileState::Present => {}
        }
        let path = Self::path(agent_dir);
        if path.exists() {
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
        }
        let mut body = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        body.push('\n');
        davinci_sys::fs::atomic_write(&path, body.as_bytes())
            .map_err(|error| format!("could not save {}: {error}", path.display()))
    }
}

/// A feature the user can switch on or off for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Memory,
    Governor,
}

impl Feature {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "memory" => Some(Self::Memory),
            "governor" => Some(Self::Governor),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Governor => "governor",
        }
    }
}

/// Record the user's on/off choice in the setup profile. The previous profile
/// is kept as `.bak`. A corrupt or newer profile is never replaced (the error
/// names it), so a failed switch changes nothing. Call this before touching
/// any running component: the intent is on disk first, so a crash in between
/// still leaves the feature off.
pub fn set_feature_enabled(
    agent_dir: &Path,
    feature: Feature,
    enabled: bool,
) -> Result<(), String> {
    let (mut profile, _) = SetupProfile::load(agent_dir);
    match feature {
        Feature::Memory => profile.memory.enabled = Some(enabled),
        Feature::Governor => profile.governor.enabled = Some(enabled),
    }
    profile.save(agent_dir)
}

/// Something the user has to look at, with a stable code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub code: &'static str,
    pub detail: String,
    pub remediation: String,
}

/// Where the answer to "is this feature on" came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EnabledSource {
    /// Nothing set it: the built-in default.
    Default,
    ConfigFile,
    /// The user's explicit choice in the setup profile.
    Profile,
    /// An environment variable (named).
    Environment(String),
    /// The config file is unreadable, so the feature stays off.
    FailClosed,
    /// The setup profile is unreadable or newer, so both features stay off:
    /// the user's choice in it cannot be known.
    ProfileUnreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvOverride {
    pub var: String,
    pub field: String,
}

#[derive(Debug, Clone)]
pub struct FeatureConfig<T> {
    /// The config with every layer applied. `config.enabled` equals `enabled`.
    pub config: T,
    pub file: ConfigFileState,
    pub path: PathBuf,
    pub enabled: bool,
    pub enabled_source: EnabledSource,
    pub overrides: Vec<EnvOverride>,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone)]
pub struct EffectiveSetupConfig {
    pub profile: SetupProfile,
    pub profile_state: ProfileState,
    pub memory: FeatureConfig<VectorMemoryConfig>,
    pub governor: FeatureConfig<TokenGovernorConfig>,
}

impl EffectiveSetupConfig {
    /// Why `feature` is off, in words the user can act on.
    pub fn off_reason(&self, feature: Feature) -> String {
        let (source, name) = match feature {
            Feature::Memory => (&self.memory.enabled_source, "vector-memory.json"),
            Feature::Governor => (&self.governor.enabled_source, "token-governor.json"),
        };
        match source {
            EnabledSource::Environment(var) => format!("{var} turns it off"),
            EnabledSource::FailClosed => format!("{name} cannot be read"),
            EnabledSource::ProfileUnreadable => {
                "setup-profile.json cannot be read or is from a newer DaVinci".into()
            }
            EnabledSource::ConfigFile => format!("{name} has enabled set to false"),
            EnabledSource::Profile => "the setup profile turns it off".into(),
            EnabledSource::Default => "it is off by default".into(),
        }
    }

    /// Why setup cannot be trusted to read the user's choices, if so.
    pub fn profile_problem(&self) -> Option<String> {
        match &self.profile_state {
            ProfileState::Corrupt(error) => {
                Some(format!("setup-profile.json cannot be read: {error}"))
            }
            ProfileState::Newer(version) => Some(format!(
                "setup-profile.json is schema {version}, newer than this DaVinci understands"
            )),
            _ => None,
        }
    }

    pub fn issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        match &self.profile_state {
            ProfileState::Corrupt(error) => issues.push(Issue {
                code: PROFILE_CORRUPT,
                detail: format!("{PROFILE_FILE} cannot be read: {error}"),
                remediation: format!("Fix or move {PROFILE_FILE}; it is not reset automatically."),
            }),
            ProfileState::Newer(version) => issues.push(Issue {
                code: PROFILE_NEWER,
                detail: format!("{PROFILE_FILE} is schema {version}"),
                remediation: "Update DaVinci; this version leaves the file alone.".into(),
            }),
            ProfileState::Missing | ProfileState::Present => {}
        }
        issues.extend(self.memory.issues.clone());
        issues.extend(self.governor.issues.clone());
        issues
    }
}

fn env_value(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    let davinci = name.replace("PI_", "DAVINCI_");
    env(&davinci)
        .or_else(|| env(name))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Layer profile, config files and environment into what applies.
///
/// `env` is the environment (use [`super::vector_memory::process_env`] for the
/// real one), so a test or `/setup check` can resolve without touching it.
/// The profile's Qdrant choice onto the config. The environment still wins:
/// `PI_MEMORY_BACKEND=local` is the escape hatch that turns the remote off
/// without editing a file. A profile that cannot be read changes nothing.
fn apply_profile_projection(
    profile: &SetupProfile,
    config: &mut VectorMemoryConfig,
    env: &dyn Fn(&str) -> Option<String>,
) {
    if profile.memory.backend != MemoryProjectionBackend::Qdrant {
        return;
    }
    if VectorMemoryConfig::env_sets_backend(env) {
        return;
    }
    config.projection.backend = MemoryProjectionBackend::Qdrant;
    if let Some(url) = &profile.memory.qdrant_setup.url {
        if env_value(env, "PI_MEMORY_QDRANT_URL").is_none() {
            config.qdrant_url = url.clone();
        }
    }
    if profile.memory.qdrant_setup.allow_remote {
        config.projection.allow_remote = true;
    }
}

pub fn load_effective_setup(
    agent_dir: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> EffectiveSetupConfig {
    let (profile, profile_state) = SetupProfile::load(agent_dir);

    // Memory.
    let memory_path = agent_dir.join(&profile.config_files.memory);
    let (mut memory_config, memory_file) = VectorMemoryConfig::load_file(&memory_path, env);
    let mut overrides = Vec::new();
    for (var, field) in VectorMemoryConfig::ENV_OVERRIDES {
        if env_value(env, var).is_some() {
            overrides.push(EnvOverride {
                var: (*var).into(),
                field: (*field).into(),
            });
        }
    }
    let env_enabled = env_value(env, "PI_MEMORY_ENABLED").and_then(|value| parse_bool(&value));
    let mut issues = Vec::new();
    if let ConfigFileState::Corrupt(error) = &memory_file {
        issues.push(Issue {
            code: CONFIG_CORRUPT,
            detail: format!("{} cannot be read: {error}", memory_path.display()),
            remediation: format!(
                "Fix or remove {MEMORY_CONFIG_FILE}. Memory stays off until then."
            ),
        });
    }
    let profile_bad = matches!(
        profile_state,
        ProfileState::Corrupt(_) | ProfileState::Newer(_)
    );
    let (enabled, source) = if profile_bad {
        (false, EnabledSource::ProfileUnreadable)
    } else {
        resolve_enabled(
            &memory_file,
            memory_config.enabled,
            profile.memory.enabled,
            env_enabled.map(|value| ("PI_MEMORY_ENABLED".to_string(), value)),
            &mut issues,
            "memory",
        )
    };
    memory_config.enabled = enabled;
    apply_profile_projection(&profile, &mut memory_config, env);
    let memory = FeatureConfig {
        config: memory_config,
        file: memory_file,
        path: memory_path,
        enabled,
        enabled_source: source,
        overrides,
        issues,
    };

    // Governor.
    let governor_path = agent_dir.join(&profile.config_files.governor);
    let (mut governor_config, governor_file) = TokenGovernorConfig::load_file(&governor_path, env);
    let mut overrides = Vec::new();
    let mut env_enabled = None;
    for var in TokenGovernorConfig::ENABLED_ENV {
        if let Some(value) = env_value(env, var) {
            overrides.push(EnvOverride {
                var: (*var).into(),
                field: "enabled".into(),
            });
            if env_enabled.is_none() {
                env_enabled = parse_bool(&value).map(|parsed| ((*var).to_string(), parsed));
            }
        }
    }
    let mut issues = Vec::new();
    if let ConfigFileState::Corrupt(error) = &governor_file {
        issues.push(Issue {
            code: CONFIG_CORRUPT,
            detail: format!("{} cannot be read: {error}", governor_path.display()),
            remediation: format!(
                "Fix or remove {GOVERNOR_CONFIG_FILE}. The governor stays off until then."
            ),
        });
    }
    let (enabled, source) = if profile_bad {
        (false, EnabledSource::ProfileUnreadable)
    } else {
        resolve_enabled(
            &governor_file,
            governor_config.enabled,
            profile.governor.enabled,
            env_enabled,
            &mut issues,
            "governor",
        )
    };
    governor_config.enabled = enabled;
    let governor = FeatureConfig {
        config: governor_config,
        file: governor_file,
        path: governor_path,
        enabled,
        enabled_source: source,
        overrides,
        issues,
    };

    EffectiveSetupConfig {
        profile,
        profile_state,
        memory,
        governor,
    }
}

/// On/off from the layers. `file_enabled` already includes the environment
/// (the loaders apply it), so the environment's own answer is passed again to
/// name it as the source and to detect a conflict with the profile.
fn resolve_enabled(
    file: &ConfigFileState,
    file_enabled: bool,
    profile: Option<bool>,
    env: Option<(String, bool)>,
    issues: &mut Vec<Issue>,
    feature: &str,
) -> (bool, EnabledSource) {
    if profile == Some(false) {
        if let Some((var, true)) = &env {
            issues.push(Issue {
                code: ENV_CONFLICT,
                detail: format!("{var} asks for {feature} on, but setup has it switched off"),
                remediation: format!(
                    "Run /setup enable {feature} to turn it on, or unset {var}. The switch-off wins."
                ),
            });
        }
        return (false, EnabledSource::Profile);
    }
    if let Some((var, _)) = env {
        return (file_enabled, EnabledSource::Environment(var));
    }
    match file {
        ConfigFileState::Corrupt(_) => (false, EnabledSource::FailClosed),
        ConfigFileState::Present => (file_enabled, EnabledSource::ConfigFile),
        ConfigFileState::Missing => (file_enabled, EnabledSource::Default),
    }
}
