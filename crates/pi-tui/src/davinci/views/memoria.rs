//! `1f` — the session picker, an overlay over a dimmed transcript.
//!
//! Every panel states its own exits in its footer (design.md §9).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/memoria.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, meter, pad, run_width, span, span_on, span_strong, spread, Surface, MEASURE,
};

/// The sessions list (`1f`).
pub fn sessions(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let width = model.width;
    let inner = width.saturating_sub(inset).saturating_sub(4);
    let selected = model.selection(model.sessions.len());

    let mut body: Vec<Vec<Span<'static>>> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(index, session)| {
            picker_row(
                th,
                inner,
                &session.name,
                &session.age,
                Some(index) == selected,
            )
        })
        .collect();

    if body.is_empty() {
        // A first session in a new folder has nothing to resume; say so
        // rather than opening an empty list.
        body.push(vec![span("no earlier sessions in this folder", th.muted)]);
    }

    body.push(Vec::new());
    body.push(vec![
        span(format!("{} turns", model.transcript.len()), th.muted),
        span(" │ ", th.border),
        span(
            format!("{} tokens", super::chrome::thousands(model.context.0)),
            th.muted,
        ),
        span(" │ ", th.border),
        span("this session", th.muted),
    ]);
    body.push(vec![
        span("enter resume", th.border),
        span(" · ", th.border),
        span("d delete", th.border),
        span(" · ", th.border),
        span("f fork", th.border),
        span(" · ", th.border),
        span("ctrl+s close", th.border),
    ]);

    Surface::new(width, th)
        .inset(inset)
        .title(vec![
            span("MEMORIA", th.primary),
            span(" · ", th.border),
            span("SESSIONS", th.muted),
        ])
        .right(vec![span("ctrl+s", th.border)])
        .rows(body)
        .lines()
}

/// `2b` — vector recall.
///
/// Each hit is two rows: score, summary and location, then a proportion meter
/// and provenance. Hits below the relevance floor are shown as held back, with
/// the count, so the retrieval stays auditable (design.md §6).
pub fn recall(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let meta = &model.recall_meta;
    let selected = model.selection(model.recall.len());

    let mut rows = Surface::new(width, th)
        .border(th.secondary)
        .title(vec![
            span("MEMORIA", th.secondary),
            span(" · ", th.border),
            span("RECALL", th.muted),
        ])
        .right(vec![
            span(meta.metric.clone(), th.border),
            span(" · ", th.muted),
            span(meta.elapsed.clone(), th.border),
            span(" · ", th.muted),
            span(format!("k={}", meta.k), th.border),
        ])
        .row(vec![
            span(format!("{} ", glyph::SEARCH), th.secondary),
            span(meta.query.clone(), th.text),
        ])
        .lines();

    rows.push(Line::from(vec![
        span(format!("{} vectors", meta.vectors), th.muted),
        span(" │ ", th.border),
        span(format!("{} shards", meta.shards), th.muted),
        span(" │ ", th.border),
        span(meta.embedding.clone(), th.muted),
    ]));
    rows.push(blank());

    let held_back = model.recall.iter().filter(|hit| !hit.above_floor).count();

    for (index, hit) in model
        .recall
        .iter()
        .filter(|hit| hit.above_floor)
        .enumerate()
    {
        let current = Some(index) == selected;
        let score_color = if current { th.primary } else { th.secondary };
        rows.push(spread(
            width,
            vec![
                span_strong(format!("{:.2}  ", hit.score), score_color, th),
                span(
                    hit.summary.clone(),
                    if current { th.text } else { th.muted },
                ),
            ],
            vec![span(hit.location.clone(), th.secondary)],
        ));
        let mut second = vec![pad(6, None)];
        second.extend(meter(hit.score, 20, th, Some(score_color)));
        second.push(span("  ", th.border));
        second.push(span(hit.provenance.clone(), th.border));
        rows.push(Line::from(second));
    }

    rows.push(blank());
    rows.push(Line::from(vec![
        span("promoted to context ", th.muted),
        span(meta.promoted.clone(), th.text),
    ]));
    rows.push(Line::from(vec![
        span_strong(format!("{} ", glyph::ATTENTION), th.warning, th),
        span(format!("held back {held_back}"), th.muted),
        span(" · ", th.border),
        span(format!("below {:.2} relevance floor", meta.floor), th.muted),
    ]));
    rows.push(Line::from(vec![
        span("index freshness ", th.muted),
        span_strong(format!("{} ", glyph::DONE), th.success, th),
        span(meta.freshness.clone(), th.muted),
    ]));
    rows
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
    use crate::davinci::model::Overlay;
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
    fn a_folder_with_no_earlier_sessions_says_so_and_still_states_its_exits() {
        let mut m = model(100);
        m.sessions.clear();
        let rows: Vec<String> = sessions(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("no earlier sessions in this folder")));
        assert!(rows.iter().any(|row| row.contains("ctrl+s close")));
    }

    #[test]
    fn the_sessions_overlay_names_itself_and_its_key() {
        let m = model(100);
        let rows = sessions(&m);
        let top = text(&rows[0]);
        assert!(top.contains("╭─ MEMORIA · SESSIONS ─"), "{top}");
        assert!(top.ends_with("─ ctrl+s ─╮"), "{top}");
        for row in &rows {
            assert_eq!(run_width(&row.spans), 100);
        }
    }

    #[test]
    fn the_current_session_is_marked_with_a_glyph_and_a_tint() {
        let m = model(100);
        let rows = sessions(&m);
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
        let rows: Vec<String> = sessions(&m).iter().map(text).collect();
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
        let rows: Vec<String> = sessions(&m).iter().map(text).collect();
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
