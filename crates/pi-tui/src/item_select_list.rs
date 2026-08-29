//! SelectList matching `vendor/pi/packages/tui/src/components/select-list.ts`.

use crate::ansi::{truncate_to_width, visible_width};
use crate::keybindings::Keybindings;
use crate::render::Component;

const DEFAULT_PRIMARY_COLUMN_WIDTH: usize = 32;
const PRIMARY_COLUMN_GAP: usize = 2;
const MIN_DESCRIPTION_WIDTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

pub struct SelectListTheme {
    pub selected_prefix: Box<dyn Fn(&str) -> String>,
    pub selected_text: Box<dyn Fn(&str) -> String>,
    pub description: Box<dyn Fn(&str) -> String>,
    pub scroll_info: Box<dyn Fn(&str) -> String>,
    pub no_match: Box<dyn Fn(&str) -> String>,
}

impl SelectListTheme {
    pub fn identity() -> Self {
        Self {
            selected_prefix: Box::new(|text| text.to_string()),
            selected_text: Box::new(|text| text.to_string()),
            description: Box::new(|text| text.to_string()),
            scroll_info: Box::new(|text| text.to_string()),
            no_match: Box::new(|text| text.to_string()),
        }
    }
}

pub struct SelectListTruncatePrimaryContext<'a> {
    pub text: &'a str,
    pub max_width: usize,
    pub column_width: usize,
    pub item: &'a SelectItem,
    pub is_selected: bool,
}

type TruncatePrimaryFn = Box<dyn Fn(SelectListTruncatePrimaryContext<'_>) -> String>;

#[derive(Default)]
pub struct SelectListLayoutOptions {
    pub min_primary_column_width: Option<usize>,
    pub max_primary_column_width: Option<usize>,
    pub truncate_primary: Option<TruncatePrimaryFn>,
}

pub struct ItemSelectList {
    items: Vec<SelectItem>,
    filtered_items: Vec<SelectItem>,
    selected_index: usize,
    max_visible: usize,
    theme: SelectListTheme,
    layout: SelectListLayoutOptions,
    pub selected: Option<SelectItem>,
    pub cancelled: bool,
}

impl ItemSelectList {
    pub fn new(
        items: Vec<SelectItem>,
        max_visible: usize,
        theme: SelectListTheme,
        layout: SelectListLayoutOptions,
    ) -> Self {
        let filtered_items = items.clone();
        Self {
            items,
            filtered_items,
            selected_index: 0,
            max_visible,
            theme,
            layout,
            selected: None,
            cancelled: false,
        }
    }

    pub fn set_filter(&mut self, filter: &str) {
        let needle = filter.to_ascii_lowercase();
        self.filtered_items = self
            .items
            .iter()
            .filter(|item| item.value.to_ascii_lowercase().starts_with(&needle))
            .cloned()
            .collect();
        self.selected_index = 0;
    }

    pub fn set_selected_index(&mut self, index: usize) {
        self.selected_index = index.min(self.filtered_items.len().saturating_sub(1));
    }

    pub fn selected_item(&self) -> Option<&SelectItem> {
        self.filtered_items.get(self.selected_index)
    }

    fn normalize_to_single_line(text: &str) -> String {
        let mut out = String::new();
        let mut in_break = false;
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                if !in_break {
                    out.push(' ');
                    in_break = true;
                }
            } else {
                in_break = false;
                out.push(ch);
            }
        }
        out.trim().to_string()
    }

    fn display_value(item: &SelectItem) -> &str {
        if item.label.is_empty() {
            &item.value
        } else {
            &item.label
        }
    }

    fn primary_column_bounds(&self) -> (usize, usize) {
        let raw_min = self
            .layout
            .min_primary_column_width
            .or(self.layout.max_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        let raw_max = self
            .layout
            .max_primary_column_width
            .or(self.layout.min_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        (1.max(raw_min.min(raw_max)), 1.max(raw_min.max(raw_max)))
    }

    fn primary_column_width(&self) -> usize {
        let (min, max) = self.primary_column_bounds();
        let widest = self.filtered_items.iter().fold(0usize, |widest, item| {
            widest.max(visible_width(Self::display_value(item)) + PRIMARY_COLUMN_GAP)
        });
        widest.clamp(min, max)
    }

    fn truncate_primary(
        &self,
        item: &SelectItem,
        is_selected: bool,
        max_width: usize,
        column_width: usize,
    ) -> String {
        let display_value = Self::display_value(item);
        let truncated = if let Some(custom) = &self.layout.truncate_primary {
            custom(SelectListTruncatePrimaryContext {
                text: display_value,
                max_width,
                column_width,
                item,
                is_selected,
            })
        } else {
            truncate_to_width(display_value, max_width, "", false)
        };
        truncate_to_width(&truncated, max_width, "", false)
    }

    fn render_item(
        &self,
        item: &SelectItem,
        is_selected: bool,
        width: usize,
        description: Option<&str>,
        primary_column_width: usize,
    ) -> String {
        let prefix = if is_selected { "→ " } else { "  " };
        let prefix_width = visible_width(prefix);
        if let Some(description) = description {
            if width > 40 {
                let effective =
                    1.max(primary_column_width.min(width.saturating_sub(prefix_width + 4)));
                let max_primary = 1.max(effective.saturating_sub(PRIMARY_COLUMN_GAP));
                let truncated_value =
                    self.truncate_primary(item, is_selected, max_primary, effective);
                let truncated_value_width = visible_width(&truncated_value);
                let spacing = " ".repeat(1.max(effective.saturating_sub(truncated_value_width)));
                let description_start = prefix_width + truncated_value_width + spacing.len();
                let remaining = width.saturating_sub(description_start + 2);
                if remaining > MIN_DESCRIPTION_WIDTH {
                    let truncated_desc = truncate_to_width(description, remaining, "", false);
                    if is_selected {
                        return (self.theme.selected_text)(&format!(
                            "{prefix}{truncated_value}{spacing}{truncated_desc}"
                        ));
                    }
                    let desc_text = (self.theme.description)(&format!("{spacing}{truncated_desc}"));
                    return format!("{prefix}{truncated_value}{desc_text}");
                }
            }
        }
        let max_width = width.saturating_sub(prefix_width + 2);
        let truncated_value = self.truncate_primary(item, is_selected, max_width, max_width);
        if is_selected {
            (self.theme.selected_text)(&format!("{prefix}{truncated_value}"))
        } else {
            format!("{prefix}{truncated_value}")
        }
    }
}

impl Component for ItemSelectList {
    fn render(&self, width: usize) -> Vec<String> {
        if self.filtered_items.is_empty() {
            return vec![(self.theme.no_match)("  No matching commands")];
        }
        let primary_column_width = self.primary_column_width();
        let start_index = self
            .selected_index
            .saturating_sub(self.max_visible / 2)
            .min(self.filtered_items.len().saturating_sub(self.max_visible));
        let end_index = (start_index + self.max_visible).min(self.filtered_items.len());
        let mut lines = Vec::new();
        for (index, item) in self.filtered_items[start_index..end_index]
            .iter()
            .enumerate()
        {
            let absolute = start_index + index;
            let description = item
                .description
                .as_deref()
                .map(Self::normalize_to_single_line);
            lines.push(self.render_item(
                item,
                absolute == self.selected_index,
                width,
                description.as_deref(),
                primary_column_width,
            ));
        }
        if start_index > 0 || end_index < self.filtered_items.len() {
            let scroll_text = format!(
                "  ({}/{})",
                self.selected_index + 1,
                self.filtered_items.len()
            );
            lines.push((self.theme.scroll_info)(&truncate_to_width(
                &scroll_text,
                width.saturating_sub(2),
                "",
                false,
            )));
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let kb = Keybindings::defaults();
        if kb.matches(data, "tui.select.up") {
            self.selected_index = if self.selected_index == 0 {
                self.filtered_items.len().saturating_sub(1)
            } else {
                self.selected_index - 1
            };
        } else if kb.matches(data, "tui.select.down") {
            self.selected_index = if self.selected_index + 1 >= self.filtered_items.len() {
                0
            } else {
                self.selected_index + 1
            };
        } else if kb.matches(data, "tui.select.confirm") {
            self.selected = self.filtered_items.get(self.selected_index).cloned();
        } else if kb.matches(data, "tui.select.cancel") {
            self.cancelled = true;
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::visible_width;

    fn visible_index_of(line: &str, text: &str) -> usize {
        let index = line.find(text).expect(text);
        visible_width(&line[..index])
    }

    #[test]
    fn select_list_layout_options_match_ts() {
        let theme = SelectListTheme::identity();
        let items = vec![SelectItem {
            value: "test".into(),
            label: "test".into(),
            description: Some("Line one\nLine two\nLine three".into()),
        }];
        let list = ItemSelectList::new(items, 5, theme, SelectListLayoutOptions::default());
        let rendered = list.render(100);
        assert!(!rendered[0].contains('\n'));
        assert!(rendered[0].contains("Line one Line two Line three"));

        let theme = SelectListTheme::identity();
        let items = vec![
            SelectItem {
                value: "short".into(),
                label: "short".into(),
                description: Some("short description".into()),
            },
            SelectItem {
                value: "very-long-command-name-that-needs-truncation".into(),
                label: "very-long-command-name-that-needs-truncation".into(),
                description: Some("long description".into()),
            },
        ];
        let list = ItemSelectList::new(items, 5, theme, SelectListLayoutOptions::default());
        let rendered = list.render(80);
        assert_eq!(
            visible_index_of(&rendered[0], "short description"),
            visible_index_of(&rendered[1], "long description")
        );

        let theme = SelectListTheme::identity();
        let items = vec![
            SelectItem {
                value: "a".into(),
                label: "a".into(),
                description: Some("first".into()),
            },
            SelectItem {
                value: "bb".into(),
                label: "bb".into(),
                description: Some("second".into()),
            },
        ];
        let list = ItemSelectList::new(
            items,
            5,
            theme,
            SelectListLayoutOptions {
                min_primary_column_width: Some(12),
                max_primary_column_width: Some(20),
                truncate_primary: None,
            },
        );
        let rendered = list.render(80);
        assert_eq!(visible_index_of(&rendered[0], "first"), 14);
        assert_eq!(visible_index_of(&rendered[1], "second"), 14);

        let theme = SelectListTheme::identity();
        let items = vec![
            SelectItem {
                value: "very-long-command-name-that-needs-truncation".into(),
                label: "very-long-command-name-that-needs-truncation".into(),
                description: Some("first".into()),
            },
            SelectItem {
                value: "short".into(),
                label: "short".into(),
                description: Some("second".into()),
            },
        ];
        let list = ItemSelectList::new(
            items,
            5,
            theme,
            SelectListLayoutOptions {
                min_primary_column_width: Some(12),
                max_primary_column_width: Some(20),
                truncate_primary: None,
            },
        );
        let rendered = list.render(80);
        assert_eq!(visible_index_of(&rendered[0], "first"), 22);
        assert_eq!(visible_index_of(&rendered[1], "second"), 22);
    }
}
