//! Static Rust and Python project resolution. No subprocess execution.

use crate::native_extensions::language_intelligence::identity::{LanguageFamily, ResolvedProject};
use crate::native_extensions::language_intelligence::metadata::{
    MetadataClass, ResolutionContext,
};
use crate::native_extensions::language_intelligence::protocol::{
    IntelligenceError, Result,
};
use std::path::{Component, Path, PathBuf};

const MAX_MANIFEST: usize = 256 * 1024;
const MAX_ANCESTORS: usize = 64;
const MAX_MEMBER_PATHS: usize = 1024;

pub(crate) fn resolve_project(
    family: LanguageFamily,
    context: &ResolutionContext,
    source: &Path,
    explicit_roots: &[PathBuf],
) -> Result<ResolvedProject> {
    let workspace = context
        .reader
        .resolve_path(
            &context.workspace,
            MetadataClass::WorkspaceConfiguration,
            &context.budget,
        )?;
    let source = context.reader.resolve_path(
        source,
        MetadataClass::WorkspaceConfiguration,
        &context.budget,
    )?;
    if !source.starts_with(&workspace) || !source.is_file() {
        return Err(IntelligenceError::new(
            "invalid_source_path",
            "Source must be a file inside the authorized workspace",
        ));
    }
    if let Some(root) = explicit_root(&workspace, &source, explicit_roots) {
        return Ok(ResolvedProject {
            workspace,
            root,
            family,
            analysis_environment: None,
            config_files: Vec::new(),
            limitations: vec!["explicit_project_root".into()],
        });
    }
    match family {
        LanguageFamily::Rust => resolve_rust(context, workspace, source),
        LanguageFamily::Python => resolve_python(context, workspace, source),
        LanguageFamily::TypeScript => Err(IntelligenceError::new(
            "unsupported_language",
            "TypeScript project resolution remains on the compatibility resolver",
        )),
    }
}

fn explicit_root(workspace: &Path, source: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter_map(|root| {
            let joined = if root.is_absolute() {
                root.clone()
            } else {
                workspace.join(root)
            };
            let canonical = joined.canonicalize().ok()?;
            (canonical.starts_with(workspace)
                && source.starts_with(&canonical)
                && canonical.is_dir())
            .then_some(canonical)
        })
        .max_by_key(|root| root.components().count())
}

fn resolve_rust(
    context: &ResolutionContext,
    workspace: PathBuf,
    source: PathBuf,
) -> Result<ResolvedProject> {
    if source.extension().and_then(|value| value.to_str()) != Some("rs") {
        return Err(IntelligenceError::new(
            "unsupported_language",
            "Rust project resolution requires a .rs source file",
        ));
    }
    let mut manifests = Vec::new();
    for directory in source
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .take(MAX_ANCESTORS)
    {
        if !directory.starts_with(&workspace) {
            break;
        }
        let manifest = directory.join("Cargo.toml");
        if manifest.is_file() {
            let doc = read_toml(context, &manifest)?;
            manifests.push((directory.to_path_buf(), manifest, doc));
        }
        if directory == workspace {
            break;
        }
    }
    if manifests.is_empty() {
        return Err(IntelligenceError::new(
            "project_not_found",
            "No Cargo.toml owns this Rust source",
        ));
    }

    // Explicit package.workspace wins when it resolves to an ancestor manifest.
    if let Some((package_dir, _, package_doc)) = manifests.first() {
        if let Some(workspace_value) = package_doc
            .get("package")
            .and_then(|value| value.get("workspace"))
            .and_then(|value| value.as_str())
        {
            let candidate = normalize_join(package_dir, Path::new(workspace_value))?;
            if !candidate.starts_with(&workspace) {
                return Err(IntelligenceError::new(
                    "project_resolution_incomplete",
                    "package.workspace escapes the authorized checkout",
                ));
            }
            let manifest = candidate.join("Cargo.toml");
            if manifest.is_file() {
                let doc = read_toml(context, &manifest)?;
                if doc.get("workspace").is_some() {
                    return Ok(ResolvedProject {
                        workspace,
                        root: candidate,
                        family: LanguageFamily::Rust,
                        analysis_environment: None,
                        config_files: vec![manifest],
                        limitations: Vec::new(),
                    });
                }
            }
            return Err(IntelligenceError::new(
                "project_resolution_incomplete",
                "package.workspace does not name a valid workspace manifest",
            ));
        }
    }

    // Prefer the nearest ancestor workspace that statically includes the
    // package. A virtual workspace is valid; a nested independent workspace
    // prevents walking out to an unrelated outer workspace.
    let package_root = manifests
        .first()
        .map(|(root, _, _)| root.clone())
        .expect("non-empty manifest list");
    for (root, manifest, doc) in &manifests {
        if doc.get("workspace").is_none() {
            continue;
        }
        if workspace_contains(context, root, doc, &package_root)? {
            return Ok(ResolvedProject {
                workspace,
                root: root.clone(),
                family: LanguageFamily::Rust,
                analysis_environment: None,
                config_files: vec![manifest.clone()],
                limitations: Vec::new(),
            });
        }
        if root == &package_root {
            break;
        }
    }

    let (root, manifest, _) = manifests
        .first()
        .cloned()
        .expect("non-empty manifest list");
    Ok(ResolvedProject {
        workspace,
        root,
        family: LanguageFamily::Rust,
        analysis_environment: None,
        config_files: vec![manifest],
        limitations: vec!["cargo_workspace_membership_not_proven".into()],
    })
}

fn workspace_contains(
    context: &ResolutionContext,
    workspace_root: &Path,
    doc: &toml_edit::DocumentMut,
    package_root: &Path,
) -> Result<bool> {
    if workspace_root == package_root {
        return Ok(true);
    }
    let Some(table) = doc.get("workspace") else {
        return Ok(false);
    };
    if let Some(excludes) = table.get("exclude").and_then(|value| value.as_array()) {
        for value in excludes.iter().filter_map(|value| value.as_str()) {
            if member_pattern_matches(workspace_root, value, package_root)? {
                return Ok(false);
            }
        }
    }
    let Some(members) = table.get("members").and_then(|value| value.as_array()) else {
        return Ok(false);
    };
    if members.len() > MAX_MEMBER_PATHS {
        return Err(IntelligenceError::new(
            "project_resolution_incomplete",
            "Cargo workspace has too many member patterns",
        ));
    }
    for value in members.iter().filter_map(|value| value.as_str()) {
        if member_pattern_matches(workspace_root, value, package_root)? {
            return Ok(true);
        }
    }

    // Keep a bounded directory read in the policy seam so wildcard-heavy
    // manifests cannot silently trigger an unbounded filesystem traversal.
    let _ = context.reader.list_directory(
        workspace_root,
        MetadataClass::WorkspaceConfiguration,
        MAX_MEMBER_PATHS,
        &context.budget,
    )?;
    Ok(false)
}

fn member_pattern_matches(root: &Path, pattern: &str, package_root: &Path) -> Result<bool> {
    let normalized = pattern.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains("..") {
        return Ok(false);
    }
    let package_relative = package_root
        .strip_prefix(root)
        .map_err(|_| {
            IntelligenceError::new(
                "project_resolution_incomplete",
                "Cargo member is outside its workspace",
            )
        })?
        .to_string_lossy()
        .replace('\\', "/");

    if !normalized.contains('*') {
        return Ok(package_relative.trim_end_matches('/') == normalized.trim_end_matches('/'));
    }
    let p: Vec<&str> = normalized.split('/').collect();
    let v: Vec<&str> = package_relative.split('/').collect();
    if p.len() != v.len() {
        return Ok(false);
    }
    Ok(p.iter().zip(v.iter()).all(|(pattern, value)| {
        *pattern == "*" || (!pattern.contains('*') && *pattern == *value)
    }))
}

fn resolve_python(
    context: &ResolutionContext,
    workspace: PathBuf,
    source: PathBuf,
) -> Result<ResolvedProject> {
    if !matches!(
        source.extension().and_then(|value| value.to_str()),
        Some("py" | "pyi")
    ) {
        return Err(IntelligenceError::new(
            "unsupported_language",
            "Python project resolution requires a .py or .pyi source file",
        ));
    }

    let mut fallback = None;
    for directory in source
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .take(MAX_ANCESTORS)
    {
        if !directory.starts_with(&workspace) {
            break;
        }

        let pyright = directory.join("pyrightconfig.json");
        if pyright.is_file() {
            bounded_read(context, &pyright)?;
            return python_project(
                workspace,
                directory.to_path_buf(),
                source,
                vec![pyright],
            );
        }

        let pyproject = directory.join("pyproject.toml");
        if pyproject.is_file() {
            let doc = read_toml(context, &pyproject)?;
            if doc
                .get("tool")
                .is_some_and(|tool| tool.get("basedpyright").is_some() || tool.get("pyright").is_some())
            {
                return python_project(
                    workspace,
                    directory.to_path_buf(),
                    source,
                    vec![pyproject],
                );
            }
            fallback.get_or_insert((directory.to_path_buf(), pyproject));
        }

        for marker in ["setup.cfg", "setup.py", "requirements.txt"] {
            let path = directory.join(marker);
            if path.is_file() && fallback.is_none() {
                bounded_read(context, &path)?;
                fallback = Some((directory.to_path_buf(), path));
            }
        }

        if directory == workspace {
            break;
        }
    }

    let (root, config) = fallback.unwrap_or_else(|| (workspace.clone(), source.clone()));
    python_project(workspace, root, source, vec![config])
}

fn python_project(
    workspace: PathBuf,
    root: PathBuf,
    source: PathBuf,
    config_files: Vec<PathBuf>,
) -> Result<ResolvedProject> {
    let analysis_environment = [".venv", "venv"]
        .into_iter()
        .map(|name| root.join(name))
        .find(|candidate| candidate.is_dir());
    Ok(ResolvedProject {
        workspace,
        root,
        family: LanguageFamily::Python,
        analysis_environment,
        config_files: config_files
            .into_iter()
            .filter(|path| path != &source)
            .collect(),
        limitations: Vec::new(),
    })
}

fn read_toml(
    context: &ResolutionContext,
    path: &Path,
) -> Result<toml_edit::DocumentMut> {
    let bytes = bounded_read(context, path)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        IntelligenceError::new("invalid_project_config", "Project TOML is not UTF-8")
    })?;
    text.parse::<toml_edit::DocumentMut>().map_err(|_| {
        IntelligenceError::new("invalid_project_config", "Project TOML could not be parsed")
    })
}

fn bounded_read(context: &ResolutionContext, path: &Path) -> Result<Vec<u8>> {
    context.reader.read(
        path,
        MetadataClass::WorkspaceConfiguration,
        MAX_MANIFEST,
        &context.budget,
    )
}

fn normalize_join(base: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.is_absolute() {
        return Err(IntelligenceError::new(
            "project_resolution_incomplete",
            "Project-relative root must be relative",
        ));
    }
    let mut output = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => output.push(value),
            Component::ParentDir => {
                output.pop();
            }
            _ => {
                return Err(IntelligenceError::new(
                    "project_resolution_incomplete",
                    "Unsupported project root component",
                ))
            }
        }
    }
    output.canonicalize().map_err(|_| {
        IntelligenceError::new(
            "project_resolution_incomplete",
            "Referenced project root is unavailable",
        )
    })
}
