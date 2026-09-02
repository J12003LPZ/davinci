//! `4b` — `/tree`. The session as it actually is: a tree, with the forks that
//! were abandoned still on it.
//!
//! The graph rules from `2a` hold here (design.md §6): the trunk column a
//! child inherits is drawn for every row of that child, and no vertical ever
//! descends through label text — the trunk is built as its own run of segments
//! before the glyph, never interleaved with the label.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/tree.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, TreeNode};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, pad, run_width, span, span_on, span_strong, truncate_run, Surface,
    MEASURE,
};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let list = &model.session_tree;

    if list.iter().all(|row| row.id.is_none()) {
        return vec![
            Line::from(vec![span(
                "the session has no turns yet — the tree grows as you work",
                th.muted,
            )]),
            Line::from(vec![span("esc close", th.border)]),
        ];
    }

    let filters = Line::from(vec![
        span("filter ", th.border),
        span_on(" all ", th.background, Some(th.primary)),
        span(" ", th.border),
        span(" no tools ", th.muted),
        span(" user only ", th.muted),
        span(" labeled ", th.muted),
    ]);

    let turns = list.iter().filter(|row| row.id.is_some()).count();
    let inner = width.saturating_sub(4);
    // A long session is windowed around the turn in hand — the sheet is
    // tail-anchored and would otherwise lose its head. The window is wide so
    // the trunks stay legible; what is folded is counted either side.
    const WINDOW: usize = 24;
    let total = list.len();
    let start = model
        .tree_index
        .saturating_sub(WINDOW / 2)
        .min(total.saturating_sub(WINDOW));
    let end = (start + WINDOW).min(total);
    let mut body: Vec<Vec<Span<'static>>> = Vec::new();
    if start > 0 {
        body.push(vec![span(format!("… {start} rows above"), th.border)]);
    }
    body.extend(
        list.iter()
            .enumerate()
            .skip(start)
            .take(end - start)
            .map(|(index, entry)| {
                if entry.id.is_none() {
                    vec![span(entry.trunk.clone(), th.border)]
                } else {
                    row(entry, index == model.tree_index, inner, th)
                }
            }),
    );
    if end < total {
        body.push(vec![span(
            format!("… {} rows below", total - end),
            th.border,
        )]);
    }

    let mut out = vec![filters, blank()];
    out.extend(
        Surface::new(width, th)
            .title(vec![
                span("MEMORIA", th.primary),
                span(" · ", th.border),
                span("SESSION TREE", th.muted),
            ])
            .right(vec![
                span(format!("{turns} turns"), th.muted),
                span(" · ", th.border),
                span(format!("{} nothing lost", glyph::DONE), th.success),
            ])
            .rows(body)
            .lines(),
    );
    out.push(blank());

    // What is knowable about the turn in hand from the model itself; the
    // Elixir fixture's cost and turn-count prose has no source here yet.
    let current = list
        .get(model.tree_index)
        .filter(|row| row.id.is_some())
        .or_else(|| list.iter().find(|row| row.id.is_some()));
    if let Some(current) = current {
        let (used, cap) = model.context;
        out.push(Line::from(vec![
            span("turn ", th.muted),
            span(current.id.clone().unwrap_or_default(), th.primary),
            span(" · context ", th.muted),
            span(
                format!(
                    "{}/{}",
                    super::chrome::thousands(used),
                    super::chrome::thousands(cap)
                ),
                th.text,
            ),
        ]));
    }
    out.push(Line::from(vec![
        span(format!("{} ", glyph::READ), th.secondary),
        span("the working tree is ahead of this turn", th.muted),
        span(" · the tree never moves your files", th.border),
    ]));
    out.push(blank());
    out.push(Line::from(vec![
        span("↑↓ move", th.border),
        span(" · ", th.border),
        span("enter switch to turn", th.border),
        span(" · ", th.border),
        span("f fork here", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));
    // Loose rows outside the surface are cut to the window, never wrapped.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

/// A segment list, not a line: these rows are the body of the surface, and the
/// surface draws the border around each one.
fn row(entry: &TreeNode, selected: bool, inner: u16, th: &Theme) -> Vec<Span<'static>> {
    let state = if selected {
        State::Active
    } else {
        entry.state.unwrap_or(State::Queued)
    };
    let text_color = if selected { th.text } else { th.muted };

    let mut left = vec![
        span(entry.trunk.clone(), th.border),
        span_strong(format!("{} ", state.glyph()), th.state_color(state), th),
        span(
            format!("{}  ", entry.id.clone().unwrap_or_default()),
            th.border,
        ),
        span(
            clip_ellipsis(entry.label.as_deref().unwrap_or(""), 40),
            text_color,
        ),
    ];
    let right: Vec<Span<'static>> = if selected {
        vec![span(format!("{} here", glyph::READ), th.primary)]
    } else if let Some(meta) = &entry.meta {
        vec![span(meta.clone(), th.border)]
    } else {
        Vec::new()
    };
    let gap = inner
        .saturating_sub(run_width(&left))
        .saturating_sub(run_width(&right))
        .max(1);
    left.push(pad(gap, None));
    left.extend(right);
    left
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn node(trunk: &str, state: Option<State>, id: Option<&str>, label: Option<&str>) -> TreeNode {
        TreeNode {
            trunk: trunk.into(),
            state,
            id: id.map(str::to_string),
            label: label.map(str::to_string),
            meta: id.map(|_| "12:04".to_string()),
            entry_id: id.unwrap_or_default().to_string(),
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.session_tree = vec![
            node(
                "",
                Some(State::Queued),
                Some("01"),
                Some("explain the runtime"),
            ),
            node("│", None, None, None),
            node(
                "├── ",
                Some(State::Done),
                Some("02"),
                Some("surveyed the workspace"),
            ),
            node("│", None, None, None),
            node(
                "└── ",
                Some(State::Active),
                Some("03"),
                Some("fix the type error"),
            ),
        ];
        model.tree_index = 0;
        model.context = (47_000, 200_000);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_turn_is_on_the_tree_and_spacers_keep_the_trunk() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        for expected in [
            "01",
            "02",
            "03",
            "explain the runtime",
            "SESSION TREE",
            "3 turns",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is missing"
            );
        }
        // A spacer row inside the surface carries only the trunk.
        assert!(
            rows.iter()
                .any(|row| row.starts_with("│ │") && !row.contains('◉')),
            "{rows:?}"
        );
    }

    #[test]
    fn the_selected_turn_is_marked_here_and_carries_the_context_cost() {
        let mut m = model(100);
        m.tree_index = 4;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let here = rows
            .iter()
            .find(|row| row.contains("here"))
            .expect("the selection is marked");
        assert!(here.contains("03"), "{here}");
        assert!(rows
            .iter()
            .any(|row| row.contains("turn") && row.contains("03") && row.contains("47k/200k")));
    }

    #[test]
    fn an_empty_tree_says_so() {
        let mut m = model(100);
        m.session_tree.clear();
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no turns yet")));
    }

    #[test]
    fn no_row_overflows_the_window_and_the_surface_is_row_exact() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            let sheet = width.min(MEASURE + 14);
            let rows = lines(&m);
            let surface_top = rows
                .iter()
                .find(|row| text(row).contains("SESSION TREE"))
                .expect("the surface is drawn");
            assert_eq!(run_width(&surface_top.spans), sheet, "at {width}");
            for row in &rows {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(row)
                );
            }
        }
    }
}
