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
    if let Ok(path) =
        std::env::var("DAVINCI_CHANGELOG_PATH").or_else(|_| std::env::var("PI_CHANGELOG_PATH"))
    {
        return PathBuf::from(path);
    }
    let davinci_vendor = PathBuf::from("vendor/davinci/packages/coding-agent/CHANGELOG.md");
    if davinci_vendor.exists() {
        return davinci_vendor;
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

pub fn get_new_entries<'a>(
    entries: &'a [ChangelogEntry],
    last_version: &str,
) -> Vec<&'a ChangelogEntry> {
    let Some((major, minor, patch)) = parse_version_string(last_version) else {
        return entries.iter().collect();
    };
    entries
        .iter()
        .filter(|entry| (entry.major, entry.minor, entry.patch) > (major, minor, patch))
        .collect()
}

/// TS `getChangelogForDisplay` + `showStartupNoticesIfNeeded`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogDisplay {
    pub markdown: Option<String>,
    pub persist_version: Option<String>,
    pub report_telemetry: bool,
}

pub fn changelog_for_display(
    last_version: Option<&str>,
    current_version: &str,
    entries: &[ChangelogEntry],
    session_has_messages: bool,
) -> ChangelogDisplay {
    if session_has_messages {
        return ChangelogDisplay {
            markdown: None,
            persist_version: None,
            report_telemetry: false,
        };
    }
    if last_version.is_none() {
        return ChangelogDisplay {
            markdown: None,
            persist_version: Some(current_version.to_string()),
            report_telemetry: true,
        };
    }
    let new_entries = get_new_entries(entries, last_version.unwrap_or(""));
    if new_entries.is_empty() {
        return ChangelogDisplay {
            markdown: None,
            persist_version: None,
            report_telemetry: false,
        };
    }
    ChangelogDisplay {
        markdown: Some(
            new_entries
                .iter()
                .map(|entry| normalize_changelog_links(&entry.content, entry))
                .collect::<Vec<_>>()
                .join("\n\n"),
        ),
        persist_version: Some(current_version.to_string()),
        report_telemetry: true,
    }
}

pub fn format_startup_changelog(markdown: &str, collapse: bool, current_version: &str) -> String {
    if collapse {
        let latest = markdown
            .lines()
            .find_map(|line| {
                line.strip_prefix("## ").and_then(|rest| {
                    let rest = rest.trim().trim_start_matches('[');
                    parse_version_header(rest)
                        .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
                })
            })
            .unwrap_or_else(|| current_version.to_string());
        return format!("Updated to v{latest}. Use /changelog to view full changelog.");
    }
    format!("What's New\n\n{}", markdown.trim())
}

pub fn install_telemetry_url(version: &str) -> String {
    format!(
        "https://pi.dev/api/report-install?version={}",
        urlencoding_version(version)
    )
}

pub fn report_install_telemetry(version: &str, enabled: bool) {
    if std::env::var("PI_OFFLINE").is_ok() || !enabled {
        return;
    }
    if let Ok(path) = std::env::var("PI_INSTALL_TELEMETRY_REPLY") {
        let _ = std::fs::write(path, format!("version={version}"));
        return;
    }
    if cfg!(test) {
        return;
    }
    let url = install_telemetry_url(version);
    let _ = ureq::get(&url)
        .set("User-Agent", &pi_user_agent(version))
        .timeout(std::time::Duration::from_secs(5))
        .call();
}

pub fn pi_user_agent(version: &str) -> String {
    format!(
        "pi/{version} ({}; rust/{}; {})",
        std::env::consts::OS,
        rustc_version_meta(),
        std::env::consts::ARCH
    )
}

fn rustc_version_meta() -> &'static str {
    option_env!("CARGO_PKG_RUST_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn urlencoding_version(version: &str) -> String {
    let mut out = String::new();
    for ch in version.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => out.push_str(&format!("%{:02X}", ch as u32)),
        }
    }
    out
}

const GITHUB_REPO: &str = "earendil-works/pi";
const CHANGELOG_LINK_BASE: &str = "packages/coding-agent";

pub fn normalize_changelog_links(markdown: &str, entry: &ChangelogEntry) -> String {
    let tag = format!("v{}.{}.{}", entry.major, entry.minor, entry.patch);
    let mut out = String::new();
    let mut rest = markdown;
    while let Some(start) = rest.find("](") {
        let prefix_end = start + 2;
        out.push_str(&rest[..prefix_end]);
        let after = &rest[prefix_end..];
        let Some(end) = after.find(')') else {
            out.push_str(after);
            rest = "";
            break;
        };
        let target = &after[..end];
        let (url, suffix_start) = target
            .split_once(' ')
            .map(|(url, extra)| (url, extra.len() + 1))
            .unwrap_or((target, 0));
        out.push_str(&normalize_changelog_link_target(url, &tag));
        if suffix_start > 0 {
            out.push(' ');
            out.push_str(&target[url.len() + 1..]);
        }
        out.push(')');
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn normalize_changelog_link_target(target: &str, tag: &str) -> String {
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
    let Some(repository_path) = resolve_repository_path(path_part) else {
        return canonical;
    };
    let route = if path_part.ends_with('/')
        || !Path::new(&repository_path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains('.'))
    {
        "tree"
    } else {
        "blob"
    };
    format!("{repo_url}/{route}/{tag}/{repository_path}{query}{fragment}")
}

fn split_local_target(target: &str) -> (&str, &str, &str) {
    let (before_hash, fragment) = target
        .split_once('#')
        .map(|(path, _frag)| (path, &target[path.len()..]))
        .unwrap_or((target, ""));
    let (path_part, query) = before_hash
        .split_once('?')
        .map(|(path, _)| (path, &before_hash[path.len()..]))
        .unwrap_or((before_hash, ""));
    (path_part, query, fragment)
}

fn resolve_repository_path(target_path: &str) -> Option<String> {
    let normalized = target_path.replace('\\', "/");
    let joined = if normalized.starts_with('/') {
        normalized.trim_start_matches('/').to_string()
    } else {
        format!("{CHANGELOG_LINK_BASE}/{normalized}")
    };
    let mut parts = Vec::new();
    for part in joined.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop()?;
            continue;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
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
        let display = changelog_for_display(None, "0.84.4", &entries, false);
        assert!(display.markdown.is_none());
        assert_eq!(display.persist_version.as_deref(), Some("0.84.4"));
        assert!(display.report_telemetry);
        let resumed = changelog_for_display(Some("0.83.0"), "0.84.4", &entries, true);
        assert!(resumed.markdown.is_none());
        let updated = changelog_for_display(Some("0.83.0"), "0.84.4", &entries, false);
        assert!(updated.markdown.as_ref().unwrap().contains("- first"));
        assert_eq!(
            format_startup_changelog(updated.markdown.as_deref().unwrap(), true, "0.84.4"),
            "Updated to v0.84.4. Use /changelog to view full changelog."
        );
        assert!(
            format_startup_changelog(updated.markdown.as_deref().unwrap(), false, "0.84.4")
                .starts_with("What's New")
        );
        assert!(normalize_changelog_links("[docs](README.md)", &entries[0])
            .contains("github.com/earendil-works/pi/blob/v0.84.4/packages/coding-agent/README.md"));
        assert_eq!(
            install_telemetry_url("0.84.4"),
            "https://pi.dev/api/report-install?version=0.84.4"
        );
    }
}
