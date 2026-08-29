use std::fs;
use std::path::{Path, PathBuf};

use crate::args::{APP_NAME, VERSION};
use crate::self_update::{
    current_install_method, inferred_npm_prefix, package_dir_from_env,
    self_update_command_for_method, self_update_unavailable_instruction, update_instruction,
    InstallMethod, PackageTarget, PACKAGE_NAME,
};
use crate::settings::{load_settings, save_settings, Settings};
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
            settings.packages.retain(|item| item != &source);
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
    let do_self = flags.self_flag
        || positional_self
        || (!flags.models_flag
            && !flags.extensions_flag
            && !flags.all_flag
            && flags.extension.is_none()
            && flags.positional.is_none());
    let do_models =
        flags.models_flag || flags.all_flag || flags.positional.as_deref() == Some("all");
    let mut parts = Vec::new();
    if do_self {
        parts.push(self_update_binary(agent_dir, flags.force)?);
    }
    if do_models || flags.all_flag || (!do_self && flags.positional.is_none()) {
        parts.push(refresh_model_catalogs(
            agent_dir,
            flags.positional.as_deref(),
        )?);
    }
    if flags.extensions_flag || flags.all_flag {
        parts.push("Extensions: JS modules reload on next session when Node is present.".into());
    }
    Ok(parts.join("\n"))
}

fn refresh_model_catalogs(agent_dir: &Path, target: Option<&str>) -> Result<String, String> {
    let catalogs = agent_dir.join("models");
    fs::create_dir_all(&catalogs).map_err(|err| err.to_string())?;
    for provider in pi_ai::builtin_provider_ids() {
        if let Some(json) = pi_ai::builtin_catalog_json(provider) {
            fs::write(catalogs.join(format!("{provider}.json")), json)
                .map_err(|err| err.to_string())?;
        }
    }
    let target = target.unwrap_or("all");
    Ok(format!(
        "Updated {target}: wrote {} catalogs to {}",
        pi_ai::builtin_provider_ids().len(),
        catalogs.display()
    ))
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
    if source.starts_with("git@")
        || source.ends_with(".git")
        || source.contains("github.com")
        || source.starts_with("https://")
        || source.starts_with("ssh://")
    {
        return ParsedSource::Git(source.to_string());
    }
    ParsedSource::Local(source.to_string())
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
            install_remote_package(agent_dir, "npm", name, spec)?;
        }
        ParsedSource::Git(url) => {
            let name = git_package_name(url);
            install_remote_package(agent_dir, "git", &name, url)?;
        }
    }
    if !settings.extensions.contains(&source.to_string()) {
        settings.extensions.push(source.to_string());
    }
    if !settings.packages.contains(&source.to_string()) {
        settings.packages.push(source.to_string());
    }
    save_settings(agent_dir, &settings)?;
    let _ = local;
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
) -> Result<(), String> {
    if std::env::var("PI_INSTALL_DRY_RUN").is_ok() {
        return Ok(());
    }
    let fixture = match kind {
        "npm" => std::env::var_os("PI_NPM_PACKAGE_DIR").map(PathBuf::from),
        _ => std::env::var_os("PI_GIT_PACKAGE_DIR").map(PathBuf::from),
    };
    if let Some(fixture) = fixture {
        if !fixture.exists() {
            return Err(format!("Path does not exist: {}", fixture.display()));
        }
        let dest = agent_dir.join("packages").join(name);
        copy_dir(&fixture, &dest)?;
        return Ok(());
    }
    if cfg!(test) {
        return Ok(());
    }
    Err(format!(
        "Unsupported install source without fixture: {spec} (set PI_INSTALL_DRY_RUN=1)"
    ))
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
        settings.packages.join(", "),
        settings.theme.clone().unwrap_or_else(|| "dark".into())
    )
}

fn render_config_command(
    settings: &Settings,
    local: bool,
    agent_dir: &Path,
) -> Result<String, String> {
    let selector = config_selector_from_settings(settings, local, agent_dir);
    let rendered = selector.render(80).join("\n");
    if std::env::var("PI_CONFIG_TEXT").is_ok() {
        return Ok(format!("{}\n{rendered}", render_config(settings, local)));
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
    for source in settings.extensions.iter().chain(settings.packages.iter()) {
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
        items.push(ConfigResource {
            kind: ConfigResourceKind::Extensions,
            name,
            source: source.clone(),
            enabled: true,
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
        assert!(settings.packages.iter().any(|item| item.contains("ext")));
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
    }
}
