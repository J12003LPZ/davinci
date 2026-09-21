//! Session selection and read-only memory recall share the section typography,
//! but only session selection has a cursor. Recall displays measured evidence.

use super::sheet::hint;
use crate::davinci::ui::{self, section_detail, section_heading, section_row, span, Surface};
use crate::davinci::{model::Model, theme::Theme};
use ratatui::text::{Line, Span};

pub fn sessions(model: &Model, height: usize) -> Vec<Line<'static>> {
    ui::window_section(
        session_lines(model),
        height,
        1,
        model.overlay_offset,
        &model.theme,
    )
}

pub(crate) fn session_lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let inner = model.width.saturating_sub(inset * 2).saturating_sub(4);
    let selected = model.selection(model.sessions.len());
    let mut body = Vec::new();
    if model.sessions.is_empty() {
        body.extend(section_detail(
            inner,
            th,
            "No earlier sessions in this folder.",
        ));
    }
    for (index, session) in model.sessions.iter().enumerate() {
        let focused = Some(index) == selected;
        body.push(section_row(inner, th, focused, &session.name, &session.age));
        if focused {
            body.extend(section_detail(inner, th, &session.name));
            let mut facts = Vec::new();
            if !session.turns.is_empty() {
                facts.push(format!("{} messages", session.turns));
            }
            if !session.tokens.is_empty() {
                facts.push(format!("{} tokens", session.tokens));
            }
            if !session.age.is_empty() {
                facts.push(format!("updated {}", session.age));
            }
            body.extend(section_detail(inner, th, &facts.join(" · ")));
            body.extend(section_detail(inner, th, &session.lineage));
            body.extend(section_detail(inner, th, &session.path));
        }
    }
    body.push(ui::hint_row(
        inner,
        &[
            hint(th, "↑↓ move"),
            hint(
                th,
                if model.overlay_offset.is_some() {
                    "enter back"
                } else {
                    "enter resume"
                },
            ),
            hint(th, "pgup/pgdn read"),
        ],
        Some("esc close"),
        th,
    ));
    Surface::section(model.width, th)
        .inset(inset)
        .title(vec![span("Resume session", th.primary)])
        .rows(body.into_iter().map(|r| r.spans).collect())
        .lines()
}

pub fn recall(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let meta = &model.recall_meta;
    let mut rows = Vec::new();
    if !meta.query.is_empty() {
        rows.extend(section_heading(
            width,
            th,
            &format!("Query: {}", meta.query),
        ));
    }
    if model.recall.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            "No recalled memories for this query.",
        ));
    }
    for hit in &model.recall {
        let state = if hit.above_floor {
            "eligible"
        } else {
            "below relevance floor"
        };
        rows.extend(section_heading(width, th, &hit.summary));
        rows.extend(section_detail(
            width,
            th,
            &format!("Score {:.2} · {state}", hit.score),
        ));
        rows.extend(section_detail(width, th, &hit.location));
        rows.extend(section_detail(width, th, &hit.provenance));
    }
    if !model.recall.is_empty() {
        let held = model.recall.iter().filter(|hit| !hit.above_floor).count();
        rows.extend(section_detail(
            width,
            th,
            &format!("Held back: {held} · relevance floor {:.2}", meta.floor),
        ));
    }
    for (label, value) in [
        ("Promoted to context", &meta.promoted),
        ("Index freshness", &meta.freshness),
        ("Metric", &meta.metric),
        ("Elapsed", &meta.elapsed),
        ("Requested hits", &meta.k),
        ("Vectors", &meta.vectors),
        ("Shards", &meta.shards),
        ("Embedding model", &meta.embedding),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows
}

/// Kept for reusable callers: the same focus gutter and name-first allocation.
pub fn picker_row(
    theme: &Theme,
    inner: u16,
    name: &str,
    right_text: &str,
    selected: bool,
) -> Vec<Span<'static>> {
    section_row(inner, theme, selected, name, right_text).spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        model::{Overlay, SessionItem},
        theme::{ColorDepth, Theme},
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress(&mut m);
        m.toggle_overlay(Overlay::Sessions);
        m
    }
    fn text(rows: &[Line<'_>]) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn session_facts_belong_to_the_selected_session_and_are_never_filled_from_the_current_conversation(
    ) {
        let mut m = model(80);
        assert!(text(&session_lines(&m)).contains("forked from provider-parity"));
        m.sessions = vec![SessionItem::new("without-metadata", "2h")];
        let drawn = text(&session_lines(&m));
        assert!(drawn.contains("without-metadata") && drawn.contains("updated 2h"));
        assert!(!drawn.contains("this session") && !drawn.contains("tokens"));
        assert!(!drawn.contains("d delete") && !drawn.contains("f fork"));
    }
    #[test]
    fn tall_session_lists_keep_the_selected_name_title_and_exit() {
        let mut m = model(80);
        m.sessions = (0..80)
            .map(|i| SessionItem::new(&format!("session-{i}"), "2h"))
            .collect();
        m.session_index = 50;
        let rows = sessions(&m, 14);
        let drawn = text(&rows);
        assert!(rows.len() <= 14);
        for value in [
            "Resume session",
            "session-50",
            "above",
            "below",
            "esc close",
        ] {
            assert!(drawn.contains(value), "{drawn}");
        }
        m.move_selection(30);
        assert_eq!(m.selection(80), Some(0));
    }
    #[test]
    fn recall_preserves_sources_scores_and_the_floor_without_an_invented_projection_or_cursor() {
        let m = model(80);
        let rows = recall(&m);
        let drawn = text(&rows);
        for hit in &m.recall {
            assert!(drawn.contains(&format!("{:.2}", hit.score)));
            assert!(drawn.contains(&hit.location));
        }
        assert!(drawn.contains("relevance floor"));
        assert!(!drawn.contains("PROJECTION") && !drawn.contains("18k pts"));
        assert!(ui::focused_row(&rows).is_none());
    }
    #[test]
    fn empty_and_narrow_sessions_and_recall_are_bounded() {
        let mut m = model(80);
        m.sessions.clear();
        m.recall.clear();
        assert!(text(&sessions(&m, 24)).contains("No earlier sessions"));
        assert!(text(&sessions(&m, 24)).contains("esc close"));
        assert!(text(&recall(&m)).contains("No recalled memories"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let m = model(width);
            for row in sessions(&m, 24).into_iter().chain(recall(&m)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
