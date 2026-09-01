//! `1f` — the model and provider picker. An overlay over a dimmed transcript;
//! it states its own exits in its footer (design.md §9).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/cogitator.ex`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::memoria::picker_row;
use crate::davinci::model::{CatalogRow, Credential, Model};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, pad, run_width, span, span_on, surface_rule, Surface, MEASURE,
};

/// The picker is a narrow card, as the mockup draws it beside Memoria (`1f`);
/// alone, it floats centred over the transcript. Wide enough that
/// `configured in %USERPROFILE%\.pi\config.json` is never clipped.
const CARD_WIDTH: u16 = 50;

pub fn lines(model: &Model, config_path: &str) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let inset = if model.bare() {
        0
    } else {
        width.saturating_sub(CARD_WIDTH.min(width)) / 2
    };
    let inner = width.saturating_sub(inset * 2).saturating_sub(4);
    let selected = model.selection(model.models.len());

    let mut body: Vec<Vec<Span<'static>>> = model
        .models
        .iter()
        .enumerate()
        .map(|(index, item)| {
            picker_row(th, inner, &item.name, &item.window, Some(index) == selected)
        })
        .collect();

    if body.is_empty() {
        // No catalog reached this session — offline, or no credentials. Say
        // which, rather than showing a picker with nothing to pick.
        body.push(vec![span(
            "no models available — /login to add a provider",
            th.muted,
        )]);
    }

    body.push(surface_rule(width.saturating_sub(inset * 2), th));
    body.push(vec![
        span("configured in ", th.muted),
        span(config_path.to_string(), th.secondary),
    ]);
    body.push(vec![
        span("↑↓ move", th.border),
        span(" │ ", th.border),
        span("enter select", th.border),
    ]);

    Surface::new(width, th)
        .inset(inset)
        .border(th.secondary)
        .title(vec![
            span("COGITATOR", th.secondary),
            span(" · ", th.border),
            span("MODEL", th.muted),
        ])
        .rows(body)
        .lines()
}

/// Column widths of the catalog table (`3a`), as the Elixir reference sets
/// them.
const NAME: u16 = 30;
const WINDOW: usize = 6;
const THINKING: usize = 8;
const PRICE: usize = 14;
const CREDENTIAL: usize = 15;

/// `3a` — the full model catalog, the screen `/model` opens: the same list as
/// the overlay with what each row costs you, and the rows you have no
/// credential for kept on screen with the ramp dropped rather than hidden, so
/// the catalog reads the same every time (design.md §2).
pub fn catalog(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);

    // The filter box the sheet opens with, caret and all.
    let caret_style = if model.blink() {
        Style::default().bg(th.secondary).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };
    let mut rows = Surface::new(width, th)
        .border(th.secondary)
        .row(vec![
            span(format!("{} ", glyph::PROMPT), th.secondary),
            span("filter models…", th.muted),
            Span::styled(" ", caret_style),
        ])
        .lines();
    rows.push(blank());

    rows.push(Line::from(vec![
        pad(2, None),
        span(format!("{:<31}", "PROVIDER / MODEL"), th.border),
        span(format!("{:>WINDOW$} ", "WINDOW"), th.border),
        span(format!("{:<w$}", "THINKING", w = THINKING + 1), th.border),
        span(format!("{:>PRICE$} ", "$/Mtok"), th.border),
        span("CREDENTIAL", th.border),
    ]));

    if model.catalog.is_empty() {
        rows.push(Line::from(vec![span(
            "no models in the catalog — /login adds a provider",
            th.muted,
        )]));
    } else {
        let selected = model.catalog_index % model.catalog.len();
        // A real catalog runs to hundreds of rows and the sheet is
        // tail-anchored: show a window around the selection and count the
        // rest, so the selection is always on screen.
        const WINDOW: usize = 16;
        let total = model.catalog.len();
        let start = selected
            .saturating_sub(WINDOW / 2)
            .min(total.saturating_sub(WINDOW));
        let end = (start + WINDOW).min(total);
        if start > 0 {
            rows.push(Line::from(vec![span(
                format!("… {start} above"),
                th.border,
            )]));
        }
        for (index, entry) in model.catalog.iter().enumerate() {
            if index < start || index >= end {
                continue;
            }
            rows.push(catalog_row(entry, index == selected, th));
        }
        if end < total {
            rows.push(Line::from(vec![span(
                format!("… {} more below", total - end),
                th.border,
            )]));
        }
    }

    rows.push(blank());
    rows.push(Line::from(vec![
        span(format!("{} ", glyph::ACTIVE), th.border),
        span("is the ctrl+p ring", th.muted),
        span(" · ", th.border),
        span("dimmed rows have no credential", th.muted),
        span(" · ", th.border),
        span("/login xai", th.text),
        span(" adds one", th.muted),
    ]));
    rows.push(Line::from(vec![span(
        "switching keeps the transcript and re-primes the cache",
        th.muted,
    )]));
    rows.push(Line::from(vec![
        Span::styled(
            format!("{} ", glyph::ATTENTION),
            Style::default().fg(th.warning).add_modifier(th.emphasis),
        ),
        span("a large context will not fit a small window", th.muted),
    ]));
    rows.push(Line::from(vec![
        span("↑↓ move", th.border),
        span(" · ", th.border),
        span("enter select", th.border),
        span(" · ", th.border),
        span("s scope to ring", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));
    // A row that would push past the sheet is cut, never wrapped, so the
    // table keeps its shape on a narrow window.
    rows.into_iter()
        .map(|line| Line::from(crate::davinci::ui::truncate_run(line.spans, width)))
        .collect()
}

/// A glyph in the theme's emphasis, on the selection band when there is one.
fn strong_on(
    content: String,
    color: ratatui::style::Color,
    band: Option<ratatui::style::Color>,
    th: &Theme,
) -> Span<'static> {
    let mut style = Style::default().fg(color).add_modifier(th.emphasis);
    if let Some(band) = band {
        style = style.bg(band);
    }
    Span::styled(content, style)
}

fn catalog_row(entry: &CatalogRow, selected: bool, th: &Theme) -> Line<'static> {
    let absent = entry.credential == Credential::Absent;
    let band = selected.then_some(th.surface);

    let state = if selected {
        State::Active
    } else if entry.credential == Credential::Expired {
        State::Attention
    } else {
        State::Queued
    };
    let name_color = if selected {
        th.text
    } else if absent {
        th.border
    } else {
        th.muted
    };
    let detail = if absent { th.border } else { th.muted };

    // The ring is a second mark on the row rather than a suffix on the name:
    // trimming a long name to fit a label made two rows read identically.
    let ring = if entry.ring { glyph::ACTIVE } else { " " };

    let mut left = vec![
        strong_on(
            format!("{} ", state.glyph()),
            th.state_color(state),
            band,
            th,
        ),
        span_on(clip_ellipsis(&entry.name, NAME - 2), name_color, band),
        span_on(format!(" {ring}"), th.border, band),
    ];
    let gap = (2 + NAME + 1).saturating_sub(run_width(&left)).max(1);
    left.push(pad(gap, band));

    let (credential_glyph, credential_color) = match entry.credential {
        Credential::Ready | Credential::Local => (glyph::DONE, th.success),
        Credential::Pending => (glyph::ACTIVE, th.primary),
        Credential::Expired => (glyph::ATTENTION, th.warning),
        Credential::Absent => (glyph::QUEUED, th.border),
    };
    left.extend([
        span_on(format!("{:>WINDOW$} ", entry.window), detail, band),
        span_on(
            format!("{:<w$}", entry.thinking, w = THINKING + 1),
            detail,
            band,
        ),
        span_on(format!("{:>PRICE$} ", entry.price), detail, band),
        span_on(
            format!(
                "{:<CREDENTIAL$}",
                format!("{credential_glyph} {}", entry.note)
            ),
            credential_color,
            band,
        ),
    ]);
    Line::from(left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Overlay;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_overlay(Overlay::Cogitator);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_session_with_no_catalog_is_told_how_to_get_one() {
        let mut m = model(100);
        m.models.clear();
        let rows: Vec<String> = lines(&m, "config.json").iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no models available")));
        assert!(rows.iter().any(|row| row.contains("/login")));
    }

    #[test]
    fn the_picker_is_a_narrow_verdigris_card_naming_where_it_is_configured() {
        let m = model(100);
        let rows = lines(&m, "%USERPROFILE%\\.pi\\config.json");
        let top = text(&rows[0]);
        assert!(top.contains("╭─ COGITATOR · MODEL ─"), "{top}");
        // The card is narrow and centred, as the mockup draws it (`1f`).
        assert!(
            top.trim().chars().count() < 60,
            "the card should be narrow: {top}"
        );
        assert_eq!(rows[0].spans[1].style.fg, Some(m.theme.secondary));
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("configured in %USERPROFILE%\\.pi\\config.json")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("↑↓ move │ enter select")));
    }

    #[test]
    fn every_model_row_carries_its_context_window() {
        let m = model(100);
        let rows: Vec<String> = lines(&m, "config.toml").iter().map(text).collect();
        for item in &m.models {
            assert!(
                rows.iter()
                    .any(|row| row.contains(&item.name) && row.contains(&item.window)),
                "{} has no window",
                item.name
            );
        }
    }

    #[test]
    fn the_model_in_hand_is_marked_with_a_glyph() {
        let m = model(100);
        let rows = lines(&m, "config.toml");
        assert!(text(&rows[1]).contains("◉ "));
        assert!(text(&rows[2]).contains("○ "));
    }

    #[test]
    fn the_picker_is_row_exact_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m, "config.toml") {
                assert_eq!(run_width(&row.spans), width, "at {width}");
            }
        }
    }

    // --- 3a: the full catalog ------------------------------------------------

    fn catalog_model(width: u16) -> Model {
        use crate::davinci::model::{CatalogRow, Credential, Screen};
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        m.catalog = vec![
            CatalogRow {
                name: "anthropic / claude-sonnet".into(),
                window: "200k".into(),
                thinking: "budget".into(),
                price: "3.00 · 15.00".into(),
                credential: Credential::Ready,
                note: "oauth".into(),
                ring: true,
                provider: "anthropic".into(),
                id: "claude-sonnet".into(),
            },
            CatalogRow {
                name: "github-copilot / gpt".into(),
                window: "128k".into(),
                thinking: "effort".into(),
                price: "seat".into(),
                credential: Credential::Expired,
                note: "expired".into(),
                ring: false,
                provider: "github-copilot".into(),
                id: "gpt".into(),
            },
            CatalogRow {
                name: "xai / grok".into(),
                window: "256k".into(),
                thinking: "effort".into(),
                price: "3.00 · 15.00".into(),
                credential: Credential::Absent,
                note: "none".into(),
                ring: false,
                provider: "xai".into(),
                id: "grok".into(),
            },
        ];
        m.toggle_screen(Screen::Models);
        m
    }

    #[test]
    fn the_catalog_lists_every_row_with_window_price_and_credential() {
        let m = catalog_model(100);
        let drawn: Vec<String> = catalog(&m).iter().map(text).collect();
        for (name, window, note) in [
            ("anthropic / claude-sonnet", "200k", "oauth"),
            ("github-copilot / gpt", "128k", "expired"),
            ("xai / grok", "256k", "none"),
        ] {
            assert!(
                drawn
                    .iter()
                    .any(|row| row.contains(name) && row.contains(window) && row.contains(note)),
                "{name} row incomplete: {drawn:?}"
            );
        }
    }

    #[test]
    fn rows_without_a_credential_stay_listed_dimmed_rather_than_hidden() {
        let m = catalog_model(100);
        let rows = catalog(&m);
        let absent = rows
            .iter()
            .find(|row| text(row).contains("xai / grok"))
            .expect("the absent row stays on screen");
        // Dropped ramp: name in border ink, credential in queued glyph.
        assert!(absent
            .spans
            .iter()
            .any(|span| span.content.as_ref().contains("xai / grok")
                && span.style.fg == Some(m.theme.border)));
        assert!(text(absent).contains('○'));
        let expired = rows
            .iter()
            .find(|row| text(row).contains("github-copilot"))
            .expect("the expired row");
        assert!(text(expired).contains('!'));
    }

    #[test]
    fn the_selected_row_and_the_ring_are_both_marked() {
        let m = catalog_model(100);
        let drawn: Vec<String> = catalog(&m).iter().map(text).collect();
        let selected = drawn
            .iter()
            .find(|row| row.contains("anthropic / claude-sonnet"))
            .unwrap();
        // Selection glyph at the head, ring mark after the name.
        assert!(selected.starts_with('◉'), "{selected}");
        assert!(
            selected.matches('◉').count() >= 2,
            "the ring is a second mark: {selected}"
        );
        // The legend explains both marks and the way in.
        assert!(drawn.iter().any(|row| row.contains("is the ctrl+p ring")));
        assert!(drawn.iter().any(|row| row.contains("/login")));
    }

    #[test]
    fn an_empty_catalog_says_how_to_fill_it() {
        let mut m = catalog_model(100);
        m.catalog.clear();
        let drawn: Vec<String> = catalog(&m).iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("no models in the catalog")));
    }

    #[test]
    fn the_catalog_never_overflows_its_sheet() {
        use crate::davinci::ui::MEASURE;
        use unicode_width::UnicodeWidthStr;
        for width in [72u16, 80, 100, 120, 160] {
            let cap = width.min(MEASURE + 14);
            for row in catalog(&catalog_model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= cap as usize,
                    "row wider than {cap} at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
