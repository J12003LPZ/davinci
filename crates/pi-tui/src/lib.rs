//! Terminal UI matching `@earendil-works/pi-tui`.

pub mod alt_screen;
pub mod ansi_text;
pub mod component;
pub mod constrained_layout;
pub mod diff;
pub mod editor;
pub mod fuzzy;
pub mod image;
pub mod keybindings;
pub mod keys;
pub mod layout;
pub mod markdown;
pub mod mouse;
pub mod screen;
pub mod selectors;
pub mod terminal;
pub mod terminal_image;
pub mod themes;
pub mod viewport;
pub mod widgets;
pub mod word_nav;

pub use alt_screen::{
    find_alt_screen_search_matches, layout_transcript_and_dock, TuiAltScreen, TuiAltScreenOptions,
};
pub use component::{wrap_text_with_ansi, Component, Text};
pub use constrained_layout::{LayoutHStack, LayoutVStack, Node, StackEntry};
pub use diff::{
    apply_line_resets, composite_tui_line, extract_cursor_position, hardware_cursor_sequence,
    normalize_terminal_output, truncate_to_width, visible_width, DiffScreen,
};
pub use editor::Editor;
pub use image::{Image, ImageOptions};
pub use keybindings::{matches_key, reset_alt_screen_bindings, set_alt_screen_bindings};
pub use keys::{default_keybindings, parse_bytes, parse_key, read_key, Key, Keybinding};
pub use layout::{ChatView, Container, Overlay};
pub use markdown::Markdown;
pub use mouse::{parse_sgr_mouse, MouseEvent};
pub use screen::{OverlayHandle, OverlayOptions, Tui};
pub use selectors::SelectList;
pub use terminal::{
    disable_mouse, disable_raw_input, enable_mouse, enable_raw_input, enter_alt_screen,
    enter_raw_mode, leave_alt_screen, leave_raw_mode, set_title, TuiMode,
};
pub use terminal_image::{
    allocate_image_id, calculate_image_cell_size, crop_kitty_image_line, delete_all_kitty_images,
    delete_all_kitty_placements, delete_kitty_image, detect_capabilities,
    emit_reserved_image_block, encode_iterm2, encode_kitty, get_capabilities, get_image_dimensions,
    get_kitty_image_metadata, get_kitty_image_placement, hyperlink, image_fallback, is_image_line,
    kitty_image_reserved_rows, prepare_kitty_screen, register_kitty_image_metadata, render_image,
    reset_capabilities_cache, set_capabilities, set_capability_overrides, set_cell_dimensions,
    CachedKittyImage, CellDimensions, ImageDimensions, ImageProtocol, ImageRenderOptions,
    KittyImageMetadata, KittyImagePlacement, RenderedImage, TerminalCapabilities, ITERM2_PREFIX,
    KITTY_PREFIX, MAX_CACHED_OFFSCREEN_KITTY_IMAGES,
};
pub use themes::Theme;
pub use viewport::{Overscroll, ScrollbarMode, ViewportScroll, ViewportScrollOptions};
pub use widgets::{
    BoxWidget, HStack, Input, InputAction, ScrollView, SettingsList, VStack, CURSOR_MARKER,
};
pub use word_nav::{find_word_backward, find_word_forward};

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
        let underline_on = "\u{1b}[4m";
        let underline_off = "\u{1b}[24m";
        let url = "https://example.com/very/long/path/that/will/wrap";
        let ansi = crate::component::wrap_text_with_ansi(
            &format!("read this thread {underline_on}{url}{underline_off}"),
            40,
        );
        assert_eq!(ansi[0], "read this thread");
        assert!(ansi[1].starts_with(underline_on));
        assert!(ansi[1].contains("https://"));
        assert_eq!(parse_key("\u{1b}[A"), Key::Up);
        assert_eq!(parse_key("ctrl+c"), Key::Ctrl('c'));
        assert_eq!(parse_key("enter"), Key::Enter);
        assert_eq!(parse_bytes(&[0x03]), Key::Ctrl('c'));
        assert_eq!(parse_bytes(b"\x1b[D"), Key::Left);
        let mut title_buf = Vec::new();
        crate::terminal::set_title(&mut title_buf, "pi").unwrap();
        assert_eq!(title_buf, b"\x1b]0;pi\x07");
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
