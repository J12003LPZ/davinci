//! Deterministic, source-level design fingerprinting for frontend evals.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_SOURCE_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DOMINANT_COLORS: usize = 8;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DesignFingerprint {
    pub font_families: Vec<String>,
    pub dominant_colors: Vec<String>,
    pub border_radius_buckets: Vec<u16>,
    pub uses_gradient: bool,
    pub repeated_card_ratio: f64,
    pub all_caps_label_count: u32,
    pub numbered_section_count: u32,
}

/// Fingerprint frontend source files beneath `root` without executing or
/// modifying the project. Files outside the canonical root and common build
/// output directories are ignored.
pub fn fingerprint_frontend_source(root: &Path) -> Result<DesignFingerprint, String> {
    let root = root.canonicalize().map_err(|error| {
        format!(
            "failed to resolve frontend root {}: {error}",
            root.display()
        )
    })?;
    if !root.is_dir() {
        return Err(format!(
            "frontend root is not a directory: {}",
            root.display()
        ));
    }

    let mut files = Vec::new();
    let mut visited_dirs = HashSet::new();
    collect_frontend_files(&root, &root, &mut visited_dirs, &mut files)?;
    files.sort();

    if files.is_empty() {
        return Err(format!(
            "no frontend source files found under {}",
            root.display()
        ));
    }

    let mut source = String::new();
    for path in files {
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.len() > MAX_SOURCE_FILE_BYTES {
            return Err(format!(
                "frontend source file exceeds {} bytes: {}",
                MAX_SOURCE_FILE_BYTES,
                path.display()
            ));
        }
        let contents = fs::read_to_string(&path).map_err(|error| {
            format!("failed to read frontend source {}: {error}", path.display())
        })?;
        source.push_str(&contents);
        source.push('\n');
    }

    let source = strip_comments(&source);
    Ok(DesignFingerprint {
        font_families: extract_font_families(&source),
        dominant_colors: extract_dominant_colors(&source),
        border_radius_buckets: extract_radius_buckets(&source),
        uses_gradient: source.to_ascii_lowercase().contains("gradient("),
        repeated_card_ratio: extract_repeated_card_ratio(&source),
        all_caps_label_count: extract_all_caps_label_count(&source),
        numbered_section_count: extract_numbered_section_count(&source),
    })
}

fn collect_frontend_files(
    directory: &Path,
    root: &Path,
    visited_dirs: &mut HashSet<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let canonical_directory = directory
        .canonicalize()
        .map_err(|error| format!("failed to inspect {}: {error}", directory.display()))?;
    if !canonical_directory.starts_with(root) || !visited_dirs.insert(canonical_directory.clone()) {
        return Ok(());
    }

    let mut entries = fs::read_dir(&canonical_directory)
        .map_err(|error| format!("failed to list {}: {error}", canonical_directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read directory entry: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let canonical_path = match path.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !canonical_path.starts_with(root) {
            continue;
        }
        if canonical_path.is_dir() {
            if !is_ignored_directory(&canonical_path) {
                collect_frontend_files(&canonical_path, root, visited_dirs, files)?;
            }
        } else if is_frontend_file(&canonical_path) {
            files.push(canonical_path);
        }
    }
    Ok(())
}

fn is_ignored_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            matches!(
                name,
                ".git" | ".next" | ".nuxt" | ".pi" | "build" | "dist" | "node_modules" | "target"
            )
        })
        .unwrap_or(false)
}

fn is_frontend_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "css"
                    | "htm"
                    | "html"
                    | "jsx"
                    | "less"
                    | "sass"
                    | "scss"
                    | "svelte"
                    | "tsx"
                    | "vue"
            )
        })
        .unwrap_or(false)
}

fn strip_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        let remainder = &source[cursor..];
        let css_start = remainder.find("/*");
        let html_start = remainder.find("<!--");
        let next = match (css_start, html_start) {
            (Some(css), Some(html)) if css <= html => Some((css, 2, "*/")),
            (Some(_css), Some(html)) => Some((html, 4, "-->")),
            (Some(css), None) => Some((css, 2, "*/")),
            (None, Some(html)) => Some((html, 4, "-->")),
            (None, None) => None,
        };
        let Some((offset, marker_len, end_marker)) = next else {
            result.push_str(remainder);
            break;
        };
        result.push_str(&remainder[..offset]);
        let comment_start = cursor + offset;
        let comment_body_start = comment_start + marker_len;
        let Some(end_offset) = source[comment_body_start..].find(end_marker) else {
            break;
        };
        cursor = comment_body_start + end_offset + end_marker.len();
        result.push(' ');
    }
    result
}

fn extract_font_families(source: &str) -> Vec<String> {
    let lower = source.to_ascii_lowercase();
    let mut families = Vec::new();
    for property in ["font-family", "fontfamily"] {
        let mut cursor = 0;
        while let Some(offset) = lower[cursor..].find(property) {
            let start = cursor + offset;
            let end = start + property.len();
            if !is_property_boundary(&lower, start, end) {
                cursor = end;
                continue;
            }
            let Some(value) = property_value(source, end) else {
                cursor = end;
                continue;
            };
            for family in value.split(',') {
                let family = family
                    .trim()
                    .trim_matches(['\'', '"', '`'])
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_ascii_lowercase();
                if !family.is_empty()
                    && !matches!(family.as_str(), "inherit" | "initial" | "revert" | "unset")
                    && !families.contains(&family)
                {
                    families.push(family);
                }
            }
            cursor = end;
        }
    }
    families
}

fn is_property_boundary(source: &str, start: usize, end: usize) -> bool {
    let before_ok = source[..start]
        .chars()
        .next_back()
        .map(|character| !character.is_ascii_alphanumeric() && character != '-')
        .unwrap_or(true);
    let after_ok = source[end..]
        .chars()
        .next()
        .map(|character| !character.is_ascii_alphanumeric() && character != '-')
        .unwrap_or(true);
    before_ok && after_ok
}

fn property_value(source: &str, property_end: usize) -> Option<&str> {
    let remainder = &source[property_end..];
    let delimiter = remainder.find([':', '='])?;
    let value_start = delimiter + 1;
    let value_end = remainder[value_start..]
        .find([';', '}', '\n', '<'])
        .map(|offset| value_start + offset)
        .unwrap_or(remainder.len());
    Some(remainder[value_start..value_end].trim())
}

fn extract_dominant_colors(source: &str) -> Vec<String> {
    let mut counts = BTreeMap::new();
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'#' {
            let start = cursor + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            if matches!(end - start, 3 | 4 | 6 | 8) {
                if let Some(color) = normalize_hex(&source[start..end]) {
                    *counts.entry(color).or_insert(0_u32) += 1;
                }
            }
            cursor = end;
        } else {
            cursor += 1;
        }
    }

    let lower = source.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find("rgb(") {
        let start = cursor + offset + 4;
        let Some(end_offset) = source[start..].find(')') else {
            break;
        };
        if let Some(color) = normalize_rgb(&source[start..start + end_offset]) {
            *counts.entry(color).or_insert(0_u32) += 1;
        }
        cursor = start + end_offset + 1;
    }

    let mut colors = counts.into_iter().collect::<Vec<_>>();
    colors.sort_by(|(left_color, left_count), (right_color, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_color.cmp(right_color))
    });
    colors
        .into_iter()
        .take(MAX_DOMINANT_COLORS)
        .map(|(color, _)| color)
        .collect()
}

fn normalize_hex(value: &str) -> Option<String> {
    let value = value.to_ascii_lowercase();
    match value.len() {
        3 => Some(format!(
            "#{}{}{}{}{}{}",
            &value[0..1],
            &value[0..1],
            &value[1..2],
            &value[1..2],
            &value[2..3],
            &value[2..3]
        )),
        4 => Some(format!(
            "#{}{}{}{}{}{}{}{}",
            &value[0..1],
            &value[0..1],
            &value[1..2],
            &value[1..2],
            &value[2..3],
            &value[2..3],
            &value[3..4],
            &value[3..4]
        )),
        6 | 8 => Some(format!("#{value}")),
        _ => None,
    }
}

fn normalize_rgb(value: &str) -> Option<String> {
    let components = value.split(',').take(3).collect::<Vec<_>>();
    if components.len() != 3 {
        return None;
    }
    let mut channels = [0_u8; 3];
    for (index, component) in components.into_iter().enumerate() {
        let component = component.trim();
        let channel = if let Some(percent) = component.strip_suffix('%') {
            percent.parse::<f64>().ok()?.clamp(0.0, 100.0) * 2.55
        } else {
            component.parse::<f64>().ok()?.clamp(0.0, 255.0)
        };
        channels[index] = channel.round() as u8;
    }
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channels[0], channels[1], channels[2]
    ))
}

fn extract_radius_buckets(source: &str) -> Vec<u16> {
    let lower = source.to_ascii_lowercase();
    let mut buckets = Vec::new();
    for property in ["border-radius", "borderradius"] {
        let mut cursor = 0;
        while let Some(offset) = lower[cursor..].find(property) {
            let start = cursor + offset;
            let end = start + property.len();
            if !is_property_boundary(&lower, start, end) {
                cursor = end;
                continue;
            }
            if let Some(value) = property_value(source, end) {
                for (number, unit) in dimensions(value) {
                    let bucket = radius_bucket(number, unit);
                    if !buckets.contains(&bucket) {
                        buckets.push(bucket);
                    }
                }
            }
            cursor = end;
        }
    }
    buckets
}

fn dimensions(value: &str) -> Vec<(f64, &str)> {
    let mut result = Vec::new();
    let bytes = value.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let is_number_start = bytes[cursor].is_ascii_digit()
            || (bytes[cursor] == b'.' && bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit));
        if !is_number_start {
            cursor += 1;
            continue;
        }
        let number_start = cursor;
        while cursor < bytes.len() && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'.') {
            cursor += 1;
        }
        let number = match value[number_start..cursor].parse::<f64>() {
            Ok(number) => number,
            Err(_) => continue,
        };
        let unit_start = cursor;
        while cursor < bytes.len() && (bytes[cursor].is_ascii_alphabetic() || bytes[cursor] == b'%')
        {
            cursor += 1;
        }
        result.push((number, &value[unit_start..cursor]));
    }
    result
}

fn radius_bucket(number: f64, unit: &str) -> u16 {
    if unit == "%" {
        return if number > 50.0 { 100 } else { 50 };
    }
    let pixels = match unit {
        "rem" | "em" => number * 16.0,
        "vw" => number * 12.0,
        _ => number,
    };
    if pixels <= 0.0 {
        0
    } else if pixels <= 4.0 {
        4
    } else if pixels <= 8.0 {
        8
    } else if pixels <= 16.0 {
        16
    } else if pixels <= 24.0 {
        24
    } else if pixels <= 32.0 {
        32
    } else if pixels <= 48.0 {
        48
    } else {
        64
    }
}

fn extract_repeated_card_ratio(source: &str) -> f64 {
    let mut names = extract_card_names_from_attributes(source);
    if names.is_empty() {
        names = extract_card_names_from_selectors(source);
    }
    if names.is_empty() {
        return 0.0;
    }

    let mut counts = BTreeMap::new();
    for name in names {
        *counts.entry(name).or_insert(0_usize) += 1;
    }
    let total = counts.values().sum::<usize>();
    let unique = counts.len();
    (total.saturating_sub(unique) as f64 / total as f64).clamp(0.0, 1.0)
}

fn extract_card_names_from_attributes(source: &str) -> Vec<String> {
    let lower = source.to_ascii_lowercase();
    let mut names = Vec::new();
    for attribute in ["classname", "class"] {
        let mut cursor = 0;
        while let Some(offset) = lower[cursor..].find(attribute) {
            let start = cursor + offset;
            let end = start + attribute.len();
            if !is_property_boundary(&lower, start, end) {
                cursor = end;
                continue;
            }
            let remainder = &source[end..];
            let Some(delimiter) = remainder.find(['=', ':']) else {
                cursor = end;
                continue;
            };
            let mut value_start = end + delimiter + 1;
            while source[value_start..]
                .chars()
                .next()
                .is_some_and(|character| character.is_whitespace())
            {
                value_start += source[value_start..].chars().next().unwrap().len_utf8();
            }
            let (value, value_end) = if let Some(quote) = source[value_start..].chars().next() {
                if matches!(quote, '\'' | '"' | '`') {
                    let content_start = value_start + quote.len_utf8();
                    let end_offset = source[content_start..].find(quote).unwrap_or(0);
                    (
                        &source[content_start..content_start + end_offset],
                        content_start + end_offset + quote.len_utf8(),
                    )
                } else {
                    let end_offset = source[value_start..]
                        .find(char::is_whitespace)
                        .unwrap_or(source.len() - value_start);
                    (
                        &source[value_start..value_start + end_offset],
                        value_start + end_offset,
                    )
                }
            } else {
                ("", value_start)
            };
            names.extend(value.split_whitespace().filter_map(normalize_card_name));
            cursor = value_end.max(end);
        }
    }
    names
}

fn extract_card_names_from_selectors(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = source.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'.' && *byte != b'#' {
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_'))
        {
            end += 1;
        }
        if let Some(name) = source.get(start..end).and_then(normalize_card_name) {
            names.push(name);
        }
    }
    names
}

fn normalize_card_name(name: &str) -> Option<String> {
    let name =
        name.trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-');
    let lower = name.to_ascii_lowercase();
    let motifs = ["card", "feature", "panel", "product", "tile", "testimonial"];
    motifs
        .iter()
        .any(|motif| lower.split(['-', '_']).any(|part| part == *motif))
        .then_some(lower)
}

fn extract_markup_text(source: &str) -> Vec<&str> {
    let lower = source.to_ascii_lowercase();
    let mut fragments = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'<' {
            if lower[cursor..].starts_with("<script") || lower[cursor..].starts_with("<style") {
                let closing = if lower[cursor..].starts_with("<script") {
                    "</script"
                } else {
                    "</style"
                };
                cursor += lower[cursor..]
                    .find(closing)
                    .unwrap_or(source.len() - cursor);
            }
            if let Some(end_offset) = source[cursor..].find('>') {
                cursor += end_offset + 1;
            } else {
                break;
            }
        } else if let Some(end_offset) = source[cursor..].find('<') {
            if !source[cursor..cursor + end_offset].trim().is_empty() {
                fragments.push(&source[cursor..cursor + end_offset]);
            }
            cursor += end_offset;
        } else {
            if !source[cursor..].trim().is_empty() {
                fragments.push(&source[cursor..]);
            }
            break;
        }
    }
    fragments
}

fn extract_all_caps_label_count(source: &str) -> u32 {
    extract_markup_text(source)
        .into_iter()
        .filter(|fragment| !is_numbered_section(fragment) && is_all_caps_label(fragment))
        .count() as u32
}

fn extract_numbered_section_count(source: &str) -> u32 {
    extract_markup_text(source)
        .into_iter()
        .filter(|fragment| is_numbered_section(fragment))
        .count() as u32
}

fn is_numbered_section(fragment: &str) -> bool {
    let fragment = fragment.trim();
    let digit_count = fragment.chars().take_while(char::is_ascii_digit).count();
    if digit_count == 0 {
        return false;
    }
    let remainder = fragment[digit_count..].trim_start();
    matches!(remainder.chars().next(), Some('.' | ')' | ':'))
        && remainder
            .chars()
            .any(|character| character.is_ascii_alphabetic())
}

fn is_all_caps_label(fragment: &str) -> bool {
    let mut letters = 0;
    for character in fragment.chars() {
        if character.is_ascii_alphabetic() {
            letters += 1;
            if character.is_ascii_lowercase() {
                return false;
            }
        }
    }
    letters >= 2
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn write_fixture(source: &str) -> tempfile::TempDir {
        let dir = tempdir().expect("temporary fixture directory");
        fs::write(dir.path().join("index.html"), source).expect("fixture source");
        dir
    }

    #[test]
    fn extracts_design_motifs_from_html_and_css() {
        let fixture = write_fixture(
            r#"
            <html><head><style>
              :root { --ink: #101820; --accent: rgb(255, 107, 53); }
              body { font-family: 'Space Grotesk', sans-serif; color: #101820; }
              .card { border-radius: 0.75rem; background: linear-gradient(90deg, #ff6b35, #101820); }
            </style></head>
            <body>
              <main>
                <h1>MAKE IT MATTER</h1>
                <h2>01. INTRODUCTION</h2>
                <article class="card">ONE</article>
                <article class="card">TWO</article>
                <article class="card">THREE</article>
              </main>
            </body></html>
            "#,
        );

        let fingerprint = fingerprint_frontend_source(fixture.path()).expect("fingerprint");

        assert_eq!(fingerprint.font_families, ["space grotesk", "sans-serif"]);
        assert_eq!(fingerprint.dominant_colors[0], "#101820");
        assert!(fingerprint.dominant_colors.contains(&"#ff6b35".to_string()));
        assert_eq!(fingerprint.border_radius_buckets, [16]);
        assert!(fingerprint.uses_gradient);
        assert!((fingerprint.repeated_card_ratio - (2.0 / 3.0)).abs() < f64::EPSILON);
        assert_eq!(fingerprint.all_caps_label_count, 4);
        assert_eq!(fingerprint.numbered_section_count, 1);
    }

    #[test]
    fn rejects_missing_or_non_frontend_roots() {
        let missing = std::path::Path::new("this-root-does-not-exist");
        assert!(fingerprint_frontend_source(missing).is_err());

        let dir = tempdir().expect("temporary fixture directory");
        fs::write(dir.path().join("notes.txt"), "not frontend source").expect("fixture source");
        let error = fingerprint_frontend_source(dir.path()).expect_err("no source should fail");
        assert!(error.contains("frontend source"));
    }
}
