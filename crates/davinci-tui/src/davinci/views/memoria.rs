//! `1f` — the session picker, an overlay over a dimmed transcript.
//!
//! Every panel states its own exits in its footer (design.md §9).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/memoria.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, meter, pad, run_width, span, span_on, span_strong, spread, spread_on, surface_rule,
    Surface, MEASURE,
};

/// The sessions list (`1f`).
pub fn sessions(model: &Model, height: usize) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let width = model.width;
    let inner = width.saturating_sub(inset * 2).saturating_sub(4);
    let selected = model.selection(model.sessions.len());

    // A store with more sessions than the window holds is folded around the
    // selection, counted either side — the list never pushes the title, the
    // facts row or the keys off the screen. Five rows of the height are the
    // surface's own: two borders, the rule, the facts and the keys.
    let avail = height.saturating_sub(5).max(3);
    let total = model.sessions.len();
    let (start, end) = if total <= avail {
        (0, total)
    } else {
        let window = avail.saturating_sub(2).max(1);
        let anchor = selected.unwrap_or(0);
        let start = anchor
            .saturating_sub(window / 2)
            .min(total.saturating_sub(window));
        (start, start + window)
    };

    let mut body: Vec<Vec<Span<'static>>> = Vec::new();
    if start > 0 {
        body.push(vec![span(format!("… {start} above"), th.border)]);
    }
    body.extend(
        model
            .sessions
            .iter()
            .enumerate()
            .skip(start)
            .take(end - start)
            .map(|(index, session)| {
                picker_row(
                    th,
                    inner,
                    &session.name,
                    &session.age,
                    Some(index) == selected,
                )
            }),
    );
    if end < total {
        body.push(vec![span(format!("… {} below", total - end), th.border)]);
    }

    if body.is_empty() {
        // A first session in a new folder has nothing to resume; say so
        // rather than opening an empty list.
        body.push(vec![span("no earlier sessions in this folder", th.muted)]);
    }

    // The footer describes the highlighted session — its turns, its weight,
    // where it came from (`1f`) — falling back to the session in hand.
    let facts = selected
        .and_then(|index| model.sessions.get(index))
        .filter(|session| !session.turns.is_empty());
    let (turns, tokens, lineage) = match facts {
        Some(session) => (
            session.turns.clone(),
            session.tokens.clone(),
            session.lineage.clone(),
        ),
        None => (
            model.transcript.len().to_string(),
            super::chrome::thousands(model.context.0),
            "this session".to_string(),
        ),
    };
    body.push(surface_rule(width.saturating_sub(inset * 2), th));
    body.push(vec![
        span(format!("{turns} turns"), th.muted),
        span(" │ ", th.border),
        span(format!("{tokens} tokens"), th.muted),
        span(" │ ", th.border),
        span(lineage, th.muted),
    ]);
    body.push(vec![
        span("enter resume", th.border),
        span(" │ ", th.border),
        span("d delete", th.border),
        span(" │ ", th.border),
        span("f fork", th.border),
        span(" │ ", th.border),
        span("ctrl+s close", th.border),
    ]);

    Surface::new(width, th)
        .inset(inset)
        .border(th.primary)
        .title(vec![
            span("MEMORIA", th.primary),
            span(" · ", th.border),
            span("SESSIONS", th.muted),
        ])
        .rows(body)
        .lines()
}

/// `2b` — vector recall.
///
/// Each promoted hit is two rows: score, summary and location, then a
/// proportion meter and provenance. Hits below the relevance floor are drawn
/// in the dimmest ink and counted as held back, so the retrieval stays
/// auditable (design.md §6). Beside the audit sits the projection panel:
/// decoration with a job, the query against the session cluster.
pub fn recall(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let meta = &model.recall_meta;
    let selected = model.selection(model.recall.len());

    // The query box carries its own facts inline; the index's live in the
    // header (`2b`).
    let mut rows = Surface::new(width, th)
        .border(th.secondary)
        .row(spread_row(
            width.saturating_sub(4),
            vec![
                span(format!("{} ", glyph::SEARCH), th.secondary),
                span(meta.query.clone(), th.text),
            ],
            vec![
                span(meta.metric.clone(), th.border),
                span(" · ", th.muted),
                span(meta.elapsed.clone(), th.border),
                span(" · ", th.muted),
                span(format!("k={}", meta.k), th.border),
            ],
        ))
        .lines();
    rows.push(blank());

    let held_back = model.recall.iter().filter(|hit| !hit.above_floor).count();

    for (index, hit) in model.recall.iter().enumerate() {
        let current = Some(index) == selected && hit.above_floor;
        if !hit.above_floor {
            // Below the floor: shown, not promoted — the dimmest ink, no
            // meter, so the cut is visible rather than silent.
            rows.push(spread(
                width,
                vec![
                    span(format!(" {:.2}  ", hit.score), th.border),
                    span(hit.summary.clone(), th.border),
                ],
                vec![span(hit.location.clone(), th.border)],
            ));
            continue;
        }
        let score_color = if current { th.primary } else { th.muted };
        let bar = if current {
            span_on("▌", th.primary, Some(th.surface))
        } else {
            span(" ", th.muted)
        };
        let tint = if current { Some(th.surface) } else { None };
        rows.push(spread_on(
            width,
            vec![
                bar,
                span_on(format!("{:.2}  ", hit.score), score_color, tint),
                span_on(
                    hit.summary.clone(),
                    if current { th.text } else { th.muted },
                    tint,
                ),
            ],
            vec![span_on(
                hit.location.clone(),
                if current { th.muted } else { th.border },
                tint,
            )],
            tint,
        ));
        let mut second = vec![pad(7, None)];
        second.extend(meter(hit.score, 20, th, Some(score_color)));
        second.push(span("  ", th.border));
        second.push(span(hit.provenance.clone(), th.border));
        rows.push(Line::from(second));
    }

    rows.push(blank());

    // The audit block, with the projection beside it when there is room.
    let audit = vec![
        Line::from(vec![
            span("promoted to context ", th.muted),
            span(meta.promoted.clone(), th.text),
        ]),
        Line::from(vec![
            span(format!("held back {held_back}"), th.muted),
            span(" · ", th.border),
            span(format!("below {:.2} relevance floor", meta.floor), th.muted),
        ]),
        Line::from(vec![
            span("index freshness ", th.muted),
            span_strong(format!("{} ", glyph::DONE), th.success, th),
            span(meta.freshness.clone(), th.muted),
        ]),
    ];
    if model.decoration() {
        rows.extend(beside(audit, projection(th), width));
    } else {
        rows.extend(audit);
    }
    rows
}

/// A left run and a right run inside a surface row.
fn spread_row(
    inner: u16,
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
) -> Vec<Span<'static>> {
    let gap = inner
        .saturating_sub(run_width(&left))
        .saturating_sub(run_width(&right))
        .max(1);
    let mut row = left;
    row.push(pad(gap, None));
    row.extend(right);
    row
}

/// The projection panel: the query drawn against the session cluster (`2b`).
/// The dot field is fixed; only its inks come from the theme.
fn projection(th: &Theme) -> Vec<Line<'static>> {
    const FIELD: [&str; 5] = [
        "  ·   ·      ·        ·   ·",
        "    ·  ◉ ·   ·   ·    ·  ·",
        "  ·  ∘ ∘ ·      ·  ·   ·  ·",
        "     ·   ·     ∘ ·  ·      ·",
        "  ·    ·   ·     ·   ∘  ·  ·",
    ];
    let mut surface =
        Surface::new(PROJECTION_WIDTH, th).title(vec![span("PROJECTION", th.secondary)]);
    for (index, row) in FIELD.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for ch in row.chars() {
            match ch {
                '◉' => spans.push(span("◉", th.primary)),
                // `∘` stands in for the near-cluster and query-adjacent dots,
                // drawn brighter than the field.
                '∘' if index == 2 => spans.push(span("·", th.muted)),
                '∘' => spans.push(span("·", th.secondary)),
                other => spans.push(span(other.to_string(), th.border)),
            }
        }
        surface = surface.row(spans);
    }
    surface = surface.row(vec![
        span("session cluster ", th.border),
        span("◉", th.primary),
        span(" query", th.border),
        span(" · ", th.border),
        span("18k pts", th.border),
    ]);
    surface.lines()
}

const PROJECTION_WIDTH: u16 = 38;

/// Lay the audit lines beside the projection panel, row by row.
fn beside(left: Vec<Line<'static>>, right: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let left_width = width.saturating_sub(PROJECTION_WIDTH + 2);
    let rows = left.len().max(right.len());
    (0..rows)
        .map(|index| {
            let mut spans = left.get(index).cloned().unwrap_or_else(blank).spans;
            let used = run_width(&spans);
            spans.push(pad(left_width.saturating_sub(used) + 2, None));
            if let Some(panel_row) = right.get(index) {
                spans.extend(panel_row.spans.clone());
            }
            Line::from(spans)
        })
        .collect()
}

/// A picker row: `◉`/`○`, a name, and something on the right. Shared by
/// Memoria sessions and Cogitator so the two read identically (`1f`).
pub fn picker_row(
    theme: &Theme,
    inner: u16,
    name: &str,
    right_text: &str,
    selected: bool,
) -> Vec<Span<'static>> {
    let state = if selected {
        State::Active
    } else {
        State::Queued
    };
    let tint = if selected { Some(theme.surface) } else { None };
    let name_color = if selected { theme.text } else { theme.muted };

    let mut row = vec![
        span_on(
            format!("{} ", state.glyph()),
            theme.state_color(state),
            tint,
        ),
        span_on(name.to_string(), name_color, tint),
    ];
    if selected {
        // Emphasis is what carries the selection under NO_COLOR.
        row[0] = span_strong(
            format!("{} ", state.glyph()),
            theme.state_color(state),
            theme,
        );
    }
    let right = vec![span_on(right_text.to_string(), theme.border, tint)];
    let gap = inner
        .saturating_sub(run_width(&row))
        .saturating_sub(run_width(&right))
        .max(1);
    row.push(pad(gap, tint));
    row.extend(right);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::{Overlay, SessionItem};
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_overlay(Overlay::Sessions);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_deep_store_is_folded_so_the_title_and_keys_never_scroll_off() {
        let mut m = model(100);
        m.sessions = (0..80)
            .map(|i| SessionItem {
                name: format!("session-{i}"),
                age: "2h".into(),
                path: String::new(),
                turns: String::new(),
                tokens: String::new(),
                lineage: String::new(),
            })
            .collect();
        let rows = sessions(&m, 18);
        assert!(rows.len() <= 18, "{} rows for a height of 18", rows.len());
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn[0].contains("MEMORIA"), "the title survives");
        assert!(
            drawn.iter().any(|row| row.contains("below")),
            "the fold is counted: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|row| row.contains("enter resume")),
            "the keys survive"
        );
    }

    #[test]
    fn a_folder_with_no_earlier_sessions_says_so_and_still_states_its_exits() {
        let mut m = model(100);
        m.sessions.clear();
        let rows: Vec<String> = sessions(&m, 44).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("no earlier sessions in this folder")));
        assert!(rows.iter().any(|row| row.contains("ctrl+s close")));
    }

    #[test]
    fn the_sessions_overlay_is_a_copper_panel_naming_itself() {
        let m = model(100);
        let rows = sessions(&m, 44);
        let top = text(&rows[0]);
        assert!(top.contains("╭─ MEMORIA · SESSIONS ─"), "{top}");
        assert_eq!(rows[0].spans[1].style.fg, Some(m.theme.primary));
        for row in &rows {
            assert_eq!(run_width(&row.spans), 100);
        }
    }

    #[test]
    fn the_footer_describes_the_highlighted_session() {
        let m = model(100);
        let rows: Vec<String> = sessions(&m, 44).iter().map(text).collect();
        assert!(
            rows.iter()
                .any(|row| row.contains("42 turns │ 128k tokens │ forked from provider-parity")),
            "{rows:?}"
        );

        // A row with no recorded facts falls back to the session in hand.
        let mut m = model(100);
        m.move_selection(1);
        let rows: Vec<String> = sessions(&m, 44).iter().map(text).collect();
        assert!(
            rows.iter().any(|row| row.contains("this session")),
            "{rows:?}"
        );
    }

    #[test]
    fn the_current_session_is_marked_with_a_glyph_and_a_tint() {
        let m = model(100);
        let rows = sessions(&m, 44);
        assert!(text(&rows[1]).contains("◉ "), "{:?}", text(&rows[1]));
        assert!(text(&rows[2]).contains("○ "), "{:?}", text(&rows[2]));
        assert!(rows[1]
            .spans
            .iter()
            .any(|span| span.style.bg == Some(m.theme.surface)));
    }

    #[test]
    fn every_session_row_carries_its_age() {
        let m = model(100);
        let rows: Vec<String> = sessions(&m, 44).iter().map(text).collect();
        for session in &m.sessions {
            assert!(
                rows.iter()
                    .any(|row| row.contains(&session.name) && row.contains(&session.age)),
                "{} has no age",
                session.name
            );
        }
    }

    #[test]
    fn the_footer_states_its_exits() {
        let m = model(100);
        let rows: Vec<String> = sessions(&m, 44).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("ctrl+s close")));
        assert!(rows.iter().any(|row| row.contains("enter resume")));
    }

    #[test]
    fn selection_moves_and_wraps() {
        let mut m = model(100);
        let len = m.sessions.len();
        m.move_selection(1);
        assert_eq!(m.selection(len), Some(1));
        m.move_selection(-2);
        assert_eq!(m.selection(len), Some(len - 1));
    }
}
