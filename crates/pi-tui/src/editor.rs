use crate::autocomplete::{
    ApplyLinesResult, AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions,
};
use crate::component::{wrap, Component};
use crate::keys::Key;
use std::rc::Rc;

const DEFAULT_TRIGGER_CHARACTERS: &[&str] = &["@", "#"];
const ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS: u64 = 20;
const AUTOCOMPLETE_MAX_VISIBLE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutocompleteMode {
    Regular,
    Force,
}

pub struct Editor {
    pub buffer: String,
    pub cursor: usize,
    pub history: Vec<String>,
    provider: Option<Rc<dyn AutocompleteProvider>>,
    trigger_characters: Vec<String>,
    autocomplete_mode: Option<AutocompleteMode>,
    autocomplete_items: Vec<AutocompleteItem>,
    autocomplete_selected: usize,
    autocomplete_prefix: String,
    autocomplete_max_visible: usize,
    pending: Option<PendingAutocomplete>,
    clock_ms: u64,
    sync_autocomplete: bool,
    undo_buffer: Option<(String, usize)>,
}

#[derive(Clone)]
struct PendingAutocomplete {
    force: bool,
    explicit_tab: bool,
    deadline_ms: u64,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            provider: None,
            trigger_characters: default_triggers(),
            autocomplete_mode: None,
            autocomplete_items: Vec::new(),
            autocomplete_selected: 0,
            autocomplete_prefix: String::new(),
            autocomplete_max_visible: AUTOCOMPLETE_MAX_VISIBLE,
            pending: None,
            clock_ms: 0,
            sync_autocomplete: false,
            undo_buffer: None,
        }
    }
}

fn default_triggers() -> Vec<String> {
    DEFAULT_TRIGGER_CHARACTERS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

impl Editor {
    pub fn set_autocomplete_provider(&mut self, provider: Rc<dyn AutocompleteProvider>) {
        self.set_trigger_characters(provider.trigger_characters());
        self.provider = Some(provider);
    }

    pub fn set_sync_autocomplete(&mut self, sync: bool) {
        self.sync_autocomplete = sync;
    }

    pub fn is_showing_autocomplete(&self) -> bool {
        self.autocomplete_mode.is_some()
    }

    pub fn autocomplete_items(&self) -> &[AutocompleteItem] {
        &self.autocomplete_items
    }

    pub fn insert(&mut self, ch: char) {
        self.insert_char(ch, false);
    }

    pub fn handle_key(&mut self, key: &Key) {
        if self.autocomplete_mode.is_some() {
            match key {
                Key::Escape => {
                    self.cancel_autocomplete();
                    return;
                }
                Key::Up => {
                    if self.autocomplete_selected > 0 {
                        self.autocomplete_selected -= 1;
                    }
                    return;
                }
                Key::Down => {
                    if self.autocomplete_selected + 1 < self.autocomplete_items.len() {
                        self.autocomplete_selected += 1;
                    }
                    return;
                }
                Key::Tab => {
                    self.apply_selected();
                    return;
                }
                Key::Enter => {
                    let _ = self.confirm_autocomplete();
                    return;
                }
                _ => {}
            }
        }

        match key {
            Key::Tab => self.handle_tab_completion(),
            Key::Char(c) => self.insert_char(*c, false),
            Key::Backspace => self.backspace(),
            Key::Enter => {
                if !self.buffer.trim().is_empty()
                    && self.history.last().map(String::as_str) != Some(self.buffer.as_str())
                {
                    self.history.push(self.buffer.clone());
                }
            }
            Key::Left => {
                self.move_left();
                self.update_autocomplete_after_cursor();
            }
            Key::Right => {
                self.move_right();
                self.update_autocomplete_after_cursor();
            }
            Key::Ctrl('-') => self.undo(),
            _ => {}
        }
    }

    /// Apply the highlighted item. Returns true when a slash completion should submit.
    pub fn confirm_autocomplete(&mut self) -> bool {
        if self.autocomplete_mode.is_none() {
            return true;
        }
        let slash = self.autocomplete_prefix.starts_with('/');
        self.apply_selected();
        slash
    }

    pub fn advance_autocomplete(&mut self, ms: u64) {
        self.clock_ms = self.clock_ms.saturating_add(ms);
        self.fire_due_autocomplete();
    }

    pub fn poll_autocomplete(&mut self) {
        if self.sync_autocomplete {
            self.fire_due_autocomplete();
        }
    }

    fn set_trigger_characters(&mut self, extra: &[String]) {
        let mut next = default_triggers();
        for character in extra {
            if character.chars().count() != 1
                || character == "/"
                || character.chars().all(char::is_whitespace)
                || next.iter().any(|existing| existing == character)
            {
                continue;
            }
            next.push(character.clone());
        }
        self.trigger_characters = next;
    }

    fn char_cursor(&self) -> usize {
        self.buffer.get(..self.cursor).unwrap_or("").chars().count()
    }

    fn set_char_cursor(&mut self, col: usize) {
        self.cursor = self
            .buffer
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(self.buffer.len());
    }

    fn lines(&self) -> Vec<String> {
        vec![self.buffer.clone()]
    }

    fn text_before_cursor(&self) -> String {
        self.buffer.get(..self.cursor).unwrap_or("").to_string()
    }

    fn insert_char(&mut self, ch: char, skip_autocomplete: bool) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        if skip_autocomplete {
            return;
        }
        if self.autocomplete_mode.is_none() {
            if ch == '/' && self.is_at_start_of_message() {
                self.try_trigger_autocomplete(false);
            } else if self.trigger_characters.iter().any(|t| t == &ch.to_string()) {
                let before = self.text_before_cursor();
                let char_before_symbol = before.chars().rev().nth(1);
                if before.chars().count() == 1
                    || char_before_symbol == Some(' ')
                    || char_before_symbol == Some('\t')
                {
                    self.try_trigger_autocomplete(false);
                }
            } else if is_word_char(ch) {
                let before = self.text_before_cursor();
                if self.is_in_slash_command_context(&before)
                    || matches_trigger_pattern(&before, &self.trigger_characters)
                {
                    self.try_trigger_autocomplete(false);
                }
            }
        } else {
            self.update_autocomplete();
        }
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.buffer[..self.cursor]
                .chars()
                .next_back()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor -= prev;
            self.buffer.drain(self.cursor..self.cursor + prev);
        }
        if self.autocomplete_mode.is_some() {
            self.update_autocomplete();
        } else if matches_trigger_pattern(&self.text_before_cursor(), &self.trigger_characters) {
            self.try_trigger_autocomplete(false);
        }
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.buffer[..self.cursor]
                .chars()
                .next_back()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor -= prev;
        }
    }

    fn move_right(&mut self) {
        if let Some(c) = self.buffer[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    fn undo(&mut self) {
        if let Some((buffer, cursor)) = self.undo_buffer.take() {
            self.buffer = buffer;
            self.cursor = cursor;
            self.cancel_autocomplete();
        }
    }

    fn is_at_start_of_message(&self) -> bool {
        let before = self.text_before_cursor();
        let trimmed = before.trim();
        trimmed.is_empty() || trimmed == "/"
    }

    fn is_in_slash_command_context(&self, text_before_cursor: &str) -> bool {
        text_before_cursor.trim_start().starts_with('/')
    }

    fn handle_tab_completion(&mut self) {
        if self.provider.is_none() {
            return;
        }
        let before = self.text_before_cursor();
        if self.is_in_slash_command_context(&before) && !before.trim_start().contains(' ') {
            self.request_autocomplete(false, true);
        } else {
            self.request_autocomplete(true, true);
        }
    }

    fn try_trigger_autocomplete(&mut self, explicit_tab: bool) {
        self.request_autocomplete(false, explicit_tab);
    }

    fn update_autocomplete(&mut self) {
        if let Some(mode) = self.autocomplete_mode {
            self.request_autocomplete(mode == AutocompleteMode::Force, false);
        }
    }

    fn update_autocomplete_after_cursor(&mut self) {
        if self.autocomplete_mode.is_some() {
            self.update_autocomplete();
        }
    }

    fn request_autocomplete(&mut self, force: bool, explicit_tab: bool) {
        let Some(provider) = self.provider.clone() else {
            return;
        };
        if force && !provider.should_trigger_file_completion(&self.lines(), 0, self.char_cursor()) {
            return;
        }
        self.cancel_pending();
        let debounce = self.debounce_ms(force, explicit_tab);
        if debounce > 0 && !self.sync_autocomplete {
            self.pending = Some(PendingAutocomplete {
                force,
                explicit_tab,
                deadline_ms: self.clock_ms.saturating_add(debounce),
            });
            return;
        }
        self.run_autocomplete(force, explicit_tab);
    }

    fn debounce_ms(&self, force: bool, explicit_tab: bool) -> u64 {
        if explicit_tab || force {
            return 0;
        }
        let before = self.text_before_cursor();
        if matches_debounce_pattern(&before, &self.trigger_characters) {
            ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS
        } else {
            0
        }
    }

    fn fire_due_autocomplete(&mut self) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        if self.clock_ms < pending.deadline_ms && !self.sync_autocomplete {
            return;
        }
        self.pending = None;
        self.run_autocomplete(pending.force, pending.explicit_tab);
    }

    fn cancel_pending(&mut self) {
        self.pending = None;
    }

    fn run_autocomplete(&mut self, force: bool, explicit_tab: bool) {
        let Some(provider) = self.provider.clone() else {
            return;
        };
        let lines = self.lines();
        let col = self.char_cursor();
        let Some(suggestions) = provider.get_suggestions(&lines, 0, col, force) else {
            self.cancel_autocomplete();
            return;
        };
        if suggestions.items.is_empty() {
            self.cancel_autocomplete();
            return;
        }
        if force && explicit_tab && suggestions.items.len() == 1 {
            self.push_undo();
            self.apply_item(&provider, &suggestions.items[0], &suggestions.prefix);
            self.clear_autocomplete_ui();
            return;
        }
        self.apply_suggestions(suggestions, force);
    }

    fn apply_suggestions(&mut self, suggestions: AutocompleteSuggestions, force: bool) {
        let selected = best_match_index(&suggestions.items, &suggestions.prefix)
            .unwrap_or(0)
            .min(suggestions.items.len().saturating_sub(1));
        self.autocomplete_prefix = suggestions.prefix;
        self.autocomplete_items = suggestions.items;
        self.autocomplete_selected = selected;
        self.autocomplete_mode = Some(if force {
            AutocompleteMode::Force
        } else {
            AutocompleteMode::Regular
        });
    }

    fn apply_selected(&mut self) {
        let Some(provider) = self.provider.clone() else {
            return;
        };
        let Some(item) = self
            .autocomplete_items
            .get(self.autocomplete_selected)
            .cloned()
        else {
            self.cancel_autocomplete();
            return;
        };
        let prefix = self.autocomplete_prefix.clone();
        self.push_undo();
        self.apply_item(&provider, &item, &prefix);
        self.cancel_autocomplete();
    }

    fn apply_item(
        &mut self,
        provider: &Rc<dyn AutocompleteProvider>,
        item: &AutocompleteItem,
        prefix: &str,
    ) {
        let ApplyLinesResult {
            lines, cursor_col, ..
        } = provider.apply_completion_lines(&self.lines(), 0, self.char_cursor(), item, prefix);
        self.buffer = lines.into_iter().next().unwrap_or_default();
        self.set_char_cursor(cursor_col);
    }

    fn push_undo(&mut self) {
        self.undo_buffer = Some((self.buffer.clone(), self.cursor));
    }

    fn cancel_autocomplete(&mut self) {
        self.cancel_pending();
        self.clear_autocomplete_ui();
    }

    fn clear_autocomplete_ui(&mut self) {
        self.autocomplete_mode = None;
        self.autocomplete_items.clear();
        self.autocomplete_selected = 0;
        self.autocomplete_prefix.clear();
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_'
}

fn matches_trigger_pattern(text: &str, triggers: &[String]) -> bool {
    let Some(last) = text.split([' ', '\t']).next_back() else {
        return false;
    };
    triggers.iter().any(|trigger| last.starts_with(trigger))
}

fn matches_debounce_pattern(text: &str, triggers: &[String]) -> bool {
    let without_at: Vec<&str> = triggers
        .iter()
        .map(String::as_str)
        .filter(|t| *t != "@")
        .collect();
    let Some(last) = text.split([' ', '\t']).next_back() else {
        return false;
    };
    if last.starts_with('@') {
        return true;
    }
    without_at.iter().any(|trigger| last.starts_with(trigger))
}

fn best_match_index(items: &[AutocompleteItem], prefix: &str) -> Option<usize> {
    if prefix.is_empty() {
        return None;
    }
    let mut first_prefix = None;
    for (i, item) in items.iter().enumerate() {
        if item.value == prefix {
            return Some(i);
        }
        if first_prefix.is_none() && item.value.starts_with(prefix) {
            first_prefix = Some(i);
        }
    }
    first_prefix
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = wrap(&self.buffer, width);
        if self.is_showing_autocomplete() {
            let visible = self
                .autocomplete_items
                .iter()
                .take(self.autocomplete_max_visible);
            for (i, item) in visible.enumerate() {
                let marker = if i == self.autocomplete_selected {
                    ">"
                } else {
                    " "
                };
                let mut line = match &item.description {
                    Some(desc) if !desc.is_empty() => {
                        format!("{marker} {}  {desc}", item.label)
                    }
                    _ => format!("{marker} {}", item.label),
                };
                if line.len() > width {
                    line.truncate(width);
                }
                lines.push(line);
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autocomplete::AutocompleteSuggestions;

    type ItemsFor = Box<dyn Fn(&str, bool) -> Option<AutocompleteSuggestions>>;

    struct MockProvider {
        force_only: bool,
        items_for: ItemsFor,
        trigger_characters: Vec<String>,
        calls: std::cell::Cell<u32>,
    }

    impl AutocompleteProvider for MockProvider {
        fn trigger_characters(&self) -> &[String] {
            &self.trigger_characters
        }

        fn get_suggestions(
            &self,
            lines: &[String],
            _cursor_line: usize,
            cursor_col: usize,
            force: bool,
        ) -> Option<AutocompleteSuggestions> {
            if self.force_only && !force {
                return None;
            }
            self.calls.set(self.calls.get() + 1);
            let text = lines.first().cloned().unwrap_or_default();
            let prefix: String = text.chars().take(cursor_col).collect();
            (self.items_for)(&prefix, force)
        }

        fn apply_completion_lines(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            item: &AutocompleteItem,
            prefix: &str,
        ) -> ApplyLinesResult {
            let line = lines.get(cursor_line).cloned().unwrap_or_default();
            let prefix_start = cursor_col.saturating_sub(prefix.chars().count());
            let before: String = line.chars().take(prefix_start).collect();
            let after: String = line.chars().skip(cursor_col).collect();
            let new_line = format!("{before}{}{after}", item.value);
            let mut out = lines.to_vec();
            if cursor_line >= out.len() {
                out.resize(cursor_line + 1, String::new());
            }
            out[cursor_line] = new_line;
            ApplyLinesResult {
                lines: out,
                cursor_line,
                cursor_col: prefix_start + item.value.chars().count(),
            }
        }
    }

    fn item(value: &str) -> AutocompleteItem {
        AutocompleteItem {
            value: value.into(),
            label: value.into(),
            description: None,
        }
    }

    #[test]
    fn editor_insert_and_backspace() {
        let mut editor = Editor::default();
        editor.handle_key(&Key::Char('a'));
        editor.handle_key(&Key::Char('b'));
        editor.handle_key(&Key::Backspace);
        assert_eq!(editor.buffer, "a");
    }

    #[test]
    fn tab_auto_applies_single_force_file_suggestion() {
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(Rc::new(MockProvider {
            force_only: true,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|prefix, force| {
                if force && prefix == "Work" {
                    Some(AutocompleteSuggestions {
                        items: vec![item("Workspace/")],
                        prefix: "Work".into(),
                    })
                } else {
                    None
                }
            }),
        }));
        for ch in ['W', 'o', 'r', 'k'] {
            editor.handle_key(&Key::Char(ch));
        }
        assert_eq!(editor.buffer, "Work");
        editor.handle_key(&Key::Tab);
        assert_eq!(editor.buffer, "Workspace/");
        assert!(!editor.is_showing_autocomplete());
        editor.handle_key(&Key::Ctrl('-'));
        assert_eq!(editor.buffer, "Work");
    }

    #[test]
    fn tab_shows_menu_for_multiple_force_suggestions() {
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(Rc::new(MockProvider {
            force_only: true,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|prefix, force| {
                if force && prefix == "src" {
                    Some(AutocompleteSuggestions {
                        items: vec![item("src/"), item("src.txt")],
                        prefix: "src".into(),
                    })
                } else {
                    None
                }
            }),
        }));
        for ch in ['s', 'r', 'c'] {
            editor.handle_key(&Key::Char(ch));
        }
        editor.handle_key(&Key::Tab);
        assert_eq!(editor.buffer, "src");
        assert!(editor.is_showing_autocomplete());
        editor.handle_key(&Key::Tab);
        assert_eq!(editor.buffer, "src/");
        assert!(!editor.is_showing_autocomplete());
    }

    #[test]
    fn force_mode_stays_open_while_typing() {
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(Rc::new(MockProvider {
            force_only: false,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|prefix, force| {
                if !(force || prefix.contains('/') || prefix.starts_with('.')) {
                    return None;
                }
                let files = ["readme.md", "package.json", "src/", "dist/"];
                let items: Vec<_> = files
                    .into_iter()
                    .filter(|f| {
                        f.to_ascii_lowercase()
                            .starts_with(&prefix.to_ascii_lowercase())
                    })
                    .map(item)
                    .collect();
                if items.is_empty() {
                    None
                } else {
                    Some(AutocompleteSuggestions {
                        items,
                        prefix: prefix.to_string(),
                    })
                }
            }),
        }));
        editor.handle_key(&Key::Tab);
        assert!(editor.is_showing_autocomplete());
        editor.handle_key(&Key::Char('r'));
        assert_eq!(editor.buffer, "r");
        assert!(editor.is_showing_autocomplete());
        editor.handle_key(&Key::Char('e'));
        assert_eq!(editor.buffer, "re");
        assert!(editor.is_showing_autocomplete());
        editor.handle_key(&Key::Tab);
        assert_eq!(editor.buffer, "readme.md");
        assert!(!editor.is_showing_autocomplete());
    }

    #[test]
    fn hides_autocomplete_when_backspacing_slash_to_empty() {
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(Rc::new(MockProvider {
            force_only: false,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|prefix, _force| {
                if !prefix.starts_with('/') {
                    return None;
                }
                let commands = [item("/model"), item("/help")];
                let query = &prefix[1..];
                let items: Vec<_> = commands
                    .into_iter()
                    .filter(|c| c.value[1..].starts_with(query) || c.value.starts_with(prefix))
                    .collect();
                if items.is_empty() {
                    None
                } else {
                    Some(AutocompleteSuggestions {
                        items,
                        prefix: prefix.to_string(),
                    })
                }
            }),
        }));
        editor.handle_key(&Key::Char('/'));
        assert!(editor.is_showing_autocomplete());
        editor.handle_key(&Key::Backspace);
        assert_eq!(editor.buffer, "");
        assert!(!editor.is_showing_autocomplete());
    }

    #[test]
    fn debounces_at_and_hash_until_advance() {
        let provider = Rc::new(MockProvider {
            force_only: false,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|prefix, _force| {
                Some(AutocompleteSuggestions {
                    items: vec![item("@main.ts")],
                    prefix: prefix.to_string(),
                })
            }),
        });
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(provider.clone());
        for ch in ['@', 'm', 'a', 'i'] {
            editor.handle_key(&Key::Char(ch));
        }
        assert_eq!(provider.calls.get(), 0);
        assert!(!editor.is_showing_autocomplete());
        editor.advance_autocomplete(50);
        assert_eq!(provider.calls.get(), 1);
        assert!(editor.is_showing_autocomplete());
    }

    #[test]
    fn custom_trigger_characters_reset_when_provider_changes() {
        let first = Rc::new(MockProvider {
            force_only: false,
            trigger_characters: vec!["$".into()],
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|prefix, _force| {
                Some(AutocompleteSuggestions {
                    items: vec![item("$skill-name")],
                    prefix: prefix.to_string(),
                })
            }),
        });
        let second = Rc::new(MockProvider {
            force_only: false,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|_prefix, _force| {
                Some(AutocompleteSuggestions {
                    items: vec![item("$skill-name")],
                    prefix: "$".into(),
                })
            }),
        });
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(first);
        editor.set_autocomplete_provider(second.clone());
        editor.handle_key(&Key::Char('$'));
        editor.handle_key(&Key::Char('s'));
        editor.advance_autocomplete(50);
        assert_eq!(second.calls.get(), 0);
        assert!(!editor.is_showing_autocomplete());
    }

    #[test]
    fn requeries_when_cursor_moves_back_into_command_name() {
        let mut editor = Editor::default();
        editor.set_autocomplete_provider(Rc::new(MockProvider {
            force_only: false,
            trigger_characters: Vec::new(),
            calls: std::cell::Cell::new(0),
            items_for: Box::new(|before, _force| {
                if !before.starts_with('/') {
                    return None;
                }
                if before.contains(' ') {
                    Some(AutocompleteSuggestions {
                        items: vec![item("repo"), item("message"), item("help")],
                        prefix: before[before.find(' ').unwrap() + 1..].to_string(),
                    })
                } else {
                    Some(AutocompleteSuggestions {
                        items: vec![item("cmd")],
                        prefix: before.to_string(),
                    })
                }
            }),
        }));
        for ch in ['/', 'c', 'm', 'd', ' '] {
            editor.handle_key(&Key::Char(ch));
        }
        assert!(editor.render(80).iter().any(|line| line.contains("repo")));
        editor.handle_key(&Key::Left);
        let rendered = editor.render(80).join("\n");
        assert!(!rendered.contains("repo"), "{rendered}");
        assert!(!rendered.contains("message"), "{rendered}");
    }
}
