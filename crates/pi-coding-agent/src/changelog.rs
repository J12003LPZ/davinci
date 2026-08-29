//! Changelog parse/normalize matching TypeScript `utils/changelog.ts`.

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

const GITHUB_REPO: &str = "earendil-works/pi";
const CHANGELOG_LINK_BASE_PATH: &str = "packages/coding-agent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogEntry {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub content: String,
}

impl ChangelogEntry {
    pub fn version(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub fn changelog_path() -> PathBuf {
    let candidates = [
        PathBuf::from("vendor/pi/packages/coding-agent/CHANGELOG.md"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/pi/packages/coding-agent/CHANGELOG.md"),
    ];
    for path in candidates {
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("CHANGELOG.md")
}

pub fn parse_changelog(path: &Path) -> Vec<ChangelogEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut current: Option<(u32, u32, u32)> = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some((major, minor, patch)) = current.take() {
                if !current_lines.is_empty() {
                    entries.push(ChangelogEntry {
                        major,
                        minor,
                        patch,
                        content: current_lines.join("\n").trim().to_string(),
                    });
                }
            }
            current_lines.clear();
            if let Some((major, minor, patch)) = parse_version_header(rest) {
                current = Some((major, minor, patch));
                current_lines.push(line.to_string());
            }
        } else if current.is_some() {
            current_lines.push(line.to_string());
        }
    }
    if let Some((major, minor, patch)) = current {
        if !current_lines.is_empty() {
            entries.push(ChangelogEntry {
                major,
                minor,
                patch,
                content: current_lines.join("\n").trim().to_string(),
            });
        }
    }
    entries
}

fn parse_version_header(rest: &str) -> Option<(u32, u32, u32)> {
    let rest = rest.trim().trim_start_matches('[');
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub fn normalize_changelog_links(markdown: &str, version: &str) -> String {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let re = Regex::new(r"(!?\[[^\]\n]+\]\()([^\s)]+)((?:\s+[^)]*)?\))").unwrap();
    re.replace_all(markdown, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let target = &caps[2];
        let suffix = &caps[3];
        format!("{prefix}{}{suffix}", normalize_link_target(target, &tag))
    })
    .into_owned()
}

fn normalize_link_target(target: &str, tag: &str) -> String {
    let mut canonical = target.replace(
        "https://github.com/badlogic/pi-mono",
        &format!("https://github.com/{GITHUB_REPO}"),
    );
    canonical = canonical.replace(
        "https://github.com/earendil-works/pi-mono",
        &format!("https://github.com/{GITHUB_REPO}"),
    );
    let repo_url = format!("https://github.com/{GITHUB_REPO}");
    for route in ["blob", "tree"] {
        for branch in ["main", "master"] {
            let prefix = format!("{repo_url}/{route}/{branch}/");
            if let Some(rest) = canonical.strip_prefix(&prefix) {
                canonical = format!("{repo_url}/{route}/{tag}/{rest}");
            }
        }
    }
    if canonical.starts_with('#') || canonical.starts_with("//") || canonical.contains("://") {
        return canonical;
    }
    let (path_part, query, fragment) = split_local_target(&canonical);
    if path_part.is_empty() {
        return canonical;
    }
    let Some(repository_path) = resolve_repository_path(&path_part) else {
        return canonical;
    };
    let route = if path_part.ends_with('/')
        || !Path::new(&repository_path)
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.contains('.'))
    {
        "tree"
    } else {
        "blob"
    };
    format!(
        "{repo_url}/{route}/{tag}/{}{query}{fragment}",
        repository_path
    )
}

fn split_local_target(target: &str) -> (String, String, String) {
    let (before_hash, fragment) = match target.split_once('#') {
        Some((left, right)) => (left, format!("#{right}")),
        None => (target, String::new()),
    };
    match before_hash.split_once('?') {
        Some((path, query)) => (path.to_string(), format!("?{query}"), fragment),
        None => (before_hash.to_string(), String::new(), fragment),
    }
}

fn resolve_repository_path(target_path: &str) -> Option<String> {
    let normalized = target_path.replace('\\', "/");
    let joined = if normalized.starts_with('/') {
        normalized.trim_start_matches('/').to_string()
    } else {
        format!("{CHANGELOG_LINK_BASE_PATH}/{normalized}")
    };
    if joined == "." || joined == ".." || joined.starts_with("../") {
        return None;
    }
    Some(joined)
}

pub fn render_changelog(path: &Path) -> String {
    let entries = parse_changelog(path);
    if entries.is_empty() {
        return "No changelog entries found.".into();
    }
    entries
        .into_iter()
        .rev()
        .map(|entry| {
            let version = entry.version();
            normalize_changelog_links(&entry.content, &version)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parses_version_headers_like_typescript() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "# Changelog\n\n## [Unreleased]\n\n## [0.84.4] - 2026-08-28\n\n- RPC `clear_queue`"
        )
        .unwrap();
        let entries = parse_changelog(file.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version(), "0.84.4");
        assert!(entries[0].content.contains("clear_queue"));
    }

    #[test]
    fn normalizes_relative_doc_links() {
        let out = normalize_changelog_links("[docs](docs/rpc.md#clear_queue)", "0.84.4");
        assert_eq!(
            out,
            "[docs](https://github.com/earendil-works/pi/blob/v0.84.4/packages/coding-agent/docs/rpc.md#clear_queue)"
        );
    }

    #[test]
    fn empty_file_uses_ts_empty_string() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# Changelog").unwrap();
        assert_eq!(render_changelog(file.path()), "No changelog entries found.");
    }
}
