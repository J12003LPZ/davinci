//! `1e` — the workspace sidebar.
//!
//! The only persistent split in the product, opt-in at ≥120 columns
//! (design.md §1, §7). At ≥150 columns the git changes popover is allowed
//! under the transcript.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/codex.ex`.

use ratatui::text::{Line, Span};

use super::transcript;
use crate::davinci::model::Model;
use crate::davinci::ui::{
    blank, clip_ellipsis, pad, run_width, span, span_on, span_strong, surface_rule, tail, Surface,
};

/// The sidebar takes a quarter of the window, and the transcript the rest.
pub fn sidebar_width(width: u16) -> u16 {
    (width / 4).clamp(24, 40)
}

/// The two columns, already merged into one row list of `height` rows.
pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    let side_width = sidebar_width(model.width);
    let main_width = model.width - side_width - 1;

    let side = fit(sidebar(model, side_width), height, side_width);

    // The git changes popover floats at the bottom right of the transcript
    // column, clear of the flow (`1e`).
    let mut main_rows = transcript::lines(model, &model.transcript, main_width);
    if model.wide() && !model.changes_list.is_empty() {
        let popover = changes(model, main_width);
        let room = height.saturating_sub(popover.len());
        main_rows = tail(main_rows, room);
        while main_rows.len() < room {
            main_rows.push(blank());
        }
        let lead = main_width.saturating_sub(46);
        for row in popover {
            let mut spans = vec![pad(lead, None)];
            spans.extend(row.spans);
            main_rows.push(Line::from(spans));
        }
    }
    let main = fit(main_rows, height, main_width);

    side.into_iter()
        .zip(main)
        .map(|(left, right)| {
            let mut spans = left.spans;
            spans.push(span(" ", model.theme.border));
            spans.extend(right.spans);
            Line::from(spans)
        })
        .collect()
}

/// Tail-truncate, pad to `height`, and make every row exactly `width` wide so
/// the two columns stay aligned.
fn fit(rows: Vec<Line<'static>>, height: usize, width: u16) -> Vec<Line<'static>> {
    let mut rows = tail(rows, height);
    while rows.len() < height {
        rows.push(blank());
    }
    rows.into_iter()
        .map(|row| {
            let used = run_width(&row.spans);
            let mut spans = row.spans;
            spans.push(pad(width.saturating_sub(used), None));
            Line::from(spans)
        })
        .collect()
}

fn sidebar(model: &Model, width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut body: Vec<Vec<Span<'static>>> = model
        .tree
        .iter()
        .map(|row| {
            // The row in hand carries the same 1-cell copper bar and tint as
            // every other selection (`1e`).
            let tint = if row.selected { Some(th.surface) } else { None };
            let name_color = if row.selected || row.depth == 0 {
                th.text
            } else {
                th.muted
            };
            let twisty_color = if row.selected { th.primary } else { th.border };
            let lead = row.depth * 2;
            let mut spans = if row.selected {
                vec![
                    span_on("▌", th.primary, tint),
                    pad(lead.saturating_sub(1), tint),
                ]
            } else {
                vec![pad(lead, tint)]
            };
            spans.push(span_on(
                match &row.twisty {
                    Some(twisty) => format!("{twisty} "),
                    None => "  ".to_string(),
                },
                twisty_color,
                tint,
            ));
            spans.push(span_on(
                clip_ellipsis(&row.name, width.saturating_sub(12)),
                name_color,
                tint,
            ));
            if let Some(status) = row.status {
                spans.push(span_on(
                    format!(" {}", status.glyph()),
                    th.state_color(status),
                    tint,
                ));
            }
            spans
        })
        .collect();

    body.push(Vec::new());
    body.push(surface_rule(width, th));
    body.push(vec![
        span("ctrl+e close", th.border),
        span(" │ ", th.border),
        span("/ filter", th.border),
    ]);

    Surface::new(width, th)
        .title(vec![
            span("CODEX", th.primary),
            span(" · ", th.border),
            span("WORKSPACE", th.muted),
        ])
        .rows(body)
        .lines()
}

/// The git changes popover, allowed only at ≥150 columns (`1e`).
fn changes(model: &Model, width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    let box_width = width.min(46);
    let inner = box_width.saturating_sub(4);

    let body: Vec<Vec<Span<'static>>> = model
        .changes_list
        .iter()
        .map(|change| {
            let color = if change.status == "A" {
                th.success
            } else if change.status == "D" {
                th.error
            } else {
                th.warning
            };
            let left = vec![
                span_strong(format!("{}  ", change.status), color, th),
                span(
                    clip_ellipsis(&change.path, inner.saturating_sub(10)),
                    th.muted,
                ),
            ];
            let right = vec![span(change.count.clone(), th.success)];
            let gap = inner
                .saturating_sub(run_width(&left))
                .saturating_sub(run_width(&right))
                .max(1);
            let mut row = left;
            row.push(pad(gap, None));
            row.extend(right);
            row
        })
        .collect();

    Surface::new(box_width, th)
        .title(vec![span("CHANGES", th.secondary)])
        .rows(body)
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_codex();
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_split_is_row_exact_and_fills_the_window() {
        for width in [120u16, 160, 200] {
            for row in lines(&model(width), 30) {
                assert_eq!(run_width(&row.spans), width, "at {width}");
            }
        }
    }

    #[test]
    fn the_sidebar_names_itself_and_states_its_exit() {
        let rows: Vec<String> = lines(&model(160), 30).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("╭─ CODEX · WORKSPACE ─")));
        assert!(rows
            .iter()
            .any(|row| row.contains("ctrl+e close │ / filter")));
    }

    #[test]
    fn the_row_in_hand_carries_a_copper_bar_and_a_tint() {
        let m = model(160);
        let rows = lines(&m, 30);
        let selected = rows
            .iter()
            .find(|row| text(row).contains("davinci-session Δ"))
            .expect("the selected tree row");
        assert!(text(selected).contains('▌'), "{}", text(selected));
        assert!(selected
            .spans
            .iter()
            .any(|span| span.style.bg == Some(m.theme.surface)));
    }

    #[test]
    fn a_changed_file_carries_a_glyph_not_just_a_colour() {
        let m = model(160);
        let rows: Vec<String> = lines(&m, 30).iter().map(text).collect();
        assert!(
            rows.iter().any(|row| row.contains("davinci-session Δ")),
            "a changed directory is marked"
        );
        assert!(
            rows.iter().any(|row| row.contains("store.rs ×")),
            "a failing file is marked"
        );
    }

    #[test]
    fn the_changes_popover_appears_only_at_a_hundred_and_fifty_columns() {
        let narrow: Vec<String> = lines(&model(120), 30).iter().map(text).collect();
        assert!(!narrow.iter().any(|row| row.contains("CHANGES")));

        let wide: Vec<String> = lines(&model(160), 30).iter().map(text).collect();
        assert!(wide.iter().any(|row| row.contains("╭─ CHANGES ─")));
        // The popover floats bottom-right of the transcript column (`1e`).
        let top = wide
            .iter()
            .find(|row| row.contains("╭─ CHANGES ─"))
            .unwrap();
        let column = top.find("╭─ CHANGES").unwrap();
        assert!(column > 60, "the popover hugs the right edge: {top}");
    }

    #[test]
    fn the_sidebar_is_a_quarter_of_the_window_within_bounds() {
        assert_eq!(sidebar_width(120), 30);
        assert_eq!(sidebar_width(160), 40);
        assert_eq!(sidebar_width(400), 40);
        assert_eq!(sidebar_width(80), 24);
    }

    #[test]
    fn the_transcript_beside_the_sidebar_still_wraps_at_the_measure() {
        let m = model(200);
        let side = sidebar_width(200);
        for row in lines(&m, 30) {
            let main: String = text(&row).chars().skip((side + 1) as usize).collect();
            assert!(
                unicode_width::UnicodeWidthStr::width(main.trim_end()) <= (200 - side - 1) as usize
            );
        }
    }
}
