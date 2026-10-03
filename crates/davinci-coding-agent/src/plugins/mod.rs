//! Plugins in the Claude Code / Codex format: skills, commands, agents,
//! hooks and MCP servers, from DaVinci's own marketplaces or adopted from an
//! existing Claude Code or Codex install.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.
//!
//! `DAVINCI_PLUGINS=off` loads nothing. Graph workers (`PI_GRAPH_ROLE`)
//! load resources but never run plugin hooks.

pub mod command;
pub mod external;
pub mod hooks;
pub mod manager;
pub mod manifest;
pub mod marketplace;
pub mod store;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::agent_profiles::AgentProfile;
use hooks::{HookContext, HookEvent, HookOutcome};
use manifest::Plugin;
use store::{InstalledPlugin, Origin};

/// One enabled, loadable plugin.
#[derive(Debug, Clone)]
pub struct ActivePlugin {
    pub key: String,
    pub record: InstalledPlugin,
    pub plugin: Plugin,
    pub data_dir: PathBuf,
    /// User store that authorized this loaded plugin, bound independently of cwd.
    pub agent_dir: PathBuf,
}

impl ActivePlugin {
    /// Hooks run only when the approved digest matches the hooks on disk.
    pub fn hooks_approved(&self) -> bool {
        #[cfg(test)]
        tests::APPROVAL_CHECKS.with(|count| count.set(count.get() + 1));
        let Some(digest) = &self.plugin.hooks_digest else {
            return false;
        };
        #[cfg(test)]
        tests::REGISTRY_READS.with(|count| count.set(count.get() + 1));
        let Ok(installed) = store::load(&self.agent_dir) else {
            return false;
        };
        let Some(record) = installed.plugins.get(&self.key) else {
            return false;
        };
        record.enabled
            && record.hooks_approved.as_ref() == Some(digest)
            && locate(&self.key, record)
                .ok()
                .and_then(|path| manifest::canonical(&path).ok())
                .as_ref()
                == Some(&self.plugin.root)
            && {
                #[cfg(test)]
                tests::DIGEST_CHECKS.with(|count| count.set(count.get() + 1));
                hooks::digest_unless_unchanged(&self.plugin.root, &self.plugin.hooks).as_ref()
                    == Some(digest)
            }
    }

    fn hook_context(&self, cwd: &Path) -> HookContext {
        HookContext {
            plugin_name: self.plugin.name.clone(),
            plugin_root: self.plugin.root.clone(),
            data_dir: self.data_dir.clone(),
            cwd: cwd.to_path_buf(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActivePlugins {
    pub plugins: Vec<ActivePlugin>,
    /// Enabled plugins that could not be loaded, with the reason.
    pub errors: Vec<(String, String)>,
}

pub fn disabled_by_env() -> bool {
    std::env::var("DAVINCI_PLUGINS")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
        .unwrap_or(false)
}

fn hooks_allowed_here() -> bool {
    !disabled_by_env() && std::env::var_os("PI_GRAPH_ROLE").is_none()
}

/// Where an installed plugin lives right now.
pub fn locate(key: &str, record: &InstalledPlugin) -> Result<PathBuf, String> {
    match record.origin {
        Origin::Davinci => record
            .install_path
            .clone()
            .ok_or_else(|| "install record has no path".to_string()),
        origin => external::resolve(origin, key)
            .map(|found| found.path)
            .ok_or_else(|| format!("no longer installed in {}", origin.label())),
    }
}

/// Enabled plugins from `<agent_dir>/plugins/installed.json`.
pub fn active(agent_dir: &Path) -> ActivePlugins {
    let mut out = ActivePlugins::default();
    if disabled_by_env() {
        return out;
    }
    let installed = match store::load(agent_dir) {
        Ok(installed) => installed,
        Err(err) => {
            out.errors.push(("installed.json".into(), err));
            return out;
        }
    };
    for (key, record) in installed.plugins {
        if !record.enabled {
            continue;
        }
        let loaded = manifest::canonical(agent_dir)
            .map_err(|error| format!("plugin store unavailable: {error}"))
            .and_then(|bound_dir| {
                locate(&key, &record)
                    .and_then(|path| manifest::load_plugin(&path))
                    .map(|plugin| (bound_dir, plugin))
            });
        match loaded {
            Ok((bound_dir, plugin)) => {
                let data_dir = store::plugins_dir(agent_dir)
                    .join("data")
                    .join(store::split_key(&key).0);
                out.plugins.push(ActivePlugin {
                    key,
                    record,
                    plugin,
                    data_dir,
                    agent_dir: bound_dir,
                });
            }
            Err(err) => out.errors.push((key, err)),
        }
    }
    out
}

impl ActivePlugins {
    pub fn skill_files(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .flat_map(|p| p.plugin.skill_files.iter().cloned())
            .collect()
    }

    pub fn command_files(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .flat_map(|p| p.plugin.command_files.iter().cloned())
            .collect()
    }

    /// Plugin agents as DaVinci agent profiles.
    pub fn agent_profiles(&self) -> Vec<AgentProfile> {
        let mut out = Vec::new();
        for active in &self.plugins {
            for path in &active.plugin.agent_files {
                let Ok(content) = std::fs::read_to_string(path) else {
                    continue;
                };
                let explicit_mode = content_declares(&content, &["permission_mode", "permissions"]);
                if let Ok(profile) = AgentProfile::parse(&content, path, false) {
                    out.push(adapt_agent_profile(profile, explicit_mode));
                }
            }
        }
        out
    }

    /// MCP servers as `plugin_<plugin>_<server>` with plugin variables
    /// expanded. Entries that do not parse are skipped; [`Self::diagnostics`]
    /// names them.
    pub fn mcp_servers(&self) -> BTreeMap<String, davinci_mcp::ServerConfig> {
        self.parsed_mcp_servers().0
    }

    /// Load failures and unparseable MCP entries, one line each.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .errors
            .iter()
            .map(|(key, error)| format!("plugin {key} failed to load: {error}"))
            .collect();
        out.extend(self.parsed_mcp_servers().1);
        out
    }

    fn parsed_mcp_servers(&self) -> (BTreeMap<String, davinci_mcp::ServerConfig>, Vec<String>) {
        let mut out = BTreeMap::new();
        let mut errors = Vec::new();
        for active in &self.plugins {
            let vars = [
                (
                    "${CLAUDE_PLUGIN_ROOT}",
                    active.plugin.root.display().to_string(),
                ),
                (
                    "${DAVINCI_PLUGIN_ROOT}",
                    active.plugin.root.display().to_string(),
                ),
                (
                    "${CLAUDE_PLUGIN_DATA}",
                    active.data_dir.display().to_string(),
                ),
            ];
            for (server, config) in &active.plugin.mcp_servers {
                let expanded = expand_strings(config, &vars);
                match serde_json::from_value::<davinci_mcp::ServerConfig>(expanded) {
                    Ok(parsed) => {
                        let name = format!("plugin_{}_{}", active.plugin.name, server);
                        out.insert(name, parsed);
                    }
                    Err(error) => errors.push(format!(
                        "plugin {} MCP server {server} skipped: {error}",
                        active.plugin.name
                    )),
                }
            }
        }
        (out, errors)
    }

    /// Check consent only for relevant plugins, once per synchronous query.
    /// Callers must still recheck consent at execution time.
    fn approved_plugins_for<'a>(
        &'a self,
        event: HookEvent,
        subject: Option<&'a str>,
    ) -> impl Iterator<Item = &'a ActivePlugin> {
        let allowed = hooks_allowed_here();
        self.plugins.iter().filter(move |active| {
            allowed
                && active.plugin.hooks.iter().any(|hook| {
                    hook.event == event
                        && subject.map_or(true, |subject| {
                            hooks::matcher_accepts(hook.matcher.as_deref(), subject)
                        })
                })
                && active.hooks_approved()
        })
    }

    pub fn has_hooks(&self, event: HookEvent) -> bool {
        self.approved_plugins_for(event, None).next().is_some()
    }

    /// Whether an approved hook for `event` would run for the Claude tool
    /// name `subject`. The harness asks this before reading a file on the
    /// model's behalf, since such a hook would never see that read.
    pub fn has_matching_hook(&self, event: HookEvent, subject: &str) -> bool {
        self.approved_plugins_for(event, Some(subject))
            .next()
            .is_some()
    }

    /// Run every matching hook for `event`. Contexts are collected in order;
    /// the first block stops the run.
    pub fn run_event(
        &self,
        event: HookEvent,
        subject: Option<&str>,
        input: &HookInput,
    ) -> EventResult {
        self.run_event_cancellable(event, subject, input, &|| false)
    }

    /// [`Self::run_event`] that stops at the first hook once `cancelled`
    /// reports true, including one that is already running.
    pub fn run_event_cancellable(
        &self,
        event: HookEvent,
        subject: Option<&str>,
        input: &HookInput,
        cancelled: &dyn Fn() -> bool,
    ) -> EventResult {
        if !input.session_id.is_empty() {
            *last_session_id().lock().unwrap_or_else(|e| e.into_inner()) = input.session_id.clone();
        }
        let mut result = EventResult::default();
        if !hooks_allowed_here() {
            return result;
        }
        // Candidate discovery is cheap and carries no execution consent.
        let candidates = self.plugins.iter().flat_map(|active| {
            active.plugin.hooks.iter().filter_map(move |hook| {
                let matched = hook.event == event
                    && match event {
                        HookEvent::PreToolUse
                        | HookEvent::PostToolUse
                        | HookEvent::SessionStart => {
                            hooks::matcher_accepts(hook.matcher.as_deref(), subject.unwrap_or(""))
                        }
                        _ => true,
                    };
                matched.then_some((active, hook))
            })
        });
        for (active, hook) in candidates {
            if cancelled() || !hooks_allowed_here() {
                break;
            }
            // Recheck even the first dispatch: cancellation callbacks and
            // earlier hooks may revoke approval or change the plugin's files.
            if !active.hooks_approved() {
                continue;
            }
            let outcome: HookOutcome = hooks::run_cancellable(
                hook,
                &active.hook_context(&input.cwd),
                &input.payload(event),
                cancelled,
            );
            if let Some(context) = outcome.context {
                result.contexts.push((active.plugin.name.clone(), context));
            }
            if let Some(warning) = outcome.warning {
                result.warnings.push(format!(
                    "plugin {} {} hook: {warning}",
                    active.plugin.name,
                    event.as_str()
                ));
            }
            if let Some(reason) = outcome.block {
                result.block = Some(format!("plugin {}: {reason}", active.plugin.name));
                break;
            }
        }
        result
    }
}

/// The parts of Claude Code's hook stdin document DaVinci can supply.
#[derive(Debug, Clone, Default)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: Option<PathBuf>,
    pub cwd: PathBuf,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub tool_response: Option<Value>,
    pub prompt: Option<String>,
    pub source: Option<String>,
    pub stop_hook_active: bool,
}

impl HookInput {
    fn payload(&self, event: HookEvent) -> Value {
        let mut value = json!({
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": self.cwd,
            "hook_event_name": event.as_str(),
        });
        if event == HookEvent::Stop {
            value["stop_hook_active"] = json!(self.stop_hook_active);
        }
        let object = value.as_object_mut().expect("object literal");
        if let Some(tool) = &self.tool_name {
            object.insert("tool_name".into(), json!(tool));
        }
        if let Some(input) = &self.tool_input {
            object.insert("tool_input".into(), input.clone());
        }
        if let Some(response) = &self.tool_response {
            object.insert("tool_response".into(), response.clone());
        }
        if let Some(prompt) = &self.prompt {
            object.insert("prompt".into(), json!(prompt));
        }
        if let Some(source) = &self.source {
            object.insert("source".into(), json!(source));
        }
        value
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventResult {
    /// `(plugin name, text)` pairs.
    pub contexts: Vec<(String, String)>,
    pub block: Option<String>,
    pub warnings: Vec<String>,
}

impl EventResult {
    pub fn joined_context(&self) -> Option<String> {
        if self.contexts.is_empty() {
            return None;
        }
        Some(
            self.contexts
                .iter()
                .map(|(plugin, text)| {
                    format!("<plugin-context plugin=\"{plugin}\">\n{text}\n</plugin-context>")
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }
}

/// `(plugin name, text)` pairs of hook output.
pub type PluginContexts = Vec<(String, String)>;

/// What `SessionStart` hooks produced: context for the model, and warnings
/// (failed or timed-out hooks) for the user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionStartOutput {
    pub contexts: PluginContexts,
    pub warnings: Vec<String>,
}

/// `SessionStart` output, computed once per cwd, session and set of
/// approved hooks: resource discovery runs on every reload and prompt, a
/// session starts once. A new session, or a plugin enabled or approved
/// mid-session, changes the key and runs the hooks again. Warnings from a
/// fresh run are also queued for [`take_notices`].
pub fn session_start_context(
    plugins: &ActivePlugins,
    cwd: &Path,
    session_id: &str,
) -> SessionStartOutput {
    type Key = (PathBuf, String, String);
    static CACHE: OnceLock<Mutex<BTreeMap<Key, SessionStartOutput>>> = OnceLock::new();
    let fingerprint = plugins
        .approved_plugins_for(HookEvent::SessionStart, Some("startup"))
        .map(|active| {
            format!(
                "{}@{}",
                active.key,
                active.plugin.hooks_digest.as_deref().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    if fingerprint.is_empty() {
        return SessionStartOutput::default();
    }
    let key = (cwd.to_path_buf(), session_id.to_string(), fingerprint);
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(found) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return found.clone();
    }
    let input = HookInput {
        session_id: session_id.to_string(),
        cwd: cwd.to_path_buf(),
        source: Some("startup".into()),
        ..HookInput::default()
    };
    let result = plugins.run_event(HookEvent::SessionStart, Some("startup"), &input);
    let output = SessionStartOutput {
        contexts: result.contexts,
        warnings: result.warnings,
    };
    pending_notices()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(output.warnings.iter().cloned());
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, output.clone());
    output
}

fn pending_notices() -> &'static Mutex<Vec<String>> {
    static PENDING: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

/// Plugin warnings not yet shown to the user: `SessionStart` hook failures
/// since the last call. Each is returned once.
pub fn take_notices() -> Vec<String> {
    std::mem::take(&mut *pending_notices().lock().unwrap_or_else(|e| e.into_inner()))
}

/// Everything about the enabled plugins the user should hear at startup:
/// plugins that failed to load, MCP entries that did not parse, and pending
/// hook warnings.
pub fn startup_warnings(agent_dir: &Path) -> Vec<String> {
    let mut out = active(agent_dir).diagnostics();
    out.extend(take_notices());
    out
}

/// Session id most recently passed to a hook. The session file is created
/// lazily, so `SessionEnd` (which runs with no agent at hand) reuses the id
/// the turn hooks saw.
fn last_session_id() -> &'static Mutex<String> {
    static LAST: OnceLock<Mutex<String>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(String::new()))
}

/// `SessionEnd` hooks, run where the user's own `stop` hooks run.
pub fn run_session_end(agent_dir: &Path, cwd: &Path) {
    let plugins = active(agent_dir);
    if !plugins.has_hooks(HookEvent::SessionEnd) {
        return;
    }
    let input = HookInput {
        session_id: last_session_id()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
        cwd: cwd.to_path_buf(),
        ..HookInput::default()
    };
    for warning in plugins
        .run_event(HookEvent::SessionEnd, None, &input)
        .warnings
    {
        eprintln!("[davinci-plugins] {warning}");
    }
}

fn content_declares(content: &str, keys: &[&str]) -> bool {
    let (frontmatter, _) = davinci_agent::parse_frontmatter(content);
    keys.iter().any(|key| frontmatter.contains_key(*key))
}

/// Claude Code agent files use Claude tool names and short model aliases.
fn adapt_agent_profile(mut profile: AgentProfile, explicit_mode: bool) -> AgentProfile {
    profile.tools = profile
        .tools
        .iter()
        .flat_map(|tool| match tool.as_str() {
            "Read" => vec!["read"],
            "Write" => vec!["write"],
            "Edit" | "MultiEdit" => vec!["edit"],
            "Bash" => vec!["bash"],
            "PowerShell" => vec!["powershell"],
            "Grep" => vec!["grep"],
            "Glob" => vec!["find"],
            "LS" => vec!["ls"],
            "WebFetch" => vec!["web_fetch"],
            "WebSearch" => vec!["web_search"],
            "TodoWrite" => vec!["todo"],
            "NotebookEdit" => vec!["notebook_edit"],
            "Task" | "Agent" => vec!["agent"],
            other => vec![other],
        })
        .map(str::to_string)
        .collect();
    profile.tools.dedup();
    if !profile.model.contains('/') {
        profile.model = "inherit".into();
    }
    if !explicit_mode
        && profile
            .tools
            .iter()
            .any(|tool| matches!(tool.as_str(), "write" | "edit" | "notebook_edit"))
    {
        profile.permission_mode = "edits".into();
    }
    profile
}

fn expand_strings(value: &Value, vars: &[(&str, String)]) -> Value {
    match value {
        Value::String(text) => {
            let mut text = text.clone();
            for (name, replacement) in vars {
                text = text.replace(name, replacement);
            }
            Value::String(text)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| expand_strings(v, vars)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), expand_strings(v, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::external::tests::{write, FakeHomes};

    thread_local! {
        pub(super) static APPROVAL_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static REGISTRY_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static DIGEST_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn selection_fixture(agent_dir: &Path, root: &Path, name: &str) {
        write(
            &root.join("plugin.json"),
            &json!({"name": name}).to_string(),
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"PreToolUse":[{"matcher":"Write","hooks":[{"command":"exit 2"}]}],"SessionStart":[{"matcher":"resume","hooks":[{"command":"exit 2"}]}]}}"#,
        );
        install_fixture(agent_dir, root, true);
    }

    #[test]
    fn simplification_nonmatching_hook_performs_no_approval_scan() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        selection_fixture(&agent_dir, &dir.path().join("plugin"), "selection");
        let loaded = active(&agent_dir);
        APPROVAL_CHECKS.with(|count| count.set(0));
        assert!(!loaded.has_hooks(HookEvent::Stop));
        assert!(!loaded.has_matching_hook(HookEvent::PreToolUse, "Read"));
        assert_eq!(
            session_start_context(&loaded, dir.path(), "selection-test"),
            SessionStartOutput::default()
        );
        let result = loaded.run_event(HookEvent::PreToolUse, Some("Read"), &HookInput::default());
        assert_eq!(result, EventResult::default());
        assert_eq!(APPROVAL_CHECKS.with(|count| count.get()), 0);
        assert!(loaded.has_matching_hook(HookEvent::PreToolUse, "Write"));
        assert_eq!(APPROVAL_CHECKS.with(|count| count.get()), 1);
    }

    #[test]
    fn simplification_matching_hook_rechecks_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        selection_fixture(&agent_dir, &dir.path().join("plugin"), "selection");
        let loaded = active(&agent_dir);
        assert!(loaded.has_matching_hook(HookEvent::PreToolUse, "Write"));
        let revoked = std::cell::Cell::new(false);
        let result = loaded.run_event_cancellable(
            HookEvent::PreToolUse,
            Some("Write"),
            &HookInput {
                cwd: dir.path().into(),
                ..Default::default()
            },
            &|| {
                if !revoked.replace(true) {
                    store::update(&agent_dir, |registry| {
                        registry
                            .plugins
                            .get_mut("selection@local")
                            .unwrap()
                            .hooks_approved = None;
                        Ok(())
                    })
                    .unwrap();
                }
                false
            },
        );
        assert_eq!(
            result,
            EventResult::default(),
            "revoked first hook must not execute"
        );
    }

    #[test]
    fn simplification_first_hook_mutation_invalidates_second() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("mutation");
        let marker = root.join("changed.txt");
        write(&root.join("plugin.json"), r#"{"name":"mutation"}"#);
        #[cfg(windows)]
        let command = format!(
            "$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('{}', 'changed')",
            marker.display()
        );
        #[cfg(not(windows))]
        let command = format!("cat >/dev/null; echo changed > '{}'", marker.display());
        write(&root.join("hooks/hooks.json"), &json!({"hooks": {"Stop": [{
            "matcher": "ignored-for-stop",
            "hooks": [
                {"command": command, "shell": if cfg!(windows) { "powershell" } else { "bash" }},
                {"command": "exit 2"}
            ]
        }]}}).to_string());
        install_fixture(&agent_dir, &root, true);
        let loaded = active(&agent_dir);
        let result = loaded.run_event(
            HookEvent::Stop,
            None,
            &HookInput {
                cwd: dir.path().into(),
                ..Default::default()
            },
        );
        assert!(
            marker.exists(),
            "first hook must actually run despite its Stop matcher"
        );
        assert_eq!(
            result,
            EventResult::default(),
            "changed plugin must not run its second hook"
        );
    }

    #[test]
    fn simplification_disabled_hooks_skip_dispatch_callbacks() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        selection_fixture(&agent_dir, &dir.path().join("plugin"), "selection");
        let loaded = active(&agent_dir);
        let previous = std::env::var_os("DAVINCI_PLUGINS");
        std::env::set_var("DAVINCI_PLUGINS", "off");
        let callbacks = std::cell::Cell::new(0);
        APPROVAL_CHECKS.with(|count| count.set(0));
        let result = loaded.run_event_cancellable(
            HookEvent::PreToolUse,
            Some("Write"),
            &HookInput::default(),
            &|| {
                callbacks.set(callbacks.get() + 1);
                false
            },
        );
        if let Some(value) = previous {
            std::env::set_var("DAVINCI_PLUGINS", value);
        } else {
            std::env::remove_var("DAVINCI_PLUGINS");
        }
        assert_eq!(result, EventResult::default());
        assert_eq!(APPROVAL_CHECKS.with(|count| count.get()), 0);
        assert_eq!(callbacks.get(), 0);
    }

    #[test]
    fn simplification_cancelled_event_performs_no_approval_scan() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        selection_fixture(&agent_dir, &dir.path().join("plugin"), "selection");
        let loaded = active(&agent_dir);
        APPROVAL_CHECKS.with(|count| count.set(0));
        assert_eq!(
            loaded.run_event_cancellable(
                HookEvent::PreToolUse,
                Some("Write"),
                &HookInput::default(),
                &|| true
            ),
            EventResult::default()
        );
        assert_eq!(APPROVAL_CHECKS.with(|count| count.get()), 0);
    }

    #[test]
    #[ignore = "local paired timing probe; no network or provider calls"]
    fn simplification_hook_selection_probe() {
        for plugin_count in [0, 1, 24] {
            let dir = tempfile::tempdir().unwrap();
            let _homes = FakeHomes::new(dir.path());
            let agent_dir = dir.path().join("agent");
            for index in 0..plugin_count {
                let name = format!("selection-{index}");
                let root = dir.path().join(&name);
                for file in 0..16 {
                    write(
                        &root.join(format!("resource-{file}.txt")),
                        "fixture resource",
                    );
                }
                selection_fixture(&agent_dir, &root, &name);
            }
            let loaded = active(&agent_dir);
            for subject in ["Read", "Write"] {
                for sample in 0..30 {
                    APPROVAL_CHECKS.with(|count| count.set(0));
                    REGISTRY_READS.with(|count| count.set(0));
                    DIGEST_CHECKS.with(|count| count.set(0));
                    let started = std::time::Instant::now();
                    let matched = loaded.has_matching_hook(HookEvent::PreToolUse, subject);
                    assert_eq!(matched, plugin_count > 0 && subject == "Write");
                    println!(
                        "SIMPLIFICATION_PROBE {}",
                        json!({
                            "plugins": plugin_count,
                            "subject": subject, "sample": sample, "matched": matched,
                            "elapsed_ns": started.elapsed().as_nanos(),
                            "approval_checks": APPROVAL_CHECKS.with(|count| count.get()),
                            "registry_reads": REGISTRY_READS.with(|count| count.get()),
                            "digest_checks": DIGEST_CHECKS.with(|count| count.get()),
                        })
                    );
                }
            }
        }
    }

    #[test]
    fn audit_corrupt_registry_is_visible_in_loader_command_and_manager() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let registry = dir.path().join("plugins/installed.json");
        write(&registry, "{broken");
        let loaded = active(dir.path());
        assert!(loaded.plugins.is_empty());
        assert_eq!(loaded.errors.len(), 1);
        assert!(loaded.errors[0].1.contains("invalid registry"));
        assert!(command::run(&[], dir.path(), dir.path())
            .unwrap_err()
            .contains("installed.json"));
        let rows = manager::plugin_rows(dir.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].health, manager::Health::Failed);
        assert!(!rows[0].can_toggle && !rows[0].can_approve && !rows[0].can_delete);
        assert_eq!(std::fs::read_to_string(registry).unwrap(), "{broken");
    }

    #[test]
    fn stop_payload_reports_continuation_only_for_stop() {
        for active in [false, true] {
            let input = HookInput {
                stop_hook_active: active,
                ..Default::default()
            };
            assert_eq!(input.payload(HookEvent::Stop)["stop_hook_active"], active);
            assert!(input
                .payload(HookEvent::PostToolUse)
                .get("stop_hook_active")
                .is_none());
        }
    }

    #[test]
    fn stop_hooks_require_current_approval_digest_and_never_run_in_workers() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("stop-demo");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"stop-demo"}"#,
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"echo completion denied >&2; exit 2"}]}]}}"#,
        );
        install_fixture(&agent_dir, &root, false);
        let unapproved = active(&agent_dir);
        assert!(!unapproved.has_hooks(HookEvent::Stop));
        assert_eq!(
            unapproved
                .run_event(HookEvent::Stop, None, &HookInput::default())
                .block,
            None
        );
        install_fixture(&agent_dir, &root, true);
        let approved = active(&agent_dir);
        assert!(approved.has_hooks(HookEvent::Stop));
        std::env::set_var("PI_GRAPH_ROLE", "writer");
        let worker_has_hooks = approved.has_hooks(HookEvent::Stop);
        let worker_result = approved.run_event(HookEvent::Stop, None, &HookInput::default());
        std::env::remove_var("PI_GRAPH_ROLE");
        assert!(!worker_has_hooks);
        assert_eq!(worker_result.block, None);
        write(&root.join("new-script.sh"), "changed during this prompt");
        assert!(
            !approved.has_hooks(HookEvent::Stop),
            "already loaded plugins must recheck their digest"
        );
        assert_eq!(
            approved
                .run_event(HookEvent::Stop, None, &HookInput::default())
                .block,
            None
        );
    }

    fn install_fixture(agent_dir: &Path, root: &Path, approve: bool) {
        let plugin = manifest::load_plugin(root).unwrap();
        store::update(agent_dir, |file| {
            file.plugins.insert(
                format!("{}@local", plugin.name),
                InstalledPlugin {
                    origin: Origin::Davinci,
                    install_path: Some(plugin.root.clone()),
                    version: None,
                    enabled: true,
                    hooks_approved: if approve {
                        plugin.hooks_digest.clone()
                    } else {
                        None
                    },
                    installed_at: 0,
                },
            );
            Ok(())
        })
        .unwrap();
    }

    fn loaded_hook_revocation(change: &str) {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("revocation-demo");
        let marker = dir.path().join("hook-marker");
        write(&root.join("plugin.json"), r#"{"name":"revocation-demo"}"#);
        #[cfg(windows)]
        let command = format!(
            "$null = [Console]::In.ReadToEnd(); [IO.File]::AppendAllText('{}', 'ran')",
            marker.display()
        );
        #[cfg(not(windows))]
        let command = format!("cat >/dev/null; echo ran >> '{}'", marker.display());
        let hook =
            json!({"command": command, "shell": if cfg!(windows) { "powershell" } else { "bash" }});
        write(
            &root.join("hooks/hooks.json"),
            &json!({"hooks": {
                "Stop": [{"hooks": [hook.clone()]}],
                "PostToolUse": [{"hooks": [hook]}]
            }})
            .to_string(),
        );
        install_fixture(&agent_dir, &root, true);
        let loaded = active(&agent_dir);
        let input = HookInput {
            cwd: dir.path().to_path_buf(),
            ..Default::default()
        };
        loaded.run_event(HookEvent::Stop, None, &input);
        assert!(marker.exists(), "approved fixture must actually execute");
        std::fs::remove_file(&marker).unwrap();
        if change == "env-off" {
            std::env::set_var("DAVINCI_PLUGINS", "off");
        } else if change == "corrupt" {
            write(&agent_dir.join("plugins/installed.json"), "{broken");
        } else {
            store::update(&agent_dir, |file| {
                if change == "remove" {
                    file.plugins.remove("revocation-demo@local");
                } else {
                    let record = file.plugins.get_mut("revocation-demo@local").unwrap();
                    match change {
                        "revoke" => record.hooks_approved = None,
                        "disable" => record.enabled = false,
                        _ => panic!("unknown fixture change"),
                    }
                }
                Ok(())
            })
            .unwrap();
        }
        let stop_present = loaded.has_hooks(HookEvent::Stop);
        let post_present = loaded.has_matching_hook(HookEvent::PostToolUse, "Write");
        loaded.run_event(HookEvent::Stop, None, &input);
        loaded.run_event(HookEvent::PostToolUse, Some("Write"), &input);
        if change == "env-off" {
            std::env::remove_var("DAVINCI_PLUGINS");
        }
        assert!(
            !marker.exists(),
            "{change}: loaded hooks executed after revocation"
        );
        assert!(!stop_present, "{change}: loaded Stop remained approved");
        assert!(
            !post_present,
            "{change}: loaded PostToolUse remained approved"
        );
    }

    #[test]
    fn loaded_hooks_recheck_revoked_approval() {
        loaded_hook_revocation("revoke");
    }

    #[test]
    fn loaded_hooks_recheck_disabled_plugin() {
        loaded_hook_revocation("disable");
    }

    #[test]
    fn loaded_hooks_recheck_removed_plugin() {
        loaded_hook_revocation("remove");
    }

    #[test]
    fn loaded_hooks_recheck_environment_off() {
        loaded_hook_revocation("env-off");
    }

    #[test]
    fn audit_loaded_hooks_stop_when_the_registry_is_corrupt() {
        loaded_hook_revocation("corrupt");
    }

    #[test]
    fn loaded_hooks_refuse_direct_spawn_when_sandbox_boundary_is_enabled() {
        const CHILD_FLAG: &str = "DAVINCI_TEST_PLUGIN_SANDBOX_BOUNDARY";
        if std::env::var_os(CHILD_FLAG).is_none() {
            // The boundary is monotonic, so enable it only in an isolated test process.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "plugins::tests::loaded_hooks_refuse_direct_spawn_when_sandbox_boundary_is_enabled",
                    "--nocapture",
                ])
                .env(CHILD_FLAG, "1")
                .env_remove("PI_HOOKS_DRY_RUN")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "isolated fixture failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("boundary-demo");
        let marker = dir.path().join("hook-marker");
        write(&root.join("plugin.json"), r#"{"name":"boundary-demo"}"#);
        #[cfg(windows)]
        let command = format!(
            "$null = [Console]::In.ReadToEnd(); [IO.File]::AppendAllText('{}', 'ran')",
            marker.display()
        );
        #[cfg(not(windows))]
        let command = format!("cat >/dev/null; echo ran >> '{}'", marker.display());
        let hook =
            json!({"command": command, "shell": if cfg!(windows) { "powershell" } else { "bash" }});
        let hook_path = root.join("hooks/hooks.json");
        write(
            &hook_path,
            &json!({"hooks": {"Stop": [{"hooks": [hook.clone()]}]}}).to_string(),
        );
        install_fixture(&agent_dir, &root, true);
        let input = HookInput {
            cwd: dir.path().to_path_buf(),
            ..Default::default()
        };
        active(&agent_dir).run_event(HookEvent::Stop, None, &input);
        assert!(marker.exists(), "approved fixture must actually execute");
        std::fs::remove_file(&marker).unwrap();

        let mut async_hook = hook.clone();
        async_hook["async"] = json!(true);
        write(
            &hook_path,
            &json!({"hooks": {
                "Stop": [{"hooks": [hook.clone(), async_hook.clone()]}],
                "PostToolUse": [{"hooks": [hook, async_hook]}]
            }})
            .to_string(),
        );
        install_fixture(&agent_dir, &root, true);
        let loaded = active(&agent_dir);
        crate::execution_boundary::enable();
        for (event, subject) in [
            (HookEvent::Stop, None),
            (HookEvent::PostToolUse, Some("Write")),
        ] {
            let outcome = loaded.run_event(event, subject, &input);
            assert!(!marker.exists(), "{event:?} bypassed the sandbox boundary");
            assert_eq!(outcome.block, None);
            assert_eq!(outcome.warnings.len(), 2, "{event:?}: {outcome:?}");
            assert!(outcome.warnings.iter().all(|warning| {
                warning.contains("plugin hook process requires the sandbox executor transport")
            }));
        }
    }

    #[test]
    fn active_plugins_expose_resources_and_gate_hooks_on_approval() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("demo");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"demo"}"#,
        );
        write(&root.join("skills/s/SKILL.md"), "---\nname: s\n---\n");
        write(&root.join("commands/c.md"), "c");
        write(
            &root.join("agents/a.md"),
            "---\nname: plug-agent\ntools: Read, Glob, Edit\nmodel: sonnet\n---\nDo it.",
        );
        write(
            &root.join(".mcp.json"),
            r#"{"srv":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}}"#,
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"exit 2"}]}]}}"#,
        );
        install_fixture(&agent_dir, &root, false);

        let plugins = active(&agent_dir);
        assert_eq!(plugins.plugins.len(), 1);
        assert_eq!(plugins.skill_files().len(), 1);
        assert_eq!(plugins.command_files().len(), 1);
        let profiles = plugins.agent_profiles();
        assert_eq!(profiles[0].tools, vec!["read", "find", "edit"]);
        assert_eq!(profiles[0].model, "inherit");
        assert_eq!(profiles[0].permission_mode, "edits");
        let servers = plugins.mcp_servers();
        let server = &servers["plugin_demo_srv"];
        assert!(server.args[0].ends_with("server.js"));
        assert!(!server.args[0].contains("${"));

        // Unapproved: the hook never runs.
        assert!(!plugins.has_hooks(HookEvent::PreToolUse));
        assert!(!plugins.has_matching_hook(HookEvent::PreToolUse, "Bash"));

        // Approved: it runs; a changed hook file revokes it.
        install_fixture(&agent_dir, &root, true);
        let plugins = active(&agent_dir);
        assert!(plugins.plugins[0].hooks_approved());
        assert!(plugins.has_matching_hook(HookEvent::PreToolUse, "Bash"));
        assert!(!plugins.has_matching_hook(HookEvent::PreToolUse, "Read"));
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"exit 0"}]}]}}"#,
        );
        let plugins = active(&agent_dir);
        assert!(!plugins.plugins[0].hooks_approved());
    }

    #[test]
    fn env_switch_and_missing_adopted_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        store::update(&agent_dir, |file| {
            file.plugins.insert(
                "ghost@mk".into(),
                InstalledPlugin {
                    origin: Origin::Claude,
                    install_path: None,
                    version: None,
                    enabled: true,
                    hooks_approved: None,
                    installed_at: 0,
                },
            );
            Ok(())
        })
        .unwrap();
        let plugins = active(&agent_dir);
        assert!(plugins.plugins.is_empty());
        assert_eq!(plugins.errors.len(), 1);

        std::env::set_var("DAVINCI_PLUGINS", "off");
        let plugins = active(&agent_dir);
        std::env::remove_var("DAVINCI_PLUGINS");
        assert!(plugins.errors.is_empty());
    }

    #[test]
    fn load_failures_and_bad_mcp_entries_become_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("demo");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"demo"}"#,
        );
        write(
            &root.join(".mcp.json"),
            r#"{"good":{"command":"node"},"bad":{"command":42}}"#,
        );
        install_fixture(&agent_dir, &root, false);
        store::update(&agent_dir, |file| {
            file.plugins.insert(
                "ghost@mk".into(),
                InstalledPlugin {
                    origin: Origin::Claude,
                    install_path: None,
                    version: None,
                    enabled: true,
                    hooks_approved: None,
                    installed_at: 0,
                },
            );
            Ok(())
        })
        .unwrap();
        let plugins = active(&agent_dir);
        assert!(plugins.mcp_servers().contains_key("plugin_demo_good"));
        let diagnostics = plugins.diagnostics();
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(diagnostics[0].starts_with("plugin ghost@mk failed to load"));
        assert!(diagnostics[1].starts_with("plugin demo MCP server bad skipped"));
        let warnings = startup_warnings(&agent_dir);
        assert!(warnings.iter().any(|line| line.contains("ghost@mk")));
    }

    #[test]
    fn session_start_gets_the_session_id_reruns_on_change_and_reports_failures() {
        if hooks::find_bash().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        let root = dir.path().join("greeter");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"greeter"}"#,
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[
                {"type":"command","command":"sed 's/^/got: /'"},
                {"type":"command","command":"echo hook-broke >&2; exit 1"}
            ]}]}}"#,
        );
        // Installed but not approved: nothing runs.
        install_fixture(&agent_dir, &root, false);
        let output = session_start_context(&active(&agent_dir), &cwd, "sess-1");
        assert_eq!(output, SessionStartOutput::default());

        // Approved mid-session: the same session now gets its context.
        install_fixture(&agent_dir, &root, true);
        let output = session_start_context(&active(&agent_dir), &cwd, "sess-1");
        assert_eq!(output.contexts.len(), 1, "{output:?}");
        assert!(output.contexts[0].1.contains("\"session_id\":\"sess-1\""));
        assert!(output
            .warnings
            .iter()
            .any(|line| line.contains("hook-broke")));
        assert!(take_notices()
            .iter()
            .any(|line| line.contains("hook-broke")));

        // A new session runs the hooks again with its own id.
        let output = session_start_context(&active(&agent_dir), &cwd, "sess-2");
        assert!(output.contexts[0].1.contains("\"session_id\":\"sess-2\""));
    }

    #[test]
    fn joined_context_names_each_plugin() {
        let result = EventResult {
            contexts: vec![("a".into(), "one".into()), ("b".into(), "two".into())],
            ..EventResult::default()
        };
        let text = result.joined_context().unwrap();
        assert!(text.contains("plugin=\"a\"") && text.contains("two"));
        assert_eq!(EventResult::default().joined_context(), None);
    }
}
