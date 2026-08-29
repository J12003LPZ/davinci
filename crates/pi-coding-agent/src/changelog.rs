//! Changelog parser matching TS `utils/changelog.ts`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogEntry {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub content: String,
}

pub fn changelog_path() -> PathBuf {
    if let Ok(path) = std::env::var("PI_CHANGELOG_PATH") {
        return PathBuf::from(path);
    }
    PathBuf::from("vendor/pi/packages/coding-agent/CHANGELOG.md")
}

pub fn parse_changelog(path: &Path) -> Vec<ChangelogEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_changelog_text(&content)
}

pub fn parse_changelog_text(content: &str) -> Vec<ChangelogEntry> {
    let mut entries = Vec::new();
    let mut current_version: Option<(u32, u32, u32)> = None;
    let mut current_lines: Vec<String> = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some((major, minor, patch)) = current_version.take() {
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
            if let Some(version) = parse_version_header(rest) {
                current_version = Some(version);
                current_lines.push(line.to_string());
            }
        } else if current_version.is_some() {
            current_lines.push(line.to_string());
        }
    }
    if let Some((major, minor, patch)) = current_version {
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
    let mut chars = rest.trim().chars().peekable();
    if chars.peek() == Some(&'[') {
        chars.next();
    }
    let mut version = String::new();
    for ch in chars {
        if ch.is_ascii_digit() || ch == '.' {
            version.push(ch);
        } else {
            break;
        }
    }
    let mut parts = version.split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

pub fn format_changelog(entries: &[ChangelogEntry]) -> String {
    format_changelog_since(entries, None)
}

pub fn format_changelog_since(entries: &[ChangelogEntry], since: Option<&str>) -> String {
    let filtered: Vec<&ChangelogEntry> = match since.and_then(parse_version_string) {
        Some((major, minor, patch)) => entries
            .iter()
            .filter(|entry| (entry.major, entry.minor, entry.patch) > (major, minor, patch))
            .collect(),
        None => entries.iter().collect(),
    };
    if filtered.is_empty() {
        return "No changelog entries found.".into();
    }
    filtered
        .iter()
        .map(|entry| entry.content.clone())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn parse_version_string(value: &str) -> Option<(u32, u32, u32)> {
    let trimmed = value.trim().trim_start_matches('v');
    let mut parts = trimmed.split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_ts_headers() {
        let text =
            "## [0.84.4] - 2026-01-01\n\n- first\n\n## 0.83.0\n\n- older\n\n## Notes\n\nskip\n";
        let entries = parse_changelog_text(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].major, entries[0].minor, entries[0].patch),
            (0, 84, 4)
        );
        assert!(entries[0].content.contains("- first"));
        assert_eq!(
            (entries[1].major, entries[1].minor, entries[1].patch),
            (0, 83, 0)
        );
        assert!(format_changelog(&entries).contains("- older"));
        let newer = format_changelog_since(&entries, Some("0.83.0"));
        assert!(newer.contains("- first"));
        assert!(!newer.contains("- older"));
    }
}
