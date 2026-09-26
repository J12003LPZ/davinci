//! `davinci plugin …` and `/plugin …`.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use super::external;
use super::manifest::{self, Plugin};
use super::marketplace;
use super::store::{self, InstalledPlugin, Origin};

pub const USAGE: &str = "\
plugin [list]                          installed plugins
plugin browse [query]                  plugins in every known marketplace
plugin install <name>[@marketplace]    install and enable
plugin import [claude|codex] [<key>|--all]
                                       list or adopt Claude Code / Codex plugins
plugin info <plugin>                   components, hooks and warnings
plugin approve <plugin>                allow the plugin's listed hooks to run
plugin revoke <plugin>                 stop its hooks
plugin test <plugin>                   run its approved SessionStart hooks once
plugin enable|disable <plugin>
plugin update <plugin>                 reinstall from its marketplace
plugin uninstall <plugin>
plugin marketplace add <owner/repo | git-url | directory>
plugin marketplace list | update [name] | remove <name>

Changes apply to new sessions, or after /reload.";

const RELOAD_HINT: &str = "Start a new session or run /reload to load it.";

/// Run one `plugin` subcommand and return the text to show.
pub fn run(args: &[String], agent_dir: &Path, cwd: &Path) -> Result<String, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["list"] | ["ls"] => Ok(list(agent_dir)),
        ["help"] | ["--help"] | ["-h"] => Ok(USAGE.to_string()),
        ["browse", rest @ ..] | ["search", rest @ ..] => Ok(browse(agent_dir, &rest.join(" "))),
        ["install", target] | ["add", target] => install(agent_dir, target),
        ["import"] => Ok(importable(None, agent_dir)),
        ["import", tool @ ("claude" | "codex")] => Ok(importable(Some(tool), agent_dir)),
        ["import", "--all"] => import_all(None, agent_dir),
        ["import", tool @ ("claude" | "codex"), "--all"] => import_all(Some(tool), agent_dir),
        ["import", tool @ ("claude" | "codex"), key] => import_one(Some(tool), key, agent_dir),
        ["import", key] => import_one(None, key, agent_dir),
        ["info", target] | ["show", target] => info(agent_dir, target),
        ["approve", target] => approve(agent_dir, target),
        ["revoke", target] => revoke(agent_dir, target),
        ["test", target] => test_hooks(agent_dir, target, cwd),
        ["enable", target] => set_enabled(agent_dir, target, true),
        ["disable", target] => set_enabled(agent_dir, target, false),
        ["update", target] => update(agent_dir, target),
        ["uninstall", target] | ["remove", target] => uninstall(agent_dir, target),
        ["marketplace"] | ["marketplace", "list"] => Ok(marketplaces(agent_dir)),
        ["marketplace", "add", source] => {
            let name = marketplace::add(agent_dir, source, cwd)?;
            Ok(format!(
                "Added marketplace {name}. Browse it with `plugin browse`."
            ))
        }
        ["marketplace", "update"] => Ok(marketplace::update(agent_dir, None)?.join("\n")),
        ["marketplace", "update", name] => {
            Ok(marketplace::update(agent_dir, Some(name))?.join("\n"))
        }
        ["marketplace", "remove", name] => {
            marketplace::remove(agent_dir, name)?;
            Ok(format!("Removed marketplace {name}."))
        }
        _ => Err(format!("unknown plugin command\n\n{USAGE}")),
    }
}

fn hook_state(record: &InstalledPlugin, plugin: &Plugin) -> &'static str {
    match &plugin.hooks_digest {
        None => "none",
        Some(digest) if record.hooks_approved.as_ref() == Some(digest) => "approved",
        Some(_) if record.hooks_approved.is_some() => "changed, approval needed",
        Some(_) => "approval needed",
    }
}

fn list(agent_dir: &Path) -> String {
    let file = store::load(agent_dir);
    let mut out = String::new();
    if super::disabled_by_env() {
        out.push_str("DAVINCI_PLUGINS=off: no plugin is loaded in this environment.\n\n");
    }
    if file.plugins.is_empty() {
        out.push_str("No plugins installed.\n");
    } else {
        out.push_str("Installed plugins:\n");
        for (key, record) in &file.plugins {
            let state = if record.enabled {
                "enabled"
            } else {
                "disabled"
            };
            match super::locate(key, record).and_then(|path| manifest::load_plugin(&path)) {
                Ok(plugin) => {
                    let _ = writeln!(
                        out,
                        "  {key}  [{state}, from {}]  {}{}  hooks: {}",
                        record.origin.label(),
                        plugin.component_summary(),
                        plugin
                            .version
                            .as_deref()
                            .map(|v| format!("  v{v}"))
                            .unwrap_or_default(),
                        hook_state(record, &plugin)
                    );
                }
                Err(err) => {
                    let _ = writeln!(
                        out,
                        "  {key}  [{state}, from {}]  unavailable: {err}",
                        record.origin.label()
                    );
                }
            }
        }
    }
    let importable = external::all_plugins()
        .into_iter()
        .filter(|found| !file.plugins.contains_key(&found.key))
        .count();
    if importable > 0 {
        let _ = write!(
            out,
            "\n{importable} plugin(s) from Claude Code / Codex can be adopted: `plugin import`."
        );
    }
    let _ = write!(out, "\nRun `plugin help` for all commands.");
    out
}

fn browse(agent_dir: &Path, query: &str) -> String {
    let catalogs = marketplace::catalogs(agent_dir);
    if catalogs.is_empty() {
        return "No marketplaces known. Add one with `plugin marketplace add <owner/repo>`, \
                for example `plugin marketplace add anthropics/claude-plugins-official`."
            .into();
    }
    let installed = store::load(agent_dir);
    let query = query.trim().to_ascii_lowercase();
    let mut out = String::new();
    for catalog in catalogs {
        let mut rows = Vec::new();
        for entry in catalog.entries() {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("");
            let description = entry
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !query.is_empty()
                && !name.to_ascii_lowercase().contains(&query)
                && !description.to_ascii_lowercase().contains(&query)
            {
                continue;
            }
            let key = format!("{name}@{}", catalog.name);
            let mark = if installed.plugins.contains_key(&key) {
                "*"
            } else {
                " "
            };
            rows.push(format!("  {mark} {key}  {}", first_line(description, 90)));
        }
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(out, "{} (via {}):", catalog.name, catalog.from);
        for row in rows {
            let _ = writeln!(out, "{row}");
        }
    }
    if out.is_empty() {
        return format!("No plugin matches {query:?}.");
    }
    out.push_str("\n* installed. Install with `plugin install <name>@<marketplace>`.");
    out
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() <= max {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(max).collect::<String>())
    }
}

fn install(agent_dir: &Path, target: &str) -> Result<String, String> {
    let (catalog, entry) = marketplace::find_entry(agent_dir, target)?;
    let (path, plugin) = marketplace::install(agent_dir, &catalog, &entry)?;
    let key = format!("{}@{}", plugin.name, catalog.name);
    let previous = store::update(agent_dir, |file| {
        let previous = file.plugins.get(&key).cloned();
        file.plugins.insert(
            key.clone(),
            InstalledPlugin {
                origin: Origin::Davinci,
                install_path: Some(path.clone()),
                version: plugin.version.clone(),
                enabled: true,
                hooks_approved: previous.as_ref().and_then(|p| p.hooks_approved.clone()),
                installed_at: store::now_ms(),
            },
        );
        Ok(previous)
    })?;
    let mut out = format!("Installed {key} ({}).", plugin.component_summary());
    if let Some(previous) = previous.and_then(|p| p.install_path) {
        if previous != path && previous.starts_with(store::plugins_dir(agent_dir).join("cache")) {
            let _ = std::fs::remove_dir_all(previous);
        }
    }
    let record = &store::load(agent_dir).plugins[&key];
    append_hook_notice(&mut out, &key, record, &plugin);
    let _ = write!(out, "\n{RELOAD_HINT}");
    Ok(out)
}

fn append_hook_notice(out: &mut String, key: &str, record: &InstalledPlugin, plugin: &Plugin) {
    match hook_state(record, plugin) {
        "none" | "approved" => {}
        _ => {
            let _ = write!(
                out,
                "\n\nThis plugin has hooks: shell commands that run on your machine.\n{}\
                 They stay off until you approve them: `plugin approve {key}`.",
                describe_hooks(plugin)
            );
        }
    }
}

fn describe_hooks(plugin: &Plugin) -> String {
    let mut out = String::new();
    for hook in &plugin.hooks {
        let _ = writeln!(
            out,
            "  {}{}{}: {}",
            hook.event.as_str(),
            hook.matcher
                .as_deref()
                .map(|m| format!(" [{m}]"))
                .unwrap_or_default(),
            if hook.is_async { " (async)" } else { "" },
            hook.command
        );
    }
    out
}

fn importable(tool: Option<&str>, agent_dir: &Path) -> String {
    let installed = store::load(agent_dir);
    let found: Vec<_> = external::all_plugins()
        .into_iter()
        .filter(|p| tool.map_or(true, |t| p.origin.label() == t))
        .collect();
    if found.is_empty() {
        return "No Claude Code or Codex plugins found on this machine.".into();
    }
    let mut out = String::from("Plugins installed by other tools:\n");
    for plugin in &found {
        let adopted = if installed.plugins.contains_key(&plugin.key) {
            "*"
        } else {
            " "
        };
        let _ = writeln!(
            out,
            "  {adopted} {}  [{}{}]  {}",
            plugin.key,
            plugin.origin.label(),
            if plugin.enabled_there {
                ", enabled there"
            } else {
                ", disabled there"
            },
            plugin.version.as_deref().unwrap_or("")
        );
    }
    out.push_str(
        "\n* already adopted. Adopt with `plugin import <key>`, or every plugin enabled there \
         with `plugin import --all`. Adopted plugins load in place and follow that tool's updates.",
    );
    out
}

fn adopt(found: &external::ExternalPlugin, agent_dir: &Path) -> Result<String, String> {
    let plugin = manifest::load_plugin(&found.path)?;
    store::update(agent_dir, |file| {
        if let Some(existing) = file.plugins.get(&found.key) {
            if existing.origin != found.origin {
                return Err(format!(
                    "{} is already installed from {}",
                    found.key,
                    existing.origin.label()
                ));
            }
        }
        let previous = file.plugins.get(&found.key).cloned();
        file.plugins.insert(
            found.key.clone(),
            InstalledPlugin {
                origin: found.origin,
                install_path: None,
                version: found.version.clone(),
                enabled: true,
                hooks_approved: previous.and_then(|p| p.hooks_approved),
                installed_at: store::now_ms(),
            },
        );
        Ok(())
    })?;
    let record = &store::load(agent_dir).plugins[&found.key];
    let mut out = format!(
        "Adopted {} from {} ({}).",
        found.key,
        found.origin.label(),
        plugin.component_summary()
    );
    append_hook_notice(&mut out, &found.key, record, &plugin);
    Ok(out)
}

fn import_one(tool: Option<&str>, key: &str, agent_dir: &Path) -> Result<String, String> {
    let candidates: Vec<_> = external::all_plugins()
        .into_iter()
        .filter(|p| tool.map_or(true, |t| p.origin.label() == t))
        .filter(|p| p.key == key || store::split_key(&p.key).0 == key)
        .collect();
    // The same key in both tools is the same plugin: take Claude Code's copy,
    // as `import --all` does.
    let same_key = candidates.windows(2).all(|pair| pair[0].key == pair[1].key);
    let found = match candidates.as_slice() {
        [one] => one,
        [first, ..] if same_key => first,
        [] => return Err(format!("no Claude Code or Codex plugin named {key}; see `plugin import`")),
        _ => {
            return Err(format!(
                "{key} is installed in several places ({}); name the tool and key, e.g. `plugin import claude {}`",
                candidates
                    .iter()
                    .map(|p| format!("{} in {}", p.key, p.origin.label()))
                    .collect::<Vec<_>>()
                    .join(", "),
                candidates[0].key
            ))
        }
    };
    Ok(format!("{}\n{RELOAD_HINT}", adopt(found, agent_dir)?))
}

fn import_all(tool: Option<&str>, agent_dir: &Path) -> Result<String, String> {
    let installed = store::load(agent_dir);
    let mut seen = std::collections::BTreeSet::new();
    let mut lines = Vec::new();
    // Claude Code first: a plugin present in both tools is adopted once.
    for found in external::all_plugins() {
        if tool.is_some_and(|t| found.origin.label() != t)
            || !found.enabled_there
            || installed.plugins.contains_key(&found.key)
            || !seen.insert(found.key.clone())
        {
            continue;
        }
        match adopt(&found, agent_dir) {
            Ok(text) => lines.push(text),
            Err(err) => lines.push(format!("Skipped {}: {err}", found.key)),
        }
    }
    if lines.is_empty() {
        return Ok("Nothing new to adopt.".into());
    }
    lines.push(RELOAD_HINT.into());
    Ok(lines.join("\n\n"))
}

fn load_installed(
    agent_dir: &Path,
    target: &str,
) -> Result<(String, InstalledPlugin, Plugin), String> {
    let file = store::load(agent_dir);
    let key = file.resolve_key(target)?;
    let record = file.plugins[&key].clone();
    let plugin = super::locate(&key, &record).and_then(|path| manifest::load_plugin(&path))?;
    Ok((key, record, plugin))
}

fn info(agent_dir: &Path, target: &str) -> Result<String, String> {
    let (key, record, plugin) = load_installed(agent_dir, target)?;
    let mut out = format!(
        "{key}  v{}\n{}\nfrom {}, {}\npath: {}\n",
        plugin.version.as_deref().unwrap_or("?"),
        plugin.description.as_deref().unwrap_or(""),
        record.origin.label(),
        if record.enabled {
            "enabled"
        } else {
            "disabled"
        },
        plugin.root.display()
    );
    let names = |files: &[std::path::PathBuf], parent: bool| -> String {
        files
            .iter()
            .filter_map(|path| {
                let target = if parent {
                    path.parent()?
                } else {
                    path.as_path()
                };
                target
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let servers = plugin
        .mcp_servers
        .keys()
        .map(|server| format!("plugin_{}_{server}", plugin.name))
        .collect::<Vec<_>>()
        .join(", ");
    for (label, value) in [
        ("skills", names(&plugin.skill_files, true)),
        ("commands", names(&plugin.command_files, false)),
        ("agents", names(&plugin.agent_files, false)),
        ("MCP servers", servers),
    ] {
        if !value.is_empty() {
            let _ = writeln!(out, "{label}: {value}");
        }
    }
    let _ = writeln!(out, "hooks: {}", hook_state(&record, &plugin));
    out.push_str(&describe_hooks(&plugin));
    for unsupported in &plugin.unsupported_hooks {
        let _ = writeln!(out, "  not supported, never run: {unsupported}");
    }
    for warning in &plugin.warnings {
        let _ = writeln!(out, "warning: {warning}");
    }
    Ok(out.trim_end().to_string())
}

fn approve(agent_dir: &Path, target: &str) -> Result<String, String> {
    let (key, _, plugin) = load_installed(agent_dir, target)?;
    // Hash afresh: approval must cover the files as they are now, not as a
    // cached digest from earlier in this process saw them.
    let Some(digest) = super::hooks::digest(&plugin.root, &plugin.hooks) else {
        return Ok(format!("{key} has no hooks to approve."));
    };
    store::update(agent_dir, |file| {
        let record = file.plugins.get_mut(&key).ok_or("plugin disappeared")?;
        record.hooks_approved = Some(digest);
        Ok(())
    })?;
    Ok(format!(
        "Approved these hooks for {key}:\n{}Any change to them or to the plugin's files turns them off until you approve again.\n{RELOAD_HINT}",
        describe_hooks(&plugin)
    ))
}

/// Run the plugin's approved `SessionStart` hooks once, so a user can see
/// whether they work on this machine before starting a session.
fn test_hooks(agent_dir: &Path, target: &str, cwd: &Path) -> Result<String, String> {
    use super::hooks::HookEvent;
    let (key, _, _) = load_installed(agent_dir, target)?;
    let active = super::active(agent_dir);
    let Some(plugin) = active.plugins.iter().find(|p| p.key == key) else {
        return Err(format!("{key} is disabled or failed to load"));
    };
    if !plugin.hooks_approved() {
        return Err(format!(
            "{key} has no approved hooks; see `plugin info {key}`"
        ));
    }
    let single = super::ActivePlugins {
        plugins: vec![plugin.clone()],
        errors: Vec::new(),
    };
    let input = super::HookInput {
        cwd: cwd.to_path_buf(),
        source: Some("startup".into()),
        ..super::HookInput::default()
    };
    let result = single.run_event(HookEvent::SessionStart, Some("startup"), &input);
    let mut out = format!("SessionStart hooks of {key}:\n");
    if result.contexts.is_empty() && result.warnings.is_empty() {
        out.push_str("  ran with no output.");
    }
    for (_, context) in &result.contexts {
        let chars = context.chars().count();
        let _ = writeln!(
            out,
            "  context ({chars} chars): {}",
            first_line(context, 120)
        );
    }
    for warning in &result.warnings {
        let _ = writeln!(out, "  warning: {warning}");
    }
    Ok(out.trim_end().to_string())
}

fn revoke(agent_dir: &Path, target: &str) -> Result<String, String> {
    let file = store::load(agent_dir);
    let key = file.resolve_key(target)?;
    store::update(agent_dir, |file| {
        if let Some(record) = file.plugins.get_mut(&key) {
            record.hooks_approved = None;
        }
        Ok(())
    })?;
    Ok(format!("Hooks for {key} are off."))
}

fn set_enabled(agent_dir: &Path, target: &str, enabled: bool) -> Result<String, String> {
    let file = store::load(agent_dir);
    let key = file.resolve_key(target)?;
    store::update(agent_dir, |file| {
        if let Some(record) = file.plugins.get_mut(&key) {
            record.enabled = enabled;
        }
        Ok(())
    })?;
    Ok(format!(
        "{} {key}. {RELOAD_HINT}",
        if enabled { "Enabled" } else { "Disabled" }
    ))
}

fn update(agent_dir: &Path, target: &str) -> Result<String, String> {
    let file = store::load(agent_dir);
    let key = file.resolve_key(target)?;
    let record = &file.plugins[&key];
    if record.origin != Origin::Davinci {
        return Ok(format!(
            "{key} is managed by {}; update it there. DaVinci follows it automatically.",
            record.origin.label()
        ));
    }
    let (_, market) = store::split_key(&key);
    let _ = marketplace::update(agent_dir, Some(market));
    install(agent_dir, &key)
}

fn uninstall(agent_dir: &Path, target: &str) -> Result<String, String> {
    let file = store::load(agent_dir);
    let key = file.resolve_key(target)?;
    let record = store::update(agent_dir, |file| {
        file.plugins
            .remove(&key)
            .ok_or_else(|| "plugin disappeared".to_string())
    })?;
    if let Some(path) = record.install_path {
        let cache = store::plugins_dir(agent_dir).join("cache");
        if record.origin == Origin::Davinci && path.starts_with(&cache) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    Ok(match record.origin {
        Origin::Davinci => format!("Uninstalled {key}."),
        origin => format!(
            "Stopped loading {key}. It stays installed in {}.",
            origin.label()
        ),
    })
}

fn marketplaces(agent_dir: &Path) -> String {
    let registry = marketplace::load_registry(agent_dir);
    let mut out = String::new();
    for catalog in marketplace::catalogs(agent_dir) {
        let source = registry
            .get(&catalog.name)
            .map(|entry| entry.source.describe())
            .unwrap_or_else(|| catalog.root.display().to_string());
        let _ = writeln!(
            out,
            "  {}  [{}]  {} plugin(s)  {source}",
            catalog.name,
            catalog.from,
            catalog.entries().len()
        );
    }
    if out.is_empty() {
        "No marketplaces known. Add one with `plugin marketplace add <owner/repo>`.".into()
    } else {
        format!("Marketplaces:\n{out}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::external::tests::{write, FakeHomes};

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn install_approve_disable_and_uninstall_flow() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let market = dir.path().join("market");
        write(
            &market.join(".claude-plugin/marketplace.json"),
            r#"{"name":"mk","plugins":[{"name":"guard","source":"./guard","description":"Blocks things"}]}"#,
        );
        write(
            &market.join("guard/.claude-plugin/plugin.json"),
            r#"{"name":"guard","version":"1.0.0"}"#,
        );
        write(
            &market.join("guard/hooks/hooks.json"),
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"exit 2"}]}]}}"#,
        );
        let run_cmd = |words: &[&str]| run(&args(words), &agent_dir, dir.path());

        run_cmd(&["marketplace", "add", market.to_str().unwrap()]).unwrap();
        assert!(run_cmd(&["browse", "block"]).unwrap().contains("guard@mk"));
        let installed = run_cmd(&["install", "guard"]).unwrap();
        assert!(installed.contains("plugin approve guard@mk"), "{installed}");
        assert!(run_cmd(&["list"]).unwrap().contains("approval needed"));

        let approved = run_cmd(&["approve", "guard"]).unwrap();
        assert!(approved.contains("PreToolUse [Bash]: exit 2"));
        assert!(run_cmd(&["info", "guard"])
            .unwrap()
            .contains("hooks: approved"));
        // Reinstalling keeps the approval while the hooks are unchanged.
        run_cmd(&["update", "guard"]).unwrap();
        assert!(run_cmd(&["list"]).unwrap().contains("hooks: approved"));

        run_cmd(&["disable", "guard"]).unwrap();
        assert!(super::super::active(&agent_dir).plugins.is_empty());
        run_cmd(&["enable", "guard"]).unwrap();
        assert_eq!(super::super::active(&agent_dir).plugins.len(), 1);

        let cache = store::load(&agent_dir).plugins["guard@mk"]
            .install_path
            .clone()
            .unwrap();
        run_cmd(&["uninstall", "guard"]).unwrap();
        assert!(!cache.exists());
        assert!(run_cmd(&["list"]).unwrap().contains("No plugins installed"));
        assert!(run_cmd(&["frobnicate"]).is_err());
    }

    #[test]
    fn imports_claude_code_plugins_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let plugin_dir = homes.claude.join("plugins/cache/mk/tool/1.0.0");
        write(
            &plugin_dir.join(".claude-plugin/plugin.json"),
            r#"{"name":"tool"}"#,
        );
        write(&plugin_dir.join("skills/t/SKILL.md"), "---\nname: t\n---\n");
        write(
            &homes.claude.join("plugins/installed_plugins.json"),
            &serde_json::json!({"version": 2, "plugins": {
                "tool@mk": [{"scope": "user", "installPath": plugin_dir, "version": "1.0.0"}]
            }})
            .to_string(),
        );
        let run_cmd = |words: &[&str]| run(&args(words), &agent_dir, dir.path());

        assert!(run_cmd(&["import"]).unwrap().contains("tool@mk"));
        assert!(run_cmd(&["import", "--all"])
            .unwrap()
            .contains("Adopted tool@mk from claude"));
        assert_eq!(
            run_cmd(&["import", "--all"]).unwrap(),
            "Nothing new to adopt."
        );
        let active = super::super::active(&agent_dir);
        assert_eq!(
            active.plugins[0].plugin.root,
            manifest::canonical(&plugin_dir).unwrap()
        );
        let removed = run_cmd(&["uninstall", "tool"]).unwrap();
        assert!(removed.contains("stays installed in claude"));
        assert!(plugin_dir.exists());
    }
}
