//! Resolve literal argv and a minimal child environment before supervised spawn.
use crate::jobs::supervisor::ProcessConfig;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Start {
    pub executable: String,
    #[serde(default)]
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub restart: crate::jobs::managed::RestartPolicy,
    #[serde(default)]
    pub ports: Vec<u16>,
}

pub(super) fn resolve(
    workspace: &Path,
    cwd: &Path,
    request: Start,
) -> Result<ProcessConfig, String> {
    if request.executable.is_empty()
        || request.executable.len() > 4096
        || request.executable.contains('\0')
        || request.argv.len() > 128
        || request.argv.iter().any(|arg| arg.contains('\0'))
        || request.argv.iter().map(String::len).sum::<usize>() > 32 * 1024
    {
        return Err("invalid or oversized executable/argv".into());
    }
    let cwd = request
        .cwd
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .unwrap_or_else(|| cwd.into())
        .canonicalize()
        .map_err(|_| "process cwd unavailable")?;
    if !cwd.is_dir() || !cwd.starts_with(workspace) {
        return Err("process cwd must be a directory inside its workspace".into());
    }
    let mut environment = BTreeMap::new();
    for name in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ] {
        if let Ok(value) = std::env::var(name) {
            environment.insert(name.to_owned(), value);
        }
    }
    if request.env.len() > 32
        || request
            .env
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            > 8192
    {
        return Err("process environment overrides exceed their bound".into());
    }
    for (name, value) in request.env {
        if name.is_empty()
            || name.len() > 128
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || name.as_bytes()[0].is_ascii_digit()
            || value.contains('\0')
        {
            return Err("invalid process environment entry".into());
        }
        let upper = name.to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "PATH"
                | "PATHEXT"
                | "SYSTEMROOT"
                | "WINDIR"
                | "COMSPEC"
                | "NODE_OPTIONS"
                | "NODE_PATH"
                | "BASH_ENV"
                | "ENV"
                | "PYTHONPATH"
                | "PYTHONHOME"
                | "PERL5OPT"
                | "RUBYOPT"
        ) || upper.starts_with("LD_")
            || upper.starts_with("DYLD_")
            || upper.starts_with("DAVINCI_INTERNAL_")
        {
            return Err("process environment cannot override executable discovery or interpreter injection variables".into());
        }
        environment.insert(name, value);
    }
    let (executable, argv) = executable(&request.executable, request.argv, &cwd)?;
    Ok(ProcessConfig::new(executable, argv, cwd, environment))
}

fn candidates(name: &str, cwd: &Path) -> Vec<PathBuf> {
    if Path::new(name).is_absolute() || name.contains(['/', '\\']) {
        return vec![cwd.join(name)];
    }
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(name))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn direct(name: &str, cwd: &Path) -> Result<PathBuf, String> {
    for path in candidates(name, cwd) {
        let mut options = vec![path.clone()];
        if cfg!(windows) && path.extension().is_none() {
            options = vec![path.with_extension("exe"), path.with_extension("com")];
        }
        for option in options {
            match option.canonicalize() {
                Ok(canonical) => {
                    if canonical.is_file() && (!cfg!(windows) || has_native_extension(&canonical)) {
                        return Ok(canonical);
                    }
                }
                Err(error) if app_execution_alias(&option, &error) => return Ok(option),
                Err(_) => {}
            }
        }
    }
    Err(
        "process executable unavailable; use an installed native executable and literal argv"
            .into(),
    )
}

fn has_native_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "exe" | "com"))
}

/// Store-installed programs, including PowerShell 7, reach PATH as App
/// Execution Aliases: reparse points that CreateProcess launches but that
/// cannot be opened, so canonicalization fails with ERROR_CANT_ACCESS_FILE.
/// Rejecting them silently replaced `pwsh` with Windows PowerShell 5.1, whose
/// native argument passing strips embedded double quotes.
#[cfg(windows)]
fn app_execution_alias(path: &Path, error: &std::io::Error) -> bool {
    use std::os::windows::fs::MetadataExt;
    let attributes = std::fs::symlink_metadata(path)
        .ok()
        .map(|metadata| metadata.file_attributes());
    is_app_execution_alias(path, error.raw_os_error(), attributes)
}

#[cfg(not(windows))]
fn app_execution_alias(_path: &Path, _error: &std::io::Error) -> bool {
    false
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn is_app_execution_alias(path: &Path, error_code: Option<i32>, attributes: Option<u32>) -> bool {
    const ERROR_CANT_ACCESS_FILE: i32 = 1920;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    error_code == Some(ERROR_CANT_ACCESS_FILE)
        && path.is_absolute()
        && has_native_extension(path)
        && attributes.is_some_and(|value| {
            value & FILE_ATTRIBUTE_REPARSE_POINT != 0 && value & FILE_ATTRIBUTE_DIRECTORY == 0
        })
}

fn executable(name: &str, argv: Vec<String>, cwd: &Path) -> Result<(PathBuf, Vec<String>), String> {
    #[cfg(windows)]
    if matches!(
        name,
        "npm" | "npm.cmd" | "pnpm" | "pnpm.cmd" | "yarn" | "yarn.cmd"
    ) {
        let tool = name.trim_end_matches(".cmd");
        let node = direct("node", cwd)?;
        let suffixes: &[&str] = match tool {
            "npm" => &["node_modules/npm/bin/npm-cli.js"],
            "pnpm" => &[
                "node_modules/pnpm/bin/pnpm.cjs",
                "node_modules/corepack/dist/pnpm.js",
            ],
            _ => &[
                "node_modules/yarn/bin/yarn.js",
                "node_modules/corepack/dist/yarn.js",
            ],
        };
        for shim in candidates(&format!("{tool}.cmd"), cwd)
            .into_iter()
            .filter(|p| p.is_file())
        {
            let Some(parent) = shim.parent() else {
                continue;
            };
            for suffix in suffixes {
                if let Ok(script) = parent.join(suffix).canonicalize() {
                    if script.is_file() {
                        // Node's module resolver cannot load Windows verbatim paths.
                        // Only remove the prefix if the ordinary path still names
                        // the same canonical entry point.
                        let node_script = crate::permission::strip_verbatim_prefix(&script);
                        if node_script.canonicalize().ok().as_ref() != Some(&script) {
                            continue;
                        }
                        let mut resolved = vec![node_script
                            .to_str()
                            .ok_or("package manager path is not UTF-8")?
                            .to_owned()];
                        resolved.extend(argv);
                        return Ok((node, resolved));
                    }
                }
            }
        }
        return Err(
            "package manager JavaScript entry point unavailable; shell shims are not executed"
                .into(),
        );
    }
    direct(name, cwd).map(|program| (program, argv))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPARSE_FILE: u32 = 0x420;

    #[test]
    fn only_unopenable_executable_reparse_files_count_as_app_aliases() {
        let alias = std::env::temp_dir().join("pwsh.exe");
        assert!(is_app_execution_alias(
            &alias,
            Some(1920),
            Some(REPARSE_FILE)
        ));
        // A missing or dangling target fails differently and stays rejected.
        assert!(!is_app_execution_alias(&alias, Some(2), Some(REPARSE_FILE)));
        assert!(!is_app_execution_alias(&alias, Some(1920), None));
        // Ordinary files and reparse directories are not aliases.
        assert!(!is_app_execution_alias(&alias, Some(1920), Some(0x20)));
        assert!(!is_app_execution_alias(&alias, Some(1920), Some(0x410)));
        // Scripts and relative names never bypass canonical resolution.
        let script = std::env::temp_dir().join("pwsh.cmd");
        assert!(!is_app_execution_alias(
            &script,
            Some(1920),
            Some(REPARSE_FILE)
        ));
        assert!(!is_app_execution_alias(
            Path::new("pwsh.exe"),
            Some(1920),
            Some(REPARSE_FILE)
        ));
    }

    #[test]
    fn missing_executable_is_still_unavailable() {
        let cwd = tempfile::tempdir().unwrap();
        let missing = cwd.path().join("davinci-missing-tool.exe");
        assert!(direct(missing.to_str().unwrap(), cwd.path()).is_err());
    }

    /// Windows-only end-to-end check against a real Store alias such as
    /// `pwsh.exe` or `winget.exe`. Hosts without any alias have nothing to prove.
    #[cfg(windows)]
    #[test]
    fn installed_app_execution_alias_resolves_to_the_launchable_alias() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let directory = PathBuf::from(local).join("Microsoft").join("WindowsApps");
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return;
        };
        let alias = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                has_native_extension(path)
                    && path
                        .canonicalize()
                        .err()
                        .is_some_and(|error| app_execution_alias(path, &error))
            });
        let Some(alias) = alias else {
            return;
        };
        let cwd = tempfile::tempdir().unwrap();
        assert_eq!(
            direct(alias.to_str().unwrap(), cwd.path()).unwrap(),
            alias,
            "a Store alias on PATH must not fall through to another program"
        );
    }
}
