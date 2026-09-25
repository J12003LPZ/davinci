//! Installed-only server and launcher discovery. No program is executed here.
use super::super::config::ServerOverride;
use super::super::identity::ServerInvocation;
use super::super::protocol::{IntelligenceError, Result};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub(super) fn explicit(server: &ServerOverride, search_path: &OsStr) -> Result<ServerInvocation> {
    invocation_for_path(&server.program, server.args.clone(), search_path)
}

pub(super) fn path_program(search_path: &OsStr, name: &str) -> Option<PathBuf> {
    let pathext = if cfg!(windows) {
        Some(std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()))
    } else {
        None
    };
    davinci_sys::process::resolve_program_in(name, search_path, pathext.as_deref())
}

pub(super) fn invocation_for_path(
    path: &Path,
    args: Vec<String>,
    search_path: &OsStr,
) -> Result<ServerInvocation> {
    if !path.is_absolute() {
        return Err(IntelligenceError::new(
            "server_not_installed",
            "Language-server candidates must resolve to absolute installed paths",
        ));
    }
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "cmd" | "bat" | "ps1" | "sh") {
        return Err(IntelligenceError::new(
            "server_not_installed",
            "Shell launchers are not eligible language-server commands",
        ));
    }
    if matches!(extension.as_str(), "js" | "mjs" | "cjs") {
        let node = path_program(search_path, "node").ok_or_else(|| {
            IntelligenceError::new(
                "server_not_installed",
                "A verified Node executable is required for this installed language server",
            )
        })?;
        let script = path.canonicalize().map_err(|_| {
            IntelligenceError::new(
                "server_not_installed",
                "Language-server script is unavailable",
            )
        })?;
        // Node's module resolver cannot load Windows verbatim (\\?\) script paths.
        // Keep canonical identity for fingerprinting; use the file-URI roundtrip for argv.
        let argv_script = url::Url::from_file_path(&script)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .unwrap_or_else(|| script.clone());
        let mut node_args = vec![argv_script.to_string_lossy().into_owned()];
        node_args.extend(args);
        let mut invocation = ServerInvocation::new(node, node_args).map_err(|_| {
            IntelligenceError::new(
                "server_not_installed",
                "Language-server runtime is unavailable",
            )
        })?;
        invocation.executable_fingerprint = fingerprint_with_files(&invocation, &[script]);
        return Ok(invocation);
    }
    ServerInvocation::new(path.to_path_buf(), args).map_err(|_| {
        IntelligenceError::new(
            "server_not_installed",
            "Language-server executable is unavailable",
        )
    })
}

pub(super) fn named(
    search_path: &OsStr,
    name: &str,
    args: Vec<String>,
) -> Option<ServerInvocation> {
    let path = path_program(search_path, name)?;
    invocation_for_path(&path, args, search_path).ok()
}

pub(super) fn env_absolute(
    name: &str,
    args: Vec<String>,
    search_path: &OsStr,
) -> Option<ServerInvocation> {
    let path = PathBuf::from(std::env::var_os(name)?);
    path.is_absolute()
        .then(|| invocation_for_path(&path, args, search_path))
        .and_then(Result::ok)
}

fn fingerprint_with_files(invocation: &ServerInvocation, files: &[PathBuf]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(invocation.executable_fingerprint.as_bytes());
    for file in files {
        hasher.update([0]);
        hasher.update(file.to_string_lossy().as_bytes());
        if let Ok(metadata) = std::fs::metadata(file) {
            hasher.update(metadata.len().to_le_bytes());
            if let Ok(modified) = metadata.modified().and_then(|v| {
                v.duration_since(std::time::UNIX_EPOCH)
                    .map_err(std::io::Error::other)
            }) {
                hasher.update(modified.as_nanos().to_le_bytes());
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_script_argv_is_never_a_windows_verbatim_path() {
        let node = davinci_sys::process::resolve_program("node");
        let Some(node_dir) = node.parent().filter(|_| node.is_absolute()) else {
            return; // Node is not installed; nothing to launch.
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.cjs");
        std::fs::write(&script, "").unwrap();
        let search_path = std::env::join_paths([node_dir]).unwrap();
        let invocation =
            invocation_for_path(&script, vec!["--stdio".into()], &search_path).unwrap();
        assert!(
            !invocation.args[0].starts_with(r"\\?\"),
            "{:?}",
            invocation.args
        );
        assert!(Path::new(&invocation.args[0]).is_file());
        assert_eq!(invocation.args[1], "--stdio");
    }
}
