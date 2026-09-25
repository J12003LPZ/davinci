//! BasedPyright/Pyright adapter with explicit target-interpreter identity.
use super::super::config::{LanguageIntelligenceConfig, PythonBackend, PythonDiagnosticMode};
use super::super::identity::{LanguageFamily, ResolvedProject};
use super::super::metadata::ResolutionContext;
use super::super::protocol::{IntelligenceError, Result};
use super::{discovery, ServerAdapter, ServerBackend, ServerCommand};
use serde_json::json;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(in crate::native_extensions::language_intelligence) struct PythonAdapter;

impl ServerAdapter for PythonAdapter {
    fn family(&self) -> LanguageFamily { LanguageFamily::Python }
    fn language_id(&self, path: &Path) -> Option<&'static str> {
        matches!(path.extension()?.to_str()?, "py" | "pyi").then_some("python")
    }
    fn explicit_roots<'a>(&self, settings: &'a LanguageIntelligenceConfig) -> &'a [PathBuf] {
        &settings.python.project_roots
    }
}

fn interpreter(project: &ResolvedProject, settings: &LanguageIntelligenceConfig, search_path: &OsStr) -> Result<Option<PathBuf>> {
    let profile = &settings.python;
    if let Some(path) = &profile.interpreter {
        let path = if path.is_absolute() { path.clone() } else { project.root.join(path) };
        if !path.is_file() {
            return Err(IntelligenceError::new("interpreter_not_found", "The configured Python interpreter does not exist"));
        }
        return Ok(Some(path.canonicalize().unwrap_or(path)));
    }
    let names: &[&str] = if cfg!(windows) { &[".venv/Scripts/python.exe", "venv/Scripts/python.exe"] } else { &[".venv/bin/python", "venv/bin/python"] };
    for name in names {
        let path = project.root.join(name);
        if path.is_file() {
            return Ok(Some(path.canonicalize().unwrap_or(path)));
        }
    }
    if let Some(venv) = std::env::var_os("VIRTUAL_ENV") {
        let path = if cfg!(windows) { PathBuf::from(venv).join("Scripts/python.exe") } else { PathBuf::from(venv).join("bin/python") };
        if path.is_file() { return Ok(Some(path.canonicalize().unwrap_or(path))); }
    }
    Ok(discovery::path_program(search_path, if cfg!(windows) { "python.exe" } else { "python3" })
        .or_else(|| discovery::path_program(search_path, "python")))
}

fn project_server(project: &ResolvedProject, name: &str, search_path: &OsStr) -> Option<super::super::identity::ServerInvocation> {
    let path = if cfg!(windows) {
        project.root.join(".venv/Scripts").join(format!("{name}.exe"))
    } else {
        project.root.join(".venv/bin").join(name)
    };
    path.is_file().then(|| discovery::invocation_for_path(&path, vec!["--stdio".into()], search_path)).and_then(Result::ok)
}

pub(super) fn discover(
    _context: &ResolutionContext,
    project: &ResolvedProject,
    settings: &LanguageIntelligenceConfig,
    search_path: &OsStr,
) -> Result<Vec<ServerCommand>> {
    let profile = &settings.python;
    if !profile.enabled {
        return Err(IntelligenceError::new("disabled", "Python language intelligence is disabled"));
    }
    let target = interpreter(project, settings, search_path)?;
    let explicit = profile.server.as_ref().map(|server| discovery::explicit(server, search_path)).transpose()?;
    let choose = |backend: PythonBackend| -> Option<(ServerBackend, super::super::identity::ServerInvocation)> {
        match backend {
            PythonBackend::Basedpyright => project_server(project, "basedpyright-langserver", search_path)
                .or_else(|| discovery::named(search_path, "basedpyright-langserver", vec!["--stdio".into()]))
                .map(|v| (ServerBackend::BasedPyright, v)),
            PythonBackend::Pyright => project_server(project, "pyright-langserver", search_path)
                .or_else(|| discovery::named(search_path, "pyright-langserver", vec!["--stdio".into()]))
                .map(|v| (ServerBackend::Pyright, v)),
            PythonBackend::Auto => None,
        }
    };
    let (backend, invocation) = if let Some(invocation) = explicit {
        let backend = match profile.backend {
            PythonBackend::Pyright => ServerBackend::Pyright,
            _ => ServerBackend::BasedPyright,
        };
        (backend, invocation)
    } else {
        match profile.backend {
            PythonBackend::Auto => choose(PythonBackend::Basedpyright).or_else(|| choose(PythonBackend::Pyright)),
            other => choose(other),
        }.ok_or_else(|| IntelligenceError::new("server_not_installed", "Install BasedPyright/Pyright or configure languageIntelligence.python.server"))?
    };

    let diagnostic_mode = match profile.diagnostic_mode {
        PythonDiagnosticMode::OpenFilesOnly => "openFilesOnly",
        PythonDiagnosticMode::Workspace => "workspace",
    };
    let mut configuration = json!({
        "python": {
            "pythonPath": target.as_ref().map(|p| p.to_string_lossy().into_owned())
        }
    });
    match backend {
        ServerBackend::BasedPyright => {
            configuration["basedpyright"] = json!({"analysis":{"diagnosticMode":diagnostic_mode,"baselineMode":"discard"}});
        }
        ServerBackend::Pyright => {
            configuration["pyright"] = json!({"analysis":{"diagnosticMode":diagnostic_mode}});
        }
        _ => {}
    }
    let mut limitations = Vec::new();
    if target.is_none() { limitations.push("python_interpreter_unresolved".into()); }
    if target.as_ref().is_some_and(|p| p.starts_with(&project.workspace)) {
        limitations.push("project_interpreter_executed_requires_trust".into());
    }

    Ok(vec![ServerCommand {
        kind: backend,
        backend,
        program: invocation.program.clone(),
        args: invocation.args.clone(),
        invocation,
        workspace: project.root.clone(),
        version: None,
        typescript_version: None,
        initialization_options: json!({}),
        client_configuration: configuration,
        family: LanguageFamily::Python,
        profile_fingerprint: settings.profile_fingerprint(profile),
        analysis_environment: target,
        limitations,
        env: BTreeMap::new(),
    }])
}
