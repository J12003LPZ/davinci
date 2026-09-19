use super::LanguageAdapter;
use crate::native_extensions::security_scan::snapshot::open_confined;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub(super) const MAX_FILES: usize = 20_000;
const MAX_ENTRIES: usize = 100_000;

#[derive(Debug, Default)]
pub(super) struct Scan {
    pub sources: Vec<String>,
    pub metadata: Vec<String>,
    pub text_files: Vec<String>,
    pub warnings: Vec<String>,
    pub stamps: BTreeMap<String, FileStamp>,
    pub directories: BTreeSet<PathBuf>,
    pub ignore_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileStamp {
    pub len: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    #[cfg(unix)]
    changed: (i64, i64, u64),
}

impl FileStamp {
    fn new(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec(), metadata.ino()),
        }
    }
}

pub(super) fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

pub(super) fn validate_relative(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() || raw.contains(':') || raw.contains('\0') || raw.contains('\\') {
        return Err("invalid_path".into());
    }
    let path = Path::new(raw);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("outside_workspace: relative path required".into());
    }
    if excluded(path) {
        return Err("invalid_path: excluded path".into());
    }
    Ok(path.to_path_buf())
}

pub(super) fn excluded(path: &Path) -> bool {
    davinci_agent::is_sensitive_file_path(&path.to_string_lossy())
        || path.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some(
                    "node_modules"
                        | "target"
                        | "dist"
                        | "build"
                        | "coverage"
                        | ".next"
                        | ".cache"
                        | ".turbo"
                        | ".claude"
                        | ".codex"
                )
            )
        })
}

pub(super) fn read_bounded(root: &Path, relative: &Path, limit: usize) -> Result<String, String> {
    let file = open_confined(root, relative).map_err(|e| format!("invalid_path: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit as u64 {
        return Err("result_limit_exceeded: file bytes".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("result_limit_exceeded: file grew".into());
    }
    String::from_utf8(bytes).map_err(|_| "parse_failed: non UTF-8 file".into())
}

pub(crate) fn config_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package.json"
            | "tsconfig.json"
            | "jsconfig.json"
            | "turbo.json"
            | "nx.json"
            | "pnpm-workspace.yaml"
            | "pnpm-lock.yaml"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
    ) || [
        "vite.config.",
        "next.config.",
        "eslint.config.",
        "vitest.config.",
        "jest.config.",
        "playwright.config.",
        ".eslintrc",
        "tsconfig.",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

pub(super) fn scan(
    root: &Path,
    authorize: &impl Fn(&str) -> Result<(), String>,
    mut observe: impl FnMut(&Path),
) -> Result<Scan, String> {
    let mut scan = Scan::default();
    let mut stack = vec![(PathBuf::new(), Vec::<Gitignore>::new())];
    let mut visited = 0;
    while let Some((relative, mut rules)) = stack.pop() {
        if relative.components().count() > 48 {
            scan.warnings.push("directory depth limit".into());
            continue;
        }
        let directory = root.join(&relative);
        let metadata = fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if linked(&metadata)
            || !directory
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(root)
        {
            scan.warnings.push("linked directory excluded".into());
            continue;
        }
        let ignore_path = relative.join(".gitignore");
        observe(&directory);
        scan.directories.insert(directory.clone());
        if root.join(&ignore_path).exists() {
            authorize(&ignore_path.to_string_lossy().replace('\\', "/"))?;
            match read_bounded(root, &ignore_path, 64 * 1024) {
                Ok(body) => {
                    scan.ignore_hashes.insert(
                        ignore_path.to_string_lossy().replace('\\', "/"),
                        super::symbols::digest(body.as_bytes()),
                    );
                    let mut builder = GitignoreBuilder::new(&directory);
                    for line in body.lines() {
                        if builder
                            .add_line(Some(root.join(&ignore_path)), line)
                            .is_err()
                        {
                            scan.warnings.push("invalid gitignore rule".into());
                        }
                    }
                    if let Ok(rule) = builder.build() {
                        rules.push(rule);
                    }
                }
                Err(_) => {
                    // Do not silently scan a subtree whose ignore rules cannot be trusted.
                    scan.warnings.push(format!(
                        "unreadable ignore rules: {}",
                        ignore_path.display()
                    ));
                    continue;
                }
            }
        }
        let mut entries = fs::read_dir(&directory)
            .map_err(|e| e.to_string())?
            .take(MAX_ENTRIES.saturating_sub(visited) + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            visited += 1;
            if visited > MAX_ENTRIES || scan.text_files.len() >= MAX_FILES {
                scan.warnings.push("repository traversal/file limit".into());
                scan.warnings.truncate(32);
                return Ok(scan);
            }
            let path = relative.join(entry.file_name());
            if excluded(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if linked(&meta) {
                continue;
            }
            let mut ignored = false;
            for rule in &rules {
                let matched = rule.matched(entry.path(), meta.is_dir());
                if !matched.is_none() {
                    ignored = matched.is_ignore();
                }
            }
            if ignored {
                continue;
            }
            if meta.is_dir() {
                stack.push((path, rules.clone()));
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let name = path.to_string_lossy().replace('\\', "/");
            let supported = LanguageAdapter::for_path(&name).is_some();
            let config = config_file(&name);
            if supported {
                scan.stamps.insert(name.clone(), FileStamp::new(&meta));
                scan.sources.push(name.clone());
            }
            if config {
                scan.metadata.push(name.clone());
            }
            if supported
                || config
                || matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md" | "txt" | "json" | "yaml" | "yml" | "toml" | "html" | "css" | "rs")
                )
            {
                scan.text_files.push(name);
            }
        }
    }
    scan.sources.sort();
    scan.metadata.sort();
    scan.text_files.sort();
    scan.warnings.truncate(32);
    Ok(scan)
}
