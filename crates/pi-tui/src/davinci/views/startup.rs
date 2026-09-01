//! `1a` — startup and the empty state.
//!
//! The identity mark is a line-drawn Vitruvian Man — l'uomo vitruviano, the
//! figure in the circle and the square — built from the same box-drawing set
//! as the UI, with the navel, the compass point of Leonardo's circle, as its
//! only copper stroke. It appears here and nowhere else (design.md §10), and
//! it is dropped entirely below 100 columns (§7).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/startup.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, Startup};
use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{blank, center, hair_rule, indent, span, span_strong};

/// The mark, as it appears in screen `1a`: the figure with both pairs of
/// limbs — arms straight to the square, arms raised to the circle, legs
/// together on the square's base, legs spread to the circle's rim.
const EMBLEM: [&str; 14] = [
    "         ·───────────·",
    "      ╱                 ╲",
    "    ╱    ┌───────────┐    ╲",
    "   │     │    ╭─╮    │     │",
    "  │    ╲ │    ╰┬╯    │ ╱    │",
    "  │     ╲├─────┼─────┤╱     │",
    "  │      │     │     │      │",
    "  │      │    ─·─    │      │",
    "  │      │   ╱│ │╲   │      │",
    "  │      │  ╱ │ │ ╲  │      │",
    "   │     │ ╱  │ │  ╲ │     │",
    "    ╲    │╱   │ │   ╲│    ╱",
    "      ╲  └────┴─┴────┘  ╱",
    "         ·───────────·",
];

/// The one copper stroke: the navel, centre of the circle, as Leonardo
/// annotated it.
const NAVEL_ROW: usize = 7;
const NAVEL: &str = "─·─";

pub fn lines(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let mut rows = Vec::new();

    if model.decoration() {
        rows.extend(emblem(th, width));
        rows.push(blank());
        annotate(th, width, &mut rows);
    }

    rows.push(center(
        width,
        vec![span_strong("D A V I N C I", th.text, th)],
    ));
    rows.push(center(
        width,
        vec![span("macchina dell'intelletto", th.muted)],
    ));
    rows.push(blank());
    rows.push(center(width, vec![span(info.cwd.clone(), th.secondary)]));
    rows.push(center(
        width,
        vec![
            span(info.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(info.language.clone(), th.muted),
            span(" · ", th.border),
            span(info.crates.clone(), th.muted),
        ],
    ));
    rows.push(center(width, restored_row(th, info.restored)));
    rows.push(blank());

    let rule_width = width.min(62);
    let rule = hair_rule(rule_width, th, "◦");
    rows.push(center(width, rule.spans));
    rows.push(blank());
    rows.push(center(
        width,
        vec![span("A machine for thought, built in Rust.", th.text)],
    ));

    rows
}

/// The margin note beside the mark — `proportio humana`, in faded copper, at
/// the mark's shoulder (`1a`). Decoration only; dropped with the mark.
fn annotate(theme: &Theme, width: u16, rows: &mut [Line<'static>]) {
    let faded = theme.dim().primary;
    let margin = width.saturating_sub(emblem_width()) / 2 + emblem_width() + 4;
    for (row, note) in [(2usize, "proportio"), (3, "humana")] {
        let Some(line) = rows.get_mut(row) else {
            continue;
        };
        let used = crate::davinci::ui::run_width(&line.spans);
        if margin + 12 > width || used > margin {
            continue;
        }
        line.spans
            .push(crate::davinci::ui::pad(margin - used, None));
        line.spans.push(span(note, faded));
    }
}

fn restored_row(theme: &Theme, restored: bool) -> Vec<Span<'static>> {
    if restored {
        vec![
            span_strong(format!("{} ", glyph::DONE), theme.success, theme),
            span("session restored", theme.muted),
            span(" · ", theme.border),
            span("memoria intacta", theme.muted),
        ]
    } else {
        vec![
            span_strong(format!("{} ", glyph::QUEUED), theme.border, theme),
            span("new session", theme.muted),
        ]
    }
}

/// The mark is centred as one block, not row by row: each row keeps its own
/// leading space, or the drawing skews.
fn emblem(theme: &Theme, width: u16) -> Vec<Line<'static>> {
    let lead = width.saturating_sub(emblem_width()) / 2;
    EMBLEM
        .iter()
        .enumerate()
        .map(|(index, row)| indent(lead, emblem_row(theme, row, index)))
        .collect()
}

fn emblem_row(theme: &Theme, row: &str, index: usize) -> Vec<Span<'static>> {
    if index != NAVEL_ROW {
        return vec![span(row.to_string(), theme.muted)];
    }
    match row.split_once(NAVEL) {
        Some((head, tail)) => vec![
            span(head.to_string(), theme.muted),
            span_strong(NAVEL, theme.primary, theme),
            span(tail.to_string(), theme.muted),
        ],
        None => vec![span(row.to_string(), theme.muted)],
    }
}

/// Row count, so the shell can centre the empty state in the body.
pub fn height(model: &Model) -> usize {
    // mark + gap, then the ten rows of copy; the margin note rides on the
    // mark's own rows.
    let copy = 10;
    if model.decoration() {
        EMBLEM.len() + 1 + copy
    } else {
        copy
    }
}

/// Widest row, so nothing overflows a narrow window.
pub fn emblem_width() -> u16 {
    EMBLEM
        .iter()
        .map(|row| unicode_width::UnicodeWidthStr::width(*row) as u16)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::ColorDepth;
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        )
    }

    fn info() -> Startup {
        Startup {
            cwd: "C:\\dev\\oss\\davinci-rust".into(),
            branch: "main".into(),
            language: "rust".into(),
            crates: "11 crates".into(),
            restored: true,
        }
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_mark_is_drawn_at_a_hundred_columns_and_dropped_below() {
        let wide = lines(&model(100), &info());
        assert!(text(&wide[0]).contains('·'));
        assert_eq!(wide.len(), height(&model(100)));

        let narrow = lines(&model(80), &info());
        assert!(!text(&narrow[0]).contains('╱'), "no ASCII art at 80");
        assert!(text(&narrow[0]).contains("D A V I N C I"));
        assert_eq!(narrow.len(), height(&model(80)));
    }

    #[test]
    fn the_navel_is_the_only_copper_stroke_in_the_mark() {
        let m = model(120);
        let th = m.theme;
        let rows = lines(&m, &info());
        let copper: Vec<String> = rows[..EMBLEM.len()]
            .iter()
            .flat_map(|row| row.spans.iter())
            .filter(|span| span.style.fg == Some(th.primary))
            .map(|span| span.content.to_string())
            .collect();
        assert_eq!(copper, vec![NAVEL.to_string()]);
    }

    #[test]
    fn the_mark_is_centred_as_a_block_so_the_drawing_does_not_skew() {
        let rows = lines(&model(120), &info());
        let leads: Vec<usize> = rows[..EMBLEM.len()]
            .iter()
            .map(|row| text(row).len() - text(row).trim_start().len())
            .collect();
        let own: Vec<usize> = EMBLEM
            .iter()
            .map(|row| row.len() - row.trim_start().len())
            .collect();
        let base = leads[0] - own[0];
        for (index, lead) in leads.iter().enumerate() {
            assert_eq!(
                lead - own[index],
                base,
                "row {index} was centred on its own width"
            );
        }
    }

    #[test]
    fn the_mark_never_overflows_the_window() {
        for width in [100u16, 120, 160] {
            for row in lines(&model(width), &info()) {
                assert!(
                    run_width(&row.spans) <= width,
                    "row overflows {width}: {:?}",
                    text(&row)
                );
            }
        }
        assert!(emblem_width() < 100);
    }

    #[test]
    fn the_empty_state_names_where_it_is_and_what_it_found() {
        let rows: Vec<String> = lines(&model(120), &info()).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("macchina dell'intelletto")));
        assert!(rows
            .iter()
            .any(|row| row.contains("C:\\dev\\oss\\davinci-rust")));
        assert!(rows
            .iter()
            .any(|row| row.contains("main · rust · 11 crates")));
        assert!(rows
            .iter()
            .any(|row| row.contains("✓ session restored · memoria intacta")));
        assert!(rows
            .iter()
            .any(|row| row.contains("A machine for thought, built in Rust.")));
        // The margin note rides the mark's shoulder, two words on two rows.
        assert!(rows.iter().any(|row| row.contains("proportio")));
        assert!(rows.iter().any(|row| row.contains("humana")));
    }

    #[test]
    fn a_fresh_session_says_so_with_its_own_glyph() {
        let mut fresh = info();
        fresh.restored = false;
        let rows: Vec<String> = lines(&model(120), &fresh).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("○ new session")));
        assert!(!rows.iter().any(|row| row.contains("session restored")));
    }

    #[test]
    fn the_hair_rule_is_capped_so_it_does_not_span_a_wide_window() {
        let rows = lines(&model(160), &info());
        let rule = rows
            .iter()
            .find(|row| text(row).contains('◦'))
            .expect("hair rule");
        assert!(run_width(&rule.spans) <= 62 + (160 - 62) / 2 + 1);
    }
}
