//! The data and actions behind the `/plugin` manager sheet: installed
//! plugins, discovered skills and configured MCP servers, one row each, and
//! what a row's keys do.
//!
//! No TypeScript counterpart. Every action re-reads the current state and
//! refuses a key the row no longer allows, so a stale sheet cannot remove
//! something it no longer lists. Skills are moved to `<agent_dir>/trash/`,
//! never deleted outright.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::store::{self, Origin};
use super::{command, manifest};

/// How a row reads at a glance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Health {
    #[default]
    Ok,
    Off,
    Attention,
    Failed,
}

/// One manageable item. The `can_*` flags are the actions the host accepts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManagedRow {
    pub key: String,
    pub title: String,
    pub status: String,
    pub health: Health,
    pub detail: String,
    pub note: Option<String>,
    pub can_update: bool,
    pub can_toggle: bool,
    pub can_approve: bool,
    pub can_revoke: bool,
    pub can_delete: bool,
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

// --- plugins ---------------------------------------------------------------

pub fn plugin_rows(agent_dir: &Path) -> Vec<ManagedRow> {
    let file = store::load(agent_dir);
    let off_by_env = super::disabled_by_env();
    file.plugins
        .iter()
        .map(|(key, record)| {
            let mut row = ManagedRow {
                key: key.clone(),
                title: key.clone(),
                status: if record.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
                .into(),
                health: if record.enabled {
                    Health::Ok
                } else {
                    Health::Off
                },
                can_update: record.origin == Origin::Davinci,
                can_toggle: true,
                can_delete: true,
                ..ManagedRow::default()
            };
            let mut notes = Vec::new();
            if off_by_env {
                notes.push("DAVINCI_PLUGINS=off: nothing loads in this environment.".to_string());
            }
            if record.origin != Origin::Davinci {
                notes.push(format!(
                    "Adopted from {0}: update it in {0}. Delete only stops DaVinci loading it.",
                    record.origin.label()
                ));
            }
            match super::locate(key, record).and_then(|path| manifest::load_plugin(&path)) {
                Ok(plugin) => {
                    let hooks = match &plugin.hooks_digest {
                        None => "no hooks",
                        Some(digest) if record.hooks_approved.as_ref() == Some(digest) => {
                            "hooks approved"
                        }
                        Some(_) if record.hooks_approved.is_some() => {
                            "hooks changed, approval needed"
                        }
                        Some(_) => "hooks need approval",
                    };
                    let needs_approval = plugin.hooks_digest.is_some()
                        && record.hooks_approved != plugin.hooks_digest;
                    row.can_approve = needs_approval;
                    row.can_revoke = record.hooks_approved.is_some();
                    if needs_approval && record.enabled {
                        row.health = Health::Attention;
                    }
                    row.detail = [
                        format!("from {}", record.origin.label()),
                        plugin
                            .version
                            .as_deref()
                            .map(|v| format!("v{v}"))
                            .unwrap_or_default(),
                        plugin.component_summary(),
                        hooks.to_string(),
                    ]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                    notes.extend(plugin.warnings.iter().map(|w| format!("warning: {w}")));
                }
                Err(err) => {
                    row.health = Health::Failed;
                    row.detail = format!("from {}", record.origin.label());
                    notes.push(format!("unavailable: {err}"));
                }
            }
            if !notes.is_empty() {
                row.note = Some(notes.join("\n"));
            }
            row
        })
        .collect()
}

pub fn plugin_action(
    agent_dir: &Path,
    cwd: &Path,
    action: &str,
    key: &str,
) -> Result<String, String> {
    let row = plugin_rows(agent_dir)
        .into_iter()
        .find(|row| row.key == key)
        .ok_or_else(|| format!("{key} is no longer installed"))?;
    let words: &[&str] = match action {
        "info" => &["info", key],
        "update" if row.can_update => &["update", key],
        "toggle" if row.status == "disabled" => &["enable", key],
        "toggle" => &["disable", key],
        "approve" if row.can_approve => &["approve", key],
        "revoke" if row.can_revoke => &["revoke", key],
        "delete" => &["uninstall", key],
        _ => return Err(format!("{action} does not apply to {key}")),
    };
    command::run(&args(words), agent_dir, cwd)
}

// --- skills ----------------------------------------------------------------

fn canonical_or_same(path: &Path) -> PathBuf {
    manifest::canonical(path).unwrap_or_else(|_| path.to_path_buf())
}

/// What a skill delete moves: its directory for `<name>/SKILL.md`, else the
/// file itself. Never a removable root.
fn skill_target(path: &Path, roots: &[PathBuf]) -> PathBuf {
    let is_skill_md = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"));
    match path.parent() {
        Some(parent) if is_skill_md && !roots.iter().any(|root| root == parent) => {
            parent.to_path_buf()
        }
        _ => path.to_path_buf(),
    }
}

/// The removable root a skill sits in, by its own path first so a symlinked
/// skill file counts where it is listed, then by its resolved path.
fn removable_root<'a>(path: &Path, raw: &'a [PathBuf], resolved: &'a [PathBuf]) -> Option<usize> {
    raw.iter()
        .position(|root| path.starts_with(root))
        .or_else(|| {
            let path = canonical_or_same(path);
            resolved.iter().position(|root| path.starts_with(root))
        })
}

/// `skills` are the skills the session discovered; `removable_roots` are the
/// user and project skill directories whose contents the manager may move.
pub fn skill_rows(
    skills: &[davinci_agent::Skill],
    agent_dir: &Path,
    removable_roots: &[PathBuf],
) -> Vec<ManagedRow> {
    let plugin_roots: Vec<(String, PathBuf)> = super::active(agent_dir)
        .plugins
        .iter()
        .map(|active| (active.key.clone(), canonical_or_same(&active.plugin.root)))
        .collect();
    let resolved: Vec<PathBuf> = removable_roots
        .iter()
        .map(|r| canonical_or_same(r))
        .collect();
    let agent_dir = canonical_or_same(agent_dir);
    let mut rows: Vec<ManagedRow> = skills
        .iter()
        .map(|skill| {
            let path = canonical_or_same(&skill.path);
            let mut row = ManagedRow {
                key: skill.path.display().to_string(),
                title: skill.name.clone(),
                detail: skill.description.clone(),
                ..ManagedRow::default()
            };
            if let Some((plugin, _)) = plugin_roots.iter().find(|(_, root)| path.starts_with(root))
            {
                row.status = "plugin".into();
                row.note = Some(format!(
                    "From plugin {plugin}. Manage it in the Plugins tab."
                ));
            } else if let Some(index) = removable_root(&skill.path, removable_roots, &resolved) {
                row.status = if resolved[index].starts_with(&agent_dir) {
                    "user"
                } else {
                    "project"
                }
                .into();
                let target = skill_target(&skill.path, removable_roots);
                // Moving a folder that other listed skills live in would take
                // them along without saying so.
                let shared = skills
                    .iter()
                    .any(|other| other.path != skill.path && other.path.starts_with(&target));
                if shared {
                    row.note = Some(format!(
                        "{} also holds other skills; remove them from disk yourself.",
                        target.display()
                    ));
                } else {
                    row.can_delete = true;
                    row.note = Some(target.display().to_string());
                }
            } else {
                row.status = "other".into();
                row.note = Some(format!(
                    "{} (added by a setting or package; remove it there)",
                    skill.path.display()
                ));
            }
            row
        })
        .collect();
    rows.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    rows
}

/// Move a user or project skill to `<agent_dir>/trash/skills/`. Only a
/// rename: if it fails (another drive, a file in use) nothing is touched.
pub fn skill_action(
    skills: &[davinci_agent::Skill],
    agent_dir: &Path,
    removable_roots: &[PathBuf],
    action: &str,
    key: &str,
) -> Result<String, String> {
    let rows = skill_rows(skills, agent_dir, removable_roots);
    let row = rows
        .iter()
        .find(|row| row.key == key)
        .ok_or_else(|| "that skill is no longer loaded".to_string())?;
    match action {
        "info" => Ok(format!(
            "{} ({})\n{}\n{}",
            row.title,
            row.status,
            row.detail,
            row.note.as_deref().unwrap_or(key)
        )),
        "delete" if row.can_delete => {
            let target = skill_target(Path::new(key), removable_roots);
            if !removable_roots
                .iter()
                .any(|root| target.starts_with(root) && &target != root)
            {
                return Err(format!("refusing to move {}", target.display()));
            }
            let trash = agent_dir.join("trash").join("skills");
            std::fs::create_dir_all(&trash).map_err(|err| err.to_string())?;
            let name = target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill");
            let dest = trash.join(format!("{name}-{}", store::now_ms()));
            std::fs::rename(&target, &dest).map_err(|err| {
                format!(
                    "could not move {} to the trash ({err}); nothing was removed. \
                     Close programs using it, or delete it yourself if it is on another drive.",
                    target.display()
                )
            })?;
            Ok(format!(
                "Moved skill {} to {}. Move it back to restore it.",
                row.title,
                dest.display()
            ))
        }
        _ => Err(format!("{action} does not apply to skill {}", row.title)),
    }
}

// --- MCP servers -----------------------------------------------------------

/// The `mcp.json` files the session reads, in increasing precedence.
#[derive(Debug, Clone, Default)]
pub struct McpFiles {
    pub user: PathBuf,
    /// A trusted project's file; `None` when untrusted or absent.
    pub project: Option<PathBuf>,
    /// Whether enabled plugins' servers load: not under `DAVINCI_MCP_CONFIG`.
    pub include_plugins: bool,
    /// `--no-mcp`: this session starts no server at all.
    pub session_off: bool,
}

/// What the running session knows about a server.
#[derive(Debug, Clone, Default)]
pub struct LiveServer {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub error: Option<String>,
}

enum McpSource {
    Plugin(String),
    File(PathBuf),
}

fn read_servers(path: &Path) -> BTreeMap<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|doc| doc.get("mcpServers").and_then(Value::as_object).cloned())
        .map(|map| map.into_iter().collect())
        .unwrap_or_default()
}

/// Every configured server with the source that wins, as `mcp.rs` merges.
fn mcp_sources(agent_dir: &Path, files: &McpFiles) -> BTreeMap<String, (McpSource, Value)> {
    let mut out = BTreeMap::new();
    let plugins = if files.include_plugins {
        super::active(agent_dir).plugins
    } else {
        Vec::new()
    };
    for active in plugins {
        for (server, config) in &active.plugin.mcp_servers {
            // `ActivePlugins::mcp_servers` skips entries that do not parse.
            if serde_json::from_value::<davinci_mcp::ServerConfig>(config.clone()).is_err() {
                continue;
            }
            out.insert(
                format!("plugin_{}_{server}", active.plugin.name),
                (McpSource::Plugin(active.key.clone()), config.clone()),
            );
        }
    }
    for path in std::iter::once(&files.user).chain(files.project.iter()) {
        for (name, config) in read_servers(path) {
            out.insert(name, (McpSource::File(path.clone()), config));
        }
    }
    out
}

fn describe_server(config: &Value) -> String {
    if let Some(url) = config.get("url").and_then(Value::as_str) {
        return format!("http · {url}");
    }
    let command = config.get("command").and_then(Value::as_str).unwrap_or("?");
    let args = config
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    format!("stdio · {command} {args}").trim_end().to_string()
}

pub fn mcp_rows(agent_dir: &Path, files: &McpFiles, live: &[LiveServer]) -> Vec<ManagedRow> {
    mcp_sources(agent_dir, files)
        .into_iter()
        .map(|(name, (source, config))| {
            let disabled = config
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // The file decides enabled/disabled; the session only says whether
            // the server is running now.
            let running = live
                .iter()
                .find(|server| server.name == name && server.status != "disabled");
            let (status, health) = match running {
                _ if disabled => ("disabled".to_string(), Health::Off),
                Some(server) if server.status == "connected" => {
                    (format!("connected · {} tools", server.tools), Health::Ok)
                }
                Some(server) => (server.status.clone(), Health::Failed),
                None => ("enabled · not running".to_string(), Health::Attention),
            };
            let mut notes = Vec::new();
            if let Some(error) = running.and_then(|server| server.error.clone()) {
                notes.push(error);
            }
            let (from, editable) = match &source {
                McpSource::Plugin(plugin) => {
                    notes.push(format!(
                        "From plugin {plugin}. Manage it in the Plugins tab."
                    ));
                    (format!("plugin {plugin}"), false)
                }
                McpSource::File(path) => (path.display().to_string(), true),
            };
            if running.is_none() && !disabled {
                notes.push(if files.session_off {
                    "MCP is off in this session (--no-mcp).".into()
                } else {
                    "Starts with the next DaVinci session.".to_string()
                });
            }
            ManagedRow {
                key: name.clone(),
                title: name,
                status,
                health,
                detail: format!("{} · {from}", describe_server(&config)),
                note: (!notes.is_empty()).then(|| notes.join("\n")),
                can_toggle: editable,
                can_delete: editable,
                ..ManagedRow::default()
            }
        })
        .collect()
}

fn edit_servers(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), String>,
) -> Result<(), String> {
    let body = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut doc: Value =
        serde_json::from_str(&body).map_err(|err| format!("{}: {err}", path.display()))?;
    let servers = doc
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("{} has no mcpServers", path.display()))?;
    edit(servers)?;
    // Write through a symlink to the real file, and keep its permissions:
    // `mcp.json` often holds secrets and may be linked from a dotfiles repo.
    let real = manifest::canonical(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let permissions = std::fs::metadata(&real)
        .map_err(|err| format!("{}: {err}", real.display()))?
        .permissions();
    store::write_json_atomic(&real, &doc)?;
    std::fs::set_permissions(&real, permissions).map_err(|err| format!("{}: {err}", real.display()))
}

const MCP_RESTART: &str = "MCP servers change when the next DaVinci session starts.";

pub fn mcp_action(
    agent_dir: &Path,
    files: &McpFiles,
    action: &str,
    name: &str,
) -> Result<String, String> {
    let mut sources = mcp_sources(agent_dir, files);
    let (source, config) = sources
        .remove(name)
        .ok_or_else(|| format!("{name} is no longer configured"))?;
    if action == "info" {
        return Ok(format!(
            "{name}\n{}",
            serde_json::to_string_pretty(&config).unwrap_or_default()
        ));
    }
    let McpSource::File(path) = source else {
        return Err(format!(
            "{name} comes from a plugin; disable or delete the plugin instead"
        ));
    };
    match action {
        "toggle" => {
            let mut now_disabled = false;
            edit_servers(&path, |servers| {
                let entry = servers
                    .get_mut(name)
                    .and_then(Value::as_object_mut)
                    .ok_or("server disappeared")?;
                now_disabled = !entry
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if now_disabled {
                    entry.insert("disabled".into(), Value::Bool(true));
                } else {
                    entry.remove("disabled");
                }
                Ok(())
            })?;
            Ok(format!(
                "{} {name} in {}. {MCP_RESTART}",
                if now_disabled { "Disabled" } else { "Enabled" },
                path.display()
            ))
        }
        "delete" => {
            edit_servers(&path, |servers| {
                servers
                    .remove(name)
                    .map(|_| ())
                    .ok_or_else(|| "server disappeared".into())
            })?;
            Ok(format!(
                "Removed {name} from {}. {MCP_RESTART}",
                path.display()
            ))
        }
        _ => Err(format!("{action} does not apply to {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::external::tests::{write, FakeHomes};

    fn skill(path: &Path, name: &str) -> davinci_agent::Skill {
        davinci_agent::Skill {
            name: name.into(),
            path: path.to_path_buf(),
            description: format!("{name} description"),
            body: String::new(),
            base_dir: path.parent().unwrap().to_path_buf(),
        }
    }

    #[test]
    fn plugin_rows_offer_the_actions_each_plugin_allows() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = dir.path().join("demo");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"demo","version":"1.2.0"}"#,
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"true"}]}]}}"#,
        );
        store::update(&agent_dir, |file| {
            file.plugins.insert(
                "demo@local".into(),
                store::InstalledPlugin {
                    origin: Origin::Davinci,
                    install_path: Some(root.clone()),
                    version: None,
                    enabled: true,
                    hooks_approved: None,
                    installed_at: 0,
                },
            );
            Ok(())
        })
        .unwrap();

        let rows = plugin_rows(&agent_dir);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.status, "enabled");
        assert_eq!(row.health, Health::Attention);
        assert!(row.detail.contains("v1.2.0") && row.detail.contains("hooks need approval"));
        assert!(row.can_update && row.can_toggle && row.can_approve && row.can_delete);
        assert!(!row.can_revoke);

        plugin_action(&agent_dir, dir.path(), "approve", "demo@local").unwrap();
        let row = &plugin_rows(&agent_dir)[0];
        assert!(row.can_revoke && !row.can_approve);
        assert_eq!(row.health, Health::Ok);

        plugin_action(&agent_dir, dir.path(), "toggle", "demo@local").unwrap();
        assert_eq!(plugin_rows(&agent_dir)[0].status, "disabled");
        assert!(plugin_action(&agent_dir, dir.path(), "approve", "demo@local").is_err());
        plugin_action(&agent_dir, dir.path(), "delete", "demo@local").unwrap();
        assert!(plugin_rows(&agent_dir).is_empty());
        assert!(plugin_action(&agent_dir, dir.path(), "info", "demo@local").is_err());
    }

    #[test]
    fn user_skills_move_to_trash_and_other_skills_stay() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let user_root = agent_dir.join("skills");
        let mine = user_root.join("mine/SKILL.md");
        let extra = dir.path().join("elsewhere/other/SKILL.md");
        write(&mine, "---\nname: mine\n---\n");
        write(&user_root.join("mine/notes.md"), "kept with the skill");
        write(&extra, "---\nname: other\n---\n");
        let skills = vec![skill(&mine, "mine"), skill(&extra, "other")];
        let roots = vec![user_root.clone()];

        let rows = skill_rows(&skills, &agent_dir, &roots);
        let by = |name: &str| rows.iter().find(|row| row.title == name).unwrap().clone();
        assert!(by("mine").can_delete);
        assert_eq!(by("mine").status, "user");
        assert!(!by("other").can_delete);
        assert!(skill_action(&skills, &agent_dir, &roots, "delete", &by("other").key).is_err());
        assert!(extra.exists());

        let out = skill_action(&skills, &agent_dir, &roots, "delete", &by("mine").key).unwrap();
        assert!(out.contains("Moved skill mine"));
        assert!(!user_root.join("mine").exists());
        assert!(user_root.exists(), "the root itself is never moved");
        let trashed: Vec<_> = std::fs::read_dir(agent_dir.join("trash/skills"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(trashed.len(), 1);
        assert!(trashed[0].path().join("notes.md").exists());
    }

    #[test]
    fn a_skill_folder_holding_other_skills_is_not_deletable() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let root = agent_dir.join("skills");
        let pack = root.join("pack/SKILL.md");
        let inner = root.join("pack/sub/SKILL.md");
        let lone = root.join("lone.md");
        write(&pack, "---\nname: pack\n---\n");
        write(&inner, "---\nname: inner\n---\n");
        write(&lone, "---\nname: lone\n---\n");
        let skills = vec![
            skill(&pack, "pack"),
            skill(&inner, "inner"),
            skill(&lone, "lone"),
        ];
        let roots = vec![root.clone()];
        let rows = skill_rows(&skills, &agent_dir, &roots);
        let by = |name: &str| rows.iter().find(|row| row.title == name).unwrap().clone();
        assert!(!by("pack").can_delete);
        assert!(by("pack").note.unwrap().contains("also holds other skills"));
        assert!(skill_action(&skills, &agent_dir, &roots, "delete", &by("pack").key).is_err());
        assert!(inner.exists());
        // The nested skill and a flat file skill move on their own.
        assert!(by("inner").can_delete);
        skill_action(&skills, &agent_dir, &roots, "delete", &by("lone").key).unwrap();
        assert!(!lone.exists() && root.exists());
    }

    #[test]
    fn mcp_rows_merge_sources_and_edit_only_their_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let user = agent_dir.join("mcp.json");
        let project = dir.path().join("proj/.davinci/mcp.json");
        write(
            &user,
            r#"{"mcpServers":{"docs":{"url":"https://example.com/mcp"},"shared":{"command":"user"}},"other":1}"#,
        );
        write(
            &project,
            r#"{"mcpServers":{"shared":{"command":"project","args":["--x"]}}}"#,
        );
        let files = McpFiles {
            user: user.clone(),
            project: Some(project.clone()),
            include_plugins: true,
            session_off: false,
        };
        let mut live = vec![LiveServer {
            name: "docs".into(),
            status: "connected".into(),
            tools: 3,
            error: None,
        }];
        let rows = mcp_rows(&agent_dir, &files, &live);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, "connected · 3 tools");
        assert!(rows[1].detail.contains("project --x"), "{}", rows[1].detail);
        assert!(rows[1].detail.contains(&project.display().to_string()));

        mcp_action(&agent_dir, &files, "toggle", "docs").unwrap();
        let rows = mcp_rows(&agent_dir, &files, &live);
        assert_eq!(rows[0].status, "disabled");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["docs"]["disabled"], true);
        assert_eq!(doc["other"], 1, "unrelated keys survive");
        mcp_action(&agent_dir, &files, "toggle", "docs").unwrap();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
        assert!(doc["mcpServers"]["docs"].get("disabled").is_none());

        // Disabled when the session started, enabled since: the file wins,
        // so the row offers `disable`, not a second `enable`.
        live[0].status = "disabled".into();
        let rows = mcp_rows(&agent_dir, &files, &live);
        assert_eq!(rows[0].status, "enabled · not running");
        assert!(rows[0]
            .note
            .as_deref()
            .unwrap()
            .contains("next DaVinci session"));
        let off = McpFiles {
            session_off: true,
            ..files.clone()
        };
        assert!(mcp_rows(&agent_dir, &off, &live)[0]
            .note
            .as_deref()
            .unwrap()
            .contains("--no-mcp"));

        // `shared` is the project's entry; deleting it leaves the user's.
        mcp_action(&agent_dir, &files, "delete", "shared").unwrap();
        let rows = mcp_rows(&agent_dir, &files, &live);
        let shared = rows.iter().find(|row| row.key == "shared").unwrap();
        assert!(shared.detail.contains("user"));
        assert!(mcp_action(&agent_dir, &files, "delete", "missing").is_err());
    }
}
