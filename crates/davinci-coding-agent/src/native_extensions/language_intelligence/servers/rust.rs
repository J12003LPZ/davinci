//! rust-analyzer adapter and restricted navigation profile.

use super::{path_executable, ServerCommand, ServerKind};
use crate::native_extensions::language_intelligence::config::RustConfig;
use crate::native_extensions::language_intelligence::protocol::{IntelligenceError, Result};
use serde_json::json;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub(super) fn discover(
    project: &Path,
    config: &RustConfig,
    search_path: &OsStr,
) -> Result<Vec<ServerCommand>> {
    let mut candidates = Vec::new();

    if let Some(server) = &config.server {
        if server.program.is_file() {
            candidates.push(command(
                project,
                server.program.clone(),
                server.args.clone(),
                config,
            ));
        }
    } else if let Some(path) = std::env::var_os("DAVINCI_RUST_ANALYZER")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
    {
        candidates.push(command(project, path, Vec::new(), config));
    } else if let Some(path) = path_executable(search_path, "rust-analyzer") {
        candidates.push(command(project, path, Vec::new(), config));
    }

    if candidates.is_empty() {
        Err(IntelligenceError::new(
            "server_not_installed",
            "Install rust-analyzer or configure an absolute rust.server.program; DaVinci never installs language servers",
        ))
    } else {
        Ok(candidates)
    }
}

fn command(
    project: &Path,
    program: PathBuf,
    args: Vec<String>,
    config: &RustConfig,
) -> ServerCommand {
    let sysroot = config
        .sysroot
        .as_ref()
        .filter(|path| path.is_absolute() && path.is_dir());
    let sysroot_src = config
        .sysroot_src
        .as_ref()
        .filter(|path| path.is_absolute() && path.join("core/src/lib.rs").is_file());

    let mut cargo = json!({
        "buildScripts": {"enable": false},
        "sysroot": sysroot,
        "sysrootSrc": sysroot_src,
        "extraEnv": {
            "CARGO_NET_OFFLINE": "true",
            "RUSTUP_AUTO_INSTALL": "0"
        }
    });
    if let Some(target) = &config.target {
        cargo["target"] = json!(target);
    }
    if !config.features.is_empty() {
        cargo["features"] = json!(config.features);
    }

    ServerCommand {
        kind: ServerKind::RustAnalyzer,
        program,
        args,
        workspace: project.into(),
        version: None,
        typescript_version: None,
        initialization_options: json!({
            "cargo": cargo,
            "procMacro": {"enable": false},
            "checkOnSave": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_profile_disables_executable_analysis() {
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path().join(if cfg!(windows) { "rust-analyzer.exe" } else { "rust-analyzer" });
        std::fs::write(&server, "fixture").unwrap();
        let mut config = RustConfig::default();
        config.server = Some(crate::native_extensions::language_intelligence::ServerOverride {
            program: server,
            args: Vec::new(),
        });
        let command = discover(dir.path(), &config, OsStr::new("")).unwrap().remove(0);
        assert_eq!(command.kind, ServerKind::RustAnalyzer);
        assert_eq!(command.initialization_options["cargo"]["buildScripts"]["enable"], false);
        assert_eq!(command.initialization_options["procMacro"]["enable"], false);
        assert_eq!(command.initialization_options["checkOnSave"], false);
        assert!(command.initialization_options["cargo"]["sysroot"].is_null());
    }
}
