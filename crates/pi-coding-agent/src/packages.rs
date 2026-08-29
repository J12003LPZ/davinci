use std::fs;
use std::path::{Path, PathBuf};

use crate::args::{APP_NAME, VERSION};
use crate::self_update::{
    current_install_method, inferred_npm_prefix, package_dir_from_env,
    self_update_command_for_method, self_update_unavailable_instruction, update_instruction,
    InstallMethod, PackageTarget, PACKAGE_NAME,
};
use crate::settings::{load_settings, save_settings, PackageSource, Settings};
use pi_tui::{Component, ConfigResource, ConfigResourceKind, ConfigScope, ConfigSelector};

const CANNOT_SELF_UPDATE: &str = "error: pi cannot self-update this installation.";
const ALL_CONFLICT: &str =
    "--all cannot be combined with --self, --extensions, --models, or --extension";
const MODELS_CONFLICT: &str =
    "--models cannot be combined with --self, --extensions, --all, or --extension";
const EXTENSION_CONFLICT: &str =
    "--extension cannot be combined with --self, --extensions, or --all";
const POSITIONAL_CONFLICT: &str =
    "positional update targets cannot be combined with --self, --extensions, or --all";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct UpdateFlags {
    self_flag: bool,
    extensions_flag: bool,
    models_flag: bool,
    all_flag: bool,
    extension: Option<String>,
    positional: Option<String>,
    force: bool,
}

pub fn handle_package_command(
    command: &str,
    args: &[String],
    agent_dir: &Path,
) -> Result<String, String> {
    let local = args.iter().any(|a| a == "-l" || a == "--local");
    let source = args.iter().find(|a| !a.starts_with('-')).cloned();
    let mut settings = load_settings(agent_dir);
    match command {
        "install" => {
            let source = source.ok_or("install <source> [-l]")?;
            let installed = install_and_persist(&source, local, agent_dir)?;
            Ok(format!("Installed {installed}{}", scope(local)))
        }
        "remove" | "uninstall" => {
            let source = source.ok_or("remove <source> [-l]")?;
            settings.extensions.retain(|item| item != &source);
            settings.packages.retain(|item| item.source() != source);
            save_settings(agent_dir, &settings)?;
            Ok(format!("Removed {source}{}", scope(local)))
        }
        "update" => handle_update(args, agent_dir),
        "list" => Ok(render_list(&settings)),
        "config" => render_config_command(&settings, local, agent_dir),
        _ => Err(format!("Unknown command {command}")),
    }
}

fn parse_update_flags(args: &[String]) -> Result<UpdateFlags, String> {
    let mut flags = UpdateFlags::default();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--self" => flags.self_flag = true,
            "--extensions" => flags.extensions_flag = true,
            "--models" => flags.models_flag = true,
            "--all" => flags.all_flag = true,
            "--force" => flags.force = true,
            "--extension" => {
                index += 1;
                flags.extension = args.get(index).cloned();
            }
            "-h" | "--help" => {}
            other if other.starts_with('-') => {}
            other => {
                if flags.positional.is_none() {
                    flags.positional = Some(other.to_string());
                }
            }
        }
        index += 1;
    }
    if flags.all_flag
        && (flags.self_flag
            || flags.extensions_flag
            || flags.models_flag
            || flags.extension.is_some())
    {
        return Err(ALL_CONFLICT.into());
    }
    if flags.models_flag
        && (flags.self_flag || flags.extensions_flag || flags.all_flag || flags.extension.is_some())
    {
        return Err(MODELS_CONFLICT.into());
    }
    if flags.extension.is_some() && (flags.self_flag || flags.extensions_flag || flags.all_flag) {
        return Err(EXTENSION_CONFLICT.into());
    }
    if flags.positional.is_some() && (flags.self_flag || flags.extensions_flag || flags.all_flag) {
        return Err(POSITIONAL_CONFLICT.into());
    }
    Ok(flags)
}

fn handle_update(args: &[String], agent_dir: &Path) -> Result<String, String> {
    let flags = parse_update_flags(args)?;
    let positional_self = matches!(flags.positional.as_deref(), Some("self") | Some("pi"));
    let extension_source = flags.extension.clone().or_else(|| {
        flags
            .positional
            .clone()
            .filter(|value| !matches!(value.as_str(), "self" | "pi"))
    });
    let do_self = flags.self_flag
        || positional_self
        || flags.all_flag
        || !flags.models_flag
            && !flags.extensions_flag
            && flags.extension.is_none()
            && flags.positional.is_none();
    let do_models = flags.models_flag || flags.all_flag;
    let do_extensions = flags.extensions_flag || flags.all_flag || extension_source.is_some();
    let mut parts = Vec::new();
    if do_self {
        parts.push(self_update_binary(agent_dir, flags.force)?);
    }
    if do_models {
        parts.push(refresh_model_catalogs(
            agent_dir,
            flags.positional.as_deref(),
        )?);
    }
    if do_extensions {
        parts.push(update_extensions(
            agent_dir,
            extension_source.as_deref(),
            flags.force,
        )?);
    }
    Ok(parts.join("\n"))
}

/// TS `packageManager.update` for `--extensions` / `--all`: reinstall unpinned npm/git packages.
pub fn update_extensions(
    agent_dir: &Path,
    source: Option<&str>,
    _force: bool,
) -> Result<String, String> {
    if std::env::var("PI_OFFLINE").is_ok() || std::env::var("PI_INSTALL_DRY_RUN").is_ok() {
        return Ok(source
            .map(|value| format!("Updated {value}"))
            .unwrap_or_else(|| "Updated packages".into()));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let user = load_settings(agent_dir);
    let project = load_settings(&cwd.join(".pi"));
    let mut targets = Vec::new();
    for (pkg, local) in user
        .packages
        .iter()
        .map(|pkg| (pkg, false))
        .chain(project.packages.iter().map(|pkg| (pkg, true)))
    {
        let spec = pkg.source();
        if let Some(filter) = source {
            if package_identity(spec) != package_identity(filter) && spec != filter {
                continue;
            }
        }
        match parse_package_source(spec) {
            ParsedSource::Local(_) => {}
            ParsedSource::Npm { spec, name } => {
                if npm_spec_version(&spec, &name)
                    .as_deref()
                    .is_some_and(is_exact_npm_version)
                {
                    continue;
                }
                install_remote_package(agent_dir, "npm", &name, &spec, local)?;
                targets.push(name);
            }
            ParsedSource::Git(url) => {
                let name = git_package_name(&url);
                install_remote_package(agent_dir, "git", &name, &url, local)?;
                targets.push(git_host_path(&url));
            }
        }
    }
    if let Some(filter) = source {
        if targets.is_empty() {
            return Err(format!("No matching package found for {filter}"));
        }
        return Ok(format!("Updated {}", source.unwrap_or("packages")));
    }
    Ok("Updated packages".into())
}

pub fn persist_config_toggle(
    agent_dir: &Path,
    local: bool,
    source: &str,
    enabled: bool,
) -> Result<(), String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dir = if local {
        cwd.join(".pi")
    } else {
        agent_dir.to_path_buf()
    };
    let mut settings = load_settings(&dir);
    apply_resource_enabled(&mut settings, source, enabled);
    save_settings(&dir, &settings)
}

pub fn apply_resource_enabled(settings: &mut Settings, source: &str, enabled: bool) {
    let mut found = false;
    for pkg in &mut settings.packages {
        if pkg.source() != source {
            continue;
        }
        found = true;
        *pkg = match pkg.clone() {
            PackageSource::Spec(spec) if enabled => PackageSource::Spec(spec),
            PackageSource::Spec(spec) => PackageSource::Filtered(crate::settings::PackageFilter {
                source: spec,
                autoload: Some(false),
                ..crate::settings::PackageFilter::default()
            }),
            PackageSource::Filtered(mut filter) => {
                filter.autoload = Some(enabled);
                if enabled
                    && filter.extensions.is_none()
                    && filter.skills.is_none()
                    && filter.prompts.is_none()
                    && filter.themes.is_none()
                {
                    PackageSource::Spec(filter.source)
                } else {
                    PackageSource::Filtered(filter)
                }
            }
        };
    }
    if !found && !source.is_empty() {
        settings.packages.push(if enabled {
            PackageSource::from_spec(source)
        } else {
            PackageSource::Filtered(crate::settings::PackageFilter {
                source: source.to_string(),
                autoload: Some(false),
                ..crate::settings::PackageFilter::default()
            })
        });
    }
}

fn refresh_model_catalogs(agent_dir: &Path, _target: Option<&str>) -> Result<String, String> {
    let mut result = crate::catalog_refresh::refresh_model_catalogs(agent_dir, true, true);
    let stored = load_settings(agent_dir);
    let host = crate::extension_host::ExtensionHost::load(agent_dir, &stored.extensions);
    crate::catalog_refresh::refresh_js_providers(
        &mut result,
        &host.js_refresh_providers(),
        true,
        true,
    );
    crate::catalog_refresh::cli_refresh_message(&result)
}

/// TS `pi update --self`: package-manager argv when npm/pnpm/yarn/bun; copy `~/.pi/bin/pi` otherwise.
pub fn self_update_binary(agent_dir: &Path, _force: bool) -> Result<String, String> {
    let method = current_install_method();
    let target = PackageTarget::new(PACKAGE_NAME, None);
    let package_dir = package_dir_from_env();
    let windows = cfg!(windows)
        || package_dir
            .as_ref()
            .is_some_and(|path| path.to_string_lossy().contains('\\'));
    let inferred = package_dir
        .as_ref()
        .and_then(|path| inferred_npm_prefix(&path.to_string_lossy(), windows));
    let command = self_update_command_for_method(
        method,
        PACKAGE_NAME,
        &target,
        None,
        inferred.as_deref(),
        std::env::var("PNPM_HOME").ok().as_deref(),
    );
    match method {
        InstallMethod::Npm | InstallMethod::Pnpm | InstallMethod::Yarn | InstallMethod::Bun => {
            if let Some(command) = command.as_ref() {
                let writable = package_dir
                    .as_ref()
                    .map(|path| {
                        path.metadata()
                            .map(|meta| !meta.permissions().readonly())
                            .unwrap_or(false)
                    })
                    .unwrap_or(true);
                if !writable {
                    return Err(self_update_unavailable_instruction(
                        method,
                        PACKAGE_NAME,
                        &target,
                        Some(command),
                        true,
                        false,
                    ));
                }
                if std::env::var("PI_SELF_UPDATE_DRY_RUN").is_ok() {
                    return Ok(update_instruction(method, Some(command), &command.display));
                }
                run_self_update_command(command)?;
                return Ok(format!("Updated {APP_NAME} from {VERSION} to {VERSION}"));
            }
            Err(self_update_unavailable_instruction(
                method,
                PACKAGE_NAME,
                &target,
                None,
                false,
                true,
            ))
        }
        InstallMethod::BunBinary => Err(self_update_unavailable_instruction(
            method,
            PACKAGE_NAME,
            &target,
            None,
            false,
            true,
        )),
        InstallMethod::Unknown => copy_native_binary(agent_dir),
    }
}

fn run_self_update_command(command: &crate::self_update::SelfUpdateCommand) -> Result<(), String> {
    let steps = command.steps.clone().unwrap_or_else(|| {
        vec![crate::self_update::SelfUpdateCommandStep {
            command: command.command.clone(),
            args: command.args.clone(),
            display: command.display.clone(),
        }]
    });
    for step in steps {
        let status = std::process::Command::new(&step.command)
            .args(&step.args)
            .status()
            .map_err(|err| err.to_string())?;
        if !status.success() {
            return Err(format!("self-update failed: {}", step.display));
        }
    }
    Ok(())
}

fn copy_native_binary(agent_dir: &Path) -> Result<String, String> {
    let dest_dir = managed_bin_dir(agent_dir);
    fs::create_dir_all(&dest_dir).map_err(|err| err.to_string())?;
    let dest = dest_dir.join(APP_NAME);
    let current = std::env::current_exe().map_err(|err| err.to_string())?;
    if same_path(&current, &dest) {
        return Err(CANNOT_SELF_UPDATE.into());
    }
    if !dest_dir
        .metadata()
        .map(|meta| !meta.permissions().readonly())
        .unwrap_or(false)
    {
        return Err(CANNOT_SELF_UPDATE.into());
    }
    fs::copy(&current, &dest).map_err(|_| CANNOT_SELF_UPDATE.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)
            .map_err(|err| err.to_string())?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms).map_err(|err| err.to_string())?;
    }
    Ok(format!("Updated {APP_NAME} from {VERSION} to {VERSION}"))
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => a == b,
    }
}

fn scope(local: bool) -> &'static str {
    if local {
        " (local)"
    } else {
        ""
    }
}

fn render_list(settings: &Settings) -> String {
    if settings.extensions.is_empty() {
        "No extensions installed.".into()
    } else {
        settings.extensions.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSource {
    Local(String),
    Npm { spec: String, name: String },
    Git(String),
}

pub fn parse_package_source(source: &str) -> ParsedSource {
    if let Some(spec) = source.strip_prefix("npm:") {
        let spec = spec.trim();
        let name = spec
            .split_once('@')
            .filter(|(head, _)| !head.is_empty() || spec.starts_with('@'))
            .map(|(head, _)| {
                if spec.starts_with('@') {
                    let rest = spec.trim_start_matches('@');
                    rest.split_once('@')
                        .map(|(pkg, _)| format!("@{pkg}"))
                        .unwrap_or_else(|| spec.to_string())
                } else {
                    head.to_string()
                }
            })
            .unwrap_or_else(|| spec.to_string());
        return ParsedSource::Npm {
            spec: spec.to_string(),
            name,
        };
    }
    if source.starts_with("git:")
        || source.starts_with("git@")
        || source.ends_with(".git")
        || source.contains("github.com")
        || source.starts_with("https://")
        || source.starts_with("ssh://")
    {
        return ParsedSource::Git(source.to_string());
    }
    ParsedSource::Local(source.to_string())
}

pub fn parse_git_source(source: &str) -> (String, Option<String>) {
    let raw = source.strip_prefix("git:").unwrap_or(source);
    if raw.starts_with("git@") {
        if let Some(idx) = raw.rfind('@') {
            if idx > 3 {
                return (raw[..idx].to_string(), Some(raw[idx + 1..].to_string()));
            }
        }
        return (raw.to_string(), None);
    }
    if let Some(idx) = raw.rfind('@') {
        let spec = &raw[idx + 1..];
        if !spec.contains('/') && !spec.contains(':') {
            return (raw[..idx].to_string(), Some(spec.to_string()));
        }
    }
    (raw.to_string(), None)
}

pub fn npm_install_args(manager: &str, specs: &[String], install_root: &Path) -> Vec<String> {
    let mut args = vec!["install".into()];
    args.extend(specs.iter().cloned());
    match manager {
        "pnpm" => {
            args.push("--prefix".into());
            args.push(install_root.display().to_string());
            args.push("--config.auto-install-peers=false".into());
            args.push("--config.strict-peer-dependencies=false".into());
            args.push("--config.strict-dep-builds=false".into());
        }
        "bun" => {
            args.push("--cwd".into());
            args.push(install_root.display().to_string());
            args.push("--omit=peer".into());
        }
        _ => {
            args.push("--prefix".into());
            args.push(install_root.display().to_string());
            args.push("--legacy-peer-deps".into());
        }
    }
    args
}

pub fn npm_install_root(agent_dir: &Path, local: bool, cwd: &Path) -> PathBuf {
    if local {
        cwd.join(".pi").join("npm")
    } else {
        agent_dir.join("npm")
    }
}

pub fn git_install_root(agent_dir: &Path, local: bool, cwd: &Path) -> PathBuf {
    if local {
        cwd.join(".pi").join("git")
    } else {
        agent_dir.join("git")
    }
}

fn install_and_persist(source: &str, local: bool, agent_dir: &Path) -> Result<String, String> {
    let mut settings = load_settings(agent_dir);
    let parsed = parse_package_source(source);
    match &parsed {
        ParsedSource::Local(path) => {
            let resolved = resolve_local_path(path);
            if !resolved.exists() {
                return Err(format!("Path does not exist: {}", resolved.display()));
            }
        }
        ParsedSource::Npm { name, spec } => {
            install_remote_package(agent_dir, "npm", name, spec, local)?;
        }
        ParsedSource::Git(url) => {
            let name = git_package_name(url);
            install_remote_package(agent_dir, "git", &name, url, local)?;
        }
    }
    if !settings.extensions.contains(&source.to_string()) {
        settings.extensions.push(source.to_string());
    }
    if !settings.packages.iter().any(|item| item.source() == source) {
        settings.packages.push(source.into());
    }
    save_settings(agent_dir, &settings)?;
    Ok(source.to_string())
}

fn resolve_local_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn git_package_name(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or("package")
        .trim_end_matches(".git")
        .to_string()
}

fn install_remote_package(
    agent_dir: &Path,
    kind: &str,
    name: &str,
    spec: &str,
    local: bool,
) -> Result<(), String> {
    if std::env::var("PI_INSTALL_DRY_RUN").is_ok() {
        return Ok(());
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let fixture = match kind {
        "npm" => std::env::var_os("PI_NPM_PACKAGE_DIR").map(PathBuf::from),
        _ => std::env::var_os("PI_GIT_PACKAGE_DIR").map(PathBuf::from),
    };
    if let Some(fixture) = fixture {
        if !fixture.exists() {
            return Err(format!("Path does not exist: {}", fixture.display()));
        }
        let dest = if kind == "npm" {
            npm_install_root(agent_dir, local, &cwd)
                .join("node_modules")
                .join(name)
        } else {
            git_checkout_path(agent_dir, local, &cwd, spec)
        };
        copy_dir(&fixture, &dest)?;
        return Ok(());
    }
    if cfg!(test) {
        return Ok(());
    }
    if kind == "npm" {
        return install_npm_live(agent_dir, local, &cwd, spec);
    }
    install_git_live(agent_dir, local, &cwd, spec)
}

fn git_checkout_path(agent_dir: &Path, local: bool, cwd: &Path, spec: &str) -> PathBuf {
    let (url, _) = parse_git_source(spec);
    let host_path = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("ssh://")
        .trim_start_matches("git@")
        .replace(':', "/");
    git_install_root(agent_dir, local, cwd).join(host_path.trim_end_matches(".git"))
}

fn npm_command(agent_dir: &Path) -> Result<Vec<String>, String> {
    if let Ok(raw) = std::env::var("PI_NPM_CMD") {
        let parts: Vec<String> = raw.split_whitespace().map(String::from).collect();
        if parts.is_empty() || parts[0].is_empty() {
            return Err("Invalid npmCommand: first array entry must be a non-empty command".into());
        }
        return Ok(parts);
    }
    let settings = load_settings(agent_dir);
    if let Some(command) = settings.npm_command {
        if command.is_empty() || command[0].is_empty() {
            return Err("Invalid npmCommand: first array entry must be a non-empty command".into());
        }
        return Ok(command);
    }
    Ok(vec!["npm".into()])
}

fn ensure_npm_project(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|err| err.to_string())?;
    let package = root.join("package.json");
    if !package.exists() {
        fs::write(
            package,
            "{\n  \"name\": \"pi-extensions\",\n  \"private\": true\n}\n",
        )
        .map_err(|err| err.to_string())?;
    }
    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        fs::write(gitignore, "*\n!.gitignore\n").map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn run_install_command(program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().map_err(|err| err.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!("{} {}", stderr.trim(), stdout.trim())
        .trim()
        .to_string())
}

fn install_npm_live(agent_dir: &Path, local: bool, cwd: &Path, spec: &str) -> Result<(), String> {
    let root = npm_install_root(agent_dir, local, cwd);
    ensure_npm_project(&root)?;
    let command = npm_command(agent_dir)?;
    let manager = command.last().map(String::as_str).unwrap_or("npm");
    let mut args = command[1..].to_vec();
    args.extend(npm_install_args(manager, &[spec.to_string()], &root));
    run_install_command(&command[0], &args, None)
}

fn install_git_live(agent_dir: &Path, local: bool, cwd: &Path, spec: &str) -> Result<(), String> {
    let (url, git_ref) = parse_git_source(spec);
    let dest = git_checkout_path(agent_dir, local, cwd, spec);
    if dest.exists() {
        let _ = fs::remove_dir_all(&dest);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let git = std::env::var("PI_GIT_CMD").unwrap_or_else(|_| "git".into());
    if let Err(err) = run_install_command(
        &git,
        &["clone".into(), url.clone(), dest.display().to_string()],
        None,
    ) {
        let _ = fs::remove_dir_all(&dest);
        return Err(err);
    }
    if let Some(git_ref) = git_ref {
        if let Err(err) = run_install_command(&git, &["checkout".into(), git_ref], Some(&dest)) {
            let _ = fs::remove_dir_all(&dest);
            return Err(err);
        }
    }
    if dest.join("package.json").exists() {
        let command = npm_command(agent_dir)?;
        let mut args = command[1..].to_vec();
        args.push("install".into());
        if command.last().map(String::as_str) == Some("npm") && command.len() == 1 {
            args.push("--omit=dev".into());
        }
        if let Err(err) = run_install_command(&command[0], &args, Some(&dest)) {
            let _ = fs::remove_dir_all(&dest);
            return Err(err);
        }
    }
    Ok(())
}

pub fn check_for_available_updates(
    settings: &Settings,
    agent_dir: &Path,
    cwd: &Path,
) -> Vec<String> {
    let mut sources = Vec::new();
    let project = load_settings(&cwd.join(".pi"));
    for pkg in &project.packages {
        sources.push((pkg.source().to_string(), true));
    }
    for pkg in &settings.packages {
        sources.push((pkg.source().to_string(), false));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut updates = Vec::new();
    for (source, local) in sources {
        let identity = package_identity(&source);
        if !seen.insert(identity) {
            continue;
        }
        if let Some(name) = source_has_update(&source, local, agent_dir, cwd) {
            updates.push(name);
        }
    }
    updates
}

fn package_identity(source: &str) -> String {
    match parse_package_source(source) {
        ParsedSource::Npm { name, .. } => format!("npm:{name}"),
        ParsedSource::Git(url) => format!("git:{}", git_host_path(&url)),
        ParsedSource::Local(path) => format!("local:{path}"),
    }
}

fn git_host_path(url: &str) -> String {
    let (url, _) = parse_git_source(url);
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("ssh://")
        .trim_start_matches("git@")
        .replace(':', "/")
        .trim_end_matches(".git")
        .to_string()
}

fn npm_spec_version(spec: &str, name: &str) -> Option<String> {
    let rest = spec.strip_prefix(name)?.strip_prefix('@')?;
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

fn is_exact_npm_version(version: &str) -> bool {
    let trimmed = version.trim().trim_start_matches('v');
    if trimmed.is_empty()
        || trimmed.contains('^')
        || trimmed.contains('~')
        || trimmed.contains('*')
        || trimmed.contains('x')
        || trimmed.contains('X')
        || trimmed.contains(' ')
    {
        return false;
    }
    let mut parts = trimmed.split('.');
    let major = parts.next().and_then(|part| part.parse::<u64>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u64>().ok());
    let patch = parts.next().and_then(|part| {
        part.chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .parse::<u64>()
            .ok()
    });
    major.is_some() && minor.is_some() && patch.is_some() && parts.next().is_none()
}

fn source_has_update(source: &str, local: bool, agent_dir: &Path, cwd: &Path) -> Option<String> {
    match parse_package_source(source) {
        ParsedSource::Local(_) => None,
        ParsedSource::Npm { spec, name } => {
            if npm_spec_version(&spec, &name)
                .as_deref()
                .is_some_and(is_exact_npm_version)
            {
                return None;
            }
            let installed = npm_install_root(agent_dir, local, cwd)
                .join("node_modules")
                .join(&name);
            if !installed.exists() {
                return None;
            }
            if npm_has_available_update(&spec, &name, &installed, agent_dir, cwd) {
                Some(name)
            } else {
                None
            }
        }
        ParsedSource::Git(url) => {
            let installed = git_checkout_path(agent_dir, local, cwd, &url);
            if !installed.exists() {
                return None;
            }
            if git_has_available_update(&installed) {
                Some(git_host_path(&url))
            } else {
                None
            }
        }
    }
}

fn npm_has_available_update(
    spec: &str,
    name: &str,
    installed: &Path,
    agent_dir: &Path,
    cwd: &Path,
) -> bool {
    let installed_version = installed_npm_version(installed);
    let Some(installed_version) = installed_version else {
        return false;
    };
    let view_spec = if npm_spec_version(spec, name).is_some() {
        spec
    } else {
        name
    };
    let Some(latest) = latest_npm_version(view_spec, agent_dir, cwd) else {
        return false;
    };
    is_newer_installed_version(&latest, &installed_version)
}

fn is_newer_installed_version(candidate: &str, current: &str) -> bool {
    match (
        parse_install_semver(candidate),
        parse_install_semver(current),
    ) {
        (Some(left), Some(right)) => left > right,
        _ => candidate.trim() != current.trim(),
    }
}

fn parse_install_semver(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()?
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

fn installed_npm_version(installed: &Path) -> Option<String> {
    let raw = fs::read_to_string(installed.join("package.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn latest_npm_version(spec: &str, agent_dir: &Path, cwd: &Path) -> Option<String> {
    let raw = if let Ok(reply) = std::env::var("PI_NPM_VIEW_REPLY") {
        let path = PathBuf::from(&reply);
        if path.exists() {
            fs::read_to_string(path).ok()?
        } else {
            reply
        }
    } else if cfg!(test) {
        return None;
    } else {
        let command = npm_command(agent_dir).ok()?;
        let mut args = command[1..].to_vec();
        args.extend([
            "view".into(),
            spec.into(),
            "version".into(),
            "--json".into(),
        ]);
        let output = std::process::Command::new(&command[0])
            .args(&args)
            .current_dir(cwd)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    parse_npm_view_version(&raw)
}

fn parse_npm_view_version(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    match value {
        serde_json::Value::String(version) => Some(version),
        serde_json::Value::Array(items) => items
            .into_iter()
            .rev()
            .find_map(|item| item.as_str().map(str::to_string)),
        _ => None,
    }
}

fn git_has_available_update(installed: &Path) -> bool {
    let Some(local) = git_rev_parse(installed, "HEAD") else {
        return false;
    };
    let Some(remote) = remote_git_head(installed) else {
        return false;
    };
    local.trim() != remote.trim()
}

fn git_rev_parse(installed: &Path, rev: &str) -> Option<String> {
    if let Ok(reply) = std::env::var("PI_GIT_REV_PARSE_REPLY") {
        return Some(reply);
    }
    if cfg!(test) {
        return None;
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(installed)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn remote_git_head(installed: &Path) -> Option<String> {
    if let Ok(reply) = std::env::var("PI_GIT_LS_REMOTE_REPLY") {
        let path = PathBuf::from(&reply);
        let raw = if path.exists() {
            fs::read_to_string(path).ok()?
        } else {
            reply
        };
        return parse_ls_remote_head(&raw);
    }
    if cfg!(test) {
        return None;
    }
    let git = std::env::var("PI_GIT_CMD").unwrap_or_else(|_| "git".into());
    let output = std::process::Command::new(git)
        .args(["ls-remote", "origin", "HEAD"])
        .current_dir(installed)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_ls_remote_head(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ls_remote_head(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let dest = parts.next().unwrap_or("");
        if dest == "HEAD" || dest.ends_with("/HEAD") {
            return Some(hash.to_string());
        }
        if hash.len() == 40 && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Some(hash.to_string());
        }
    }
    None
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|err| err.to_string())?;
    for entry in fs::read_dir(from).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

fn render_config(settings: &Settings, local: bool) -> String {
    format!(
        "scope: {}\nextensions: {}\npackages: {}\ntheme: {}\n",
        if local { "local" } else { "user" },
        settings.extensions.join(", "),
        settings
            .packages
            .iter()
            .map(crate::settings::PackageSource::source)
            .collect::<Vec<_>>()
            .join(", "),
        settings.theme.clone().unwrap_or_else(|| "dark".into())
    )
}

fn render_config_command(
    settings: &Settings,
    local: bool,
    agent_dir: &Path,
) -> Result<String, String> {
    let mut selector = config_selector_from_settings(settings, local, agent_dir);
    if let Ok(spec) = std::env::var("PI_CONFIG_TOGGLE") {
        if let Some((source, enabled)) = spec.split_once('=') {
            persist_config_toggle(agent_dir, local, source, enabled == "true")?;
            selector = config_selector_from_settings(&load_settings(agent_dir), local, agent_dir);
        }
    }
    let rendered = selector.render(80).join("\n");
    if std::env::var("PI_CONFIG_TEXT").is_ok() {
        let stored = load_settings(agent_dir);
        return Ok(format!("{}\n{rendered}", render_config(&stored, local)));
    }
    Ok(rendered)
}

pub fn config_selector_from_settings(
    settings: &Settings,
    local: bool,
    _agent_dir: &Path,
) -> ConfigSelector {
    let scope = if local {
        ConfigScope::Project
    } else {
        ConfigScope::User
    };
    let mut items = Vec::new();
    let package_sources: Vec<String> = settings
        .packages
        .iter()
        .map(|item| item.source().to_string())
        .collect();
    for source in settings.extensions.iter().chain(package_sources.iter()) {
        let name = Path::new(source)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| source.clone());
        if items
            .iter()
            .any(|item: &ConfigResource| item.source == *source && item.scope == scope)
        {
            continue;
        }
        let enabled = settings
            .packages
            .iter()
            .find(|item| item.source() == source)
            .map(|item| item.autoload())
            .unwrap_or(true);
        items.push(ConfigResource {
            kind: ConfigResourceKind::Extensions,
            name,
            source: source.clone(),
            enabled,
            scope,
        });
    }
    let mut selector = ConfigSelector::new(items);
    selector.scope = scope;
    selector
}

pub fn ensure_agent_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| err.to_string())
}

pub fn managed_bin_dir(agent_dir: &Path) -> PathBuf {
    agent_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| agent_dir.to_path_buf())
        .join("bin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn update_self_copies_binary_and_rejects_conflicts() {
        assert_eq!(
            handle_package_command(
                "update",
                &["--all".into(), "--self".into()],
                Path::new("/tmp")
            )
            .unwrap_err(),
            ALL_CONFLICT
        );
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        fs::create_dir_all(&agent).unwrap();
        let message = self_update_binary(&agent, true).unwrap();
        assert!(message.starts_with("Updated pi from "));
        assert!(managed_bin_dir(&agent).join("pi").exists());
    }

    #[test]
    fn install_resolves_local_path_and_rejects_missing() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        fs::create_dir_all(&agent).unwrap();
        let ext = dir.path().join("ext");
        fs::create_dir_all(&ext).unwrap();
        fs::write(ext.join("index.js"), "module.exports = () => {}").unwrap();
        let installed =
            handle_package_command("install", &[ext.display().to_string()], &agent).unwrap();
        assert!(installed.starts_with("Installed "));
        let settings = load_settings(&agent);
        assert!(settings
            .packages
            .iter()
            .any(|item| item.source().contains("ext")));
        let err = handle_package_command(
            "install",
            &[dir.path().join("missing").display().to_string()],
            &agent,
        )
        .unwrap_err();
        assert!(err.starts_with("Path does not exist:"));
        std::env::set_var("PI_INSTALL_DRY_RUN", "1");
        let npm = handle_package_command("install", &["npm:demo-ext".into()], &agent).unwrap();
        std::env::remove_var("PI_INSTALL_DRY_RUN");
        assert!(npm.contains("npm:demo-ext"));
        assert_eq!(
            parse_package_source("npm:demo-ext"),
            ParsedSource::Npm {
                spec: "demo-ext".into(),
                name: "demo-ext".into(),
            }
        );
        let selector = config_selector_from_settings(&load_settings(&agent), false, &agent);
        assert!(selector
            .render(80)
            .iter()
            .any(|line| line.contains("Package resources")));
        assert_eq!(
            parse_git_source("https://github.com/acme/ext@v1"),
            ("https://github.com/acme/ext".into(), Some("v1".into()))
        );
        assert_eq!(
            npm_install_args("npm", &["demo-ext".into()], Path::new("/tmp/npm"))[1],
            "demo-ext"
        );
        assert!(
            npm_install_args("npm", &["demo-ext".into()], Path::new("/tmp/npm"))
                .contains(&"--legacy-peer-deps".into())
        );
        assert!(
            npm_install_args("bun", &["demo-ext".into()], Path::new("/tmp/npm"))
                .contains(&"--omit=peer".into())
        );
        persist_config_toggle(&agent, false, &ext.display().to_string(), false).unwrap();
        let toggled = load_settings(&agent);
        assert!(!toggled
            .packages
            .iter()
            .find(|item| item.source().contains("ext"))
            .expect("pkg")
            .autoload());
        std::env::set_var("PI_INSTALL_DRY_RUN", "1");
        let updated = handle_package_command("update", &["--extensions".into()], &agent).unwrap();
        std::env::remove_var("PI_INSTALL_DRY_RUN");
        assert!(updated.contains("Updated packages") || updated.contains("Updated "));
    }
}
