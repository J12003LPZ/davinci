//! TUI host: children + overlays + differential render + mouse hit-testing.

use crate::component::Component;
use crate::diff::{composite_tui_line, visible_width, DiffScreen};
use crate::mouse::{overlay_rect, parse_sgr_mouse, MouseEvent, Rect};
use crate::terminal::TuiMode;

#[derive(Debug, Clone)]
pub struct OverlayOptions {
    pub width: Option<usize>,
    pub anchor: String,
    pub offset_x: i32,
    pub offset_y: i32,
    pub non_capturing: bool,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            width: None,
            anchor: "center".into(),
            offset_x: 0,
            offset_y: 0,
            non_capturing: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OverlayHandle {
    pub id: u64,
}

#[derive(Clone)]
struct OverlayEntry {
    id: u64,
    lines: Vec<String>,
    options: OverlayOptions,
    hidden: bool,
}

pub struct Tui {
    pub mode: TuiMode,
    pub columns: usize,
    pub rows: usize,
    children: Vec<Vec<String>>,
    overlays: Vec<OverlayEntry>,
    next_id: u64,
    screen: DiffScreen,
}

impl Tui {
    pub fn new(mode: TuiMode, columns: usize, rows: usize) -> Self {
        Self {
            mode,
            columns,
            rows,
            children: Vec::new(),
            overlays: Vec::new(),
            next_id: 1,
            screen: DiffScreen::new(columns, rows),
        }
    }

    pub fn add_child_lines(&mut self, lines: Vec<String>) {
        self.children.push(lines);
    }

    pub fn clear_children(&mut self) {
        self.children.clear();
    }

    pub fn show_overlay(&mut self, lines: Vec<String>, options: OverlayOptions) -> OverlayHandle {
        let id = self.next_id;
        self.next_id += 1;
        self.overlays.push(OverlayEntry {
            id,
            lines,
            options,
            hidden: false,
        });
        OverlayHandle { id }
    }

    pub fn hide_overlay(&mut self, handle: OverlayHandle) {
        self.overlays.retain(|o| o.id != handle.id);
    }

    pub fn set_hidden(&mut self, handle: OverlayHandle, hidden: bool) {
        if let Some(entry) = self.overlays.iter_mut().find(|o| o.id == handle.id) {
            entry.hidden = hidden;
        }
    }

    pub fn has_overlay(&self) -> bool {
        self.overlays.iter().any(|o| !o.hidden)
    }

    pub fn full_redraws(&self) -> u32 {
        self.screen.full_redraws
    }

    pub fn compose_frame(&self) -> Vec<String> {
        let mut frame: Vec<String> = self.children.iter().flatten().cloned().collect();
        if frame.len() < self.rows {
            frame.resize(self.rows, String::new());
        }
        for line in &mut frame {
            if visible_width(line) > self.columns {
                *line = crate::diff::truncate_visible(line, self.columns);
            }
        }
        for overlay in self.overlays.iter().filter(|o| !o.hidden) {
            let width = overlay
                .options
                .width
                .unwrap_or_else(|| {
                    overlay
                        .lines
                        .iter()
                        .map(|l| visible_width(l))
                        .max()
                        .unwrap_or(0)
                })
                .min(self.columns);
            let height = overlay.lines.len().min(self.rows);
            let rect = overlay_rect(
                self.columns,
                self.rows,
                width,
                height,
                &overlay.options.anchor,
                overlay.options.offset_x,
                overlay.options.offset_y,
            );
            for (i, overlay_line) in overlay.lines.iter().enumerate() {
                let row = rect.row + i;
                if row >= frame.len() {
                    break;
                }
                frame[row] = composite_tui_line(
                    &frame[row],
                    overlay_line,
                    rect.col,
                    rect.width,
                    self.columns,
                );
            }
        }
        frame.truncate(self.rows);
        frame
    }

    pub fn render_now(&mut self, force: bool) -> String {
        let frame = self.compose_frame();
        self.screen.render(&frame, force)
    }

    pub fn overlay_at(&self, col: usize, row: usize) -> Option<u64> {
        for overlay in self.overlays.iter().rev().filter(|o| !o.hidden) {
            let width = overlay.options.width.unwrap_or_else(|| {
                overlay
                    .lines
                    .iter()
                    .map(|l| visible_width(l))
                    .max()
                    .unwrap_or(0)
            });
            let rect = overlay_rect(
                self.columns,
                self.rows,
                width.min(self.columns),
                overlay.lines.len().min(self.rows),
                &overlay.options.anchor,
                overlay.options.offset_x,
                overlay.options.offset_y,
            );
            if rect.contains(col, row) {
                return Some(overlay.id);
            }
        }
        None
    }

    pub fn handle_input(&self, data: &str) -> Option<MouseEvent> {
        parse_sgr_mouse(data)
    }

    pub fn hit_test_mouse(&self, data: &str) -> Option<(MouseEvent, Option<u64>)> {
        let ev = parse_sgr_mouse(data)?;
        let hit = self.overlay_at(ev.col, ev.row);
        Some((ev, hit))
    }

    pub fn focused_overlay_rect(&self) -> Option<Rect> {
        let overlay = self.overlays.iter().rev().find(|o| !o.hidden)?;
        let width = overlay.options.width.unwrap_or_else(|| {
            overlay
                .lines
                .iter()
                .map(|l| visible_width(l))
                .max()
                .unwrap_or(0)
        });
        Some(overlay_rect(
            self.columns,
            self.rows,
            width.min(self.columns),
            overlay.lines.len().min(self.rows),
            &overlay.options.anchor,
            overlay.options.offset_x,
            overlay.options.offset_y,
        ))
    }
}

impl Component for Tui {
    fn render(&self, width: usize) -> Vec<String> {
        self.compose_frame()
            .into_iter()
            .map(|line| {
                if visible_width(&line) > width {
                    crate::diff::truncate_visible(&line, width)
                } else {
                    line
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_composites_without_exceeding_width() {
        let mut tui = Tui::new(TuiMode::Regular, 80, 8);
        tui.add_child_lines(vec!["base".into(); 8]);
        tui.show_overlay(
            vec!["X".repeat(100)],
            OverlayOptions {
                width: Some(20),
                ..OverlayOptions::default()
            },
        );
        let frame = tui.compose_frame();
        assert!(frame.iter().all(|l| visible_width(l) <= 80));
        let seq = tui.render_now(false);
        assert!(seq.contains("\u{1b}[?2026h"));
        let rect = tui.focused_overlay_rect().expect("overlay");
        let click = format!("\u{1b}[<0;{};{}M", rect.col + 1, rect.row + 1);
        let hit = tui.hit_test_mouse(&click);
        assert!(hit.unwrap().1.is_some());
    }
}
