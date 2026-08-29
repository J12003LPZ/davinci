//! Session selector matching TS `session-selector.ts`.

use crate::fuzzy::fuzzy_match;
use crate::render::Component;

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
}

#[derive(Debug, Clone)]
pub struct SessionSelector {
    items: Vec<SessionItem>,
    filtered: Vec<SessionItem>,
    pub selected: usize,
    pub sort_mode: SortMode,
    pub show_path: bool,
    pub query: String,
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
            rename: None,
        };
        selector.apply_filter();
        selector
    }

    pub fn selected_id(&self) -> Option<String> {
        self.filtered.get(self.selected).map(|item| item.id.clone())
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
            "\x7f" | "\x08" => {
                self.query.pop();
                self.apply_filter();
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
        if !self.query.is_empty() {
            items.retain(|item| {
                let hay = format!(
                    "{} {} {} {}",
                    item.id,
                    item.name.as_deref().unwrap_or(""),
                    item.path,
                    item.cwd
                );
                fuzzy_match(&self.query, &hay).matches
                    || hay.to_lowercase().contains(&self.query.to_lowercase())
            });
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
            "  Sessions".into(),
            format!(
                "  {} · path {path_state} · ctrl+p path · ctrl+s sort · ctrl+r rename",
                self.sort_mode.label()
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
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
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
    fn path_sort_and_rename_match_ts() {
        let mut selector = SessionSelector::new(sample());
        let rendered = selector.render(80).join("\n");
        assert!(rendered.contains("Sessions"));
        assert!(rendered.contains("Threaded"));
        assert!(rendered.contains("path (off)"));
        assert!(rendered.contains("ctrl+p path"));
        assert!(rendered.contains("ctrl+s sort"));
        assert!(rendered.contains("ctrl+r rename"));
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
}
