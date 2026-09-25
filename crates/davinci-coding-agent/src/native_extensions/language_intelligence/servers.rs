//! Language-specific project markers, IDs and server discovery live here.

pub(super) mod project;
pub(super) mod python;
pub(super) mod rust;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum ServerKind {
    TypeScriptNative,
    TypeScriptLanguageServer,
    RustAnalyzer,
    Basedpyright,
    Pyright,
}

#[derive(Debug, Clone)]
pub(super) struct ServerCommand {
    pub kind: ServerKind,
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

pub(super) fn path_executable(search_path: &OsStr, name: &str) -> Option<PathBuf> {
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
                .or_else(|| pnpm_shim_package(&path, name))
        })
}

/// Read the package location from a pnpm Windows shim as bounded data. Never
/// execute the shim or interpret shell substitutions; the manifest's entry point
/// still goes through `package_command` and the normal launch permission gate.
fn pnpm_shim_package(bin_dir: &Path, name: &str) -> Option<Package> {
    let file = std::fs::File::open(bin_dir.join(format!("{name}.CMD"))).ok()?;
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 8192 {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    for quoted in text.split('"').skip(1).step_by(2) {
        let Some(relative) = quoted.strip_prefix("%~dp0") else {
            continue;
        };
        if relative.contains(['%', '!', '\r', '\n']) {
            continue;
        }
        let relative = relative.replace('\\', "/");
        let relative = relative.trim_start_matches('/');
        if !relative.starts_with("../global/")
            || !relative.contains(&format!("/node_modules/{name}/"))
        {
            continue;
        }
        let Some(entry) = bin_dir
            .join(relative)
            .canonicalize()
            .ok()
            .filter(|p| p.is_file())
        else {
            continue;
        };
        for parent in entry.ancestors().skip(1).take(4) {
            if let Some(package) = read_package(parent) {
                if package.manifest["name"].as_str() == Some(name) {
                    return Some(package);
                }
            }
        }
    }
    None
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
    let preview = local_package(workspace, project, "@typescript/native-preview");
    let project_typescript = local_package(workspace, project, "typescript");
    let global_typescript = if project_typescript.is_none()
        && (preview.is_none() || backend == Backend::TypeScriptLanguageServer)
    {
        global_package(search_path, "typescript")
    } else {
        None
    };
    // A global compatibility compiler may supply tsserver for the compatibility
    // backend, but it must never become Auto's preferred native TypeScript.
    // Otherwise a CI/host PATH upgrade can silently replace a project-local
    // language-server backend and change semantic results.
    let typescript = project_typescript.as_ref().or(global_typescript.as_ref());
    let ts_version = typescript.and_then(version);
    let tsserver = typescript
        .map(|p| p.root.join("lib/tsserver.js"))
        .filter(|p| p.is_file());
    let native_ts = project_typescript.as_ref().filter(|p| {
        p.manifest["version"]
            .as_str()
            .and_then(|v| v.split('.').next()?.parse::<u32>().ok())
            .is_some_and(|major| major >= 7)
    });
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
                        kind: ServerKind::TypeScriptNative,
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
    if backend != Backend::TypeScriptNative
        && (backend == Backend::TypeScriptLanguageServer
            || native_ts.is_none()
            || tsserver.is_some())
    {
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
                kind: ServerKind::TypeScriptLanguageServer,
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

pub(super) fn discover_for_family(
    family: crate::native_extensions::language_intelligence::LanguageFamily,
    workspace: &Path,
    project: &Path,
    config: &crate::native_extensions::language_intelligence::LanguageIntelligenceConfig,
    search_path: &OsStr,
) -> Result<Vec<ServerCommand>> {
    use crate::native_extensions::language_intelligence::LanguageFamily;
    match family {
        LanguageFamily::TypeScript => discover(workspace, project, config.typescript.backend, search_path),
        LanguageFamily::Rust => rust::discover(project, &config.rust, search_path),
        LanguageFamily::Python => python::discover(project, &config.python, search_path),
    }
}

pub(super) trait ServerAdapter: Send + Sync {
    fn family(&self) -> crate::native_extensions::language_intelligence::LanguageFamily;
    fn language_id(&self, path: &Path) -> Option<&'static str>;
    fn project_markers(&self) -> &'static [&'static str] { &[] }
}

pub(super) struct TypeScriptAdapter;

impl ServerAdapter for TypeScriptAdapter {
    fn family(&self) -> crate::native_extensions::language_intelligence::LanguageFamily {
        crate::native_extensions::language_intelligence::LanguageFamily::TypeScript
    }
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


pub(super) struct RustAdapter;
pub(super) struct PythonAdapter;

impl ServerAdapter for RustAdapter {
    fn family(&self) -> crate::native_extensions::language_intelligence::LanguageFamily {
        crate::native_extensions::language_intelligence::LanguageFamily::Rust
    }
    fn language_id(&self, path: &Path) -> Option<&'static str> {
        (path.extension()?.to_str()? == "rs").then_some("rust")
    }
}

impl ServerAdapter for PythonAdapter {
    fn family(&self) -> crate::native_extensions::language_intelligence::LanguageFamily {
        crate::native_extensions::language_intelligence::LanguageFamily::Python
    }
    fn language_id(&self, path: &Path) -> Option<&'static str> {
        matches!(path.extension()?.to_str()?, "py" | "pyi").then_some("python")
    }
}

static TYPESCRIPT_ADAPTER: TypeScriptAdapter = TypeScriptAdapter;
static RUST_ADAPTER: RustAdapter = RustAdapter;
static PYTHON_ADAPTER: PythonAdapter = PythonAdapter;

pub(super) fn family_for_path(path: &Path) -> Option<crate::native_extensions::language_intelligence::LanguageFamily> {
    use crate::native_extensions::language_intelligence::LanguageFamily;
    match path.extension()?.to_str()? {
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Some(LanguageFamily::TypeScript),
        "rs" => Some(LanguageFamily::Rust),
        "py" | "pyi" => Some(LanguageFamily::Python),
        _ => None,
    }
}

pub(super) fn adapter(family: crate::native_extensions::language_intelligence::LanguageFamily) -> &'static dyn ServerAdapter {
    use crate::native_extensions::language_intelligence::LanguageFamily;
    match family {
        LanguageFamily::TypeScript => &TYPESCRIPT_ADAPTER,
        LanguageFamily::Rust => &RUST_ADAPTER,
        LanguageFamily::Python => &PYTHON_ADAPTER,
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
            fn family(&self) -> crate::native_extensions::language_intelligence::LanguageFamily {
                crate::native_extensions::language_intelligence::LanguageFamily::TypeScript
            }
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
    fn pnpm_windows_shim_discovers_package_without_executing_shell() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let store = dir.path().join("global/v11/hash");
        package(
            &store,
            "typescript-language-server",
            "5.3.0",
            Some(("typescript-language-server", "lib/cli.mjs")),
        );
        std::fs::write(bin.join("typescript-language-server.CMD"),
            "@SETLOCAL\nnode \"%~dp0\\..\\global\\v11\\hash\\node_modules\\typescript-language-server\\lib\\cli.mjs\" %*\n").unwrap();
        let found = global_package(bin.as_os_str(), "typescript-language-server").unwrap();
        assert_eq!(version(&found).as_deref(), Some("5.3.0"));
        let (program, args) = package_command(
            &found,
            "typescript-language-server",
            &std::env::var_os("PATH").unwrap(),
        )
        .unwrap();
        assert!(!program.to_string_lossy().to_lowercase().ends_with(".cmd"));
        assert!(args[0].ends_with("cli.mjs"));
        std::fs::write(bin.join("typescript-language-server.CMD"),
            "node \"%~dp0\\..\\global\\%UNTRUSTED%\\node_modules\\typescript-language-server\\lib\\cli.mjs\" %*\n").unwrap();
        assert!(global_package(bin.as_os_str(), "typescript-language-server").is_none());
        std::fs::write(bin.join("typescript-language-server.CMD"), "x".repeat(8193)).unwrap();
        assert!(global_package(bin.as_os_str(), "typescript-language-server").is_none());
    }

    #[test]
    fn global_compiler_fallback_never_overrides_project_typescript() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let global = dir.path().join("global-bin");
        std::fs::create_dir(&root).unwrap();
        package(
            &global,
            "typescript",
            "5.8.1",
            Some(("tsserver", "lib/tsserver.js")),
        );
        package(
            &global,
            "typescript-language-server",
            "5.3.0",
            Some(("typescript-language-server", "lib/cli.mjs")),
        );
        let paths = std::iter::once(global.clone())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect::<Vec<_>>());
        let search = std::env::join_paths(paths).unwrap();
        let found = discover(&root, &root, Backend::Auto, &search).unwrap();
        assert_eq!(found[0].typescript_version.as_deref(), Some("5.8.1"));
        package(
            &root,
            "@typescript/native-preview",
            "7.0.0-dev",
            Some(("tsgo", "bin/tsgo")),
        );
        let found = discover(&root, &root, Backend::Auto, &search).unwrap();
        assert_eq!(found[0].kind, ServerKind::TypeScriptNative);
        package(
            &root,
            "typescript",
            "5.7.0",
            Some(("tsserver", "lib/tsserver.js")),
        );
        let found = discover(&root, &root, Backend::Auto, &search).unwrap();
        assert_eq!(found[0].typescript_version.as_deref(), Some("5.7.0"));
        assert!(found[0].initialization_options["tsserver"]["path"]
            .as_str()
            .unwrap()
            .contains("project"));
    }

    #[test]
    fn auto_does_not_let_global_native_typescript_displace_project_language_server() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let global = dir.path().join("global-bin");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&global).unwrap();
        package(&global, "typescript", "7.0.0", Some(("tsc", "bin/tsc.cjs")));
        package(
            &root,
            "typescript-language-server",
            "fixture",
            Some(("typescript-language-server", "server.cjs")),
        );
        let paths = std::iter::once(global.clone())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect::<Vec<_>>());
        let search = std::env::join_paths(paths).unwrap();

        let found = discover(&root, &root, Backend::Auto, &search).unwrap();

        assert_eq!(found[0].kind, Backend::TypeScriptLanguageServer);
        assert!(
            found[0].args.iter().any(|arg| arg.ends_with("server.cjs")),
            "{:?}",
            found[0].args
        );
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
