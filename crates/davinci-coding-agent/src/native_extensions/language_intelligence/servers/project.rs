//! Static Rust/Python project-root resolution. This module never executes code.
use super::super::identity::{LanguageFamily, ResolvedProject};
use super::super::metadata::{MetadataClass, ResolutionContext};
use super::super::protocol::{IntelligenceError, Result};
use globset::{Glob, GlobSetBuilder};
use std::path::{Path, PathBuf};
use std::str::FromStr;

const MANIFEST_CAP: usize = 256 * 1024;
const MAX_ANCESTORS: usize = 64;

pub(super) fn resolve_project(
    family: LanguageFamily,
    context: &ResolutionContext,
    source: &Path,
    explicit_roots: &[PathBuf],
) -> Result<ResolvedProject> {
    context.budget.check()?;
    let source = context.reader.resolve_path(
        source,
        MetadataClass::WorkspaceConfiguration,
        &context.budget,
    )?;
    if !source.starts_with(&context.workspace) {
        return Err(IntelligenceError::new(
            "outside_workspace",
            "Source path escapes the authorized workspace",
        ));
    }
    if let Some(root) = explicit_root(context, &source, explicit_roots)? {
        return Ok(ResolvedProject {
            workspace: context.workspace.clone(),
            root,
            family,
            analysis_environment: None,
            config_files: Vec::new(),
            limitations: Vec::new(),
        });
    }
    match family {
        LanguageFamily::TypeScript => resolve_typescript(context, &source),
        LanguageFamily::Rust => resolve_rust(context, &source),
        LanguageFamily::Python => resolve_python(context, &source),
    }
}

fn explicit_root(
    context: &ResolutionContext,
    source: &Path,
    roots: &[PathBuf],
) -> Result<Option<PathBuf>> {
    let mut matches = Vec::new();
    for root in roots {
        if root.is_absolute()
            || root
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(IntelligenceError::new(
                "invalid_settings",
                "Explicit project roots must be workspace-relative",
            ));
        }
        let joined = context.workspace.join(root);
        let canonical = context.reader.resolve_path(
            &joined,
            MetadataClass::WorkspaceConfiguration,
            &context.budget,
        )?;
        if source.starts_with(&canonical) {
            matches.push(canonical);
        }
    }
    matches.sort_by_key(|p| p.components().count());
    Ok(matches.pop())
}

fn ancestors<'a>(workspace: &'a Path, source: &'a Path) -> impl Iterator<Item = &'a Path> {
    source
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .take(MAX_ANCESTORS)
        .take_while(move |path| path.starts_with(workspace))
}

fn exists(context: &ResolutionContext, path: &Path) -> bool {
    context
        .reader
        .resolve_path(path, MetadataClass::WorkspaceConfiguration, &context.budget)
        .is_ok()
}

fn resolve_typescript(context: &ResolutionContext, source: &Path) -> Result<ResolvedProject> {
    for dir in ancestors(&context.workspace, source) {
        for marker in ["tsconfig.json", "jsconfig.json", "package.json"] {
            let path = dir.join(marker);
            if exists(context, &path) {
                return Ok(ResolvedProject {
                    workspace: context.workspace.clone(),
                    root: dir.to_path_buf(),
                    family: LanguageFamily::TypeScript,
                    analysis_environment: None,
                    config_files: vec![path],
                    limitations: Vec::new(),
                });
            }
        }
    }
    Ok(ResolvedProject {
        workspace: context.workspace.clone(),
        root: context.workspace.clone(),
        family: LanguageFamily::TypeScript,
        analysis_environment: None,
        config_files: Vec::new(),
        limitations: Vec::new(),
    })
}

fn read_toml(context: &ResolutionContext, path: &Path) -> Result<toml_edit::DocumentMut> {
    let raw = context.reader.read(
        path,
        MetadataClass::WorkspaceConfiguration,
        MANIFEST_CAP,
        &context.budget,
    )?;
    let text = std::str::from_utf8(&raw).map_err(|_| {
        IntelligenceError::new("project_resolution_incomplete", "Cargo.toml must be UTF-8")
    })?;
    toml_edit::DocumentMut::from_str(text).map_err(|_| {
        IntelligenceError::new("project_resolution_incomplete", "Cargo.toml is malformed")
    })
}

fn resolve_rust(context: &ResolutionContext, source: &Path) -> Result<ResolvedProject> {
    let mut manifests = Vec::<(PathBuf, toml_edit::DocumentMut)>::new();
    for dir in ancestors(&context.workspace, source) {
        let manifest = dir.join("Cargo.toml");
        if exists(context, &manifest) {
            manifests.push((manifest.clone(), read_toml(context, &manifest)?));
        }
    }
    if manifests.is_empty() {
        return Err(IntelligenceError::new(
            "project_not_found",
            "No Cargo project owns this Rust source",
        ));
    }
    let package = manifests
        .iter()
        .find(|(_, doc)| doc.get("package").is_some())
        .map(|(path, doc)| (path.clone(), doc));
    if let Some((package_manifest, package_doc)) = package {
        if let Some(relative) = package_doc
            .get("package")
            .and_then(|v| v.get("workspace"))
            .and_then(|v| v.as_str())
        {
            let package_dir = package_manifest.parent().unwrap_or(&context.workspace);
            let candidate = package_dir.join(relative).join("Cargo.toml");
            if exists(context, &candidate) {
                let doc = read_toml(context, &candidate)?;
                if doc.get("workspace").is_some() {
                    let root = candidate
                        .parent()
                        .unwrap_or(&context.workspace)
                        .to_path_buf();
                    return rust_result(context, &root, vec![package_manifest, candidate]);
                }
            }
            return Err(IntelligenceError::new(
                "project_resolution_incomplete",
                "package.workspace does not resolve to an authorized Cargo workspace",
            ));
        }
        let package_dir = package_manifest
            .parent()
            .unwrap_or(&context.workspace)
            .to_path_buf();
        for (manifest, doc) in &manifests {
            let Some(root) = manifest.parent() else {
                continue;
            };
            if doc.get("workspace").is_some() && workspace_contains(doc, root, &package_dir)? {
                return rust_result(
                    context,
                    root,
                    vec![package_manifest.clone(), manifest.clone()],
                );
            }
        }
        return rust_result(context, &package_dir, vec![package_manifest]);
    }
    if let Some((manifest, _)) = manifests
        .iter()
        .find(|(_, doc)| doc.get("workspace").is_some())
    {
        return rust_result(
            context,
            manifest.parent().unwrap_or(&context.workspace),
            vec![manifest.clone()],
        );
    }
    Err(IntelligenceError::new(
        "project_resolution_incomplete",
        "Cargo ownership could not be established safely",
    ))
}

fn rust_result(
    context: &ResolutionContext,
    root: &Path,
    mut config_files: Vec<PathBuf>,
) -> Result<ResolvedProject> {
    for name in [
        "Cargo.lock",
        "rust-toolchain.toml",
        "rust-analyzer.toml",
        ".cargo/config.toml",
    ] {
        let path = root.join(name);
        if exists(context, &path) {
            config_files.push(path);
        }
    }
    config_files.sort();
    config_files.dedup();
    Ok(ResolvedProject {
        workspace: context.workspace.clone(),
        root: root.to_path_buf(),
        family: LanguageFamily::Rust,
        analysis_environment: None,
        config_files,
        limitations: Vec::new(),
    })
}

fn workspace_contains(
    doc: &toml_edit::DocumentMut,
    workspace: &Path,
    package: &Path,
) -> Result<bool> {
    if workspace == package {
        return Ok(true);
    }
    let relative = match package.strip_prefix(workspace) {
        Ok(value) => value.to_string_lossy().replace('\\', "/"),
        Err(_) => return Ok(false),
    };
    let Some(table) = doc.get("workspace") else {
        return Ok(false);
    };
    let excluded = table
        .get("exclude")
        .and_then(|v| v.as_array())
        .map(|v| v.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    if matches_patterns(&relative, &excluded)? {
        return Ok(false);
    }
    let members = table
        .get("members")
        .and_then(|v| v.as_array())
        .map(|v| v.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    if members.is_empty() {
        return Ok(false);
    }
    matches_patterns(&relative, &members)
}

fn matches_patterns(relative: &str, patterns: &[&str]) -> Result<bool> {
    if patterns.len() > 1_024 {
        return Err(IntelligenceError::new(
            "project_resolution_incomplete",
            "Cargo member expansion exceeded its bounded limit",
        ));
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|_| {
            IntelligenceError::new(
                "project_resolution_incomplete",
                "Cargo workspace member pattern is invalid",
            )
        })?);
    }
    let set = builder.build().map_err(|_| {
        IntelligenceError::new(
            "project_resolution_incomplete",
            "Cargo workspace member patterns are invalid",
        )
    })?;
    Ok(set.is_match(relative))
}

fn resolve_python(context: &ResolutionContext, source: &Path) -> Result<ResolvedProject> {
    let dirs: Vec<_> = ancestors(&context.workspace, source).collect();
    for marker in ["pyrightconfig.json", "pyproject.toml"] {
        if let Some(dir) = dirs
            .iter()
            .copied()
            .find(|dir| exists(context, &dir.join(marker)))
        {
            let mut configs = vec![dir.join(marker)];
            if marker == "pyrightconfig.json" && exists(context, &dir.join("pyproject.toml")) {
                configs.push(dir.join("pyproject.toml"));
            }
            return Ok(ResolvedProject {
                workspace: context.workspace.clone(),
                root: dir.to_path_buf(),
                family: LanguageFamily::Python,
                analysis_environment: None,
                config_files: configs,
                limitations: Vec::new(),
            });
        }
    }
    for dir in dirs {
        for marker in ["setup.cfg", "setup.py", "requirements.txt"] {
            let path = dir.join(marker);
            if exists(context, &path) {
                return Ok(ResolvedProject {
                    workspace: context.workspace.clone(),
                    root: dir.to_path_buf(),
                    family: LanguageFamily::Python,
                    analysis_environment: None,
                    config_files: vec![path],
                    limitations: vec!["fallback_python_project_marker".into()],
                });
            }
        }
    }
    Err(IntelligenceError::new(
        "project_not_found",
        "No supported Python project boundary was found",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::language_intelligence::metadata::FsMetadataReader;
    use crate::native_extensions::language_intelligence::protocol::RequestBudget;
    use std::sync::Arc;
    use std::time::Duration;

    fn context(root: &Path) -> ResolutionContext {
        ResolutionContext {
            workspace: root.to_path_buf(),
            reader: Arc::new(FsMetadataReader::new(root.to_path_buf(), Vec::new())),
            budget: RequestBudget::from_timeout(Duration::from_secs(3)),
        }
    }

    #[test]
    fn rust_member_uses_workspace_root_and_python_nesting_stays_local() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("crates/core/src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/core/Cargo.toml"),
            "[package]\nname=\"core_fixture\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/core/src/lib.rs"), "pub fn x(){}").unwrap();
        let rust = resolve_project(
            LanguageFamily::Rust,
            &context(&root),
            &root.join("crates/core/src/lib.rs"),
            &[],
        )
        .unwrap();
        assert_eq!(rust.root, root);

        std::fs::create_dir_all(root.join("service/nested")).unwrap();
        std::fs::write(root.join("service/pyproject.toml"), "[tool.pyright]\n").unwrap();
        std::fs::write(root.join("service/nested/pyrightconfig.json"), "{}").unwrap();
        std::fs::write(root.join("service/nested/app.py"), "x=1").unwrap();
        let python = resolve_project(
            LanguageFamily::Python,
            &context(&root),
            &root.join("service/nested/app.py"),
            &[],
        )
        .unwrap();
        assert_eq!(
            python.root,
            root.join("service/nested").canonicalize().unwrap()
        );
    }

    #[test]
    fn setup_py_is_never_executed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let marker = root.join("executed");
        std::fs::write(
            root.join("setup.py"),
            format!("open({:?}, 'w').write('bad')", marker),
        )
        .unwrap();
        std::fs::write(root.join("app.py"), "x=1").unwrap();
        let project = resolve_project(
            LanguageFamily::Python,
            &context(&root),
            &root.join("app.py"),
            &[],
        )
        .unwrap();
        assert_eq!(project.root, root);
        assert!(!marker.exists());
    }
}
