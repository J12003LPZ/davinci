//! Combined slash + path autocomplete matching
//! `vendor/pi/packages/tui/src/autocomplete.ts` and editor trigger/debounce
//! from `vendor/pi/packages/tui/src/components/editor.ts`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::fuzzy::{fuzzy_filter, fuzzy_match};

type LiveAutocompleteFn = dyn Fn(&str) -> Vec<AutocompleteItem> + Send + Sync;

/// Live `getSuggestions` callback from a JS/native extension provider.
#[derive(Clone)]
pub struct LiveAutocompleteQuery(pub Arc<LiveAutocompleteFn>);

impl std::fmt::Debug for LiveAutocompleteQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveAutocompleteQuery")
    }
}

/// TS `ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS`.
pub const ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS: u64 = 20;
/// TS `DEFAULT_AUTOCOMPLETE_TRIGGER_CHARACTERS`.
pub const DEFAULT_AUTOCOMPLETE_TRIGGER_CHARACTERS: &[char] = &['@', '#'];

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

#[derive(Debug, Clone, Default)]
pub struct SlashCommandSpec {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub argument_items: Vec<AutocompleteItem>,
}

/// Extension `AutocompleteProvider` with `triggerCharacters` (typically `#`).
/// Combined file search stays `@`-only; `#` is never treated as a file attach.
#[derive(Debug, Clone, Default)]
pub struct ExtraAutocompleteProvider {
    pub trigger_characters: Vec<char>,
    pub items: Vec<AutocompleteItem>,
    pub live_query: Option<LiveAutocompleteQuery>,
}

pub struct SuggestionQuery<'a> {
    pub text: &'a str,
    pub commands: &'a [SlashCommandSpec],
    pub models: &'a [String],
    pub thinking_levels: &'a [String],
    pub login_providers: &'a [String],
    pub extra_providers: &'a [ExtraAutocompleteProvider],
    pub cwd: &'a Path,
    pub force_path: bool,
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
    let is_quoted_prefix = prefix.starts_with('"') || prefix.starts_with("@\"");
    let has_leading_quote_after = after.starts_with('"');
    let has_trailing_quote_in_item = item.value.ends_with('"');
    let after = if is_quoted_prefix && has_trailing_quote_in_item && has_leading_quote_after {
        after.get(1..).unwrap_or(after)
    } else {
        after
    };
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

/// TS `getAutocompleteDebounceMs`: 20ms for `@` / extra trigger tokens, 0 for Tab/force.
pub fn autocomplete_debounce_ms(
    force: bool,
    text: &str,
    extra: &[ExtraAutocompleteProvider],
) -> u64 {
    if force {
        return 0;
    }
    if extract_at_prefix(text).is_some() {
        return ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS;
    }
    if extra_token(text, extra).is_some() {
        return ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS;
    }
    if last_token_starts_with(text, '#') {
        return ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS;
    }
    0
}

pub fn suggestions(query: SuggestionQuery<'_>) -> Option<AutocompleteSuggestions> {
    if let Some((prefix, provider)) = extra_token(query.text, query.extra_providers) {
        let trigger = provider
            .trigger_characters
            .iter()
            .find(|ch| prefix.starts_with(**ch))?;
        let rest = prefix[trigger.len_utf8()..].to_string();
        let items = if let Some(live) = &provider.live_query {
            let live_items = (live.0)(query.text);
            if live_items.is_empty() {
                filter_extra_items(&provider.items, &rest)
            } else {
                live_items
            }
        } else {
            filter_extra_items(&provider.items, &rest)
        };
        if items.is_empty() {
            return None;
        }
        return Some(AutocompleteSuggestions { items, prefix });
    }
    if let Some(at) = extract_at_prefix(query.text) {
        let parsed = parse_path_prefix(&at);
        let items = file_suggestions(query.cwd, &parsed.raw_prefix, true, parsed.is_quoted);
        if items.is_empty() {
            return None;
        }
        return Some(AutocompleteSuggestions { items, prefix: at });
    }
    if let Some(rest) = query.text.strip_prefix('/') {
        if let Some((name, args)) = rest.split_once(' ') {
            if let Some(found) = argument_suggestions(
                name,
                args,
                query.commands,
                query.models,
                query.thinking_levels,
                query.login_providers,
            ) {
                return Some(found);
            }
        } else {
            // Command names use a stricter relevance pipeline than generic fuzzy lists.
            // Short slash queries intentionally reject weak interior/subsequence hits so
            // `/me` stays about memory commands instead of filling with model/name/resume.
            let filtered = rank_slash_commands(rest, query.commands);
            if filtered.is_empty() {
                return None;
            }
            let items = filtered
                .into_iter()
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
                prefix: query.text.to_string(),
            });
        }
    }
    if query.force_path {
        if let Some(prefix) = extract_path_prefix(query.text, true) {
            if prefix.starts_with('/') && !prefix[1..].contains('/') && !prefix.contains(' ') {
                return None;
            }
            let parsed = parse_path_prefix(&prefix);
            let items = readdir_file_suggestions(
                query.cwd,
                &parsed.raw_prefix,
                parsed.is_at,
                parsed.is_quoted,
            );
            if !items.is_empty() {
                return Some(AutocompleteSuggestions { items, prefix });
            }
        }
    }
    None
}

const MAX_SLASH_SUGGESTIONS: usize = 20;

#[derive(Debug, Clone, Copy)]
struct SlashMatch {
    class: u8,
    distance: usize,
    position: usize,
    fuzzy_score: f64,
}

fn rank_slash_commands<'a>(
    query: &str,
    commands: &'a [SlashCommandSpec],
) -> Vec<&'a SlashCommandSpec> {
    if query.is_empty() {
        return commands.iter().collect();
    }

    let query_lower = query.to_lowercase();
    let mut ranked = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            let name = command.name.to_lowercase();
            slash_name_match(&query_lower, &name).map(|matched| {
                let description_hit = command.description.to_lowercase().contains(&query_lower);
                (
                    matched,
                    description_hit,
                    name.chars().count(),
                    index,
                    command,
                )
            })
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|a, b| {
        a.0.class
            .cmp(&b.0.class)
            .then_with(|| a.0.distance.cmp(&b.0.distance))
            .then_with(|| a.0.position.cmp(&b.0.position))
            .then_with(|| {
                a.0.fuzzy_score
                    .partial_cmp(&b.0.fuzzy_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.3.cmp(&b.3))
    });

    ranked
        .into_iter()
        .take(MAX_SLASH_SUGGESTIONS)
        .map(|(_, _, _, _, command)| command)
        .collect()
}

fn slash_name_match(query: &str, name: &str) -> Option<SlashMatch> {
    if name == query {
        return Some(SlashMatch {
            class: 0,
            distance: 0,
            position: 0,
            fuzzy_score: 0.0,
        });
    }
    if name.starts_with(query) {
        return Some(SlashMatch {
            class: 1,
            distance: 0,
            position: 0,
            fuzzy_score: 0.0,
        });
    }

    if let Some(position) = segment_prefix_position(name, query) {
        return Some(SlashMatch {
            class: 2,
            distance: 0,
            position,
            fuzzy_score: 0.0,
        });
    }

    let query_len = query.chars().count();
    if query_len >= 3 {
        if let Some(position) = name.find(query) {
            return Some(SlashMatch {
                class: 3,
                distance: 0,
                position,
                fuzzy_score: 0.0,
            });
        }
    }

    if query_len >= 4 {
        let typo_limit = if query_len >= 7 { 2 } else { 1 };
        if let Some(distance) = best_typo_distance(query, name, typo_limit) {
            return Some(SlashMatch {
                class: 4,
                distance,
                position: 0,
                fuzzy_score: 0.0,
            });
        }
    }

    if query_len >= 3 {
        let fuzzy = fuzzy_match(query, name);
        if fuzzy.matches {
            if let Some(span) = subsequence_span(query, name) {
                if span <= query_len + 1 {
                    return Some(SlashMatch {
                        class: 5,
                        distance: 0,
                        position: span.saturating_sub(query_len),
                        fuzzy_score: fuzzy.score,
                    });
                }
            }
        }
    }

    None
}

fn segment_prefix_position(name: &str, query: &str) -> Option<usize> {
    let mut segment_start = 0;
    for (index, ch) in name.char_indices() {
        if matches!(ch, '-' | '_' | '.' | ':' | '/' | ' ') {
            segment_start = index + ch.len_utf8();
            continue;
        }
        if index == segment_start && name[index..].starts_with(query) {
            return Some(index);
        }
    }
    None
}

fn best_typo_distance(query: &str, name: &str, limit: usize) -> Option<usize> {
    let mut best = damerau_levenshtein(query, name);
    for segment in name.split(['-', '_', '.', ':', '/', ' ']) {
        if !segment.is_empty() {
            best = best.min(damerau_levenshtein(query, segment));
        }
    }
    (best <= limit).then_some(best)
}

fn damerau_levenshtein(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut matrix = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=right.len() {
        matrix[0][j] = j;
    }
    for i in 1..=left.len() {
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            let mut value = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                value = value.min(matrix[i - 2][j - 2] + 1);
            }
            matrix[i][j] = value;
        }
    }
    matrix[left.len()][right.len()]
}

fn subsequence_span(query: &str, text: &str) -> Option<usize> {
    let query = query.chars().collect::<Vec<_>>();
    if query.is_empty() {
        return Some(0);
    }
    let mut query_index = 0usize;
    let mut first = None;
    for (index, ch) in text.chars().enumerate() {
        if ch == query[query_index] {
            first.get_or_insert(index);
            query_index += 1;
            if query_index == query.len() {
                return first.map(|start| index - start + 1);
            }
        }
    }
    None
}

/// Display order from the model picker reference. Unknown models retain their
/// source order after these featured generations; no release dates are inferred.
pub fn model_picker_rank(model: &str) -> usize {
    let id = model.split_once('/').map_or(model, |(_, id)| id).trim();
    match id {
        "gpt-6-astra" => 0,
        "gpt-5.6-luna" => 1,
        "gpt-5.6-sol" => 2,
        "gpt-5.6-terra" => 3,
        "gpt-5.5" => 4,
        "gpt-5.4-mini" => 5,
        _ => 6,
    }
}

fn argument_suggestions(
    command: &str,
    args: &str,
    commands: &[SlashCommandSpec],
    models: &[String],
    thinking_levels: &[String],
    login_providers: &[String],
) -> Option<AutocompleteSuggestions> {
    if let Some(spec) = commands.iter().find(|c| c.name == command) {
        if !spec.argument_items.is_empty() {
            let labels: Vec<String> = spec
                .argument_items
                .iter()
                .map(|item| item.value.clone())
                .collect();
            let filtered = fuzzy_filter(args, &labels);
            if filtered.is_empty() {
                return None;
            }
            let items = filtered
                .into_iter()
                .filter_map(|value| {
                    spec.argument_items
                        .iter()
                        .find(|item| item.value == value)
                        .cloned()
                })
                .collect();
            return Some(AutocompleteSuggestions {
                items,
                prefix: args.to_string(),
            });
        }
    }
    let pool = match command {
        "model" => models,
        "thinking" | "effort" => thinking_levels,
        "login" => login_providers,
        _ => return None,
    };
    let mut filtered = fuzzy_filter(args, pool);
    if command == "model" {
        filtered.sort_by(|left, right| {
            let provider = |value: &str| {
                value
                    .split_once('/')
                    .map_or("", |(provider, _)| provider)
                    .trim()
                    .to_string()
            };
            provider(left)
                .cmp(&provider(right))
                .then_with(|| model_picker_rank(left).cmp(&model_picker_rank(right)))
        });
    }
    if filtered.is_empty() {
        return None;
    }
    Some(AutocompleteSuggestions {
        items: filtered
            .into_iter()
            .map(|value| AutocompleteItem {
                label: value.clone(),
                value: if command == "model" {
                    value
                        .split_once('/')
                        .map(|(provider, id)| format!("{}/{}", provider.trim(), id.trim()))
                        .unwrap_or(value)
                } else {
                    value
                },
                description: None,
            })
            .collect(),
        prefix: args.to_string(),
    })
}

fn extra_token<'a>(
    text: &'a str,
    providers: &'a [ExtraAutocompleteProvider],
) -> Option<(String, &'a ExtraAutocompleteProvider)> {
    for provider in providers {
        for ch in &provider.trigger_characters {
            if *ch == '@' || *ch == '/' {
                continue;
            }
            if last_token_starts_with(text, *ch) {
                let token = last_token(text);
                return Some((token.to_string(), provider));
            }
        }
    }
    None
}

fn last_token(text: &str) -> &str {
    let start = text
        .rfind(|ch: char| ch.is_whitespace() || ch == '=' || ch == '"' || ch == '\'')
        .map(|i| i + 1)
        .unwrap_or(0);
    &text[start..]
}

fn last_token_starts_with(text: &str, ch: char) -> bool {
    last_token(text).starts_with(ch)
}

fn filter_extra_items(items: &[AutocompleteItem], query: &str) -> Vec<AutocompleteItem> {
    if query.is_empty() {
        return items.iter().take(20).cloned().collect();
    }
    let haystack: Vec<String> = items
        .iter()
        .map(|item| {
            item.description
                .as_ref()
                .map(|desc| format!("{} {}", item.value, desc))
                .unwrap_or_else(|| item.value.clone())
        })
        .collect();
    let filtered = fuzzy_filter(query, &haystack);
    let mut out = Vec::new();
    for key in filtered {
        if let Some(item) = items.iter().find(|item| {
            let hay = item
                .description
                .as_ref()
                .map(|desc| format!("{} {}", item.value, desc))
                .unwrap_or_else(|| item.value.clone());
            hay == key
        }) {
            if !out
                .iter()
                .any(|existing: &AutocompleteItem| existing == item)
            {
                out.push(item.clone());
            }
        }
        if out.len() >= 20 {
            break;
        }
    }
    if out.is_empty() {
        items
            .iter()
            .filter(|item| {
                item.value
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
                    || item
                        .label
                        .to_ascii_lowercase()
                        .contains(&query.to_ascii_lowercase())
            })
            .take(20)
            .cloned()
            .collect()
    } else {
        out
    }
}

fn extract_at_prefix(text: &str) -> Option<String> {
    if let Some(quoted) = extract_quoted_prefix(text) {
        if quoted.starts_with("@\"") {
            return Some(quoted);
        }
    }
    let start = text.rfind('@')?;
    if start > 0 {
        let prev = text[..start].chars().next_back()?;
        if !prev.is_whitespace() && prev != '=' {
            return None;
        }
    }
    Some(text[start..].to_string())
}

fn extract_quoted_prefix(text: &str) -> Option<String> {
    let mut in_quotes = false;
    let mut quote_start = None;
    for (i, ch) in text.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
            if in_quotes {
                quote_start = Some(i);
            }
        }
    }
    let quote_start = if in_quotes { quote_start } else { None }?;
    if quote_start > 0 {
        let prev = text[..quote_start].chars().next_back()?;
        if prev == '@' {
            let at = quote_start - prev.len_utf8();
            if at > 0 {
                let before = text[..at].chars().next_back()?;
                if !before.is_whitespace() && before != '=' {
                    return None;
                }
            }
            return Some(text[at..].to_string());
        }
        if !prev.is_whitespace() && prev != '=' {
            return None;
        }
    }
    Some(text[quote_start..].to_string())
}

fn extract_path_prefix(text: &str, force: bool) -> Option<String> {
    if let Some(at) = extract_at_prefix(text) {
        return Some(at);
    }
    if let Some(quoted) = extract_quoted_prefix(text) {
        return Some(quoted);
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

struct ParsedPrefix {
    raw_prefix: String,
    is_at: bool,
    is_quoted: bool,
}

fn parse_path_prefix(prefix: &str) -> ParsedPrefix {
    if let Some(rest) = prefix.strip_prefix("@\"") {
        return ParsedPrefix {
            raw_prefix: rest.to_string(),
            is_at: true,
            is_quoted: true,
        };
    }
    if let Some(rest) = prefix.strip_prefix('"') {
        return ParsedPrefix {
            raw_prefix: rest.to_string(),
            is_at: false,
            is_quoted: true,
        };
    }
    if let Some(rest) = prefix.strip_prefix('@') {
        return ParsedPrefix {
            raw_prefix: rest.to_string(),
            is_at: true,
            is_quoted: false,
        };
    }
    ParsedPrefix {
        raw_prefix: prefix.to_string(),
        is_at: false,
        is_quoted: false,
    }
}

fn file_suggestions(
    cwd: &Path,
    raw_prefix: &str,
    at_prefix: bool,
    is_quoted: bool,
) -> Vec<AutocompleteItem> {
    if let Some(items) = get_fuzzy_file_suggestions(cwd, raw_prefix, is_quoted) {
        return items;
    }
    readdir_file_suggestions(cwd, raw_prefix, at_prefix, is_quoted)
}

struct FdEntry {
    path: String,
    is_directory: bool,
}

fn get_fuzzy_file_suggestions(
    cwd: &Path,
    query: &str,
    is_quoted: bool,
) -> Option<Vec<AutocompleteItem>> {
    if !fd_available() {
        return None;
    }
    let scoped = resolve_scoped_fuzzy_query(cwd, query);
    let fd_base = scoped
        .as_ref()
        .map(|s| s.base_dir.clone())
        .unwrap_or_else(|| cwd.to_path_buf());
    let fd_query = scoped.as_ref().map(|s| s.query.as_str()).unwrap_or(query);
    let base_dir_entries = walk_directory_with_fd(&fd_base, fd_query, Some(1));
    let recursive_entries = walk_directory_with_fd(&fd_base, fd_query, None);
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for entry in base_dir_entries.into_iter().chain(recursive_entries) {
        if seen.insert(entry.path.clone()) {
            entries.push(entry);
        }
    }
    let mut scored: Vec<(i32, FdEntry)> = entries
        .into_iter()
        .map(|entry| {
            let score = if fd_query.is_empty() {
                1
            } else {
                score_entry(&entry.path, fd_query, entry.is_directory)
            };
            (score, entry)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| {
        b_score.cmp(a_score).then_with(|| {
            let a_depth = to_display_path(&a.path)
                .split('/')
                .filter(|s| !s.is_empty())
                .count();
            let b_depth = to_display_path(&b.path)
                .split('/')
                .filter(|s| !s.is_empty())
                .count();
            a_depth
                .cmp(&b_depth)
                .then_with(|| a.path.len().cmp(&b.path.len()))
                .then_with(|| a.path.cmp(&b.path))
        })
    });
    let mut suggestions = Vec::new();
    for (_, entry) in scored.into_iter().take(20) {
        let path_without_slash = if entry.is_directory {
            entry.path.trim_end_matches('/').to_string()
        } else {
            entry.path.clone()
        };
        let display_path = if let Some(scoped) = &scoped {
            scoped_path_for_display(&scoped.display_base, &path_without_slash)
        } else {
            path_without_slash.clone()
        };
        let entry_name = Path::new(&path_without_slash)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&path_without_slash)
            .to_string();
        let completion_path = if entry.is_directory {
            format!("{display_path}/")
        } else {
            display_path.clone()
        };
        let value = build_completion_value(&completion_path, entry.is_directory, true, is_quoted);
        suggestions.push(AutocompleteItem {
            value,
            label: format!(
                "{}{}",
                entry_name,
                if entry.is_directory { "/" } else { "" }
            ),
            description: Some(display_path),
        });
    }
    Some(suggestions)
}

fn fd_available() -> bool {
    std::env::var("PI_FD_REPLY").is_ok() || resolve_fd_binary().is_some()
}

fn resolve_fd_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("PI_FD_PATH") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Some(path);
        }
    }
    let mut bin_dirs = Vec::new();
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        bin_dirs.push(PathBuf::from(dir).join("bin"));
    }
    if let Ok(home) = std::env::var("HOME") {
        bin_dirs.push(PathBuf::from(home).join(".pi").join("agent").join("bin"));
    }
    for dir in bin_dirs {
        for name in ["fd", "fdfind"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    for name in ["fd", "fdfind"] {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    None
}

fn walk_directory_with_fd(base_dir: &Path, query: &str, max_depth: Option<u32>) -> Vec<FdEntry> {
    if let Ok(reply) = std::env::var("PI_FD_REPLY") {
        return parse_fd_stdout(&reply, max_depth);
    }
    let Some(fd_path) = resolve_fd_binary() else {
        return Vec::new();
    };
    let mut cmd = Command::new(fd_path);
    cmd.arg("--base-directory")
        .arg(base_dir)
        .arg("--max-results")
        .arg("100")
        .arg("--type")
        .arg("f")
        .arg("--type")
        .arg("d")
        .arg("--follow")
        .arg("--hidden")
        .arg("--exclude")
        .arg(".git")
        .arg("--exclude")
        .arg(".git/*")
        .arg("--exclude")
        .arg(".git/**");
    if let Some(depth) = max_depth {
        cmd.arg("--max-depth").arg(depth.to_string());
    }
    if to_display_path(query).contains('/') {
        cmd.arg("--full-path");
    }
    if !query.is_empty() {
        cmd.arg(build_fd_path_query(query));
    }
    let Ok(output) = cmd.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_fd_stdout(&String::from_utf8_lossy(&output.stdout), None)
}

fn parse_fd_stdout(stdout: &str, max_depth: Option<u32>) -> Vec<FdEntry> {
    let mut results = Vec::new();
    for line in stdout.trim().split('\n').filter(|line| !line.is_empty()) {
        let display = to_display_path(line);
        let has_trailing = display.ends_with('/');
        let normalized = if has_trailing {
            display.trim_end_matches('/').to_string()
        } else {
            display.clone()
        };
        if normalized == ".git" || normalized.starts_with(".git/") || normalized.contains("/.git/")
        {
            continue;
        }
        if let Some(depth) = max_depth {
            let segments = normalized.split('/').filter(|s| !s.is_empty()).count();
            if segments > depth as usize {
                continue;
            }
        }
        results.push(FdEntry {
            path: display,
            is_directory: has_trailing,
        });
    }
    results
}

fn to_display_path(value: &str) -> String {
    value.replace('\\', "/")
}

fn escape_regex(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn build_fd_path_query(query: &str) -> String {
    let normalized = to_display_path(query);
    if !normalized.contains('/') {
        return normalized;
    }
    let has_trailing_separator = normalized.ends_with('/');
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return normalized;
    }
    let segments: Vec<String> = trimmed
        .split('/')
        .filter(|s| !s.is_empty())
        .map(escape_regex)
        .collect();
    if segments.is_empty() {
        return normalized;
    }
    let mut pattern = segments.join("[\\\\/]");
    if has_trailing_separator {
        pattern.push_str("[\\\\/]");
    }
    pattern
}

fn score_entry(file_path: &str, query: &str, is_directory: bool) -> i32 {
    let file_name = Path::new(file_path.trim_end_matches('/'))
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file_path);
    let lower_file_name = file_name.to_ascii_lowercase();
    let lower_query = query.to_ascii_lowercase();
    let mut score = 0;
    if lower_file_name == lower_query {
        score = 100;
    } else if lower_file_name.starts_with(&lower_query) {
        score = 80;
    } else if lower_file_name.contains(&lower_query) {
        score = 50;
    } else if file_path.to_ascii_lowercase().contains(&lower_query) {
        score = 30;
    }
    if is_directory && score > 0 {
        score += 10;
    }
    score
}

struct ScopedFuzzyQuery {
    base_dir: PathBuf,
    query: String,
    display_base: String,
}

fn resolve_scoped_fuzzy_query(cwd: &Path, raw_query: &str) -> Option<ScopedFuzzyQuery> {
    let normalized = to_display_path(raw_query);
    let slash = normalized.rfind('/')?;
    let display_base = normalized[..=slash].to_string();
    let query = normalized[slash + 1..].to_string();
    let base_dir = if display_base.starts_with("~/") {
        expand_path(cwd, &display_base)
    } else if display_base.starts_with('/') {
        PathBuf::from(&display_base)
    } else {
        cwd.join(&display_base)
    };
    if !base_dir.is_dir() {
        return None;
    }
    Some(ScopedFuzzyQuery {
        base_dir,
        query,
        display_base,
    })
}

fn scoped_path_for_display(display_base: &str, relative_path: &str) -> String {
    let relative = to_display_path(relative_path);
    if display_base == "/" {
        format!("/{relative}")
    } else {
        format!("{}{relative}", to_display_path(display_base))
    }
}

fn build_completion_value(
    path: &str,
    _is_directory: bool,
    is_at_prefix: bool,
    is_quoted_prefix: bool,
) -> String {
    let needs_quotes = is_quoted_prefix || path.contains(' ');
    let prefix = if is_at_prefix { "@" } else { "" };
    if !needs_quotes {
        return format!("{prefix}{path}");
    }
    format!("{prefix}\"{path}\"")
}

fn readdir_file_suggestions(
    cwd: &Path,
    raw_prefix: &str,
    at_prefix: bool,
    is_quoted: bool,
) -> Vec<AutocompleteItem> {
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
        let value = if is_quoted && !path.contains('"') {
            if let Some(rest) = path.strip_prefix('@') {
                format!("@\"{rest}\"")
            } else {
                format!("\"{path}\"")
            }
        } else {
            path
        };
        items.push(AutocompleteItem {
            value,
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
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_fd_reply<T>(reply: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        match reply {
            Some(value) => std::env::set_var("PI_FD_REPLY", value),
            None => std::env::remove_var("PI_FD_REPLY"),
        }
        let result = f();
        std::env::remove_var("PI_FD_REPLY");
        result
    }

    fn query<'a>(
        text: &'a str,
        commands: &'a [SlashCommandSpec],
        models: &'a [String],
        cwd: &'a Path,
    ) -> SuggestionQuery<'a> {
        SuggestionQuery {
            text,
            commands,
            models,
            thinking_levels: &[],
            login_providers: &[],
            extra_providers: &[],
            cwd,
            force_path: false,
        }
    }

    #[test]
    fn completes_slash_commands_and_model_args() {
        let commands = vec![
            SlashCommandSpec {
                name: "model".into(),
                description: "Select model".into(),
                argument_hint: Some("<provider/model>".into()),
                argument_items: Vec::new(),
            },
            SlashCommandSpec {
                name: "thinking".into(),
                description: "Set thinking".into(),
                argument_hint: Some("<level>".into()),
                argument_items: Vec::new(),
            },
        ];
        let models = vec!["google/gemini".into(), "anthropic/sonnet".into()];
        let slash = suggestions(query("/mo", &commands, &models, Path::new("."))).unwrap();
        assert_eq!(slash.items[0].value, "model");
        assert!(slash.items[0]
            .description
            .as_ref()
            .unwrap()
            .contains("provider/model"));
        let args = suggestions(query("/model son", &commands, &models, Path::new("."))).unwrap();
        assert_eq!(args.items[0].value, "anthropic/sonnet");
        assert_eq!(
            apply_completion("/mo", 3, "/mo", &slash.items[0]),
            "/model "
        );
        let forced = suggestions(SuggestionQuery {
            force_path: true,
            ..query("/mo", &commands, &models, Path::new("."))
        })
        .unwrap();
        assert_eq!(forced.items[0].value, "model");
    }

    #[test]
    fn completes_at_paths_without_fd_via_readdir() {
        with_fd_reply(None, || {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join("readme.md"), "x").unwrap();
            std::fs::create_dir(dir.path().join("src")).unwrap();
            let found = suggestions(query("@re", &[], &[], dir.path())).unwrap();
            assert!(found.items.iter().any(|item| item.label == "readme.md"));
            let dirs = suggestions(query("@s", &[], &[], dir.path())).unwrap();
            assert!(dirs.items.iter().any(|item| item.label == "src/"));
        });
    }

    #[test]
    fn hash_is_not_a_file_attach() {
        with_fd_reply(None, || {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join("readme.md"), "x").unwrap();
            assert!(suggestions(query("#re", &[], &[], dir.path())).is_none());
        });
    }

    #[test]
    fn extra_hash_provider_completes_issues() {
        let extra = ExtraAutocompleteProvider {
            trigger_characters: vec!['#'],
            items: vec![
                AutocompleteItem {
                    value: "#42".into(),
                    label: "#42".into(),
                    description: Some("[open] Login crash".into()),
                },
                AutocompleteItem {
                    value: "#7".into(),
                    label: "#7".into(),
                    description: Some("[closed] Docs".into()),
                },
            ],
            live_query: None,
        };
        let found = suggestions(SuggestionQuery {
            extra_providers: std::slice::from_ref(&extra),
            ..query("#4", &[], &[], Path::new("."))
        })
        .unwrap();
        assert_eq!(found.items[0].value, "#42");
        assert_eq!(apply_completion("#4", 2, "#4", &found.items[0]), "#42");
        assert_eq!(
            autocomplete_debounce_ms(false, "#4", std::slice::from_ref(&extra)),
            ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS
        );
        assert_eq!(autocomplete_debounce_ms(true, "@re", &[]), 0);
        assert_eq!(
            autocomplete_debounce_ms(false, "@re", &[]),
            ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS
        );
    }

    #[test]
    fn extra_provider_live_query_is_called_with_editor_text() {
        let extra = ExtraAutocompleteProvider {
            trigger_characters: vec!['#'],
            items: Vec::new(),
            live_query: Some(LiveAutocompleteQuery(std::sync::Arc::new(|text| {
                vec![AutocompleteItem {
                    value: format!("live:{text}"),
                    label: format!("live:{text}"),
                    description: None,
                }]
            }))),
        };
        let found = suggestions(SuggestionQuery {
            extra_providers: std::slice::from_ref(&extra),
            ..query("#abc", &[], &[], Path::new("."))
        })
        .unwrap();
        assert_eq!(found.items[0].value, "live:#abc");
    }

    #[test]
    fn slash_get_argument_completions_from_spec() {
        let commands = vec![SlashCommandSpec {
            name: "commands".into(),
            description: "List commands".into(),
            argument_hint: None,
            argument_items: vec![
                AutocompleteItem {
                    value: "extension".into(),
                    label: "extension".into(),
                    description: None,
                },
                AutocompleteItem {
                    value: "prompt".into(),
                    label: "prompt".into(),
                    description: None,
                },
                AutocompleteItem {
                    value: "skill".into(),
                    label: "skill".into(),
                    description: None,
                },
            ],
        }];
        let found = suggestions(query("/commands ext", &commands, &[], Path::new("."))).unwrap();
        assert_eq!(found.items[0].value, "extension");
    }

    #[test]
    fn fd_fixture_finds_nested_file_like_ts() {
        with_fd_reply(Some("src/index.ts\nreadme.md\nsrc/\n"), || {
            let found = suggestions(query("@index", &[], &[], Path::new("."))).unwrap();
            assert!(found.items.iter().any(|item| item.value == "@src/index.ts"));
        });
    }

    #[test]
    fn fd_fixture_ranks_directories_and_excludes_git() {
        with_fd_reply(Some("src/\nsrc.txt\n.git/\n.git/config\n.pi/\n"), || {
            let found = suggestions(query("@src", &[], &[], Path::new("."))).unwrap();
            assert_eq!(found.items[0].value, "@src/");
            assert!(found.items.iter().any(|item| item.value == "@src.txt"));
            let all = suggestions(query("@", &[], &[], Path::new("."))).unwrap();
            assert!(all.items.iter().any(|item| item.value == "@.pi/"));
            assert!(!all
                .items
                .iter()
                .any(|item| item.value == "@.git" || item.value.starts_with("@.git/")));
        });
    }

    #[test]
    fn fd_path_query_joins_segments_and_quotes_spaces() {
        assert_eq!(build_fd_path_query("index"), "index");
        assert_eq!(
            build_fd_path_query("tui/src/auto"),
            "tui[\\\\/]src[\\\\/]auto"
        );
        assert_eq!(build_fd_path_query("components/"), "components[\\\\/]");
        with_fd_reply(Some("my folder/\nmy folder/test.txt\n"), || {
            let found = suggestions(query("@my", &[], &[], Path::new("."))).unwrap();
            assert!(found
                .items
                .iter()
                .any(|item| item.value == "@\"my folder/\""));
        });
    }

    #[test]
    fn fd_fixture_scoped_and_full_path_queries() {
        with_fd_reply(
            Some("packages/tui/src/autocomplete.ts\npackages/ai/src/autocomplete.ts\nsrc/components/Button.tsx\nsrc/utils/helpers.ts\n"),
            || {
                let scoped = suggestions(query("@tui/src/auto", &[], &[], Path::new("."))).unwrap();
                assert!(scoped
                    .items
                    .iter()
                    .any(|item| item.value == "@packages/tui/src/autocomplete.ts"));
                assert!(!scoped
                    .items
                    .iter()
                    .any(|item| item.value == "@packages/ai/src/autocomplete.ts"));
                let mid = suggestions(query("@components/", &[], &[], Path::new("."))).unwrap();
                assert!(mid
                    .items
                    .iter()
                    .any(|item| item.value == "@src/components/Button.tsx"));
                assert!(!mid
                    .items
                    .iter()
                    .any(|item| item.value == "@src/utils/helpers.ts"));
            },
        );
    }
}

#[cfg(test)]
mod slash_relevance_tests {
    use super::*;

    fn query<'a>(text: &'a str, commands: &'a [SlashCommandSpec]) -> SuggestionQuery<'a> {
        SuggestionQuery {
            text,
            commands,
            models: &[],
            thinking_levels: &[],
            login_providers: &[],
            extra_providers: &[],
            cwd: Path::new("."),
            force_path: false,
        }
    }

    #[test]
    fn model_argument_values_keep_display_spaces_out_of_provider_ids() {
        let models = vec!["openai-codex / gpt-6-astra".into()];
        let found = suggestions(SuggestionQuery {
            models: &models,
            ..query("/model ", &[])
        })
        .unwrap();
        assert_eq!(found.items[0].label, "openai-codex / gpt-6-astra");
        assert_eq!(found.items[0].value, "openai-codex/gpt-6-astra");
    }

    #[test]
    fn model_arguments_put_reference_models_before_older_models() {
        let models = [
            "gpt-5.3-codex",
            "gpt-5.5",
            "gpt-5.6-luna",
            "gpt-6-astra",
            "gpt-5.4-mini",
            "gpt-5.6-terra",
            "gpt-5.6-sol",
        ]
        .map(|id| format!("openai-codex / {id}"));
        let found = suggestions(SuggestionQuery {
            models: &models,
            ..query("/model ", &[])
        })
        .unwrap();
        assert_eq!(
            found
                .items
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            [
                "openai-codex/gpt-6-astra",
                "openai-codex/gpt-5.6-luna",
                "openai-codex/gpt-5.6-sol",
                "openai-codex/gpt-5.6-terra",
                "openai-codex/gpt-5.5",
                "openai-codex/gpt-5.4-mini",
                "openai-codex/gpt-5.3-codex"
            ]
        );
    }

    fn slash_spec(name: &str, description: &str) -> SlashCommandSpec {
        SlashCommandSpec {
            name: name.into(),
            description: description.into(),
            argument_hint: None,
            argument_items: Vec::new(),
        }
    }

    fn values(found: &AutocompleteSuggestions) -> Vec<&str> {
        found.items.iter().map(|item| item.value.as_str()).collect()
    }

    #[test]
    fn slash_me_keeps_memory_commands_and_drops_weak_subsequence_noise() {
        let commands = vec![
            slash_spec("memory-status", "Show vector-memory health"),
            slash_spec("memory-clear", "Clear local vector-memory records"),
            slash_spec("model", "Select model"),
            slash_spec("name", "Set session display name"),
            slash_spec("scoped-models", "Enable models for cycling"),
            slash_spec("resume", "Resume a different session"),
            slash_spec("graph-resume", "Resume an unfinished graph run"),
        ];
        let found = suggestions(query("/me", &commands)).unwrap();
        let values = values(&found);
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"memory-status"));
        assert!(values.contains(&"memory-clear"));
    }

    #[test]
    fn slash_prefix_and_segment_prefix_rank_by_name_strength() {
        let commands = vec![
            slash_spec("scoped-models", "Enable models"),
            slash_spec("model", "Select model"),
            slash_spec("graph-resume", "Resume graph"),
            slash_spec("resume", "Resume session"),
        ];
        let mo = suggestions(query("/mo", &commands)).unwrap();
        assert_eq!(mo.items[0].value, "model");

        let res = suggestions(query("/res", &commands)).unwrap();
        let values = values(&res);
        let resume = values.iter().position(|value| *value == "resume").unwrap();
        let graph_resume = values
            .iter()
            .position(|value| *value == "graph-resume")
            .unwrap();
        assert!(resume < graph_resume);
    }

    #[test]
    fn slash_typo_tolerance_recovers_model() {
        let commands = vec![
            slash_spec("memory-status", "Memory status"),
            slash_spec("model", "Select model"),
            slash_spec("reload", "Reload configuration"),
        ];
        let found = suggestions(query("/mdoel", &commands)).unwrap();
        assert_eq!(found.items[0].value, "model");
    }

    #[test]
    fn slash_name_relevance_beats_description_only_hits() {
        let commands = vec![
            slash_spec("memory-status", "Show records"),
            slash_spec("unrelated", "Memory status dashboard and record counts"),
        ];
        let found = suggestions(query("/memory", &commands)).unwrap();
        assert_eq!(values(&found), vec!["memory-status"]);
    }

    #[test]
    fn slash_weak_fuzzy_matches_below_threshold_are_excluded() {
        let commands = vec![
            slash_spec("memory-clear", "Clear memory"),
            slash_spec("model", "Select model"),
            slash_spec("resume", "Resume session"),
            slash_spec("graph-resume", "Resume graph"),
        ];
        let found = suggestions(query("/me", &commands)).unwrap();
        assert_eq!(values(&found), vec!["memory-clear"]);
    }

    #[test]
    fn slash_argument_completion_keeps_existing_fuzzy_behavior() {
        let commands = vec![SlashCommandSpec {
            name: "commands".into(),
            description: "List commands".into(),
            argument_hint: None,
            argument_items: vec![
                AutocompleteItem {
                    value: "extension".into(),
                    label: "extension".into(),
                    description: None,
                },
                AutocompleteItem {
                    value: "prompt".into(),
                    label: "prompt".into(),
                    description: None,
                },
                AutocompleteItem {
                    value: "skill".into(),
                    label: "skill".into(),
                    description: None,
                },
            ],
        }];
        let found = suggestions(query("/commands etn", &commands)).unwrap();
        assert_eq!(found.items[0].value, "extension");
    }

    #[test]
    fn slash_external_templates_skills_and_extensions_still_participate() {
        let commands = vec![
            slash_spec("deploy-extension", "Extension command"),
            slash_spec("review-template", "Prompt template"),
            slash_spec("skill:rust", "Skill command"),
        ];
        for (typed, expected) in [
            ("/dep", "deploy-extension"),
            ("/rev", "review-template"),
            ("/skill:r", "skill:rust"),
        ] {
            let found = suggestions(query(typed, &commands)).unwrap();
            assert_eq!(found.items[0].value, expected);
        }
    }

    #[test]
    fn slash_results_are_bounded_even_when_many_names_are_relevant() {
        let commands = (0..64)
            .map(|index| slash_spec(&format!("memory-{index:02}"), "Memory command"))
            .collect::<Vec<_>>();
        let found = suggestions(query("/mem", &commands)).unwrap();
        assert!(found.items.len() <= 20, "{} results", found.items.len());
    }
}

#[cfg(test)]
mod slash_source_visibility_tests {
    use super::*;

    #[test]
    fn empty_slash_query_keeps_late_extension_template_and_skill_sources_reachable() {
        let mut commands = (0..24)
            .map(|index| SlashCommandSpec {
                name: format!("builtin-{index:02}"),
                description: "Builtin command".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            })
            .collect::<Vec<_>>();
        commands.extend([
            SlashCommandSpec {
                name: "extension:deploy".into(),
                description: "Extension command".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            },
            SlashCommandSpec {
                name: "prompt:review".into(),
                description: "Prompt template".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            },
            SlashCommandSpec {
                name: "skill:rust".into(),
                description: "Skill command".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            },
        ]);
        let found = suggestions(SuggestionQuery {
            text: "/",
            commands: &commands,
            models: &[],
            thinking_levels: &[],
            login_providers: &[],
            extra_providers: &[],
            cwd: Path::new("."),
            force_path: false,
        })
        .unwrap();
        let values = found
            .items
            .iter()
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>();
        assert!(values.contains(&"extension:deploy"));
        assert!(values.contains(&"prompt:review"));
        assert!(values.contains(&"skill:rust"));
    }
}

#[cfg(test)]
mod slash_narrow_render_tests {
    use super::*;
    use crate::davinci::model::Model;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::views::chrome;

    #[test]
    fn relevant_slash_list_stays_bounded_and_renders_at_narrow_widths() {
        let commands = (0..64)
            .map(|index| SlashCommandSpec {
                name: format!("memory-{index:02}"),
                description: "A deliberately long memory command description".into(),
                argument_hint: None,
                argument_items: Vec::new(),
            })
            .collect::<Vec<_>>();

        for width in [0u16, 1, 8, 20, 32] {
            let mut model = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, false),
                width,
                24,
                false,
            );
            model.slash_commands = commands.clone();
            model.type_char("/mem");
            assert!(model.suggestions.as_ref().unwrap().items.len() <= MAX_SLASH_SUGGESTIONS);
            let rows = chrome::suggestions(&model);
            assert!(rows.len() <= MAX_SLASH_SUGGESTIONS + 4);
        }
    }
}
