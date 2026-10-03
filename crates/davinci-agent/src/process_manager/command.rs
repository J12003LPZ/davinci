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
            if let Ok(canonical) = option.canonicalize() {
                let extension = canonical
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if canonical.is_file()
                    && (!cfg!(windows) || matches!(extension.as_str(), "exe" | "com"))
                {
                    return Ok(canonical);
                }
            }
        }
    }
    Err(
        "process executable unavailable; use an installed native executable and literal argv"
            .into(),
    )
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
