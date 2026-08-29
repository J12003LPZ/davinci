//! Session selector matching TS `session-selector.ts`.

use crate::fuzzy::fuzzy_match;
use crate::render::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionScope {
    Current,
    All,
}

impl SessionScope {
    pub fn cycle(self) -> Self {
        match self {
            Self::Current => Self::All,
            Self::All => Self::Current,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "Current Folder",
            Self::All => "All",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFilter {
    All,
    Named,
}

impl NameFilter {
    pub fn cycle(self) -> Self {
        match self {
            Self::All => Self::Named,
            Self::Named => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Named => "named",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Threaded,
    Recent,
    Relevance,
}

impl SortMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Threaded => "threaded",
            Self::Recent => "recent",
            Self::Relevance => "relevance",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Threaded => Self::Recent,
            Self::Recent => Self::Relevance,
            Self::Relevance => Self::Threaded,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Threaded => "Threaded",
            Self::Recent => "Recent",
            Self::Relevance => "Fuzzy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionItem {
    pub id: String,
    pub name: Option<String>,
    pub path: String,
    pub cwd: String,
    pub modified_at: u64,
    pub parent_id: Option<String>,
    pub all_messages_text: String,
}

impl SessionItem {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            path: String::new(),
            cwd: String::new(),
            modified_at: 0,
            parent_id: None,
            all_messages_text: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSelectorAction {
    None,
    Select(String),
    Cancel,
    Rename { id: String, name: String },
    Delete { id: String, path: String },
}

#[derive(Debug, Clone)]
pub struct SessionSelector {
    items: Vec<SessionItem>,
    filtered: Vec<SessionItem>,
    pub selected: usize,
    pub sort_mode: SortMode,
    pub show_path: bool,
    pub query: String,
    pub scope: SessionScope,
    pub name_filter: NameFilter,
    pub current_cwd: String,
    confirming_delete: bool,
    rename: Option<(String, String)>,
}

impl SessionSelector {
    pub fn new(items: Vec<SessionItem>) -> Self {
        let mut selector = Self {
            items,
            filtered: Vec::new(),
            selected: 0,
            sort_mode: SortMode::Threaded,
            show_path: false,
            query: String::new(),
            scope: SessionScope::Current,
            name_filter: NameFilter::All,
            current_cwd: String::new(),
            confirming_delete: false,
            rename: None,
        };
        selector.apply_filter();
        selector
    }

    pub fn selected_id(&self) -> Option<String> {
        self.filtered.get(self.selected).map(|item| item.id.clone())
    }

    pub fn set_cwd(&mut self, cwd: impl Into<String>) {
        self.current_cwd = cwd.into();
        self.apply_filter();
    }

    pub fn remove(&mut self, id: &str) {
        self.items.retain(|item| item.id != id);
        self.confirming_delete = false;
        self.apply_filter();
    }

    pub fn apply_rename(&mut self, id: &str, name: &str) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.name = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
        }
        self.apply_filter();
    }

    pub fn handle_key(&mut self, data: &str) -> SessionSelectorAction {
        if self.confirming_delete {
            return match data {
                "\r" | "\n" => {
                    self.confirming_delete = false;
                    self.filtered
                        .get(self.selected)
                        .map(|item| SessionSelectorAction::Delete {
                            id: item.id.clone(),
                            path: item.path.clone(),
                        })
                        .unwrap_or(SessionSelectorAction::None)
                }
                "\x1b" => {
                    self.confirming_delete = false;
                    SessionSelectorAction::None
                }
                _ => SessionSelectorAction::None,
            };
        }
        if let Some((id, mut buffer)) = self.rename.take() {
            return match data {
                "\r" | "\n" => SessionSelectorAction::Rename {
                    id,
                    name: buffer.trim().to_string(),
                },
                "\x1b" => SessionSelectorAction::None,
                "\x7f" | "\x08" => {
                    buffer.pop();
                    self.rename = Some((id, buffer));
                    SessionSelectorAction::None
                }
                other if !other.chars().any(char::is_control) => {
                    buffer.push_str(other);
                    self.rename = Some((id, buffer));
                    SessionSelectorAction::None
                }
                _ => {
                    self.rename = Some((id, buffer));
                    SessionSelectorAction::None
                }
            };
        }
        match data {
            "\x1b[A" | "k" => {
                self.move_sel(-1);
                SessionSelectorAction::None
            }
            "\x1b[B" | "j" => {
                self.move_sel(1);
                SessionSelectorAction::None
            }
            "\r" | "\n" => self
                .selected_id()
                .map(SessionSelectorAction::Select)
                .unwrap_or(SessionSelectorAction::None),
            "\x1b" => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.apply_filter();
                    SessionSelectorAction::None
                } else {
                    SessionSelectorAction::Cancel
                }
            }
            "\x10" => {
                self.show_path = !self.show_path;
                SessionSelectorAction::None
            }
            "\x13" => {
                self.sort_mode = self.sort_mode.cycle();
                self.apply_filter();
                SessionSelectorAction::None
            }
            "\x12" => {
                if let Some(id) = self.selected_id() {
                    let current = self
                        .filtered
                        .get(self.selected)
                        .and_then(|item| item.name.clone())
                        .unwrap_or_default();
                    self.rename = Some((id, current));
                }
                SessionSelectorAction::None
            }
            "\t" => {
                self.scope = self.scope.cycle();
                self.apply_filter();
                SessionSelectorAction::None
            }
            "\x0e" => {
                self.name_filter = self.name_filter.cycle();
                self.apply_filter();
                SessionSelectorAction::None
            }
            "\x04" => {
                self.confirming_delete = self.selected_id().is_some();
                SessionSelectorAction::None
            }
            "\x1b[3;5~" => {
                if self.query.is_empty() {
                    self.confirming_delete = self.selected_id().is_some();
                } else {
                    self.query.pop();
                    self.apply_filter();
                }
                SessionSelectorAction::None
            }
            "\x7f" | "\x08" => {
                if self.query.is_empty() && data == "\x08" {
                    self.confirming_delete = self.selected_id().is_some();
                } else {
                    self.query.pop();
                    self.apply_filter();
                }
                SessionSelectorAction::None
            }
            other if !other.chars().any(char::is_control) => {
                self.query.push_str(other);
                self.apply_filter();
                SessionSelectorAction::None
            }
            _ => SessionSelectorAction::None,
        }
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    fn apply_filter(&mut self) {
        let last = self.selected_id();
        let mut items = self.items.clone();
        if self.scope == SessionScope::Current && !self.current_cwd.is_empty() {
            items.retain(|item| item.cwd == self.current_cwd);
        }
        if self.name_filter == NameFilter::Named {
            items.retain(has_session_name);
        }
        let parsed = if self.query.trim().is_empty() {
            None
        } else {
            Some(parse_search_query(&self.query))
        };
        if let Some(parsed) = &parsed {
            if parsed.error.is_some() {
                items.clear();
            } else {
                items.retain(|item| match_session(item, parsed).matches);
            }
        }
        match self.sort_mode {
            SortMode::Recent if parsed.is_none() => {
                items.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
            }
            SortMode::Recent => {}
            SortMode::Relevance | SortMode::Threaded if parsed.is_some() => {
                let parsed = parsed.as_ref().expect("parsed query");
                items.sort_by(|a, b| {
                    let a_score = match_session(a, parsed).score;
                    let b_score = match_session(b, parsed).score;
                    a_score
                        .partial_cmp(&b_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.modified_at.cmp(&a.modified_at))
                });
            }
            SortMode::Threaded | SortMode::Relevance => {
                items.sort_by(|a, b| match (&a.parent_id, &b.parent_id) {
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    _ => b.modified_at.cmp(&a.modified_at),
                });
            }
        }
        self.filtered = items;
        if let Some(id) = last {
            if let Some(index) = self.filtered.iter().position(|item| item.id == id) {
                self.selected = index;
            } else {
                self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
            }
        } else {
            self.selected = 0;
        }
    }
}

impl Component for SessionSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let path_state = if self.show_path { "(on)" } else { "(off)" };
        let mut lines = vec![
            format!("  Resume Session ({})", self.scope.label()),
            format!(
                "  {} · path {path_state} · {} · named {} · ctrl+p path · ctrl+s sort · ctrl+r rename · ctrl+n named · ctrl+d delete · tab scope",
                self.sort_mode.label(),
                self.scope.label(),
                self.name_filter.label()
            ),
        ];
        if self.query.is_empty() {
            lines.push("  Type to search:".into());
        } else {
            lines.push(format!("  Type to search: {}", self.query));
        }
        if self.filtered.is_empty() {
            lines.push("  No sessions found".into());
            return lines;
        }
        for (index, item) in self.filtered.iter().enumerate() {
            let prefix = if index == self.selected { "> " } else { "  " };
            let name = item
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&item.id);
            let mut line = format!("{prefix}{name}");
            if self.show_path {
                line.push_str("  ");
                line.push_str(&item.path);
            }
            if crate::render::visible_width(&line) > width {
                line = line.chars().take(width).collect();
            }
            lines.push(line);
        }
        if let Some((_, buffer)) = &self.rename {
            lines.push("  Rename (empty to clear):".into());
            lines.push(format!("  {buffer}"));
            lines.push("  enter save  escape cancel".into());
        }
        if self.confirming_delete {
            lines.push("  Delete selected session?".into());
            lines.push("  enter confirm  escape cancel".into());
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchToken {
    pub kind: SearchTokenKind,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTokenKind {
    Fuzzy,
    Phrase,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSearchQuery {
    pub mode: SearchMode,
    pub tokens: Vec<SearchToken>,
    pub regex: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Tokens,
    Regex,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchResult {
    pub matches: bool,
    pub score: f64,
}

fn has_session_name(item: &SessionItem) -> bool {
    item.name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
}

fn session_search_text(item: &SessionItem) -> String {
    format!(
        "{} {} {} {}",
        item.id,
        item.name.as_deref().unwrap_or(""),
        item.all_messages_text,
        item.cwd
    )
}

fn normalize_whitespace_lower(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// TS `parseSearchQuery`.
pub fn parse_search_query(query: &str) -> ParsedSearchQuery {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return ParsedSearchQuery {
            mode: SearchMode::Tokens,
            tokens: Vec::new(),
            regex: None,
            error: None,
        };
    }
    if let Some(pattern) = trimmed.strip_prefix("re:") {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return ParsedSearchQuery {
                mode: SearchMode::Regex,
                tokens: Vec::new(),
                regex: None,
                error: Some("Empty regex".into()),
            };
        }
        return match js_regex(pattern) {
            Ok(_) => ParsedSearchQuery {
                mode: SearchMode::Regex,
                tokens: Vec::new(),
                regex: Some(pattern.to_string()),
                error: None,
            },
            Err(err) => ParsedSearchQuery {
                mode: SearchMode::Regex,
                tokens: Vec::new(),
                regex: None,
                error: Some(err.to_string()),
            },
        };
    }

    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    let mut had_unclosed_quote = false;

    let flush = |kind: SearchTokenKind, buf: &mut String, tokens: &mut Vec<SearchToken>| {
        let value = buf.trim().to_string();
        buf.clear();
        if !value.is_empty() {
            tokens.push(SearchToken { kind, value });
        }
    };

    for ch in trimmed.chars() {
        if ch == '"' {
            if in_quote {
                flush(SearchTokenKind::Phrase, &mut buf, &mut tokens);
                in_quote = false;
            } else {
                flush(SearchTokenKind::Fuzzy, &mut buf, &mut tokens);
                in_quote = true;
            }
            continue;
        }
        if !in_quote && ch.is_whitespace() {
            flush(SearchTokenKind::Fuzzy, &mut buf, &mut tokens);
            continue;
        }
        buf.push(ch);
    }

    if in_quote {
        had_unclosed_quote = true;
    }
    if had_unclosed_quote {
        return ParsedSearchQuery {
            mode: SearchMode::Tokens,
            tokens: trimmed
                .split_whitespace()
                .filter(|token| !token.is_empty())
                .map(|token| SearchToken {
                    kind: SearchTokenKind::Fuzzy,
                    value: token.to_string(),
                })
                .collect(),
            regex: None,
            error: None,
        };
    }
    flush(SearchTokenKind::Fuzzy, &mut buf, &mut tokens);
    ParsedSearchQuery {
        mode: SearchMode::Tokens,
        tokens,
        regex: None,
        error: None,
    }
}

fn js_regex(pattern: &str) -> Result<fancy_regex::Regex, String> {
    // JS `new RegExp(pattern, "i")` — `(?i)` matches that better than
    // RegexBuilder::case_insensitive, which breaks `\\b` in fancy-regex.
    fancy_regex::Regex::new(&format!("(?i){pattern}")).map_err(|err| err.to_string())
}

fn compiled_regex(pattern: &str) -> Option<fancy_regex::Regex> {
    js_regex(pattern).ok()
}

/// TS `matchSession`.
pub fn match_session(item: &SessionItem, parsed: &ParsedSearchQuery) -> MatchResult {
    let text = session_search_text(item);
    if parsed.mode == SearchMode::Regex {
        let Some(pattern) = parsed.regex.as_deref() else {
            return MatchResult {
                matches: false,
                score: 0.0,
            };
        };
        let Some(regex) = compiled_regex(pattern) else {
            return MatchResult {
                matches: false,
                score: 0.0,
            };
        };
        return match regex.find(&text) {
            Ok(Some(found)) => MatchResult {
                matches: true,
                score: found.start() as f64 * 0.1,
            },
            _ => MatchResult {
                matches: false,
                score: 0.0,
            },
        };
    }
    if parsed.tokens.is_empty() {
        return MatchResult {
            matches: true,
            score: 0.0,
        };
    }
    let mut total_score = 0.0;
    let mut normalized_text: Option<String> = None;
    for token in &parsed.tokens {
        if token.kind == SearchTokenKind::Phrase {
            if normalized_text.is_none() {
                normalized_text = Some(normalize_whitespace_lower(&text));
            }
            let hay = normalized_text.as_deref().unwrap_or("");
            let phrase = normalize_whitespace_lower(&token.value);
            if phrase.is_empty() {
                continue;
            }
            let Some(idx) = hay.find(&phrase) else {
                return MatchResult {
                    matches: false,
                    score: 0.0,
                };
            };
            total_score += idx as f64 * 0.1;
            continue;
        }
        let m = fuzzy_match(&token.value, &text);
        if !m.matches {
            return MatchResult {
                matches: false,
                score: 0.0,
            };
        }
        total_score += m.score;
    }
    MatchResult {
        matches: true,
        score: total_score,
    }
}

#[cfg(test)]
fn regex_is_match(pattern: &str, text: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    compiled_regex(pattern)
        .map(|regex| regex.is_match(text).unwrap_or(false))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<SessionItem> {
        vec![
            SessionItem {
                id: "aaa".into(),
                name: Some("alpha".into()),
                path: "/tmp/aaa.jsonl".into(),
                cwd: "/work".into(),
                modified_at: 20,
                parent_id: None,
                all_messages_text: String::new(),
            },
            SessionItem {
                id: "bbb".into(),
                name: None,
                path: "/tmp/bbb.jsonl".into(),
                cwd: "/work".into(),
                modified_at: 10,
                parent_id: Some("aaa".into()),
                all_messages_text: String::new(),
            },
        ]
    }

    #[test]
    fn js_like_regex_supports_flags_lookaround_and_unicode() {
        assert!(regex_is_match("^aaa$", "aaa"));
        assert!(regex_is_match("^AAA$", "aaa"));
        assert!(!regex_is_match("^aaa$", "baaa"));
        assert!(regex_is_match("foo|bar", "xxbarxx"));
        assert!(regex_is_match("[ab]{3}", "xaaab"));
        assert!(regex_is_match(r"\d+", "id-12"));
        assert!(!regex_is_match(r"\d+", "abc"));
        assert!(regex_is_match("[^x]+", "abc"));
        assert!(regex_is_match("(ab)+c", "ababc"));
        assert!(regex_is_match(r"(?=aaa)aaa", "xxxaaa"));
        assert!(!regex_is_match(r"(?!aaa)aaa", "aaa"));
        assert!(regex_is_match(r"\p{L}+", "alpha"));
        assert!(!regex_is_match(r"\p{L}+", "123"));
        assert!(!regex_is_match("", "aaa"));
        assert!(!regex_is_match("(", "aaa"));
    }

    #[test]
    fn path_sort_and_rename_match_ts() {
        let mut selector = SessionSelector::new(sample());
        let rendered = selector.render(80).join("\n");
        assert!(rendered.contains("Resume Session (Current Folder)"));
        assert!(rendered.contains("Threaded"));
        assert!(rendered.contains("path (off)"));
        assert!(rendered.contains("ctrl+p path"));
        assert!(rendered.contains("ctrl+s sort"));
        assert!(rendered.contains("ctrl+r rename"));
        assert!(rendered.contains("ctrl+n named"));
        assert!(rendered.contains("ctrl+d delete"));
        assert!(rendered.contains("tab scope"));
        assert!(!rendered.contains("/tmp/aaa.jsonl"));
        selector.handle_key("\x10");
        assert!(selector.show_path);
        assert!(selector.render(80).join("\n").contains("path (on)"));
        assert!(selector.render(80).join("\n").contains("/tmp/aaa.jsonl"));
        selector.handle_key("\x13");
        assert_eq!(selector.sort_mode, SortMode::Recent);
        assert!(selector.render(80).join("\n").contains("Recent"));
        selector.handle_key("\x13");
        assert_eq!(selector.sort_mode, SortMode::Relevance);
        assert!(selector.render(80).join("\n").contains("Fuzzy"));
        selector.selected = 0;
        selector.handle_key("\x12");
        assert!(selector
            .render(80)
            .join("\n")
            .contains("Rename (empty to clear):"));
        for _ in 0..5 {
            selector.handle_key("\x7f");
        }
        selector.handle_key("renamed");
        assert_eq!(
            selector.handle_key("\r"),
            SessionSelectorAction::Rename {
                id: selector.filtered[0].id.clone(),
                name: "renamed".into(),
            }
        );
    }

    #[test]
    fn scope_named_delete_and_search_match_ts() {
        let mut items = sample();
        items.push(SessionItem {
            id: "ccc".into(),
            name: Some("other".into()),
            path: "/tmp/ccc.jsonl".into(),
            cwd: "/elsewhere".into(),
            modified_at: 5,
            parent_id: None,
            all_messages_text: String::new(),
        });
        let mut selector = SessionSelector::new(items);
        selector.set_cwd("/work");
        assert_eq!(selector.filtered.len(), 2);
        selector.handle_key("\t");
        assert_eq!(selector.scope, SessionScope::All);
        assert!(selector
            .render(80)
            .join("\n")
            .contains("Resume Session (All)"));
        assert_eq!(selector.filtered.len(), 3);
        selector.handle_key("\x0e");
        assert_eq!(selector.name_filter, NameFilter::Named);
        assert_eq!(selector.filtered.len(), 2);
        selector.handle_key("\t");
        assert_eq!(selector.scope, SessionScope::Current);
        assert_eq!(selector.filtered.len(), 1);
        selector.query.clear();
        selector.name_filter = NameFilter::All;
        selector.scope = SessionScope::All;
        selector.apply_filter();
        selector.handle_key("re:aa+");
        assert_eq!(selector.filtered.len(), 1);
        assert_eq!(selector.filtered[0].id, "aaa");
        selector.query.clear();
        selector.apply_filter();
        selector.handle_key("re:^aaa ");
        assert_eq!(selector.filtered.len(), 1);
        selector.query.clear();
        selector.apply_filter();
        selector.handle_key("re:bbb|other");
        assert_eq!(selector.filtered.len(), 2);
        selector.query.clear();
        selector.apply_filter();
        selector.handle_key("re:[ab]{3}");
        assert_eq!(selector.filtered.len(), 2);
        selector.query.clear();
        selector.apply_filter();
        selector.handle_key(r"re:\d+");
        assert_eq!(selector.filtered.len(), 0);
        selector.query.clear();
        selector.apply_filter();
        selector.handle_key("\"other\"");
        assert_eq!(selector.filtered.len(), 1);
        selector.query.clear();
        selector.apply_filter();
        selector.selected = 0;
        selector.handle_key("\x04");
        assert!(selector
            .render(80)
            .join("\n")
            .contains("Delete selected session?"));
        assert_eq!(
            selector.handle_key("\r"),
            SessionSelectorAction::Delete {
                id: selector.filtered[0].id.clone(),
                path: selector.filtered[0].path.clone(),
            }
        );
    }

    fn message_item(id: &str, modified_at: u64, all_messages_text: &str) -> SessionItem {
        SessionItem {
            id: id.into(),
            name: None,
            path: format!("/tmp/{id}.jsonl"),
            cwd: String::new(),
            modified_at,
            parent_id: None,
            all_messages_text: all_messages_text.into(),
        }
    }

    #[test]
    fn search_corpus_matches_ts_session_selector_search() {
        let mut selector = SessionSelector::new(vec![
            message_item("a", 1, "node\n\n   cve was discussed"),
            message_item("b", 2, "node something else"),
        ]);
        selector.scope = SessionScope::All;
        selector.sort_mode = SortMode::Recent;
        selector.query.clear();
        selector.apply_filter();
        selector.handle_key("\"node cve\"");
        assert_eq!(
            selector
                .filtered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );

        let mut selector = SessionSelector::new(vec![
            message_item("a", 2, "Brave is great"),
            message_item("b", 3, "bravery is not the same"),
        ]);
        selector.scope = SessionScope::All;
        selector.sort_mode = SortMode::Recent;
        selector.handle_key(r"re:\bbrave\b");
        assert_eq!(selector.filtered.len(), 1);
        assert_eq!(selector.filtered[0].id, "a");

        let mut selector = SessionSelector::new(vec![
            message_item("newer", 3, "brave"),
            message_item("older", 1, "brave"),
            message_item("nomatch", 4, "something else"),
        ]);
        selector.scope = SessionScope::All;
        selector.sort_mode = SortMode::Recent;
        selector.handle_key("\"brave\"");
        assert_eq!(
            selector
                .filtered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );

        let mut selector = SessionSelector::new(vec![
            message_item("late", 3, "xxxx brave"),
            message_item("early", 1, "brave xxxx"),
        ]);
        selector.scope = SessionScope::All;
        selector.sort_mode = SortMode::Relevance;
        selector.handle_key("\"brave\"");
        assert_eq!(
            selector
                .filtered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["early", "late"]
        );

        let mut selector = SessionSelector::new(vec![
            message_item("newer", 3, "brave"),
            message_item("older", 1, "brave"),
        ]);
        selector.scope = SessionScope::All;
        selector.sort_mode = SortMode::Relevance;
        selector.handle_key("\"brave\"");
        assert_eq!(
            selector
                .filtered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );

        let mut selector = SessionSelector::new(vec![message_item("a", 1, "brave")]);
        selector.scope = SessionScope::All;
        selector.handle_key("re:(");
        assert!(selector.filtered.is_empty());

        let parsed = parse_search_query(r#"foo "node cve" bar"#);
        assert_eq!(parsed.mode, SearchMode::Tokens);
        assert_eq!(parsed.tokens.len(), 3);
        assert_eq!(parsed.tokens[0].value, "foo");
        assert_eq!(parsed.tokens[1].value, "node cve");
        assert_eq!(parsed.tokens[2].value, "bar");
        let hit = SessionItem {
            id: "hit".into(),
            name: Some("foo".into()),
            path: String::new(),
            cwd: String::new(),
            modified_at: 1,
            parent_id: None,
            all_messages_text: "node\ncve bar".into(),
        };
        let miss = SessionItem {
            id: "miss".into(),
            name: Some("foo".into()),
            path: String::new(),
            cwd: String::new(),
            modified_at: 1,
            parent_id: None,
            all_messages_text: "node something bar".into(),
        };
        assert!(match_session(&hit, &parsed).matches);
        assert!(!match_session(&miss, &parsed).matches);

        let unclosed = parse_search_query(r#"foo "node cve"#);
        assert!(unclosed
            .tokens
            .iter()
            .all(|token| token.kind == SearchTokenKind::Fuzzy));
        assert_eq!(unclosed.tokens.len(), 3);

        let mut selector = SessionSelector::new(vec![
            SessionItem {
                id: "whitespace".into(),
                name: Some("   ".into()),
                path: String::new(),
                cwd: String::new(),
                modified_at: 1,
                parent_id: None,
                all_messages_text: "test".into(),
            },
            SessionItem {
                id: "named".into(),
                name: Some("Real Name".into()),
                path: String::new(),
                cwd: String::new(),
                modified_at: 3,
                parent_id: None,
                all_messages_text: "test".into(),
            },
        ]);
        selector.scope = SessionScope::All;
        selector.handle_key("\x0e");
        assert_eq!(selector.filtered.len(), 1);
        assert_eq!(selector.filtered[0].id, "named");
    }
}
