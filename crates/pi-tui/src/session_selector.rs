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
            items.retain(|item| item.name.as_ref().is_some_and(|name| !name.is_empty()));
        }
        if !self.query.is_empty() {
            items.retain(|item| item_matches_query(item, &self.query));
        }
        match self.sort_mode {
            SortMode::Recent => items.sort_by(|a, b| b.modified_at.cmp(&a.modified_at)),
            SortMode::Relevance if !self.query.is_empty() => items.sort_by(|a, b| {
                let a_score = fuzzy_match(&self.query, a.name.as_deref().unwrap_or(&a.id)).score;
                let b_score = fuzzy_match(&self.query, b.name.as_deref().unwrap_or(&b.id)).score;
                b_score
                    .partial_cmp(&a_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
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

fn item_matches_query(item: &SessionItem, query: &str) -> bool {
    let hay = format!(
        "{} {} {} {}",
        item.id,
        item.name.as_deref().unwrap_or(""),
        item.path,
        item.cwd
    );
    if let Some(pattern) = query.strip_prefix("re:") {
        return regex_is_match(pattern.trim(), &hay);
    }
    if query.len() >= 2 && query.starts_with('"') && query.ends_with('"') {
        let phrase = &query[1..query.len() - 1];
        return hay.to_lowercase().contains(&phrase.to_lowercase());
    }
    fuzzy_match(query, &hay).matches || hay.to_lowercase().contains(&query.to_lowercase())
}

fn regex_is_match(pattern: &str, text: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    match fancy_regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
    {
        Ok(regex) => regex.is_match(text).unwrap_or(false),
        Err(_) => false,
    }
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
            },
            SessionItem {
                id: "bbb".into(),
                name: None,
                path: "/tmp/bbb.jsonl".into(),
                cwd: "/work".into(),
                modified_at: 10,
                parent_id: Some("aaa".into()),
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
}
