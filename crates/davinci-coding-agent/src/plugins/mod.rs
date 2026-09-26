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
pub mod manifest;
pub mod marketplace;
pub mod store;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::agent_profiles::AgentProfile;
use hooks::{HookContext, HookEvent, HookOutcome, PluginHook};
use manifest::Plugin;
use store::{InstalledPlugin, Origin};

/// One enabled, loadable plugin.
#[derive(Debug, Clone)]
pub struct ActivePlugin {
    pub key: String,
    pub record: InstalledPlugin,
    pub plugin: Plugin,
    pub data_dir: PathBuf,
}

impl ActivePlugin {
    /// Hooks run only when the approved digest matches the hooks on disk.
    pub fn hooks_approved(&self) -> bool {
        self.plugin.hooks_digest.is_some() && self.record.hooks_approved == self.plugin.hooks_digest
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
    std::env::var_os("PI_GRAPH_ROLE").is_none()
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
    for (key, record) in store::load(agent_dir).plugins {
        if !record.enabled {
            continue;
        }
        let loaded = locate(&key, &record).and_then(|path| manifest::load_plugin(&path));
        match loaded {
            Ok(plugin) => {
                let data_dir = store::plugins_dir(agent_dir)
                    .join("data")
                    .join(store::split_key(&key).0);
                out.plugins.push(ActivePlugin {
                    key,
                    record,
                    plugin,
                    data_dir,
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
    /// expanded. Entries that do not parse are skipped.
    pub fn mcp_servers(&self) -> BTreeMap<String, davinci_mcp::ServerConfig> {
        let mut out = BTreeMap::new();
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
                if let Ok(parsed) = serde_json::from_value::<davinci_mcp::ServerConfig>(expanded) {
                    let name = format!("plugin_{}_{}", active.plugin.name, server);
                    out.insert(name, parsed);
                }
            }
        }
        out
    }

    /// Approved hooks for one event, with the plugin that owns each.
    fn hooks_for(&self, event: HookEvent) -> Vec<(&ActivePlugin, &PluginHook)> {
        if !hooks_allowed_here() {
            return Vec::new();
        }
        self.plugins
            .iter()
            .filter(|active| active.hooks_approved())
            .flat_map(|active| {
                active
                    .plugin
                    .hooks
                    .iter()
                    .filter(move |hook| hook.event == event)
                    .map(move |hook| (active, hook))
            })
            .collect()
    }

    pub fn has_hooks(&self, event: HookEvent) -> bool {
        !self.hooks_for(event).is_empty()
    }

    /// Run every matching hook for `event`. Contexts are collected in order;
    /// the first block stops the run.
    pub fn run_event(
        &self,
        event: HookEvent,
        subject: Option<&str>,
        input: &HookInput,
    ) -> EventResult {
        let mut result = EventResult::default();
        for (active, hook) in self.hooks_for(event) {
            let matched = match event {
                HookEvent::PreToolUse | HookEvent::PostToolUse | HookEvent::SessionStart => {
                    hooks::matcher_accepts(hook.matcher.as_deref(), subject.unwrap_or(""))
                }
                _ => true,
            };
            if !matched {
                continue;
            }
            let outcome: HookOutcome = hooks::run(
                hook,
                &active.hook_context(&input.cwd),
                &input.payload(event),
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
}

impl HookInput {
    fn payload(&self, event: HookEvent) -> Value {
        let mut value = json!({
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": self.cwd,
            "hook_event_name": event.as_str(),
        });
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

/// `SessionStart` output for this process and cwd, computed once: resource
/// discovery runs on every reload and prompt, a session starts once.
pub fn session_start_context(agent_dir: &Path, cwd: &Path, session_id: &str) -> PluginContexts {
    static CACHE: OnceLock<Mutex<BTreeMap<PathBuf, PluginContexts>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(found) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(cwd) {
        return found.clone();
    }
    let plugins = active(agent_dir);
    let contexts = if plugins.has_hooks(HookEvent::SessionStart) {
        let input = HookInput {
            session_id: session_id.to_string(),
            cwd: cwd.to_path_buf(),
            source: Some("startup".into()),
            ..HookInput::default()
        };
        plugins
            .run_event(HookEvent::SessionStart, Some("startup"), &input)
            .contexts
    } else {
        Vec::new()
    };
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(cwd.to_path_buf(), contexts.clone());
    contexts
}

/// `SessionEnd` hooks, run where the user's own `stop` hooks run.
pub fn run_session_end(agent_dir: &Path, cwd: &Path) {
    let plugins = active(agent_dir);
    if !plugins.has_hooks(HookEvent::SessionEnd) {
        return;
    }
    let input = HookInput {
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

        // Approved: it runs; a changed hook file revokes it.
        install_fixture(&agent_dir, &root, true);
        let plugins = active(&agent_dir);
        assert!(plugins.plugins[0].hooks_approved());
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
