//! BasedPyright/Pyright adapter and interpreter selection.

use super::{path_executable, ServerCommand, ServerKind};
use crate::native_extensions::language_intelligence::config::{PythonBackend, PythonConfig};
use crate::native_extensions::language_intelligence::protocol::{IntelligenceError, Result};
use serde_json::json;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub(super) fn discover(
    project: &Path,
    config: &PythonConfig,
    search_path: &OsStr,
) -> Result<Vec<ServerCommand>> {
    let interpreter = select_interpreter(project, config, search_path)?;
    let mut candidates = Vec::new();

    if let Some(server) = &config.server {
        if server.program.is_file() {
            let kind = match config.backend {
                PythonBackend::Pyright => ServerKind::Pyright,
                _ => ServerKind::Basedpyright,
            };
            candidates.push(command(
                project,
                server.program.clone(),
                server.args.clone(),
                kind,
                &interpreter,
                config,
            ));
        }
    } else {
        let based = project_executable(project, "basedpyright-langserver")
            .or_else(|| path_executable(search_path, "basedpyright-langserver"));
        let pyright = project_executable(project, "pyright-langserver")
            .or_else(|| path_executable(search_path, "pyright-langserver"));

        match config.backend {
            PythonBackend::Auto | PythonBackend::Basedpyright => {
                if let Some(program) = based {
                    candidates.push(command(
                        project,
                        program,
                        vec!["--stdio".into()],
                        ServerKind::Basedpyright,
                        &interpreter,
                        config,
                    ));
                } else if matches!(config.backend, PythonBackend::Auto) {
                    if let Some(program) = pyright {
                        candidates.push(command(
                            project,
                            program,
                            vec!["--stdio".into()],
                            ServerKind::Pyright,
                            &interpreter,
                            config,
                        ));
                    }
                }
            }
            PythonBackend::Pyright => {
                if let Some(program) = pyright {
                    candidates.push(command(
                        project,
                        program,
                        vec!["--stdio".into()],
                        ServerKind::Pyright,
                        &interpreter,
                        config,
                    ));
                }
            }
        }
    }

    if candidates.is_empty() {
        Err(IntelligenceError::new(
            "server_not_installed",
            "Install BasedPyright or Pyright, or configure an absolute python.server.program; DaVinci never installs language servers",
        ))
    } else {
        Ok(candidates)
    }
}

pub(super) fn select_interpreter(
    project: &Path,
    config: &PythonConfig,
    search_path: &OsStr,
) -> Result<PathBuf> {
    if let Some(path) = &config.interpreter {
        if !path.is_absolute() || !path.is_file() {
            return Err(IntelligenceError::new(
                "interpreter_not_found",
                "Configured Python interpreter is unavailable",
            ));
        }
        return Ok(path.clone());
    }

    for environment in [".venv", "venv"] {
        let root = project.join(environment);
        let candidate = if cfg!(windows) {
            root.join("Scripts/python.exe")
        } else {
            root.join("bin/python")
        };
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    path_executable(search_path, if cfg!(windows) { "python" } else { "python3" })
        .or_else(|| path_executable(search_path, "python"))
        .ok_or_else(|| {
            IntelligenceError::new(
                "interpreter_not_found",
                "No configured, project, or system Python interpreter is available",
            )
        })
}

fn project_executable(project: &Path, name: &str) -> Option<PathBuf> {
    for environment in [".venv", "venv"] {
        let root = project.join(environment);
        let candidate = if cfg!(windows) {
            root.join("Scripts").join(format!("{name}.exe"))
        } else {
            root.join("bin").join(name)
        };
        if candidate.is_file() {
            return candidate.canonicalize().ok();
        }
    }
    None
}

fn command(
    project: &Path,
    program: PathBuf,
    mut args: Vec<String>,
    kind: ServerKind,
    interpreter: &Path,
    config: &PythonConfig,
) -> ServerCommand {
    if args.is_empty() && config.server.is_none() {
        args.push("--stdio".into());
    }
    let mut initialization_options = json!({
        "python": {"pythonPath": interpreter},
        "analysis": {"diagnosticMode": config.diagnostic_mode}
    });
    if kind == ServerKind::Basedpyright {
        initialization_options["analysis"]["baselineMode"] = json!("discard");
    }
    ServerCommand {
        kind,
        program,
        args,
        workspace: project.into(),
        version: None,
        typescript_version: None,
        initialization_options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_missing_interpreter_is_an_error() {
        let mut config = PythonConfig::default();
        config.interpreter = Some(PathBuf::from("/definitely/missing/python"));
        assert_eq!(
            select_interpreter(Path::new("."), &config, OsStr::new(""))
                .unwrap_err()
                .code,
            "interpreter_not_found"
        );
    }
}
