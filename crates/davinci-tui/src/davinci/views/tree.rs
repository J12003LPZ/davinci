//! `4b` — `/tree`. The session as it actually is: a tree, with the forks that
//! were abandoned still on it.
//!
//! The graph rules from `2a` hold here (design.md §6): the trunk column a
//! child inherits is drawn for every row of that child, and no vertical ever
//! descends through label text — the trunk is built as its own run of segments
//! before the glyph, never interleaved with the label.
//!
//! Mirrors artboard `4b` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::{Model, TreeNode};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, pad, run_width, span, span_on, span_strong, truncate_run, Surface,
};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let list = &model.session_tree;

    if list.iter().all(|row| row.id.is_none()) {
        return vec![Line::from(vec![span(
            "the session has no turns yet — the tree grows as you work",
            th.muted,
        )])];
    }

    let filters = Line::from(vec![
        span("filter ", th.border),
        span_on(" all ", th.background, Some(th.primary)),
        span(" ", th.border),
        span(" no tools ", th.muted),
        span(" user only ", th.muted),
        span(" labeled ", th.muted),
        span("   ", th.border),
        span(" timestamps on ", th.muted),
    ]);

    let branches = branch_count(list);
    let abandoned = list
        .iter()
        .filter(|row| row.state == Some(State::Failed))
        .count();
    let inner = width.saturating_sub(4);
    let mut body: Vec<Vec<Span<'static>>> = Vec::new();
    for (index, entry) in list.iter().enumerate() {
        if entry.id.is_none() {
            body.push(vec![span(entry.trunk.clone(), th.border)]);
            continue;
        }
        body.push(row(entry, index == model.tree_index, inner, th));
        if let Some(detail) = &entry.detail {
            // What the turn carries, under its label, past the trunk.
            let lead = run_width(&[span(entry.trunk.clone(), th.border)]) + 6;
            body.push(vec![
                span(continuation(&entry.trunk), th.border),
                pad(
                    lead.saturating_sub(run_width(&[span(continuation(&entry.trunk), th.border)])),
                    None,
                ),
                span(clip_ellipsis(detail, inner.saturating_sub(lead)), th.border),
            ]);
        }
    }

    let mut right = vec![span(format!("{branches} branches"), th.muted)];
    if abandoned > 0 {
        right.push(span(" · ", th.border));
        right.push(span(format!("{abandoned} abandoned"), th.muted));
    }
    right.push(span(" · ", th.border));
    right.push(span(format!("{} nothing lost", glyph::DONE), th.success));
    let mut out = vec![filters, blank()];
    out.extend(
        Surface::new(width, th)
            .title(vec![
                span("MEMORIA", th.primary),
                span(" · ", th.border),
                span("SESSION TREE", th.muted),
            ])
            .right(right)
            .rows(body)
            .lines(),
    );
    out.push(blank());

    let current = list
        .get(model.tree_index)
        .filter(|row| row.id.is_some())
        .or_else(|| list.iter().find(|row| row.id.is_some()));
    if let Some(current) = current {
        let (used, cap) = model.context;
        let id = current.id.clone().unwrap_or_default();
        out.push(Line::from(vec![
            span("turn ", th.muted),
            span(id, th.primary),
            span(" · what resuming here would carry", th.muted),
        ]));
        let mut cost = vec![
            span("context at this point ", th.muted),
            span(
                format!(
                    "{}/{}",
                    super::chrome::thousands(used),
                    super::chrome::thousands(cap)
                ),
                th.text,
            ),
        ];
        if !model.facts.session_cost.is_empty() {
            cost.push(span(" · cost so far ", th.muted));
            cost.push(span(model.facts.session_cost.clone(), th.text));
        }
        out.push(Line::from(cost));
        if !model.facts.tree_summary.is_empty() {
            out.push(Line::from(vec![
                span(format!("{} ", glyph::DONE), th.success),
                span(model.facts.tree_summary.clone(), th.muted),
            ]));
        }
    }
    let mut ahead = vec![
        span(format!("{} ", glyph::READ), th.secondary),
        span("working tree is ahead of this turn", th.muted),
    ];
    if model.changes.0 > 0 {
        ahead.push(span(" · ", th.border));
        ahead.push(span(
            format!("{} files changed since", model.changes.0),
            th.muted,
        ));
    }
    ahead.push(span(" · the tree does not move your files", th.border));
    out.push(Line::from(ahead));
    if !model.facts.tree_branch_note.is_empty() {
        out.push(Line::from(vec![
            span(format!("{} ", glyph::ATTENTION), th.warning),
            span(model.facts.tree_branch_note.clone(), th.warning),
        ]));
    }
    // Loose rows outside the surface are cut to the window, never wrapped.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The trunk a detail row under a node keeps: the node's own connector
/// becomes a vertical or a blank, so the tree's verticals stay continuous.
fn continuation(trunk: &str) -> String {
    trunk.replace("├── ", "│   ").replace("└── ", "    ")
}

/// Branches: the trunk plus every fork point drawn as `├──` or `└──` after
/// the first child of a node. A tree of one line is one branch.
fn branch_count(list: &[TreeNode]) -> usize {
    let forks = list
        .iter()
        .filter(|row| row.id.is_some() && row.trunk.contains("└── "))
        .count();
    forks.max(1)
}

/// The sheet's frame (design.md §11): the session in the header, the turn
/// in hand in the status bar, no composer.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let turns = model
        .session_tree
        .iter()
        .filter(|row| row.id.is_some())
        .count();
    let branches = branch_count(&model.session_tree);
    let current = model
        .session_tree
        .get(model.tree_index)
        .and_then(|row| row.id.clone());
    SheetChrome {
        header_right: facts(
            th,
            vec![
                (!model.facts.session_name.is_empty())
                    .then(|| vec![span(model.facts.session_name.clone(), th.text)])
                    .unwrap_or_default(),
                (turns > 0)
                    .then(|| vec![span(format!("{turns} turns"), th.muted)])
                    .unwrap_or_default(),
                (turns > 0)
                    .then(|| vec![span(format!("{branches} branches"), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: current.map(|id| vec![span(format!("turn {id} of {turns}"), th.muted)]),
        status_right: None,
        hints: vec![
            hint(th, "↑↓ move"),
            hint(th, "enter switch to turn"),
            hint_dim(th, "ctrl+←/→ fold"),
            hint_dim(th, "shift+l label"),
            hint(th, "f fork here"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
    }
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
        vec![span("◀ here", th.primary)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn node(trunk: &str, state: Option<State>, id: Option<&str>, label: Option<&str>) -> TreeNode {
        TreeNode {
            trunk: trunk.into(),
            state,
            id: id.map(str::to_string),
            label: label.map(str::to_string),
            meta: id.map(|_| "12:04".to_string()),
            entry_id: id.unwrap_or_default().to_string(),
            detail: None,
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
            "1 branches",
            "timestamps on",
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
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn the_selected_turn_is_marked_here_and_carries_the_context_cost() {
        let mut m = model(100);
        m.tree_index = 4;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let here = rows
            .iter()
            .find(|row| row.contains("◀ here"))
            .expect("the selection is marked");
        assert!(here.contains("03"), "{here}");
        assert!(rows
            .iter()
            .any(|row| row.contains("turn 03 · what resuming here would carry")));
        assert!(rows
            .iter()
            .any(|row| row.contains("context at this point 47k/200k")));
    }

    #[test]
    fn a_node_detail_sits_under_its_label_past_the_trunk() {
        let mut m = model(100);
        m.session_tree[2].detail = Some("abandoned · 2 files reverted".into());
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let index = rows
            .iter()
            .position(|row| row.contains("surveyed the workspace"))
            .unwrap();
        assert!(rows[index + 1].contains("abandoned · 2 files reverted"));
        assert!(rows[index + 1].starts_with("│ │"), "{}", rows[index + 1]);
    }

    #[test]
    fn an_empty_tree_says_so() {
        let mut m = model(100);
        m.session_tree.clear();
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no turns yet")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "4b");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "review-agent-runtime │ 6 turns │ 3 branches");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "turn 05 of 6");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Hidden);
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(hint.starts_with("↑↓ move │ enter switch to turn"), "{hint}");
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("3 branches · 1 abandoned · ✓ nothing lost")));
        assert!(rows.iter().any(|row| row.contains("cost so far $0.84")));
        assert!(rows
            .iter()
            .any(|row| row.contains("branch 06 has its own 9 turns")));
        assert!(rows
            .iter()
            .any(|row| row.contains("Δ 3 +42 -11 label: store-fix")));
    }

    #[test]
    fn no_row_overflows_the_window_and_the_surface_is_row_exact() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            let rows = lines(&m);
            let surface_top = rows
                .iter()
                .find(|row| text(row).contains("SESSION TREE"))
                .expect("the surface is drawn");
            assert_eq!(run_width(&surface_top.spans), width, "at {width}");
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
