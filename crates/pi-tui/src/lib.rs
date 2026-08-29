//! Terminal UI matching `@earendil-works/pi-tui`.

mod alt_screen_flash;
mod alt_screen_search;
mod ansi;
mod autocomplete;
mod box_comp;
mod chrome;
mod config_selector;
mod container;
mod custom_message;
mod editor;
mod extension_ui;
mod first_time;
mod footer;
mod fuzzy;
mod image;
mod input;
mod item_select_list;
mod keybindings;
mod keys;
mod kill_ring;
mod latex;
mod layout;
mod loader;
mod login_dialog;
mod markdown;
mod mermaid;
mod model_selector;
mod mouse;
mod native;
mod open_browser;
mod osc;
mod overlay;
mod render;
mod scoped_models;
mod scroll;
mod session;
mod session_selector;
mod settings;
mod settings_submenu;
mod spacer;
mod stack;
mod stdin_buffer;
mod terminal;
mod themes;
mod thinking_selector;
mod tool_card;
mod transcript;
mod tree;
mod truncated_text;
mod trust_selector;
mod tui_alt_screen;
mod tui_runtime;
mod tui_text;
mod undo_stack;
mod word_nav;
mod word_wrap;

pub use alt_screen_flash::AltScreenFlashContainer;
pub use alt_screen_search::{
    find_alt_screen_search_matches, find_alt_screen_search_matches_in,
    get_alt_screen_search_match_key, AltScreenSearchComponent, AltScreenSearchMatch,
    AltScreenSearchSegment,
};
pub use ansi::{
    extract_ansi_code, extract_segments, get_grapheme_cell_range, get_osc8_link_at_column,
    grapheme_width, normalize_terminal_output, slice_by_column, slice_with_width,
    strip_terminal_sequences as strip_ansi_sequences, truncate_to_width as truncate_to_width_ansi,
    visible_width as visible_width_ansi, wrap_text_with_ansi, ExtractedSegments, GraphemeCellRange,
    SlicedText,
};
pub use autocomplete::{
    apply_completion, autocomplete_debounce_ms, suggestions, AutocompleteItem,
    AutocompleteSuggestions, ExtraAutocompleteProvider, LiveAutocompleteQuery, SlashCommandSpec,
    SuggestionQuery, ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS, DEFAULT_AUTOCOMPLETE_TRIGGER_CHARACTERS,
};
pub use box_comp::TuiBox;
pub use chrome::ChatChrome;
pub use config_selector::{
    ConfigResource, ConfigResourceKind, ConfigScope, ConfigSelector, ConfigSelectorAction,
};
pub use container::{Container, SharedComponent};
pub use custom_message::{
    CustomMessage, MessageRenderOptions, MessageRenderer, MessageRendererRegistry,
};
pub use editor::Editor;
pub use extension_ui::{
    ExtensionConfirm, ExtensionDialogAction, ExtensionEditor, ExtensionInput, ExtensionProgress,
    ExtensionSelector, ExtensionWidget, WidgetPlacement, MAX_WIDGET_LINES,
};
pub use first_time::{
    detect_terminal_theme, FirstTimeAction, FirstTimeSetup, FirstTimeSetupResult, SETUP_LOGO_LINES,
};
pub use footer::{
    find_git_paths, format_cwd_for_footer, format_pwd_line, resolve_git_branch, truncate_to_width,
    GitPaths,
};
pub use fuzzy::{fuzzy_filter, fuzzy_match, FuzzyMatch};
pub use image::{
    crop_kitty_image_line, delete_all_kitty_images, delete_all_kitty_placements,
    delete_kitty_image, encode_kitty, get_cell_dimensions, get_kitty_image_metadata,
    get_kitty_image_placement, is_image_line, iterm_image, kitty_image_chunk, kitty_image_ids,
    parse_kitty_image_header, register_kitty_image_metadata, set_cell_dimensions, CellDimensions,
    KittyImageHeader, KittyImageMetadata, KittyImagePlacement, KITTY_IMAGE_PREFIX,
};
pub use input::{Input, InputAction};
pub use item_select_list::{
    ItemSelectList, SelectItem, SelectListLayoutOptions, SelectListTheme,
    SelectListTruncatePrimaryContext,
};
pub use keybindings::{key_to_bytes, Keybindings};
pub use keys::{decode_kitty_printable, is_key_release, parse_key, Key};
pub use latex::render_latex;
pub use layout::{
    get_scroll_view_box, get_scroll_views_at, get_scrollbar_geometry, render_layout_frame,
    LayoutBox, LayoutFrame, LayoutRect, ScrollbarGeometry,
};
pub use loader::{
    CancellableLoader, Loader, LoaderIndicatorOptions, DEFAULT_LOADER_FRAMES,
    DEFAULT_LOADER_INTERVAL_MS,
};
pub use login_dialog::{AuthInfoLink, DeviceCodeInfo, LoginDialog, LoginDialogAction};
pub use markdown::{
    format_markdown_link, hyperlinks_enabled, osc8_hyperlink, render_markdown,
    render_markdown_with, DEFAULT_CODE_BLOCK_INDENT,
};
pub use mermaid::{transform_mermaid, MermaidArt, MermaidContext, MermaidMode, MermaidTheme};
pub use model_selector::{ModelScope, ModelSelector, ModelSelectorAction, ModelSelectorItem};
pub use mouse::{parse_mouse_sgr, MouseButton, MouseEvent, MouseKind, MOUSE_DISABLE, MOUSE_ENABLE};
pub use native::{
    enable_virtual_terminal_input, get_native_module_candidates, is_native_modifier_pressed,
    native_helper_path, ModifierKey, NativeModuleCandidateOptions, TUI_PACKAGE_NAME,
};
pub use open_browser::{copy_text, open_browser, open_browser_argv, open_browser_dry_run};
pub use osc::{
    detect_terminal_background_from_env, detect_terminal_theme_for_auto, drain_osc_tty,
    parse_osc11_background_color, parse_terminal_color_scheme_report,
    query_terminal_background_color, ThemeDetection, COLOR_SCHEME_QUERY, OSC_11_QUERY,
    TERMINAL_PROGRESS_ACTIVE_SEQUENCE, TERMINAL_PROGRESS_CLEAR_SEQUENCE,
    TERMINAL_PROGRESS_KEEPALIVE_MS,
};
pub use overlay::{
    composite_overlay_lines, composite_tui_line, overlay_options_from_json, resolve_anchor_col,
    resolve_anchor_row, resolve_overlay_layout, Overlay, OverlayAnchor, OverlayLayout,
    OverlayMargin, OverlayOptions, SizeValue,
};
pub use render::{
    strip_terminal_sequences, visible_width, visible_width_stripped, Component, RenderedLines,
    SparseLines, Text,
};
pub use scoped_models::{
    clear_all, enable_all, get_sorted_ids, is_enabled, move_id, toggle, EnabledIds, ScopedModel,
    ScopedModelsAction, ScopedModelsSelector,
};
pub use scroll::{
    ScrollFollow, ScrollOverscroll, ScrollView, ScrollViewOptions, ScrollViewScrollbar,
};
pub use session::{
    DoubleEscapeAction, InteractiveSession, OverlayKind, SessionAction, BRACKETED_PASTE_DISABLE,
    BRACKETED_PASTE_ENABLE, DISABLE_AUTOWRAP, DOUBLE_ESCAPE_MS, ENABLE_AUTOWRAP,
    KITTY_KEYBOARD_DISABLE, KITTY_KEYBOARD_QUERY, OSC_QUERY_TIMEOUT_MS,
};
pub use session_selector::{
    match_session, parse_search_query, NameFilter, ParsedSearchQuery, SearchMode, SessionItem,
    SessionScope, SessionSelector, SessionSelectorAction, SortMode,
};
pub use settings::{
    default_interactive_settings, format_http_idle_timeout, interactive_settings_list,
    parse_http_idle_timeout, InteractiveSettingsConfig, SettingItem, SettingsList,
};
pub use settings_submenu::{
    parse_auto_theme, ModelThinkingItem, SettingsSubmenu, SettingsSubmenuAction,
    SettingsSubmenuKind, AUTOMATIC_THEME_VALUE,
};
pub use spacer::Spacer;
pub use stack::{
    allocate_stack_sizes, HStack, LayoutViewport, StackAlign, StackBasis, StackEntryOptions,
    StackLayoutEntry, VStack,
};
pub use stdin_buffer::{StdinBuffer, StdinBufferOptions, StdinEvents};
pub use terminal::{
    is_apple_terminal_session, is_keyboard_protocol_negotiation_sequence_prefix,
    is_kitty_protocol_active, normalize_apple_terminal_input, normalize_native_shift_enter_input,
    parse_keyboard_protocol_negotiation_sequence, resolve_escape_timeout_ms,
    resolve_escape_timeout_ms_from_env, rewrite_shift_enter_input, set_kitty_protocol_active,
    KeyboardProtocolNegotiationSequence, MemoryTerminal, ProcessTerminal, TerminalIo,
    DEFAULT_ESCAPE_TIMEOUT_MS, DEFAULT_SSH_ESCAPE_TIMEOUT_MS,
    DESIRED_KITTY_KEYBOARD_PROTOCOL_FLAGS, KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT_MS,
    KITTY_KEYBOARD_PROTOCOL_QUERY, MODIFY_OTHER_KEYS_DISABLE, MODIFY_OTHER_KEYS_ENABLE,
    NATIVE_SHIFT_ENTER_SEQUENCE,
};
pub use themes::{builtin_themes, load_themes_from_dir, Theme};
pub use thinking_selector::{
    ThinkingSelector, ThinkingSelectorAction, LEVEL_DESCRIPTIONS as THINKING_LEVEL_DESCRIPTIONS,
};
pub use tool_card::{ToolCard, ToolCardState, FALLBACK_PREVIEW_LINES};
pub use transcript::{Transcript, TranscriptLine};
pub use tree::{
    build_session_tree, FilterMode, SessionTreeEntry, SessionTreeNode, TreeAction, TreeSelector,
    FILTER_MODES,
};
pub use truncated_text::TruncatedText;
pub use trust_selector::{
    TrustOption, TrustSavedDecision, TrustSelector, TrustSelectorAction, TrustUpdate,
};
pub use tui_alt_screen::{TuiAltScreen, TuiAltScreenOptions};
pub use tui_runtime::{
    OverlayHandle, TuiMainScreen, TuiMainScreenRenderState, TuiRuntimeMode, TuiStopOptions,
};
pub use tui_text::{apply_background_to_line, TuiText};

pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";
pub const ALT_BUFFER_ENTER: &str = "\x1b[?1049h";
pub const ALT_BUFFER_LEAVE: &str = "\x1b[?1049l";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiMode {
    Regular,
    Fullscreen,
}

impl TuiMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "regular" => Some(Self::Regular),
            "fullscreen" => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keybinding {
    pub action: String,
    pub keys: Vec<String>,
}

pub const TUI_KEYBINDINGS: &[(&str, &str)] = &[
    ("submit", "enter"),
    ("newline", "shift+enter"),
    ("abort", "escape"),
    ("quit", "ctrl+c"),
    ("cycle-model", "ctrl+p"),
    ("cycle-thinking", "ctrl+t"),
    ("clear", "ctrl+l"),
];

pub fn get_keybindings() -> Vec<Keybinding> {
    let bindings = Keybindings::defaults();
    let mut out: Vec<Keybinding> = TUI_KEYBINDINGS
        .iter()
        .map(|(action, key)| Keybinding {
            action: (*action).to_string(),
            keys: vec![(*key).to_string()],
        })
        .collect();
    for action in [
        "app.interrupt",
        "app.clear",
        "app.exit",
        "app.model.select",
        "app.model.cycleForward",
        "app.model.cycleBackward",
        "app.tools.expand",
        "app.thinking.cycle",
        "app.thinking.toggle",
        "app.editor.external",
        "app.clipboard.pasteImage",
        "app.message.followUp",
        "app.message.dequeue",
        "app.session.new",
        "app.session.tree",
        "app.session.fork",
        "app.session.resume",
        "app.session.delete",
    ] {
        out.push(Keybinding {
            action: action.into(),
            keys: bindings.keys_for(action).to_vec(),
        });
    }
    out
}

#[derive(Debug, Clone)]
pub struct SelectList {
    pub items: Vec<String>,
    pub selected: usize,
    pub query: String,
}

impl SelectList {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            selected: 0,
            query: String::new(),
        }
    }

    pub fn filtered(&self) -> Vec<String> {
        fuzzy_filter(&self.query, &self.items)
    }

    pub fn selected_item(&self) -> Option<String> {
        self.filtered().get(self.selected).cloned()
    }

    pub fn move_by(&mut self, delta: isize) {
        let filtered = self.filtered();
        if filtered.is_empty() {
            return;
        }
        let len = filtered.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }
}

impl Component for SelectList {
    fn render(&self, width: usize) -> Vec<String> {
        let filtered = fuzzy_filter(&self.query, &self.items);
        filtered
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if index == self.selected { "> " } else { "  " };
                truncate_line(&format!("{prefix}{item}"), width)
            })
            .collect()
    }

    fn invalidate(&mut self) {}
}

fn truncate_line(line: &str, width: usize) -> String {
    if visible_width(line) <= width {
        line.to_string()
    } else {
        let mut out = String::new();
        for ch in line.chars() {
            if visible_width(&out) + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
                > width.saturating_sub(1)
            {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_render_width_and_fuzzy() {
        let list = SelectList::new(vec!["openai/gpt-4".into(), "anthropic/sonnet".into()]);
        let lines = list.render(10);
        assert!(lines.iter().all(|line| visible_width(line) <= 10));
        assert!(fuzzy_match("gpt", "openai/gpt-4").matches);
        assert!(!fuzzy_match("zzz", "openai/gpt-4").matches);
    }

    #[test]
    fn markdown_and_keybindings() {
        let lines = render_markdown("# Title\n\nHello **world**", 40);
        assert!(lines.iter().any(|line| line.contains("Title")));
        assert!(get_keybindings().iter().any(|b| b.action == "cycle-model"));
        let mut chrome = ChatChrome::new(builtin_themes()[0].clone(), "pi 0.84.4");
        chrome
            .transcript
            .extra_transformers
            .push(|text, _, _| text.replace("hello", "hallo"));
        chrome.transcript.push("user", "hi");
        chrome.transcript.push("assistant", "# hello");
        let rendered = chrome.render(40);
        assert!(rendered.iter().any(|line| line.contains("hallo")));
        assert!(rendered.iter().any(|line| line.contains("pi 0.84.4")));
        assert!(parse_mouse_sgr("\x1b[<0;2;2M").is_some());
        assert_eq!(render_latex("\\pi", false).as_deref(), Some("π"));
        assert_eq!(
            decode_kitty_printable("\u{1b}[57399u").as_deref(),
            Some("0")
        );
        assert_eq!(
            decode_kitty_printable("\u{1b}[57416u").as_deref(),
            Some(",")
        );
        assert!(decode_kitty_printable("\u{1b}[57417u").is_none());
        let mut settings = SettingsList::new(
            vec![SettingItem {
                id: "theme".into(),
                label: "Theme".into(),
                description: None,
                current_value: "dark".into(),
                values: vec!["dark".into(), "light".into()],
            }],
            8,
        );
        settings.cycle();
        assert_eq!(settings.items[0].current_value, "light");
    }
}
