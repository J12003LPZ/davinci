//! Bounded cell drawing, converted back to the normal sheet line contract.
use super::graph_layout::GraphLayout;
use crate::davinci::{
    model::{GraphViewMode, Model},
    theme::State,
    ui,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Text writes operate in display cells, including at the left clip boundary.
pub(super) struct Cells {
    buffer: Buffer,
    clip: Rect,
}
impl Cells {
    pub(super) fn blit(&mut self, rows: Vec<Line<'static>>, rect: Rect) {
        let previous = self.clip;
        self.clip = rect.intersection(self.buffer.area);
        for (i, row) in rows.into_iter().take(rect.height as usize).enumerate() {
            let mut x = rect.x as i32;
            for span in row.spans {
                self.write(x, rect.y as i32 + i as i32, &span.content, span.style);
                x += UnicodeWidthStr::width(span.content.as_ref()) as i32;
            }
        }
        self.clip = previous;
    }
    pub(super) fn new(width: u16, height: u16) -> Self {
        let clip = Rect::new(0, 0, width, height);
        Self {
            buffer: Buffer::empty(clip),
            clip,
        }
    }
    pub(super) fn write(&mut self, mut x: i32, y: i32, text: &str, style: Style) {
        if y < self.clip.y as i32 || y >= self.clip.bottom() as i32 {
            return;
        }
        for g in text.graphemes(true) {
            if g.chars().any(char::is_control) {
                continue;
            }
            let width = UnicodeWidthStr::width(g) as i32;
            if width == 0 {
                continue;
            }
            if x >= self.clip.x as i32 && x + width <= self.clip.right() as i32 {
                self.buffer
                    .set_stringn(x as u16, y as u16, g, width as usize, style);
            }
            x += width;
            if x >= self.clip.right() as i32 {
                break;
            }
        }
    }
    pub(super) fn into_lines(self) -> Vec<Line<'static>> {
        (0..self.buffer.area.height)
            .map(|y| {
                let mut spans = Vec::new();
                let mut x = 0;
                while x < self.buffer.area.width {
                    let cell = &self.buffer[(x, y)];
                    spans.push(Span::styled(cell.symbol().to_string(), cell.style()));
                    x += (UnicodeWidthStr::width(cell.symbol()) as u16).max(1);
                }
                Line::from(spans)
            })
            .collect()
    }
}

pub fn lines(model: &Model, layout: &GraphLayout, phase: u8) -> Vec<Line<'static>> {
    let mut cells = Cells::new(layout.canvas.width, layout.canvas.height);
    let Some(run) = &model.graph_run else {
        return cells.into_lines();
    };
    let (pan_x, pan_y) = super::graph_nav::viewport(layout, run, &model.graph_canvas);
    let th = &model.theme;
    let focused = super::graph_nav::focus_nodes(layout, run, &model.graph_canvas);
    for edge in &layout.edges {
        let state = run.tasks[layout.nodes[edge.to].task_index].state;
        let (glyph, color) = match state {
            State::Active => (if phase % 2 == 0 { "━" } else { "┄" }, th.primary),
            State::Failed | State::Attention => ("┄", th.warning),
            _ => ("─", th.muted),
        };
        let style = Style::default().fg(color);
        for segment in edge.points.windows(2) {
            let ((x1, y1), (x2, y2)) = (segment[0], segment[1]);
            // Bound iteration to visible cells, even for very long edges.
            if y1 == y2 {
                let left = (x1.min(x2) as i32 - pan_x).max(0);
                let right = (x1.max(x2) as i32 - pan_x).min(cells.clip.right() as i32 - 1);
                for x in left..=right {
                    cells.write(x, y1 as i32 - pan_y, glyph, style);
                }
            } else {
                let top = (y1.min(y2) as i32 - pan_y).max(0);
                let bottom = (y1.max(y2) as i32 - pan_y).min(cells.clip.bottom() as i32 - 1);
                for y in top..=bottom {
                    cells.write(x1 as i32 - pan_x, y, "│", style);
                }
            }
        }
        for corner in edge.points.windows(3) {
            let (before, (x, y), after) = (corner[0], corner[1], corner[2]);
            let left = before.0 < x || after.0 < x;
            let right = before.0 > x || after.0 > x;
            let up = before.1 < y || after.1 < y;
            let down = before.1 > y || after.1 > y;
            let glyph = match (left, right, up, down) {
                (true, false, false, true) => "┐",
                (false, true, true, false) => "└",
                (true, false, true, false) => "┘",
                (false, true, false, true) => "┌",
                _ => continue,
            };
            cells.write(x as i32 - pan_x, y as i32 - pan_y, glyph, style);
        }
        if let Some(&(x, y)) = edge.points.last() {
            cells.write(
                x as i32 - pan_x,
                y as i32 - pan_y,
                if state == State::Attention || state == State::Failed {
                    "!"
                } else {
                    "▶"
                },
                style,
            );
        }
    }
    let mut phases = std::collections::BTreeMap::<_, std::collections::BTreeSet<_>>::new();
    for node in &layout.nodes {
        let phase = &run.tasks[node.task_index].phase;
        if !phase.is_empty() {
            phases.entry(node.depth).or_default().insert(phase.as_str());
        }
    }
    let mut lanes = std::collections::BTreeSet::new();
    for node in &layout.nodes {
        let task = &run.tasks[node.task_index];
        if lanes.insert(node.depth) {
            let label = phases
                .get(&node.depth)
                .map(|names| names.iter().copied().collect::<Vec<_>>().join(" / "))
                .unwrap_or_else(|| format!("Stage {}", node.depth + 1));
            cells.write(
                node.rect.x as i32 - pan_x,
                0,
                &ui::clip_ellipsis(
                    &super::graph_inspector::public_text(&label),
                    node.rect.width,
                ),
                Style::default().fg(th.muted),
            );
        }
        let selected = run.selected_node_id.as_deref() == Some(&node.id)
            || model.graph_canvas.selected_group.as_deref() == Some(&node.id);
        let dimmed =
            model.graph_canvas.view_mode == GraphViewMode::Focus && !focused.contains(&node.id);
        let color = if dimmed {
            th.muted
        } else if selected || task.state == State::Active {
            th.primary
        } else if task.state == State::Failed {
            th.error
        } else if task.state == State::Attention {
            th.warning
        } else {
            th.border
        };
        let style =
            Style::default()
                .fg(color)
                .add_modifier(if selected || task.state == State::Active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
        let (x, y) = (node.rect.x as i32 - pan_x, node.rect.y as i32 - pan_y);
        let (w, h) = (node.rect.width as usize, node.rect.height as i32);
        let heavy = task.state == State::Active;
        let (tl, tr, bl, br, horizontal, vertical) = if heavy {
            ("┏", "┓", "┗", "┛", "━", "┃")
        } else {
            ("┌", "┐", "└", "┘", "─", "│")
        };
        // Paint the entire card after connectors. No connector survives in a label.
        for row in 1..h - 1 {
            cells.write(
                x,
                y + row,
                &format!("{vertical}{}{vertical}", " ".repeat(w - 2)),
                style,
            );
        }
        cells.write(
            x,
            y,
            &format!("{tl}{}{tr}", horizontal.repeat(w - 2)),
            style,
        );
        cells.write(
            x,
            y + h - 1,
            &format!("{bl}{}{br}", horizontal.repeat(w - 2)),
            style,
        );
        if selected {
            cells.write(x + 2, y, " SELECTED ", style);
        }
        let label = if node.members.len() > 1 {
            format!("✓ {} ×{}", task.role, node.members.len())
        } else {
            format!("{} {}", task.state.glyph(), task.display_title())
        };
        let text_style = Style::default()
            .fg(if dimmed { th.muted } else { th.text })
            .add_modifier(if selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        cells.write(
            x + 1,
            y + 1,
            &ui::clip_ellipsis(
                &super::graph_inspector::public_text(&label),
                node.rect.width - 2,
            ),
            style,
        );
        let activity = if node.members.len() > 1 {
            "completed · enter expand".to_string()
        } else if task.status == "cancelled" {
            "cancelled".to_string()
        } else if task.state == State::Attention {
            format!(
                "blocked · {}",
                task.error.as_deref().unwrap_or(&task.artifact)
            )
        } else if task.status == "ready" {
            "ready".to_string()
        } else {
            task.error.as_deref().unwrap_or(&task.artifact).to_string()
        };
        cells.write(
            x + 1,
            y + 2,
            &ui::clip_ellipsis(
                &super::graph_inspector::public_text(&activity),
                node.rect.width - 2,
            ),
            text_style,
        );
        if h > 4 && node.members.len() == 1 {
            cells.write(
                x + 1,
                y + 3,
                &ui::clip_ellipsis(
                    &super::graph_inspector::public_text(task.state_label()),
                    node.rect.width - 2,
                ),
                Style::default().fg(th.muted),
            );
        }
    }
    cells.into_lines()
}

#[cfg(test)]
mod tests {
    use super::super::graph_layout::layout_graph;
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
        ui,
    };

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, true),
            width,
            36,
            false,
        );
        model.graph_run = Some(fixtures::blueprint_graph());
        model.graph_canvas.follow_live = false;
        model
    }

    #[test]
    fn graph_canvas_static_monochrome_states_folds_and_selection() {
        let mut model = model(240);
        let run = model.graph_run.as_mut().unwrap();
        run.selected_node_id = Some("blocked".into());
        let layout = layout_graph(run, &model.graph_canvas, 240, 34);
        let rows = lines(&model, &layout, 0);
        let text = rows
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for expected in [
            "◉ writer",
            "× failure",
            "! blocked",
            "✓ researcher ×3",
            "SELECTED",
            "○ review",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        assert_eq!(rows, lines(&model, &layout, 0));
        assert!(!text.contains("writer─"));
    }

    #[test]
    fn graph_canvas_unicode_and_extreme_pan_never_overflow() {
        for width in [0, 1, 20, 40, 49, 50, 71, 72, 80, 119, 120] {
            let mut model = model(width);
            model.graph_run.as_mut().unwrap().tasks[0].artifact = "界e\u{301}🦀".repeat(90);
            model.graph_canvas.pan_x = i32::MAX;
            let layout = layout_graph(
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
                width,
                34,
            );
            let rows = lines(&model, &layout, 3);
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|row| ui::run_width(&row.spans) <= width));
            assert!(rows.len() <= 34);
        }
    }

    #[test]
    fn graph_canvas_labels_known_phases_and_distinguishes_cancelled() {
        let mut model = model(240);
        let run = model.graph_run.as_mut().unwrap();
        run.tasks[5].phase = "implement".into();
        run.tasks[7].status = "cancelled".into();
        run.tasks[7].artifact = "cancelled".into();
        let layout = layout_graph(run, &model.graph_canvas, 240, 34);
        let text = lines(&model, &layout, 0)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("implement"));
        assert!(text.contains("cancelled"));
    }
}
