//! Combined slash + path autocomplete matching
//! `vendor/pi/packages/tui/src/autocomplete.ts`.

use std::path::{Path, PathBuf};

use crate::fuzzy::fuzzy_filter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteSuggestions {
    pub items: Vec<AutocompleteItem>,
    pub prefix: String,
}

#[derive(Debug, Clone)]
pub struct SlashCommandSpec {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
}

pub fn apply_completion(
    buffer: &str,
    cursor: usize,
    prefix: &str,
    item: &AutocompleteItem,
) -> String {
    let cursor = cursor.min(buffer.len());
    let prefix_start = cursor.saturating_sub(prefix.len());
    let before = &buffer[..prefix_start];
    let after = &buffer[cursor..];
    let is_slash =
        prefix.starts_with('/') && before.trim().is_empty() && !prefix[1..].contains('/');
    if is_slash {
        return format!("{before}/{} {after}", item.value);
    }
    if prefix.starts_with('@') {
        let suffix = if item.label.ends_with('/') { "" } else { " " };
        return format!("{before}{}{suffix}{after}", item.value);
    }
    format!("{before}{}{after}", item.value)
}

pub fn suggestions(
    text: &str,
    commands: &[SlashCommandSpec],
    models: &[String],
    thinking_levels: &[String],
    login_providers: &[String],
    cwd: &Path,
    force_path: bool,
) -> Option<AutocompleteSuggestions> {
    if let Some(at) = extract_at_prefix(text) {
        let raw = parse_path_raw(&at);
        let items = file_suggestions(cwd, &raw, at.starts_with('@'));
        if items.is_empty() {
            return None;
        }
        return Some(AutocompleteSuggestions { items, prefix: at });
    }
    if let Some(rest) = text.strip_prefix('/') {
        if let Some((name, args)) = rest.split_once(' ') {
            return argument_suggestions(name, args, models, thinking_levels, login_providers);
        }
        let filtered = fuzzy_filter(
            rest,
            &commands.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        );
        if filtered.is_empty() {
            return None;
        }
        let items = filtered
            .into_iter()
            .filter_map(|name| commands.iter().find(|c| c.name == name))
            .map(|command| {
                let description = match &command.argument_hint {
                    Some(hint) if !command.description.is_empty() => {
                        Some(format!("{} — {}", hint, command.description))
                    }
                    Some(hint) => Some(hint.clone()),
                    None if command.description.is_empty() => None,
                    None => Some(command.description.clone()),
                };
                AutocompleteItem {
                    value: command.name.clone(),
                    label: command.name.clone(),
                    description,
                }
            })
            .collect();
        return Some(AutocompleteSuggestions {
            items,
            prefix: text.to_string(),
        });
    }
    if force_path {
        if let Some(prefix) = extract_path_prefix(text, true) {
            let raw = parse_path_raw(&prefix);
            let items = file_suggestions(cwd, &raw, prefix.starts_with('@'));
            if !items.is_empty() {
                return Some(AutocompleteSuggestions { items, prefix });
            }
        }
    }
    None
}

fn argument_suggestions(
    command: &str,
    args: &str,
    models: &[String],
    thinking_levels: &[String],
    login_providers: &[String],
) -> Option<AutocompleteSuggestions> {
    let pool = match command {
        "model" => models,
        "thinking" => thinking_levels,
        "login" => login_providers,
        _ => return None,
    };
    let filtered = fuzzy_filter(args, pool);
    if filtered.is_empty() {
        return None;
    }
    Some(AutocompleteSuggestions {
        items: filtered
            .into_iter()
            .map(|value| AutocompleteItem {
                label: value.clone(),
                value,
                description: None,
            })
            .collect(),
        prefix: args.to_string(),
    })
}

fn extract_at_prefix(text: &str) -> Option<String> {
    let start = text.rfind('@')?;
    if start > 0 {
        let prev = text[..start].chars().next_back()?;
        if !prev.is_whitespace() && prev != '=' {
            return None;
        }
    }
    Some(text[start..].to_string())
}

fn extract_path_prefix(text: &str, force: bool) -> Option<String> {
    if let Some(at) = extract_at_prefix(text) {
        return Some(at);
    }
    if !force {
        return None;
    }
    let start = text
        .rfind(|ch: char| ch.is_whitespace() || ch == '=' || ch == '"' || ch == '\'')
        .map(|i| i + 1)
        .unwrap_or(0);
    Some(text[start..].to_string())
}

fn parse_path_raw(prefix: &str) -> String {
    let mut raw = prefix;
    if let Some(rest) = raw.strip_prefix('@') {
        raw = rest;
    }
    raw.trim_matches('"').to_string()
}

fn file_suggestions(cwd: &Path, raw_prefix: &str, at_prefix: bool) -> Vec<AutocompleteItem> {
    let expanded = expand_path(cwd, raw_prefix);
    let (dir, file_prefix) = if raw_prefix.ends_with('/') || raw_prefix.ends_with('\\') {
        (expanded, String::new())
    } else {
        let parent = expanded
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf());
        let name = expanded
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        (parent, name)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !file_prefix.starts_with('.') {
            continue;
        }
        if !file_prefix.is_empty()
            && !name
                .to_ascii_lowercase()
                .starts_with(&file_prefix.to_ascii_lowercase())
        {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let mut label = name.clone();
        if is_dir {
            label.push('/');
        }
        let mut path = if raw_prefix.contains('/') || raw_prefix.contains('\\') {
            let parent = raw_prefix.trim_end_matches(|c| c != '/' && c != '\\');
            format!("{parent}{label}")
        } else {
            label.clone()
        };
        if at_prefix && !path.starts_with('@') {
            path = format!("@{path}");
        }
        items.push(AutocompleteItem {
            value: path,
            label,
            description: None,
        });
        if items.len() >= 20 {
            break;
        }
    }
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

fn expand_path(cwd: &Path, raw: &str) -> PathBuf {
    if raw.is_empty() {
        return cwd.to_path_buf();
    }
    if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| cwd.display().to_string());
        return PathBuf::from(raw.replacen('~', &home, 1));
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn completes_slash_commands_and_model_args() {
        let commands = vec![
            SlashCommandSpec {
                name: "model".into(),
                description: "Select model".into(),
                argument_hint: Some("<provider/model>".into()),
            },
            SlashCommandSpec {
                name: "thinking".into(),
                description: "Set thinking".into(),
                argument_hint: Some("<level>".into()),
            },
        ];
        let models = vec!["google/gemini".into(), "anthropic/sonnet".into()];
        let slash =
            suggestions("/mo", &commands, &models, &[], &[], Path::new("."), false).unwrap();
        assert_eq!(slash.items[0].value, "model");
        assert!(slash.items[0]
            .description
            .as_ref()
            .unwrap()
            .contains("provider/model"));
        let args = suggestions(
            "/model son",
            &commands,
            &models,
            &[],
            &[],
            Path::new("."),
            false,
        )
        .unwrap();
        assert_eq!(args.items[0].value, "anthropic/sonnet");
        assert_eq!(
            apply_completion("/mo", 3, "/mo", &slash.items[0]),
            "/model "
        );
    }

    #[test]
    fn completes_at_paths() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("readme.md"), "x").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let found = suggestions("@re", &[], &[], &[], &[], dir.path(), false).unwrap();
        assert!(found.items.iter().any(|item| item.label == "readme.md"));
        let dirs = suggestions("@s", &[], &[], &[], &[], dir.path(), false).unwrap();
        assert!(dirs.items.iter().any(|item| item.label == "src/"));
    }
}
