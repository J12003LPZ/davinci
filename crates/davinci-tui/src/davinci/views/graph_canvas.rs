//! Bounded cell drawing, converted back to the normal sheet line contract.
use super::graph_inspector::public_text;
use super::graph_layout::GraphLayout;
use crate::davinci::{
    model::{GraphBucket, GraphFilter, GraphTask, GraphViewMode, Model},
    theme::{State, Theme},
    ui,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::BTreeMap;
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

/// The colour a bucket speaks in, on cards, tabs and the details panel.
pub fn bucket_color(theme: &Theme, bucket: GraphBucket, task: Option<&GraphTask>) -> Color {
    match bucket {
        GraphBucket::Working => theme.primary,
        GraphBucket::Done => theme.success,
        GraphBucket::Waiting => theme.secondary,
        GraphBucket::Inactive => theme.muted,
        GraphBucket::Attention => {
            if task.is_some_and(|t| t.state == State::Attention) {
                theme.warning
            } else {
                theme.error
            }
        }
    }
}

/// `Working`, `Done`, `Waiting`, `Inactive`, or the specific trouble.
pub fn status_label(bucket: GraphBucket, task: &GraphTask) -> &'static str {
    match task.status.as_str() {
        "cancelled" => "Cancelled",
        "ready" if bucket == GraphBucket::Waiting => "Ready",
        _ => match (bucket, task.state) {
            (GraphBucket::Attention, State::Attention) => "Blocked",
            (GraphBucket::Attention, _) => "Failed",
            (bucket, _) => bucket.label(),
        },
    }
}

/// `writer (t4)`: the role names the agent; the id says which one.
pub fn agent_name(task: &GraphTask) -> String {
    if task.role.is_empty() || task.role == task.id {
        task.id.clone()
    } else {
        format!("{} ({})", task.role, task.id)
    }
}

/// What the agent is doing or produced, in one line.
pub fn progress_text(task: &GraphTask) -> String {
    if task.status == "cancelled" {
        "cancelled".into()
    } else if task.state == State::Attention {
        format!(
            "blocked · {}",
            task.error.as_deref().unwrap_or(&task.artifact)
        )
    } else if task.status == "ready" && task.artifact.is_empty() {
        "ready".into()
    } else {
        task.error.as_deref().unwrap_or(&task.artifact).to_string()
    }
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

fn junction(mask: u8) -> &'static str {
    match mask {
        m if m == UP | DOWN | LEFT | RIGHT => "┼",
        m if m == UP | DOWN | RIGHT => "├",
        m if m == UP | DOWN | LEFT => "┤",
        m if m == LEFT | RIGHT | DOWN => "┬",
        m if m == LEFT | RIGHT | UP => "┴",
        m if m == DOWN | RIGHT => "┌",
        m if m == DOWN | LEFT => "┐",
        m if m == UP | RIGHT => "└",
        m if m == UP | LEFT => "┘",
        m if m & (LEFT | RIGHT) != 0 && m & (UP | DOWN) == 0 => "─",
        _ => "│",
    }
}

#[derive(Default, Clone, Copy)]
struct Wire {
    mask: u8,
    color: Option<Color>,
    animated: bool,
}

/// Connectors are drawn from direction masks, so two routes meeting in one
/// cell render as the junction they form. Only visible cells are touched.
fn draw_edges(cells: &mut Cells, model: &Model, layout: &GraphLayout, pan: (i32, i32), phase: u8) {
    let run = model.graph_run.as_ref().unwrap();
    let th = &model.theme;
    let (width, height) = (cells.clip.width as i32, cells.clip.height as i32);
    let mut wires = BTreeMap::<(i32, i32), Wire>::new();
    let mut heads = Vec::new();
    for edge in &layout.edges {
        let target = &run.tasks[layout.nodes[edge.to].task_index];
        let color = match target.state {
            State::Active => th.primary,
            State::Failed => th.error,
            State::Attention => th.warning,
            _ => th.border,
        };
        let animated = target.state == State::Active;
        let mut mark = |x: i32, y: i32, bits: u8| {
            let (x, y) = (x - pan.0, y - pan.1);
            if (0..width).contains(&x) && (0..height).contains(&y) {
                let wire = wires.entry((x, y)).or_default();
                wire.mask |= bits;
                wire.color = Some(color);
                wire.animated |= animated;
            }
        };
        for segment in edge.points.windows(2) {
            let ((x1, y1), (x2, y2)) = (segment[0], segment[1]);
            let (x1, y1, x2, y2) = (x1 as i32, y1 as i32, x2 as i32, y2 as i32);
            if y1 == y2 {
                let (lo, hi) = (x1.min(x2), x1.max(x2));
                // Bound iteration to visible cells, even for very long edges.
                let from = lo.max(pan.0 - 1);
                let to = hi.min(pan.0 + width);
                for x in from..=to {
                    let bits = if x > lo { LEFT } else { 0 } | if x < hi { RIGHT } else { 0 };
                    mark(x, y1, bits);
                }
            } else {
                let (lo, hi) = (y1.min(y2), y1.max(y2));
                let from = lo.max(pan.1 - 1);
                let to = hi.min(pan.1 + height);
                for y in from..=to {
                    let bits = if y > lo { UP } else { 0 } | if y < hi { DOWN } else { 0 };
                    mark(x1, y, bits);
                }
            }
        }
        if let Some(&(x, y)) = edge.points.last() {
            heads.push((x as i32 - pan.0, y as i32 - pan.1, edge.head, color));
        }
    }
    for ((x, y), wire) in wires {
        let mut glyph = junction(wire.mask);
        if wire.animated && phase % 2 == 1 {
            glyph = match glyph {
                "─" => "╌",
                "│" => "╎",
                other => other,
            };
        }
        let style = Style::default().fg(wire.color.unwrap_or(th.border));
        cells.write(x, y, glyph, style);
    }
    for (x, y, head, color) in heads {
        cells.write(x, y, head, Style::default().fg(color));
    }
}

struct CardInk {
    frame: Color,
    heading: Color,
    status: Color,
    body: Color,
    heavy: bool,
    selected: bool,
}

fn card_ink(
    th: &Theme,
    bucket: GraphBucket,
    task: &GraphTask,
    selected: bool,
    dimmed: bool,
) -> CardInk {
    let tone = bucket_color(th, bucket, Some(task));
    if dimmed {
        return CardInk {
            frame: th.border,
            heading: th.muted,
            status: th.muted,
            body: th.muted,
            heavy: false,
            selected,
        };
    }
    CardInk {
        frame: if selected {
            th.primary
        } else if bucket == GraphBucket::Inactive {
            th.border
        } else {
            tone
        },
        heading: if bucket == GraphBucket::Inactive {
            th.text
        } else {
            tone
        },
        status: tone,
        body: th.text,
        heavy: task.state == State::Active,
        selected,
    }
}

pub fn lines(model: &Model, layout: &GraphLayout, phase: u8) -> Vec<Line<'static>> {
    let mut cells = Cells::new(layout.canvas.width, layout.canvas.height);
    let Some(run) = &model.graph_run else {
        return cells.into_lines();
    };
    let pan = super::graph_nav::viewport(layout, run, &model.graph_canvas);
    let th = &model.theme;
    draw_edges(&mut cells, model, layout, pan, phase);
    let focused = super::graph_nav::focus_nodes(layout, run, &model.graph_canvas);
    let filter = model.graph_canvas.filter;
    // One classification pass per frame, not one run scan per card.
    let buckets = run.buckets();
    let index: std::collections::HashMap<&str, usize> = run
        .tasks
        .iter()
        .enumerate()
        .map(|(i, task)| (task.id.as_str(), i))
        .collect();
    for node in &layout.nodes {
        let task = &run.tasks[node.task_index];
        let (x, y) = (node.rect.x as i32 - pan.0, node.rect.y as i32 - pan.1);
        let (w, h) = (node.rect.width as usize, node.rect.height as i32);
        if w < 4 || h < 3 || y + h <= 0 || y >= cells.clip.height as i32 {
            continue;
        }
        let selected = run.selected_node_id.as_deref() == Some(&node.id)
            || model.graph_canvas.selected_group.as_deref() == Some(&node.id);
        let filtered_out = filter != GraphFilter::All
            && !node
                .members
                .iter()
                .filter_map(|id| index.get(id.as_str()))
                .any(|&member| filter.admits(buckets[member]));
        let bucket = buckets[node.task_index];
        let dimmed = filtered_out
            || (model.graph_canvas.view_mode == GraphViewMode::Focus
                && !focused.contains(&node.id));
        let ink = card_ink(th, bucket, task, selected, dimmed);
        let frame = Style::default()
            .fg(ink.frame)
            .add_modifier(if ink.selected || ink.heavy {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let (tl, tr, bl, br, horizontal, vertical) = if ink.selected {
            ("╔", "╗", "╚", "╝", "═", "║")
        } else if ink.heavy {
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
                frame,
            );
        }
        cells.write(
            x,
            y,
            &format!("{tl}{}{tr}", horizontal.repeat(w - 2)),
            frame,
        );
        cells.write(
            x,
            y + h - 1,
            &format!("{bl}{}{br}", horizontal.repeat(w - 2)),
            frame,
        );
        let inner = (w as u16).saturating_sub(4);
        let bold = |color: Color| Style::default().fg(color).add_modifier(Modifier::BOLD);
        let mut rows: Vec<(String, Style)> = Vec::new();
        if node.members.len() > 1 {
            rows.push((
                format!("✓ {} ×{}", task.role, node.members.len()),
                bold(ink.heading),
            ));
            rows.push((
                "completed · Enter expand".into(),
                Style::default().fg(ink.body),
            ));
            rows.push(("Done".into(), Style::default().fg(ink.status)));
        } else {
            rows.push((
                format!("{} {}", task.state.glyph(), agent_name(task)),
                bold(ink.heading),
            ));
            if h >= 6 && !task.title.trim().is_empty() {
                rows.push((task.title.clone(), Style::default().fg(ink.body)));
            }
            let status_style = if bucket == GraphBucket::Working && !dimmed {
                bold(ink.status)
            } else {
                Style::default().fg(ink.status)
            };
            // Progress wraps into whatever rows the card has left, so a card
            // with no task line shows more of it instead of an empty row.
            let room = (h - 2).max(1) as usize;
            let free = room.saturating_sub(1).saturating_sub(rows.len());
            let progress = public_text(&progress_text(task));
            if free > 0 && !progress.is_empty() {
                let mut wrapped = ui::wrap(&progress, inner.max(1));
                if wrapped.len() > free {
                    wrapped.truncate(free);
                    let last = wrapped.last_mut().unwrap();
                    *last = ui::clip_ellipsis(&format!("{last}…"), inner);
                }
                for line in wrapped {
                    rows.push((line, Style::default().fg(ink.body)));
                }
            }
            // The status follows the content directly; it is never cut.
            rows.truncate(room - 1);
            rows.push((status_label(bucket, task).into(), status_style));
        }
        for (i, (text, style)) in rows.into_iter().take((h - 2).max(0) as usize).enumerate() {
            cells.write(
                x + 2,
                y + 1 + i as i32,
                &ui::clip_ellipsis(&public_text(&text), inner),
                style,
            );
        }
    }
    // Say when cards continue beyond the visible canvas.
    let marker = Style::default().fg(th.muted);
    let width = cells.clip.width as i32;
    if pan.1 > 0 {
        let text = " ▲ more above ";
        cells.write(width - text.chars().count() as i32, 0, text, marker);
    }
    if layout.content_height as i32 > pan.1 + cells.clip.height as i32 {
        let text = " ▼ more below ";
        cells.write(
            width - text.chars().count() as i32,
            cells.clip.height as i32 - 1,
            text,
            marker,
        );
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

    fn text(rows: &[Line<'_>]) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn graph_canvas_static_monochrome_states_folds_and_selection() {
        let mut model = model(240);
        let run = model.graph_run.as_mut().unwrap();
        run.selected_node_id = Some("blocked".into());
        let layout = layout_graph(run, &model.graph_canvas, 240, 34);
        let rows = lines(&model, &layout, 0);
        let drawn = text(&rows);
        for expected in [
            "◉ writer",
            "× reviewer (failure)",
            "! verifier (blocked)",
            "✓ researcher ×3",
            "○ reviewer (review)",
            "Working",
            "Failed",
            "Blocked",
            "Waiting",
            "╔",
        ] {
            assert!(drawn.contains(expected), "missing {expected}: {drawn}");
        }
        assert_eq!(rows, lines(&model, &layout, 0));
        assert!(!drawn.contains("writer─"));
    }

    #[test]
    fn graph_canvas_draws_arrows_and_junctions_between_cards() {
        let model = model(240);
        let layout = layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            240,
            34,
        );
        let drawn = text(&lines(&model, &layout, 0));
        assert!(drawn.contains('→'), "{drawn}");
        assert!(drawn.contains('↓'), "{drawn}");
        assert!(
            drawn.chars().any(|c| "┴┬├┤┼".contains(c)),
            "routes that meet must render a junction: {drawn}"
        );
    }

    #[test]
    fn graph_canvas_filter_dims_other_agents_without_moving_them() {
        let mut model = model(240);
        let layout = layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            240,
            34,
        );
        let all = lines(&model, &layout, 0);
        model.graph_canvas.filter = GraphFilter::Only(GraphBucket::Working);
        let working = lines(&model, &layout, 0);
        assert_eq!(text(&all), text(&working), "filtering never hides a card");
        // Cells render one span per grapheme: find the needle's first cell.
        let color_of = |rows: &[Line<'_>], needle: &str| {
            rows.iter().find_map(|row| {
                let drawn = row.to_string();
                let at = drawn.find(needle)?;
                row.spans[drawn[..at].chars().count()].style.fg
            })
        };
        let th = &model.theme;
        assert_eq!(color_of(&working, "Done"), Some(th.muted));
        assert_eq!(color_of(&working, "Working"), Some(th.primary));
    }

    #[test]
    fn following_view_starts_on_a_card_row_and_marks_hidden_cards() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 90, 40, false);
        let run = fixtures::command_center_graph();
        model.graph_canvas.node_order = run.tasks.iter().map(|t| t.id.clone()).collect();
        model.graph_run = Some(run);
        model.graph_canvas.follow_live = true;
        for height in [14u16, 18, 22] {
            let layout = layout_graph(
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
                90,
                height + 10,
            );
            let rows = lines(&model, &layout, 0);
            let first = rows
                .iter()
                .map(Line::to_string)
                .find(|row| !row.trim().is_empty())
                .unwrap();
            assert!(
                first.trim_start().starts_with(['┌', '╔', '┏']),
                "the view opens on a card's top border: {first:?}"
            );
            let drawn = text(&rows);
            let hidden_below = layout.content_height > layout.canvas.height
                && super::super::graph_nav::viewport(
                    &layout,
                    model.graph_run.as_ref().unwrap(),
                    &model.graph_canvas,
                )
                .1 + (layout.canvas.height as i32)
                    < layout.content_height as i32;
            assert_eq!(drawn.contains("▼ more below"), hidden_below, "{drawn}");
            // The running writer stays fully visible.
            assert!(
                drawn.contains("◉ writer (t4)") && drawn.contains("╚"),
                "{drawn}"
            );
        }
    }

    #[test]
    fn graph_canvas_unicode_and_extreme_pan_never_overflow() {
        for width in [0, 1, 20, 40, 49, 50, 71, 72, 80, 119, 120] {
            let mut model = model(width);
            model.graph_run.as_mut().unwrap().tasks[0].artifact = "界e\u{301}🦀".repeat(90);
            model.graph_canvas.pan_x = i32::MAX;
            model.graph_canvas.pan_y = i32::MAX;
            let layout = layout_graph(
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
                width,
                34,
            );
            let rows = lines(&model, &layout, 3);
            assert!(rows.iter().all(|row| ui::run_width(&row.spans) <= width));
            assert!(rows.len() <= 34);
        }
    }

    #[test]
    fn graph_canvas_distinguishes_cancelled_and_animates_only_live_routes() {
        let mut model = model(240);
        let run = model.graph_run.as_mut().unwrap();
        run.tasks[7].status = "cancelled".into();
        run.tasks[7].artifact = "cancelled".into();
        let layout = layout_graph(run, &model.graph_canvas, 240, 34);
        let still = text(&lines(&model, &layout, 0));
        assert!(still.contains("Cancelled"), "{still}");
        let moving = text(&lines(&model, &layout, 1));
        assert_ne!(still, moving, "routes into a working agent animate");
        assert_eq!(
            still.replace(['╌', '╎'], ""),
            still,
            "a still frame has no dashes"
        );
    }
}
