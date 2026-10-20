//! The bottom roster and the read-only worker view.
//!
//! The roster is one row for the main agent and one per worker: status glyph,
//! name, one line of what it is doing, elapsed time, tokens and where it
//! works. A small window collapses it to a single row. The worker view shows
//! that worker's own activity, newest at the bottom, its workspace and the
//! state of any message sent to it. It is read-only: nothing here changes a
//! worker.

use ratatui::text::Line;
use ratatui::text::Span;

use crate::davinci::model::{AgentRow, Model};
use crate::davinci::roster::{plain, Receipt, ViewScope, WorkerLineKind, MAIN_ROW};
use crate::davinci::theme::State;
use crate::davinci::ui::{
    clip_ellipsis, pad_to, run_width, span, span_strong, spread, truncate_run,
};

/// One drawn roster row and the agent id a click on it opens.
#[derive(Debug, Clone)]
pub struct RosterLine {
    pub line: Line<'static>,
    /// `None` for rows that are not clickable (summary, hint, "+N more").
    pub target: Option<String>,
}

fn glyph_color(model: &Model, state: State) -> ratatui::style::Color {
    let cc = model.theme.cc();
    match state {
        State::Done => cc.success,
        State::Active | State::Agent => cc.claude,
        State::Failed => cc.error,
        State::Attention => model.theme.warning,
        _ => cc.inactive,
    }
}

fn is_running(row: &AgentRow) -> bool {
    matches!(row.state, State::Active | State::Queued)
        && !matches!(row.status.as_str(), "completed" | "failed" | "cancelled")
}

fn main_row(model: &Model) -> AgentRow {
    let mut row = AgentRow::new(
        MAIN_ROW,
        "main",
        if model.running {
            State::Active
        } else {
            State::Done
        },
    );
    row.activity = if model.running {
        "working".into()
    } else {
        "idle".into()
    };
    row
}

/// The cells right of the name: `1:23 · 23k · ⎇ davinci/agent/x`.
fn right_cells(row: &AgentRow, width: u16) -> String {
    if row.id == MAIN_ROW {
        return String::new();
    }
    let mut parts = vec![if row.elapsed.is_empty() {
        "0:00".to_string()
    } else {
        plain(&row.elapsed)
    }];
    parts.push(super::agents::tokens_cell(row));
    if width >= 56 && !row.workspace.is_empty() {
        parts.push(plain(&row.workspace));
    }
    parts.join(" · ")
}

fn row_line(
    model: &Model,
    row: &AgentRow,
    width: u16,
    focused: bool,
    viewing: bool,
) -> Line<'static> {
    let cc = model.theme.cc();
    let right = clip_ellipsis(&right_cells(row, width), width / 2);
    let bar = if focused { "❯ " } else { "  " };
    let name = plain(&row.name);
    let mut left = vec![
        span(bar, if focused { cc.permission } else { cc.inactive }),
        span(
            format!("{} ", row.state.glyph()),
            glyph_color(model, row.state),
        ),
    ];
    left.push(if viewing {
        span_strong(name.clone(), cc.permission, &model.theme)
    } else {
        span(name.clone(), model.theme.text)
    });
    let used = run_width(&left) + run_width(&[span(right.clone(), cc.inactive)]) + 3;
    let room = width.saturating_sub(used);
    let activity = plain(&row.activity);
    if room >= 6 && !activity.is_empty() {
        left.push(span(
            format!("  {}", clip_ellipsis(&activity, room.saturating_sub(2))),
            cc.inactive,
        ));
    }
    spread(width, left, vec![span(right, cc.inactive)])
}

/// The roster as rows for a frame `height` rows tall. Empty when it is not
/// shown. Collapses to one row on a short window.
pub fn roster_lines(model: &Model, width: u16, height: u16) -> Vec<RosterLine> {
    if !model.roster_visible() || width == 0 {
        return Vec::new();
    }
    let cc = model.theme.cc();
    let mut rows: Vec<AgentRow> = vec![main_row(model)];
    rows.extend(model.roster.workers.iter().cloned());
    let viewing_id = model
        .viewing_worker()
        .map(str::to_string)
        .unwrap_or_else(|| MAIN_ROW.to_string());
    let current = rows
        .iter()
        .position(|row| row.id == viewing_id)
        .unwrap_or(0);
    let focus = model.roster.focus.map(|at| at.min(rows.len() - 1));
    let running = rows.iter().skip(1).filter(|row| is_running(row)).count();

    let budget = if height < 10 {
        1
    } else {
        (height as usize / 4).clamp(2, 8)
    };
    if budget == 1 {
        let shown = focus.unwrap_or(current);
        let row = &rows[shown];
        let hint = if focus.is_some() {
            "↑↓ choose · enter open · esc back"
        } else {
            "↓ select"
        };
        let left = vec![
            span(if focus.is_some() { "❯ " } else { "  " }, cc.permission),
            span(
                format!("{} ", row.state.glyph()),
                glyph_color(model, row.state),
            ),
            span(plain(&row.name), model.theme.text),
            span(
                format!(
                    "  {} agent{} ({running} running)",
                    rows.len() - 1,
                    if rows.len() == 2 { "" } else { "s" }
                ),
                cc.inactive,
            ),
        ];
        let line = spread(width, left, vec![span(hint, cc.inactive)]);
        return vec![RosterLine {
            line,
            target: Some(row.id.clone()),
        }];
    }

    // Keep the row in focus (or in view) inside the window.
    let anchor = focus.unwrap_or(current);
    let visible = budget.min(rows.len());
    let start = if rows.len() <= visible {
        0
    } else {
        anchor.saturating_sub(visible / 2).min(rows.len() - visible)
    };
    let mut out: Vec<RosterLine> = Vec::new();
    for (at, row) in rows.iter().enumerate().skip(start).take(visible) {
        out.push(RosterLine {
            line: row_line(model, row, width, focus == Some(at), at == current),
            target: Some(row.id.clone()),
        });
    }
    let hidden = rows.len() - visible;
    if hidden > 0 {
        out.push(RosterLine {
            line: Line::from(truncate_run(
                vec![span(format!("  +{hidden} more · ↑↓"), cc.inactive)],
                width,
            )),
            target: None,
        });
    }
    if focus.is_some() {
        out.push(RosterLine {
            line: Line::from(truncate_run(
                vec![span("  ↑↓ choose · enter open · esc back", cc.inactive)],
                width,
            )),
            target: None,
        });
    }
    out
}

fn receipt_text(receipt: &Receipt) -> String {
    let id = receipt
        .id
        .as_deref()
        .map(|id| format!(" {}", id.chars().take(8).collect::<String>()))
        .unwrap_or_default();
    let reason = receipt
        .reason
        .as_deref()
        .map(|reason| format!(" ({reason})"))
        .unwrap_or_default();
    format!(
        "to worker{id}: {}{reason} · {}",
        receipt.state, receipt.text
    )
}

/// The worker in view, `height` rows tall: header, details, activity, receipts.
pub fn worker_body(model: &Model, height: usize) -> Vec<Line<'static>> {
    let Some(id) = model.viewing_worker() else {
        return Vec::new();
    };
    let width = model.width;
    let cc = model.theme.cc();
    let row = model.viewed_row();
    let name = row
        .map(|row| plain(&row.name))
        .unwrap_or_else(|| id.chars().take(8).collect());
    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    lines.push(spread(
        width,
        vec![
            span("  Viewing: ", cc.inactive),
            span_strong(name, cc.permission, &model.theme),
        ],
        vec![span("esc back to main", cc.inactive)],
    ));
    match row {
        Some(row) => {
            let mut facts = vec![
                plain(&row.status),
                if row.elapsed.is_empty() {
                    "0:00".into()
                } else {
                    plain(&row.elapsed)
                },
                format!("{} tokens", super::agents::tokens_cell(row)),
            ];
            if row.cache_read + row.cache_write > 0 {
                facts.push(format!(
                    "cache {} read · {} written",
                    row.cache_read, row.cache_write
                ));
            }
            if row.tool_count > 0 {
                facts.push(format!("{} tool uses", row.tool_count));
            }
            lines.push(Line::from(truncate_run(
                vec![span(format!("  {}", facts.join(" · ")), cc.inactive)],
                width,
            )));
            let workspace = if row.workspace_detail.is_empty() {
                "unknown".to_string()
            } else {
                plain(&row.workspace_detail)
            };
            lines.push(Line::from(truncate_run(
                vec![span(format!("  workspace: {workspace}"), cc.inactive)],
                width,
            )));
        }
        None => lines.push(Line::from(truncate_run(
            vec![span("  not in the current roster", cc.inactive)],
            width,
        ))),
    }
    let view = model.roster.views.get(id);
    if let Some(view) = view.filter(|view| !view.changed_paths.is_empty()) {
        let shown: Vec<&str> = view
            .changed_paths
            .iter()
            .take(4)
            .map(String::as_str)
            .collect();
        let more = view.changed_paths.len().saturating_sub(shown.len());
        let tail = if more > 0 {
            format!(" +{more} more")
        } else {
            String::new()
        };
        lines.push(Line::from(truncate_run(
            vec![span(
                format!("  changed (read-only): {}{tail}", shown.join(", ")),
                cc.inactive,
            )],
            width,
        )));
    }
    lines.push(Line::default());
    // A short window keeps the title and the vital line, and gives the rest
    // of its rows to the activity.
    if height < 10 {
        lines.truncate(3);
        lines.remove(0);
    }

    // Receipts keep the last rows; activity fills the rest, newest at the bottom.
    let receipts: Vec<String> = view
        .map(|view| {
            view.receipts
                .iter()
                .rev()
                .take(3)
                .rev()
                .map(receipt_text)
                .collect()
        })
        .unwrap_or_default();
    let reserved = lines.len() + receipts.len() + usize::from(!receipts.is_empty());
    let room = height.saturating_sub(reserved);
    let mut activity: Vec<Line<'static>> = Vec::new();
    if let Some(view) = view {
        let total = view.lines.len();
        let end = total.saturating_sub(view.scroll.min(total.saturating_sub(1)));
        let begin = end.saturating_sub(room.max(1));
        if view.dropped > 0 && begin == 0 {
            activity.push(Line::from(truncate_run(
                vec![span(
                    format!("  … {} earlier entries are no longer kept", view.dropped),
                    cc.inactive,
                )],
                width,
            )));
        }
        for line in view.lines.iter().skip(begin).take(end - begin) {
            let (lead, color) = match line.kind {
                WorkerLineKind::Started => ("  ", cc.inactive),
                WorkerLineKind::Call => ("  ⎿ ", cc.inactive),
                WorkerLineKind::Finished => ("  ", cc.success),
                WorkerLineKind::Failed => ("  ", cc.error),
            };
            let text = clip_ellipsis(&plain(&line.text), width.saturating_sub(4));
            activity.push(Line::from(truncate_run(
                vec![span(format!("{lead}{text}"), color)],
                width,
            )));
        }
        if view.scroll > 0 {
            activity.push(Line::from(truncate_run(
                vec![span(
                    format!("  ↓ {} newer · scroll down", view.scroll),
                    cc.inactive,
                )],
                width,
            )));
        }
    }
    if activity.is_empty() {
        activity.push(Line::from(truncate_run(
            vec![span("  no activity recorded yet", cc.inactive)],
            width,
        )));
    }
    // Keep only the newest rows that fit.
    let skip = activity.len().saturating_sub(room.max(1));
    lines.extend(activity.into_iter().skip(skip));
    if !receipts.is_empty() {
        lines.push(Line::default());
        for text in receipts {
            lines.push(Line::from(truncate_run(
                vec![span(format!("  {text}"), cc.permission)],
                width,
            )));
        }
    }
    let mut lines = pad_to(lines, height);
    lines.truncate(height);
    lines
}

/// The one row above the composer while a worker is in view.
pub fn recipient_row(model: &Model) -> Option<Line<'static>> {
    let label = model.recipient_label()?;
    let cc = model.theme.cc();
    let spans: Vec<Span<'static>> = vec![
        span("  ", cc.inactive),
        span_strong(label, cc.permission, &model.theme),
        span(
            "  enter sends to this worker · esc returns to main",
            cc.inactive,
        ),
    ];
    Some(Line::from(truncate_run(spans, model.width)))
}

/// Keys that belong to the roster and the worker view. Returns whether the
/// key was used. Meant to run before the ordinary key handling, in the idle
/// loop and while a turn runs, so `Esc` in a worker's view returns to main and
/// never reaches the interrupt.
pub fn handle_key(model: &mut Model, key: crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    if !model.roster.enabled
        || key.kind == KeyEventKind::Release
        || model.overlay.is_some()
        || model.viewer.is_some()
        || model.history_search.is_some()
        || model.screen != crate::davinci::model::Screen::Agent
    {
        return false;
    }
    let plain_key = (key.modifiers - KeyModifiers::SHIFT).is_empty();
    if !plain_key {
        return false;
    }
    let in_worker = matches!(model.roster.scope, ViewScope::Worker(_));
    if !in_worker && !model.roster_visible() {
        return false;
    }
    // Roster keyboard focus owns the arrows, enter and esc.
    if model.roster.focus.is_some() {
        match key.code {
            KeyCode::Up => {
                if model.roster.focus == Some(0) {
                    model.roster.focus = None;
                    model.dirty = true;
                } else {
                    model.roster_focus_move(-1);
                }
            }
            KeyCode::Down => model.roster_focus_move(1),
            KeyCode::Enter => model.roster_focus_activate(),
            KeyCode::Esc => {
                model.roster.focus = None;
                model.dirty = true;
            }
            _ => {
                model.roster.focus = None;
                return false;
            }
        }
        return true;
    }
    if in_worker {
        return match key.code {
            KeyCode::Esc if model.suggestions.is_none() => {
                model.focus_main();
                true
            }
            KeyCode::Enter if key.modifiers.is_empty() => model.submit_to_worker(),
            KeyCode::PageUp => {
                model.scroll_worker(10);
                true
            }
            KeyCode::PageDown => {
                model.scroll_worker(-10);
                true
            }
            KeyCode::Up if model.composer.is_empty() => {
                model.scroll_worker(1);
                true
            }
            KeyCode::Down if model.composer.is_empty() && model.suggestions.is_none() => {
                let at_bottom = model
                    .viewing_worker()
                    .and_then(|id| model.roster.views.get(id))
                    .is_none_or(|view| view.scroll == 0);
                if at_bottom {
                    model.roster_focus_enter();
                } else {
                    model.scroll_worker(-1);
                }
                true
            }
            _ => false,
        };
    }
    // Main scope: Down from an empty composer reaches the roster.
    if key.code == KeyCode::Down
        && model.composer.is_empty()
        && model.suggestions.is_none()
        && !model.shortcuts_open
    {
        model.roster_focus_enter();
        return true;
    }
    false
}

/// Resolve a click on roster row `index` of the last drawn frame.
pub fn click(model: &mut Model, index: usize) -> bool {
    let id = model.roster.hit.borrow().get(index).cloned();
    match id {
        Some(id) => {
            model.focus_agent(&id);
            true
        }
        None => false,
    }
}
