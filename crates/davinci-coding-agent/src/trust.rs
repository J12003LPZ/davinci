//! Project trust store matching `vendor/pi/packages/coding-agent/src/core/trust-manager.ts`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::settings::with_settings_lock;

/// Project resources under either config directory that can run code or change
/// the agent's prompt or permissions. Keep in sync with every project loader.
const TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES: &[&str] = &[
    "settings.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
    "hooks.json",
    "mcp.json",
    "agents",
    "git",
    "npm",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustStoreEntry {
    pub path: String,
    pub decision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustUpdate {
    pub path: String,
    pub decision: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrustOption {
    pub label: String,
    pub trusted: bool,
    pub updates: Vec<ProjectTrustUpdate>,
    pub saved_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectTrustStore {
    trust_path: PathBuf,
}

impl ProjectTrustStore {
    pub fn open(agent_dir: &Path) -> Self {
        Self {
            trust_path: agent_dir.join("trust.json"),
        }
    }

    pub fn get(&self, cwd: &Path) -> Option<bool> {
        self.get_entry(cwd).map(|entry| entry.decision)
    }

    pub fn get_entry(&self, cwd: &Path) -> Option<ProjectTrustStoreEntry> {
        with_settings_lock(&self.trust_path, || {
            let data = read_trust_file(&self.trust_path)?;
            Ok(find_nearest_trust_entry(&data, cwd))
        })
        .ok()
        .flatten()
    }

    /// `Err` means a decision may exist but cannot be read. Callers that
    /// grant trust must treat that as "not trusted".
    pub fn try_get(&self, cwd: &Path) -> Result<Option<bool>, String> {
        if let Some(parent) = self.trust_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Failed to create trust store directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        with_settings_lock(&self.trust_path, || {
            let data = read_trust_file(&self.trust_path)?;
            Ok(find_nearest_trust_entry(&data, cwd).map(|entry| entry.decision))
        })
    }

    pub fn set(&self, cwd: &Path, decision: Option<bool>) -> Result<(), String> {
        self.set_many(&[ProjectTrustUpdate {
            path: canonicalize_trust_path(cwd),
            decision,
        }])
    }

    pub fn set_many(&self, decisions: &[ProjectTrustUpdate]) -> Result<(), String> {
        if let Some(parent) = self.trust_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        with_settings_lock(&self.trust_path, || {
            let mut data = read_trust_file(&self.trust_path)?;
            for update in decisions {
                let key = canonicalize_trust_path(Path::new(&update.path));
                match update.decision {
                    None => {
                        data.remove(&key);
                    }
                    Some(value) => {
                        data.insert(key, Value::Bool(value));
                    }
                }
            }
            write_trust_file(&self.trust_path, &data)
        })
    }
}

/// Trust that global (user-owned) settings grant without a stored decision:
/// a `trustedProjects` entry or `defaultProjectTrust: always`. A project cannot
/// set either, so both count as the explicit trusted-host override.
fn host_configured_trust(
    cwd: &Path,
    default_project_trust: Option<&str>,
    trusted_projects: &[String],
) -> bool {
    let canonical = canonicalize_trust_path(cwd);
    let display = cwd.to_string_lossy();
    if trusted_projects
        .iter()
        .any(|path| path == &canonical || path.as_str() == display.as_ref())
    {
        return true;
    }
    matches!(default_project_trust.unwrap_or("ask"), "always")
}

/// TS `resolveProjectTrusted` without the interactive ask / extension hook.
pub fn resolve_project_trusted(
    agent_dir: &Path,
    cwd: &Path,
    override_trust: Option<bool>,
    default_project_trust: Option<&str>,
    trusted_projects: &[String],
) -> bool {
    if let Some(value) = override_trust {
        return value;
    }
    if !has_trust_requiring_project_resources(cwd) {
        return true;
    }
    match ProjectTrustStore::open(agent_dir).try_get(cwd) {
        Ok(Some(decision)) => return decision,
        Ok(None) => {}
        // A stored "Do not trust" we cannot read must not be overridden by
        // `defaultProjectTrust` or `trustedProjects`.
        Err(_) => return false,
    }
    host_configured_trust(cwd, default_project_trust, trusted_projects)
}

/// Stable machine-readable code for "project resources were not loaded because
/// the folder is not trusted". Headless callers match on this string.
pub const UNTRUSTED_PROJECT: &str = "UNTRUSTED_PROJECT";

/// Who can answer a trust question for this launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMode {
    /// A terminal user can be asked.
    Interactive,
    /// `--print`, `--mode json|rpc`, the SDK: nobody can be asked.
    Headless,
}

/// What startup must do about project trust before any project-controlled
/// resource (hooks, MCP servers, extensions, skills, packages) loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustGateDecision {
    /// Nothing to ask: no trust-requiring resources, a stored grant, a CLI
    /// override, or a trusted-host setting. Project resources may load.
    Preapproved,
    /// An explicit denial is stored (or `--no-approve`). No prompt; project
    /// resources stay unloaded.
    Declined,
    /// Interactive launch with trust-requiring resources and no stored
    /// decision: ask `No - Exit DaVinci` / `Yes, I trust this folder`.
    Prompt,
    /// Headless launch with trust-requiring resources and no grant: nobody can
    /// be asked, so project resources stay unloaded and the caller reports
    /// [`UNTRUSTED_PROJECT`].
    Unavailable,
    /// The trust store could not be read. Fails closed; the message names the
    /// file and the parse or IO error.
    StoreUnreadable(String),
}

/// Host-owned inputs to the trust decision. None of them can come from the
/// project: the override is a CLI flag and the rest is global settings.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrustPolicy<'a> {
    pub override_trust: Option<bool>,
    pub default_project_trust: Option<&'a str>,
    pub trusted_projects: &'a [String],
}

/// Decide, without side effects, whether startup must ask about `cwd`.
pub fn preflight_project_trust(
    agent_dir: &Path,
    cwd: &Path,
    policy: &TrustPolicy<'_>,
    mode: HostMode,
) -> TrustGateDecision {
    if let Some(value) = policy.override_trust {
        return if value {
            TrustGateDecision::Preapproved
        } else {
            TrustGateDecision::Declined
        };
    }
    if !has_trust_requiring_project_resources(cwd) {
        return TrustGateDecision::Preapproved;
    }
    match ProjectTrustStore::open(agent_dir).try_get(cwd) {
        Ok(Some(true)) => return TrustGateDecision::Preapproved,
        Ok(Some(false)) => return TrustGateDecision::Declined,
        Ok(None) => {}
        Err(message) => return TrustGateDecision::StoreUnreadable(message),
    }
    if host_configured_trust(cwd, policy.default_project_trust, policy.trusted_projects) {
        return TrustGateDecision::Preapproved;
    }
    match mode {
        HostMode::Interactive => TrustGateDecision::Prompt,
        HostMode::Headless => TrustGateDecision::Unavailable,
    }
}

/// The answer to the startup question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustGateChoice {
    /// `No - Exit DaVinci`. The default.
    Exit,
    /// `Yes, I trust this folder`.
    Trust,
}

/// Persist the startup answer to the canonical trust store (the same store
/// `/trust` and `/setup trust` write), keyed by the canonical project path.
pub fn record_gate_choice(
    agent_dir: &Path,
    cwd: &Path,
    choice: TrustGateChoice,
) -> Result<(), String> {
    ProjectTrustStore::open(agent_dir).set(cwd, Some(choice == TrustGateChoice::Trust))
}

/// One-line, copy-safe diagnostic for a headless launch that left project
/// resources unloaded. The code comes first so callers can match on it.
pub fn untrusted_project_message(cwd: &Path) -> String {
    format!(
        "{UNTRUSTED_PROJECT}: {} has project hooks, MCP servers, extensions, skills or packages that were not loaded because the folder is not trusted. Trust it once from an interactive session, pass --approve, or set defaultProjectTrust to \"always\" in the global settings.",
        canonicalize_trust_path(cwd)
    )
}

/// The same diagnostic as a JSON object for machine consumers.
pub fn untrusted_project_json(cwd: &Path) -> Value {
    serde_json::json!({
        "code": UNTRUSTED_PROJECT,
        "path": canonicalize_trust_path(cwd),
        "message": untrusted_project_message(cwd),
    })
}

pub fn canonicalize_trust_path(path: &Path) -> String {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    display_path(&canonical)
}

pub fn display_path(path: &Path) -> String {
    davinci_agent::strip_verbatim_prefix(path)
        .to_string_lossy()
        .into_owned()
}

pub fn get_project_trust_parent_path(cwd: &Path) -> Option<String> {
    let trust_path = PathBuf::from(canonicalize_trust_path(cwd));
    let parent = trust_path.parent()?;
    if parent == trust_path {
        None
    } else {
        Some(parent.to_string_lossy().into_owned())
    }
}

pub fn get_project_trust_options(
    cwd: &Path,
    include_session_only: bool,
) -> Vec<ProjectTrustOption> {
    let trust_path = canonicalize_trust_path(cwd);
    let mut options = vec![ProjectTrustOption {
        label: "Trust".into(),
        trusted: true,
        updates: vec![ProjectTrustUpdate {
            path: trust_path.clone(),
            decision: Some(true),
        }],
        saved_path: Some(trust_path.clone()),
    }];
    if let Some(parent_path) = get_project_trust_parent_path(cwd) {
        options.push(ProjectTrustOption {
            label: format!("Trust parent folder ({parent_path})"),
            trusted: true,
            updates: vec![
                ProjectTrustUpdate {
                    path: parent_path.clone(),
                    decision: Some(true),
                },
                ProjectTrustUpdate {
                    path: trust_path.clone(),
                    decision: None,
                },
            ],
            saved_path: Some(parent_path),
        });
    }
    if include_session_only {
        options.push(ProjectTrustOption {
            label: "Trust (this session only)".into(),
            trusted: true,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options.push(ProjectTrustOption {
        label: "Do not trust".into(),
        trusted: false,
        updates: vec![ProjectTrustUpdate {
            path: trust_path.clone(),
            decision: Some(false),
        }],
        saved_path: Some(trust_path),
    });
    if include_session_only {
        options.push(ProjectTrustOption {
            label: "Do not trust (this session only)".into(),
            trusted: false,
            updates: Vec::new(),
            saved_path: None,
        });
    }
    options
}

/// Names of the project resources that make `cwd` ask for trust, for the
/// startup question. Same list as [`has_trust_requiring_project_resources`].
pub fn trust_requiring_project_resource_names(cwd: &Path) -> Vec<String> {
    let current = PathBuf::from(canonicalize_trust_path(cwd));
    let mut names: Vec<String> = TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES
        .iter()
        .filter(|name| crate::project_config::any_exists(&current, &[name]))
        .map(|name| (*name).to_string())
        .collect();
    let home = davinci_session::home_dir()
        .map(|path| canonicalize_trust_path(&path))
        .unwrap_or_default();
    let user_agents_skills =
        canonicalize_trust_path(&PathBuf::from(&home).join(".agents").join("skills"));
    let mut dir = Some(current.as_path());
    while let Some(candidate) = dir {
        let agents_skills = canonicalize_trust_path(&candidate.join(".agents").join("skills"));
        if agents_skills != user_agents_skills && Path::new(&agents_skills).exists() {
            names.push(".agents/skills".to_string());
            break;
        }
        dir = candidate.parent().filter(|parent| *parent != candidate);
    }
    names
}

/// TS `hasTrustRequiringProjectResources`.
pub fn has_trust_requiring_project_resources(cwd: &Path) -> bool {
    let home = davinci_session::home_dir()
        .map(|path| canonicalize_trust_path(&path))
        .unwrap_or_default();
    let user_agents_skills = PathBuf::from(&home).join(".agents").join("skills");
    let user_agents_skills = canonicalize_trust_path(&user_agents_skills);
    let mut current = PathBuf::from(canonicalize_trust_path(cwd));
    if crate::project_config::any_exists(&current, TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES) {
        return true;
    }
    loop {
        let agents_skills = current.join(".agents").join("skills");
        let agents_skills = canonicalize_trust_path(&agents_skills);
        if agents_skills != user_agents_skills && Path::new(&agents_skills).exists() {
            return true;
        }
        let Some(parent) = current.parent() else {
            return false;
        };
        if parent == current {
            return false;
        }
        current = parent.to_path_buf();
    }
}

fn read_trust_file(path: &Path) -> Result<BTreeMap<String, Value>, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read trust store {}: {err}", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let parsed: Value = serde_json::from_str(text)
        .map_err(|err| format!("Failed to read trust store {}: {err}", path.display()))?;
    let Some(object) = parsed.as_object() else {
        return Err(format!(
            "Invalid trust store {}: expected an object",
            path.display()
        ));
    };
    let mut data = BTreeMap::new();
    for (key, value) in object {
        if !value.is_boolean() && !value.is_null() {
            return Err(format!(
                "Invalid trust store {}: value for {key:?} must be true, false, or null",
                path.display()
            ));
        }
        data.insert(key.clone(), value.clone());
    }
    Ok(data)
}

fn write_trust_file(path: &Path, data: &BTreeMap<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut sorted = serde_json::Map::new();
    for (key, value) in data {
        if value.is_boolean() || value.is_null() {
            sorted.insert(key.clone(), value.clone());
        }
    }
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(sorted)).map_err(|err| err.to_string())?
    );
    davinci_sys::fs::atomic_write(path, body.as_bytes()).map_err(|err| err.to_string())
}

fn find_nearest_trust_entry(
    data: &BTreeMap<String, Value>,
    cwd: &Path,
) -> Option<ProjectTrustStoreEntry> {
    let mut current = PathBuf::from(canonicalize_trust_path(cwd));
    loop {
        let key = current.to_string_lossy().into_owned();
        let legacy = format!(r"\\?\{key}");
        if let Some(Value::Bool(decision)) = data.get(&key).or_else(|| data.get(&legacy)) {
            return Some(ProjectTrustStoreEntry {
                path: key,
                decision: *decision,
            });
        }
        let parent = current.parent()?;
        if parent == current {
            return None;
        }
        current = parent.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn display_path_never_exposes_a_windows_verbatim_prefix() {
        let path = Path::new(r"\\?\C:\work\repo");
        assert_eq!(display_path(path), r"C:\work\repo");
    }

    #[test]
    fn stores_decisions_and_inherits_from_parent_directories() {
        let dir = tempdir().unwrap();
        let store = ProjectTrustStore::open(&dir.path().join("agent"));
        let parent = dir.path().join("trusted-parent");
        let child = parent.join("project");
        fs::create_dir_all(&child).unwrap();
        assert_eq!(store.get(&child), None);
        store.set(&parent, Some(true)).unwrap();
        assert_eq!(store.get(&child), Some(true));
        store.set(&child, Some(false)).unwrap();
        assert_eq!(store.get(&child), Some(false));
        store.set(&child, None).unwrap();
        assert_eq!(store.get(&child), Some(true));
    }

    #[test]
    #[cfg(not(windows))]
    fn detects_trust_requiring_project_resources() {
        let dir = tempdir().unwrap();
        let previous = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());
        let cwd = dir.path().join("project");
        fs::create_dir_all(dir.path().join(".pi").join("agent")).unwrap();
        fs::create_dir_all(dir.path().join(".agents").join("skills")).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        assert!(!has_trust_requiring_project_resources(dir.path()));
        assert!(!has_trust_requiring_project_resources(&cwd));
        fs::write(dir.path().join(".pi").join("settings.json"), "{}").unwrap();
        assert!(has_trust_requiring_project_resources(dir.path()));
        let _ = fs::remove_file(dir.path().join(".pi").join("settings.json"));
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("settings.json"), "{}").unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
        let _ = fs::remove_dir_all(cwd.join(".pi"));
        fs::create_dir_all(cwd.join(".agents").join("skills")).unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
        match previous {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn trust_options_include_parent_folder() {
        let options = get_project_trust_options(Path::new("/parent/project"), false);
        assert_eq!(options[0].label, "Trust");
        assert!(options
            .iter()
            .any(|option| option.label == "Trust parent folder (/parent)"));
        assert!(options.iter().any(|option| option.label == "Do not trust"));
        assert!(!options
            .iter()
            .any(|option| option.label.contains("session only")));
        let with_session = get_project_trust_options(Path::new("/parent/project"), true);
        assert!(with_session
            .iter()
            .any(|option| option.label == "Trust (this session only)"));
    }

    #[test]
    fn resolve_uses_store_override_and_default() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("ask"),
            &[]
        ));
        assert!(resolve_project_trusted(
            &agent,
            &project,
            Some(true),
            Some("never"),
            &[]
        ));
        assert!(resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("always"),
            &[]
        ));
        ProjectTrustStore::open(&agent)
            .set(&project, Some(false))
            .unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("always"),
            &[]
        ));
    }

    #[test]
    fn project_mcp_config_requires_trust() {
        let dir = tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(
            cwd.join(".pi").join("mcp.json"),
            r#"{"mcpServers":{"x":{"command":"calc"}}}"#,
        )
        .unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
    }

    #[test]
    fn empty_davinci_dir_does_not_hide_legacy_hooks() {
        let dir = tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join(".davinci")).unwrap();
        fs::write(cwd.join(".davinci").join("README"), "").unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("hooks.json"), "{}").unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
    }

    #[test]
    fn project_agents_and_local_package_roots_require_trust() {
        for name in ["agents", "git", "npm"] {
            let dir = tempdir().unwrap();
            let cwd = dir.path().join("repo");
            fs::create_dir_all(cwd.join(".pi").join(name)).unwrap();
            assert!(has_trust_requiring_project_resources(&cwd), "{name}");
        }
    }

    #[test]
    fn untrusted_repo_with_only_mcp_json_is_not_auto_trusted() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("mcp.json"), "{}").unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &cwd,
            None,
            Some("ask"),
            &[]
        ));
    }

    #[test]
    fn unreadable_trust_store_is_not_trusted_even_with_default_always() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("trust.json"), "{ not json").unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("always"),
            &[]
        ));
    }

    #[test]
    fn held_trust_lock_is_not_trusted_even_with_default_always() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        fs::create_dir_all(&agent).unwrap();
        let _held =
            davinci_sys::lock::ExclusiveFileLock::try_acquire(&agent.join("trust.json.lock"))
                .unwrap();
        assert!(!resolve_project_trusted(
            &agent,
            &project,
            None,
            Some("always"),
            &[]
        ));
    }

    fn project_with_hooks(root: &Path) -> PathBuf {
        let project = root.join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("hooks.json"), "{}").unwrap();
        project
    }

    fn policy<'a>(
        override_trust: Option<bool>,
        default: Option<&'a str>,
        trusted: &'a [String],
    ) -> TrustPolicy<'a> {
        TrustPolicy {
            override_trust,
            default_project_trust: default,
            trusted_projects: trusted,
        }
    }

    #[test]
    fn preflight_asks_only_when_resources_exist_and_no_decision_is_stored() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = project_with_hooks(dir.path());
        let plain = dir.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        let none = policy(None, None, &[]);
        assert_eq!(
            preflight_project_trust(&agent, &plain, &none, HostMode::Interactive),
            TrustGateDecision::Preapproved
        );
        assert_eq!(
            preflight_project_trust(&agent, &project, &none, HostMode::Interactive),
            TrustGateDecision::Prompt
        );
        assert_eq!(
            preflight_project_trust(&agent, &project, &none, HostMode::Headless),
            TrustGateDecision::Unavailable
        );
    }

    #[test]
    fn preflight_honors_stored_grants_and_denials_without_asking() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = project_with_hooks(dir.path());
        let none = policy(None, None, &[]);
        let store = ProjectTrustStore::open(&agent);
        store.set(&project, Some(true)).unwrap();
        for mode in [HostMode::Interactive, HostMode::Headless] {
            assert_eq!(
                preflight_project_trust(&agent, &project, &none, mode),
                TrustGateDecision::Preapproved
            );
        }
        store.set(&project, Some(false)).unwrap();
        for mode in [HostMode::Interactive, HostMode::Headless] {
            assert_eq!(
                preflight_project_trust(&agent, &project, &none, mode),
                TrustGateDecision::Declined
            );
        }
    }

    #[test]
    fn preflight_host_overrides_only_come_from_host_inputs() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = project_with_hooks(dir.path());
        assert_eq!(
            preflight_project_trust(
                &agent,
                &project,
                &policy(Some(true), None, &[]),
                HostMode::Headless
            ),
            TrustGateDecision::Preapproved
        );
        assert_eq!(
            preflight_project_trust(
                &agent,
                &project,
                &policy(Some(false), Some("always"), &[]),
                HostMode::Interactive
            ),
            TrustGateDecision::Declined
        );
        assert_eq!(
            preflight_project_trust(
                &agent,
                &project,
                &policy(None, Some("always"), &[]),
                HostMode::Headless
            ),
            TrustGateDecision::Preapproved
        );
        let listed = [canonicalize_trust_path(&project)];
        assert_eq!(
            preflight_project_trust(
                &agent,
                &project,
                &policy(None, Some("ask"), &listed),
                HostMode::Headless
            ),
            TrustGateDecision::Preapproved
        );
    }

    #[test]
    fn preflight_stored_denial_beats_default_always() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = project_with_hooks(dir.path());
        ProjectTrustStore::open(&agent)
            .set(&project, Some(false))
            .unwrap();
        assert_eq!(
            preflight_project_trust(
                &agent,
                &project,
                &policy(None, Some("always"), &[]),
                HostMode::Headless
            ),
            TrustGateDecision::Declined
        );
    }

    #[test]
    fn preflight_reports_an_unreadable_store_instead_of_asking() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = project_with_hooks(dir.path());
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("trust.json"), "{ not json").unwrap();
        let decision = preflight_project_trust(
            &agent,
            &project,
            &policy(None, Some("always"), &[]),
            HostMode::Interactive,
        );
        match decision {
            TrustGateDecision::StoreUnreadable(message) => {
                assert!(message.contains("trust store"), "{message}");
                assert!(message.contains("trust.json"), "{message}");
            }
            other => panic!("expected StoreUnreadable, got {other:?}"),
        }
    }

    #[test]
    fn recorded_gate_choices_persist_and_feed_the_next_preflight() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = project_with_hooks(dir.path());
        let none = policy(None, None, &[]);
        record_gate_choice(&agent, &project, TrustGateChoice::Exit).unwrap();
        assert_eq!(
            preflight_project_trust(&agent, &project, &none, HostMode::Interactive),
            TrustGateDecision::Declined
        );
        record_gate_choice(&agent, &project, TrustGateChoice::Trust).unwrap();
        assert_eq!(
            preflight_project_trust(&agent, &project, &none, HostMode::Interactive),
            TrustGateDecision::Preapproved
        );
    }

    #[test]
    fn untrusted_project_diagnostic_leads_with_the_stable_code() {
        let dir = tempdir().unwrap();
        let project = project_with_hooks(dir.path());
        assert!(untrusted_project_message(&project).starts_with("UNTRUSTED_PROJECT: "));
        let json = untrusted_project_json(&project);
        assert_eq!(json["code"], "UNTRUSTED_PROJECT");
        assert_eq!(json["path"], canonicalize_trust_path(&project));
    }
}
