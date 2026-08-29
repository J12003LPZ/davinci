//! TypeScript `TuiAltScreen`: search, selection, scrollbar, flashes, OSC 133.

use crate::ansi_text::{
    extract_ansi_code, get_grapheme_cell_range, get_osc8_link_at_column, is_osc133_prompt_start,
    slice_by_column, strip_osc133_zone_prefix, strip_terminal_sequences,
};
use crate::constrained_layout::{
    get_scroll_view_box, get_scroll_views_at, get_scrollbar_geometry, render_layout_frame,
    LayoutFrame, LayoutVStack, Node, StackEntry,
};
use crate::diff::{
    apply_line_resets, composite_tui_line, extract_cursor_position, visible_width, SYNC_BEGIN,
    SYNC_END,
};
use crate::keybindings::{is_key_release, matches_alt_screen, AltScreenAction};
use crate::mouse::overlay_rect;
use crate::terminal_image::{
    delete_all_kitty_images, delete_all_kitty_placements, get_capabilities, is_image_line,
    prepare_kitty_screen, set_capabilities, CachedKittyImage, ImageProtocol, TerminalCapabilities,
};
use crate::viewport::{ViewportScroll, ViewportScrollOptions};
use crate::widgets::{Input, CURSOR_MARKER};
use crate::TuiMode;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

const ENTER_ALT_SCREEN: &str = "\u{1b}[?1049h";
const EXIT_ALT_SCREEN: &str = "\u{1b}[?1049l";
const DISABLE_AUTOWRAP: &str = "\u{1b}[?7l";
const ENABLE_AUTOWRAP: &str = "\u{1b}[?7h";
const ENABLE_BUTTON_MOTION_MOUSE: &str = "\u{1b}[?1000h\u{1b}[?1002h\u{1b}[?1004h\u{1b}[?1006h";
const ENABLE_ALL_MOTION_MOUSE: &str =
    "\u{1b}[?1000h\u{1b}[?1002h\u{1b}[?1003h\u{1b}[?1004h\u{1b}[?1006h";
const DISABLE_MOUSE: &str = "\u{1b}[?1006l\u{1b}[?1004l\u{1b}[?1003l\u{1b}[?1002l\u{1b}[?1000l";
const FOCUS_IN: &str = "\u{1b}[I";
const FOCUS_OUT: &str = "\u{1b}[O";
const PAGE_SCROLL_OVERLAP: usize = 4;
const DOUBLE_CLICK_INTERVAL_MS: u64 = 500;
const WORD_JOINERS: &[char] = &['/', '-'];

#[derive(Debug, Clone)]
pub struct AltScreenSearchSegment {
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone)]
pub struct AltScreenSearchMatch {
    pub segments: Vec<AltScreenSearchSegment>,
}

pub fn find_alt_screen_search_matches(lines: &[String], query: &str) -> Vec<AltScreenSearchMatch> {
    let normalized = normalize_query(query);
    if normalized.is_empty() {
        return Vec::new();
    }
    let (corpus, source) = build_search_corpus(lines);
    let haystack = corpus.to_ascii_lowercase();
    let needle = normalized.to_ascii_lowercase();
    let mut matches = Vec::new();
    let mut start = 0usize;
    while let Some(found) = haystack[start..].find(&needle) {
        let index = start + found;
        let end = index + needle.len();
        let mut segments: Vec<AltScreenSearchSegment> = Vec::new();
        for span in source.iter().take(end).skip(index).flatten() {
            if let Some(previous) = segments.last_mut() {
                if previous.row == span.0 && span.1 <= previous.end_col {
                    previous.end_col = previous.end_col.max(span.2);
                    continue;
                }
            }
            segments.push(AltScreenSearchSegment {
                row: span.0,
                start_col: span.1,
                end_col: span.2,
            });
        }
        if !segments.is_empty() {
            matches.push(AltScreenSearchMatch { segments });
        }
        start = index + 1;
    }
    matches
}

pub fn get_alt_screen_search_match_key(match_: &AltScreenSearchMatch) -> String {
    let Some(first) = match_.segments.first() else {
        return String::new();
    };
    let last = match_.segments.last().unwrap();
    format!(
        "{}:{}:{}:{}",
        first.row, first.start_col, last.row, last.end_col
    )
}

fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

type SearchSourceSpan = Option<(usize, usize, usize)>;

fn build_search_corpus(lines: &[String]) -> (String, Vec<SearchSourceSpan>) {
    let mut text = String::new();
    let mut source = Vec::new();
    let mut pending_separator = false;
    for (row, line) in lines.iter().enumerate() {
        let line = strip_terminal_sequences(line);
        let mut column = 0usize;
        let mut rest = line.as_str();
        while !rest.is_empty() {
            let Some((segment, next)) = crate::ansi_text::next_grapheme(rest) else {
                break;
            };
            let width = crate::ansi_text::grapheme_width(segment);
            if segment.chars().all(char::is_whitespace) {
                if !text.is_empty() {
                    pending_separator = true;
                }
                column += width;
                rest = next;
                continue;
            }
            if pending_separator {
                text.push(' ');
                source.push(None);
                pending_separator = false;
            }
            for ch in segment.chars() {
                text.push(ch);
                source.push(Some((row, column, column + width)));
            }
            column += width;
            rest = next;
        }
        if !text.is_empty() {
            pending_separator = true;
        }
    }
    (text, source)
}

struct SearchComponent {
    input: Input,
    result_count: usize,
    result_index: isize,
}

impl SearchComponent {
    fn new() -> Self {
        Self {
            input: Input::new(""),
            result_count: 0,
            result_index: -1,
        }
    }

    fn set_result(&mut self, index: isize, count: usize) {
        self.result_index = index;
        self.result_count = count;
    }

    fn render(&self, width: usize) -> Vec<String> {
        let safe = width.max(1);
        let label = " Find transcript";
        let query = self.input.get_value();
        let status = if query.is_empty() {
            String::new()
        } else if self.result_count == 0 {
            "No matches ".into()
        } else {
            format!("{}/{} ", self.result_index + 1, self.result_count)
        };
        let gap = " ".repeat(
            safe.saturating_sub(visible_width(label))
                .saturating_sub(visible_width(&status))
                .max(1),
        );
        let title = crate::diff::truncate_to_width(&format!("{label}{gap}{status}"), safe, "");
        let padding = " ".repeat(safe.saturating_sub(visible_width(&title)));
        let mut input = self.input.clone();
        input.focused = true;
        let mut lines = vec![format!("\u{1b}[7m{title}{padding}\u{1b}[27m")];
        let value = self.input.get_value();
        lines.push(format!("> {value}"));
        lines
    }
}

struct FlashEntry {
    message: String,
    expire_at: u64,
}

#[derive(Debug, Clone, Copy)]
struct SelectionPoint {
    row: usize,
    col: usize,
    scroll: Option<usize>,
    boundary: bool,
}

#[derive(Debug, Clone, Copy)]
enum Granularity {
    Character,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy)]
enum SearchMode {
    Query,
    Retain,
    Next,
    Previous,
}

pub struct TuiAltScreenOptions {
    pub wheel_scroll_lines: usize,
    pub mouse: bool,
    pub search_match_style: fn(&str) -> String,
    pub search_current_match_style: fn(&str) -> String,
    pub copy_on_select: bool,
    pub copy_selection: Option<fn(&str) -> bool>,
    pub open_url: Option<fn(&str)>,
    pub on_right_click_paste: Option<fn()>,
    pub platform_windows: bool,
    pub term_program_vscode: bool,
}

#[derive(Default, Clone, Copy)]
pub struct OverlayShowOptions {
    pub non_capturing: bool,
}

#[derive(Clone)]
pub struct InputOverlay {
    inner: Rc<RefCell<InputOverlayState>>,
}

#[derive(Default)]
struct InputOverlayState {
    focused: bool,
    inputs: Vec<String>,
}

impl Default for InputOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl InputOverlay {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(InputOverlayState::default())),
        }
    }

    pub fn focused(&self) -> bool {
        self.inner.borrow().focused
    }

    pub fn inputs(&self) -> Vec<String> {
        self.inner.borrow().inputs.clone()
    }

    fn set_focused(&self, focused: bool) {
        self.inner.borrow_mut().focused = focused;
    }

    fn push_input(&self, data: &str) {
        self.inner.borrow_mut().inputs.push(data.to_string());
    }

    fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

pub struct OverlayHandle {
    id: u64,
}

struct OverlayEntry {
    id: u64,
    overlay: InputOverlay,
    hidden: bool,
    non_capturing: bool,
    pre_focus: Option<InputOverlay>,
    focus_order: u64,
}

impl Default for TuiAltScreenOptions {
    fn default() -> Self {
        Self {
            wheel_scroll_lines: 1,
            mouse: true,
            search_match_style: |text| format!("\u{1b}[4m{text}\u{1b}[24m"),
            search_current_match_style: |text| format!("\u{1b}[1;7m{text}\u{1b}[22;27m"),
            copy_on_select: true,
            copy_selection: None,
            open_url: None,
            on_right_click_paste: None,
            platform_windows: false,
            term_program_vscode: false,
        }
    }
}

pub struct TuiAltScreen {
    pub mode: TuiMode,
    pub columns: usize,
    pub rows: usize,
    pub writes: Vec<String>,
    children: Vec<Node>,
    layout_root: Option<Node>,
    implicit_scroll: ViewportScroll,
    implicit_document: Node,
    current_layout: Option<LayoutFrame>,
    previous_screen: Vec<String>,
    flashes: Vec<FlashEntry>,
    now_ms: u64,
    options: TuiAltScreenOptions,
    alt_active: bool,
    image_protocol: Option<ImageProtocol>,
    saved_capabilities: Option<TerminalCapabilities>,
    uploaded_kitty: IndexMap<u32, CachedKittyImage>,
    selection_anchor: Option<SelectionPoint>,
    selection_focus: Option<SelectionPoint>,
    selection_granularity: Granularity,
    selection_initial: Option<(SelectionPoint, SelectionPoint)>,
    last_click: Option<(u64, usize, Option<usize>, usize, usize, u8)>,
    selection_press_active: bool,
    selection_dragged: bool,
    selection_auto_dir: i8,
    selection_drag_pointer: Option<(usize, usize)>,
    last_auto_scroll_ms: u64,
    scrollbar_drag: Option<(usize, isize)>,
    scrollbar_hover: Option<ViewportScroll>,
    search: Option<ActiveSearch>,
    pressed_url: Option<String>,
    focus_inputs: Rc<RefCell<Vec<String>>>,
    focused: Option<InputOverlay>,
    overlays: Vec<OverlayEntry>,
    next_overlay_id: u64,
    focus_order_counter: u64,
    copy_on_select: bool,
    opened_urls: Vec<String>,
    right_click_pastes: usize,
    copy_log: Rc<RefCell<Vec<String>>>,
    use_copy_handler: bool,
    copy_ok: bool,
}

struct ActiveSearch {
    component: SearchComponent,
    query: String,
    matches: Vec<AltScreenSearchMatch>,
    selected_index: isize,
    selected_key: Option<String>,
    anchor_row: usize,
    mode: SearchMode,
}

impl TuiAltScreen {
    pub fn new(columns: usize, rows: usize, options: TuiAltScreenOptions) -> Self {
        let document = Node::Text(Rc::new(RefCell::new(String::new())));
        let implicit = ViewportScroll::new(
            document.clone(),
            ViewportScrollOptions {
                follow_end: true,
                primary: true,
                ..ViewportScrollOptions::default()
            },
        );
        let copy_on_select = options.copy_on_select;
        Self {
            mode: TuiMode::Fullscreen,
            columns,
            rows,
            writes: Vec::new(),
            children: Vec::new(),
            layout_root: None,
            implicit_scroll: implicit,
            implicit_document: document,
            current_layout: None,
            previous_screen: Vec::new(),
            flashes: Vec::new(),
            now_ms: 0,
            options,
            alt_active: false,
            image_protocol: None,
            saved_capabilities: None,
            uploaded_kitty: IndexMap::new(),
            selection_anchor: None,
            selection_focus: None,
            selection_granularity: Granularity::Character,
            selection_initial: None,
            last_click: None,
            selection_press_active: false,
            selection_dragged: false,
            selection_auto_dir: 0,
            selection_drag_pointer: None,
            last_auto_scroll_ms: 0,
            scrollbar_drag: None,
            scrollbar_hover: None,
            search: None,
            pressed_url: None,
            focus_inputs: Rc::new(RefCell::new(Vec::new())),
            focused: None,
            overlays: Vec::new(),
            next_overlay_id: 1,
            focus_order_counter: 0,
            copy_on_select,
            opened_urls: Vec::new(),
            right_click_pastes: 0,
            copy_log: Rc::new(RefCell::new(Vec::new())),
            use_copy_handler: false,
            copy_ok: true,
        }
    }

    pub fn focus_inputs(&self) -> Rc<RefCell<Vec<String>>> {
        Rc::clone(&self.focus_inputs)
    }

    pub fn set_focus_editor(&mut self) {
        self.blur_focus();
    }

    pub fn set_focus(&mut self, overlay: &InputOverlay) {
        self.blur_focus();
        overlay.set_focused(true);
        self.focused = Some(overlay.clone());
    }

    pub fn set_copy_handler(&mut self, ok: bool) -> Rc<RefCell<Vec<String>>> {
        self.use_copy_handler = true;
        self.copy_ok = ok;
        self.copy_log.borrow_mut().clear();
        Rc::clone(&self.copy_log)
    }

    pub fn show_overlay(
        &mut self,
        overlay: InputOverlay,
        options: OverlayShowOptions,
    ) -> OverlayHandle {
        self.focus_order_counter += 1;
        let id = self.next_overlay_id;
        self.next_overlay_id += 1;
        let pre_focus = self.focused.clone();
        self.overlays.push(OverlayEntry {
            id,
            overlay: overlay.clone(),
            hidden: false,
            non_capturing: options.non_capturing,
            pre_focus,
            focus_order: self.focus_order_counter,
        });
        if !options.non_capturing {
            self.set_focus(&overlay);
        }
        OverlayHandle { id }
    }

    pub fn hide_overlay(&mut self, handle: &OverlayHandle) {
        let Some(index) = self.overlays.iter().position(|entry| entry.id == handle.id) else {
            return;
        };
        let entry = self.overlays.remove(index);
        entry.overlay.set_focused(false);
        if self
            .focused
            .as_ref()
            .is_some_and(|focused| focused.ptr_eq(&entry.overlay))
        {
            let next = self
                .topmost_visible_overlay()
                .map(|entry| entry.overlay.clone())
                .or(entry.pre_focus);
            self.blur_focus();
            if let Some(next) = next {
                next.set_focused(true);
                self.focused = Some(next);
            }
        }
    }

    pub fn set_overlay_hidden(&mut self, handle: &OverlayHandle, hidden: bool) {
        let Some(index) = self.overlays.iter().position(|entry| entry.id == handle.id) else {
            return;
        };
        if self.overlays[index].hidden == hidden {
            return;
        }
        self.overlays[index].hidden = hidden;
        let overlay = self.overlays[index].overlay.clone();
        let non_capturing = self.overlays[index].non_capturing;
        let pre_focus = self.overlays[index].pre_focus.clone();
        if hidden {
            overlay.set_focused(false);
            if self
                .focused
                .as_ref()
                .is_some_and(|focused| focused.ptr_eq(&overlay))
            {
                let next = self
                    .topmost_visible_overlay()
                    .map(|entry| entry.overlay.clone())
                    .or(pre_focus);
                self.blur_focus();
                if let Some(next) = next {
                    next.set_focused(true);
                    self.focused = Some(next);
                }
            }
        } else if !non_capturing {
            self.focus_order_counter += 1;
            self.overlays[index].focus_order = self.focus_order_counter;
            self.set_focus(&overlay);
        }
    }

    pub fn unfocus_overlay(&mut self, handle: &OverlayHandle) {
        let Some(entry) = self.overlays.iter().find(|entry| entry.id == handle.id) else {
            return;
        };
        let overlay = entry.overlay.clone();
        let pre_focus = entry.pre_focus.clone();
        let is_focused = self
            .focused
            .as_ref()
            .is_some_and(|focused| focused.ptr_eq(&overlay));
        if !is_focused {
            return;
        }
        overlay.set_focused(false);
        let next = self
            .topmost_visible_overlay()
            .filter(|top| !top.overlay.ptr_eq(&overlay))
            .map(|top| top.overlay.clone())
            .or(pre_focus);
        self.blur_focus();
        if let Some(next) = next {
            next.set_focused(true);
            self.focused = Some(next);
        }
    }

    fn blur_focus(&mut self) {
        if let Some(previous) = self.focused.take() {
            previous.set_focused(false);
        }
    }

    fn topmost_visible_overlay(&self) -> Option<&OverlayEntry> {
        self.overlays
            .iter()
            .filter(|entry| !entry.hidden && !entry.non_capturing)
            .max_by_key(|entry| entry.focus_order)
    }

    fn deliver_overlay_input(&self, data: &str) -> bool {
        if let Some(focused) = &self.focused {
            if self
                .overlays
                .iter()
                .any(|entry| !entry.hidden && entry.overlay.ptr_eq(focused))
            {
                focused.push_input(data);
                return true;
            }
        }
        false
    }

    fn has_visible_overlay(&self) -> bool {
        self.search.is_some() || self.overlays.iter().any(|entry| !entry.hidden)
    }

    pub fn add_child(&mut self, node: Node) {
        self.children.push(node);
        self.sync_document();
    }

    pub fn set_layout_root(&mut self, root: Node) {
        self.layout_root = Some(root);
        self.current_layout = None;
    }

    fn sync_document(&mut self) {
        if let Node::Text(inner) = &self.implicit_document {
            let joined = self
                .children
                .iter()
                .flat_map(|child| child.render(self.columns.max(1)))
                .collect::<Vec<_>>()
                .join("\n");
            *inner.borrow_mut() = joined;
        }
    }

    pub fn viewport_top(&self) -> usize {
        self.primary_scroll().scroll_top()
    }

    pub fn is_following_output(&self) -> bool {
        self.primary_scroll().is_following_end()
    }

    fn primary_scroll(&self) -> ViewportScroll {
        self.current_layout
            .as_ref()
            .and_then(|layout| layout.primary_scroll_view.clone())
            .unwrap_or_else(|| self.implicit_scroll.clone())
    }

    pub fn start(&mut self) {
        self.alt_active = true;
        let capabilities = get_capabilities();
        self.image_protocol = capabilities.images;
        self.uploaded_kitty.clear();
        if capabilities.images == Some(ImageProtocol::ITerm2) {
            self.saved_capabilities = Some(capabilities);
            set_capabilities(TerminalCapabilities {
                images: None,
                ..capabilities
            });
        }
        self.previous_screen.clear();
        self.selection_anchor = None;
        self.selection_focus = None;
        let term = std::env::var("TERM")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mouse = if std::env::var("TMUX").is_ok()
            || std::env::var("ZELLIJ").is_ok()
            || std::env::var("STY").is_ok()
            || term.starts_with("tmux")
            || term.starts_with("screen")
        {
            ENABLE_BUTTON_MOTION_MOUSE
        } else {
            ENABLE_ALL_MOTION_MOUSE
        };
        let mouse = if self.options.mouse { mouse } else { "" };
        self.write(format!(
            "{ENTER_ALT_SCREEN}{DISABLE_AUTOWRAP}{mouse}\u{1b}[2J\u{1b}[H\u{1b}[?25l"
        ));
        self.render_now();
    }

    pub fn stop(&mut self) {
        self.close_search();
        self.flashes.clear();
        if !self.alt_active {
            return;
        }
        let delete = if self.image_protocol == Some(ImageProtocol::Kitty) {
            delete_all_kitty_images()
        } else {
            String::new()
        };
        let mouse = if self.options.mouse {
            DISABLE_MOUSE
        } else {
            ""
        };
        self.write(format!(
            "{SYNC_BEGIN}{delete}{mouse}{ENABLE_AUTOWRAP}{SYNC_END}"
        ));
        let width = self.columns.max(1);
        let document = self
            .render_document(width)
            .into_iter()
            .map(|line| strip_osc133_zone_prefix(&line).replace(CURSOR_MARKER, ""))
            .collect::<Vec<_>>();
        let mut buffer = format!("{SYNC_BEGIN}{EXIT_ALT_SCREEN}{DISABLE_AUTOWRAP}");
        for (row, line) in document.iter().enumerate() {
            if row > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\r\u{1b}[2K");
            buffer.push_str(line);
        }
        buffer.push_str(&format!(
            "\u{1b}[0m{ENABLE_AUTOWRAP}\r\n\u{1b}[?25h{SYNC_END}"
        ));
        self.write(buffer);
        self.alt_active = false;
        if let Some(saved) = self.saved_capabilities.take() {
            set_capabilities(saved);
        }
    }

    fn render_document(&self, width: usize) -> Vec<String> {
        if let Some(root) = &self.layout_root {
            root.render(width)
        } else {
            self.children
                .iter()
                .flat_map(|child| child.render(width))
                .collect()
        }
    }

    fn write(&mut self, data: impl Into<String>) {
        self.writes.push(data.into());
    }

    pub fn viewport(&self) -> Vec<String> {
        self.previous_screen.clone()
    }

    pub fn flash(&mut self, message: impl Into<String>, duration_ms: u64) {
        self.flashes.push(FlashEntry {
            message: message.into(),
            expire_at: self.now_ms.saturating_add(duration_ms),
        });
        self.render_now();
    }

    pub fn advance_ms(&mut self, delta: u64) {
        let previous = self.now_ms;
        self.now_ms = self.now_ms.saturating_add(delta);
        let now = self.now_ms;
        self.flashes.retain(|entry| entry.expire_at > now);
        let primary = self.primary_scroll();
        let mut dirty = primary.tick(now);
        if let Some(hover) = &self.scrollbar_hover {
            dirty |= hover.tick(now);
        }
        if self.selection_auto_dir != 0 && self.selection_press_active {
            let mut tick_at = self.last_auto_scroll_ms.saturating_add(50);
            while tick_at <= now {
                if tick_at > previous {
                    self.auto_scroll_selection();
                    dirty = true;
                }
                self.last_auto_scroll_ms = tick_at;
                tick_at = tick_at.saturating_add(50);
                if self.selection_auto_dir == 0 {
                    break;
                }
            }
        }
        let _ = dirty;
        self.render_now();
    }

    pub fn scroll_to_bottom(&mut self) {
        self.primary_scroll().scroll_to_end(self.now_ms);
        self.render_now();
    }

    pub fn has_active_selection(&self) -> bool {
        self.active_selection_text().is_some()
    }

    pub fn copy_active_selection(&mut self) -> bool {
        let Some(text) = self.active_selection_text() else {
            return false;
        };
        self.copy_text(&text)
    }

    pub fn set_copy_on_select(&mut self, enabled: bool) {
        self.copy_on_select = enabled;
    }

    pub fn opened_urls(&self) -> &[String] {
        &self.opened_urls
    }

    pub fn right_click_pastes(&self) -> usize {
        self.right_click_pastes
    }

    pub fn handle_input(&mut self, data: &str) -> bool {
        if data == FOCUS_OUT {
            let had = self.selection_press_active;
            let nonempty = had && self.selection_bounds().is_some();
            self.selection_press_active = false;
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            self.stop_scrollbar_hover();
            self.scrollbar_drag = None;
            self.pressed_url = None;
            self.selection_dragged = false;
            if had {
                self.selection_anchor = None;
                self.selection_focus = None;
                if nonempty {
                    self.render_now();
                }
            }
            self.last_click = None;
            return true;
        }
        if data == FOCUS_IN {
            return true;
        }
        if let Some(wheel) = parse_wheel(data) {
            if self.should_defer_overlay() {
                self.deliver_overlay_input(data);
                return true;
            }
            self.route_wheel(wheel.0, wheel.1, wheel.2);
            return true;
        }
        if let Some(mouse) = parse_sgr(data) {
            if self.handle_right_click(mouse) {
                return true;
            }
            let handled = self.handle_scrollbar_mouse(mouse);
            if self.scrollbar_drag.is_none() {
                self.update_scrollbar_hover(mouse.1, mouse.2);
            }
            if !handled {
                self.handle_selection_mouse(mouse);
            }
            self.render_now();
            return true;
        }
        if is_mouse_sequence(data) {
            return true;
        }

        if matches_alt_screen(data, AltScreenAction::Search) {
            if !is_key_release(data) {
                self.open_search();
            }
            return true;
        }
        if self.search.is_some() {
            if matches_alt_screen(data, AltScreenAction::SearchNext) {
                if !is_key_release(data) {
                    self.navigate_search(1);
                }
                return true;
            }
            if matches_alt_screen(data, AltScreenAction::SearchPrevious) {
                if !is_key_release(data) {
                    self.navigate_search(-1);
                }
                return true;
            }
            if matches_alt_screen(data, AltScreenAction::SearchClose) {
                if !is_key_release(data) {
                    self.close_search();
                    self.render_now();
                }
                return true;
            }
            if !is_key_release(data) {
                let fallback_row = self.primary_scroll().scroll_top();
                if let Some(search) = &mut self.search {
                    let previous = search.component.input.get_value().to_string();
                    search.component.input.handle_input(data);
                    let query = search.component.input.get_value().to_string();
                    if query != previous {
                        search.anchor_row = search
                            .matches
                            .get(search.selected_index.max(0) as usize)
                            .and_then(|m| m.segments.first().map(|s| s.row))
                            .unwrap_or(fallback_row);
                        search.query = query;
                        search.mode = SearchMode::Query;
                        search.component.set_result(-1, 0);
                    }
                }
                self.render_now();
            }
            return true;
        }
        if self.should_defer_overlay() {
            self.deliver_overlay_input(data);
            return true;
        }

        let scroll = self.primary_scroll();
        let viewport = scroll.viewport_height().max(1);
        let consumed = if matches_alt_screen(data, AltScreenAction::PageUp) {
            Some(-(viewport.saturating_sub(PAGE_SCROLL_OVERLAP).max(1) as isize))
        } else if matches_alt_screen(data, AltScreenAction::PageDown) {
            Some(viewport.saturating_sub(PAGE_SCROLL_OVERLAP).max(1) as isize)
        } else if matches_alt_screen(data, AltScreenAction::HalfPageUp) {
            Some(-((viewport / 2).max(1) as isize))
        } else if matches_alt_screen(data, AltScreenAction::HalfPageDown) {
            Some((viewport / 2).max(1) as isize)
        } else if matches_alt_screen(data, AltScreenAction::LineUp) {
            Some(-1)
        } else if matches_alt_screen(data, AltScreenAction::LineDown) {
            Some(1)
        } else {
            None
        };
        if let Some(delta) = consumed {
            if !is_key_release(data) {
                scroll.scroll_by(delta, self.now_ms);
                self.render_now();
            }
            return true;
        }
        if matches_alt_screen(data, AltScreenAction::PreviousPrompt) {
            if !is_key_release(data) {
                self.scroll_to_prompt(-1);
            }
            return true;
        }
        if matches_alt_screen(data, AltScreenAction::NextPrompt) {
            if !is_key_release(data) {
                self.scroll_to_prompt(1);
            }
            return true;
        }
        if matches_alt_screen(data, AltScreenAction::Top) {
            if !is_key_release(data) {
                scroll.scroll_to_start(self.now_ms);
                self.render_now();
            }
            return true;
        }
        if matches_alt_screen(data, AltScreenAction::Bottom) {
            if !is_key_release(data) {
                scroll.scroll_to_end(self.now_ms);
                self.render_now();
            }
            return true;
        }
        self.focus_inputs.borrow_mut().push(data.to_string());
        false
    }

    fn should_defer_overlay(&self) -> bool {
        self.search.is_none()
            && self.focused.as_ref().is_some_and(|focused| {
                self.overlays.iter().any(|entry| {
                    !entry.hidden && !entry.non_capturing && entry.overlay.ptr_eq(focused)
                })
            })
    }

    fn open_search(&mut self) {
        if self.search.is_some() {
            return;
        }
        self.search = Some(ActiveSearch {
            component: SearchComponent::new(),
            query: String::new(),
            matches: Vec::new(),
            selected_index: -1,
            selected_key: None,
            anchor_row: self.primary_scroll().scroll_top(),
            mode: SearchMode::Query,
        });
        self.render_now();
    }

    fn close_search(&mut self) {
        self.search = None;
    }

    fn navigate_search(&mut self, direction: i8) {
        if let Some(search) = &mut self.search {
            if search.query.is_empty() {
                return;
            }
            search.mode = if direction < 0 {
                SearchMode::Previous
            } else {
                SearchMode::Next
            };
        }
        self.render_now();
    }

    fn scroll_to_prompt(&mut self, direction: i8) {
        let Some(layout) = &self.current_layout else {
            return;
        };
        let scroll = self.primary_scroll();
        let Some(box_) = get_scroll_view_box(layout, &scroll) else {
            return;
        };
        let Some(lines) = &box_.scroll_content_lines else {
            return;
        };
        let mut row = scroll.scroll_top() as isize + direction as isize;
        while row >= 0 && (row as usize) < lines.len() {
            if is_osc133_prompt_start(&lines[row as usize]) {
                scroll.scroll_to(row as usize, false, self.now_ms);
                self.render_now();
                return;
            }
            row += direction as isize;
        }
    }

    fn route_wheel(&mut self, direction: i8, x: usize, y: usize) {
        let mut remaining = direction as isize * self.options.wheel_scroll_lines as isize;
        let mut seen = Vec::new();
        if let Some(layout) = &self.current_layout {
            for scroll in get_scroll_views_at(layout, x, y) {
                remaining = scroll.scroll_by(remaining, self.now_ms);
                seen.push(scroll);
                if remaining == 0 {
                    break;
                }
                if seen.last().is_some_and(|s| {
                    s.inner.borrow().overscroll == crate::viewport::Overscroll::Contain
                }) {
                    break;
                }
            }
        }
        let primary = self.primary_scroll();
        if remaining != 0 && !seen.iter().any(|s| s.ptr_eq(&primary)) {
            primary.scroll_by(remaining, self.now_ms);
        }
        self.update_scrollbar_hover(x, y);
        self.render_now();
    }

    fn handle_right_click(&mut self, event: (u16, usize, usize, bool)) -> bool {
        if self.options.on_right_click_paste.is_none()
            || !self.options.platform_windows
            || self.options.term_program_vscode
            || std::env::var("TERM_PROGRAM")
                .map(|v| v.eq_ignore_ascii_case("vscode"))
                .unwrap_or(false)
            || event.3
            || event.0 != 2
        {
            return false;
        }
        self.right_click_pastes += 1;
        if let Some(handler) = self.options.on_right_click_paste {
            handler();
        }
        true
    }

    fn scroll_id(scroll: &ViewportScroll) -> usize {
        Rc::as_ptr(&scroll.inner) as usize
    }

    fn scroll_by_id(&self, id: usize) -> Option<ViewportScroll> {
        if Self::scroll_id(&self.implicit_scroll) == id {
            return Some(self.implicit_scroll.clone());
        }
        if let Some(layout) = &self.current_layout {
            return find_scroll(&layout.root, id);
        }
        None
    }

    fn handle_scrollbar_mouse(&mut self, event: (u16, usize, usize, bool)) -> bool {
        if let Some((id, grab)) = self.scrollbar_drag {
            if event.3 {
                self.scrollbar_drag = None;
                return true;
            }
            if let (Some(layout), Some(scroll)) = (&self.current_layout, self.scroll_by_id(id)) {
                if let Some(box_) = get_scroll_view_box(layout, &scroll) {
                    if let Some(geometry) = get_scrollbar_geometry(box_) {
                        let max_thumb =
                            geometry.track_height.saturating_sub(geometry.thumb_height) as isize;
                        let thumb_offset = (event.2 as isize - geometry.track_top as isize - grab)
                            .clamp(0, max_thumb.max(0));
                        let scroll_top = if max_thumb == 0 {
                            0
                        } else {
                            ((thumb_offset as usize) * geometry.max_scroll_top
                                + max_thumb as usize / 2)
                                / max_thumb as usize
                        };
                        scroll.scroll_to(scroll_top, false, self.now_ms);
                    }
                }
            }
            return true;
        }
        if event.3 || (event.0 & 32) != 0 || (event.0 & 3) != 0 {
            return false;
        }
        let Some(target) = self.scrollbar_target_at(event.1, event.2) else {
            return false;
        };
        self.selection_press_active = false;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.pressed_url = None;
        self.selection_dragged = false;
        target.0.set_scrollbar_active(true, self.now_ms);
        self.scrollbar_hover = Some(target.0.clone());
        self.scrollbar_drag = Some((
            Self::scroll_id(&target.0),
            event.2 as isize - target.1 as isize,
        ));
        true
    }

    fn scrollbar_target_at(&self, x: usize, y: usize) -> Option<(ViewportScroll, usize)> {
        let layout = self.current_layout.as_ref()?;
        if self.search.is_some() {
            return None;
        }
        for scroll in get_scroll_views_at(layout, x, y) {
            if let Some(box_) = get_scroll_view_box(layout, &scroll) {
                if let Some(geometry) = get_scrollbar_geometry(box_) {
                    if x == geometry.column
                        && y >= geometry.thumb_top
                        && y < geometry.thumb_top + geometry.thumb_height
                    {
                        return Some((scroll, geometry.thumb_top));
                    }
                }
            }
        }
        None
    }

    fn update_scrollbar_hover(&mut self, x: usize, y: usize) {
        let next = self.scrollbar_target_at(x, y).map(|(scroll, _)| scroll);
        if let Some(current) = &self.scrollbar_hover {
            if next.as_ref().is_some_and(|n| n.ptr_eq(current)) {
                return;
            }
            current.set_scrollbar_active(false, self.now_ms);
        }
        if let Some(next) = &next {
            next.set_scrollbar_active(true, self.now_ms);
        }
        self.scrollbar_hover = next;
    }

    fn stop_scrollbar_hover(&mut self) {
        if let Some(hover) = self.scrollbar_hover.take() {
            hover.set_scrollbar_active(false, self.now_ms);
        }
    }

    fn handle_selection_mouse(&mut self, event: (u16, usize, usize, bool)) {
        let button = event.0 & 3;
        if button != 0 && !(event.3 && button == 3) {
            return;
        }
        let anchor_scroll = self.selection_anchor.and_then(|p| p.scroll);
        let point = self.selection_point(event.1, event.2, anchor_scroll);
        if event.3 {
            if !self.selection_press_active {
                return;
            }
            self.selection_press_active = false;
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            if self.selection_anchor.is_none() {
                return;
            }
            self.update_selection_focus(point);
            let clicked_url = if !self.selection_dragged
                && self.selection_anchor.is_some_and(|a| {
                    a.scroll == point.scroll && a.row == point.row && a.col == point.col
                }) {
                self.pressed_url.clone()
            } else {
                None
            };
            self.pressed_url = None;
            if let Some(url) = clicked_url {
                self.selection_anchor = None;
                self.selection_focus = None;
                self.opened_urls.push(url.clone());
                if let Some(open) = self.options.open_url {
                    open(&url);
                }
                return;
            }
            if self.copy_on_select {
                if let Some(text) = self.active_selection_text() {
                    self.copy_text(&text);
                }
            }
            return;
        }
        if (event.0 & 32) != 0 {
            if !self.selection_press_active || self.selection_anchor.is_none() {
                return;
            }
            self.selection_dragged = true;
            self.last_click = None;
            self.pressed_url = None;
            self.update_selection_focus(point);
            self.update_selection_auto_scroll(event.1, event.2);
            return;
        }
        self.selection_auto_dir = 0;
        self.selection_drag_pointer = None;
        self.selection_press_active = true;
        let scroll = if self.has_visible_overlay() {
            None
        } else {
            self.current_layout.as_ref().and_then(|layout| {
                get_scroll_views_at(layout, event.1, event.2)
                    .into_iter()
                    .next()
            })
        };
        let anchor = self.selection_point(event.1, event.2, scroll.as_ref().map(Self::scroll_id));
        let word = self.word_selection(anchor);
        let count = self.click_count(anchor, word.as_ref());
        let range = if count == 2 {
            word
        } else if count == 3 {
            Some(self.line_selection(anchor))
        } else {
            None
        };
        self.selection_granularity = if count == 2 {
            Granularity::Word
        } else if count == 3 {
            Granularity::Line
        } else {
            Granularity::Character
        };
        self.selection_initial = range;
        self.selection_anchor = range.map(|r| r.0).or(Some(anchor));
        self.selection_focus = range.map(|r| r.1).or(Some(anchor));
        self.selection_dragged = false;
        self.pressed_url = if range.is_some() {
            None
        } else {
            let line = self
                .previous_screen
                .get(event.2.min(self.rows.saturating_sub(1)))
                .cloned()
                .unwrap_or_default();
            get_osc8_link_at_column(&line, event.1.min(self.columns.saturating_sub(1)))
        };
    }

    fn selection_point(&self, x: usize, y: usize, scroll_id: Option<usize>) -> SelectionPoint {
        if let Some(id) = scroll_id {
            if let Some(scroll) = self.scroll_by_id(id) {
                if let Some(layout) = &self.current_layout {
                    if let Some(box_) = get_scroll_view_box(layout, &scroll) {
                        if box_.rect.height > 0 && box_.clip.height > 0 {
                            let visible_top = as_pos(box_.rect.y.max(box_.clip.y));
                            let visible_bottom = as_pos(
                                (self.rows.saturating_sub(1) as i32)
                                    .min(box_.rect.y + box_.rect.height.saturating_sub(1) as i32)
                                    .min(box_.clip.y + box_.clip.height.saturating_sub(1) as i32),
                            );
                            if visible_bottom >= visible_top {
                                let pointer_row = y.clamp(visible_top, visible_bottom);
                                let max_row = box_
                                    .scroll_content_lines
                                    .as_ref()
                                    .map(|lines| lines.len().saturating_sub(1))
                                    .unwrap_or(0);
                                return SelectionPoint {
                                    row: as_pos(
                                        scroll.scroll_top() as i32 + pointer_row as i32
                                            - box_.rect.y,
                                    )
                                    .min(max_row),
                                    col: as_pos(x as i32 - box_.rect.x)
                                        .min(box_.rect.width.saturating_sub(1)),
                                    scroll: Some(id),
                                    boundary: false,
                                };
                            }
                        }
                    }
                }
            }
        }
        SelectionPoint {
            row: y.min(self.rows.saturating_sub(1)),
            col: x.min(self.columns.saturating_sub(1)),
            scroll: None,
            boundary: false,
        }
    }

    fn source_line(&self, point: SelectionPoint) -> String {
        if let Some(id) = point.scroll {
            if let Some(scroll) = self.scroll_by_id(id) {
                if let Some(layout) = &self.current_layout {
                    if let Some(box_) = get_scroll_view_box(layout, &scroll) {
                        if let Some(lines) = &box_.scroll_content_lines {
                            return lines.get(point.row).cloned().unwrap_or_default();
                        }
                    }
                }
            }
        }
        self.previous_screen
            .get(point.row)
            .cloned()
            .unwrap_or_default()
    }

    fn word_selection(&self, point: SelectionPoint) -> Option<(SelectionPoint, SelectionPoint)> {
        let line = strip_terminal_sequences(&self.source_line(point));
        let segments = word_segments(&line);
        let index = segments
            .iter()
            .position(|(s, e, _, _)| point.col >= *s && point.col < *e)?;
        let can_join = |left: &(usize, usize, bool, bool), right: &(usize, usize, bool, bool)| {
            left.2 && right.2 && (left.3 || right.3)
        };
        let mut sel_start = segments[index].0;
        let mut sel_end = segments[index].1;
        let mut i = index;
        while i > 0 && can_join(&segments[i - 1], &segments[i]) {
            i -= 1;
            sel_start = segments[i].0;
        }
        let mut j = index;
        while j + 1 < segments.len() && can_join(&segments[j], &segments[j + 1]) {
            j += 1;
            sel_end = segments[j].1;
        }
        Some((
            SelectionPoint {
                col: sel_start,
                boundary: false,
                ..point
            },
            SelectionPoint {
                col: sel_end,
                boundary: true,
                ..point
            },
        ))
    }

    fn line_selection(&self, point: SelectionPoint) -> (SelectionPoint, SelectionPoint) {
        let width = visible_width(&self.source_line(point));
        (
            SelectionPoint {
                col: 0,
                boundary: false,
                ..point
            },
            SelectionPoint {
                col: width,
                boundary: true,
                ..point
            },
        )
    }

    fn update_selection_focus(&mut self, point: SelectionPoint) {
        if matches!(self.selection_granularity, Granularity::Character)
            || self.selection_initial.is_none()
        {
            self.selection_focus = Some(point);
            return;
        }
        let range = match self.selection_granularity {
            Granularity::Word => self.word_selection(point),
            Granularity::Line => Some(self.line_selection(point)),
            Granularity::Character => return,
        };
        let Some(range) = range else {
            return;
        };
        let Some(initial) = self.selection_initial else {
            return;
        };
        let before = range.0.row < initial.0.row
            || (range.0.row == initial.0.row && range.0.col < initial.0.col);
        if before {
            self.selection_anchor = Some(initial.1);
            self.selection_focus = Some(range.0);
        } else {
            self.selection_anchor = Some(initial.0);
            self.selection_focus = Some(range.1);
        }
    }

    fn click_count(
        &mut self,
        point: SelectionPoint,
        word: Option<&(SelectionPoint, SelectionPoint)>,
    ) -> u8 {
        let now = self.now_ms;
        let count = if let (Some(word), Some((ts, row, scroll, start, end, prev))) =
            (word, self.last_click)
        {
            if now.saturating_sub(ts) <= DOUBLE_CLICK_INTERVAL_MS
                && row == point.row
                && scroll == point.scroll
                && start == word.0.col
                && end == word.1.col
            {
                (prev % 3) + 1
            } else {
                1
            }
        } else {
            1
        };
        self.last_click =
            word.map(|word| (now, point.row, point.scroll, word.0.col, word.1.col, count));
        count
    }

    fn update_selection_auto_scroll(&mut self, x: usize, y: usize) {
        let Some(id) = self.selection_anchor.and_then(|p| p.scroll) else {
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            return;
        };
        let Some(scroll) = self.scroll_by_id(id) else {
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            return;
        };
        let Some(layout) = &self.current_layout else {
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            return;
        };
        let Some(box_) = get_scroll_view_box(layout, &scroll) else {
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            return;
        };
        let visible_top = as_pos(box_.rect.y.max(box_.clip.y));
        let visible_bottom = as_pos(
            (self.rows.saturating_sub(1) as i32)
                .min(box_.rect.y + box_.rect.height.saturating_sub(1) as i32)
                .min(box_.clip.y + box_.clip.height.saturating_sub(1) as i32),
        );
        self.selection_drag_pointer = Some((x, y));
        let dir = if y <= visible_top {
            -1
        } else if y >= visible_bottom {
            1
        } else {
            0
        };
        if dir == 0 {
            self.selection_auto_dir = 0;
            self.selection_drag_pointer = None;
            return;
        }
        if self.selection_auto_dir == 0 {
            self.last_auto_scroll_ms = self.now_ms;
        }
        self.selection_auto_dir = dir;
    }

    fn auto_scroll_selection(&mut self) {
        let Some(id) = self.selection_anchor.and_then(|p| p.scroll) else {
            self.selection_auto_dir = 0;
            return;
        };
        let Some(scroll) = self.scroll_by_id(id) else {
            return;
        };
        let dir = self.selection_auto_dir as isize;
        if dir == 0 {
            return;
        }
        let remaining = scroll.scroll_by(dir, self.now_ms);
        if remaining == dir {
            self.selection_auto_dir = 0;
            return;
        }
        if let Some((x, y)) = self.selection_drag_pointer {
            let point = self.selection_point(x, y, Some(id));
            self.update_selection_focus(point);
        }
    }

    fn selection_bounds(&self) -> Option<(SelectionPoint, SelectionPoint)> {
        let anchor = self.selection_anchor?;
        let focus = self.selection_focus?;
        if anchor.scroll != focus.scroll {
            return None;
        }
        if anchor.row == focus.row && anchor.col == focus.col {
            return None;
        }
        let before = anchor.row < focus.row || (anchor.row == focus.row && anchor.col < focus.col);
        Some(if before {
            (anchor, focus)
        } else {
            (focus, anchor)
        })
    }

    fn selection_columns(
        &self,
        line: &str,
        row: usize,
        selection: (SelectionPoint, SelectionPoint),
        min_col: usize,
        max_col: usize,
    ) -> (usize, usize) {
        let line_width = visible_width(line);
        let mut start = min_col;
        let mut end = max_col.min(line_width);
        if row == selection.0.row {
            start = get_grapheme_cell_range(line, selection.0.col)
                .map(|r| r.start)
                .unwrap_or(selection.0.col.min(line_width));
        }
        if row == selection.1.row {
            end = if selection.1.boundary {
                selection.1.col.min(line_width)
            } else {
                get_grapheme_cell_range(line, selection.1.col)
                    .map(|r| r.end)
                    .unwrap_or((selection.1.col + 1).min(line_width))
            };
        }
        (start.max(min_col), end.min(max_col))
    }

    fn active_selection_text(&self) -> Option<String> {
        let selection = self.selection_bounds()?;
        let source_lines = if let Some(id) = selection.0.scroll {
            let scroll = self.scroll_by_id(id)?;
            let layout = self.current_layout.as_ref()?;
            let box_ = get_scroll_view_box(layout, &scroll)?;
            box_.scroll_content_lines.clone()?
        } else {
            self.previous_screen.clone()
        };
        let mut lines = Vec::new();
        for row in selection.0.row..=selection.1.row {
            let line = source_lines.get(row).cloned().unwrap_or_default();
            let (start, end) =
                self.selection_columns(&line, row, selection, 0, visible_width(&line));
            lines.push(
                strip_terminal_sequences(&slice_by_column(
                    &line,
                    start,
                    end.saturating_sub(start),
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

    fn copy_text(&mut self, text: &str) -> bool {
        if self.use_copy_handler {
            self.copy_log.borrow_mut().push(text.to_string());
            self.flash(
                if self.copy_ok {
                    "Copied!"
                } else {
                    "Copy failed"
                },
                1000,
            );
            return self.copy_ok;
        }
        if let Some(handler) = self.options.copy_selection {
            let ok = handler(text);
            self.flash(if ok { "Copied!" } else { "Copy failed" }, 1000);
            return ok;
        }
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, text.as_bytes());
        self.write(format!("\u{1b}]52;c;{encoded}\u{07}"));
        self.flash("Copied!", 1000);
        true
    }

    fn refresh_search(&mut self, layout: &LayoutFrame) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let scroll = layout
            .primary_scroll_view
            .clone()
            .unwrap_or_else(|| self.implicit_scroll.clone());
        let box_ = get_scroll_view_box(layout, &scroll);
        let lines = box_.and_then(|b| b.scroll_content_lines.clone());
        if lines.is_none() || search.query.trim().is_empty() {
            search.matches.clear();
            search.selected_index = -1;
            search.selected_key = None;
            search.mode = SearchMode::Retain;
            search.component.set_result(-1, 0);
            return false;
        }
        let lines = lines.unwrap();
        let should_reveal = !matches!(search.mode, SearchMode::Retain);
        let matches = find_alt_screen_search_matches(&lines, &search.query);
        let exact = search
            .selected_key
            .as_ref()
            .and_then(|key| {
                matches
                    .iter()
                    .position(|m| get_alt_screen_search_match_key(m) == *key)
            })
            .map(|i| i as isize)
            .unwrap_or(-1);
        let selected = if matches.is_empty() {
            -1
        } else {
            match search.mode {
                SearchMode::Query => {
                    let found = matches.iter().position(|m| {
                        m.segments.first().map(|s| s.row).unwrap_or(0) >= search.anchor_row
                    });
                    found.unwrap_or(0) as isize
                }
                SearchMode::Next => {
                    let base = if exact >= 0 {
                        exact
                    } else {
                        search.selected_index.min(matches.len() as isize - 1)
                    };
                    if base < 0 {
                        0
                    } else {
                        (base + 1) % matches.len() as isize
                    }
                }
                SearchMode::Previous => {
                    let base = if exact >= 0 {
                        exact
                    } else {
                        search.selected_index.min(matches.len() as isize - 1)
                    };
                    if base < 0 {
                        matches.len() as isize - 1
                    } else {
                        (base - 1 + matches.len() as isize) % matches.len() as isize
                    }
                }
                SearchMode::Retain => {
                    if exact >= 0 {
                        exact
                    } else {
                        search.selected_index.clamp(0, matches.len() as isize - 1)
                    }
                }
            }
        };
        search.matches = matches;
        search.selected_index = selected;
        search.selected_key = if selected >= 0 {
            Some(get_alt_screen_search_match_key(
                &search.matches[selected as usize],
            ))
        } else {
            None
        };
        search.mode = SearchMode::Retain;
        search.component.set_result(selected, search.matches.len());
        if !should_reveal {
            return false;
        }
        let Some(selected_match) = search.matches.get(selected.max(0) as usize) else {
            return false;
        };
        let Some(first) = selected_match.segments.first() else {
            return false;
        };
        let Some(last) = selected_match.segments.last() else {
            return false;
        };
        if box_.is_none() {
            return false;
        }
        if scroll.viewport_height() == 0 {
            return false;
        }
        let before = scroll.scroll_top();
        let visible_bottom = before + scroll.viewport_height() - 1;
        let mut target = before;
        if first.row < before || last.row > visible_bottom {
            target = first.row.saturating_sub(scroll.viewport_height() / 3);
        }
        scroll.scroll_to(target, true, self.now_ms);
        scroll.scroll_top() != before
    }

    fn apply_search_highlights(&self, screen: &[String], layout: &LayoutFrame) -> Vec<String> {
        let Some(search) = &self.search else {
            return screen.to_vec();
        };
        if search.selected_index < 0 || search.matches.is_empty() {
            return screen.to_vec();
        }
        let scroll = layout
            .primary_scroll_view
            .clone()
            .unwrap_or_else(|| self.implicit_scroll.clone());
        let Some(box_) = get_scroll_view_box(layout, &scroll) else {
            return screen.to_vec();
        };
        let scrollbar_col = get_scrollbar_geometry(box_).map(|g| g.column);
        let min_row = as_pos(box_.rect.y.max(box_.clip.y));
        let max_row = as_pos(
            (screen.len() as i32)
                .min(box_.rect.y + box_.rect.height as i32)
                .min(box_.clip.y + box_.clip.height as i32),
        );
        let min_col = as_pos(box_.rect.x.max(box_.clip.x));
        let max_col = self
            .columns
            .min(as_pos(box_.rect.x + box_.rect.width as i32))
            .min(as_pos(box_.clip.x + box_.clip.width as i32))
            .min(scrollbar_col.unwrap_or(usize::MAX));
        let mut ranges: Vec<(usize, usize, usize, bool)> = Vec::new();
        for (match_index, match_) in search.matches.iter().enumerate() {
            for segment in &match_.segments {
                let row = as_pos(box_.rect.y + segment.row as i32 - scroll.scroll_top() as i32);
                if row < min_row || row >= max_row {
                    continue;
                }
                let start = as_pos(box_.rect.x + segment.start_col as i32).max(min_col);
                let end = as_pos(box_.rect.x + segment.end_col as i32).min(max_col);
                if end <= start {
                    continue;
                }
                ranges.push((
                    row,
                    start,
                    end,
                    match_index == search.selected_index as usize,
                ));
            }
        }
        let mut result = screen.to_vec();
        ranges.sort_by_key(|r| (r.0, std::cmp::Reverse(r.1)));
        for (row, start, end, current) in ranges {
            if is_image_line(&result[row]) {
                continue;
            }
            let line_width = visible_width(&result[row]);
            let start = start.min(line_width);
            let end = end.min(line_width);
            if end <= start {
                continue;
            }
            let before = slice_by_column(&result[row], 0, start, true);
            let highlighted = slice_by_column(&result[row], start, end - start, true);
            let after = slice_by_column(&result[row], end, line_width.saturating_sub(end), true);
            let styled = apply_search_style(
                &highlighted,
                current,
                self.options.search_current_match_style,
                self.options.search_match_style,
            );
            result[row] = format!("{before}{styled}{after}");
        }
        result
    }

    fn apply_selection(&self, screen: &[String], layout: &LayoutFrame) -> Vec<String> {
        let Some(selection) = self.selection_bounds() else {
            return screen.to_vec();
        };
        let mut min_row = 0;
        let mut max_row = screen.len().saturating_sub(1);
        let mut min_col = 0;
        let mut max_col = self.columns;
        let mut screen_sel = selection;
        if let Some(id) = selection.0.scroll {
            let Some(scroll) = self.scroll_by_id(id) else {
                return screen.to_vec();
            };
            let Some(box_) = get_scroll_view_box(layout, &scroll) else {
                return screen.to_vec();
            };
            min_row = as_pos(box_.rect.y.max(box_.clip.y));
            max_row = as_pos(
                (screen.len().saturating_sub(1) as i32)
                    .min(box_.rect.y + box_.rect.height.saturating_sub(1) as i32)
                    .min(box_.clip.y + box_.clip.height.saturating_sub(1) as i32),
            );
            min_col = as_pos(box_.rect.x.max(box_.clip.x));
            max_col = self
                .columns
                .min(as_pos(box_.rect.x + box_.rect.width as i32))
                .min(as_pos(box_.clip.x + box_.clip.width as i32));
            screen_sel = (
                SelectionPoint {
                    row: as_pos(box_.rect.y + selection.0.row as i32 - scroll.scroll_top() as i32),
                    col: as_pos(box_.rect.x + selection.0.col as i32),
                    ..selection.0
                },
                SelectionPoint {
                    row: as_pos(box_.rect.y + selection.1.row as i32 - scroll.scroll_top() as i32),
                    col: as_pos(box_.rect.x + selection.1.col as i32),
                    ..selection.1
                },
            );
        }
        screen
            .iter()
            .enumerate()
            .map(|(row, line)| {
                if row < min_row
                    || row > max_row
                    || row < screen_sel.0.row
                    || row > screen_sel.1.row
                    || is_image_line(line)
                {
                    return line.clone();
                }
                let line_width = visible_width(line);
                let (start, end) =
                    self.selection_columns(line, row, screen_sel, min_col, max_col.min(line_width));
                if end <= start {
                    return line.clone();
                }
                let before = slice_by_column(line, 0, start, true);
                let selected = slice_by_column(line, start, end - start, true);
                let after = slice_by_column(line, end, line_width.saturating_sub(end), true);
                format!("{before}{}{after}", apply_selection_highlight(&selected))
            })
            .collect()
    }

    fn composite_flashes(&self, screen: &[String]) -> Vec<String> {
        let flash_lines: Vec<String> = self
            .flashes
            .iter()
            .map(|entry| {
                let message = crate::diff::truncate_to_width(
                    &format!(" {} ", entry.message),
                    self.columns,
                    "",
                );
                format!("\u{1b}[7m{message}\u{1b}[27m")
            })
            .collect();
        if flash_lines.is_empty() {
            return screen.to_vec();
        }
        let mut result = screen.to_vec();
        while result.len() < self.rows {
            result.push(String::new());
        }
        for (row, line) in flash_lines.iter().enumerate().take(self.rows) {
            let flash_width = visible_width(line);
            if flash_width == 0 {
                continue;
            }
            result[row] = composite_tui_line(
                &result[row],
                line,
                self.columns.saturating_sub(flash_width),
                flash_width,
                self.columns,
            );
        }
        result
    }

    fn composite_search_overlay(&self, screen: &[String]) -> Vec<String> {
        let Some(search) = &self.search else {
            return screen.to_vec();
        };
        let width = ((self.columns * 40) / 100)
            .max(24)
            .min(self.columns.saturating_sub(2));
        let lines = search.component.render(width);
        let height = lines.len().min(self.rows);
        let rect = overlay_rect(self.columns, self.rows, width, height, "top-right", -1, 1);
        let mut result = screen.to_vec();
        while result.len() < self.rows {
            result.push(String::new());
        }
        for (i, line) in lines.iter().enumerate() {
            let row = rect.row + i;
            if row >= result.len() {
                break;
            }
            result[row] =
                composite_tui_line(&result[row], line, rect.col, rect.width, self.columns);
        }
        result
    }

    pub fn render_now(&mut self) {
        self.render_now_forced(false);
    }

    pub fn render_now_forced(&mut self, force: bool) {
        self.sync_document();
        let root = self
            .layout_root
            .clone()
            .unwrap_or_else(|| Node::Scroll(self.implicit_scroll.clone()));
        let mut layout = render_layout_frame(&root, self.columns, self.rows);
        if self.refresh_search(&layout) {
            layout = render_layout_frame(&root, self.columns, self.rows);
            let _ = self.refresh_search(&layout);
        }
        let mut screen: Vec<String> = layout
            .lines
            .iter()
            .map(|line| strip_osc133_zone_prefix(line))
            .collect();
        screen = self.apply_search_highlights(&screen, &layout);
        screen = self.composite_search_overlay(&screen);
        if screen.len() > self.rows {
            screen = screen[screen.len() - self.rows..].to_vec();
        }
        screen = self.apply_selection(&screen, &layout);
        screen = self.composite_flashes(&screen);
        let mut cursor_lines = screen.clone();
        let cursor = extract_cursor_position(&mut cursor_lines, self.rows);
        apply_line_resets(&mut screen);
        screen = screen
            .into_iter()
            .map(|line| {
                if is_image_line(&line) || visible_width(&line) <= self.columns {
                    line
                } else {
                    slice_by_column(&line, 0, self.columns, true)
                }
            })
            .collect();
        while screen.len() < self.rows {
            screen.push(String::new());
        }
        let full = force || self.previous_screen.is_empty();
        let images_need = screen.iter().enumerate().any(|(row, line)| {
            line != self
                .previous_screen
                .get(row)
                .map(String::as_str)
                .unwrap_or("")
                && (is_image_line(line)
                    || self
                        .previous_screen
                        .get(row)
                        .is_some_and(|p| is_image_line(p)))
        });
        let prepared = if (full || images_need) && self.image_protocol == Some(ImageProtocol::Kitty)
        {
            prepare_kitty_screen(&screen, &mut self.uploaded_kitty)
        } else {
            (screen.clone(), String::new())
        };
        let mut buffer = SYNC_BEGIN.to_string();
        if full {
            let clear = if self.image_protocol == Some(ImageProtocol::Kitty)
                && !self.uploaded_kitty.is_empty()
            {
                delete_all_kitty_placements()
            } else if self.image_protocol == Some(ImageProtocol::Kitty) {
                delete_all_kitty_images()
            } else {
                String::new()
            };
            buffer.push_str(&clear);
            buffer.push_str("\u{1b}[2J");
        } else if images_need {
            if self.image_protocol == Some(ImageProtocol::ITerm2) {
                buffer.push_str("\u{1b}[2J");
            } else if self.image_protocol == Some(ImageProtocol::Kitty) {
                buffer.push_str(&delete_all_kitty_placements());
            }
        }
        buffer.push_str(&prepared.1);
        for (row, line) in screen.iter().enumerate().take(self.rows) {
            if !full && !images_need && self.previous_screen.get(row) == Some(line) {
                continue;
            }
            buffer.push_str(&format!(
                "\u{1b}[{};1H\u{1b}[2K{}",
                row + 1,
                prepared.0.get(row).cloned().unwrap_or_default()
            ));
        }
        if let Some((row, col)) = cursor {
            buffer.push_str(&format!(
                "\u{1b}[{};{}H",
                row + 1,
                col.min(self.columns) + 1
            ));
            buffer.push_str("\u{1b}[?25l");
        } else {
            buffer.push_str("\u{1b}[?25l");
        }
        buffer.push_str(SYNC_END);
        self.write(buffer);
        self.previous_screen = screen;
        self.current_layout = Some(layout);
    }

    pub fn request_render(&mut self) {
        self.render_now();
    }
}

fn as_pos(value: i32) -> usize {
    value.max(0) as usize
}

fn apply_search_style(
    text: &str,
    current: bool,
    current_style: fn(&str) -> String,
    match_style: fn(&str) -> String,
) -> String {
    let style = if current { current_style } else { match_style };
    let mut result = String::new();
    let mut plain_start = 0usize;
    let mut index = 0usize;
    while index < text.len() {
        if let Some(ansi) = extract_ansi_code(text, index) {
            if index > plain_start {
                result.push_str(&style(&text[plain_start..index]));
            }
            result.push_str(ansi.code);
            index += ansi.length;
            plain_start = index;
        } else {
            index += text[index..].chars().next().unwrap().len_utf8();
        }
    }
    if plain_start < text.len() {
        result.push_str(&style(&text[plain_start..]));
    }
    result
}

fn apply_selection_highlight(text: &str) -> String {
    let mut result = String::from("\u{1b}[7m");
    let mut index = 0usize;
    while index < text.len() {
        if let Some(ansi) = extract_ansi_code(text, index) {
            result.push_str(ansi.code);
            if ansi.code.ends_with('m') {
                result.push_str("\u{1b}[7m");
            }
            index += ansi.length;
        } else {
            let ch = text[index..].chars().next().unwrap();
            result.push(ch);
            index += ch.len_utf8();
        }
    }
    result.push_str("\u{1b}[27m");
    result
}

fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || is_cjk_char(ch)
}

fn is_mid_word(ch: char) -> bool {
    matches!(ch, '.' | '\'' | '\u{2019}' | '\u{00B7}' | '-')
}

fn word_segments(line: &str) -> Vec<(usize, usize, bool, bool)> {
    let mut graphemes = Vec::new();
    let mut col = 0usize;
    let mut rest = line;
    while !rest.is_empty() {
        let Some((segment, next)) = crate::ansi_text::next_grapheme(rest) else {
            break;
        };
        let width = crate::ansi_text::grapheme_width(segment);
        graphemes.push((segment, col, col + width));
        col += width;
        rest = next;
    }
    let mut segments = Vec::new();
    let mut index = 0usize;
    while index < graphemes.len() {
        let (segment, start, _) = graphemes[index];
        if segment.chars().all(char::is_whitespace) {
            let mut end = graphemes[index].2;
            index += 1;
            while index < graphemes.len() && graphemes[index].0.chars().all(char::is_whitespace) {
                end = graphemes[index].2;
                index += 1;
            }
            segments.push((start, end, false, false));
            continue;
        }
        if segment.chars().any(is_word_char) {
            let mut end = graphemes[index].2;
            index += 1;
            while index < graphemes.len() {
                let current = graphemes[index].0;
                if current.chars().any(is_word_char) {
                    end = graphemes[index].2;
                    index += 1;
                    continue;
                }
                let mid =
                    current.chars().count() == 1 && current.chars().next().is_some_and(is_mid_word);
                if mid
                    && index + 1 < graphemes.len()
                    && graphemes[index + 1].0.chars().any(is_word_char)
                {
                    end = graphemes[index + 1].2;
                    index += 2;
                    continue;
                }
                break;
            }
            segments.push((start, end, true, false));
            continue;
        }
        let joiner = segment.chars().count() == 1
            && segment
                .chars()
                .next()
                .is_some_and(|ch| WORD_JOINERS.contains(&ch));
        segments.push((graphemes[index].1, graphemes[index].2, joiner, joiner));
        index += 1;
    }
    segments
}

fn parse_wheel(data: &str) -> Option<(i8, usize, usize)> {
    if let Some(rest) = data.strip_prefix("\u{1b}[<") {
        if !rest.ends_with('M') && !rest.ends_with('m') {
            return None;
        }
        let body = &rest[..rest.len() - 1];
        let mut parts = body.split(';');
        let button: u16 = parts.next()?.parse().ok()?;
        if button & 64 == 0 {
            return None;
        }
        let direction = button & 3;
        if direction != 0 && direction != 1 {
            return None;
        }
        let x: usize = parts.next()?.parse().ok()?;
        let y: usize = parts.next()?.parse().ok()?;
        return Some((
            if direction == 0 { -1 } else { 1 },
            x.saturating_sub(1),
            y.saturating_sub(1),
        ));
    }
    None
}

fn parse_sgr(data: &str) -> Option<(u16, usize, usize, bool)> {
    let rest = data.strip_prefix("\u{1b}[<")?;
    let release = rest.ends_with('m');
    if !release && !rest.ends_with('M') {
        return None;
    }
    let body = &rest[..rest.len() - 1];
    let mut parts = body.split(';');
    let button: u16 = parts.next()?.parse().ok()?;
    let x: usize = parts.next()?.parse().ok()?;
    let y: usize = parts.next()?.parse().ok()?;
    Some((button, x.saturating_sub(1), y.saturating_sub(1), release))
}

fn is_mouse_sequence(data: &str) -> bool {
    parse_sgr(data).is_some() || (data.len() == 6 && data.starts_with("\u{1b}[M"))
}

fn find_scroll(box_: &crate::constrained_layout::LayoutBox, id: usize) -> Option<ViewportScroll> {
    if box_
        .scroll_view
        .as_ref()
        .is_some_and(|scroll| Rc::as_ptr(&scroll.inner) as usize == id)
    {
        return box_.scroll_view.clone();
    }
    box_.children
        .iter()
        .find_map(|child| find_scroll(child, id))
}

pub fn default_search_match_style(text: &str) -> String {
    format!("\u{1b}[4m{text}\u{1b}[24m")
}

pub fn layout_transcript_and_dock(transcript: ViewportScroll, dock: Node) -> Node {
    Node::VStack(LayoutVStack::new(vec![
        StackEntry::grow(Node::Scroll(transcript), 1, 1),
        StackEntry::auto(dock),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybindings::{reset_alt_screen_bindings, set_alt_screen_bindings};
    use crate::terminal_image::{reset_capabilities_cache, set_capabilities};
    use crate::viewport::{Overscroll, ScrollbarMode};

    fn lines(count: usize) -> Node {
        Node::text(
            (0..count)
                .map(|i| format!("line {}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    #[test]
    fn viewport_preserves_manual_scroll() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        let text = lines(10);
        tui.add_child(text.clone());
        tui.start();
        assert_eq!(
            trim_view(&tui),
            vec!["line 7", "line 8", "line 9", "line 10"]
        );
        assert!(tui.is_following_output());
        tui.handle_input("\u{1b}[<64;1;1M");
        assert_eq!(
            trim_view(&tui),
            vec!["line 6", "line 7", "line 8", "line 9"]
        );
        assert_eq!(tui.viewport_top(), 5);
        assert!(!tui.is_following_output());
        text.set_text(
            (0..12)
                .map(|i| format!("line {}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        tui.request_render();
        assert_eq!(
            trim_view(&tui),
            vec!["line 6", "line 7", "line 8", "line 9"]
        );
        tui.stop();
    }

    #[test]
    fn dock_stays_fixed_while_transcript_scrolls() {
        let mut tui = TuiAltScreen::new(20, 6, TuiAltScreenOptions::default());
        let transcript_text = lines(8);
        let transcript = ViewportScroll::new(
            transcript_text.clone(),
            ViewportScrollOptions {
                follow_end: true,
                primary: true,
                ..ViewportScrollOptions::default()
            },
        );
        let dock = Node::VStack(LayoutVStack::new(vec![
            StackEntry::auto(Node::text("editor")),
            StackEntry::auto(Node::text("footer")),
        ]));
        tui.set_layout_root(layout_transcript_and_dock(transcript.clone(), dock));
        tui.start();
        assert_eq!(
            trim_view(&tui),
            vec!["line 5", "line 6", "line 7", "line 8", "editor", "footer"]
        );
        tui.handle_input("\u{1b}[<64;1;6M");
        assert_eq!(
            trim_view(&tui),
            vec!["line 4", "line 5", "line 6", "line 7", "editor", "footer"]
        );
        assert!(!transcript.is_following_end());
        transcript_text.set_text(
            (0..10)
                .map(|i| format!("line {}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        tui.request_render();
        assert_eq!(
            trim_view(&tui),
            vec!["line 4", "line 5", "line 6", "line 7", "editor", "footer"]
        );
        tui.scroll_to_bottom();
        assert_eq!(
            trim_view(&tui),
            vec!["line 7", "line 8", "line 9", "line 10", "editor", "footer"]
        );
        tui.stop();
    }

    #[test]
    fn search_normalized_text_across_rows() {
        let matches = find_alt_screen_search_matches(
            &["alpha QUICK".into(), "brown fox".into()],
            "quick brown",
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].segments[0].row, 0);
        assert_eq!(matches[0].segments[0].start_col, 6);
        assert_eq!(matches[0].segments[0].end_col, 11);
        assert_eq!(matches[0].segments[1].row, 1);
        assert_eq!(matches[0].segments[1].start_col, 0);
        assert_eq!(matches[0].segments[1].end_col, 5);
    }

    #[test]
    fn search_ctrl_shift_f_and_restore_editor() {
        let mut tui = TuiAltScreen::new(60, 8, TuiAltScreenOptions::default());
        let body = (0..12)
            .map(|index| {
                if index == 4 {
                    "line 5 needle one".into()
                } else if index == 9 {
                    "line 10 needle two".into()
                } else {
                    format!("line {}", index + 1)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let transcript = ViewportScroll::new(
            Node::text(body),
            ViewportScrollOptions {
                follow_end: true,
                primary: true,
                ..ViewportScrollOptions::default()
            },
        );
        let dock = Node::text("editor");
        tui.set_layout_root(layout_transcript_and_dock(transcript.clone(), dock));
        tui.set_focus_editor();
        tui.start();
        assert_eq!(tui.viewport_top(), 5);
        tui.handle_input("\u{1b}[102;6u");
        tui.handle_input("n");
        tui.handle_input("e");
        tui.handle_input("e");
        tui.handle_input("d");
        tui.handle_input("l");
        tui.handle_input("e");
        assert!(!transcript.is_following_end());
        assert!(tui
            .viewport()
            .iter()
            .any(|line| line.contains("Find transcript") && line.contains("2/2")));
        assert!(
            trim_view(&tui)
                .iter()
                .any(|line| line.contains("line 10 needle two")),
            "view={:?}",
            trim_view(&tui)
        );
        assert!(tui.focus_inputs.borrow().is_empty());
        assert!(tui
            .writes
            .iter()
            .any(|w| w.contains("\u{1b}[1;7mneedle\u{1b}[22;27m")));
        for _ in 0..6 {
            tui.handle_input("\u{1b}[<64;1;4M");
        }
        assert_eq!(transcript.scroll_top(), 0);
        assert!(tui.viewport().iter().any(|line| line.contains("> needle")));
        tui.handle_input("\u{07}");
        assert!(tui
            .viewport()
            .iter()
            .any(|line| line.contains("Find transcript") && line.contains("1/2")));
        assert!(trim_view(&tui)
            .iter()
            .any(|line| line.contains("line 5 needle one")));
        tui.handle_input("\u{1b}[103;6u");
        assert!(tui
            .viewport()
            .iter()
            .any(|line| line.contains("Find transcript") && line.contains("2/2")));
        tui.handle_input("\u{1b}");
        tui.handle_input("x");
        assert!(!tui
            .viewport()
            .iter()
            .any(|line| line.contains("Find transcript")));
        assert_eq!(tui.focus_inputs.borrow().as_slice(), &["x".to_string()]);
        tui.stop();
    }

    #[test]
    fn half_page_and_line_custom_bindings() {
        set_alt_screen_bindings(&[
            ("tui.altScreen.halfPageUp", "ctrl+u"),
            ("tui.altScreen.halfPageDown", "ctrl+d"),
        ]);
        let mut tui = TuiAltScreen::new(20, 10, TuiAltScreenOptions::default());
        tui.add_child(lines(30));
        tui.start();
        assert_eq!(tui.viewport_top(), 20);
        tui.handle_input("\u{15}");
        assert_eq!(tui.viewport_top(), 15);
        tui.handle_input("\u{04}");
        assert_eq!(tui.viewport_top(), 20);
        tui.stop();
        set_alt_screen_bindings(&[
            ("tui.altScreen.lineUp", "ctrl+y"),
            ("tui.altScreen.lineDown", "ctrl+e"),
        ]);
        let mut tui = TuiAltScreen::new(20, 10, TuiAltScreenOptions::default());
        tui.add_child(lines(30));
        tui.start();
        tui.handle_input("\u{19}");
        assert_eq!(tui.viewport_top(), 19);
        tui.handle_input("\u{05}");
        assert_eq!(tui.viewport_top(), 20);
        tui.stop();
        reset_alt_screen_bindings();
    }

    #[test]
    fn osc133_prompt_jumps() {
        let mut tui = TuiAltScreen::new(20, 3, TuiAltScreenOptions::default());
        let body = [1, 2, 3, 4]
            .into_iter()
            .flat_map(|message| {
                [
                    format!("\u{1b}]133;A\u{07}message {message}"),
                    "detail".into(),
                ]
            })
            .collect::<Vec<_>>()
            .join("\n");
        tui.add_child(Node::text(body));
        tui.start();
        assert_eq!(tui.viewport_top(), 5);
        tui.handle_input("\u{1b}[57419;6u");
        tui.handle_input("\u{1b}[57419;6:3u");
        assert_eq!(tui.viewport_top(), 4);
        assert_eq!(trim_view(&tui)[0], "message 3");
        tui.handle_input("\u{1b}[1;6A");
        assert_eq!(tui.viewport_top(), 2);
        assert_eq!(trim_view(&tui)[0], "message 2");
        tui.handle_input("\u{1b}[57420;6u");
        tui.handle_input("\u{1b}[57420;6:3u");
        assert_eq!(tui.viewport_top(), 4);
        assert_eq!(trim_view(&tui)[0], "message 3");
        tui.handle_input("\u{1b}[1;6B");
        assert_eq!(tui.viewport_top(), 5);
        assert_eq!(trim_view(&tui)[1], "message 4");
        assert!(tui.is_following_output());
        tui.stop();
    }

    #[test]
    fn scrollbar_drag_and_hidden_column_select() {
        let mut tui = TuiAltScreen::new(10, 5, TuiAltScreenOptions::default());
        let scroll = ViewportScroll::new(
            lines(20),
            ViewportScrollOptions {
                primary: true,
                scrollbar: ScrollbarMode::Auto,
                scrollbar_hide_delay_ms: 50,
                ..ViewportScrollOptions::default()
            },
        );
        tui.set_layout_root(Node::Scroll(scroll.clone()));
        tui.start();
        assert!(!scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<65;10;1M");
        assert_eq!(scroll.scroll_top(), 1);
        assert!(scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<0;10;1M");
        tui.advance_ms(70);
        assert!(scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<32;10;4M");
        assert_eq!(scroll.scroll_top(), 15);
        assert_eq!(
            trim_view(&tui),
            vec!["line 16", "line 17", "line 18", "line 19", "line 20"]
        );
        tui.handle_input("\u{1b}[<0;10;4m");
        assert!(scroll.is_scrollbar_visible());
        tui.advance_ms(70);
        assert!(scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<35;9;4M");
        tui.advance_ms(70);
        assert!(!scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<64;10;5M");
        assert_eq!(scroll.scroll_top(), 14);
        tui.advance_ms(70);
        assert!(scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<35;9;5M");
        tui.advance_ms(70);
        assert!(!scroll.is_scrollbar_visible());
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]52;c;")));
        tui.stop();

        let mut tui = TuiAltScreen::new(10, 2, TuiAltScreenOptions::default());
        let scroll = ViewportScroll::new(
            Node::text("123456789A\nabcdefghij\nmore\nlines"),
            ViewportScrollOptions {
                scrollbar: ScrollbarMode::Auto,
                ..ViewportScrollOptions::default()
            },
        );
        tui.set_layout_root(Node::Scroll(scroll.clone()));
        tui.start();
        assert!(!scroll.is_scrollbar_visible());
        tui.handle_input("\u{1b}[<0;10;1M");
        tui.handle_input("\u{1b}[<32;10;2M");
        tui.handle_input("\u{1b}[<0;10;2m");
        let expected = format!(
            "\u{1b}]52;c;{}\u{07}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"A\nabcdefghij")
        );
        assert!(tui.writes.iter().any(|w| w.contains(&expected)));
        tui.stop();
    }

    #[test]
    fn wheel_chains_and_page_overlap() {
        let mut tui = TuiAltScreen::new(
            20,
            4,
            TuiAltScreenOptions {
                wheel_scroll_lines: 3,
                ..TuiAltScreenOptions::default()
            },
        );
        let inner = ViewportScroll::new(
            Node::text("i1\ni2\ni3\ni4\ni5\ni6"),
            ViewportScrollOptions::default(),
        );
        let outer = ViewportScroll::new(
            Node::VStack(LayoutVStack::new(vec![
                StackEntry::sized(Node::Scroll(inner.clone()), 2),
                StackEntry::auto(Node::text("tail1\ntail2\ntail3\ntail4\ntail5")),
            ])),
            ViewportScrollOptions {
                primary: true,
                ..ViewportScrollOptions::default()
            },
        );
        tui.set_layout_root(Node::Scroll(outer.clone()));
        tui.start();
        tui.handle_input("\u{1b}[<65;1;1M");
        assert_eq!(inner.scroll_top(), 3);
        assert_eq!(outer.scroll_top(), 0);
        tui.handle_input("\u{1b}[<65;1;1M");
        assert_eq!(inner.scroll_top(), 4);
        assert_eq!(outer.scroll_top(), 2);
        tui.stop();
        let _ = Overscroll::Chain;
    }

    #[test]
    fn flashes_stack_and_expire() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(Node::text("one\ntwo\nthree\nfour"));
        tui.start();
        tui.flash("First", 80);
        tui.flash("Second", 500);
        let view = tui.viewport();
        assert!(view[0].ends_with(" First ") || view[0].contains(" First "));
        assert!(view[1].ends_with(" Second ") || view[1].contains(" Second "));
        tui.advance_ms(100);
        let view = tui.viewport();
        assert!(view[0].contains(" Second "));
        assert!(!view.iter().any(|line| line.contains("First")));
        tui.stop();
    }

    #[test]
    fn mouse_selection_copies_osc52() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(lines(8));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<32;5;1M");
        tui.handle_input("\u{1b}[<0;5;1m");
        assert!(tui.writes.iter().any(|w| w.contains("\u{1b}]52;c;")));
        assert!(tui.has_active_selection());
        tui.stop();
    }

    #[test]
    fn cjk_emoji_grapheme_selection() {
        let mut tui = TuiAltScreen::new(20, 2, TuiAltScreenOptions::default());
        tui.add_child(Node::text("A界🙂éZ"));
        tui.start();
        let wide = format!(
            "\u{1b}]52;c;{}\u{07}",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                "界🙂".as_bytes()
            )
        );
        tui.handle_input("\u{1b}[<0;3;1M");
        tui.handle_input("\u{1b}[<32;4;1M");
        tui.handle_input("\u{1b}[<0;4;1m");
        assert_eq!(tui.writes.iter().filter(|w| w.contains(&wide)).count(), 1);
        tui.stop();
    }

    #[test]
    fn iterm2_strips_osc133_and_kitty() {
        let _lock = crate::terminal_image::capabilities_lock();
        set_capabilities(crate::terminal_image::TerminalCapabilities {
            images: Some(crate::terminal_image::ImageProtocol::ITerm2),
            true_color: true,
            hyperlinks: true,
        });
        let mut tui = TuiAltScreen::new(20, 3, TuiAltScreenOptions::default());
        tui.add_child(Node::Render(Rc::new(|_| {
            vec!["\u{1b}]133;B\u{07}\u{1b}]133;C\u{07}\u{1b}]133;A\u{07}content".into()]
        })));
        tui.start();
        tui.stop();
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}_G")));
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]133;")));
        reset_capabilities_cache();
    }

    #[test]
    fn copy_on_select_disabled_keeps_selection() {
        let mut tui = TuiAltScreen::new(
            20,
            3,
            TuiAltScreenOptions {
                copy_on_select: false,
                ..TuiAltScreenOptions::default()
            },
        );
        tui.copy_on_select = false;
        tui.add_child(Node::text("alpha beta gamma"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<32;6;1M");
        tui.handle_input("\u{1b}[<0;6;1m");
        assert!(tui.has_active_selection());
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]52;c;")));
        tui.stop();
    }

    #[test]
    fn copy_handler_flashes_success_and_skips_osc52_on_failure() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        let copied = tui.set_copy_handler(true);
        tui.add_child(Node::text("alpha\nbeta\ngamma\ndelta"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<32;4;2M");
        tui.handle_input("\u{1b}[<0;4;2m");
        assert_eq!(copied.borrow().as_slice(), &["alpha\nbeta".to_string()]);
        assert!(tui.has_active_selection());
        copied.borrow_mut().clear();
        assert!(tui.copy_active_selection());
        assert_eq!(copied.borrow().as_slice(), &["alpha\nbeta".to_string()]);
        assert!(tui.viewport().iter().any(|line| line.contains("Copied!")));
        tui.stop();

        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.set_copy_handler(false);
        tui.add_child(Node::text("alpha\nbeta\ngamma\ndelta"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<32;4;2M");
        tui.handle_input("\u{1b}[<0;4;2m");
        assert!(tui
            .viewport()
            .iter()
            .any(|line| line.contains("Copy failed")));
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]52;c;")));
        tui.stop();
    }

    #[test]
    fn double_click_word_selection_skips_trailing_whitespace() {
        let mut tui = TuiAltScreen::new(20, 1, TuiAltScreenOptions::default());
        tui.add_child(Node::text("foo  bar"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<0;1;1m");
        tui.handle_input("\u{1b}[<0;3;1M");
        assert!(tui.writes.iter().any(|w| w.contains("foo\u{1b}[27m")));
        tui.stop();
    }

    #[test]
    fn double_click_joins_slash_and_hyphen_paths() {
        for (line, needle) in [
            ("extensions/starline/fixed-editor/compositor.ts", "starline"),
            ("earendil-works/pi-tui", "works"),
        ] {
            let mut tui = TuiAltScreen::new(80, 1, TuiAltScreenOptions::default());
            let copied = tui.set_copy_handler(true);
            tui.add_child(Node::text(line));
            tui.start();
            let col = line.find(needle).unwrap() + 1;
            let press = format!("\u{1b}[<0;{col};1M");
            let release = format!("\u{1b}[<0;{col};1m");
            tui.handle_input(&press);
            tui.handle_input(&release);
            tui.handle_input(&press);
            tui.handle_input(&release);
            assert_eq!(copied.borrow().as_slice(), &[line.to_string()]);
            tui.stop();
        }
    }

    #[test]
    fn word_drag_highlights_full_whitespace_segment() {
        let mut tui = TuiAltScreen::new(20, 1, TuiAltScreenOptions::default());
        tui.add_child(Node::text("foo  bar"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<0;1;1m");
        tui.handle_input("\u{1b}[<0;2;1M");
        tui.handle_input("\u{1b}[<32;4;1M");
        assert!(tui.writes.iter().any(|w| w.contains("foo  \u{1b}[27m")));
        tui.stop();
    }

    #[test]
    fn double_click_word_drag_and_triple_click_line() {
        let mut tui = TuiAltScreen::new(20, 2, TuiAltScreenOptions::default());
        tui.add_child(Node::text("zero alpha beta\ngamma delta"));
        tui.start();
        tui.handle_input("\u{1b}[<0;6;1M");
        tui.handle_input("\u{1b}[<0;6;1m");
        tui.handle_input("\u{1b}[<0;10;1M");
        tui.handle_input("\u{1b}[<0;10;1m");
        assert!(tui.writes.iter().any(|w| w.contains(&osc52("alpha"))));

        tui.handle_input("\u{1b}[<0;12;1M");
        tui.handle_input("\u{1b}[<0;12;1m");
        tui.handle_input("\u{1b}[<0;14;1M");
        tui.handle_input("\u{1b}[<32;3;2M");
        tui.handle_input("\u{1b}[<0;3;2m");
        assert!(tui.writes.iter().any(|w| w.contains(&osc52("beta\ngamma"))));

        tui.handle_input("\u{1b}[<0;7;2M");
        tui.handle_input("\u{1b}[<0;7;2m");
        tui.handle_input("\u{1b}[<0;9;2M");
        tui.handle_input("\u{1b}[<0;9;2m");
        tui.handle_input("\u{1b}[<0;11;2M");
        tui.handle_input("\u{1b}[<0;11;2m");
        assert!(tui.writes.iter().any(|w| w.contains(&osc52("gamma delta"))));
        tui.stop();
    }

    #[test]
    fn focus_loss_idle_zero_width_and_completed_selection() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(Node::text("alpha\nbeta\ngamma\ndelta"));
        tui.start();
        let idle_writes = tui.writes.len();
        tui.handle_input("\u{1b}[O");
        tui.handle_input("\u{1b}[I");
        assert_eq!(tui.writes.len(), idle_writes);

        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<0;1;1m");
        tui.handle_input("\u{1b}[<32;4;2M");
        tui.handle_input("\u{1b}[<0;4;2m");
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]52;c;")));

        tui.handle_input("\u{1b}[<0;1;3M");
        let pressed_writes = tui.writes.len();
        tui.handle_input("\u{1b}[O");
        tui.handle_input("\u{1b}[I");
        assert_eq!(tui.writes.len(), pressed_writes);
        tui.handle_input("\u{1b}[<32;4;2M");
        tui.handle_input("\u{1b}[<0;4;2m");
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]52;c;")));
        assert!(tui.writes.iter().any(|w| w.contains("\u{1b}[?1004h")));
        tui.stop();
        assert!(tui.writes.iter().any(|w| w.contains("\u{1b}[?1004l")));

        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(Node::text("alpha\nbeta\ngamma\ndelta"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<32;4;2M");
        let before_focus = tui.writes.len();
        tui.handle_input("\u{1b}[O");
        tui.handle_input("\u{1b}[I");
        let focus_writes = tui.writes[before_focus..].join("");
        assert!(focus_writes.contains("alpha"));
        assert!(focus_writes.contains("beta"));
        assert!(!focus_writes.contains("\u{1b}[7m"));
        tui.handle_input("\u{1b}[<32;4;2M");
        tui.handle_input("\u{1b}[<0;4;2m");
        assert!(tui.writes.iter().all(|w| !w.contains("\u{1b}]52;c;")));
        tui.stop();

        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(Node::text("alpha\nbeta\ngamma\ndelta"));
        tui.start();
        tui.handle_input("\u{1b}[<0;1;1M");
        tui.handle_input("\u{1b}[<32;4;2M");
        tui.handle_input("\u{1b}[<0;4;2m");
        let completed = tui.writes.len();
        tui.handle_input("\u{1b}[O");
        tui.handle_input("\u{1b}[I");
        assert_eq!(tui.writes.len(), completed);
        tui.render_now_forced(true);
        let redraw = tui.writes.last().cloned().unwrap_or_default();
        assert!(redraw.contains("alpha"));
        assert!(redraw.contains("beta"));
        assert!(redraw.contains("\u{1b}[7m"));
        tui.stop();
    }

    #[test]
    fn auto_scroll_extends_drag_at_viewport_edge() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(lines(10));
        tui.start();
        assert_eq!(tui.viewport_top(), 6);
        tui.handle_input("\u{1b}[<0;1;3M");
        tui.handle_input("\u{1b}[<32;1;1M");
        tui.advance_ms(130);
        let selection_top = tui.viewport_top();
        assert!(
            selection_top < 6,
            "expected auto-scroll above row 6, got {selection_top}"
        );
        tui.handle_input("\u{1b}[<0;1;1m");
        let mut selected: Vec<String> = (0..8usize.saturating_sub(selection_top))
            .map(|index| format!("line {}", selection_top + index + 1))
            .collect();
        selected.push("l".into());
        assert!(tui
            .writes
            .iter()
            .any(|w| w.contains(&osc52(&selected.join("\n")))));
        tui.stop();
    }

    #[test]
    fn cjk_emoji_and_combining_grapheme_selection() {
        let mut tui = TuiAltScreen::new(20, 2, TuiAltScreenOptions::default());
        tui.add_child(Node::text("A界🙂éZ"));
        tui.start();
        let wide = osc52("界🙂");
        tui.handle_input("\u{1b}[<0;3;1M");
        tui.handle_input("\u{1b}[<32;4;1M");
        tui.handle_input("\u{1b}[<0;4;1m");
        assert_eq!(tui.writes.iter().filter(|w| w.contains(&wide)).count(), 1);
        tui.handle_input("\u{1b}[<0;5;1M");
        tui.handle_input("\u{1b}[<32;2;1M");
        tui.handle_input("\u{1b}[<0;2;1m");
        assert_eq!(tui.writes.iter().filter(|w| w.contains(&wide)).count(), 2);
        tui.handle_input("\u{1b}[<0;6;1M");
        tui.handle_input("\u{1b}[<32;7;1M");
        tui.handle_input("\u{1b}[<0;7;1m");
        assert!(tui.writes.iter().any(|w| w.contains(&osc52("éZ"))));
        tui.stop();
    }

    #[test]
    fn ignores_horizontal_trackpad_wheel() {
        let mut tui = TuiAltScreen::new(20, 4, TuiAltScreenOptions::default());
        tui.add_child(lines(8));
        tui.start();
        tui.handle_input("\u{1b}[<66;1;1M");
        tui.handle_input("\u{1b}[<67;1;1M");
        assert_eq!(tui.viewport_top(), 4);
        assert_eq!(
            trim_view(&tui),
            vec!["line 5", "line 6", "line 7", "line 8"]
        );
        tui.stop();
    }

    #[test]
    fn focused_overlay_receives_wheel_and_viewport_keys() {
        let mut tui = TuiAltScreen::new(20, 6, TuiAltScreenOptions::default());
        tui.add_child(lines(12));
        let overlay = InputOverlay::new();
        tui.start();
        let top_before = tui.viewport_top();
        let handle = tui.show_overlay(overlay.clone(), OverlayShowOptions::default());
        assert!(overlay.focused());
        let keys = [
            "\u{1b}[5~",
            "\u{1b}[6~",
            "\u{1b}OH",
            "\u{1b}OF",
            "\u{1b}[<64;10;3M",
        ];
        for key in keys {
            tui.handle_input(key);
        }
        assert_eq!(overlay.inputs(), keys);
        assert_eq!(tui.viewport_top(), top_before);
        tui.hide_overlay(&handle);
        tui.handle_input("\u{1b}[5~");
        assert!(tui.viewport_top() < top_before);
        tui.stop();
    }

    #[test]
    fn unfocused_overlays_keep_viewport_scrolling() {
        let mut tui = TuiAltScreen::new(20, 6, TuiAltScreenOptions::default());
        let editor = InputOverlay::new();
        tui.add_child(lines(12));
        tui.set_focus(&editor);
        tui.start();
        let top_before = tui.viewport_top();
        let hidden = tui.show_overlay(InputOverlay::new(), OverlayShowOptions::default());
        tui.set_overlay_hidden(&hidden, true);
        let non_capturing = InputOverlay::new();
        tui.show_overlay(
            non_capturing.clone(),
            OverlayShowOptions {
                non_capturing: true,
            },
        );
        let unfocused = InputOverlay::new();
        let unfocused_handle = tui.show_overlay(unfocused.clone(), OverlayShowOptions::default());
        tui.unfocus_overlay(&unfocused_handle);
        assert!(!non_capturing.focused());
        assert!(!unfocused.focused());
        tui.handle_input("\u{1b}[5~");
        tui.handle_input("\u{1b}[<64;10;3M");
        assert!(tui.viewport_top() < top_before);
        assert!(non_capturing.inputs().is_empty());
        assert!(unfocused.inputs().is_empty());
        tui.stop();
    }

    #[test]
    fn search_focus_keeps_viewport_scrolling() {
        let mut tui = TuiAltScreen::new(20, 6, TuiAltScreenOptions::default());
        tui.add_child(lines(12));
        tui.start();
        let top_before = tui.viewport_top();
        tui.handle_input("\u{1b}[102;6u");
        assert!(tui
            .viewport()
            .iter()
            .any(|line| line.contains("Find transcript")));
        tui.handle_input("\u{1b}[5~");
        tui.handle_input("\u{1b}[<64;1;4M");
        assert!(tui.viewport_top() < top_before);
        assert!(tui
            .viewport()
            .iter()
            .any(|line| line.contains("Find transcript")));
        tui.stop();
    }

    #[test]
    fn right_click_paste_windows_only_outside_vscode() {
        let mut tui = TuiAltScreen::new(
            20,
            4,
            TuiAltScreenOptions {
                platform_windows: true,
                on_right_click_paste: Some(|| {}),
                ..TuiAltScreenOptions::default()
            },
        );
        tui.start();
        tui.handle_input("\u{1b}[<2;1;1M");
        tui.handle_input("\u{1b}[<2;1;1m");
        assert_eq!(tui.right_click_pastes(), 1);
        tui.options.term_program_vscode = true;
        tui.handle_input("\u{1b}[<2;1;1M");
        assert_eq!(tui.right_click_pastes(), 1);
        tui.options.term_program_vscode = false;
        tui.options.platform_windows = false;
        tui.handle_input("\u{1b}[<2;1;1M");
        assert_eq!(tui.right_click_pastes(), 1);
        tui.stop();
    }

    fn osc52(text: &str) -> String {
        format!(
            "\u{1b}]52;c;{}\u{07}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, text.as_bytes())
        )
    }

    fn trim_view(tui: &TuiAltScreen) -> Vec<String> {
        tui.viewport()
            .into_iter()
            .map(|line| {
                crate::ansi_text::strip_terminal_sequences(&line)
                    .trim_end()
                    .to_string()
            })
            .collect()
    }
}
