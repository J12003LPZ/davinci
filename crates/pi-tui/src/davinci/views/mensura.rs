//! `2c` — the token governor.
//!
//! Budget by role, one row each: `role tokens meter cap`. Rows within cap use
//! verdigris, the breaching row copper with a warning cap note. The proposal
//! always states recovers / keeps / cost / reversible, then keyed actions. It
//! never acts silently (design.md §6).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/mensura.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, Proposal};
use crate::davinci::theme::glyph;
use crate::davinci::ui::{blank, meter, pad, run_width, span, span_strong, wrap, Surface, MEASURE};

/// How many cells the per-role meters take.
const METER_CELLS: u16 = 24;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let meta = &model.budget_meta;

    let mut rows = vec![
        Line::from(vec![
            span("in use ", th.muted),
            span(meta.in_use.clone(), th.text),
            span(" of ", th.muted),
            span(meta.window.clone(), th.text),
        ]),
        Line::from(vec![
            span("headroom ", th.muted),
            span(meta.headroom.clone(), th.text),
            span(" · ", th.border),
            span(meta.rate.clone(), th.muted),
            span(" · ", th.border),
            span(
                format!(
                    "{} {}%",
                    th.pie(meta.in_use_fraction),
                    (meta.in_use_fraction * 100.0) as u32
                ),
                th.primary,
            ),
        ]),
        blank(),
    ];

    for item in &model.budget {
        let color = if item.breach {
            th.primary
        } else {
            th.secondary
        };
        let role_color = if item.breach { th.text } else { th.muted };
        let note_color = if item.breach { th.warning } else { th.border };

        let mut row = vec![
            span(format!("{:<13}", item.role), role_color),
            span(format!("{:>6}  ", item.tokens), color),
        ];
        row.extend(meter(item.fraction, METER_CELLS, th, Some(color)));
        let note = vec![span(item.note.clone(), note_color)];
        let gap = width
            .saturating_sub(run_width(&row))
            .saturating_sub(run_width(&note))
            .max(1);
        row.push(pad(gap, None));
        row.extend(note);
        rows.push(Line::from(row));
    }

    if let Some(proposal) = &model.proposal {
        rows.push(blank());
        rows.extend(governor(model, proposal, width));
    }

    rows.push(blank());
    rows.push(Line::from(vec![
        span("session spend ", th.muted),
        span(meta.session_spend.clone(), th.text),
        span(" · ", th.border),
        span(format!("daily cap {}", meta.daily_cap), th.muted),
        span(" · ", th.border),
        span(
            format!(
                "{} {}%",
                th.pie(meta.daily_fraction),
                (meta.daily_fraction * 100.0) as u32
            ),
            th.primary,
        ),
    ]));
    rows.push(Line::from(vec![span(meta.history.clone(), th.muted)]));
    rows
}

/// The proposal block. Bordered in warning, and it always says what it
/// recovers, what it keeps, what it costs and whether it can be undone.
fn governor(model: &Model, proposal: &Proposal, width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut body: Vec<Vec<Span<'static>>> = wrap(
        &format!("{} {}", glyph::ATTENTION, proposal.summary),
        width.saturating_sub(6),
    )
    .into_iter()
    .map(|row| vec![span(row, th.text)])
    .collect();

    body.push(Vec::new());
    body.push(vec![
        span("recovers ", th.muted),
        span(proposal.recovers.clone(), th.success),
        span("   keeps ", th.muted),
        span(proposal.keeps.clone(), th.text),
        span("   cost ", th.muted),
        span(proposal.cost.clone(), th.text),
        span("   reversible ", th.muted),
        if proposal.reversible {
            span_strong(glyph::DONE, th.success, th)
        } else {
            span_strong(glyph::FAILED, th.error, th)
        },
    ]);
    body.push(Vec::new());

    let mut actions: Vec<Span<'static>> = Vec::new();
    for (index, (key, what)) in proposal.actions.iter().enumerate() {
        if index > 0 {
            actions.push(span("   ", th.border));
        }
        actions.push(span(format!("[{key}]"), th.primary));
        actions.push(span(format!(" {what}"), th.muted));
    }
    body.push(actions);

    Surface::new(width, th)
        .border(th.warning)
        .title(vec![span("GOVERNOR", th.warning)])
        .right(vec![span("1 proposal", th.border)])
        .rows(body)
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};
    use unicode_width::UnicodeWidthStr;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_screen(Screen::Mensura);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_role_is_a_row_of_tokens_meter_and_cap() {
        let m = model(120);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        for item in &m.budget {
            let row = drawn
                .iter()
                .find(|row| row.starts_with(&item.role))
                .unwrap_or_else(|| panic!("{} has no row", item.role));
            assert!(row.contains(&item.tokens), "{row}");
            assert!(row.contains('━') || row.contains('─'), "{row}");
            assert!(row.contains(&item.note), "{row}");
        }
    }

    #[test]
    fn the_breaching_row_is_copper_with_a_warning_note() {
        let m = model(120);
        let rows = lines(&m);
        let breach = rows
            .iter()
            .find(|row| text(row).starts_with("transcript"))
            .expect("the breaching row");
        assert!(text(breach).contains("soft cap"), "{:?}", text(breach));
        assert!(breach
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.primary)));
        assert!(breach
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.warning)));

        let within = rows
            .iter()
            .find(|row| text(row).starts_with("memoria"))
            .expect("a row within cap");
        assert!(within
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.secondary)));
    }

    #[test]
    fn the_proposal_always_states_recovers_keeps_cost_and_reversible() {
        let m = model(120);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        let row = drawn
            .iter()
            .find(|row| row.contains("recovers"))
            .expect("the proposal facts");
        for word in ["recovers", "keeps", "cost", "reversible"] {
            assert!(row.contains(word), "{word} missing from {row}");
        }
        assert!(
            row.contains('✓'),
            "reversibility reads without color: {row}"
        );
    }

    #[test]
    fn the_proposal_is_bordered_in_warning_and_offers_keyed_actions() {
        let m = model(120);
        let rows = lines(&m);
        let top = rows
            .iter()
            .find(|row| text(row).contains("GOVERNOR"))
            .expect("the governor block");
        assert_eq!(top.spans[0].style.fg, Some(m.theme.warning));
        assert!(text(top).contains("1 proposal"), "{:?}", text(top));

        let actions = rows
            .iter()
            .find(|row| text(row).contains("[a] apply"))
            .expect("keyed actions");
        for key in ["[a]", "[e]", "[p]", "[h]", "[d]"] {
            assert!(text(actions).contains(key), "{key} missing");
        }
    }

    #[test]
    fn the_governor_never_acts_silently() {
        let mut m = model(120);
        let with = lines(&m).len();
        m.proposal = None;
        let without = lines(&m).len();
        assert!(
            with > without,
            "a proposal must occupy rows of its own, not act in the background"
        );
    }

    #[test]
    fn the_summary_carries_the_attention_glyph() {
        let m = model(120);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("! transcript is 19%")),
            "{drawn:?}"
        );
    }

    #[test]
    fn spend_is_reported_against_its_cap_never_as_a_bare_number() {
        let drawn: Vec<String> = lines(&model(120)).iter().map(text).collect();
        let row = drawn
            .iter()
            .find(|row| row.contains("session spend"))
            .expect("the spend row");
        assert!(row.contains("daily cap"), "{row}");
        assert!(
            row.contains('◐') || row.contains('◑') || row.contains('◒') || row.contains('◓'),
            "{row}"
        );
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [80u16, 100, 120, 160] {
            let cap = width.min(MEASURE + 14);
            for row in lines(&model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= cap as usize,
                    "row wider than {cap} at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
