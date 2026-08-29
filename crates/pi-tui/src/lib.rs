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

    fn display_width(s: &str) -> usize {
        unicode_width::UnicodeWidthStr::width(s)
    }
}
