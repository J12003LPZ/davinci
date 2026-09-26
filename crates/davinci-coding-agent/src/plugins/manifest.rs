//! Read one plugin directory in the Claude Code / Codex layout.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.
//!
//! The manifest is `.claude-plugin/plugin.json`, else
//! `.codex-plugin/plugin.json`, else `plugin.json`. Components come from the
//! default directories plus any extra paths the manifest names. Every path
//! must stay inside the plugin root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::hooks::{self, PluginHook};

pub const MANIFEST_PATHS: &[&str] = &[
    ".claude-plugin/plugin.json",
    ".codex-plugin/plugin.json",
    "plugin.json",
];

#[derive(Debug, Clone, Default)]
pub struct Plugin {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub root: PathBuf,
    /// Each `SKILL.md`, passed to skill discovery as a file root.
    pub skill_files: Vec<PathBuf>,
    pub command_files: Vec<PathBuf>,
    pub agent_files: Vec<PathBuf>,
    pub hooks: Vec<PluginHook>,
    /// Hook entries that will never run: unknown events or non-command types.
    pub unsupported_hooks: Vec<String>,
    /// SHA-256 over the hook configuration and the files it references.
    /// `None` when the plugin has no runnable hooks.
    pub hooks_digest: Option<String>,
    /// Raw MCP server objects, keyed by the plugin's own server name.
    pub mcp_servers: BTreeMap<String, Value>,
    pub warnings: Vec<String>,
}

impl Plugin {
    pub fn component_summary(&self) -> String {
        let mut parts = Vec::new();
        let mut push = |count: usize, label: &str| {
            if count > 0 {
                parts.push(format!(
                    "{count} {label}{}",
                    if count == 1 { "" } else { "s" }
                ));
            }
        };
        push(self.skill_files.len(), "skill");
        push(self.command_files.len(), "command");
        push(self.agent_files.len(), "agent");
        push(self.hooks.len(), "hook");
        push(self.mcp_servers.len(), "MCP server");
        if parts.is_empty() {
            "no components".into()
        } else {
            parts.join(", ")
        }
    }
}

pub fn read_manifest(root: &Path) -> Option<Value> {
    MANIFEST_PATHS.iter().find_map(|rel| {
        let body = fs::read_to_string(root.join(rel)).ok()?;
        serde_json::from_str(&body).ok()
    })
}

pub fn load_plugin(root: &Path) -> Result<Plugin, String> {
    if !root.is_dir() {
        return Err(format!("plugin directory not found: {}", root.display()));
    }
    let root = canonical(root).map_err(|err| format!("{}: {err}", root.display()))?;
    let manifest = read_manifest(&root).unwrap_or(Value::Null);
    let dir_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("plugin")
        .to_string();
    let mut plugin = Plugin {
        name: manifest
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or(dir_name),
        version: manifest
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string),
        description: manifest
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        root: root.clone(),
        ..Plugin::default()
    };

    for dir in component_paths(&root, &manifest, "skills", "skills", &mut plugin.warnings) {
        collect_skill_files(&dir, &mut plugin.skill_files);
    }
    for path in component_paths(
        &root,
        &manifest,
        "commands",
        "commands",
        &mut plugin.warnings,
    ) {
        collect_markdown(&path, 3, &mut plugin.command_files);
    }
    for path in component_paths(&root, &manifest, "agents", "agents", &mut plugin.warnings) {
        collect_markdown(&path, 2, &mut plugin.agent_files);
    }
    dedupe(&mut plugin.skill_files);
    dedupe(&mut plugin.command_files);
    dedupe(&mut plugin.agent_files);

    let mut hook_docs = Vec::new();
    if let Some(value) = manifest.get("hooks") {
        match value {
            Value::Object(_) => hook_docs.push(value.clone()),
            _ => {
                for path in manifest_paths(&root, value, &mut plugin.warnings) {
                    push_json_file(&path, &mut hook_docs, &mut plugin.warnings);
                }
            }
        }
    }
    let default_hooks = root.join("hooks").join("hooks.json");
    // `manifest_paths` drops an explicit `hooks/hooks.json`, so the default
    // file is never read twice.
    if default_hooks.is_file() {
        push_json_file(&default_hooks, &mut hook_docs, &mut plugin.warnings);
    }
    for doc in &hook_docs {
        hooks::parse_hook_document(doc, &mut plugin.hooks, &mut plugin.unsupported_hooks);
    }
    plugin.hooks_digest = hooks::digest_cached(&root, &plugin.hooks);

    let mut mcp_docs = Vec::new();
    if let Some(value) = manifest.get("mcpServers") {
        match value {
            Value::Object(_) => mcp_docs.push(value.clone()),
            _ => {
                for path in manifest_paths(&root, value, &mut plugin.warnings) {
                    push_json_file(&path, &mut mcp_docs, &mut plugin.warnings);
                }
            }
        }
    }
    let default_mcp = root.join(".mcp.json");
    if default_mcp.is_file() {
        push_json_file(&default_mcp, &mut mcp_docs, &mut plugin.warnings);
    }
    for doc in mcp_docs {
        let servers = doc.get("mcpServers").cloned().unwrap_or(doc);
        if let Value::Object(map) = servers {
            for (name, config) in map {
                if config.is_object() {
                    plugin.mcp_servers.entry(name).or_insert(config);
                }
            }
        }
    }
    Ok(plugin)
}

/// Default directory plus manifest-declared paths for one component kind.
fn component_paths(
    root: &Path,
    manifest: &Value,
    key: &str,
    default_dir: &str,
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let default = root.join(default_dir);
    if default.exists() {
        out.push(default);
    }
    if let Some(value) = manifest.get(key) {
        out.extend(manifest_paths(root, value, warnings));
    }
    out
}

/// Resolve a manifest string or string array to paths inside `root`.
fn manifest_paths(root: &Path, value: &Value, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let items: Vec<&str> = match value {
        Value::String(item) => vec![item.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for item in items {
        match resolve_inside(root, item) {
            Some(path) => {
                // The default hooks file is always read separately.
                if path != root.join("hooks").join("hooks.json") {
                    out.push(path);
                }
            }
            None => warnings.push(format!("ignored path outside the plugin: {item}")),
        }
    }
    out
}

/// `rel` joined to `root`, or `None` when it is absolute, missing, or
/// escapes the root (including through a symlink).
pub fn resolve_inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    let rel = rel
        .strip_prefix("${CLAUDE_PLUGIN_ROOT}/")
        .or_else(|| rel.strip_prefix("${DAVINCI_PLUGIN_ROOT}/"))
        .unwrap_or(rel);
    if rel.is_empty() || Path::new(rel).is_absolute() || rel.contains(':') {
        return None;
    }
    let resolved = canonical(&root.join(rel)).ok()?;
    let canonical_root = canonical(root).ok()?;
    resolved.starts_with(&canonical_root).then_some(resolved)
}

/// `canonicalize` without the Windows verbatim prefix. `\\?\C:\x` is a
/// valid path for Rust but not for Git Bash, cmd or most hook scripts, and
/// plugin roots end up in hook commands and environment variables.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let resolved = path.canonicalize()?;
    let text = resolved.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{rest}")));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        if rest.as_bytes().get(1) == Some(&b':') {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(resolved)
}

fn collect_skill_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        if dir.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            out.push(dir.to_path_buf());
        }
        return;
    }
    let direct = dir.join("SKILL.md");
    if direct.is_file() {
        out.push(direct);
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join("SKILL.md"))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    out.extend(found);
}

fn collect_markdown(path: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path.to_path_buf());
        }
        return;
    }
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(path)
        .max_depth(depth)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("md")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
        })
        .collect();
    found.sort();
    out.extend(found);
}

fn push_json_file(path: &Path, docs: &mut Vec<Value>, warnings: &mut Vec<String>) {
    match fs::read_to_string(path)
        .map_err(|err| err.to_string())
        .and_then(|body| serde_json::from_str::<Value>(&body).map_err(|err| err.to_string()))
    {
        Ok(value) => docs.push(value),
        Err(err) => warnings.push(format!("{}: {err}", path.display())),
    }
}

fn dedupe(paths: &mut Vec<PathBuf>) {
    let mut seen = std::collections::BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn loads_default_components_and_ignores_reference_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"demo","version":"1.2.0","description":"Demo"}"#,
        );
        write(
            &root.join("skills/alpha/SKILL.md"),
            "---\nname: alpha\n---\nbody",
        );
        write(&root.join("skills/alpha/reference.md"), "not a skill");
        write(&root.join("commands/go.md"), "Go $ARGUMENTS");
        write(&root.join("commands/README.md"), "docs");
        write(
            &root.join("agents/reviewer.md"),
            "---\nname: reviewer\n---\nReview.",
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
        );
        write(
            &root.join(".mcp.json"),
            r#"{"mcpServers":{"srv":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/s.js"]}}}"#,
        );

        let plugin = load_plugin(&root).unwrap();
        assert_eq!(plugin.name, "demo");
        assert_eq!(plugin.version.as_deref(), Some("1.2.0"));
        assert_eq!(plugin.skill_files.len(), 1);
        assert!(plugin.skill_files[0].ends_with("SKILL.md"));
        assert_eq!(plugin.command_files.len(), 1);
        assert_eq!(plugin.agent_files.len(), 1);
        assert_eq!(plugin.hooks.len(), 1);
        assert!(plugin.hooks_digest.is_some());
        assert!(plugin.mcp_servers.contains_key("srv"));
        assert_eq!(
            plugin.component_summary(),
            "1 skill, 1 command, 1 agent, 1 hook, 1 MCP server"
        );
    }

    #[test]
    fn codex_manifest_and_missing_manifest_are_supported() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex-one");
        write(
            &codex.join(".codex-plugin/plugin.json"),
            r#"{"name":"cx","skills":"./my-skills/"}"#,
        );
        write(
            &codex.join("my-skills/beta/SKILL.md"),
            "---\nname: beta\n---\n",
        );
        let plugin = load_plugin(&codex).unwrap();
        assert_eq!(plugin.name, "cx");
        assert_eq!(plugin.skill_files.len(), 1);

        let bare = dir.path().join("bare-plugin");
        write(&bare.join("commands/x.md"), "x");
        let plugin = load_plugin(&bare).unwrap();
        assert_eq!(plugin.name, "bare-plugin");
        assert_eq!(plugin.command_files.len(), 1);
    }

    #[test]
    fn manifest_paths_outside_the_root_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        write(&dir.path().join("outside/SKILL.md"), "secret");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"p","skills":["../outside","/etc"],"commands":"C:/x"}"#,
        );
        let plugin = load_plugin(&root).unwrap();
        assert!(plugin.skill_files.is_empty());
        assert_eq!(plugin.warnings.len(), 3, "{:?}", plugin.warnings);
    }

    #[test]
    fn inline_hooks_and_mcp_servers_in_the_manifest_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inline");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{
                "name":"inline",
                "hooks":{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}],
                         "Notification":[{"hooks":[{"type":"command","command":"x"}]}]}},
                "mcpServers":{"one":{"url":"https://example.invalid/mcp"}}
            }"#,
        );
        let plugin = load_plugin(&root).unwrap();
        assert_eq!(plugin.hooks.len(), 1);
        assert_eq!(plugin.unsupported_hooks.len(), 1);
        assert!(plugin.mcp_servers.contains_key("one"));
    }
}
