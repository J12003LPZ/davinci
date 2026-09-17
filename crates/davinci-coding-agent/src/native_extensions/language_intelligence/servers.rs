//! Language-specific project markers, IDs and server discovery live here.

use super::protocol::{IntelligenceError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
    #[default]
    Auto,
    #[serde(rename = "typescriptNative")]
    TypeScriptNative,
    #[serde(rename = "typescriptLanguageServer")]
    TypeScriptLanguageServer,
}

#[derive(Debug, Clone)]
pub(super) struct ServerCommand {
    pub kind: Backend,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub workspace: PathBuf,
    pub version: Option<String>,
    pub typescript_version: Option<String>,
    pub initialization_options: Value,
}

impl ServerCommand {
    pub fn command(&self) -> std::process::Command {
        use crate::semantic::manager::{
            build_sanitized_command, LspServerConfig, ServerExecutionPolicy,
        };
        build_sanitized_command(&LspServerConfig {
            executable: self.program.clone(),
            args: self.args.clone(),
            languages: Vec::new(),
            root_dir: self.workspace.clone(),
            policy: ServerExecutionPolicy::NavigationOnly,
            idle_timeout: std::time::Duration::from_secs(300),
            env_allowlist: Vec::new(),
            executable_hash: None,
        })
    }
}

#[derive(Debug)]
struct Package {
    root: PathBuf,
    manifest: Value,
}

fn read_package(path: &Path) -> Option<Package> {
    let file = std::fs::File::open(path.join("package.json")).ok()?;
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }
    Some(Package {
        root: path.to_path_buf(),
        manifest: serde_json::from_slice(&bytes).ok()?,
    })
}

fn local_package(workspace: &Path, project: &Path, name: &str) -> Option<Package> {
    project
        .ancestors()
        .take_while(|path| path.starts_with(workspace))
        .find_map(|path| read_package(&path.join("node_modules").join(name)))
}

fn path_executable(search_path: &OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(search_path)
        .filter(|path| path.is_absolute())
        .find_map(|path| {
            let executable = path.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            });
            executable
                .is_file()
                .then(|| executable.canonicalize().ok())
                .flatten()
        })
}

fn global_package(search_path: &OsStr, name: &str) -> Option<Package> {
    std::env::split_paths(search_path)
        .filter(|path| path.is_absolute())
        .find_map(|path| {
            read_package(&path.join("node_modules").join(name))
                .or_else(|| read_package(&path.join("../lib/node_modules").join(name)))
        })
}

fn version(package: &Package) -> Option<String> {
    package.manifest["version"]
        .as_str()
        .map(|v| super::normalize::compact(v, 128))
}

fn package_command(
    package: &Package,
    bin: &str,
    search_path: &OsStr,
) -> Option<(PathBuf, Vec<String>)> {
    let relative = package.manifest["bin"]
        .as_str()
        .or_else(|| package.manifest["bin"][bin].as_str())?;
    let entry = package.root.join(relative).canonicalize().ok()?;
    if !entry.is_file() {
        return None;
    }
    let mut header = [0; 128];
    let count = std::fs::File::open(&entry).ok()?.read(&mut header).ok()?;
    let header = String::from_utf8_lossy(&header[..count]);
    if header.starts_with("#!") && header.lines().next()?.contains("node")
        || entry
            .extension()
            .is_some_and(|ext| matches!(ext.to_str(), Some("js" | "mjs" | "cjs")))
    {
        // Node's module resolver cannot load Windows verbatim (\\?\) script paths.
        // Keep canonical identity internally and use the file-URI roundtrip for argv.
        let script = url::Url::from_file_path(&entry).ok()?.to_file_path().ok()?;
        Some((
            path_executable(search_path, "node")?,
            vec![script.to_str()?.into()],
        ))
    } else if cfg!(windows) && entry.extension().is_none_or(|ext| ext != "exe") {
        None // Never invoke Windows cmd/bat shims or a shell.
    } else {
        Some((entry, Vec::new()))
    }
}

pub(super) fn discover(
    workspace: &Path,
    project: &Path,
    backend: Backend,
    search_path: &OsStr,
) -> Result<Vec<ServerCommand>> {
    let typescript = local_package(workspace, project, "typescript");
    let ts_version = typescript.as_ref().and_then(version);
    let tsserver = typescript
        .as_ref()
        .map(|p| p.root.join("lib/tsserver.js"))
        .filter(|p| p.is_file());
    let native_ts = typescript.as_ref().filter(|p| {
        p.manifest["version"]
            .as_str()
            .and_then(|v| v.split('.').next()?.parse::<u32>().ok())
            .is_some_and(|major| major >= 7)
    });
    let preview = local_package(workspace, project, "@typescript/native-preview");
    let mut candidates = Vec::new();
    if backend != Backend::TypeScriptLanguageServer {
        let native = native_ts
            .map(|p| (p, "tsc"))
            .or_else(|| preview.as_ref().map(|p| (p, "tsgo")));
        if let Some((package, bin)) = native {
            // An explicit native override can select the preview. Auto preserves
            // a project's JS-based TypeScript instead of silently replacing it.
            if backend == Backend::TypeScriptNative || native_ts.is_some() || tsserver.is_none() {
                if let Some((program, mut args)) = package_command(package, bin, search_path) {
                    args.extend(["--lsp".into(), "--stdio".into()]);
                    candidates.push(ServerCommand {
                        kind: Backend::TypeScriptNative,
                        program,
                        args,
                        workspace: project.into(),
                        version: version(package),
                        typescript_version: version(package),
                        initialization_options: json!({"disableAutomaticTypingAcquisition":true}),
                    });
                }
            }
        }
    }
    if backend != Backend::TypeScriptNative && (typescript.is_none() || tsserver.is_some()) {
        let package = local_package(workspace, project, "typescript-language-server")
            .or_else(|| global_package(search_path, "typescript-language-server"));
        let launch = package
            .as_ref()
            .and_then(|p| package_command(p, "typescript-language-server", search_path))
            .or_else(|| {
                path_executable(search_path, "typescript-language-server").map(|p| (p, Vec::new()))
            });
        if let Some((program, mut args)) = launch {
            args.push("--stdio".into());
            let mut options = json!({"hostInfo":"DaVinci", "disableAutomaticTypingAcquisition":true,
                "tsserver":{"useSyntaxServer":"never","logVerbosity":"off"}});
            if let Some(path) = tsserver {
                options["tsserver"]["path"] = json!(url::Url::from_file_path(&path)
                    .ok()
                    .and_then(|uri| uri.to_file_path().ok())
                    .unwrap_or(path));
            }
            candidates.push(ServerCommand {
                kind: Backend::TypeScriptLanguageServer,
                program,
                args,
                workspace: project.into(),
                version: package.as_ref().and_then(version),
                typescript_version: ts_version,
                initialization_options: options,
            });
        }
    }
    if candidates.is_empty() {
        Err(IntelligenceError::new("server_not_installed", "Install a compatible project-local TypeScript language server or select an installed backend; DaVinci never downloads servers"))
    } else {
        Ok(candidates)
    }
}

pub(super) trait ServerAdapter {
    fn language_id(&self, path: &Path) -> Option<&'static str>;
    fn project_markers(&self) -> &'static [&'static str];
}

pub(super) struct TypeScriptAdapter;

impl ServerAdapter for TypeScriptAdapter {
    fn language_id(&self, path: &Path) -> Option<&'static str> {
        match path.extension()?.to_str()? {
            "ts" | "mts" | "cts" => Some("typescript"),
            "tsx" => Some("typescriptreact"),
            "js" | "mjs" | "cjs" => Some("javascript"),
            "jsx" => Some("javascriptreact"),
            _ => None,
        }
    }
    fn project_markers(&self) -> &'static [&'static str] {
        &["tsconfig.json", "jsconfig.json", "package.json"]
    }
}

pub(super) fn project_root(
    workspace: &Path,
    source: &Path,
    adapter: &dyn ServerAdapter,
) -> Result<PathBuf> {
    if !source.starts_with(workspace) {
        return Err(IntelligenceError::new(
            "outside_workspace",
            "Source path escapes the workspace",
        ));
    }
    if adapter.language_id(source).is_none() {
        return Err(IntelligenceError::new(
            "unsupported_language",
            "V1 supports TypeScript and JavaScript source files",
        ));
    }
    for directory in source.parent().into_iter().flat_map(Path::ancestors) {
        if !directory.starts_with(workspace) {
            break;
        }
        if adapter
            .project_markers()
            .iter()
            .any(|marker| directory.join(marker).is_file())
        {
            return Ok(directory.to_path_buf());
        }
    }
    Ok(workspace.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_v1_extensions_have_correct_language_ids() {
        for (extension, language) in [
            ("ts", "typescript"),
            ("tsx", "typescriptreact"),
            ("js", "javascript"),
            ("jsx", "javascriptreact"),
            ("mts", "typescript"),
            ("cts", "typescript"),
            ("mjs", "javascript"),
            ("cjs", "javascript"),
        ] {
            assert_eq!(
                TypeScriptAdapter.language_id(Path::new(&format!("file.{extension}"))),
                Some(language)
            );
        }
        for path in ["file.rs", "file.py", "file", "package.json"] {
            assert_eq!(TypeScriptAdapter.language_id(Path::new(path)), None);
        }
    }

    #[test]
    fn nearest_monorepo_markers_and_workspace_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let package = root.join("packages/app");
        std::fs::create_dir_all(package.join("src")).unwrap();
        let file = package.join("src/a.ts");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            project_root(&root, &file, &TypeScriptAdapter).unwrap(),
            root
        );
        std::fs::write(root.join("package.json"), "{}").unwrap();
        for marker in ["tsconfig.json", "jsconfig.json", "package.json"] {
            std::fs::write(package.join(marker), "{}").unwrap();
            assert_eq!(
                project_root(&root, &file, &TypeScriptAdapter).unwrap(),
                package
            );
            std::fs::remove_file(package.join(marker)).unwrap();
        }
        assert_eq!(
            project_root(&package, &root.join("outside.ts"), &TypeScriptAdapter)
                .unwrap_err()
                .code,
            "outside_workspace"
        );
    }

    #[test]
    fn generic_project_detection_uses_adapter_not_typescript_constants() {
        struct FixtureAdapter;
        impl ServerAdapter for FixtureAdapter {
            fn language_id(&self, path: &Path) -> Option<&'static str> {
                (path.extension()?.to_str()? == "fixture").then_some("fixture")
            }
            fn project_markers(&self) -> &'static [&'static str] {
                &["fixture.project"]
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = root.join("nested");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("fixture.project"), "").unwrap();
        let file = project.join("test.fixture");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            project_root(&root, &file, &FixtureAdapter).unwrap(),
            project
        );
        assert_eq!(
            project_root(&root, &file, &TypeScriptAdapter)
                .unwrap_err()
                .code,
            "unsupported_language"
        );
    }

    fn package(root: &Path, name: &str, version: &str, bin: Option<(&str, &str)>) {
        let path = root.join("node_modules").join(name);
        std::fs::create_dir_all(&path).unwrap();
        let mut manifest = serde_json::json!({"name":name,"version":version});
        if let Some((name, entry)) = bin {
            manifest["bin"] = serde_json::json!({name:entry});
            let entry = path.join(entry);
            std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
            std::fs::write(entry, "#!/usr/bin/env node\n").unwrap();
        }
        std::fs::write(path.join("package.json"), manifest.to_string()).unwrap();
    }

    #[test]
    fn hoisted_local_server_uses_nearest_project_typescript() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = root.join("packages/app");
        std::fs::create_dir_all(&project).unwrap();
        package(
            &root,
            "typescript-language-server",
            "5.1.0",
            Some(("typescript-language-server", "lib/cli.mjs")),
        );
        package(&project, "typescript", "5.9.3", None);
        std::fs::create_dir_all(project.join("node_modules/typescript/lib")).unwrap();
        std::fs::write(project.join("node_modules/typescript/lib/tsserver.js"), "").unwrap();
        let servers = discover(
            &root,
            &project,
            Backend::Auto,
            &std::env::var_os("PATH").unwrap(),
        )
        .unwrap();
        assert_eq!(servers[0].kind, Backend::TypeScriptLanguageServer);
        assert_eq!(servers[0].typescript_version.as_deref(), Some("5.9.3"));
        assert_eq!(servers[0].version.as_deref(), Some("5.1.0"));
        assert!(servers[0].initialization_options["tsserver"]["path"]
            .as_str()
            .unwrap()
            .contains("packages"));
        assert_eq!(
            servers[0].initialization_options["disableAutomaticTypingAcquisition"],
            true
        );
        assert!(servers[0].args.last().unwrap() == "--stdio");
    }

    #[test]
    fn native_toolchain_selection_does_not_assume_old_tsgo_subcommand() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        package(&root, "typescript", "7.0.1", Some(("tsc", "bin/tsc")));
        let servers = discover(
            &root,
            &root,
            Backend::Auto,
            &std::env::var_os("PATH").unwrap(),
        )
        .unwrap();
        assert_eq!(servers[0].kind, Backend::TypeScriptNative);
        assert!(servers[0]
            .args
            .ends_with(&["--lsp".into(), "--stdio".into()]));
        assert_eq!(servers[0].typescript_version.as_deref(), Some("7.0.1"));
    }

    #[test]
    fn missing_server_and_explicit_backend_are_observable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert_eq!(
            discover(&root, &root, Backend::Auto, std::ffi::OsStr::new(""))
                .unwrap_err()
                .code,
            "server_not_installed"
        );
        package(
            &root,
            "@typescript/native-preview",
            "7.0.0-dev",
            Some(("tsgo", "bin/tsgo")),
        );
        let servers = discover(
            &root,
            &root,
            Backend::TypeScriptNative,
            &std::env::var_os("PATH").unwrap(),
        )
        .unwrap();
        assert_eq!(servers[0].kind, Backend::TypeScriptNative);
        assert!(discover(
            &root,
            &root,
            Backend::TypeScriptLanguageServer,
            std::ffi::OsStr::new("")
        )
        .is_err());
    }
}
