//! TypeScript `CombinedAutocompleteProvider` from `packages/tui/src/autocomplete.ts`.

use crate::fuzzy::fuzzy_filter;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const PATH_DELIMITERS: &[char] = &[' ', '\t', '"', '\'', '='];

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

pub type ArgumentCompleter = Rc<dyn Fn(&str) -> Vec<AutocompleteItem>>;

#[derive(Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub argument_completions: Option<ArgumentCompleter>,
}

#[derive(Clone)]
pub struct CombinedAutocompleteProvider {
    commands: Vec<SlashCommand>,
    base_path: PathBuf,
    pub trigger_characters: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ApplyLinesResult {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

pub trait AutocompleteProvider {
    fn trigger_characters(&self) -> &[String];
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions>;
    fn apply_completion_lines(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> ApplyLinesResult;
    fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        let _ = (lines, cursor_line, cursor_col);
        true
    }
}

impl Default for CombinedAutocompleteProvider {
    fn default() -> Self {
        Self::new(Vec::new(), ".")
    }
}

impl AutocompleteProvider for CombinedAutocompleteProvider {
    fn trigger_characters(&self) -> &[String] {
        &self.trigger_characters
    }

    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions> {
        CombinedAutocompleteProvider::get_suggestions(self, lines, cursor_line, cursor_col, force)
    }

    fn apply_completion_lines(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> ApplyLinesResult {
        CombinedAutocompleteProvider::apply_completion_lines(
            self,
            lines,
            cursor_line,
            cursor_col,
            item,
            prefix,
        )
    }

    fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        CombinedAutocompleteProvider::should_trigger_file_completion(
            self,
            lines,
            cursor_line,
            cursor_col,
        )
    }
}

impl CombinedAutocompleteProvider {
    pub fn new(commands: Vec<SlashCommand>, base_path: impl Into<PathBuf>) -> Self {
        Self {
            commands,
            base_path: base_path.into(),
            trigger_characters: Vec::new(),
        }
    }

    pub fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions> {
        let current = lines.get(cursor_line).cloned().unwrap_or_default();
        let text = current.chars().take(cursor_col).collect::<String>();
        if let Some(at_prefix) = extract_at_prefix(&text) {
            let parsed = parse_path_prefix(&at_prefix);
            let items = self.fuzzy_file_suggestions(&parsed.raw_prefix, parsed.is_quoted);
            if items.is_empty() {
                return None;
            }
            return Some(AutocompleteSuggestions {
                items,
                prefix: at_prefix,
            });
        }
        if !force && text.starts_with('/') {
            if let Some(space) = text.find(' ') {
                let command_name = &text[1..space];
                let argument_text = &text[space + 1..];
                let command = self.commands.iter().find(|cmd| cmd.name == command_name)?;
                let completer = command.argument_completions.clone()?;
                let items = completer(argument_text);
                if items.is_empty() {
                    return None;
                }
                return Some(AutocompleteSuggestions {
                    items,
                    prefix: argument_text.to_string(),
                });
            }
            let prefix = &text[1..];
            let names: Vec<String> = self.commands.iter().map(|cmd| cmd.name.clone()).collect();
            let filtered = fuzzy_filter(prefix, &names);
            if filtered.is_empty() {
                return None;
            }
            let items = filtered
                .into_iter()
                .filter_map(|matched| {
                    self.commands
                        .iter()
                        .find(|cmd| cmd.name == matched.item)
                        .map(|cmd| {
                            let desc = match (&cmd.argument_hint, &cmd.description) {
                                (Some(hint), Some(description)) if !description.is_empty() => {
                                    Some(format!("{hint} — {description}"))
                                }
                                (Some(hint), _) => Some(hint.clone()),
                                (_, Some(description)) if !description.is_empty() => {
                                    Some(description.clone())
                                }
                                _ => None,
                            };
                            AutocompleteItem {
                                value: cmd.name.clone(),
                                label: cmd.name.clone(),
                                description: desc,
                            }
                        })
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                return None;
            }
            return Some(AutocompleteSuggestions {
                items,
                prefix: text,
            });
        }
        let path_match = extract_path_prefix(&text, force)?;
        let items = self.file_suggestions(&path_match);
        if items.is_empty() {
            return None;
        }
        Some(AutocompleteSuggestions {
            items,
            prefix: path_match,
        })
    }

    pub fn apply_completion(
        &self,
        line: &str,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> (String, usize) {
        let prefix_start = cursor_col.saturating_sub(prefix.chars().count());
        let before: String = line.chars().take(prefix_start).collect();
        let after: String = line.chars().skip(cursor_col).collect();
        let is_quoted = prefix.starts_with('"') || prefix.starts_with("@\"");
        let adjusted_after = if is_quoted && item.value.ends_with('"') && after.starts_with('"') {
            after[1..].to_string()
        } else {
            after
        };
        let is_slash =
            prefix.starts_with('/') && before.trim().is_empty() && !prefix[1..].contains('/');
        if is_slash {
            let new_line = format!("{before}/{} {adjusted_after}", item.value);
            let cursor = before.chars().count() + item.value.chars().count() + 2;
            return (new_line, cursor);
        }
        if prefix.starts_with('@') {
            let is_directory = item.label.ends_with('/');
            let suffix = if is_directory { "" } else { " " };
            let new_line = format!("{before}{}{suffix}{adjusted_after}", item.value);
            let offset = if is_directory && item.value.ends_with('"') {
                item.value.chars().count().saturating_sub(1)
            } else {
                item.value.chars().count()
            };
            return (new_line, before.chars().count() + offset + suffix.len());
        }
        let new_line = format!("{before}{}{adjusted_after}", item.value);
        let is_directory = item.label.ends_with('/');
        let offset = if is_directory && item.value.ends_with('"') {
            item.value.chars().count().saturating_sub(1)
        } else {
            item.value.chars().count()
        };
        (new_line, before.chars().count() + offset)
    }

    pub fn apply_completion_lines(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> ApplyLinesResult {
        let line = lines.get(cursor_line).cloned().unwrap_or_default();
        let (new_line, new_col) = self.apply_completion(&line, cursor_col, item, prefix);
        let mut out = lines.to_vec();
        if cursor_line >= out.len() {
            out.resize(cursor_line + 1, String::new());
        }
        out[cursor_line] = new_line;
        ApplyLinesResult {
            lines: out,
            cursor_line,
            cursor_col: new_col,
        }
    }

    pub fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        let current = lines.get(cursor_line).cloned().unwrap_or_default();
        let text: String = current.chars().take(cursor_col).collect();
        let trimmed = text.trim();
        !trimmed.starts_with('/') || trimmed.contains(' ')
    }

    fn file_suggestions(&self, prefix: &str) -> Vec<AutocompleteItem> {
        let parsed = parse_path_prefix(prefix);
        let raw = parsed.raw_prefix;
        let expanded = expand_home(&raw);
        let is_root = raw.is_empty()
            || raw == "./"
            || raw == "../"
            || raw == "~"
            || raw == "~/"
            || raw == "/"
            || (parsed.is_at && raw.is_empty());
        let (search_dir, search_prefix) = if is_root || raw.ends_with('/') {
            let dir = if raw.starts_with('~') || expanded.starts_with('/') {
                PathBuf::from(&expanded)
            } else {
                self.base_path.join(&expanded)
            };
            (dir, String::new())
        } else {
            let expanded_path = Path::new(&expanded);
            let file = expanded_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let dir = expanded_path.parent().unwrap_or(Path::new(""));
            let search = if raw.starts_with('~') || expanded.starts_with('/') {
                dir.to_path_buf()
            } else {
                self.base_path.join(dir)
            };
            (search, file)
        };
        let entries = match fs::read_dir(&search_dir) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };
        let mut suggestions = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name
                .to_ascii_lowercase()
                .starts_with(&search_prefix.to_ascii_lowercase())
            {
                continue;
            }
            let mut is_directory = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_directory && entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
                is_directory = fs::metadata(entry.path())
                    .map(|m| m.is_dir())
                    .unwrap_or(false);
            }
            let relative = display_relative(&raw, &name);
            let path_value = if is_directory {
                format!("{relative}/")
            } else {
                relative
            };
            let value =
                build_completion_value(&path_value, is_directory, parsed.is_at, parsed.is_quoted);
            suggestions.push(AutocompleteItem {
                value,
                label: format!("{}{}", name, if is_directory { "/" } else { "" }),
                description: None,
            });
        }
        suggestions.sort_by(|a, b| {
            let a_dir = a.value.ends_with('/');
            let b_dir = b.value.ends_with('/');
            match (a_dir, b_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a
                    .label
                    .to_ascii_lowercase()
                    .cmp(&b.label.to_ascii_lowercase()),
            }
        });
        suggestions
    }

    fn fuzzy_file_suggestions(&self, query: &str, is_quoted: bool) -> Vec<AutocompleteItem> {
        let scoped = self.resolve_scoped_fuzzy_query(query);
        let (base_dir, fd_query, display_base) = match &scoped {
            Some(scoped) => (scoped.0.clone(), scoped.1.as_str(), Some(scoped.2.as_str())),
            None => (self.base_path.clone(), query, None),
        };
        let mut shallow = walk_directory(&base_dir, fd_query, 100, Some(1));
        let recursive = walk_directory(&base_dir, fd_query, 100, None);
        let mut seen: std::collections::HashSet<String> =
            shallow.iter().map(|e| e.0.clone()).collect();
        for entry in recursive {
            if seen.insert(entry.0.clone()) {
                shallow.push(entry);
            }
        }
        let mut scored: Vec<(String, bool, i32)> = shallow
            .into_iter()
            .map(|(path, is_dir)| {
                let score = if fd_query.is_empty() {
                    1
                } else {
                    score_entry(&path, fd_query, is_dir)
                };
                (path, is_dir, score)
            })
            .filter(|entry| entry.2 > 0)
            .collect();
        scored.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| path_depth(&a.0).cmp(&path_depth(&b.0)))
                .then_with(|| a.0.len().cmp(&b.0.len()))
                .then_with(|| a.0.cmp(&b.0))
        });
        scored
            .into_iter()
            .take(20)
            .map(|(path, is_dir, _)| {
                let without_slash = path.trim_end_matches('/');
                let display = if let Some(base) = display_base {
                    scoped_path_for_display(base, without_slash)
                } else {
                    without_slash.to_string()
                };
                let entry_name = Path::new(without_slash)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(without_slash)
                    .to_string();
                let completion = if is_dir {
                    format!("{display}/")
                } else {
                    display.clone()
                };
                AutocompleteItem {
                    value: build_completion_value(&completion, is_dir, true, is_quoted),
                    label: format!("{}{}", entry_name, if is_dir { "/" } else { "" }),
                    description: Some(display),
                }
            })
            .collect()
    }

    fn resolve_scoped_fuzzy_query(&self, raw_query: &str) -> Option<(PathBuf, String, String)> {
        let normalized = to_display_path(raw_query);
        let slash = normalized.rfind('/')?;
        let display_base = normalized[..=slash].to_string();
        let query = normalized[slash + 1..].to_string();
        let base_dir = if display_base.starts_with("~/") {
            PathBuf::from(expand_home(&display_base))
        } else if display_base.starts_with('/') {
            PathBuf::from(&display_base)
        } else {
            self.base_path.join(&display_base)
        };
        if !base_dir.is_dir() {
            return None;
        }
        Some((base_dir, query, display_base))
    }
}

fn to_display_path(value: &str) -> String {
    value.replace('\\', "/")
}

fn expand_home(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    if path == "~" {
        home
    } else if let Some(rest) = path.strip_prefix("~/") {
        let expanded = Path::new(&home).join(rest);
        let mut text = expanded.to_string_lossy().into_owned();
        if path.ends_with('/') && !text.ends_with('/') {
            text.push('/');
        }
        text
    } else {
        path.to_string()
    }
}

fn find_last_delimiter(text: &str) -> Option<usize> {
    text.char_indices()
        .rev()
        .find(|(_, ch)| PATH_DELIMITERS.contains(ch))
        .map(|(i, _)| i)
}

fn find_unclosed_quote_start(text: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut quote_start = 0usize;
    for (i, ch) in text.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
            if in_quotes {
                quote_start = i;
            }
        }
    }
    in_quotes.then_some(quote_start)
}

fn is_token_start(text: &str, index: usize) -> bool {
    index == 0
        || text
            .get(..index)
            .and_then(|s| s.chars().next_back())
            .is_some_and(|ch| PATH_DELIMITERS.contains(&ch))
}

fn extract_quoted_prefix(text: &str) -> Option<String> {
    let quote_start = find_unclosed_quote_start(text)?;
    if quote_start > 0 && text.as_bytes().get(quote_start - 1) == Some(&b'@') {
        if !is_token_start(text, quote_start - 1) {
            return None;
        }
        return Some(text[quote_start - 1..].to_string());
    }
    if !is_token_start(text, quote_start) {
        return None;
    }
    Some(text[quote_start..].to_string())
}

fn extract_at_prefix(text: &str) -> Option<String> {
    if let Some(quoted) = extract_quoted_prefix(text) {
        if quoted.starts_with("@\"") {
            return Some(quoted);
        }
    }
    let token_start = find_last_delimiter(text).map(|i| i + 1).unwrap_or(0);
    text.get(token_start..)
        .filter(|token| token.starts_with('@'))
        .map(|token| token.to_string())
}

fn extract_path_prefix(text: &str, force: bool) -> Option<String> {
    if let Some(quoted) = extract_quoted_prefix(text) {
        return Some(quoted);
    }
    let prefix = match find_last_delimiter(text) {
        Some(i) => text[i + 1..].to_string(),
        None => text.to_string(),
    };
    if force {
        return Some(prefix);
    }
    if prefix.contains('/') || prefix.starts_with('.') || prefix.starts_with("~/") {
        return Some(prefix);
    }
    if prefix.is_empty() && text.ends_with(' ') {
        return Some(prefix);
    }
    None
}

struct ParsedPrefix {
    raw_prefix: String,
    is_at: bool,
    is_quoted: bool,
}

fn parse_path_prefix(prefix: &str) -> ParsedPrefix {
    if let Some(rest) = prefix.strip_prefix("@\"") {
        ParsedPrefix {
            raw_prefix: rest.to_string(),
            is_at: true,
            is_quoted: true,
        }
    } else if let Some(rest) = prefix.strip_prefix('"') {
        ParsedPrefix {
            raw_prefix: rest.to_string(),
            is_at: false,
            is_quoted: true,
        }
    } else if let Some(rest) = prefix.strip_prefix('@') {
        ParsedPrefix {
            raw_prefix: rest.to_string(),
            is_at: true,
            is_quoted: false,
        }
    } else {
        ParsedPrefix {
            raw_prefix: prefix.to_string(),
            is_at: false,
            is_quoted: false,
        }
    }
}

fn build_completion_value(path: &str, is_directory: bool, is_at: bool, is_quoted: bool) -> String {
    let _ = is_directory;
    let needs_quotes = is_quoted || path.contains(' ');
    let prefix = if is_at { "@" } else { "" };
    if !needs_quotes {
        return format!("{prefix}{path}");
    }
    format!("{prefix}\"{path}\"")
}

fn display_relative(display_prefix: &str, name: &str) -> String {
    if display_prefix.ends_with('/') {
        return to_display_path(&format!("{display_prefix}{name}"));
    }
    if display_prefix.contains('/') || display_prefix.contains('\\') {
        if let Some(rest) = display_prefix.strip_prefix("~/") {
            let dir = Path::new(rest)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("");
            return if dir.is_empty() || dir == "." {
                format!("~/{name}")
            } else {
                to_display_path(&format!("~/{dir}/{name}"))
            };
        }
        if display_prefix.starts_with('/') {
            let dir = Path::new(display_prefix)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("/");
            return if dir == "/" {
                format!("/{name}")
            } else {
                format!("{dir}/{name}")
            };
        }
        let joined = Path::new(display_prefix)
            .parent()
            .unwrap_or(Path::new(""))
            .join(name);
        let mut relative = to_display_path(&joined.to_string_lossy());
        if display_prefix.starts_with("./") && !relative.starts_with("./") {
            relative = format!("./{relative}");
        }
        return relative;
    }
    if display_prefix.starts_with('~') {
        format!("~/{name}")
    } else {
        name.to_string()
    }
}

fn score_entry(file_path: &str, query: &str, is_directory: bool) -> i32 {
    let file_name = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file_path);
    let lower_name = file_name.to_ascii_lowercase();
    let lower_query = query.to_ascii_lowercase();
    let mut score = if lower_name == lower_query {
        100
    } else if lower_name.starts_with(&lower_query) {
        80
    } else if lower_name.contains(&lower_query) {
        50
    } else if file_path.to_ascii_lowercase().contains(&lower_query) {
        30
    } else {
        0
    };
    if is_directory && score > 0 {
        score += 10;
    }
    score
}

fn path_depth(path: &str) -> usize {
    to_display_path(path)
        .split('/')
        .filter(|part| !part.is_empty())
        .count()
}

fn scoped_path_for_display(display_base: &str, relative: &str) -> String {
    let relative = to_display_path(relative);
    if display_base == "/" {
        format!("/{relative}")
    } else {
        format!("{}{relative}", to_display_path(display_base))
    }
}

fn query_matches(path: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let display = to_display_path(path);
    if !query.contains('/') {
        return display
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
            || Path::new(&display)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| {
                    name.to_ascii_lowercase()
                        .contains(&query.to_ascii_lowercase())
                });
    }
    let has_trailing = query.ends_with('/');
    let trimmed = query.trim_matches('/');
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return true;
    }
    let lower = display.to_ascii_lowercase();
    let needle = segments
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/");
    if has_trailing {
        lower.contains(&format!("{needle}/"))
    } else {
        lower.contains(&needle)
    }
}

fn walk_directory(
    base: &Path,
    query: &str,
    max_results: usize,
    max_depth: Option<usize>,
) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![(base.to_path_buf(), String::new(), 0usize)];
    let mut seen = std::collections::HashSet::new();
    while let Some((dir, prefix, depth)) = stack.pop() {
        if out.len() >= max_results {
            break;
        }
        if let Some(max) = max_depth {
            if depth > max {
                continue;
            }
        }
        let canonical = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen.insert(canonical) {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if rel == ".git" || rel.starts_with(".git/") || rel.contains("/.git/") {
                continue;
            }
            let path = entry.path();
            let meta = fs::metadata(&path);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            children.push((path, rel, is_dir));
        }
        children.sort_by(|a, b| a.1.cmp(&b.1));
        for (path, rel, is_dir) in children {
            if is_dir {
                if max_depth.map(|max| depth < max).unwrap_or(true) {
                    stack.push((path, rel.clone(), depth + 1));
                }
                let display = format!("{rel}/");
                if query_matches(&display, query) {
                    out.push((display, true));
                }
            } else if query_matches(&rel, query) {
                out.push((rel, false));
            }
            if out.len() >= max_results {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pi-autocomplete-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_tree(base: &Path, dirs: &[&str], files: &[(&str, &str)]) {
        for dir in dirs {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
        for (file, contents) in files {
            let path = base.join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }
    }

    #[test]
    fn force_extracts_root_slash_and_skips_slash_commands() {
        let provider = CombinedAutocompleteProvider::new(Vec::new(), "/tmp");
        let hey = provider
            .get_suggestions(&["hey /".into()], 0, 5, true)
            .expect("root");
        assert_eq!(hey.prefix, "/");
        assert!(provider
            .get_suggestions(&["/model".into()], 0, 6, true)
            .is_none());
        let arg = provider
            .get_suggestions(&["/command /".into()], 0, 10, true)
            .expect("abs");
        assert_eq!(arg.prefix, "/");
    }

    #[test]
    fn slash_commands_filter_and_apply() {
        let provider = CombinedAutocompleteProvider::new(
            vec![
                SlashCommand {
                    name: "model".into(),
                    description: Some("Select model".into()),
                    argument_hint: None,
                    argument_completions: None,
                },
                SlashCommand {
                    name: "login".into(),
                    description: Some("Login".into()),
                    argument_hint: None,
                    argument_completions: None,
                },
            ],
            "/tmp",
        );
        let suggestions = provider
            .get_suggestions(&["/mo".into()], 0, 3, false)
            .expect("slash");
        assert_eq!(suggestions.prefix, "/mo");
        assert_eq!(suggestions.items[0].value, "model");
        let (line, col) =
            provider.apply_completion("/mo", 3, &suggestions.items[0], &suggestions.prefix);
        assert_eq!(line, "/model ");
        assert_eq!(col, 7);
    }

    #[test]
    fn argument_completions_match_prefix() {
        let models: ArgumentCompleter = Rc::new(|prefix: &str| {
            let labels = [
                "openai-codex/gpt-5.5 openai-codex gpt-5.5",
                "github-copilot/gpt-5.2-codex github-copilot gpt-5.2-codex",
            ];
            let items = [
                AutocompleteItem {
                    value: "openai-codex/gpt-5.5".into(),
                    label: "gpt-5.5".into(),
                    description: Some("openai-codex".into()),
                },
                AutocompleteItem {
                    value: "github-copilot/gpt-5.2-codex".into(),
                    label: "gpt-5.2-codex".into(),
                    description: Some("github-copilot".into()),
                },
            ];
            fuzzy_filter(
                prefix,
                &labels.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
            )
            .into_iter()
            .filter_map(|matched| {
                items
                    .iter()
                    .find(|item| matched.item.starts_with(&item.value))
                    .cloned()
            })
            .collect()
        });
        let provider = CombinedAutocompleteProvider::new(
            vec![SlashCommand {
                name: "model".into(),
                description: None,
                argument_hint: None,
                argument_completions: Some(models),
            }],
            "/tmp",
        );
        let suggestions = provider
            .get_suggestions(&["/model codexgpt".into()], 0, 15, false)
            .expect("args");
        assert_eq!(suggestions.items[0].value, "openai-codex/gpt-5.5");
    }

    #[test]
    fn preserves_dot_slash_and_quoted_paths() {
        let base = temp_dir();
        write_tree(
            &base,
            &["src", "my folder"],
            &[
                ("update.sh", "#!/bin/bash"),
                ("utils.ts", "export {};"),
                ("src/index.ts", "export {};"),
                ("my folder/test.txt", "content"),
                ("my folder/other.txt", "content"),
            ],
        );
        let provider = CombinedAutocompleteProvider::new(Vec::new(), &base);
        let values = provider
            .get_suggestions(&["./up".into()], 0, 4, true)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(values.contains(&"./update.sh".into()), "{values:?}");
        let dirs = provider
            .get_suggestions(&["./sr".into()], 0, 4, true)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(dirs.contains(&"./src/".into()), "{dirs:?}");
        let quoted = provider
            .get_suggestions(&["my".into()], 0, 2, true)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(quoted.contains(&"\"my folder/\"".into()), "{quoted:?}");
        let quoted_line = "\"my folder/\"";
        let inside = provider
            .get_suggestions(
                &[quoted_line.into()],
                0,
                quoted_line.chars().count() - 1,
                true,
            )
            .unwrap();
        let inside_values: Vec<_> = inside.items.iter().map(|i| i.value.clone()).collect();
        assert!(
            inside_values.contains(&"\"my folder/test.txt\"".into()),
            "{inside_values:?}"
        );
        let apply_line = "\"my folder/te\"";
        let apply_col = apply_line.chars().count() - 1;
        let apply_suggestions = provider
            .get_suggestions(&[apply_line.into()], 0, apply_col, true)
            .unwrap();
        let item = apply_suggestions
            .items
            .iter()
            .find(|i| i.value == "\"my folder/test.txt\"")
            .unwrap();
        let (applied, _) =
            provider.apply_completion(apply_line, apply_col, item, &apply_suggestions.prefix);
        assert_eq!(applied, "\"my folder/test.txt\"");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn at_suggestions_match_typescript_fixtures() {
        let base = temp_dir();
        write_tree(
            &base,
            &[
                "src",
                "packages/tui/src",
                "packages/ai/src",
                ".pi",
                ".github",
                ".git",
            ],
            &[
                ("README.md", "readme"),
                ("file.txt", "content"),
                ("src.txt", "text"),
                ("src/index.ts", "export {};"),
                ("packages/tui/src/autocomplete.ts", "export {};"),
                ("packages/ai/src/autocomplete.ts", "export {};"),
                ("src/components/Button.tsx", "export {};"),
                ("src/utils/helpers.ts", "export {};"),
                (".pi/config.json", "{}"),
                (".github/workflows/ci.yml", "name: ci"),
                (".git/config", "[core]"),
                ("my folder/test.txt", "content"),
            ],
        );
        let provider = CombinedAutocompleteProvider::new(Vec::new(), &base);
        let empty: Vec<_> = provider
            .get_suggestions(&["@".into()], 0, 1, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect();
        assert!(empty.contains(&"@README.md".into()), "{empty:?}");
        assert!(empty.contains(&"@src/".into()), "{empty:?}");
        assert!(empty.contains(&"@.pi/".into()));
        assert!(empty.contains(&"@.github/".into()));
        assert!(!empty
            .iter()
            .any(|v| v == "@.git" || v.starts_with("@.git/")));
        let file = provider
            .get_suggestions(&["@file.txt".into()], 0, 9, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(file.contains(&"@file.txt".into()));
        let case = provider
            .get_suggestions(&["@re".into()], 0, 3, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert_eq!(case, vec!["@README.md".to_string()]);
        let ranked = provider
            .get_suggestions(&["@src".into()], 0, 4, false)
            .unwrap();
        assert_eq!(ranked.items[0].value, "@src/");
        assert!(ranked.items.iter().any(|i| i.value == "@src.txt"));
        let nested = provider
            .get_suggestions(&["@index".into()], 0, 6, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(nested.contains(&"@src/index.ts".into()));
        let deep = provider
            .get_suggestions(&["@tui/src/auto".into()], 0, 13, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(deep.contains(&"@packages/tui/src/autocomplete.ts".into()));
        assert!(!deep.contains(&"@packages/ai/src/autocomplete.ts".into()));
        let mid = provider
            .get_suggestions(&["@components/".into()], 0, 12, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(mid.contains(&"@src/components/Button.tsx".into()));
        assert!(!mid.contains(&"@src/utils/helpers.ts".into()));
        let spaces = provider
            .get_suggestions(&["@my".into()], 0, 3, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(spaces.contains(&"@\"my folder/\"".into()));
        let quoted_line = "@\"my folder/\"";
        let quoted = provider
            .get_suggestions(
                &[quoted_line.into()],
                0,
                quoted_line.chars().count() - 1,
                false,
            )
            .unwrap();
        assert!(quoted
            .items
            .iter()
            .any(|i| i.value == "@\"my folder/test.txt\""));
        let apply_line = "@\"my folder/te\"";
        let apply_col = apply_line.chars().count() - 1;
        let apply_suggestions = provider
            .get_suggestions(&[apply_line.into()], 0, apply_col, false)
            .unwrap();
        let item = apply_suggestions
            .items
            .iter()
            .find(|i| i.value == "@\"my folder/test.txt\"")
            .unwrap();
        let (applied, _) =
            provider.apply_completion(apply_line, apply_col, item, &apply_suggestions.prefix);
        assert_eq!(applied, "@\"my folder/test.txt\" ");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn at_follows_symlinks_and_ranks_shallow_matches() {
        let root = temp_dir();
        let base = root.join("cwd");
        let outside = root.join("outside");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&outside).unwrap();
        write_tree(
            &base,
            &[
                "dir",
                "scope/aaa/venv/lib/python3.12/site-packages/pkg/core/profile",
                "scope/projects",
            ],
            &[("dir/some_file.txt", "real"), ("original.txt", "content")],
        );
        write_tree(&outside, &["nested"], &[("some_file.txt", "symlinked")]);
        symlink("../outside", base.join("symlinked_dir")).unwrap();
        symlink("original.txt", base.join("link.txt")).unwrap();
        let provider = CombinedAutocompleteProvider::new(Vec::new(), &base);
        let some = provider
            .get_suggestions(&["@some".into()], 0, 5, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(some.contains(&"@dir/some_file.txt".into()), "{some:?}");
        assert!(
            some.contains(&"@symlinked_dir/some_file.txt".into()),
            "{some:?}"
        );
        let linked = provider
            .get_suggestions(&["@symlinked".into()], 0, 10, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(linked.contains(&"@symlinked_dir/".into()), "{linked:?}");
        let link_file = provider
            .get_suggestions(&["@link".into()], 0, 5, false)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.value)
            .collect::<Vec<_>>();
        assert!(link_file.contains(&"@link.txt".into()));
        let ranked = provider
            .get_suggestions(&["@scope/pro".into()], 0, 10, false)
            .unwrap();
        assert_eq!(ranked.items[0].value, "@scope/projects/");
        assert!(ranked.items.iter().any(|i| i.value.contains("/profile/")));
        let _ = fs::remove_dir_all(&root);
    }
}
