//! Terminal UI matching `@earendil-works/pi-tui`.

pub mod component;
pub mod editor;
pub mod fuzzy;
pub mod keys;
pub mod markdown;
pub mod selectors;
pub mod terminal;
pub mod themes;

pub use component::{Component, Text};
pub use editor::Editor;
pub use keys::{parse_key, Key};
pub use markdown::Markdown;
pub use selectors::SelectList;
pub use terminal::{enter_alt_screen, leave_alt_screen, TuiMode};
pub use themes::Theme;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_render_width() {
        let text = Text::new("hello world");
        let rendered = text.render(5);
        assert!(rendered.iter().all(|line| display_width(line) <= 5));
        assert!(!rendered.is_empty());
    }

    #[test]
    fn wrap_ansi_and_keys_match_ts_fixtures() {
        let wrapped = crate::component::wrap("one two three four", 8);
        assert!(wrapped.iter().all(|l| display_width(l) <= 8));
        assert_eq!(parse_key("\u{1b}[A"), Key::Up);
        assert_eq!(parse_key("ctrl+c"), Key::Ctrl('c'));
        assert_eq!(parse_key("enter"), Key::Enter);
        let mut editor = Editor::default();
        editor.handle_key(&Key::Char('π'));
        editor.handle_key(&Key::Left);
        editor.handle_key(&Key::Char('x'));
        assert_eq!(editor.buffer, "xπ");
        let list = SelectList::new(vec!["apple".into(), "apricot".into(), "banana".into()]);
        let mut list = list;
        list.query = "ap".into();
        let rendered = list.render(20);
        assert!(rendered.iter().any(|l| l.contains("apple")));
        assert!(!rendered.iter().any(|l| l.contains("banana")));
    }

    fn display_width(s: &str) -> usize {
        unicode_width::UnicodeWidthStr::width(s)
    }
}
