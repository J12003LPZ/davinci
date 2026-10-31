use super::model::PackageSymbolResult;
use std::fs;
use std::io::Read as _;
use std::path::Path;

pub fn find_symbol_in_types(
    types_path: &Path,
    repo_rel_path: &str,
    package_name: &str,
    installed_version: Option<&str>,
    symbol: &str,
) -> Result<PackageSymbolResult, String> {
    let mut warnings = Vec::new();
    if symbol.trim().is_empty() {
        return Err("symbol must not be empty".into());
    }

    let meta = fs::metadata(types_path).map_err(|e| format!("cannot read types file: {e}"))?;
    let truncated = meta.len() > 512 * 1024;
    if truncated {
        warnings.push("types file exceeds 512 KiB limit; truncated scan".to_string());
    }

    let mut file =
        fs::File::open(types_path).map_err(|e| format!("failed to read types file: {e}"))?;
    let mut bytes = Vec::new();
    std::io::Read::take(&mut file, 512 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read types file: {e}"))?;
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) if truncated => {
            let valid = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid].to_vec())
                .map_err(|_| "types file is not valid UTF-8".to_string())?
        }
        Err(_) => return Err("types file is not valid UTF-8".into()),
    };
    let lines: Vec<&str> = content.lines().collect();

    // Check for qualified symbol e.g. "z.object" -> ["z", "object"]
    let parts: Vec<&str> = symbol
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let target_name = parts.last().copied().unwrap_or(symbol);

    let mut found_line = None;
    let mut found_kind = None;

    if parts.len() > 1 {
        let parent = parts[0];
        let child = parts[1];
        // Look for parent namespace/object/interface first
        let mut in_parent_scope = false;
        let mut brace_depth = 0;

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if !in_parent_scope {
                if trimmed.contains(&format!("namespace {parent}"))
                    || trimmed.contains(&format!("interface {parent}"))
                    || trimmed.contains(&format!("const {parent}:"))
                    || trimmed.contains(&format!("class {parent}"))
                {
                    in_parent_scope = true;
                    brace_depth += line.chars().filter(|c| *c == '{').count();
                    brace_depth =
                        brace_depth.saturating_sub(line.chars().filter(|c| *c == '}').count());
                }
            } else {
                brace_depth += line.chars().filter(|c| *c == '{').count();
                brace_depth =
                    brace_depth.saturating_sub(line.chars().filter(|c| *c == '}').count());

                if matches_symbol_declaration(trimmed, child) {
                    found_line = Some(idx);
                    found_kind = Some(infer_kind(trimmed, child));
                    break;
                }

                if brace_depth == 0 {
                    in_parent_scope = false;
                }
            }
        }
    }

    // If not found in scope or single symbol, scan lines
    if found_line.is_none() {
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if matches_symbol_declaration(trimmed, target_name) {
                found_line = Some(idx);
                found_kind = Some(infer_kind(trimmed, target_name));
                break;
            }
        }
    }

    if let Some(line_idx) = found_line {
        // Collect declaration snippet (up to 20 lines)
        let end_idx = (line_idx + 20).min(lines.len());
        let mut snippet_lines = Vec::new();
        let mut depth = 0;
        for line in &lines[line_idx..end_idx] {
            snippet_lines.push(*line);
            depth += line.chars().filter(|c| *c == '{').count();
            let close = line.chars().filter(|c| *c == '}').count();
            depth = depth.saturating_sub(close);
            if depth == 0 && (line.trim().ends_with(';') || close > 0) {
                break;
            }
        }
        let declaration = snippet_lines.join("\n");

        // Collect doc comments directly preceding line_idx
        let mut doc_lines = Vec::new();
        let mut cur = line_idx;
        while cur > 0 {
            cur -= 1;
            let prev = lines[cur].trim();
            if prev.starts_with("/**") || prev.starts_with('*') || prev.starts_with("//") {
                doc_lines.push(prev);
                if prev.starts_with("/**") {
                    break;
                }
            } else {
                break;
            }
        }
        doc_lines.reverse();
        let documentation = if !doc_lines.is_empty() {
            Some(doc_lines.join("\n"))
        } else {
            None
        };

        Ok(PackageSymbolResult {
            package_name: package_name.to_string(),
            installed_version: installed_version.map(str::to_string),
            symbol: symbol.to_string(),
            found: true,
            file_path: Some(repo_rel_path.to_string()),
            line: Some(line_idx + 1),
            declaration: Some(declaration),
            kind: found_kind,
            documentation,
            warnings,
        })
    } else {
        Ok(PackageSymbolResult {
            package_name: package_name.to_string(),
            installed_version: installed_version.map(str::to_string),
            symbol: symbol.to_string(),
            found: false,
            file_path: Some(repo_rel_path.to_string()),
            line: None,
            declaration: None,
            kind: None,
            documentation: None,
            warnings,
        })
    }
}

fn matches_symbol_declaration(line: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let trimmed = line.trim_start();
    if let Some(declared) = declared_identifier(trimmed) {
        return declared == name;
    }
    // No declaration keyword: accept a method/property form only when the
    // requested identifier is the first token, not a reference later on.
    let mut rest = trimmed;
    for modifier in [
        "public",
        "private",
        "protected",
        "static",
        "readonly",
        "override",
        "async",
        "get",
        "set",
    ] {
        if let Some(after) = strip_word(rest, modifier) {
            rest = after.trim_start();
        }
    }
    let Some(after) = rest.strip_prefix(name) else {
        return false;
    };
    let boundary = after
        .chars()
        .next()
        .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == '$'));
    boundary
        && (after.starts_with('(')
            || after.starts_with('<')
            || after.starts_with(':')
            || after.starts_with(" =")
            || after.trim_start().starts_with('='))
}

fn declared_identifier(line: &str) -> Option<&str> {
    let mut tokens = line
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
        .filter(|token| !token.is_empty());
    while let Some(token) = tokens.next() {
        if matches!(
            token,
            "function"
                | "class"
                | "interface"
                | "type"
                | "const"
                | "let"
                | "var"
                | "namespace"
                | "module"
                | "enum"
        ) {
            for next in tokens.by_ref() {
                if matches!(next, "async" | "default" | "abstract") {
                    continue;
                }
                return Some(next);
            }
            return None;
        }
    }
    None
}

fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(word)?;
    rest.chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then_some(rest)
}

fn infer_kind(line: &str, name: &str) -> String {
    if line.contains(&format!("function {name}")) || line.contains(&format!("{name}(")) {
        "function".to_string()
    } else if line.contains(&format!("interface {name}")) {
        "interface".to_string()
    } else if line.contains(&format!("type {name}")) {
        "type".to_string()
    } else if line.contains(&format!("class {name}")) {
        "class".to_string()
    } else if line.contains(&format!("namespace {name}")) {
        "namespace".to_string()
    } else if line.contains(&format!("enum {name}")) {
        "enum".to_string()
    } else if line.contains(&format!("const {name}")) {
        "const".to_string()
    } else {
        "property".to_string()
    }
}

#[cfg(test)]
mod audit_review_tests {
    use super::*;

    #[test]
    fn audit_review_empty_symbol_is_rejected_without_looping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.d.ts");
        std::fs::write(&path, "export class Widget {}\n").unwrap();
        let error = find_symbol_in_types(&path, "index.d.ts", "pkg", None, "").unwrap_err();
        assert!(error.contains("must not be empty"), "{error}");
        assert!(!matches_symbol_declaration("class Widget {}", ""));
    }

    #[test]
    fn audit_review_declaration_match_uses_the_declared_identifier() {
        assert!(matches_symbol_declaration(
            "export class Widget {}",
            "Widget"
        ));
        assert!(matches_symbol_declaration(
            "export function MakeWidget() {}",
            "MakeWidget"
        ));
        assert!(!matches_symbol_declaration(
            "export function MakeWidget() {}",
            "Widget"
        ));
        assert!(!matches_symbol_declaration(
            "export type Response = UserProfile;",
            "UserProfile"
        ));
        assert!(matches_symbol_declaration(
            "export type Response = UserProfile;",
            "Response"
        ));
        assert!(!matches_symbol_declaration(
            "const name: UserProfile = value;",
            "UserProfile"
        ));
        assert!(matches_symbol_declaration(
            "const UserProfile = value;",
            "UserProfile"
        ));
        assert!(matches_symbol_declaration("UserProfile(", "UserProfile"));
        assert!(!matches_symbol_declaration(
            "other UserProfile(",
            "UserProfile"
        ));
    }
}
