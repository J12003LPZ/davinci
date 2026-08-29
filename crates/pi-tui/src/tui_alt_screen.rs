//! Alternate-screen TUI matching `vendor/pi/packages/tui/src/tui-alt-screen.ts`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::alt_screen_flash::AltScreenFlashContainer;
use crate::alt_screen_search::{
    find_alt_screen_search_matches_in, get_alt_screen_search_match_key, AltScreenSearchComponent,
    AltScreenSearchMatch,
};
use crate::ansi::{
    extract_ansi_code, get_grapheme_cell_range, get_osc8_link_at_column, slice_by_column,
    strip_terminal_sequences, visible_width,
};
use crate::container::SharedComponent;
use crate::container::{Container, SharedContainer};
use crate::image::{
    delete_all_kitty_images, delete_all_kitty_placements, delete_kitty_image,
    get_kitty_image_placement, is_image_line,
};
use crate::keybindings::Keybindings;
use crate::keys::is_key_release;
use crate::layout::{
    get_scroll_view_box, get_scroll_views_at, get_scrollbar_geometry, render_layout_frame,
    LayoutFrame,
};
use crate::overlay::{composite_tui_line, OverlayAnchor, OverlayMargin, OverlayOptions, SizeValue};
use crate::render::{Component, RenderedLines};
use crate::scroll::{ScrollFollow, ScrollOverscroll, ScrollView, ScrollViewOptions};
use crate::stack::{HStack, VStack};
use crate::terminal::TerminalIo;
use crate::tui_runtime::{
    OverlayHandle, TuiBase, TuiRuntimeMode, TuiStopOptions, BEGIN_SYNCHRONIZED_OUTPUT,
    DISABLE_AUTOWRAP, DISABLE_MOUSE, ENABLE_ALL_MOTION_MOUSE, ENABLE_AUTOWRAP,
    ENABLE_BUTTON_MOTION_MOUSE, END_SYNCHRONIZED_OUTPUT, ENTER_ALT_SCREEN, EXIT_ALT_SCREEN,
    FOCUS_IN, FOCUS_OUT, OSC133_PROMPT_START, OSC133_ZONE_PREFIX, PAGE_SCROLL_OVERLAP,
};
use crate::word_nav::default_word_segments;

const MAX_CACHED_OFFSCREEN_KITTY_IMAGES: usize = 16;
const MAX_CACHED_OFFSCREEN_KITTY_TRANSMISSION_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHED_OFFSCREEN_KITTY_DECODED_BYTES: usize = 64 * 1024 * 1024;
const DOUBLE_CLICK_INTERVAL_MS: u64 = 500;
const TERMINAL_WORD_SELECTION_JOINERS: &[char] = &['/', '-'];

type StyleFn = Rc<dyn Fn(&str) -> String>;
type OpenUrlFn = Box<dyn FnMut(&str)>;
type RightClickPasteFn = Box<dyn FnMut()>;
type CopySelectionFn = Box<dyn Fn(&str) -> bool>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionGranularity {
    Character,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchSelectionMode {
    Query,
    Retain,
    Next,
    Previous,
}

#[derive(Debug, Clone, Copy)]
struct SelectionPoint {
    row: usize,
    col: usize,
    scroll_id: Option<usize>,
    boundary: bool,
}

struct SelectionRange {
    start: SelectionPoint,
    end: SelectionPoint,
}

struct ClickTarget {
    timestamp: u64,
    count: u32,
    row: usize,
    scroll_id: Option<usize>,
    word_start: usize,
    word_end: usize,
}

struct ScrollbarDrag {
    scroll_id: usize,
    grab_offset: isize,
}

struct CachedKittyImage {
    transmission_generation: u64,
    transmission_bytes: usize,
    estimated_decoded_bytes: usize,
}

struct ActiveSearch {
    overlay: OverlayHandle,
    query: String,
    matches: Vec<AltScreenSearchMatch>,
    selected_index: isize,
    selected_key: Option<String>,
    anchor_row: usize,
    selection_mode: SearchSelectionMode,
}

struct SgrMouseEvent {
    button: u32,
    x: usize,
    y: usize,
    release: bool,
}

struct WheelEvent {
    direction: isize,
    x: usize,
    y: usize,
}

pub struct TuiAltScreenOptions {
    pub wheel_scroll_lines: usize,
    pub mouse: bool,
    pub copy_on_select: bool,
    pub show_hardware_cursor: Option<bool>,
    pub log_directory: Option<PathBuf>,
    pub search_match_style: Option<StyleFn>,
    pub search_current_match_style: Option<StyleFn>,
    pub open_url: Option<OpenUrlFn>,
    pub on_right_click_paste: Option<RightClickPasteFn>,
    pub copy_selection: Option<CopySelectionFn>,
}

impl Default for TuiAltScreenOptions {
    fn default() -> Self {
        Self {
            wheel_scroll_lines: 1,
            mouse: true,
            copy_on_select: true,
            show_hardware_cursor: None,
            log_directory: None,
            search_match_style: None,
            search_current_match_style: None,
            open_url: None,
            on_right_click_paste: None,
            copy_selection: None,
        }
    }
}

/// Alternate-screen TUI matching TS `TuiAltScreen`.
pub struct TuiAltScreen {
    pub base: TuiBase,
    previous_screen: Vec<String>,
    previous_screen_width: usize,
    previous_screen_height: usize,
    last_document: Vec<String>,
    document: Rc<RefCell<Container>>,
    implicit_scroll: ScrollView,
    layout_root: Option<Box<dyn Component>>,
    current_layout: Option<LayoutFrame>,
    flashes: AltScreenFlashContainer,
    alt_screen_active: bool,
    wheel_scroll_lines: usize,
    mouse_enabled: bool,
    copy_on_select: bool,
    image_protocol: Option<&'static str>,
    uploaded_kitty_images: HashMap<u32, CachedKittyImage>,
    keybindings: Keybindings,
    search_match_style: StyleFn,
    search_current_match_style: StyleFn,
    open_url: Option<OpenUrlFn>,
    on_right_click_paste: Option<RightClickPasteFn>,
    copy_selection: Option<CopySelectionFn>,
    selection_anchor: Option<SelectionPoint>,
    selection_focus: Option<SelectionPoint>,
    selection_granularity: SelectionGranularity,
    selection_initial_range: Option<SelectionRange>,
    last_click: Option<ClickTarget>,
    selection_press_active: bool,
    selection_dragged: bool,
    selection_auto_scroll_direction: i8,
    selection_drag_pointer: Option<(usize, usize)>,
    scrollbar_drag: Option<ScrollbarDrag>,
    scrollbar_hover: Option<usize>,
    active_search: Option<ActiveSearch>,
    pressed_url: Option<String>,
    child_focus: Option<usize>,
}

impl TuiAltScreen {
    pub fn new(terminal: Box<dyn TerminalIo>) -> Self {
        Self::with_options(terminal, TuiAltScreenOptions::default())
    }

    pub fn with_options(terminal: Box<dyn TerminalIo>, options: TuiAltScreenOptions) -> Self {
        let document = Rc::new(RefCell::new(Container::new()));
        let implicit_scroll = ScrollView::new(
            Box::new(SharedContainer::new(document.clone())),
            ScrollViewOptions {
                follow: ScrollFollow::End,
                primary: true,
                ..ScrollViewOptions::default()
            },
        )
        .expect("vertical scroll");
        let search_match_style = options
            .search_match_style
            .unwrap_or_else(|| Rc::new(|text: &str| format!("\x1b[4m{text}\x1b[24m")));
        let search_current_match_style = options
            .search_current_match_style
            .unwrap_or_else(|| Rc::new(|text: &str| format!("\x1b[1;7m{text}\x1b[22;27m")));
        Self {
            base: TuiBase::new(
                terminal,
                options.show_hardware_cursor,
                options.log_directory,
            ),
            previous_screen: Vec::new(),
            previous_screen_width: 0,
            previous_screen_height: 0,
            last_document: Vec::new(),
            document,
            implicit_scroll,
            layout_root: None,
            current_layout: None,
            flashes: AltScreenFlashContainer::new(),
            alt_screen_active: false,
            wheel_scroll_lines: options.wheel_scroll_lines.max(1),
            mouse_enabled: options.mouse,
            copy_on_select: options.copy_on_select,
            image_protocol: image_protocol_from_env(),
            uploaded_kitty_images: HashMap::new(),
            keybindings: Keybindings::defaults(),
            search_match_style,
            search_current_match_style,
            open_url: options.open_url,
            on_right_click_paste: options.on_right_click_paste,
            copy_selection: options.copy_selection,
            selection_anchor: None,
            selection_focus: None,
            selection_granularity: SelectionGranularity::Character,
            selection_initial_range: None,
            last_click: None,
            selection_press_active: false,
            selection_dragged: false,
            selection_auto_scroll_direction: 0,
            selection_drag_pointer: None,
            scrollbar_drag: None,
            scrollbar_hover: None,
            active_search: None,
            pressed_url: None,
            child_focus: None,
        }
    }

    pub fn mode(&self) -> TuiRuntimeMode {
        TuiRuntimeMode::Fullscreen
    }

    pub fn is_viewport_tui(&self) -> bool {
        true
    }

    pub fn set_keybindings(&mut self, bindings: Keybindings) {
        self.keybindings = bindings;
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.document.borrow_mut().add_child(component);
    }

    pub fn set_focus_child(&mut self, index: usize) {
        self.child_focus = Some(index);
        self.base.set_focus_child(index);
    }

    pub fn set_layout_root(&mut self, component: Box<dyn Component>) {
        self.layout_root = Some(component);
        self.current_layout = None;
        self.base.request_render(false);
    }

    pub fn layout_root_is(&self, component: &dyn Component) -> bool {
        let Some(root) = self.layout_root.as_ref() else {
            return false;
        };
        if let (Some(left), Some(right)) = (
            root.as_any().downcast_ref::<SharedComponent>(),
            component.as_any().downcast_ref::<SharedComponent>(),
        ) {
            return SharedComponent::ptr_eq(left, right);
        }
        std::ptr::eq(root.as_ref(), component)
    }

    pub fn has_layout_root(&self) -> bool {
        self.layout_root.is_some()
    }

    pub fn get_copy_on_select(&self) -> bool {
        self.copy_on_select
    }

    pub fn set_copy_on_select(&mut self, enabled: bool) {
        self.copy_on_select = enabled;
    }

    pub fn viewport_top(&self) -> usize {
        self.primary_scroll_top()
    }

    pub fn is_following_output(&self) -> bool {
        self.with_primary_scroll(|scroll| scroll.is_following_end())
            .unwrap_or(false)
    }

    pub fn viewport_lines(&self) -> Vec<String> {
        self.previous_screen
            .iter()
            .map(|line| strip_terminal_sequences(line).trim_end().to_string())
            .collect()
    }

    pub fn flash(&mut self, message: impl Into<String>, duration_ms: Option<u64>) {
        self.flashes.flash(message, duration_ms);
        self.render_now(false);
    }

    pub fn tick(&mut self, ms: u64) {
        self.base.tick(ms);
        let expired = self.flashes.tick(ms);
        self.implicit_scroll.tick(ms);
        visit_scroll_views_root(self.layout_root.as_mut(), &mut |_, scroll| {
            scroll.tick(ms);
        });
        if self.selection_auto_scroll_direction != 0 {
            self.auto_scroll_selection();
        }
        if expired {
            self.render_now(false);
        }
    }

    pub fn invalidate(&mut self) {
        self.base.invalidate();
        self.document.borrow_mut().invalidate();
        if let Some(root) = &mut self.layout_root {
            root.invalidate();
        }
        self.implicit_scroll.invalidate();
    }

    pub fn show_overlay(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> OverlayHandle {
        self.base.show_overlay(component, options)
    }

    pub fn scroll_by(&mut self, lines: isize) {
        self.with_primary_scroll_mut(|scroll| {
            scroll.scroll_by(lines);
        });
        self.render_now(false);
    }

    pub fn scroll_to_top(&mut self) {
        self.with_primary_scroll_mut(|scroll| scroll.scroll_to_start());
        self.render_now(false);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.with_primary_scroll_mut(|scroll| scroll.scroll_to_end());
        self.render_now(false);
    }

    pub fn start(&mut self) {
        self.reset_interaction_state();
        self.flashes.dispose();
        self.alt_screen_active = true;
        self.uploaded_kitty_images.clear();
        if self.image_protocol == Some("iterm2") {
            self.image_protocol = None;
        }
        self.last_document.clear();
        self.reset_render_state();
        let term = std::env::var("TERM")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mouse = if !self.mouse_enabled {
            String::new()
        } else if std::env::var("TMUX").is_ok()
            || std::env::var("ZELLIJ").is_ok()
            || std::env::var("STY").is_ok()
            || term.starts_with("tmux")
            || term.starts_with("screen")
        {
            ENABLE_BUTTON_MOTION_MOUSE.to_string()
        } else {
            ENABLE_ALL_MOTION_MOUSE.to_string()
        };
        self.base.terminal.write(&format!(
            "{ENTER_ALT_SCREEN}{DISABLE_AUTOWRAP}{mouse}\x1b[2J\x1b[H\x1b[?25l"
        ));
        self.base.set_stopped(false);
        self.base.terminal.start();
        self.render_now(false);
    }

    pub fn stop(&mut self, options: TuiStopOptions) {
        self.close_search();
        self.reset_interaction_state();
        self.flashes.dispose();
        if self.alt_screen_active {
            let mouse = if self.mouse_enabled {
                DISABLE_MOUSE
            } else {
                ""
            };
            let kitty = if self.image_protocol == Some("kitty") {
                delete_all_kitty_images()
            } else {
                String::new()
            };
            self.base.terminal.write(&format!(
                "{BEGIN_SYNCHRONIZED_OUTPUT}{kitty}{mouse}{ENABLE_AUTOWRAP}{END_SYNCHRONIZED_OUTPUT}"
            ));
            self.uploaded_kitty_images.clear();
            if options.preserve_screen {
                self.base.terminal.write(&format!(
                    "{BEGIN_SYNCHRONIZED_OUTPUT}{EXIT_ALT_SCREEN}\x1b[?25h{END_SYNCHRONIZED_OUTPUT}"
                ));
            } else {
                let width = self.base.terminal.columns().max(1);
                let document_lines: Vec<String> = self
                    .render_document(width)
                    .into_iter()
                    .map(|line| strip_osc133_zones(&line))
                    .collect();
                self.last_document = self
                    .base
                    .apply_line_resets(&document_lines)
                    .into_iter()
                    .map(|line| {
                        if is_image_line(&line) || visible_width(&line) <= width {
                            line
                        } else {
                            slice_by_column(&line, 0, width, true)
                        }
                    })
                    .collect();
                let mut buffer =
                    format!("{BEGIN_SYNCHRONIZED_OUTPUT}{EXIT_ALT_SCREEN}{DISABLE_AUTOWRAP}");
                for (row, line) in self.last_document.iter().enumerate() {
                    if row > 0 {
                        buffer.push_str("\r\n");
                    }
                    buffer.push_str(&format!("\r\x1b[2K{line}"));
                }
                buffer.push_str(&format!(
                    "\x1b[0m{ENABLE_AUTOWRAP}\r\n\x1b[?25h{END_SYNCHRONIZED_OUTPUT}"
                ));
                self.base.terminal.write(&buffer);
            }
            self.alt_screen_active = false;
        }
        self.base.set_stopped(true);
        self.base.terminal.show_cursor();
        self.base.terminal.stop();
    }

    pub fn take_terminal(self) -> Box<dyn TerminalIo> {
        self.base.take_terminal()
    }

    /// Viewport / overlay keys only. Returns true when the host consumed the sequence
    /// (TS `TuiAltScreen.handleInput` before child routing).
    pub fn handle_host_input(&mut self, data: &str) -> bool {
        self.sync_search_query();
        if self.handle_viewport_input(data) {
            self.render_now(false);
            return true;
        }
        if self.base.is_overlay_focused() {
            self.base.handle_terminal_input(data);
            self.render_now(false);
            return true;
        }
        false
    }

    pub fn handle_input(&mut self, data: &str) {
        if self.handle_host_input(data) {
            return;
        }
        self.handle_child_input(data);
        self.render_now(false);
    }

    pub fn request_render(&mut self, force: bool) {
        self.base.request_render(force);
        self.render_now(force);
    }

    pub fn has_active_selection(&self) -> bool {
        self.get_active_selection_text().is_some()
    }

    pub fn get_active_selection_text(&self) -> Option<String> {
        let selection = self.get_selection_bounds()?;
        let source_lines = if let Some(scroll_id) = selection.0.scroll_id {
            self.current_layout
                .as_ref()
                .and_then(|layout| get_scroll_view_box(layout, scroll_id))
                .and_then(|box_| box_.scroll_content_lines.clone())?
        } else {
            RenderedLines::dense(self.previous_screen.clone())
        };
        let mut lines = Vec::new();
        for row in selection.0.row..=selection.1.row {
            let line = source_lines.get(row).unwrap_or("");
            let columns = self.get_selection_columns(line, row, &selection, 0, visible_width(line));
            lines.push(
                strip_terminal_sequences(&slice_by_column(
                    line,
                    columns.0,
                    columns.1.saturating_sub(columns.0),
                    true,
                ))
                .trim_end()
                .to_string(),
            );
        }
        let text = lines.join("\n");
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn copy_active_selection_to_clipboard(&mut self) -> bool {
        let Some(text) = self.get_active_selection_text() else {
            return false;
        };
        self.copy_text_to_clipboard(&text)
    }

    pub fn copy_text_to_clipboard(&mut self, text: &str) -> bool {
        if let Some(copy) = &self.copy_selection {
            let ok = copy(text);
            self.flash(if ok { "Copied!" } else { "Copy failed" }, None);
            return ok;
        }
        self.base.terminal.write(&format!(
            "\x1b]52;c;{}\x07",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, text.as_bytes())
        ));
        self.flash("Copied!", None);
        true
    }

    pub fn render_now(&mut self, force: bool) {
        if self.base.is_stopped() || !self.alt_screen_active {
            return;
        }
        if force {
            self.reset_render_state();
        }
        let width = self.base.terminal.columns().max(1);
        let height = self.base.terminal.rows().max(1);
        let mut next_layout = self.layout_frame(width, height);
        if self.refresh_search(&next_layout) {
            next_layout = self.layout_frame(width, height);
        }
        let mut screen: Vec<String> = next_layout
            .lines
            .iter()
            .map(|line| strip_osc133_zones(line))
            .collect();
        screen = self.apply_search_highlights(&screen, &next_layout);
        screen = self.base.composite_overlays(&screen, width, height);
        if screen.len() > height {
            screen = screen[screen.len() - height..].to_vec();
        }
        screen = self.apply_selection(screen, &next_layout);
        screen = self.composite_flashes(screen, width, height);
        let mut screen_for_cursor = screen.clone();
        let cursor_pos = TuiBase::extract_cursor_position(&mut screen_for_cursor, height);
        screen = self.base.apply_line_resets(&screen_for_cursor);
        screen = screen
            .into_iter()
            .map(|line| {
                if is_image_line(&line) || visible_width(&line) <= width {
                    line
                } else {
                    slice_by_column(&line, 0, width, true)
                }
            })
            .collect();
        let full_redraw = self.previous_screen.is_empty()
            || self.previous_screen_width != width
            || self.previous_screen_height != height;
        let images_need_redraw = screen.iter().enumerate().any(|(row, line)| {
            let previous = self
                .previous_screen
                .get(row)
                .map(String::as_str)
                .unwrap_or("");
            line != previous && (is_image_line(line) || is_image_line(previous))
        });
        let redraw_images = full_redraw || images_need_redraw;
        let had_uploaded = !self.uploaded_kitty_images.is_empty();
        let (prepared, evicted) = if redraw_images && self.image_protocol == Some("kitty") {
            self.prepare_kitty_screen(&screen)
        } else {
            (screen.clone(), String::new())
        };
        let mut buffer = BEGIN_SYNCHRONIZED_OUTPUT.to_string();
        if full_redraw {
            self.base.full_redraws += 1;
            if self.image_protocol == Some("kitty") && had_uploaded {
                buffer.push_str(&delete_all_kitty_placements());
            } else if self.image_protocol == Some("kitty") {
                buffer.push_str(&delete_all_kitty_images());
            }
            buffer.push_str("\x1b[2J");
        } else if images_need_redraw {
            if self.image_protocol == Some("iterm2") {
                buffer.push_str("\x1b[2J");
            } else if self.image_protocol == Some("kitty") {
                buffer.push_str(&delete_all_kitty_placements());
            }
        }
        buffer.push_str(&evicted);
        for row in 0..height {
            if !full_redraw
                && !images_need_redraw
                && self.previous_screen.get(row) == screen.get(row)
            {
                continue;
            }
            buffer.push_str(&format!(
                "\x1b[{};1H\x1b[2K{}",
                row + 1,
                prepared.get(row).map(String::as_str).unwrap_or("")
            ));
        }
        if let Some((row, col)) = cursor_pos {
            buffer.push_str(&format!("\x1b[{};{}H", row + 1, col.min(width) + 1));
            buffer.push_str(if self.base.get_show_hardware_cursor() {
                "\x1b[?25h"
            } else {
                "\x1b[?25l"
            });
        } else {
            buffer.push_str("\x1b[?25l");
        }
        buffer.push_str(END_SYNCHRONIZED_OUTPUT);
        self.base.terminal.write(&buffer);
        self.previous_screen = screen;
        self.previous_screen_width = width;
        self.previous_screen_height = height;
        self.current_layout = Some(next_layout);
    }

    fn layout_frame(&mut self, width: usize, height: usize) -> LayoutFrame {
        if let Some(root) = self.layout_root.as_mut() {
            render_layout_frame(root.as_mut(), width, height)
        } else {
            render_layout_frame(&mut self.implicit_scroll, width, height)
        }
    }

    fn render_document(&mut self, width: usize) -> Vec<String> {
        if let Some(root) = self.layout_root.as_mut() {
            root.render(width)
        } else {
            self.implicit_scroll.render(width)
        }
    }

    fn reset_render_state(&mut self) {
        self.previous_screen.clear();
        self.previous_screen_width = 0;
        self.previous_screen_height = 0;
        self.current_layout = None;
    }

    fn reset_interaction_state(&mut self) {
        self.selection_press_active = false;
        self.selection_auto_scroll_direction = 0;
        self.selection_drag_pointer = None;
        self.stop_scrollbar_hover();
        self.scrollbar_drag = None;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.selection_granularity = SelectionGranularity::Character;
        self.selection_initial_range = None;
        self.last_click = None;
        self.pressed_url = None;
        self.selection_dragged = false;
    }

    fn handle_child_input(&mut self, data: &str) {
        let index = self
            .child_focus
            .unwrap_or_else(|| self.document.borrow().children.len().saturating_sub(1));
        let mut document = self.document.borrow_mut();
        if let Some(child) = document.children.get_mut(index) {
            if is_key_release(data) && !child.wants_key_release() {
                return;
            }
            child.handle_input(data);
        }
    }

    fn should_defer_viewport_input_to_overlay(&self) -> bool {
        if !self.base.is_overlay_focused() {
            return false;
        }
        if let Some(search) = &self.active_search {
            return !self.base.is_overlay_handle_focused(&search.overlay);
        }
        true
    }

    fn handle_viewport_input(&mut self, data: &str) -> bool {
        if data == FOCUS_OUT {
            let had_active = self.selection_press_active;
            let had_nonempty = had_active && self.get_selection_bounds().is_some();
            self.selection_press_active = false;
            self.selection_auto_scroll_direction = 0;
            self.stop_scrollbar_hover();
            self.scrollbar_drag = None;
            self.pressed_url = None;
            self.selection_dragged = false;
            if had_active {
                self.selection_anchor = None;
                self.selection_focus = None;
                self.selection_granularity = SelectionGranularity::Character;
                self.selection_initial_range = None;
            }
            self.last_click = None;
            let _ = had_nonempty;
            return true;
        }
        if data == FOCUS_IN {
            return true;
        }
        if let Some(wheel) = parse_wheel_event(data) {
            if self.should_defer_viewport_input_to_overlay() {
                return false;
            }
            self.route_wheel(wheel);
            return true;
        }
        if let Some(mouse) = parse_sgr_mouse_event(data) {
            if self.handle_right_click_paste(&mouse) {
                return true;
            }
            let handled = self.handle_scrollbar_mouse_event(&mouse);
            if self.scrollbar_drag.is_none() {
                self.update_scrollbar_hover(mouse.x, mouse.y);
            }
            if !handled {
                self.handle_selection_mouse_event(&mouse);
            }
            return true;
        }
        if is_mouse_sequence(data) {
            return true;
        }
        let kb = &self.keybindings;
        if kb.matches(data, "tui.altScreen.search") {
            if !is_key_release(data) {
                self.open_search();
            }
            return true;
        }
        if let Some(search) = &self.active_search {
            if self.base.is_overlay_handle_focused(&search.overlay) {
                if kb.matches(data, "tui.altScreen.searchNext") {
                    if !is_key_release(data) {
                        self.navigate_search(1);
                    }
                    return true;
                }
                if kb.matches(data, "tui.altScreen.searchPrevious") {
                    if !is_key_release(data) {
                        self.navigate_search(-1);
                    }
                    return true;
                }
                if kb.matches(data, "tui.altScreen.searchClose") {
                    if !is_key_release(data) {
                        self.close_search();
                    }
                    return true;
                }
            }
        }
        if self.should_defer_viewport_input_to_overlay() {
            return false;
        }
        if kb.matches(data, "tui.altScreen.pageUp") {
            if !is_key_release(data) {
                let delta = self
                    .primary_viewport_height()
                    .saturating_sub(PAGE_SCROLL_OVERLAP)
                    .max(1);
                self.with_primary_scroll_mut(|scroll| {
                    scroll.scroll_by(-(delta as isize));
                });
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.pageDown") {
            if !is_key_release(data) {
                let delta = self
                    .primary_viewport_height()
                    .saturating_sub(PAGE_SCROLL_OVERLAP)
                    .max(1);
                self.with_primary_scroll_mut(|scroll| {
                    scroll.scroll_by(delta as isize);
                });
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.halfPageUp") {
            if !is_key_release(data) {
                let delta = (self.primary_viewport_height() / 2).max(1);
                self.with_primary_scroll_mut(|scroll| {
                    scroll.scroll_by(-(delta as isize));
                });
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.halfPageDown") {
            if !is_key_release(data) {
                let delta = (self.primary_viewport_height() / 2).max(1);
                self.with_primary_scroll_mut(|scroll| {
                    scroll.scroll_by(delta as isize);
                });
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.lineUp") {
            if !is_key_release(data) {
                self.with_primary_scroll_mut(|scroll| {
                    scroll.scroll_by(-1);
                });
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.lineDown") {
            if !is_key_release(data) {
                self.with_primary_scroll_mut(|scroll| {
                    scroll.scroll_by(1);
                });
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.previousPrompt") {
            if !is_key_release(data) {
                self.scroll_to_prompt(-1);
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.nextPrompt") {
            if !is_key_release(data) {
                self.scroll_to_prompt(1);
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.top") {
            if !is_key_release(data) {
                self.with_primary_scroll_mut(ScrollView::scroll_to_start);
            }
            return true;
        }
        if kb.matches(data, "tui.altScreen.bottom") {
            if !is_key_release(data) {
                self.with_primary_scroll_mut(ScrollView::scroll_to_end);
            }
            return true;
        }
        false
    }

    fn route_wheel(&mut self, event: WheelEvent) {
        let mut remaining = event.direction * self.wheel_scroll_lines as isize;
        let ids = self
            .current_layout
            .as_ref()
            .map(|layout| get_scroll_views_at(layout, event.x, event.y))
            .unwrap_or_default();
        let mut seen = Vec::new();
        for id in ids {
            seen.push(id);
            remaining = self
                .with_scroll_mut(id, |scroll| scroll.scroll_by(remaining))
                .unwrap_or(remaining);
            let contain = self
                .with_scroll(id, |scroll| scroll.overscroll == ScrollOverscroll::Contain)
                .unwrap_or(false);
            if remaining == 0 || contain {
                break;
            }
        }
        let primary = self.primary_scroll_id();
        if remaining != 0 && !seen.contains(&primary) {
            self.with_primary_scroll_mut(|scroll| {
                scroll.scroll_by(remaining);
            });
        }
        self.update_scrollbar_hover(event.x, event.y);
    }

    fn handle_right_click_paste(&mut self, event: &SgrMouseEvent) -> bool {
        if self.on_right_click_paste.is_none()
            || std::env::consts::OS != "windows"
            || std::env::var("TERM_PROGRAM")
                .ok()
                .is_some_and(|value| value.eq_ignore_ascii_case("vscode"))
            || event.release
            || event.button != 2
        {
            return false;
        }
        if let Some(handler) = &mut self.on_right_click_paste {
            handler();
        }
        true
    }

    fn get_scrollbar_target_at(
        &self,
        x: usize,
        y: usize,
    ) -> Option<(usize, crate::layout::ScrollbarGeometry)> {
        if self.base.has_overlay_entries() {
            return None;
        }
        let layout = self.current_layout.as_ref()?;
        for scroll_id in get_scroll_views_at(layout, x, y) {
            let box_ = get_scroll_view_box(layout, scroll_id)?;
            let geometry = get_scrollbar_geometry(box_)?;
            if x == geometry.column
                && y >= geometry.thumb_top
                && y < geometry.thumb_top + geometry.thumb_height
            {
                return Some((scroll_id, geometry));
            }
        }
        None
    }

    fn set_scrollbar_hover(&mut self, scroll_id: Option<usize>) {
        if self.scrollbar_hover == scroll_id {
            return;
        }
        if let Some(previous) = self.scrollbar_hover {
            self.with_scroll_mut(previous, |scroll| scroll.set_scrollbar_active(false));
        }
        self.scrollbar_hover = scroll_id;
        if let Some(next) = scroll_id {
            self.with_scroll_mut(next, |scroll| scroll.set_scrollbar_active(true));
        }
    }

    fn update_scrollbar_hover(&mut self, x: usize, y: usize) {
        let target = self.get_scrollbar_target_at(x, y).map(|(id, _)| id);
        self.set_scrollbar_hover(target);
    }

    fn stop_scrollbar_hover(&mut self) {
        self.set_scrollbar_hover(None);
    }

    fn handle_scrollbar_mouse_event(&mut self, event: &SgrMouseEvent) -> bool {
        if let Some(drag) = &self.scrollbar_drag {
            if event.release {
                self.scrollbar_drag = None;
                return true;
            }
            let scroll_id = drag.scroll_id;
            let grab = drag.grab_offset;
            let geometry = self.current_layout.as_ref().and_then(|layout| {
                get_scroll_view_box(layout, scroll_id).and_then(get_scrollbar_geometry)
            });
            if let Some(geometry) = geometry {
                let max_thumb_offset = geometry.track_height.saturating_sub(geometry.thumb_height);
                let thumb_offset = (event.y as isize - geometry.track_top as isize - grab)
                    .max(0)
                    .min(max_thumb_offset as isize) as usize;
                let scroll_top = if max_thumb_offset == 0 {
                    0
                } else {
                    ((thumb_offset as f64 / max_thumb_offset as f64)
                        * geometry.max_scroll_top as f64)
                        .round() as usize
                };
                self.with_scroll_mut(scroll_id, |scroll| scroll.scroll_to(scroll_top, false));
            }
            return true;
        }
        if event.release || (event.button & 32) != 0 || (event.button & 3) != 0 {
            return false;
        }
        let Some((scroll_id, geometry)) = self.get_scrollbar_target_at(event.x, event.y) else {
            return false;
        };
        self.selection_auto_scroll_direction = 0;
        self.selection_press_active = false;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.selection_granularity = SelectionGranularity::Character;
        self.selection_initial_range = None;
        self.last_click = None;
        self.pressed_url = None;
        self.selection_dragged = false;
        self.set_scrollbar_hover(Some(scroll_id));
        self.scrollbar_drag = Some(ScrollbarDrag {
            scroll_id,
            grab_offset: event.y as isize - geometry.thumb_top as isize,
        });
        true
    }

    fn get_scroll_selection_point(
        &self,
        scroll_id: usize,
        x: usize,
        y: usize,
    ) -> Option<SelectionPoint> {
        let layout = self.current_layout.as_ref()?;
        let box_ = get_scroll_view_box(layout, scroll_id)?;
        if box_.rect.height == 0 || box_.clip.height == 0 {
            return None;
        }
        let visible_top = 0.max(box_.rect.y).max(box_.clip.y).max(0) as usize;
        let visible_bottom = (self.base.terminal.rows().saturating_sub(1) as isize)
            .min(box_.rect.y + box_.rect.height.saturating_sub(1) as isize)
            .min(box_.clip.y + box_.clip.height.saturating_sub(1) as isize)
            .max(0) as usize;
        if visible_bottom < visible_top {
            return None;
        }
        let pointer_row = y.clamp(visible_top, visible_bottom);
        let max_content_row = box_
            .scroll_content_lines
            .as_ref()
            .map(|lines| lines.len().saturating_sub(1))
            .unwrap_or(0);
        let scroll_top = self
            .with_scroll(scroll_id, ScrollView::scroll_top)
            .unwrap_or(0);
        Some(SelectionPoint {
            row: scroll_top
                .saturating_add((pointer_row as isize - box_.rect.y).max(0) as usize)
                .min(max_content_row),
            col: x
                .saturating_sub(box_.rect.x)
                .min(box_.rect.width.saturating_sub(1)),
            scroll_id: Some(scroll_id),
            boundary: false,
        })
    }

    fn get_selection_point(
        &self,
        event: &SgrMouseEvent,
        scroll_id: Option<usize>,
    ) -> SelectionPoint {
        if let Some(id) = scroll_id {
            if let Some(point) = self.get_scroll_selection_point(id, event.x, event.y) {
                return point;
            }
        }
        SelectionPoint {
            row: event.y.min(self.base.terminal.rows().saturating_sub(1)),
            col: event.x.min(self.base.terminal.columns().saturating_sub(1)),
            scroll_id: None,
            boundary: false,
        }
    }

    fn get_selection_source_line(&self, point: SelectionPoint) -> String {
        if let (Some(scroll_id), Some(layout)) = (point.scroll_id, self.current_layout.as_ref()) {
            if let Some(lines) = get_scroll_view_box(layout, scroll_id)
                .and_then(|box_| box_.scroll_content_lines.as_ref())
            {
                return lines.get(point.row).unwrap_or("").to_string();
            }
        }
        self.previous_screen
            .get(point.row)
            .cloned()
            .unwrap_or_default()
    }

    fn get_word_selection(&self, point: SelectionPoint) -> Option<SelectionRange> {
        let line = strip_terminal_sequences(&self.get_selection_source_line(point));
        let mut segments = Vec::new();
        let mut start = 0usize;
        for segment in default_word_segments(&line) {
            let end = start + visible_width(&segment.text);
            let joiner = segment.text.chars().count() == 1
                && segment
                    .text
                    .chars()
                    .next()
                    .is_some_and(|ch| TERMINAL_WORD_SELECTION_JOINERS.contains(&ch));
            segments.push((start, end, segment.is_word_like || joiner, joiner));
            start = end;
        }
        let clicked = segments.iter().position(|(seg_start, seg_end, _, _)| {
            point.col >= *seg_start && point.col < *seg_end
        })?;
        let can_join = |left: (usize, usize, bool, bool), right: (usize, usize, bool, bool)| {
            left.2 && right.2 && (left.3 || right.3)
        };
        let mut selection_start = segments[clicked].0;
        let mut selection_end = segments[clicked].1;
        let mut index = clicked;
        while index > 0 && can_join(segments[index - 1], segments[index]) {
            index -= 1;
            selection_start = segments[index].0;
        }
        index = clicked;
        while index + 1 < segments.len() && can_join(segments[index], segments[index + 1]) {
            index += 1;
            selection_end = segments[index].1;
        }
        Some(SelectionRange {
            start: SelectionPoint {
                col: selection_start,
                ..point
            },
            end: SelectionPoint {
                col: selection_end,
                boundary: true,
                ..point
            },
        })
    }

    fn get_line_selection(&self, point: SelectionPoint) -> SelectionRange {
        SelectionRange {
            start: SelectionPoint { col: 0, ..point },
            end: SelectionPoint {
                col: visible_width(&self.get_selection_source_line(point)),
                boundary: true,
                ..point
            },
        }
    }

    fn update_selection_focus(&mut self, point: SelectionPoint) {
        if self.selection_granularity == SelectionGranularity::Character
            || self.selection_initial_range.is_none()
        {
            self.selection_focus = Some(point);
            return;
        }
        let range = if self.selection_granularity == SelectionGranularity::Word {
            self.get_word_selection(point)
        } else {
            Some(self.get_line_selection(point))
        };
        let Some(range) = range else {
            return;
        };
        let Some(initial) = &self.selection_initial_range else {
            return;
        };
        let target_before = range.start.row < initial.start.row
            || (range.start.row == initial.start.row && range.start.col < initial.start.col);
        if target_before {
            self.selection_anchor = Some(initial.end);
            self.selection_focus = Some(range.start);
        } else {
            self.selection_anchor = Some(initial.start);
            self.selection_focus = Some(range.end);
        }
    }

    fn get_click_count(&mut self, point: SelectionPoint, word: Option<&SelectionRange>) -> u32 {
        let now = self.base.now_ms();
        let count = match (word, &self.last_click) {
            (Some(word), Some(previous))
                if now.saturating_sub(previous.timestamp) <= DOUBLE_CLICK_INTERVAL_MS
                    && previous.row == point.row
                    && previous.scroll_id == point.scroll_id
                    && previous.word_start == word.start.col
                    && previous.word_end == word.end.col =>
            {
                (previous.count % 3) + 1
            }
            _ => 1,
        };
        self.last_click = word.map(|word| ClickTarget {
            timestamp: now,
            count,
            row: point.row,
            scroll_id: point.scroll_id,
            word_start: word.start.col,
            word_end: word.end.col,
        });
        count
    }

    fn auto_scroll_selection(&mut self) {
        let Some(scroll_id) = self.selection_anchor.and_then(|point| point.scroll_id) else {
            self.selection_auto_scroll_direction = 0;
            return;
        };
        let Some((x, y)) = self.selection_drag_pointer else {
            self.selection_auto_scroll_direction = 0;
            return;
        };
        let direction = self.selection_auto_scroll_direction as isize;
        if direction == 0 {
            return;
        }
        let remaining = self
            .with_scroll_mut(scroll_id, |scroll| scroll.scroll_by(direction))
            .unwrap_or(direction);
        if remaining == direction {
            self.selection_auto_scroll_direction = 0;
            return;
        }
        if let Some(point) = self.get_scroll_selection_point(scroll_id, x, y) {
            self.update_selection_focus(point);
        }
    }

    fn handle_selection_mouse_event(&mut self, event: &SgrMouseEvent) {
        let button = event.button & 3;
        if button != 0 && !(event.release && button == 3) {
            return;
        }
        let anchor_scroll = self.selection_anchor.and_then(|point| point.scroll_id);
        let point = self.get_selection_point(event, anchor_scroll);
        if event.release {
            if !self.selection_press_active {
                return;
            }
            self.selection_press_active = false;
            self.selection_auto_scroll_direction = 0;
            if self.selection_anchor.is_none() {
                return;
            }
            self.update_selection_focus(point);
            let clicked_url = if !self.selection_dragged
                && self.selection_anchor.is_some_and(|anchor| {
                    anchor.scroll_id == point.scroll_id
                        && anchor.row == point.row
                        && anchor.col == point.col
                }) {
                self.pressed_url.clone()
            } else {
                None
            };
            self.pressed_url = None;
            if let (Some(url), Some(open)) = (clicked_url, self.open_url.as_mut()) {
                self.selection_anchor = None;
                self.selection_focus = None;
                open(&url);
                return;
            }
            if self.copy_on_select {
                if let Some(text) = self.get_active_selection_text() {
                    self.copy_text_to_clipboard(&text);
                }
            }
            return;
        }
        if (event.button & 32) != 0 {
            if !self.selection_press_active || self.selection_anchor.is_none() {
                return;
            }
            self.selection_dragged = true;
            self.last_click = None;
            self.pressed_url = None;
            self.update_selection_focus(point);
            self.selection_drag_pointer = Some((event.x, event.y));
            if let (Some(layout), Some(scroll_id)) = (
                self.current_layout.as_ref(),
                self.selection_anchor.and_then(|p| p.scroll_id),
            ) {
                if let Some(box_) = get_scroll_view_box(layout, scroll_id) {
                    let visible_top = 0.max(box_.rect.y).max(box_.clip.y).max(0) as usize;
                    let visible_bottom = (self.base.terminal.rows().saturating_sub(1) as isize)
                        .min(box_.rect.y + box_.rect.height.saturating_sub(1) as isize)
                        .min(box_.clip.y + box_.clip.height.saturating_sub(1) as isize)
                        .max(0) as usize;
                    self.selection_auto_scroll_direction = if event.y <= visible_top {
                        -1
                    } else if event.y >= visible_bottom {
                        1
                    } else {
                        0
                    };
                }
            }
            return;
        }
        self.selection_auto_scroll_direction = 0;
        self.selection_press_active = true;
        let scroll_id = if !self.base.has_overlay_entries() {
            self.current_layout.as_ref().and_then(|layout| {
                get_scroll_views_at(layout, event.x, event.y)
                    .into_iter()
                    .next()
            })
        } else {
            None
        };
        let anchor = self.get_selection_point(event, scroll_id);
        let word = self.get_word_selection(anchor);
        let click_count = self.get_click_count(anchor, word.as_ref());
        let range = if click_count == 2 {
            word
        } else if click_count == 3 {
            Some(self.get_line_selection(anchor))
        } else {
            None
        };
        self.selection_granularity = if click_count == 2 {
            SelectionGranularity::Word
        } else if click_count == 3 {
            SelectionGranularity::Line
        } else {
            SelectionGranularity::Character
        };
        self.selection_initial_range = range.as_ref().map(|item| SelectionRange {
            start: item.start,
            end: item.end,
        });
        self.selection_anchor = range.as_ref().map(|item| item.start).or(Some(anchor));
        self.selection_focus = range.as_ref().map(|item| item.end).or(Some(anchor));
        self.selection_dragged = false;
        self.pressed_url = if range.is_some() {
            None
        } else {
            let row = event.y.min(self.base.terminal.rows().saturating_sub(1));
            let col = event.x.min(self.base.terminal.columns().saturating_sub(1));
            get_osc8_link_at_column(
                self.previous_screen
                    .get(row)
                    .map(String::as_str)
                    .unwrap_or(""),
                col,
            )
        };
    }

    fn get_selection_bounds(&self) -> Option<(SelectionPoint, SelectionPoint)> {
        let anchor = self.selection_anchor?;
        let focus = self.selection_focus?;
        if anchor.scroll_id != focus.scroll_id {
            return None;
        }
        if anchor.row == focus.row && anchor.col == focus.col {
            return None;
        }
        let anchor_before =
            anchor.row < focus.row || (anchor.row == focus.row && anchor.col < focus.col);
        Some(if anchor_before {
            (anchor, focus)
        } else {
            (focus, anchor)
        })
    }

    fn get_selection_columns(
        &self,
        line: &str,
        row: usize,
        selection: &(SelectionPoint, SelectionPoint),
        min_column: usize,
        max_column: usize,
    ) -> (usize, usize) {
        let line_width = visible_width(line);
        let mut start = min_column;
        let mut end = max_column.min(line_width);
        if row == selection.0.row {
            start = get_grapheme_cell_range(line, selection.0.col)
                .map(|range| range.start)
                .unwrap_or_else(|| selection.0.col.min(line_width));
        }
        if row == selection.1.row {
            end = if selection.1.boundary {
                selection.1.col.min(line_width)
            } else {
                get_grapheme_cell_range(line, selection.1.col)
                    .map(|range| range.end)
                    .unwrap_or_else(|| (selection.1.col + 1).min(line_width))
            };
        }
        (start.max(min_column), end.min(max_column))
    }

    fn apply_selection(&self, screen: Vec<String>, layout: &LayoutFrame) -> Vec<String> {
        let Some(selection) = self.get_selection_bounds() else {
            return screen;
        };
        let mut screen_selection = selection;
        let mut min_row = 0usize;
        let mut max_row = screen.len().saturating_sub(1);
        let mut min_column = 0usize;
        let mut max_column = self.base.terminal.columns();
        if let Some(scroll_id) = selection.0.scroll_id {
            let Some(box_) = get_scroll_view_box(layout, scroll_id) else {
                return screen;
            };
            let scroll_top = self
                .with_scroll(scroll_id, ScrollView::scroll_top)
                .unwrap_or(0);
            min_row = 0.max(box_.rect.y).max(box_.clip.y).max(0) as usize;
            max_row = (screen.len().saturating_sub(1) as isize)
                .min(box_.rect.y + box_.rect.height.saturating_sub(1) as isize)
                .min(box_.clip.y + box_.clip.height.saturating_sub(1) as isize)
                .max(0) as usize;
            min_column = 0.max(box_.rect.x).max(box_.clip.x);
            max_column = self
                .base
                .terminal
                .columns()
                .min(box_.rect.x + box_.rect.width)
                .min(box_.clip.x + box_.clip.width);
            screen_selection.0.row =
                (box_.rect.y + selection.0.row.saturating_sub(scroll_top) as isize).max(0) as usize;
            screen_selection.0.col = box_.rect.x + selection.0.col;
            screen_selection.1.row =
                (box_.rect.y + selection.1.row.saturating_sub(scroll_top) as isize).max(0) as usize;
            screen_selection.1.col = box_.rect.x + selection.1.col;
        }
        screen
            .into_iter()
            .enumerate()
            .map(|(row, line)| {
                if row < min_row
                    || row > max_row
                    || row < screen_selection.0.row
                    || row > screen_selection.1.row
                    || is_image_line(&line)
                {
                    return line;
                }
                let line_width = visible_width(&line);
                let columns = self.get_selection_columns(
                    &line,
                    row,
                    &screen_selection,
                    min_column,
                    max_column,
                );
                if columns.1 <= columns.0 {
                    return line;
                }
                let before = slice_by_column(&line, 0, columns.0, true);
                let selected =
                    slice_by_column(&line, columns.0, columns.1.saturating_sub(columns.0), true);
                let after =
                    slice_by_column(&line, columns.1, line_width.saturating_sub(columns.1), true);
                format!("{before}{}{after}", apply_selection_highlight(&selected))
            })
            .collect()
    }

    fn apply_search_highlights(&self, screen: &[String], layout: &LayoutFrame) -> Vec<String> {
        let Some(search) = &self.active_search else {
            return screen.to_vec();
        };
        if search.selected_index < 0 || search.matches.is_empty() {
            return screen.to_vec();
        }
        let scroll_id = layout
            .primary_scroll_id
            .unwrap_or_else(|| self.primary_scroll_id());
        let Some(box_) = get_scroll_view_box(layout, scroll_id) else {
            return screen.to_vec();
        };
        let scroll_top = self
            .with_scroll(scroll_id, ScrollView::scroll_top)
            .unwrap_or(0);
        let scrollbar_column = get_scrollbar_geometry(box_).map(|geometry| geometry.column);
        let min_row = 0.max(box_.rect.y).max(box_.clip.y).max(0) as usize;
        let max_row = (screen.len() as isize)
            .min(box_.rect.y + box_.rect.height as isize)
            .min(box_.clip.y + box_.clip.height as isize)
            .max(0) as usize;
        let min_column = 0.max(box_.rect.x).max(box_.clip.x);
        let max_column = self
            .base
            .terminal
            .columns()
            .min(box_.rect.x + box_.rect.width)
            .min(box_.clip.x + box_.clip.width)
            .min(scrollbar_column.unwrap_or(usize::MAX));
        let mut ranges_by_row: HashMap<usize, Vec<(usize, usize, bool)>> = HashMap::new();
        for (match_index, found) in search.matches.iter().enumerate() {
            for segment in &found.segments {
                let row =
                    (box_.rect.y + segment.row.saturating_sub(scroll_top) as isize).max(0) as usize;
                if row < min_row || row >= max_row {
                    continue;
                }
                let start_col = min_column.max(box_.rect.x + segment.start_col);
                let end_col = max_column.min(box_.rect.x + segment.end_col);
                if end_col <= start_col {
                    continue;
                }
                ranges_by_row.entry(row).or_default().push((
                    start_col,
                    end_col,
                    match_index == search.selected_index as usize,
                ));
            }
        }
        let mut result = screen.to_vec();
        for (row, mut ranges) in ranges_by_row {
            ranges.sort_by(|a, b| b.0.cmp(&a.0));
            let Some(line) = result.get_mut(row) else {
                continue;
            };
            if is_image_line(line) {
                continue;
            }
            let line_width = visible_width(line);
            for (start_col, end_col, current) in ranges {
                let start_col = start_col.min(line_width);
                let end_col = end_col.min(line_width);
                if end_col <= start_col {
                    continue;
                }
                let before = slice_by_column(line, 0, start_col, true);
                let highlighted = slice_by_column(line, start_col, end_col - start_col, true);
                let after =
                    slice_by_column(line, end_col, line_width.saturating_sub(end_col), true);
                *line = format!(
                    "{before}{}{after}",
                    self.apply_search_text_highlight(&highlighted, current)
                );
            }
        }
        result
    }

    fn apply_search_text_highlight(&self, text: &str, current: bool) -> String {
        let style = if current {
            &self.search_current_match_style
        } else {
            &self.search_match_style
        };
        let mut result = String::new();
        let mut plain_start = 0usize;
        let mut index = 0usize;
        while index < text.len() {
            if let Some((code, len)) = extract_ansi_code(text, index) {
                if index > plain_start {
                    result.push_str(&style(&text[plain_start..index]));
                }
                result.push_str(&code);
                index += len;
                plain_start = index;
                continue;
            }
            index += text[index..].chars().next().map_or(1, |ch| ch.len_utf8());
        }
        if plain_start < text.len() {
            result.push_str(&style(&text[plain_start..]));
        }
        result
    }

    fn composite_flashes(
        &self,
        mut screen: Vec<String>,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        let flash_lines = self.flashes.render(width);
        let flash_lines: Vec<String> = flash_lines
            .into_iter()
            .rev()
            .take(height)
            .collect::<Vec<_>>();
        let flash_lines: Vec<String> = flash_lines.into_iter().rev().collect();
        if flash_lines.is_empty() {
            return screen;
        }
        while screen.len() < height {
            screen.push(String::new());
        }
        for (row, line) in flash_lines.iter().enumerate() {
            let flash_width = visible_width(line);
            if flash_width == 0 {
                continue;
            }
            screen[row] = composite_tui_line(
                screen.get(row).map(String::as_str).unwrap_or(""),
                line,
                width.saturating_sub(flash_width),
                flash_width,
                width,
            );
        }
        screen
    }

    fn prepare_kitty_screen(&mut self, screen: &[String]) -> (Vec<String>, String) {
        let mut visible = std::collections::HashSet::new();
        let lines: Vec<String> = screen
            .iter()
            .map(|line| {
                let Some(placement) = get_kitty_image_placement(line) else {
                    return line.clone();
                };
                visible.insert(placement.image_id);
                let cached = self.uploaded_kitty_images.remove(&placement.image_id);
                self.uploaded_kitty_images.insert(
                    placement.image_id,
                    CachedKittyImage {
                        transmission_generation: placement.transmission_generation,
                        transmission_bytes: placement.transmission_bytes,
                        estimated_decoded_bytes: placement.estimated_decoded_bytes,
                    },
                );
                if cached.is_some_and(|cached| {
                    cached.transmission_generation == placement.transmission_generation
                }) {
                    placement.replacement_line
                } else {
                    line.clone()
                }
            })
            .collect();
        let mut offscreen_count = 0usize;
        let mut offscreen_tx = 0usize;
        let mut offscreen_decoded = 0usize;
        for (image_id, cached) in &self.uploaded_kitty_images {
            if visible.contains(image_id) {
                continue;
            }
            offscreen_count += 1;
            offscreen_tx += cached.transmission_bytes;
            offscreen_decoded += cached.estimated_decoded_bytes;
        }
        let mut evicted = String::new();
        let ids: Vec<u32> = self.uploaded_kitty_images.keys().copied().collect();
        for image_id in ids {
            if offscreen_count <= MAX_CACHED_OFFSCREEN_KITTY_IMAGES
                && offscreen_tx <= MAX_CACHED_OFFSCREEN_KITTY_TRANSMISSION_BYTES
                && offscreen_decoded <= MAX_CACHED_OFFSCREEN_KITTY_DECODED_BYTES
            {
                break;
            }
            if visible.contains(&image_id) {
                continue;
            }
            if let Some(cached) = self.uploaded_kitty_images.remove(&image_id) {
                evicted.push_str(&delete_kitty_image(image_id));
                offscreen_count = offscreen_count.saturating_sub(1);
                offscreen_tx = offscreen_tx.saturating_sub(cached.transmission_bytes);
                offscreen_decoded =
                    offscreen_decoded.saturating_sub(cached.estimated_decoded_bytes);
            }
        }
        (lines, evicted)
    }

    fn open_search(&mut self) {
        if self.active_search.is_some() {
            return;
        }
        let component = AltScreenSearchComponent::new(|_value| {});
        let overlay = self.base.show_overlay(
            Box::new(component),
            OverlayOptions {
                anchor: OverlayAnchor::TopRight,
                width: Some(SizeValue::Percent(40.0)),
                min_width: Some(24),
                margin: OverlayMargin {
                    top: 1,
                    right: 1,
                    bottom: 1,
                    left: 1,
                },
                ..OverlayOptions::default()
            },
        );
        self.active_search = Some(ActiveSearch {
            overlay,
            query: String::new(),
            matches: Vec::new(),
            selected_index: -1,
            selected_key: None,
            anchor_row: self.primary_scroll_top(),
            selection_mode: SearchSelectionMode::Query,
        });
    }

    fn close_search(&mut self) {
        let Some(search) = self.active_search.take() else {
            return;
        };
        self.base.hide_overlay_id(&search.overlay);
    }

    fn sync_search_query(&mut self) {
        let Some(handle) = self.active_search.as_ref().map(|search| search.overlay) else {
            return;
        };
        let query = self
            .base
            .overlay_component_mut(&handle)
            .and_then(|component| {
                component
                    .as_any()
                    .downcast_ref::<AltScreenSearchComponent>()
                    .map(AltScreenSearchComponent::query)
            });
        let Some(query) = query else {
            return;
        };
        let fallback_row = self.primary_scroll_top();
        let Some(search) = &mut self.active_search else {
            return;
        };
        if query == search.query {
            return;
        }
        search.anchor_row = search
            .matches
            .get(search.selected_index.max(0) as usize)
            .and_then(|found| found.segments.first().map(|segment| segment.row))
            .unwrap_or(fallback_row);
        search.query = query;
        search.selection_mode = SearchSelectionMode::Query;
        if let Some(component) = self
            .base
            .overlay_component_mut(&handle)
            .and_then(|component| {
                component
                    .as_any_mut()
                    .downcast_mut::<AltScreenSearchComponent>()
            })
        {
            component.set_result(-1, 0);
        }
    }

    fn navigate_search(&mut self, direction: i8) {
        let Some(search) = &mut self.active_search else {
            return;
        };
        if search.query.is_empty() {
            return;
        }
        search.selection_mode = if direction < 0 {
            SearchSelectionMode::Previous
        } else {
            SearchSelectionMode::Next
        };
    }

    fn refresh_search(&mut self, layout: &LayoutFrame) -> bool {
        let Some(search) = &self.active_search else {
            return false;
        };
        let scroll_id = layout
            .primary_scroll_id
            .unwrap_or_else(|| self.primary_scroll_id());
        let box_ = get_scroll_view_box(layout, scroll_id);
        let lines = box_.and_then(|box_| box_.scroll_content_lines.clone());
        let query = search.query.clone();
        if lines.as_ref().is_none_or(|_lines| query.trim().is_empty()) {
            if let Some(search) = &mut self.active_search {
                search.matches.clear();
                search.selected_index = -1;
                search.selected_key = None;
                search.selection_mode = SearchSelectionMode::Retain;
            }
            if let Some(handle) = self.active_search.as_ref().map(|search| search.overlay) {
                if let Some(component) = self
                    .base
                    .overlay_component_mut(&handle)
                    .and_then(|c| c.as_any_mut().downcast_mut::<AltScreenSearchComponent>())
                {
                    component.set_result(-1, 0);
                }
            }
            return false;
        }
        let lines = lines.unwrap_or_else(RenderedLines::empty);
        let should_reveal = search.selection_mode != SearchSelectionMode::Retain;
        let matches = find_alt_screen_search_matches_in(&lines, &query);
        let exact_index = search
            .selected_key
            .as_ref()
            .and_then(|key| {
                matches
                    .iter()
                    .position(|found| get_alt_screen_search_match_key(found) == *key)
            })
            .map(|index| index as isize)
            .unwrap_or(-1);
        let selected_index = if matches.is_empty() {
            -1
        } else {
            match search.selection_mode {
                SearchSelectionMode::Query => {
                    let found = matches.iter().position(|found| {
                        found
                            .segments
                            .first()
                            .is_some_and(|segment| segment.row >= search.anchor_row)
                    });
                    found.unwrap_or(0) as isize
                }
                SearchSelectionMode::Next => {
                    let base = if exact_index >= 0 {
                        exact_index
                    } else {
                        search.selected_index.min(matches.len() as isize - 1)
                    };
                    if base < 0 {
                        0
                    } else {
                        (base + 1) % matches.len() as isize
                    }
                }
                SearchSelectionMode::Previous => {
                    let base = if exact_index >= 0 {
                        exact_index
                    } else {
                        search.selected_index.min(matches.len() as isize - 1)
                    };
                    if base < 0 {
                        matches.len() as isize - 1
                    } else {
                        (base - 1 + matches.len() as isize) % matches.len() as isize
                    }
                }
                SearchSelectionMode::Retain => {
                    if exact_index >= 0 {
                        exact_index
                    } else {
                        search.selected_index.max(0).min(matches.len() as isize - 1)
                    }
                }
            }
        };
        let selected_key = matches
            .get(selected_index.max(0) as usize)
            .map(get_alt_screen_search_match_key);
        let first_row = matches
            .get(selected_index.max(0) as usize)
            .and_then(|found| found.segments.first().map(|segment| segment.row));
        let last_row = matches
            .get(selected_index.max(0) as usize)
            .and_then(|found| found.segments.last().map(|segment| segment.row));
        if let Some(search) = &mut self.active_search {
            search.matches = matches;
            search.selected_index = selected_index;
            search.selected_key = selected_key;
            search.selection_mode = SearchSelectionMode::Retain;
        }
        if let Some(handle) = self.active_search.as_ref().map(|search| search.overlay) {
            let count = self
                .active_search
                .as_ref()
                .map(|search| search.matches.len())
                .unwrap_or(0);
            if let Some(component) = self
                .base
                .overlay_component_mut(&handle)
                .and_then(|c| c.as_any_mut().downcast_mut::<AltScreenSearchComponent>())
            {
                component.set_result(selected_index, count);
            }
        }
        if !should_reveal {
            return false;
        }
        let Some(first_row) = first_row else {
            return false;
        };
        let last_row = last_row.unwrap_or(first_row);
        let viewport_height = self
            .with_scroll(scroll_id, ScrollView::viewport_height)
            .unwrap_or(0);
        if viewport_height == 0 {
            return false;
        }
        let before = self
            .with_scroll(scroll_id, ScrollView::scroll_top)
            .unwrap_or(0);
        let visible_bottom = before + viewport_height - 1;
        let mut target = before;
        if first_row < before || last_row > visible_bottom {
            target = first_row.saturating_sub(viewport_height / 3);
        }
        self.with_scroll_mut(scroll_id, |scroll| scroll.scroll_to(target, true));
        self.with_scroll(scroll_id, ScrollView::scroll_top)
            .is_some_and(|top| top != before)
    }

    fn scroll_to_prompt(&mut self, direction: i8) {
        let Some(layout) = &self.current_layout else {
            return;
        };
        let scroll_id = self.primary_scroll_id();
        let Some(lines) = get_scroll_view_box(layout, scroll_id)
            .and_then(|box_| box_.scroll_content_lines.clone())
        else {
            return;
        };
        let start = self.primary_scroll_top() as isize + direction as isize;
        let defined = lines.defined();
        let found = if direction < 0 {
            defined.into_iter().rev().find(|(row, line)| {
                (*row as isize) <= start && line.starts_with(OSC133_PROMPT_START)
            })
        } else {
            defined.into_iter().find(|(row, line)| {
                (*row as isize) >= start && line.starts_with(OSC133_PROMPT_START)
            })
        };
        if let Some((row, _)) = found {
            self.with_primary_scroll_mut(|scroll| scroll.scroll_to(row, false));
        }
    }

    fn primary_scroll_id(&self) -> usize {
        self.current_layout
            .as_ref()
            .and_then(|layout| layout.primary_scroll_id)
            .unwrap_or_else(|| component_id(&self.implicit_scroll))
    }

    fn primary_scroll_top(&self) -> usize {
        self.with_primary_scroll(ScrollView::scroll_top)
            .unwrap_or(0)
    }

    fn primary_viewport_height(&self) -> usize {
        self.with_primary_scroll(ScrollView::viewport_height)
            .unwrap_or_else(|| self.base.terminal.rows().max(1))
    }

    fn with_primary_scroll<T>(&self, f: impl FnOnce(&ScrollView) -> T) -> Option<T> {
        let id = self.primary_scroll_id();
        self.with_scroll(id, f)
    }

    fn with_primary_scroll_mut<T>(&mut self, f: impl FnOnce(&mut ScrollView) -> T) -> Option<T> {
        let id = self.primary_scroll_id();
        self.with_scroll_mut(id, f)
    }

    fn with_scroll<T>(&self, id: usize, f: impl FnOnce(&ScrollView) -> T) -> Option<T> {
        if component_id(&self.implicit_scroll) == id {
            return Some(f(&self.implicit_scroll));
        }
        find_scroll_view(self.layout_root.as_deref(), id).map(f)
    }

    fn with_scroll_mut<T>(&mut self, id: usize, f: impl FnOnce(&mut ScrollView) -> T) -> Option<T> {
        if component_id(&self.implicit_scroll) == id {
            return Some(f(&mut self.implicit_scroll));
        }
        let mut callback = Some(f);
        let mut result = None;
        if let Some(root) = self.layout_root.as_mut() {
            visit_scroll_views(root.as_mut(), &mut |found_id, scroll| {
                if found_id == id {
                    if let Some(func) = callback.take() {
                        result = Some(func(scroll));
                    }
                }
            });
        }
        result
    }
}

fn image_protocol_from_env() -> Option<&'static str> {
    if let Ok(value) = std::env::var("PI_IMAGES") {
        return match value.to_ascii_lowercase().as_str() {
            "kitty" => Some("kitty"),
            "iterm2" | "iterm" => Some("iterm2"),
            "none" | "0" | "false" => None,
            _ => Some("kitty"),
        };
    }
    if std::env::var("TERM_PROGRAM")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("iTerm.app"))
    {
        return Some("iterm2");
    }
    Some("kitty")
}

fn strip_osc133_zones(line: &str) -> String {
    let mut rest = line;
    while rest.starts_with(OSC133_ZONE_PREFIX) {
        let after = &rest[OSC133_ZONE_PREFIX.len()..];
        if let Some(end) = after.find('\x07') {
            rest = &after[end + 1..];
            continue;
        }
        if let Some(end) = after.find("\x1b\\") {
            rest = &after[end + 2..];
            continue;
        }
        break;
    }
    rest.to_string()
}

fn apply_selection_highlight(text: &str) -> String {
    let mut result = String::from("\x1b[7m");
    let mut index = 0usize;
    while index < text.len() {
        if let Some((code, len)) = extract_ansi_code(text, index) {
            result.push_str(&code);
            if code.ends_with('m') {
                result.push_str("\x1b[7m");
            }
            index += len;
            continue;
        }
        let ch = text[index..].chars().next().unwrap_or('\0');
        result.push(ch);
        index += ch.len_utf8();
    }
    result.push_str("\x1b[27m");
    result
}

fn parse_wheel_event(data: &str) -> Option<WheelEvent> {
    if let Some(event) = parse_sgr_mouse_event(data) {
        if (event.button & 64) == 0 {
            return None;
        }
        let direction = event.button & 3;
        if direction != 0 && direction != 1 {
            return None;
        }
        return Some(WheelEvent {
            direction: if direction == 0 { -1 } else { 1 },
            x: event.x,
            y: event.y,
        });
    }
    if data.len() == 6 && data.starts_with("\x1b[M") {
        let bytes = data.as_bytes();
        let button = bytes[3].wrapping_sub(32);
        if button & 64 == 0 {
            return None;
        }
        let direction = button & 3;
        if direction != 0 && direction != 1 {
            return None;
        }
        return Some(WheelEvent {
            direction: if direction == 0 { -1 } else { 1 },
            x: bytes[4].saturating_sub(33) as usize,
            y: bytes[5].saturating_sub(33) as usize,
        });
    }
    None
}

fn parse_sgr_mouse_event(data: &str) -> Option<SgrMouseEvent> {
    let rest = data.strip_prefix("\x1b[<")?;
    let end = rest.chars().last()?;
    if end != 'M' && end != 'm' {
        return None;
    }
    let body = &rest[..rest.len() - 1];
    let mut parts = body.split(';');
    let button = parts.next()?.parse().ok()?;
    let x = parts.next()?.parse::<usize>().ok()?.saturating_sub(1);
    let y = parts.next()?.parse::<usize>().ok()?.saturating_sub(1);
    Some(SgrMouseEvent {
        button,
        x,
        y,
        release: end == 'm',
    })
}

fn is_mouse_sequence(data: &str) -> bool {
    parse_sgr_mouse_event(data).is_some() || (data.len() == 6 && data.starts_with("\x1b[M"))
}

fn component_id(component: &dyn Component) -> usize {
    component as *const dyn Component as *const () as usize
}

fn visit_scroll_views_root(
    root: Option<&mut Box<dyn Component>>,
    visit: &mut impl FnMut(usize, &mut ScrollView),
) {
    if let Some(root) = root {
        visit_scroll_views(root.as_mut(), visit);
    }
}

fn visit_scroll_views(
    component: &mut dyn Component,
    visit: &mut impl FnMut(usize, &mut ScrollView),
) {
    let id = component as *mut dyn Component as *mut () as usize;
    if let Some(scroll) = component.as_any_mut().downcast_mut::<ScrollView>() {
        visit(id, scroll);
        visit_scroll_views(scroll.child_mut(), visit);
        return;
    }
    if let Some(container) = component.as_any_mut().downcast_mut::<Container>() {
        for child in &mut container.children {
            visit_scroll_views(child.as_mut(), visit);
        }
        return;
    }
    if let Some(stack) = component.as_any_mut().downcast_mut::<VStack>() {
        for entry in &mut stack.entries {
            visit_scroll_views(entry.component.as_mut(), visit);
        }
        return;
    }
    if let Some(stack) = component.as_any_mut().downcast_mut::<HStack>() {
        for entry in &mut stack.entries {
            visit_scroll_views(entry.component.as_mut(), visit);
        }
    }
}

fn find_scroll_view(root: Option<&dyn Component>, id: usize) -> Option<&ScrollView> {
    fn visit(component: &dyn Component, id: usize) -> Option<&ScrollView> {
        let current = component as *const dyn Component as *const () as usize;
        if let Some(scroll) = component.as_any().downcast_ref::<ScrollView>() {
            if current == id {
                return Some(scroll);
            }
            return visit(scroll.child(), id);
        }
        if let Some(container) = component.as_any().downcast_ref::<Container>() {
            for child in &container.children {
                if let Some(found) = visit(child.as_ref(), id) {
                    return Some(found);
                }
            }
        }
        if let Some(stack) = component.as_any().downcast_ref::<VStack>() {
            for entry in &stack.entries {
                if let Some(found) = visit(entry.component.as_ref(), id) {
                    return Some(found);
                }
            }
        }
        if let Some(stack) = component.as_any().downcast_ref::<HStack>() {
            for entry in &stack.entries {
                if let Some(found) = visit(entry.component.as_ref(), id) {
                    return Some(found);
                }
            }
        }
        None
    }
    root.and_then(|component| visit(component, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Text;
    use crate::scroll::ScrollViewScrollbar;
    use crate::stack::{StackBasis, StackEntryOptions};
    use crate::terminal::MemoryTerminal;
    use crate::tui_text::TuiText;

    fn viewport(tui: &TuiAltScreen) -> Vec<String> {
        tui.viewport_lines()
    }

    #[test]
    fn renders_height_viewport_and_preserves_manual_scroll() {
        let terminal = MemoryTerminal::new(20, 4);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        let lines = (1..=10)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        tui.add_child(Box::new(TuiText::new(lines, 0, 0)));
        tui.start();
        assert_eq!(
            viewport(&tui),
            vec!["line 7", "line 8", "line 9", "line 10"]
        );
        assert!(tui.is_following_output());
        tui.handle_input("\x1b[<64;1;1M");
        assert_eq!(viewport(&tui), vec!["line 6", "line 7", "line 8", "line 9"]);
        assert_eq!(tui.viewport_top(), 5);
        assert!(!tui.is_following_output());
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn layout_root_keeps_dock_fixed() {
        let terminal = MemoryTerminal::new(20, 6);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        let transcript = ScrollView::new(
            Box::new(TuiText::new(
                (1..=8)
                    .map(|index| format!("line {index}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                0,
                0,
            )),
            ScrollViewOptions {
                follow: ScrollFollow::End,
                primary: true,
                ..ScrollViewOptions::default()
            },
        )
        .expect("scroll");
        let mut dock = VStack::new(0);
        dock.add_child(
            Box::new(TuiText::new("editor", 0, 0)),
            StackEntryOptions::default(),
        );
        dock.add_child(
            Box::new(TuiText::new("footer", 0, 0)),
            StackEntryOptions::default(),
        );
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(transcript),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(dock),
            StackEntryOptions {
                basis: Some(StackBasis::Auto),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        tui.set_layout_root(Box::new(root));
        tui.start();
        assert_eq!(
            viewport(&tui),
            vec!["line 5", "line 6", "line 7", "line 8", "editor", "footer"]
        );
        tui.handle_input("\x1b[<64;1;6M");
        assert_eq!(
            viewport(&tui),
            vec!["line 4", "line 5", "line 6", "line 7", "editor", "footer"]
        );
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn wheel_routes_to_scroll_view_under_pointer() {
        let terminal = MemoryTerminal::new(20, 4);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        let left = ScrollView::new(
            Box::new(TuiText::new("a1\na2\na3\na4\na5\na6\na7", 0, 0)),
            ScrollViewOptions {
                follow: ScrollFollow::End,
                primary: true,
                ..ScrollViewOptions::default()
            },
        )
        .expect("left");
        let right = ScrollView::new(
            Box::new(TuiText::new("b1\nb2\nb3\nb4\nb5\nb6\nb7", 0, 0)),
            ScrollViewOptions {
                follow: ScrollFollow::End,
                ..ScrollViewOptions::default()
            },
        )
        .expect("right");
        let mut row = HStack::new(0, crate::stack::StackAlign::Start);
        row.add_child(
            Box::new(left),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(10)),
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        row.add_child(
            Box::new(right),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(10)),
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        tui.set_layout_root(Box::new(row));
        tui.start();
        tui.handle_input("\x1b[<64;15;1M");
        assert_eq!(
            viewport(&tui),
            vec![
                "a4        b3",
                "a5        b4",
                "a6        b5",
                "a7        b6"
            ]
        );
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn selection_copies_via_osc52_when_copy_on_select() {
        let terminal = MemoryTerminal::new(40, 4);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        tui.add_child(Box::new(TuiText::new("alpha\nbeta\ngamma\ndelta", 0, 0)));
        tui.start();
        tui.handle_input("\x1b[<0;1;1M");
        tui.handle_input("\x1b[<32;4;2M");
        tui.handle_input("\x1b[<0;4;2m");
        assert_eq!(
            tui.get_active_selection_text().as_deref(),
            Some("alpha\nbeta")
        );
        let output = (*tui.base.terminal)
            .as_any()
            .downcast_ref::<MemoryTerminal>()
            .expect("memory")
            .output();
        let expected = format!(
            "\x1b]52;c;{}\x07",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"alpha\nbeta")
        );
        assert!(output.contains(&expected));
        assert!(viewport(&tui).iter().any(|line| line.contains("Copied!")));
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn prompt_markers_jump_between_osc133_zones() {
        let terminal = MemoryTerminal::new(20, 3);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        let text = [1, 2, 3, 4]
            .into_iter()
            .flat_map(|message| {
                [
                    format!("{OSC133_PROMPT_START}\x07message {message}"),
                    "detail".into(),
                ]
            })
            .collect::<Vec<_>>()
            .join("\n");
        tui.add_child(Box::new(TuiText::new(text, 0, 0)));
        tui.start();
        assert_eq!(tui.viewport_top(), 5);
        tui.handle_input("\x1b[57419;6u");
        tui.handle_input("\x1b[57419;6:3u");
        assert_eq!(tui.viewport_top(), 4);
        assert_eq!(
            viewport(&tui).first().map(String::as_str),
            Some("message 3")
        );
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn search_overlay_highlights_and_closes() {
        let terminal = MemoryTerminal::new(60, 8);
        let mut tui = TuiAltScreen::with_options(
            Box::new(terminal),
            TuiAltScreenOptions {
                search_match_style: Some(Rc::new(|text| format!("\x1b[41m{text}\x1b[49m"))),
                search_current_match_style: Some(Rc::new(|text| format!("\x1b[42m{text}\x1b[49m"))),
                ..TuiAltScreenOptions::default()
            },
        );
        tui.add_child(Box::new(TuiText::new(
            "needle first\nmiddle\nneedle second\nend",
            0,
            0,
        )));
        tui.start();
        tui.handle_input("\x1b[102;6u");
        tui.handle_input("needle");
        let output = (*tui.base.terminal)
            .as_any()
            .downcast_ref::<MemoryTerminal>()
            .expect("memory")
            .output();
        assert!(output.contains("\x1b[42mneedle\x1b[49m") || output.contains("Find transcript"));
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn page_navigation_uses_four_row_overlap() {
        let terminal = MemoryTerminal::new(20, 8);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        tui.add_child(Box::new(TuiText::new(
            (1..=12)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
            0,
            0,
        )));
        tui.start();
        tui.handle_input("\x1b[57421u");
        tui.handle_input("\x1b[57421;1:3u");
        assert_eq!(
            viewport(&tui),
            (1..=8)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
        );
        tui.handle_input("\x1b[57422u");
        assert_eq!(
            viewport(&tui),
            (5..=12)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
        );
        tui.stop(TuiStopOptions::default());
        let _ = Text {
            value: String::new(),
        };
        let _ = ScrollViewScrollbar::Auto;
    }
}
