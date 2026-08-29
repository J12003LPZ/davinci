//! Terminal UI matching `@earendil-works/pi-tui`.

mod autocomplete;
mod box_comp;
mod chrome;
mod custom_message;
mod editor;
mod first_time;
mod fuzzy;
mod image;
mod keybindings;
mod keys;
mod latex;
mod login_dialog;
mod markdown;
mod mermaid;
mod mouse;
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
mod themes;
mod tool_card;
mod transcript;
mod tree;

pub use autocomplete::{
    apply_completion, suggestions, AutocompleteItem, AutocompleteSuggestions, SlashCommandSpec,
};
pub use box_comp::TuiBox;
pub use chrome::ChatChrome;
pub use custom_message::{
    CustomMessage, MessageRenderOptions, MessageRenderer, MessageRendererRegistry,
};
pub use editor::Editor;
pub use first_time::{
    detect_terminal_theme, FirstTimeAction, FirstTimeSetup, FirstTimeSetupResult, SETUP_LOGO_LINES,
};
pub use fuzzy::{fuzzy_filter, fuzzy_match, FuzzyMatch};
pub use image::{
    delete_all_kitty_images, delete_kitty_image, encode_kitty, iterm_image, kitty_image_chunk,
    kitty_image_ids, parse_kitty_image_header, KittyImageHeader, KITTY_IMAGE_PREFIX,
};
pub use keybindings::{key_to_bytes, Keybindings};
pub use keys::{decode_kitty_printable, parse_key, Key};
pub use latex::render_latex;
pub use login_dialog::{AuthInfoLink, DeviceCodeInfo, LoginDialog, LoginDialogAction};
pub use markdown::render_markdown;
pub use mermaid::{transform_mermaid, MermaidArt, MermaidContext, MermaidMode, MermaidTheme};
pub use mouse::{parse_mouse_sgr, MouseButton, MouseEvent, MouseKind, MOUSE_DISABLE, MOUSE_ENABLE};
pub use open_browser::{copy_text, open_browser, open_browser_argv, open_browser_dry_run};
pub use osc::{
    detect_terminal_background_from_env, detect_terminal_theme_for_auto, drain_osc_tty,
    parse_osc11_background_color, parse_terminal_color_scheme_report,
    query_terminal_background_color, ThemeDetection, COLOR_SCHEME_QUERY, OSC_11_QUERY,
};
pub use overlay::Overlay;
pub use render::{visible_width, Component, Text};
pub use scoped_models::{
    clear_all, enable_all, get_sorted_ids, is_enabled, move_id, toggle, EnabledIds, ScopedModel,
    ScopedModelsAction, ScopedModelsSelector,
};
pub use scroll::ScrollView;
pub use session::{
    DoubleEscapeAction, InteractiveSession, OverlayKind, SessionAction, BRACKETED_PASTE_DISABLE,
    BRACKETED_PASTE_ENABLE, DISABLE_AUTOWRAP, DOUBLE_ESCAPE_MS, ENABLE_AUTOWRAP,
    KITTY_KEYBOARD_DISABLE, KITTY_KEYBOARD_QUERY, OSC_QUERY_TIMEOUT_MS,
};
pub use session_selector::{
    NameFilter, SessionItem, SessionScope, SessionSelector, SessionSelectorAction, SortMode,
};
pub use settings::{
    default_interactive_settings, format_http_idle_timeout, interactive_settings_list,
    parse_http_idle_timeout, InteractiveSettingsConfig, SettingItem, SettingsList,
};
pub use settings_submenu::{
    parse_auto_theme, ModelThinkingItem, SettingsSubmenu, SettingsSubmenuAction,
    SettingsSubmenuKind, AUTOMATIC_THEME_VALUE,
};
pub use themes::{builtin_themes, load_themes_from_dir, Theme};
pub use tool_card::{ToolCard, ToolCardState, FALLBACK_PREVIEW_LINES};
pub use transcript::{Transcript, TranscriptLine};
pub use tree::{
    build_session_tree, FilterMode, SessionTreeEntry, SessionTreeNode, TreeAction, TreeSelector,
    FILTER_MODES,
};

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
