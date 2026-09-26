//! Marketplaces: catalogs of plugins, and installing from them.
//!
//! No TypeScript counterpart. Spec:
//! `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.
//!
//! A marketplace is a directory or git repository with a catalog at
//! `.claude-plugin/marketplace.json` (Claude Code),
//! `.agents/plugins/marketplace.json` (Codex) or `marketplace.json`.
//! DaVinci's own registry is `<agent_dir>/plugins/marketplaces.json`; the
//! marketplaces Claude Code and Codex keep on disk are read as extra
//! catalogs. Installing always copies the plugin into DaVinci's cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::external;
use super::manifest::{self, Plugin};
use super::store::{self, plugins_dir};

pub const CATALOG_PATHS: &[&str] = &[
    ".claude-plugin/marketplace.json",
    ".agents/plugins/marketplace.json",
    "marketplace.json",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum MarketplaceSource {
    Github { repo: String },
    Git { url: String },
    Directory { path: PathBuf },
}

impl MarketplaceSource {
    pub fn describe(&self) -> String {
        match self {
            Self::Github { repo } => format!("github:{repo}"),
            Self::Git { url } => url.clone(),
            Self::Directory { path } => path.display().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownMarketplace {
    pub source: MarketplaceSource,
    pub install_location: PathBuf,
    #[serde(default)]
    pub last_updated: u64,
}

pub type Registry = BTreeMap<String, KnownMarketplace>;

fn registry_path(agent_dir: &Path) -> PathBuf {
    plugins_dir(agent_dir).join("marketplaces.json")
}

pub fn load_registry(agent_dir: &Path) -> Registry {
    std::fs::read_to_string(registry_path(agent_dir))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

fn save_registry(agent_dir: &Path, registry: &Registry) -> Result<(), String> {
    store::write_json_atomic(&registry_path(agent_dir), registry)
}

/// Names become directory names; refuse anything that could leave the
/// directory they are joined to.
pub fn validate_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid plugin or marketplace name: {name:?}"))
    }
}

pub fn parse_source(spec: &str, cwd: &Path) -> Result<MarketplaceSource, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("marketplace source is empty".into());
    }
    let local = if Path::new(spec).is_absolute() {
        PathBuf::from(spec)
    } else {
        cwd.join(spec)
    };
    if local.is_dir() {
        let path = manifest::canonical(&local).map_err(|err| err.to_string())?;
        return Ok(MarketplaceSource::Directory { path });
    }
    if spec.starts_with("https://")
        || spec.starts_with("http://")
        || spec.starts_with("git@")
        || spec.starts_with("ssh://")
        || spec.ends_with(".git")
    {
        return Ok(MarketplaceSource::Git { url: spec.into() });
    }
    let github = spec.strip_prefix("github:").unwrap_or(spec);
    let mut parts = github.split('/');
    if let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) {
        if validate_name(owner).is_ok() && validate_name(repo).is_ok() {
            return Ok(MarketplaceSource::Github {
                repo: github.into(),
            });
        }
    }
    Err(format!("not a directory, git URL or owner/repo: {spec}"))
}

pub fn read_catalog(root: &Path) -> Option<Value> {
    CATALOG_PATHS.iter().find_map(|rel| {
        let body = std::fs::read_to_string(root.join(rel)).ok()?;
        serde_json::from_str(&body).ok()
    })
}

fn git_program() -> PathBuf {
    let git = std::env::var("PI_GIT_CMD").unwrap_or_else(|_| "git".into());
    davinci_sys::process::resolve_program(&git)
}

fn run_git(args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    for arg in args.iter().skip(1) {
        if arg.starts_with('-') && !matches!(*arg, "--" | "--depth" | "--ff-only" | "--quiet") {
            return Err(format!(
                "refusing git argument that looks like an option: {arg}"
            ));
        }
    }
    let mut command = std::process::Command::new(git_program());
    command.args(args);
    // Also covers submodules and redirects git itself might follow.
    command.env("GIT_ALLOW_PROTOCOL", "https:http:ssh:git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().map_err(|err| format!("git: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn clone_url(source: &MarketplaceSource) -> Option<String> {
    match source {
        MarketplaceSource::Github { repo } => Some(format!("https://github.com/{repo}.git")),
        MarketplaceSource::Git { url } => Some(url.clone()),
        MarketplaceSource::Directory { .. } => None,
    }
}

/// Catalog URLs are written by marketplace maintainers. Only network
/// transports are accepted: git's `ext::`, `fd::` and local `file://`
/// remotes can run commands or read arbitrary paths.
pub fn validate_git_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let scp_like = lower.starts_with("git@")
        && url
            .split_once(':')
            .is_some_and(|(_, path)| !path.starts_with("//"));
    let allowed = ["https://", "http://", "ssh://", "git://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
        || scp_like;
    if allowed && !url.contains("::") && !url.chars().any(char::is_whitespace) {
        Ok(())
    } else {
        Err(format!(
            "refusing git URL (only https, http, ssh and git are allowed): {url}"
        ))
    }
}

fn clone(url: &str, git_ref: Option<&str>, dest: &Path) -> Result<(), String> {
    validate_git_url(url)?;
    let dest_text = dest.display().to_string();
    if git_ref.is_some() {
        run_git(&["clone", "--quiet", "--", url, &dest_text], None)?;
    } else {
        run_git(
            &["clone", "--quiet", "--depth", "1", "--", url, &dest_text],
            None,
        )?;
    }
    if let Some(git_ref) = git_ref {
        if git_ref.starts_with('-') {
            return Err(format!(
                "refusing git ref that looks like an option: {git_ref}"
            ));
        }
        run_git(&["checkout", "--quiet", git_ref, "--"], Some(dest))?;
    }
    Ok(())
}

/// Add a marketplace and return its catalog name.
pub fn add(agent_dir: &Path, spec: &str, cwd: &Path) -> Result<String, String> {
    let source = parse_source(spec, cwd)?;
    let base = plugins_dir(agent_dir).join("marketplaces");
    std::fs::create_dir_all(&base).map_err(|err| err.to_string())?;
    let (name, location) = match clone_url(&source) {
        None => {
            let MarketplaceSource::Directory { path } = &source else {
                unreachable!()
            };
            let catalog = read_catalog(path)
                .ok_or_else(|| format!("no marketplace.json in {}", path.display()))?;
            (catalog_name(&catalog, path)?, path.clone())
        }
        Some(url) => {
            let staging = base.join(format!(".staging-{}", uuid::Uuid::new_v4()));
            clone(&url, None, &staging)?;
            let result = (|| {
                let catalog = read_catalog(&staging)
                    .ok_or_else(|| format!("no marketplace.json in {url}"))?;
                let name = catalog_name(&catalog, &staging)?;
                let dest = base.join(&name);
                if dest.exists() {
                    std::fs::remove_dir_all(&dest).map_err(|err| err.to_string())?;
                }
                std::fs::rename(&staging, &dest).map_err(|err| err.to_string())?;
                Ok::<_, String>((name, dest))
            })();
            if result.is_err() {
                let _ = std::fs::remove_dir_all(&staging);
            }
            result?
        }
    };
    let mut registry = load_registry(agent_dir);
    registry.insert(
        name.clone(),
        KnownMarketplace {
            source,
            install_location: location,
            last_updated: store::now_ms(),
        },
    );
    save_registry(agent_dir, &registry)?;
    Ok(name)
}

fn catalog_name(catalog: &Value, root: &Path) -> Result<String, String> {
    let name = catalog
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    validate_name(&name)?;
    Ok(name)
}

/// Pull git marketplaces. Returns one line per marketplace.
pub fn update(agent_dir: &Path, only: Option<&str>) -> Result<Vec<String>, String> {
    let mut registry = load_registry(agent_dir);
    if let Some(name) = only {
        if !registry.contains_key(name) {
            return Err(format!("no marketplace named {name}"));
        }
    }
    let mut lines = Vec::new();
    for (name, entry) in registry.iter_mut() {
        if only.is_some_and(|wanted| wanted != name) {
            continue;
        }
        if clone_url(&entry.source).is_none() {
            lines.push(format!("{name}: local directory, nothing to pull"));
            continue;
        }
        match run_git(
            &["pull", "--ff-only", "--quiet"],
            Some(&entry.install_location),
        ) {
            Ok(()) => {
                entry.last_updated = store::now_ms();
                lines.push(format!("{name}: updated"));
            }
            Err(err) => lines.push(format!("{name}: {err}")),
        }
    }
    save_registry(agent_dir, &registry)?;
    Ok(lines)
}

pub fn remove(agent_dir: &Path, name: &str) -> Result<(), String> {
    let mut registry = load_registry(agent_dir);
    let entry = registry
        .remove(name)
        .ok_or_else(|| format!("no marketplace named {name}"))?;
    let owned = plugins_dir(agent_dir).join("marketplaces");
    if clone_url(&entry.source).is_some() && entry.install_location.starts_with(&owned) {
        let _ = std::fs::remove_dir_all(&entry.install_location);
    }
    save_registry(agent_dir, &registry)
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub name: String,
    pub root: PathBuf,
    /// `davinci`, `claude` or `codex`: who registered the marketplace.
    pub from: &'static str,
    pub document: Value,
}

impl Catalog {
    pub fn entries(&self) -> Vec<&Value> {
        self.document
            .get("plugins")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("name").is_some())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Every readable catalog: DaVinci's registry first, then Claude Code's and
/// Codex's. The first catalog with a name wins.
pub fn catalogs(agent_dir: &Path) -> Vec<Catalog> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut candidates: Vec<(String, PathBuf, &'static str)> = load_registry(agent_dir)
        .into_iter()
        .map(|(name, entry)| (name, entry.install_location, "davinci"))
        .collect();
    candidates.extend(external::marketplace_dirs());
    for (name, root, from) in candidates {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(document) = read_catalog(&root) {
            out.push(Catalog {
                name,
                root,
                from,
                document,
            });
        }
    }
    out
}

/// The catalog entry for `name` (optionally `name@marketplace`).
pub fn find_entry(agent_dir: &Path, wanted: &str) -> Result<(Catalog, Value), String> {
    let (name, market) = match wanted.split_once('@') {
        Some((name, market)) => (name, Some(market)),
        None => (wanted, None),
    };
    let mut found = Vec::new();
    for catalog in catalogs(agent_dir) {
        if market.is_some_and(|m| m != catalog.name) {
            continue;
        }
        for entry in catalog.entries() {
            if entry.get("name").and_then(Value::as_str) == Some(name) {
                found.push((catalog.clone(), entry.clone()));
            }
        }
    }
    match found.len() {
        0 => Err(format!(
            "no plugin {wanted} in any marketplace; add one with `plugin marketplace add <owner/repo>`"
        )),
        1 => Ok(found.remove(0)),
        _ => Err(format!(
            "{name} is in several marketplaces ({}); use {name}@<marketplace>",
            found
                .iter()
                .map(|(catalog, _)| catalog.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Install one catalog entry into `cache/<marketplace>/<plugin>/<version>/`.
pub fn install(
    agent_dir: &Path,
    catalog: &Catalog,
    entry: &Value,
) -> Result<(PathBuf, Plugin), String> {
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or("catalog entry has no name")?;
    validate_name(name)?;
    validate_name(&catalog.name)?;
    let plugin_base = plugins_dir(agent_dir)
        .join("cache")
        .join(&catalog.name)
        .join(name);
    std::fs::create_dir_all(&plugin_base).map_err(|err| err.to_string())?;
    let staging = plugin_base.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        fetch_entry_source(catalog, entry, &staging)?;
        if manifest::read_manifest(&staging).is_none() {
            // `strict: false` entries carry the manifest in the catalog.
            let mut synthesized = entry.clone();
            if let Some(object) = synthesized.as_object_mut() {
                object.remove("source");
            }
            store::write_json_atomic(
                &staging.join(".claude-plugin").join("plugin.json"),
                &synthesized,
            )?;
        }
        let staged_manifest = manifest::read_manifest(&staging).unwrap_or(Value::Null);
        let version = entry
            .get("version")
            .or_else(|| staged_manifest.get("version"))
            .and_then(Value::as_str)
            .filter(|version| validate_name(version).is_ok())
            .unwrap_or("latest");
        let dest = plugin_base.join(version);
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|err| err.to_string())?;
        }
        std::fs::rename(&staging, &dest).map_err(|err| err.to_string())?;
        let plugin = manifest::load_plugin(&dest)?;
        Ok::<_, String>((dest, plugin))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn fetch_entry_source(catalog: &Catalog, entry: &Value, dest: &Path) -> Result<(), String> {
    let source = entry.get("source").cloned().unwrap_or(Value::Null);
    let text = |key: &str| source.get(key).and_then(Value::as_str);
    match &source {
        Value::String(rel) => copy_from_catalog(catalog, rel, dest),
        Value::Object(_) => match text("source").unwrap_or("") {
            "local" | "directory" | "path" => copy_from_catalog(
                catalog,
                text("path").ok_or("local source has no path")?,
                dest,
            ),
            "github" => {
                let repo = text("repo").ok_or("github source has no repo")?;
                clone_subdir(
                    &format!("https://github.com/{repo}.git"),
                    text("ref"),
                    text("path"),
                    dest,
                )
            }
            "url" | "git" => clone_subdir(
                text("url").ok_or("git source has no url")?,
                text("ref"),
                text("path"),
                dest,
            ),
            "git-subdir" => clone_subdir(
                text("url").ok_or("git-subdir source has no url")?,
                text("ref"),
                Some(text("path").ok_or("git-subdir source has no path")?),
                dest,
            ),
            other => Err(format!("unsupported plugin source type: {other:?}")),
        },
        _ => Err("catalog entry has no source".into()),
    }
}

fn copy_from_catalog(catalog: &Catalog, rel: &str, dest: &Path) -> Result<(), String> {
    let base_rel = catalog
        .document
        .get("metadata")
        .and_then(|meta| meta.get("pluginRoot"))
        .and_then(Value::as_str)
        .filter(|_| !rel.starts_with("./") && !rel.starts_with("../"));
    let rel = match base_rel {
        Some(base) => format!("{}/{}", base.trim_end_matches('/'), rel),
        None => rel.to_string(),
    };
    let rel = if rel == "." || rel == "./" {
        String::from(".")
    } else {
        rel
    };
    let source = if rel == "." {
        manifest::canonical(&catalog.root).map_err(|err| err.to_string())?
    } else {
        manifest::resolve_inside(&catalog.root, &rel).ok_or_else(|| {
            format!("plugin path escapes or is missing from the marketplace: {rel}")
        })?
    };
    copy_tree(&source, dest)
}

fn clone_subdir(
    url: &str,
    git_ref: Option<&str>,
    subdir: Option<&str>,
    dest: &Path,
) -> Result<(), String> {
    let Some(subdir) = subdir.filter(|s| !s.is_empty() && *s != ".") else {
        clone(url, git_ref, dest)?;
        let _ = std::fs::remove_dir_all(dest.join(".git"));
        return Ok(());
    };
    let checkout = dest.with_extension("checkout");
    clone(url, git_ref, &checkout)?;
    let result = manifest::resolve_inside(&checkout, subdir)
        .ok_or_else(|| format!("path {subdir} is missing from {url}"))
        .and_then(|source| copy_tree(&source, dest));
    let _ = std::fs::remove_dir_all(&checkout);
    result
}

/// Copy a plugin tree, skipping `.git` and symlinks (a link could point
/// anywhere on the machine).
pub fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|err| err.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|err| format!("{}: {err}", from.display()))? {
        let entry = entry.map_err(|err| err.to_string())?;
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        if file_type.is_symlink() || entry.file_name() == ".git" {
            continue;
        }
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::external::tests::{write, FakeHomes};

    #[test]
    fn parses_marketplace_sources() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            parse_source("anthropics/claude-plugins-official", dir.path()).unwrap(),
            MarketplaceSource::Github {
                repo: "anthropics/claude-plugins-official".into()
            }
        );
        assert!(matches!(
            parse_source("https://example.invalid/m.git", dir.path()).unwrap(),
            MarketplaceSource::Git { .. }
        ));
        assert!(matches!(
            parse_source(".", dir.path()).unwrap(),
            MarketplaceSource::Directory { .. }
        ));
        assert!(parse_source("../evil/../x/y/z", dir.path()).is_err());
        for good in [
            "https://github.com/a/b.git",
            "ssh://git@host/a/b.git",
            "git@github.com:a/b.git",
        ] {
            assert!(validate_git_url(good).is_ok(), "{good}");
        }
        for bad in [
            "ext::sh -c touch% /tmp/pwned",
            "fd::17",
            "file:///etc",
            "-uhttps://x",
            "https://x y",
            "C:/repo",
        ] {
            assert!(validate_git_url(bad).is_err(), "{bad}");
        }
        assert!(validate_name("..").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("good-name_1.0").is_ok());
    }

    #[test]
    fn adds_a_local_marketplace_and_installs_from_it() {
        let dir = tempfile::tempdir().unwrap();
        let _homes = FakeHomes::new(dir.path());
        let agent_dir = dir.path().join("agent");
        let market = dir.path().join("market");
        write(
            &market.join(".claude-plugin/marketplace.json"),
            r#"{"name":"demo-market","plugins":[
                {"name":"hello","source":"./plugins/hello","version":"0.1.0"},
                {"name":"loose","source":"./plugins/loose","strict":false,
                 "description":"catalog manifest","commands":["./extra/cmd.md"]},
                {"name":"escape","source":"../outside"}
            ]}"#,
        );
        write(
            &market.join("plugins/hello/.claude-plugin/plugin.json"),
            r#"{"name":"hello","version":"0.1.0"}"#,
        );
        write(
            &market.join("plugins/hello/skills/greet/SKILL.md"),
            "---\nname: greet\n---\n",
        );
        write(&market.join("plugins/loose/extra/cmd.md"), "Run $ARGUMENTS");
        write(&dir.path().join("outside/secret.txt"), "x");

        let name = add(&agent_dir, market.to_str().unwrap(), dir.path()).unwrap();
        assert_eq!(name, "demo-market");
        assert_eq!(catalogs(&agent_dir).len(), 1);

        let (catalog, entry) = find_entry(&agent_dir, "hello").unwrap();
        let (path, plugin) = install(&agent_dir, &catalog, &entry).unwrap();
        assert!(path.ends_with(Path::new("demo-market").join("hello").join("0.1.0")));
        assert_eq!(plugin.skill_files.len(), 1);

        let (catalog, entry) = find_entry(&agent_dir, "loose@demo-market").unwrap();
        let (_, plugin) = install(&agent_dir, &catalog, &entry).unwrap();
        assert_eq!(plugin.description.as_deref(), Some("catalog manifest"));
        assert_eq!(plugin.command_files.len(), 1);

        let (catalog, entry) = find_entry(&agent_dir, "escape").unwrap();
        assert!(install(&agent_dir, &catalog, &entry).is_err());

        remove(&agent_dir, "demo-market").unwrap();
        assert!(market.exists(), "a local marketplace is never deleted");
        assert!(catalogs(&agent_dir).is_empty());
    }
}
