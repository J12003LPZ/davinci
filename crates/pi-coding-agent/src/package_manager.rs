use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

use crate::settings::{self, agent_dir, package_source_string, settings_path, SettingsDocument};

pub const RESOURCE_TYPES: [&str; 4] = ["extensions", "skills", "prompts", "themes"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSource {
    Npm {
        spec: String,
        name: String,
        version: Option<String>,
        pinned: bool,
    },
    Git {
        repo: String,
        host: String,
        path: String,
        git_ref: Option<String>,
        pinned: bool,
    },
    Local {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredPackage {
    pub source: String,
    pub scope: String,
    pub filtered: bool,
    pub installed_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMetadata {
    pub source: String,
    pub scope: String,
    pub origin: String,
    pub base_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource {
    pub path: PathBuf,
    pub enabled: bool,
    pub metadata: PathMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPaths {
    pub extensions: Vec<ResolvedResource>,
    pub skills: Vec<ResolvedResource>,
    pub prompts: Vec<ResolvedResource>,
    pub themes: Vec<ResolvedResource>,
}

pub fn is_local_path(value: &str) -> bool {
    let trimmed = value.trim();
    !(trimmed.starts_with("npm:")
        || trimmed.starts_with("git:")
        || trimmed.starts_with("github:")
        || trimmed.starts_with("http:")
        || trimmed.starts_with("https:")
        || trimmed.starts_with("ssh:"))
}

pub fn is_offline_mode_enabled() -> bool {
    std::env::var("PI_OFFLINE")
        .ok()
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

pub fn network_disabled() -> bool {
    std::env::var("PI_DISABLE_NETWORK")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

pub fn parse_npm_spec(spec: &str) -> (String, Option<String>) {
    let bytes = spec.as_bytes();
    let mut at = None;
    for (i, ch) in bytes.iter().enumerate() {
        if *ch == b'@' && i > 0 {
            at = Some(i);
        }
    }
    if let Some(i) = at {
        (spec[..i].to_string(), Some(spec[i + 1..].to_string()))
    } else {
        (spec.to_string(), None)
    }
}

pub fn parse_source(source: &str) -> ParsedSource {
    if let Some(spec) = source.strip_prefix("npm:") {
        let spec = spec.trim();
        let (name, version) = parse_npm_spec(spec);
        let pinned = version
            .as_deref()
            .is_some_and(|v| !v.is_empty() && !v.contains(['^', '~', '*', 'x', 'X', ' ']));
        return ParsedSource::Npm {
            spec: spec.to_string(),
            name,
            version,
            pinned,
        };
    }
    if is_local_path(source) {
        return ParsedSource::Local {
            path: source.to_string(),
        };
    }
    if let Some(git) = parse_git_url(source) {
        return git;
    }
    ParsedSource::Local {
        path: source.to_string(),
    }
}

fn split_ref(url: &str) -> (String, Option<String>) {
    if let Some(caps) = url.strip_prefix("git@") {
        if let Some(colon) = caps.find(':') {
            let host = &caps[..colon];
            let path_with_ref = &caps[colon + 1..];
            if let Some(at) = path_with_ref.find('@') {
                let repo_path = &path_with_ref[..at];
                let git_ref = &path_with_ref[at + 1..];
                if !repo_path.is_empty() && !git_ref.is_empty() {
                    return (format!("git@{host}:{repo_path}"), Some(git_ref.to_string()));
                }
            }
        }
        return (url.to_string(), None);
    }
    if url.contains("://") {
        if let Some((scheme_host, path_with_ref)) = url.split_once("://") {
            let path_with_ref = path_with_ref
                .split_once('/')
                .map(|(_, path)| path)
                .unwrap_or("");
            if let Some(at) = path_with_ref.find('@') {
                let repo_path = &path_with_ref[..at];
                let git_ref = &path_with_ref[at + 1..];
                if !repo_path.is_empty() && !git_ref.is_empty() {
                    let host = url
                        .split_once("://")
                        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
                        .unwrap_or_default();
                    let repo = format!("{scheme_host}://{host}/{repo_path}");
                    return (repo, Some(git_ref.to_string()));
                }
            }
        }
        return (url.to_string(), None);
    }
    let Some(slash) = url.find('/') else {
        return (url.to_string(), None);
    };
    let host = &url[..slash];
    let path_with_ref = &url[slash + 1..];
    if let Some(at) = path_with_ref.find('@') {
        let repo_path = &path_with_ref[..at];
        let git_ref = &path_with_ref[at + 1..];
        if !repo_path.is_empty() && !git_ref.is_empty() {
            return (format!("{host}/{repo_path}"), Some(git_ref.to_string()));
        }
    }
    (url.to_string(), None)
}

fn has_unsafe_git_part(value: &str, allow_slash: bool) -> bool {
    let decoded = urlencoding_decode(value);
    let candidates = [value, decoded.as_deref().unwrap_or(value)];
    for candidate in candidates {
        if candidate.contains('\0') || candidate.contains('\\') || candidate.starts_with('/') {
            return true;
        }
        if !allow_slash && candidate.contains('/') {
            return true;
        }
        if candidate.split('/').any(|part| part == "..") {
            return true;
        }
    }
    false
}

fn urlencoding_decode(value: &str) -> Option<String> {
    let mut out = String::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            out.push(byte as char);
            i += 3;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Some(out)
}

fn build_git_source(
    repo: String,
    host: String,
    path: String,
    git_ref: Option<String>,
) -> Option<ParsedSource> {
    if path.starts_with('/') {
        return None;
    }
    let normalized = path
        .trim_start_matches('/')
        .trim_end_matches(".git")
        .to_string();
    if host.is_empty() || normalized.is_empty() || normalized.split('/').count() < 2 {
        return None;
    }
    if has_unsafe_git_part(&host, false) || has_unsafe_git_part(&normalized, true) {
        return None;
    }
    let pinned = git_ref.is_some();
    Some(ParsedSource::Git {
        repo,
        host,
        path: normalized,
        git_ref,
        pinned,
    })
}

fn parse_hosted(repo: &str, git_ref: Option<&str>) -> Option<ParsedSource> {
    let hosted = [
        ("github:", "github.com"),
        ("gitlab:", "gitlab.com"),
        ("bitbucket:", "bitbucket.org"),
    ];
    for (prefix, host) in hosted {
        if let Some(rest) = repo.strip_prefix(prefix) {
            let rest = rest.trim_end_matches(".git");
            if rest.split('/').count() >= 2 {
                return build_git_source(
                    format!("https://{host}/{rest}"),
                    host.to_string(),
                    rest.to_string(),
                    git_ref.map(str::to_string),
                );
            }
        }
    }
    None
}

fn parse_generic_git_url(url: &str) -> Option<ParsedSource> {
    let (repo_without_ref, git_ref) = split_ref(url);
    let mut repo = repo_without_ref.clone();
    let (host, path) = if let Some(rest) = repo_without_ref.strip_prefix("git@") {
        let colon = rest.find(':')?;
        (rest[..colon].to_string(), rest[colon + 1..].to_string())
    } else if repo_without_ref.starts_with("https://")
        || repo_without_ref.starts_with("http://")
        || repo_without_ref.starts_with("ssh://")
        || repo_without_ref.starts_with("git://")
    {
        let after = repo_without_ref
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(repo_without_ref.as_str());
        let (auth_host, path) = after.split_once('/').unwrap_or((after, ""));
        let host = auth_host
            .rsplit('@')
            .next()
            .unwrap_or(auth_host)
            .split(':')
            .next()
            .unwrap_or(auth_host)
            .to_string();
        (host, path.to_string())
    } else {
        let slash = repo_without_ref.find('/')?;
        let host = repo_without_ref[..slash].to_string();
        let path = repo_without_ref[slash + 1..].to_string();
        if !host.contains('.') && host != "localhost" {
            return None;
        }
        repo = format!("https://{repo_without_ref}");
        (host, path)
    };
    build_git_source(repo, host, path, git_ref)
}

pub fn parse_git_url(source: &str) -> Option<ParsedSource> {
    let trimmed = source.trim();
    let has_git_prefix = trimmed.starts_with("git:");
    let url = if has_git_prefix {
        trimmed[4..].trim()
    } else {
        trimmed
    };
    if !has_git_prefix
        && !url.starts_with("https://")
        && !url.starts_with("http://")
        && !url.starts_with("ssh://")
        && !url.starts_with("git://")
    {
        return None;
    }
    let (repo_without_ref, git_ref) = split_ref(url);
    if let Some(hosted) = parse_hosted(&repo_without_ref, git_ref.as_deref()) {
        return Some(hosted);
    }
    if url.contains("github.com") || url.contains("gitlab.com") || url.contains("bitbucket.org") {
        if let Some(ParsedSource::Git {
            host,
            path,
            git_ref,
            pinned,
            ..
        }) = parse_hosted(
            repo_without_ref
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("ssh://")
                .trim_start_matches("git://"),
            git_ref.as_deref(),
        ) {
            let use_https = !repo_without_ref.starts_with("http://")
                && !repo_without_ref.starts_with("https://")
                && !repo_without_ref.starts_with("ssh://")
                && !repo_without_ref.starts_with("git://")
                && !repo_without_ref.starts_with("git@");
            let repo = if use_https {
                format!("https://{repo_without_ref}")
            } else {
                repo_without_ref
            };
            return Some(ParsedSource::Git {
                repo,
                host,
                path,
                git_ref,
                pinned,
            });
        }
    }
    parse_generic_git_url(url)
}

pub fn expand_install_path(source: &str) -> PathBuf {
    let trimmed = source.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        return PathBuf::from(rest);
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub fn package_is_object(value: &Value) -> bool {
    value.is_object()
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn project_dir() -> PathBuf {
    cwd().join(".pi")
}

pub fn npm_install_root(local: bool) -> PathBuf {
    if local {
        project_dir().join("npm")
    } else {
        agent_dir().join("npm")
    }
}

pub fn git_install_root(local: bool) -> PathBuf {
    if local {
        project_dir().join("git")
    } else {
        agent_dir().join("git")
    }
}

pub fn npm_install_path(name: &str, local: bool) -> PathBuf {
    npm_install_root(local).join("node_modules").join(name)
}

pub fn git_install_path(host: &str, path: &str, local: bool) -> PathBuf {
    let mut dir = git_install_root(local).join(host);
    for part in path.split('/') {
        if !part.is_empty() && part != ".." {
            dir = dir.join(part);
        }
    }
    dir
}

pub fn get_installed_path(source: &str, local: bool) -> Option<PathBuf> {
    match parse_source(source) {
        ParsedSource::Npm { name, .. } => {
            let path = npm_install_path(&name, local);
            path.exists().then_some(path)
        }
        ParsedSource::Git { host, path, .. } => {
            let installed = git_install_path(&host, &path, local);
            installed.exists().then_some(installed)
        }
        ParsedSource::Local { path } => {
            let resolved = expand_install_path(&path);
            resolved.exists().then_some(resolved)
        }
    }
}

fn ensure_npm_project(install_root: &Path) {
    let _ = fs::create_dir_all(install_root);
    let pkg = install_root.join("package.json");
    if !pkg.exists() {
        let _ = fs::write(
            pkg,
            serde_json::to_string_pretty(&json!({
                "name": "pi-extensions",
                "private": true
            }))
            .unwrap_or_else(|_| "{\"name\":\"pi-extensions\",\"private\":true}".into()),
        );
    }
    ensure_gitignore(install_root);
}

fn ensure_gitignore(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        let _ = fs::write(ignore, "*\n!.gitignore\n");
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err(format!("Path does not exist: {}", from.display()));
    }
    if from.is_file() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(from, to).map_err(|e| e.to_string())?;
        return Ok(());
    }
    for entry in WalkDir::new(from).into_iter().flatten() {
        let rel = entry.path().strip_prefix(from).unwrap_or(entry.path());
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(entry.path(), &dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn fixture_map() -> BTreeMap<String, PathBuf> {
    let mut map = BTreeMap::new();
    if let Ok(raw) = std::env::var("PI_PACKAGE_FIXTURE_MAP") {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            if let Some(obj) = value.as_object() {
                for (k, v) in obj {
                    if let Some(path) = v.as_str() {
                        map.insert(k.clone(), PathBuf::from(path));
                    }
                }
            }
        } else {
            let path = PathBuf::from(&raw);
            if path.is_file() {
                if let Ok(value) =
                    serde_json::from_str::<Value>(&fs::read_to_string(path).unwrap_or_default())
                {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            if let Some(p) = v.as_str() {
                                map.insert(k.clone(), PathBuf::from(p));
                            }
                        }
                    }
                }
            }
        }
    }
    map
}

fn fixture_dir() -> Option<PathBuf> {
    std::env::var("PI_PACKAGE_FIXTURE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn lookup_fixture(source: &str, parsed: &ParsedSource) -> Option<PathBuf> {
    let map = fixture_map();
    if let Some(path) = map.get(source) {
        if path.exists() {
            return Some(path.clone());
        }
    }
    let root = fixture_dir()?;
    match parsed {
        ParsedSource::Npm { name, spec, .. } => {
            for candidate in [
                root.join("npm").join(name),
                root.join(name),
                root.join("npm").join(spec),
                root.join(spec),
            ] {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            None
        }
        ParsedSource::Git { host, path, .. } => {
            let mut dir = root.join("git").join(host);
            for part in path.split('/') {
                dir = dir.join(part);
            }
            dir.exists().then_some(dir).or_else(|| {
                let alt = root.join(host).join(path);
                alt.exists().then_some(alt)
            })
        }
        ParsedSource::Local { .. } => None,
    }
}

fn spawn_allowed() -> bool {
    !network_disabled()
}

pub fn configured_npm_command() -> Option<Vec<String>> {
    SettingsDocument::load(&settings_path(false)).npm_command()
}

pub fn package_manager_name(argv: &[String]) -> String {
    if argv.is_empty() {
        return String::new();
    }
    let command = argv
        .iter()
        .rposition(|part| part == "--")
        .and_then(|idx| argv.get(idx + 1))
        .unwrap_or(&argv[0]);
    let base = Path::new(command)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(command);
    let lower = base.to_ascii_lowercase();
    lower
        .strip_suffix(".cmd")
        .or_else(|| lower.strip_suffix(".exe"))
        .unwrap_or(&lower)
        .to_string()
}

pub fn npm_install_args(specs: &[String], install_root: &Path, argv: &[String]) -> Vec<String> {
    let name = package_manager_name(argv);
    let root = install_root.to_string_lossy().into_owned();
    if name == "bun" {
        let mut args = vec!["install".into()];
        args.extend(specs.iter().cloned());
        args.extend(["--cwd".into(), root, "--omit=peer".into()]);
        return args;
    }
    if name == "pnpm" {
        let mut args = vec!["install".into()];
        args.extend(specs.iter().cloned());
        args.extend([
            "--prefix".into(),
            root,
            "--config.auto-install-peers=false".into(),
            "--config.strict-peer-dependencies=false".into(),
            "--config.strict-dep-builds=false".into(),
        ]);
        return args;
    }
    let mut args = vec!["install".into()];
    args.extend(specs.iter().cloned());
    args.extend(["--prefix".into(), root, "--legacy-peer-deps".into()]);
    args
}

pub fn git_dependency_install_args() -> Vec<String> {
    if configured_npm_command().is_some() {
        vec!["install".into()]
    } else {
        vec!["install".into(), "--omit=dev".into()]
    }
}

fn spawn_npm(args: &[String]) -> Result<(), String> {
    let argv = configured_npm_command().unwrap_or_else(|| vec!["npm".into()]);
    let (command, prefix) = argv
        .split_first()
        .ok_or("Invalid npmCommand: first array entry must be a non-empty command")?;
    if command.is_empty() {
        return Err("Invalid npmCommand: first array entry must be a non-empty command".into());
    }
    let status = Command::new(command)
        .args(prefix)
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "{} {} exited with code {}",
            command,
            args.join(" "),
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

fn install_npm_tree(source: &ParsedSource, local: bool) -> Result<PathBuf, String> {
    let ParsedSource::Npm { spec, name, .. } = source else {
        return Err("not an npm source".into());
    };
    let install_root = npm_install_root(local);
    ensure_npm_project(&install_root);
    let dest = npm_install_path(name, local);
    if let Some(fixture) = lookup_fixture(&format!("npm:{spec}"), source)
        .or_else(|| lookup_fixture(&format!("npm:{name}"), source))
    {
        if dest.exists() {
            let _ = fs::remove_dir_all(&dest);
        }
        copy_tree(&fixture, &dest)?;
        return Ok(dest);
    }
    if !spawn_allowed() {
        return Err(format!(
            "Network is disabled; cannot install npm:{spec}. Set PI_PACKAGE_FIXTURE to a local package tree."
        ));
    }
    let argv = configured_npm_command().unwrap_or_else(|| vec!["npm".into()]);
    let install_args = npm_install_args(&[spec.clone()], &install_root, &argv);
    spawn_npm(&install_args)?;
    Ok(dest)
}

fn install_git_tree(source: &ParsedSource, raw: &str, local: bool) -> Result<PathBuf, String> {
    let ParsedSource::Git {
        repo,
        host,
        path,
        git_ref,
        ..
    } = source
    else {
        return Err("not a git source".into());
    };
    let git_root = git_install_root(local);
    ensure_gitignore(&git_root);
    let dest = git_install_path(host, path, local);
    if let Some(fixture) = lookup_fixture(raw, source) {
        if dest.exists() {
            let _ = fs::remove_dir_all(&dest);
        }
        copy_tree(&fixture, &dest)?;
        return Ok(dest);
    }
    if dest.exists() {
        return Ok(dest);
    }
    if !spawn_allowed() {
        return Err(format!(
            "Network is disabled; cannot install {raw}. Set PI_PACKAGE_FIXTURE to a local package tree."
        ));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let status = Command::new("git")
        .args(["clone", repo])
        .arg(&dest)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = fs::remove_dir_all(&dest);
        return Err(format!(
            "git clone {repo} exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }
    if let Some(git_ref) = git_ref {
        let checkout = Command::new("git")
            .args(["checkout", git_ref])
            .current_dir(&dest)
            .status()
            .map_err(|e| e.to_string())?;
        if !checkout.success() {
            let _ = fs::remove_dir_all(&dest);
            return Err(format!("git checkout {git_ref} failed"));
        }
    }
    if dest.join("package.json").exists() {
        let argv = configured_npm_command().unwrap_or_else(|| vec!["npm".into()]);
        let (command, prefix) = argv
            .split_first()
            .ok_or("Invalid npmCommand: first array entry must be a non-empty command")?;
        let deps = git_dependency_install_args();
        let status = Command::new(command)
            .args(prefix)
            .args(&deps)
            .current_dir(&dest)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            let _ = fs::remove_dir_all(&dest);
            return Err(format!(
                "{} {} exited with code {}",
                command,
                deps.join(" "),
                status.code().unwrap_or(-1)
            ));
        }
    }
    Ok(dest)
}

fn persist_source(source: &str, local: bool) {
    let mut doc = SettingsDocument::load(&settings_path(local));
    if !doc
        .packages()
        .iter()
        .any(|pkg| package_source_string(pkg).as_deref() == Some(source))
    {
        doc.packages_mut().push(json!(source));
        doc.save(&settings_path(local));
    }
}

fn remove_source_from_settings(source: &str, local: bool) -> bool {
    let mut doc = SettingsDocument::load(&settings_path(local));
    let before = doc.packages().len();
    doc.packages_mut()
        .retain(|pkg| package_source_string(pkg).as_deref() != Some(source));
    let changed = doc.packages().len() != before;
    if changed {
        doc.save(&settings_path(local));
    }
    changed
}

pub fn install_and_persist(source: &str, local: bool) -> Result<PathBuf, String> {
    let parsed = parse_source(source);
    let installed = match &parsed {
        ParsedSource::Local { path } => {
            let resolved = expand_install_path(path);
            if !resolved.exists() {
                return Err(format!("Path does not exist: {}", resolved.display()));
            }
            resolved
        }
        ParsedSource::Npm { .. } => install_npm_tree(&parsed, local)?,
        ParsedSource::Git { .. } => install_git_tree(&parsed, source, local)?,
    };
    persist_source(source, local);
    Ok(installed)
}

fn prune_empty_parents(start: &Path, root: &Path) {
    let Ok(root) = root.canonicalize() else {
        return;
    };
    let mut current = start.parent().map(Path::to_path_buf);
    while let Some(dir) = current {
        let Ok(resolved) = dir.canonicalize() else {
            break;
        };
        if resolved == root || !resolved.starts_with(&root) {
            break;
        }
        let empty = fs::read_dir(&resolved)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if !empty {
            break;
        }
        let _ = fs::remove_dir(&resolved);
        current = resolved.parent().map(Path::to_path_buf);
    }
}

pub fn remove_and_persist(source: &str, local: bool) -> Result<bool, String> {
    let parsed = parse_source(source);
    match parsed {
        ParsedSource::Npm { name, .. } => {
            let dest = npm_install_path(&name, local);
            if dest.exists() {
                let _ = fs::remove_dir_all(&dest);
            }
        }
        ParsedSource::Git { host, path, .. } => {
            let dest = git_install_path(&host, &path, local);
            if dest.exists() {
                let _ = fs::remove_dir_all(&dest);
            }
            prune_empty_parents(&dest, &git_install_root(local));
        }
        ParsedSource::Local { .. } => {}
    }
    Ok(remove_source_from_settings(source, local))
}

pub fn list_configured_packages(include_project: bool) -> Vec<ConfiguredPackage> {
    let mut out = Vec::new();
    let scopes = if include_project {
        vec![(false, "user"), (true, "project")]
    } else {
        vec![(false, "user")]
    };
    for (local, scope) in scopes {
        let doc = SettingsDocument::load(&settings_path(local));
        for pkg in doc.packages() {
            let Some(source) = package_source_string(&pkg) else {
                continue;
            };
            out.push(ConfiguredPackage {
                source: source.clone(),
                scope: scope.to_string(),
                filtered: package_is_object(&pkg),
                installed_path: get_installed_path(&source, local),
            });
        }
    }
    out
}

pub fn format_package_list(packages: &[ConfiguredPackage]) -> String {
    if packages.is_empty() {
        return "No packages installed.\n".into();
    }
    let user: Vec<_> = packages.iter().filter(|p| p.scope == "user").collect();
    let project: Vec<_> = packages.iter().filter(|p| p.scope == "project").collect();
    let mut out = String::new();
    if !user.is_empty() {
        out.push_str("User packages:\n");
        for pkg in user {
            append_package_line(&mut out, pkg);
        }
    }
    if !project.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Project packages:\n");
        for pkg in project {
            append_package_line(&mut out, pkg);
        }
    }
    out
}

fn append_package_line(out: &mut String, pkg: &ConfiguredPackage) {
    if pkg.filtered {
        out.push_str(&format!("  {} (filtered)\n", pkg.source));
    } else {
        out.push_str(&format!("  {}\n", pkg.source));
    }
    if let Some(path) = &pkg.installed_path {
        out.push_str(&format!("    {}\n", path.display()));
    }
}

pub fn package_identity(source: &str) -> String {
    match parse_source(source) {
        ParsedSource::Npm { name, .. } => format!("npm:{name}"),
        ParsedSource::Git { host, path, .. } => format!("git:{host}/{path}"),
        ParsedSource::Local { path } => format!("local:{}", expand_install_path(&path).display()),
    }
}

pub fn update_configured(
    source: Option<&str>,
    include_project: bool,
) -> Result<Vec<String>, String> {
    if let Some(source) = source {
        let identity = package_identity(source);
        let packages = list_configured_packages(include_project);
        if !packages
            .iter()
            .any(|pkg| package_identity(&pkg.source) == identity)
        {
            return Err(build_no_matching_package_message(source, &packages));
        }
    }
    if is_offline_mode_enabled() {
        return Ok(Vec::new());
    }
    let packages = list_configured_packages(include_project);
    let mut updated = Vec::new();
    for pkg in packages {
        if let Some(source) = source {
            if package_identity(&pkg.source) != package_identity(source) {
                continue;
            }
        }
        let local = pkg.scope == "project";
        match parse_source(&pkg.source) {
            ParsedSource::Npm { .. } | ParsedSource::Git { .. } => {
                install_and_persist(&pkg.source, local)?;
                updated.push(pkg.source);
            }
            ParsedSource::Local { .. } => {}
        }
    }
    Ok(updated)
}

fn build_no_matching_package_message(source: &str, packages: &[ConfiguredPackage]) -> String {
    let trimmed = source.trim();
    for pkg in packages {
        match parse_source(&pkg.source) {
            ParsedSource::Npm { name, spec, .. } if trimmed == name || trimmed == spec => {
                return format!(
                    "No matching package found for {source}. Did you mean {}?",
                    pkg.source
                );
            }
            ParsedSource::Git {
                host,
                path,
                git_ref,
                ..
            } => {
                let shorthand = format!("{host}/{path}");
                let with_ref = git_ref
                    .as_ref()
                    .map(|r| format!("{shorthand}@{r}"))
                    .unwrap_or_default();
                if trimmed == shorthand || (!with_ref.is_empty() && trimmed == with_ref) {
                    return format!(
                        "No matching package found for {source}. Did you mean {}?",
                        pkg.source
                    );
                }
            }
            _ => {}
        }
    }
    format!("No matching package found for {source}")
}

fn read_pi_manifest(package_root: &Path) -> Option<BTreeMap<String, Vec<String>>> {
    let pkg = package_root.join("package.json");
    let raw = fs::read_to_string(pkg).ok()?;
    let value: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let pi = value.get("pi")?.as_object()?;
    let mut manifest = BTreeMap::new();
    for field in RESOURCE_TYPES {
        if let Some(arr) = pi.get(field).and_then(|v| v.as_array()) {
            let entries: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !entries.is_empty() {
                manifest.insert(field.to_string(), entries);
            }
        }
    }
    if manifest.is_empty() {
        None
    } else {
        Some(manifest)
    }
}

fn collect_files(dir: &Path, resource_type: &str) -> Vec<PathBuf> {
    match resource_type {
        "skills" => collect_skill_entries(dir),
        "extensions" => collect_auto_extension_entries(dir),
        "prompts" => collect_named(dir, "md", false),
        "themes" => collect_named(dir, "json", false),
        _ => Vec::new(),
    }
}

fn collect_named(dir: &Path, ext: &str, recursive: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return files;
    }
    let walker = if recursive {
        WalkDir::new(dir)
    } else {
        WalkDir::new(dir).max_depth(1)
    };
    for entry in walker.into_iter().flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        if path.components().any(|c| c.as_os_str() == "node_modules") {
            continue;
        }
        if entry.file_type().is_file() && path.extension().and_then(|s| s.to_str()) == Some(ext) {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    files
}

fn collect_skill_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    collect_skill_entries_inner(dir, dir, &mut entries);
    entries
}

fn collect_skill_entries_inner(dir: &Path, root: &Path, entries: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let listed: Vec<_> = read.flatten().collect();
    for entry in &listed {
        if entry.file_name() == "SKILL.md" {
            let path = entry.path();
            if path.is_file() {
                entries.push(path);
                return;
            }
        }
    }
    for entry in listed {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if path.is_file() && name.ends_with(".md") && dir == root {
            entries.push(path);
        } else if path.is_dir() {
            collect_skill_entries_inner(&path, root, entries);
        }
    }
}

fn resolve_extension_entries(dir: &Path) -> Option<Vec<PathBuf>> {
    if let Some(manifest) = read_pi_manifest(dir) {
        if let Some(exts) = manifest.get("extensions") {
            let resolved: Vec<PathBuf> = exts
                .iter()
                .map(|p| dir.join(p))
                .filter(|p| p.exists())
                .collect();
            if !resolved.is_empty() {
                return Some(resolved);
            }
        }
    }
    let index_ts = dir.join("index.ts");
    if index_ts.exists() {
        return Some(vec![index_ts]);
    }
    let index_js = dir.join("index.js");
    if index_js.exists() {
        return Some(vec![index_js]);
    }
    None
}

fn collect_auto_extension_entries(dir: &Path) -> Vec<PathBuf> {
    if let Some(root) = resolve_extension_entries(dir) {
        return root;
    }
    let mut entries = Vec::new();
    let Ok(read) = fs::read_dir(dir) else {
        return entries;
    };
    for entry in read.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if path.is_file() && (name.ends_with(".ts") || name.ends_with(".js")) {
            entries.push(path);
        } else if path.is_dir() {
            if let Some(resolved) = resolve_extension_entries(&path) {
                entries.extend(resolved);
            }
        }
    }
    entries.sort();
    entries
}

fn to_posix_rel(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn glob_to_regex(pattern: &str) -> Option<regex::Regex> {
    let mut out = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                out.push_str(".*");
                i += 2;
                if i < chars.len() && chars[i] == '/' {
                    i += 1;
                }
            }
            '*' => {
                out.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                out.push_str("[^/]");
                i += 1;
            }
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '[' | ']' | '{' | '}' | '\\' => {
                out.push('\\');
                out.push(chars[i]);
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out.push('$');
    regex::Regex::new(&out).ok()
}

fn matches_pattern(file_path: &Path, pattern: &str, base_dir: &Path) -> bool {
    let rel = to_posix_rel(base_dir, file_path);
    let name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let posix = file_path.to_string_lossy().replace('\\', "/");
    if let Some(re) = glob_to_regex(&pattern.replace('\\', "/")) {
        if re.is_match(&rel) || re.is_match(name) || re.is_match(&posix) {
            return true;
        }
    }
    rel == pattern || name == pattern || posix == pattern
}

fn apply_patterns(
    all_paths: &[PathBuf],
    patterns: &[String],
    base_dir: &Path,
) -> BTreeSet<PathBuf> {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut force_includes = Vec::new();
    let mut force_excludes = Vec::new();
    for p in patterns {
        if let Some(rest) = p.strip_prefix('+') {
            force_includes.push(rest.to_string());
        } else if let Some(rest) = p.strip_prefix('-') {
            force_excludes.push(rest.to_string());
        } else if let Some(rest) = p.strip_prefix('!') {
            excludes.push(rest.to_string());
        } else {
            includes.push(p.clone());
        }
    }
    let mut result: Vec<PathBuf> = if includes.is_empty() {
        all_paths.to_vec()
    } else {
        all_paths
            .iter()
            .filter(|p| includes.iter().any(|pat| matches_pattern(p, pat, base_dir)))
            .cloned()
            .collect()
    };
    result.retain(|p| !excludes.iter().any(|pat| matches_pattern(p, pat, base_dir)));
    for p in all_paths {
        if force_includes
            .iter()
            .any(|pat| to_posix_rel(base_dir, p) == *pat || p.to_string_lossy() == *pat)
            && !result.contains(p)
        {
            result.push(p.clone());
        }
    }
    result.retain(|p| {
        !force_excludes
            .iter()
            .any(|pat| to_posix_rel(base_dir, p) == *pat || p.to_string_lossy() == *pat)
    });
    result.into_iter().collect()
}

fn package_filter(pkg: &Value) -> Option<BTreeMap<String, Vec<String>>> {
    let obj = pkg.as_object()?;
    let mut filter = BTreeMap::new();
    let mut any = obj.get("autoload").and_then(|v| v.as_bool()) == Some(false);
    for field in RESOURCE_TYPES {
        if let Some(arr) = obj.get(field).and_then(|v| v.as_array()) {
            filter.insert(
                field.to_string(),
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
            );
            any = true;
        }
    }
    if obj.get("autoload").and_then(|v| v.as_bool()) == Some(false) {
        filter.insert("autoload".into(), vec!["false".into()]);
    }
    any.then_some(filter)
}

fn collect_package_resources(
    package_root: &Path,
    filter: Option<&BTreeMap<String, Vec<String>>>,
    metadata: &PathMetadata,
) -> ResolvedPaths {
    let mut resolved = ResolvedPaths::default();
    if let Some(filter) = filter {
        let autoload_false = filter
            .get("autoload")
            .is_some_and(|v| v.first().is_some_and(|s| s == "false"));
        for resource_type in RESOURCE_TYPES {
            let files = collect_default_files(package_root, resource_type);
            let patterns = filter.get(resource_type);
            let enabled_set = if autoload_false {
                if let Some(patterns) = patterns {
                    apply_patterns(&files, patterns, package_root)
                } else {
                    BTreeSet::new()
                }
            } else if let Some(patterns) = patterns {
                apply_patterns(&files, patterns, package_root)
            } else {
                files.iter().cloned().collect()
            };
            for file in files {
                let enabled = enabled_set.contains(&file);
                if autoload_false && patterns.map(|p| p.is_empty()).unwrap_or(true) && !enabled {
                    continue;
                }
                push_resource(&mut resolved, resource_type, file, enabled, metadata);
            }
        }
        return resolved;
    }
    if let Some(manifest) = read_pi_manifest(package_root) {
        for resource_type in RESOURCE_TYPES {
            if let Some(entries) = manifest.get(resource_type) {
                for entry in entries {
                    let path = package_root.join(entry);
                    if path.exists() {
                        push_resource(&mut resolved, resource_type, path, true, metadata);
                    } else {
                        for file in collect_default_files(package_root, resource_type) {
                            push_resource(&mut resolved, resource_type, file, true, metadata);
                        }
                    }
                }
            } else {
                for file in collect_default_files(package_root, resource_type) {
                    push_resource(&mut resolved, resource_type, file, true, metadata);
                }
            }
        }
        return resolved;
    }
    for resource_type in RESOURCE_TYPES {
        let dir = package_root.join(resource_type);
        if dir.is_dir() {
            for file in collect_files(&dir, resource_type) {
                push_resource(&mut resolved, resource_type, file, true, metadata);
            }
        }
    }
    resolved
}

fn collect_default_files(package_root: &Path, resource_type: &str) -> Vec<PathBuf> {
    if let Some(manifest) = read_pi_manifest(package_root) {
        if let Some(entries) = manifest.get(resource_type) {
            return entries
                .iter()
                .map(|e| package_root.join(e))
                .filter(|p| p.exists())
                .collect();
        }
    }
    let dir = package_root.join(resource_type);
    if dir.is_dir() {
        collect_files(&dir, resource_type)
    } else {
        Vec::new()
    }
}

fn push_resource(
    resolved: &mut ResolvedPaths,
    resource_type: &str,
    path: PathBuf,
    enabled: bool,
    metadata: &PathMetadata,
) {
    let resource = ResolvedResource {
        path,
        enabled,
        metadata: metadata.clone(),
    };
    match resource_type {
        "extensions" => resolved.extensions.push(resource),
        "skills" => resolved.skills.push(resource),
        "prompts" => resolved.prompts.push(resource),
        "themes" => resolved.themes.push(resource),
        _ => {}
    }
}

fn merge_resolved(into: &mut ResolvedPaths, from: ResolvedPaths) {
    into.extensions.extend(from.extensions);
    into.skills.extend(from.skills);
    into.prompts.extend(from.prompts);
    into.themes.extend(from.themes);
}

fn add_auto_discovered(resolved: &mut ResolvedPaths, base_dir: &Path, scope: &str) {
    let metadata = PathMetadata {
        source: "auto".into(),
        scope: scope.into(),
        origin: "top-level".into(),
        base_dir: Some(base_dir.to_path_buf()),
    };
    for resource_type in RESOURCE_TYPES {
        let dir = base_dir.join(resource_type);
        if dir.is_dir() {
            for file in collect_files(&dir, resource_type) {
                push_resource(resolved, resource_type, file, true, &metadata);
            }
        }
    }
}

fn add_settings_entries(
    resolved: &mut ResolvedPaths,
    doc: &SettingsDocument,
    base_dir: &Path,
    scope: &str,
) {
    let metadata = PathMetadata {
        source: "local".into(),
        scope: scope.into(),
        origin: "top-level".into(),
        base_dir: Some(base_dir.to_path_buf()),
    };
    for resource_type in RESOURCE_TYPES {
        let entries = doc.resource_paths(resource_type);
        if entries.is_empty() {
            continue;
        }
        let mut files = Vec::new();
        for entry in &entries {
            if entry.starts_with('!') || entry.starts_with('+') || entry.starts_with('-') {
                continue;
            }
            let path = if Path::new(entry).is_absolute() {
                PathBuf::from(entry)
            } else {
                base_dir.join(entry)
            };
            if path.is_dir() {
                files.extend(collect_files(&path, resource_type));
            } else if path.exists() {
                files.push(path);
            }
        }
        let enabled = apply_patterns(&files, &entries, base_dir);
        for file in files {
            let on = enabled.contains(&file);
            push_resource(resolved, resource_type, file, on, &metadata);
        }
    }
}

pub fn resolve_resources(agent: &Path, project_cwd: &Path, project_trusted: bool) -> ResolvedPaths {
    let mut resolved = ResolvedPaths::default();
    let global = SettingsDocument::load(&agent.join("settings.json"));
    let project_base = project_cwd.join(".pi");
    let project = if project_trusted {
        SettingsDocument::load(&project_base.join("settings.json"))
    } else {
        SettingsDocument::default()
    };

    let mut packages: Vec<(Value, bool)> = Vec::new();
    if project_trusted {
        for pkg in project.packages() {
            packages.push((pkg.clone(), true));
        }
    }
    for pkg in global.packages() {
        packages.push((pkg.clone(), false));
    }

    let mut seen = BTreeSet::new();
    for (pkg, local) in packages {
        let Some(source) = package_source_string(&pkg) else {
            continue;
        };
        let identity = package_identity(&source);
        if !seen.insert(identity) {
            continue;
        }
        let Some(root) = get_installed_path(&source, local) else {
            continue;
        };
        let metadata = PathMetadata {
            source: source.clone(),
            scope: if local { "project" } else { "user" }.into(),
            origin: "package".into(),
            base_dir: Some(root.clone()),
        };
        let filter = package_filter(&pkg);
        merge_resolved(
            &mut resolved,
            collect_package_resources(&root, filter.as_ref(), &metadata),
        );
    }

    if project_trusted {
        add_settings_entries(&mut resolved, &project, &project_base, "project");
        add_auto_discovered(&mut resolved, &project_base, "project");
    }
    add_settings_entries(&mut resolved, &global, agent, "user");
    add_auto_discovered(&mut resolved, agent, "user");
    resolved
}

pub fn resolve_current(project_trusted: bool) -> ResolvedPaths {
    resolve_resources(&agent_dir(), &cwd(), project_trusted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectTrustMode {
    Full,
    SavedOnly,
}

pub fn project_is_trusted(approve: Option<bool>, mode: ProjectTrustMode) -> bool {
    if let Some(flag) = approve {
        return flag;
    }
    let cwd = cwd();
    let agent = agent_dir();
    if mode == ProjectTrustMode::SavedOnly {
        return settings::trust_decision(&agent, &cwd) == Some(true);
    }
    if !settings::has_trust_requiring_project_resources(&cwd) {
        return true;
    }
    if let Some(decision) = settings::trust_decision(&agent, &cwd) {
        return decision;
    }
    SettingsDocument::load(&settings_path(false)).default_project_trust() == "always"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_source_matches_typescript() {
        assert!(matches!(
            parse_source("npm:left-pad"),
            ParsedSource::Npm { ref name, .. } if name == "left-pad"
        ));
        assert!(matches!(
            parse_source("npm:@foo/bar@1.2.3"),
            ParsedSource::Npm { ref name, ref version, pinned, .. }
                if name == "@foo/bar" && version.as_deref() == Some("1.2.3") && pinned
        ));
        assert!(matches!(
            parse_source("git:github.com/user/repo@v1"),
            ParsedSource::Git { ref host, ref path, ref git_ref, pinned, .. }
                if host == "github.com" && path == "user/repo" && git_ref.as_deref() == Some("v1") && pinned
        ));
        assert!(matches!(
            parse_source("https://github.com/user/repo@v1"),
            ParsedSource::Git { ref host, ref path, .. }
                if host == "github.com" && path == "user/repo"
        ));
        assert!(matches!(
            parse_source("git:git@github.com:user/repo@v1"),
            ParsedSource::Git { ref host, ref path, ref git_ref, .. }
                if host == "github.com" && path == "user/repo" && git_ref.as_deref() == Some("v1")
        ));
        assert!(matches!(
            parse_source("ssh://git@github.com/user/repo@v1"),
            ParsedSource::Git { ref host, ref path, .. }
                if host == "github.com" && path == "user/repo"
        ));
        assert!(matches!(
            parse_source("/absolute/path/to/package"),
            ParsedSource::Local { .. }
        ));
        assert!(matches!(
            parse_source("./relative/path/to/package"),
            ParsedSource::Local { .. }
        ));
        assert!(is_local_path("my-package"));
        assert!(!is_local_path("npm:package"));
        assert!(!is_local_path("git://repo"));
        assert!(!is_local_path("https://example.com"));
    }

    #[test]
    fn fixture_npm_install_writes_managed_tree() {
        let dir = tempdir().unwrap();
        let fixture = dir.path().join("fixture").join("npm").join("pi-fixture");
        fs::create_dir_all(fixture.join("extensions")).unwrap();
        fs::write(
            fixture.join("extensions").join("index.ts"),
            "export default function () {}",
        )
        .unwrap();
        fs::write(
            fixture.join("package.json"),
            r#"{"name":"pi-fixture","version":"1.0.0"}"#,
        )
        .unwrap();
        let agent = dir.path().join("agent");
        fs::create_dir_all(&agent).unwrap();
        let _lock = crate::settings::test_env_lock();
        let previous_agent = std::env::var("PI_CODING_AGENT_DIR").ok();
        let previous_fixture = std::env::var("PI_PACKAGE_FIXTURE").ok();
        let previous_net = std::env::var("PI_DISABLE_NETWORK").ok();
        let previous_cwd = std::env::current_dir().ok();
        std::env::set_var("PI_CODING_AGENT_DIR", &agent);
        let _ = std::env::set_current_dir(dir.path());
        std::env::set_var("PI_PACKAGE_FIXTURE", dir.path().join("fixture"));
        std::env::set_var("PI_DISABLE_NETWORK", "1");
        let installed = install_and_persist("npm:pi-fixture", false).unwrap();
        assert_eq!(
            installed,
            agent.join("npm").join("node_modules").join("pi-fixture")
        );
        assert!(installed.join("extensions").join("index.ts").exists());
        assert!(agent.join("npm").join("package.json").exists());
        let listed = list_configured_packages(true);
        assert_eq!(listed[0].source, "npm:pi-fixture");
        assert_eq!(listed[0].installed_path.as_ref(), Some(&installed));
        let resolved = resolve_resources(&agent, dir.path(), false);
        assert!(resolved
            .extensions
            .iter()
            .any(|r| r.path.ends_with("extensions/index.ts") && r.enabled));
        assert!(remove_and_persist("npm:pi-fixture", false).unwrap());
        assert!(!installed.exists());
        match previous_agent {
            Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
        match previous_fixture {
            Some(v) => std::env::set_var("PI_PACKAGE_FIXTURE", v),
            None => std::env::remove_var("PI_PACKAGE_FIXTURE"),
        }
        match previous_net {
            Some(v) => std::env::set_var("PI_DISABLE_NETWORK", v),
            None => std::env::remove_var("PI_DISABLE_NETWORK"),
        }
        if let Some(cwd) = previous_cwd {
            let _ = std::env::set_current_dir(cwd);
        }
    }

    #[test]
    fn npm_and_git_install_args_match_typescript_package_managers() {
        let root = PathBuf::from("/tmp/pi-npm");
        let spec = vec!["@foo/bar".into()];
        assert_eq!(
            npm_install_args(&spec, &root, &["npm".into()]),
            vec![
                "install",
                "@foo/bar",
                "--prefix",
                "/tmp/pi-npm",
                "--legacy-peer-deps"
            ]
        );
        assert_eq!(
            npm_install_args(&spec, &root, &["bun".into()]),
            vec!["install", "@foo/bar", "--cwd", "/tmp/pi-npm", "--omit=peer"]
        );
        assert_eq!(
            npm_install_args(&spec, &root, &["pnpm".into()]),
            vec![
                "install",
                "@foo/bar",
                "--prefix",
                "/tmp/pi-npm",
                "--config.auto-install-peers=false",
                "--config.strict-peer-dependencies=false",
                "--config.strict-dep-builds=false"
            ]
        );
        assert_eq!(
            package_manager_name(&[
                "mise".into(),
                "exec".into(),
                "bun@1".into(),
                "--".into(),
                "bun".into()
            ]),
            "bun"
        );
        let _lock = crate::settings::test_env_lock();
        let dir = tempdir().unwrap();
        let previous_agent = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        assert_eq!(
            git_dependency_install_args(),
            vec!["install".to_string(), "--omit=dev".into()]
        );
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"npmCommand":["pnpm"]}"#,
        )
        .unwrap();
        assert_eq!(git_dependency_install_args(), vec!["install".to_string()]);
        match previous_agent {
            Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }
}
