//! TuiBase / TuiMainScreen / TuiAltScreen matching TS `tui.ts`, `tui-main-screen.ts`,
//! and `tui-alt-screen.ts`.

use std::path::PathBuf;

use crate::ansi::{normalize_terminal_output, slice_by_column, visible_width};
use crate::container::Container;
use crate::image::{delete_kitty_image, is_image_line, kitty_image_ids, parse_kitty_image_header};
use crate::keybindings::Keybindings;
use crate::keys::is_key_release;
use crate::osc::{
    is_osc11_background_color_response, parse_osc11_background_color,
    parse_terminal_color_scheme_report, RgbColor, COLOR_SCHEME_QUERY, OSC_11_QUERY,
};
use crate::overlay::{composite_tui_line, resolve_overlay_layout, OverlayOptions};
use crate::render::Component;
use crate::terminal::TerminalIo;
use crate::CURSOR_MARKER;

const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";
const MAX_RENDER_WRITE_CHARS: usize = 1024 * 1024;
pub(crate) const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
pub(crate) const EXIT_ALT_SCREEN: &str = "\x1b[?1049l";
pub(crate) const DISABLE_AUTOWRAP: &str = "\x1b[?7l";
pub(crate) const ENABLE_AUTOWRAP: &str = "\x1b[?7h";
pub(crate) const ENABLE_BUTTON_MOTION_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1004h\x1b[?1006h";
pub(crate) const ENABLE_ALL_MOTION_MOUSE: &str =
    "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h";
pub(crate) const DISABLE_MOUSE: &str = "\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
pub(crate) const BEGIN_SYNCHRONIZED_OUTPUT: &str = "\x1b[?2026h";
pub(crate) const END_SYNCHRONIZED_OUTPUT: &str = "\x1b[?2026l";
pub(crate) const PAGE_SCROLL_OVERLAP: usize = 4;
pub(crate) const FOCUS_IN: &str = "\x1b[I";
pub(crate) const FOCUS_OUT: &str = "\x1b[O";
pub(crate) const OSC133_ZONE_PREFIX: &str = "\x1b]133;";
pub(crate) const OSC133_PROMPT_START: &str = "\x1b]133;A";

#[derive(Debug, Clone, Copy, Default)]
pub struct TuiStopOptions {
    pub preserve_screen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiRuntimeMode {
    Regular,
    Fullscreen,
}

struct OverlayStackEntry {
    component: Box<dyn Component>,
    options: OverlayOptions,
    hidden: bool,
    focus_order: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayHandle {
    pub id: usize,
}

struct BoundedTerminalWriter {
    buffer: String,
    written_chars: usize,
    writes: Vec<String>,
}

impl BoundedTerminalWriter {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            written_chars: 0,
            writes: Vec::new(),
        }
    }

    fn append(&mut self, value: &str) {
        let mut offset = 0;
        while offset < value.len() {
            let capacity = MAX_RENDER_WRITE_CHARS.saturating_sub(self.buffer.len());
            if capacity == 0 {
                self.flush();
                continue;
            }
            let remaining = value.len() - offset;
            let take = remaining.min(capacity);
            let mut end = offset + take;
            while end > offset && !value.is_char_boundary(end) {
                end -= 1;
            }
            if end == offset {
                self.flush();
                continue;
            }
            self.buffer.push_str(&value[offset..end]);
            offset = end;
            if self.buffer.len() == MAX_RENDER_WRITE_CHARS {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        self.written_chars += self.buffer.len();
        self.writes.push(std::mem::take(&mut self.buffer));
    }

    fn finish(mut self) -> Vec<String> {
        self.flush();
        self.writes
    }
}

fn default_log_directory() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".pi").join("agent")
}

fn is_termux_session() -> bool {
    std::env::var("TERMUX_VERSION").is_ok()
}

fn extract_kitty_image_ids(line: &str) -> Vec<u32> {
    kitty_image_ids(line)
        .into_iter()
        .filter_map(|id| id.parse().ok())
        .collect()
}

fn extract_kitty_image_rows(line: &str) -> u32 {
    parse_kitty_image_header(line)
        .map(|header| header.rows)
        .unwrap_or(1)
}

/// Shared TUI host matching TS `TuiBase`.
pub struct TuiBase {
    pub terminal: Box<dyn TerminalIo>,
    children: Container,
    overlays: Vec<OverlayStackEntry>,
    focused: Option<FocusTarget>,
    focus_order_counter: usize,
    next_overlay_id: usize,
    overlay_ids: Vec<usize>,
    show_hardware_cursor: bool,
    clear_on_shrink: bool,
    pub full_redraws: usize,
    stopped: bool,
    log_directory: PathBuf,
    render_requested: bool,
    now_ms: u64,
    color_scheme_listeners: Vec<Box<dyn Fn(&'static str)>>,
    pending_osc11: usize,
    last_osc11: Option<RgbColor>,
    last_color_scheme: Option<&'static str>,
    on_debug: Option<Box<dyn FnMut()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Child(usize),
    Overlay(usize),
}

impl TuiBase {
    pub fn new(
        terminal: Box<dyn TerminalIo>,
        show_hardware_cursor: Option<bool>,
        log_directory: Option<PathBuf>,
    ) -> Self {
        let show = show_hardware_cursor
            .unwrap_or_else(|| matches!(std::env::var("PI_HARDWARE_CURSOR").as_deref(), Ok("1")));
        Self {
            terminal,
            children: Container::new(),
            overlays: Vec::new(),
            focused: None,
            focus_order_counter: 0,
            next_overlay_id: 1,
            overlay_ids: Vec::new(),
            show_hardware_cursor: show,
            clear_on_shrink: matches!(std::env::var("PI_CLEAR_ON_SHRINK").as_deref(), Ok("1")),
            full_redraws: 0,
            stopped: false,
            log_directory: log_directory.unwrap_or_else(default_log_directory),
            render_requested: false,
            now_ms: 0,
            color_scheme_listeners: Vec::new(),
            pending_osc11: 0,
            last_osc11: None,
            last_color_scheme: None,
            on_debug: None,
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.children.add_child(component);
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }

    pub fn has_overlay_entries(&self) -> bool {
        !self.overlays.is_empty()
    }

    pub fn get_show_hardware_cursor(&self) -> bool {
        self.show_hardware_cursor
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.show_hardware_cursor = enabled;
        if !enabled {
            self.terminal.hide_cursor();
        }
        self.request_render(false);
    }

    pub fn get_clear_on_shrink(&self) -> bool {
        self.clear_on_shrink
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.clear_on_shrink = enabled;
    }

    pub fn set_focus_child(&mut self, index: usize) {
        self.focused = Some(FocusTarget::Child(index));
    }

    pub fn show_overlay(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> OverlayHandle {
        let id = self.next_overlay_id;
        self.next_overlay_id += 1;
        self.focus_order_counter += 1;
        let capturing = !options.non_capturing;
        self.overlays.push(OverlayStackEntry {
            component,
            options,
            hidden: false,
            focus_order: self.focus_order_counter,
        });
        self.overlay_ids.push(id);
        if capturing {
            self.focused = Some(FocusTarget::Overlay(self.overlays.len() - 1));
        }
        self.request_render(false);
        OverlayHandle { id }
    }

    pub fn hide_overlay(&mut self) {
        self.overlays.pop();
        self.overlay_ids.pop();
        self.focused = None;
        self.request_render(false);
    }

    pub fn hide_overlay_id(&mut self, handle: &OverlayHandle) {
        if let Some(index) = self.overlay_ids.iter().position(|id| *id == handle.id) {
            self.overlays.remove(index);
            self.overlay_ids.remove(index);
            self.focused = None;
            self.request_render(false);
        }
    }

    pub fn is_overlay_focused(&self) -> bool {
        matches!(self.focused, Some(FocusTarget::Overlay(_)))
    }

    pub fn is_overlay_handle_focused(&self, handle: &OverlayHandle) -> bool {
        let Some(index) = self.overlay_ids.iter().position(|id| *id == handle.id) else {
            return false;
        };
        self.focused == Some(FocusTarget::Overlay(index))
    }

    pub fn overlay_component_mut(&mut self, handle: &OverlayHandle) -> Option<&mut dyn Component> {
        let index = self.overlay_ids.iter().position(|id| *id == handle.id)?;
        Some(self.overlays[index].component.as_mut())
    }

    pub fn take_terminal(self) -> Box<dyn TerminalIo> {
        self.terminal
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    pub fn set_stopped(&mut self, stopped: bool) {
        self.stopped = stopped;
    }

    pub fn request_render(&mut self, force: bool) {
        if force {
            self.render_requested = true;
        } else if self.render_requested {
            return;
        } else {
            self.render_requested = true;
        }
    }

    pub fn tick(&mut self, ms: u64) {
        self.now_ms = self.now_ms.saturating_add(ms);
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn render_children(&self, width: usize) -> Vec<String> {
        self.children.render(width)
    }

    pub fn invalidate(&mut self) {
        self.children.invalidate();
        for overlay in &mut self.overlays {
            overlay.component.invalidate();
        }
    }

    pub fn handle_terminal_input(&mut self, data: &str) {
        if self.consume_osc11(data)
            || self.consume_color_scheme(data)
            || self.consume_cell_size(data)
        {
            return;
        }
        let kb = Keybindings::defaults();
        if kb.matches(data, "app.debug")
            || (data.contains("\x1b[100;6u") && self.on_debug.is_some())
        {
            if let Some(on_debug) = &mut self.on_debug {
                on_debug();
                return;
            }
        }
        if data == "\x04" && self.on_debug.is_some() {
            // Shift+Ctrl+D is not always this; keep hook available.
        }
        let focus = self.focused;
        match focus {
            Some(FocusTarget::Child(index)) => {
                if let Some(child) = self.children.children.get_mut(index) {
                    if is_key_release(data) && !child.wants_key_release() {
                        return;
                    }
                    child.handle_input(data);
                }
            }
            Some(FocusTarget::Overlay(index)) => {
                if let Some(overlay) = self.overlays.get_mut(index) {
                    if is_key_release(data) && !overlay.component.wants_key_release() {
                        return;
                    }
                    overlay.component.handle_input(data);
                }
            }
            None => {
                if let Some(child) = self.children.children.last_mut() {
                    if !is_key_release(data) || child.wants_key_release() {
                        child.handle_input(data);
                    }
                }
            }
        }
        self.request_render(true);
    }

    fn consume_osc11(&mut self, data: &str) -> bool {
        if self.pending_osc11 == 0 || !is_osc11_background_color_response(data) {
            return false;
        }
        self.pending_osc11 -= 1;
        self.last_osc11 = parse_osc11_background_color(data);
        true
    }

    fn consume_color_scheme(&mut self, data: &str) -> bool {
        let Some(scheme) = parse_terminal_color_scheme_report(data) else {
            return false;
        };
        self.last_color_scheme = Some(scheme);
        for listener in &self.color_scheme_listeners {
            listener(scheme);
        }
        true
    }

    fn consume_cell_size(&mut self, data: &str) -> bool {
        let Some(rest) = data.strip_prefix("\x1b[6;") else {
            return false;
        };
        let Some(body) = rest.strip_suffix('t') else {
            return false;
        };
        let mut parts = body.split(';');
        let (Some(height), Some(width)) = (parts.next(), parts.next()) else {
            return false;
        };
        let Ok(height_px) = height.parse::<u32>() else {
            return true;
        };
        let Ok(width_px) = width.parse::<u32>() else {
            return true;
        };
        if height_px == 0 || width_px == 0 {
            return true;
        }
        crate::image::set_cell_dimensions(width_px, height_px);
        self.invalidate();
        self.request_render(false);
        true
    }

    pub fn query_terminal_background_color(&mut self) {
        self.pending_osc11 += 1;
        self.terminal.write(OSC_11_QUERY);
    }

    pub fn query_terminal_color_scheme(&mut self) {
        self.terminal.write(COLOR_SCHEME_QUERY);
    }

    pub fn last_osc11(&self) -> Option<RgbColor> {
        self.last_osc11
    }

    pub fn last_color_scheme(&self) -> Option<&'static str> {
        self.last_color_scheme
    }

    pub fn composite_overlays(
        &self,
        lines: &[String],
        term_width: usize,
        term_height: usize,
    ) -> Vec<String> {
        if self.overlays.is_empty() {
            return lines.to_vec();
        }
        let mut result = lines.to_vec();
        let mut min_lines_needed = result.len();
        let mut rendered = Vec::new();
        let mut visible: Vec<usize> = self
            .overlays
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.hidden && entry.options.visible != Some(false))
            .map(|(index, _)| index)
            .collect();
        visible.sort_by_key(|&index| self.overlays[index].focus_order);
        for index in visible {
            let entry = &self.overlays[index];
            let layout = resolve_overlay_layout(&entry.options, 0, term_width, term_height);
            let mut overlay_lines = entry.component.render(layout.width);
            if let Some(max_height) = layout.max_height {
                if overlay_lines.len() > max_height {
                    overlay_lines.truncate(max_height);
                }
            }
            let placed = resolve_overlay_layout(
                &entry.options,
                overlay_lines.len(),
                term_width,
                term_height,
            );
            min_lines_needed = min_lines_needed.max(placed.row + overlay_lines.len());
            rendered.push((overlay_lines, placed.row, placed.col, placed.width));
        }
        let working_height = result.len().max(term_height).max(min_lines_needed);
        while result.len() < working_height {
            result.push(String::new());
        }
        let viewport_start = working_height.saturating_sub(term_height);
        for (overlay_lines, row, col, width) in rendered {
            for (i, line) in overlay_lines.iter().enumerate() {
                let idx = viewport_start + row + i;
                if idx < result.len() {
                    let truncated = if visible_width(line) > width {
                        slice_by_column(line, 0, width, true)
                    } else {
                        line.clone()
                    };
                    result[idx] =
                        composite_tui_line(&result[idx], &truncated, col, width, term_width);
                }
            }
        }
        result
    }

    pub fn apply_line_resets(&self, lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                if is_image_line(line) {
                    line.clone()
                } else {
                    format!("{}{SEGMENT_RESET}", normalize_terminal_output(line))
                }
            })
            .collect()
    }

    pub fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<(usize, usize)> {
        let viewport_top = lines.len().saturating_sub(height);
        for row in (viewport_top..lines.len()).rev() {
            if let Some(marker_index) = lines[row].find(CURSOR_MARKER) {
                let col = visible_width(&lines[row][..marker_index]);
                let mut next = String::new();
                next.push_str(&lines[row][..marker_index]);
                next.push_str(&lines[row][marker_index + CURSOR_MARKER.len()..]);
                lines[row] = next;
                return Some((row, col));
            }
        }
        None
    }

    pub fn log_directory(&self) -> &std::path::Path {
        &self.log_directory
    }
}

/// Main-screen TUI matching TS `TuiMainScreen`.
pub struct TuiMainScreen {
    pub base: TuiBase,
    previous_lines: Vec<String>,
    previous_kitty_image_ids: Vec<u32>,
    previous_width: usize,
    previous_height: usize,
    cursor_row: usize,
    hardware_cursor_row: usize,
    max_lines_rendered: usize,
    previous_viewport_top: usize,
}

impl TuiMainScreen {
    pub fn new(terminal: Box<dyn TerminalIo>) -> Self {
        Self::with_options(terminal, None, None)
    }

    pub fn with_options(
        terminal: Box<dyn TerminalIo>,
        show_hardware_cursor: Option<bool>,
        log_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            base: TuiBase::new(terminal, show_hardware_cursor, log_directory),
            previous_lines: Vec::new(),
            previous_kitty_image_ids: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            cursor_row: 0,
            hardware_cursor_row: 0,
            max_lines_rendered: 0,
            previous_viewport_top: 0,
        }
    }

    pub fn mode(&self) -> TuiRuntimeMode {
        TuiRuntimeMode::Regular
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.base.add_child(component);
    }

    pub fn set_focus_child(&mut self, index: usize) {
        self.base.set_focus_child(index);
    }

    pub fn handle_input(&mut self, data: &str) {
        self.base.handle_terminal_input(data);
        self.render_now(false);
    }

    pub fn request_render(&mut self, force: bool) {
        self.base.request_render(force);
        if force {
            self.reset_render_state();
        }
    }

    pub fn start(&mut self) {
        self.base.stopped = false;
        self.base.terminal.start();
        self.render_now(false);
    }

    pub fn stop(&mut self, options: TuiStopOptions) {
        self.base.stopped = true;
        if !options.preserve_screen && !self.previous_lines.is_empty() {
            self.base.terminal.write(" ");
            let target_row = self.previous_lines.len();
            let line_diff = target_row as isize - self.hardware_cursor_row as isize;
            match line_diff.cmp(&0) {
                std::cmp::Ordering::Greater => {
                    self.base.terminal.write(&format!("\x1b[{line_diff}B"));
                }
                std::cmp::Ordering::Less => {
                    self.base.terminal.write(&format!("\x1b[{}A", -line_diff));
                }
                std::cmp::Ordering::Equal => {}
            }
            self.base.terminal.write("\r\n");
        }
        self.base.terminal.show_cursor();
        self.base.terminal.stop();
    }

    pub fn capture_render_state(&self) -> TuiMainScreenRenderState {
        TuiMainScreenRenderState {
            previous_lines: self.previous_lines.clone(),
            previous_width: self.previous_width,
            previous_height: self.previous_height,
            cursor_row: self.cursor_row,
            hardware_cursor_row: self.hardware_cursor_row,
            max_lines_rendered: self.max_lines_rendered,
            previous_viewport_top: self.previous_viewport_top,
        }
    }

    pub fn restore_render_state(&mut self, state: TuiMainScreenRenderState) {
        self.previous_lines = state
            .previous_lines
            .into_iter()
            .map(|line| {
                if is_image_line(&line) {
                    String::new()
                } else {
                    line
                }
            })
            .collect();
        self.previous_kitty_image_ids.clear();
        self.previous_width = state.previous_width;
        self.previous_height = state.previous_height;
        self.cursor_row = state.cursor_row;
        self.hardware_cursor_row = state.hardware_cursor_row;
        self.max_lines_rendered = state.max_lines_rendered;
        self.previous_viewport_top = state.previous_viewport_top;
    }

    fn reset_render_state(&mut self) {
        self.previous_lines.clear();
        self.previous_width = usize::MAX;
        self.previous_height = usize::MAX;
        self.cursor_row = 0;
        self.hardware_cursor_row = 0;
        self.max_lines_rendered = 0;
        self.previous_viewport_top = 0;
    }

    pub fn take_terminal(self) -> Box<dyn TerminalIo> {
        self.base.take_terminal()
    }

    pub fn render_now(&mut self, force: bool) {
        if force {
            self.reset_render_state();
        }
        self.base.render_requested = false;
        self.do_render();
    }

    pub fn full_redraws(&self) -> usize {
        self.base.full_redraws
    }

    fn get_kitty_image_reserved_rows(
        &self,
        lines: &[String],
        index: usize,
        max_index: usize,
    ) -> usize {
        let rows = extract_kitty_image_rows(lines.get(index).map(String::as_str).unwrap_or(""));
        if rows <= 1 {
            return 1;
        }
        let max_rows = (rows as usize)
            .min(max_index.saturating_sub(index) + 1)
            .min(lines.len().saturating_sub(index));
        let mut reserved = 1;
        while reserved < max_rows {
            let line = lines
                .get(index + reserved)
                .map(String::as_str)
                .unwrap_or("");
            if is_image_line(line) || visible_width(line) > 0 {
                break;
            }
            reserved += 1;
        }
        reserved
    }

    fn collect_kitty_image_ids(lines: &[String]) -> Vec<u32> {
        let mut ids = Vec::new();
        for line in lines {
            ids.extend(extract_kitty_image_ids(line));
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn delete_kitty_images(ids: &[u32]) -> String {
        let mut buffer = String::new();
        for id in ids {
            buffer.push_str(&delete_kitty_image(*id));
        }
        buffer
    }

    fn expand_changed_range_for_kitty_images(
        &self,
        first_changed: usize,
        last_changed: usize,
        new_lines: &[String],
    ) -> (usize, usize) {
        let mut expanded_first = first_changed;
        let mut expanded_last = last_changed;
        let mut expand = |lines: &[String]| {
            for (i, line) in lines.iter().enumerate() {
                if extract_kitty_image_ids(line).is_empty() {
                    continue;
                }
                let block_end =
                    i + self.get_kitty_image_reserved_rows(lines, i, lines.len().saturating_sub(1))
                        - 1;
                if i >= first_changed || (i <= last_changed && block_end >= first_changed) {
                    expanded_first = expanded_first.min(i);
                    expanded_last = expanded_last.max(block_end);
                }
            }
        };
        expand(&self.previous_lines);
        expand(new_lines);
        (expanded_first, expanded_last)
    }

    fn delete_changed_kitty_images(&self, first_changed: usize, last_changed: usize) -> String {
        if first_changed == usize::MAX || last_changed < first_changed {
            return String::new();
        }
        let mut ids = Vec::new();
        let max_line = last_changed.min(self.previous_lines.len().saturating_sub(1));
        if first_changed <= max_line {
            for line in &self.previous_lines[first_changed..=max_line] {
                ids.extend(extract_kitty_image_ids(line));
            }
        }
        Self::delete_kitty_images(&ids)
    }

    fn write_chunks(&mut self, chunks: Vec<String>) {
        for chunk in chunks {
            self.base.terminal.write(&chunk);
        }
    }

    fn log_redraw(&self, reason: &str, new_len: usize, height: usize) {
        if std::env::var("PI_DEBUG_REDRAW").as_deref() != Ok("1") {
            return;
        }
        let path = self.base.log_directory.join("pi-debug.log");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let msg = format!(
            "[tui] fullRender: {reason} (prev={}, new={new_len}, height={height})\n",
            self.previous_lines.len()
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(msg.as_bytes())
            });
    }

    fn do_render(&mut self) {
        if self.base.stopped {
            return;
        }
        let width = self.base.terminal.columns().max(1);
        let height = self.base.terminal.rows().max(1);
        let width_changed = self.previous_width != 0
            && self.previous_width != usize::MAX
            && self.previous_width != width;
        let height_changed = self.previous_height != 0
            && self.previous_height != usize::MAX
            && self.previous_height != height;
        let previous_buffer_length =
            if self.previous_height > 0 && self.previous_height != usize::MAX {
                self.previous_viewport_top + self.previous_height
            } else {
                height
            };
        let mut prev_viewport_top = if height_changed {
            previous_buffer_length.saturating_sub(height)
        } else {
            self.previous_viewport_top
        };
        let mut viewport_top = prev_viewport_top;
        let mut hardware_cursor_row = self.hardware_cursor_row;

        let mut new_lines = self.base.render_children(width);
        if self.base.has_overlay_entries() {
            new_lines = self.base.composite_overlays(&new_lines, width, height);
        }
        let cursor_pos = TuiBase::extract_cursor_position(&mut new_lines, height);
        new_lines = self.base.apply_line_resets(&new_lines);

        let full_render = |this: &mut Self,
                           clear: bool,
                           new_lines: &[String],
                           cursor_pos: Option<(usize, usize)>| {
            this.base.full_redraws += 1;
            let mut output = BoundedTerminalWriter::new();
            output.append(BEGIN_SYNCHRONIZED_OUTPUT);
            if clear {
                output.append(&Self::delete_kitty_images(&this.previous_kitty_image_ids));
                output.append("\x1b[2J\x1b[H\x1b[3J");
            }
            let mut i = 0;
            while i < new_lines.len() {
                if i > 0 {
                    output.append("\r\n");
                }
                let line = &new_lines[i];
                let reserved = if is_image_line(line) {
                    this.get_kitty_image_reserved_rows(
                        new_lines,
                        i,
                        new_lines.len().saturating_sub(1),
                    )
                } else {
                    1
                };
                if reserved > 1 && reserved <= height {
                    for _ in 1..reserved {
                        output.append("\r\n");
                    }
                    output.append(&format!("\x1b[{}A", reserved - 1));
                    output.append(line);
                    output.append(&format!("\x1b[{}B", reserved - 1));
                    i += reserved - 1;
                    i += 1;
                    continue;
                }
                output.append(line);
                i += 1;
            }
            output.append(END_SYNCHRONIZED_OUTPUT);
            this.write_chunks(output.finish());
            this.cursor_row = new_lines.len().saturating_sub(1);
            this.hardware_cursor_row = this.cursor_row;
            if clear {
                this.max_lines_rendered = new_lines.len();
            } else {
                this.max_lines_rendered = this.max_lines_rendered.max(new_lines.len());
            }
            let buffer_length = height.max(new_lines.len());
            this.previous_viewport_top = buffer_length.saturating_sub(height);
            this.position_hardware_cursor(cursor_pos, new_lines.len());
            this.previous_lines = new_lines.to_vec();
            this.previous_kitty_image_ids = Self::collect_kitty_image_ids(new_lines);
            this.previous_width = width;
            this.previous_height = height;
        };

        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            self.log_redraw("first render", new_lines.len(), height);
            full_render(self, false, &new_lines, cursor_pos);
            return;
        }
        if width_changed {
            self.log_redraw("terminal width changed", new_lines.len(), height);
            full_render(self, true, &new_lines, cursor_pos);
            return;
        }
        if height_changed && !is_termux_session() {
            self.log_redraw("terminal height changed", new_lines.len(), height);
            full_render(self, true, &new_lines, cursor_pos);
            return;
        }
        if self.base.get_clear_on_shrink()
            && new_lines.len() < self.max_lines_rendered
            && !self.base.has_overlay_entries()
        {
            self.log_redraw("clearOnShrink", new_lines.len(), height);
            full_render(self, true, &new_lines, cursor_pos);
            return;
        }

        let mut first_changed = None;
        let mut last_changed = 0usize;
        let max_lines = new_lines.len().max(self.previous_lines.len());
        for i in 0..max_lines {
            let old = self.previous_lines.get(i).map(String::as_str).unwrap_or("");
            let new = new_lines.get(i).map(String::as_str).unwrap_or("");
            if old != new {
                if first_changed.is_none() {
                    first_changed = Some(i);
                }
                last_changed = i;
            }
        }
        let appended = new_lines.len() > self.previous_lines.len();
        if appended {
            if first_changed.is_none() {
                first_changed = Some(self.previous_lines.len());
            }
            last_changed = new_lines.len().saturating_sub(1);
        }
        let mut first_changed = match first_changed {
            Some(value) => value,
            None => {
                self.position_hardware_cursor(cursor_pos, new_lines.len());
                self.previous_viewport_top = prev_viewport_top;
                self.previous_height = height;
                return;
            }
        };
        let expanded =
            self.expand_changed_range_for_kitty_images(first_changed, last_changed, &new_lines);
        first_changed = expanded.0;
        last_changed = expanded.1;
        let append_start =
            appended && first_changed == self.previous_lines.len() && first_changed > 0;

        if first_changed >= new_lines.len() {
            if self.previous_lines.len() > new_lines.len() {
                let mut output = BoundedTerminalWriter::new();
                output.append(BEGIN_SYNCHRONIZED_OUTPUT);
                output.append(&self.delete_changed_kitty_images(first_changed, last_changed));
                let target_row = new_lines.len().saturating_sub(1);
                if target_row < prev_viewport_top {
                    self.log_redraw("deleted lines moved viewport up", new_lines.len(), height);
                    full_render(self, true, &new_lines, cursor_pos);
                    return;
                }
                let current_screen_row = hardware_cursor_row.saturating_sub(prev_viewport_top);
                let target_screen_row = target_row.saturating_sub(viewport_top);
                let line_diff = target_screen_row as isize - current_screen_row as isize;
                match line_diff.cmp(&0) {
                    std::cmp::Ordering::Greater => output.append(&format!("\x1b[{line_diff}B")),
                    std::cmp::Ordering::Less => output.append(&format!("\x1b[{}A", -line_diff)),
                    std::cmp::Ordering::Equal => {}
                }
                output.append("\r");
                let extra_lines = self.previous_lines.len() - new_lines.len();
                if extra_lines > height {
                    self.log_redraw("extraLines > height", new_lines.len(), height);
                    full_render(self, true, &new_lines, cursor_pos);
                    return;
                }
                let clear_start_offset = if new_lines.is_empty() { 0 } else { 1 };
                if extra_lines > 0 && clear_start_offset > 0 {
                    output.append(&format!("\x1b[{clear_start_offset}B"));
                }
                for i in 0..extra_lines {
                    output.append("\r\x1b[2K");
                    if i + 1 < extra_lines {
                        output.append("\x1b[1B");
                    }
                }
                let move_back = extra_lines.saturating_sub(1) + clear_start_offset;
                if move_back > 0 {
                    output.append(&format!("\x1b[{move_back}A"));
                }
                output.append(END_SYNCHRONIZED_OUTPUT);
                self.write_chunks(output.finish());
                self.cursor_row = target_row;
                self.hardware_cursor_row = target_row;
            }
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_kitty_image_ids = Self::collect_kitty_image_ids(&self.previous_lines);
            self.previous_width = width;
            self.previous_height = height;
            self.previous_viewport_top = prev_viewport_top;
            return;
        }

        if first_changed < prev_viewport_top {
            self.log_redraw("firstChanged < viewportTop", new_lines.len(), height);
            full_render(self, true, &new_lines, cursor_pos);
            return;
        }

        let mut output = BoundedTerminalWriter::new();
        output.append(BEGIN_SYNCHRONIZED_OUTPUT);
        output.append(&self.delete_changed_kitty_images(first_changed, last_changed));
        let prev_viewport_bottom = prev_viewport_top + height.saturating_sub(1);
        let move_target_row = if append_start {
            first_changed.saturating_sub(1)
        } else {
            first_changed
        };
        if move_target_row > prev_viewport_bottom {
            let current_screen_row = hardware_cursor_row
                .saturating_sub(prev_viewport_top)
                .min(height.saturating_sub(1));
            let move_to_bottom = height.saturating_sub(1).saturating_sub(current_screen_row);
            if move_to_bottom > 0 {
                output.append(&format!("\x1b[{move_to_bottom}B"));
            }
            let scroll = move_target_row - prev_viewport_bottom;
            output.append(&"\r\n".repeat(scroll));
            prev_viewport_top += scroll;
            viewport_top += scroll;
            hardware_cursor_row = move_target_row;
        }
        let current_screen_row = hardware_cursor_row.saturating_sub(prev_viewport_top);
        let target_screen_row = move_target_row.saturating_sub(viewport_top);
        let line_diff = target_screen_row as isize - current_screen_row as isize;
        match line_diff.cmp(&0) {
            std::cmp::Ordering::Greater => output.append(&format!("\x1b[{line_diff}B")),
            std::cmp::Ordering::Less => output.append(&format!("\x1b[{}A", -line_diff)),
            std::cmp::Ordering::Equal => {}
        }
        output.append(if append_start { "\r\n" } else { "\r" });

        let render_end = last_changed.min(new_lines.len().saturating_sub(1));
        let mut i = first_changed;
        while i <= render_end {
            if i > first_changed {
                output.append("\r\n");
            }
            let line = &new_lines[i];
            let reserved = if is_image_line(line) {
                self.get_kitty_image_reserved_rows(&new_lines, i, render_end)
            } else {
                1
            };
            if reserved > 1 {
                let image_start_screen_row = i as isize - viewport_top as isize;
                if image_start_screen_row < 0 || image_start_screen_row as usize + reserved > height
                {
                    self.log_redraw(
                        "kitty image pre-clear would scroll",
                        new_lines.len(),
                        height,
                    );
                    full_render(self, true, &new_lines, cursor_pos);
                    return;
                }
                output.append("\x1b[2K");
                for _ in 1..reserved {
                    output.append("\r\n\x1b[2K");
                }
                output.append(&format!("\x1b[{}A", reserved - 1));
                output.append(line);
                output.append(&format!("\x1b[{}B", reserved - 1));
                i += reserved - 1;
                i += 1;
                continue;
            }
            output.append("\x1b[2K");
            if !is_image_line(line) && visible_width(line) > width {
                let crash = self.base.log_directory.join("pi-crash.log");
                if let Some(parent) = crash.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(
                    crash,
                    format!(
                        "Rendered line {i} exceeds terminal width ({} > {width}).",
                        visible_width(line)
                    ),
                );
                self.stop(TuiStopOptions::default());
                panic!(
                    "Rendered line {i} exceeds terminal width ({} > {width}).",
                    visible_width(line)
                );
            }
            output.append(line);
            i += 1;
        }

        let mut final_cursor_row = render_end;
        if self.previous_lines.len() > new_lines.len() {
            if render_end < new_lines.len().saturating_sub(1) {
                let move_down = new_lines.len() - 1 - render_end;
                output.append(&format!("\x1b[{move_down}B"));
                final_cursor_row = new_lines.len() - 1;
            }
            let extra_lines = self.previous_lines.len() - new_lines.len();
            for _ in new_lines.len()..self.previous_lines.len() {
                output.append("\r\n\x1b[2K");
            }
            output.append(&format!("\x1b[{extra_lines}A"));
        }
        output.append(END_SYNCHRONIZED_OUTPUT);
        self.write_chunks(output.finish());
        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = final_cursor_row;
        self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        self.previous_viewport_top =
            prev_viewport_top.max(final_cursor_row.saturating_add(1).saturating_sub(height));
        self.position_hardware_cursor(cursor_pos, new_lines.len());
        self.previous_lines = new_lines;
        self.previous_kitty_image_ids = Self::collect_kitty_image_ids(&self.previous_lines);
        self.previous_width = width;
        self.previous_height = height;
    }

    fn position_hardware_cursor(&mut self, cursor_pos: Option<(usize, usize)>, total_lines: usize) {
        let Some((row, col)) = cursor_pos else {
            self.base.terminal.hide_cursor();
            return;
        };
        if total_lines == 0 {
            self.base.terminal.hide_cursor();
            return;
        }
        let target_row = row.min(total_lines - 1);
        let target_col = col;
        let row_delta = target_row as isize - self.hardware_cursor_row as isize;
        let mut buffer = String::new();
        match row_delta.cmp(&0) {
            std::cmp::Ordering::Greater => buffer.push_str(&format!("\x1b[{row_delta}B")),
            std::cmp::Ordering::Less => buffer.push_str(&format!("\x1b[{}A", -row_delta)),
            std::cmp::Ordering::Equal => {}
        }
        buffer.push_str(&format!("\x1b[{}G", target_col + 1));
        if !buffer.is_empty() {
            self.base.terminal.write(&buffer);
        }
        self.hardware_cursor_row = target_row;
        if self.base.get_show_hardware_cursor() {
            self.base.terminal.show_cursor();
        } else {
            self.base.terminal.hide_cursor();
        }
    }
}

#[derive(Debug, Clone)]
pub struct TuiMainScreenRenderState {
    pub previous_lines: Vec<String>,
    pub previous_width: usize,
    pub previous_height: usize,
    pub cursor_row: usize,
    pub hardware_cursor_row: usize,
    pub max_lines_rendered: usize,
    pub previous_viewport_top: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Text;
    use crate::terminal::MemoryTerminal;
    use crate::tui_alt_screen::TuiAltScreen;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct SharedLines {
        lines: Rc<RefCell<Vec<String>>>,
        renders: Rc<RefCell<usize>>,
    }

    impl Component for SharedLines {
        fn render(&self, _width: usize) -> Vec<String> {
            *self.renders.borrow_mut() += 1;
            self.lines.borrow().clone()
        }
        fn handle_input(&mut self, data: &str) {
            *self.lines.borrow_mut() = vec![data.to_string()];
        }
        fn invalidate(&mut self) {}
    }

    #[test]
    fn main_screen_bounded_full_and_diff_renders() {
        let terminal = MemoryTerminal::new(80, 24);
        let mut tui = TuiMainScreen::new(Box::new(terminal));
        let kitty = format!("\x1b_Ga=T,f=100;{}\x1b\\", "A".repeat(1_200_000));
        let lines = Rc::new(RefCell::new(vec![kitty.clone(), kitty]));
        let component = SharedLines {
            lines: lines.clone(),
            renders: Rc::new(RefCell::new(0)),
        };
        tui.add_child(Box::new(component));
        tui.render_now(false);
        let writes = (*tui.base.terminal)
            .as_any()
            .downcast_ref::<MemoryTerminal>()
            .expect("memory")
            .writes
            .clone();
        assert!(writes.len() > 2);
        assert!(writes
            .iter()
            .all(|write| write.len() <= MAX_RENDER_WRITE_CHARS));
        let joined = writes.concat();
        assert!(joined.starts_with(BEGIN_SYNCHRONIZED_OUTPUT));
        assert!(joined.ends_with(END_SYNCHRONIZED_OUTPUT));

        let terminal = MemoryTerminal::new(80, 24);
        let mut tui = TuiMainScreen::new(Box::new(terminal));
        let lines = Rc::new(RefCell::new(vec!["before".into()]));
        tui.add_child(Box::new(SharedLines {
            lines: lines.clone(),
            renders: Rc::new(RefCell::new(0)),
        }));
        tui.render_now(false);
        (*tui.base.terminal)
            .as_any_mut()
            .downcast_mut::<MemoryTerminal>()
            .expect("memory")
            .clear_writes();
        let kitty = format!("\x1b_Ga=T,f=100;{}\x1b\\", "A".repeat(1_200_000));
        *lines.borrow_mut() = vec!["before".into(), kitty.clone(), kitty];
        tui.render_now(false);
        let writes = &(*tui.base.terminal)
            .as_any()
            .downcast_ref::<MemoryTerminal>()
            .expect("memory")
            .writes;
        let output = writes.concat();
        assert!(output.starts_with(BEGIN_SYNCHRONIZED_OUTPUT));
        assert!(output.ends_with(END_SYNCHRONIZED_OUTPUT));
        assert!(!output.contains("\x1b[2J"));
    }

    #[test]
    fn main_screen_input_renders_immediately() {
        let terminal = MemoryTerminal::new(40, 10);
        let mut tui = TuiMainScreen::new(Box::new(terminal));
        let lines = Rc::new(RefCell::new(vec!["initial".into()]));
        let renders = Rc::new(RefCell::new(0));
        tui.add_child(Box::new(SharedLines {
            lines: lines.clone(),
            renders: renders.clone(),
        }));
        tui.set_focus_child(0);
        tui.render_now(false);
        let before = *renders.borrow();
        tui.handle_input("typed");
        assert!(*renders.borrow() > before);
        assert_eq!(*lines.borrow(), vec!["typed".to_string()]);
    }

    #[test]
    fn alt_screen_writes_enter_sequence_and_follows_end() {
        let terminal = MemoryTerminal::new(20, 4);
        let mut tui = TuiAltScreen::new(Box::new(terminal));
        tui.add_child(Box::new(Text {
            value: "1\n2\n3\n4\n5\n6".into(),
        }));
        tui.start();
        let output = (*tui.base.terminal)
            .as_any()
            .downcast_ref::<MemoryTerminal>()
            .expect("memory")
            .output();
        assert!(output.contains(ENTER_ALT_SCREEN));
        assert!(output.contains(DISABLE_AUTOWRAP));
        assert!(tui.is_following_output());
        tui.scroll_by(-2);
        assert!(!tui.is_following_output());
        tui.scroll_to_bottom();
        assert!(tui.is_following_output());
        tui.stop(TuiStopOptions::default());
        let output = (*tui.base.terminal)
            .as_any()
            .downcast_ref::<MemoryTerminal>()
            .expect("memory")
            .output();
        assert!(output.contains(EXIT_ALT_SCREEN));
    }

    #[test]
    fn debug_redraw_writes_log() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PI_DEBUG_REDRAW", "1");
        let terminal = MemoryTerminal::new(40, 10);
        let mut tui =
            TuiMainScreen::with_options(Box::new(terminal), None, Some(dir.path().to_path_buf()));
        tui.add_child(Box::new(Text {
            value: "test".into(),
        }));
        tui.render_now(false);
        std::env::remove_var("PI_DEBUG_REDRAW");
        let log = std::fs::read_to_string(dir.path().join("pi-debug.log")).unwrap();
        assert!(log.contains("fullRender: first render"));
    }
}
