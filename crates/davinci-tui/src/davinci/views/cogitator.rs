//! `1f` — the model and provider picker. An overlay over a dimmed transcript;
//! it states its own exits in its footer (design.md §9). `catalog` is `3a`,
//! the full picker `/model` opens.
//!
//! `lines` mirrors `docs/ui/davinci_tui/lib/davinci/views/cogitator.ex`;
//! `catalog` mirrors artboard `3a` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::chrome::thousands;
use super::memoria::picker_row;
use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{CatalogRow, Credential, Model};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, column_header, footnote, pad, run_width, selection_bar, span, span_on,
    surface_rule, Surface,
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

/// Column widths of the catalog table (`3a`), as the artboard sets them.
/// The name column takes the slack.
const WINDOW: u16 = 6;
const THINKING: u16 = 8;
const PRICE: u16 = 15;
const CREDENTIAL: u16 = 15;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;

/// `3a` — the full model catalog, the screen `/model` opens: the same list as
/// the overlay with what each row costs you, and the rows you have no
/// credential for kept on screen with the ramp dropped rather than hidden, so
/// the catalog reads the same every time (design.md §2).
///
/// Mirrors artboard `3a` of `docs/ui/Pi TUI Instruments.dc.html`.
pub fn catalog(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let facts = &model.facts;

    // The filter box the sheet opens with, caret and all; what the filter
    // shows of the catalog sits at its right edge.
    let caret_style = if model.blink() {
        Style::default().bg(th.secondary).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };
    let count = if facts.catalog_total > 0 {
        vec![
            span(
                format!("{} of {} shown", facts.catalog_shown, facts.catalog_total),
                th.border,
            ),
            span(" · ", th.muted),
            span(
                format!(
                    "{} of {} providers ready",
                    facts.providers_ready, facts.providers_total
                ),
                th.border,
            ),
        ]
    } else {
        Vec::new()
    };
    let query = vec![
        span(format!("{} ", glyph::PROMPT), th.secondary),
        span("filter models…", th.muted),
        Span::styled(" ", caret_style),
    ];
    let inner = width.saturating_sub(4);
    let mut rows = Surface::new(width, th)
        .border(th.secondary)
        .row(crate::davinci::ui::spread(inner, query, count).spans)
        .lines();
    rows.push(blank());

    rows.extend(column_header(
        width,
        &[
            ("", LEAD - 1, false),
            ("PROVIDER / MODEL", 0, false),
            ("WINDOW", WINDOW, true),
            ("THINKING", THINKING, false),
            ("$/Mtok in · out", PRICE, true),
            ("CREDENTIAL", CREDENTIAL, false),
        ],
        th,
    ));

    if model.catalog.is_empty() {
        rows.push(Line::from(vec![span(
            "no models in the catalog — /login adds a provider",
            th.muted,
        )]));
    } else {
        let selected = model.catalog_index % model.catalog.len();
        for (index, entry) in model.catalog.iter().enumerate() {
            rows.push(catalog_row(entry, index == selected, width, th));
        }
    }

    rows.push(blank());
    rows.extend(footnote(
        width,
        vec![
            span("dimmed rows have no credential", th.muted),
            span(" · ", th.border),
            span("/login xai", th.text),
            span(" to add one", th.muted),
        ],
        vec![
            span("switching keeps the transcript", th.border),
            span(" · ", th.border),
            span("re-primes the cache", th.border),
        ],
        th,
    ));
    let mut refreshed = Vec::new();
    if !facts.catalog_refreshed.is_empty() {
        refreshed.push(span("catalog refreshed ", th.muted));
        refreshed.push(span(glyph::DONE, th.success));
        refreshed.push(span(format!(" {}", facts.catalog_refreshed), th.muted));
    }
    if !facts.catalog_path.is_empty() {
        if !refreshed.is_empty() {
            refreshed.push(span(" · ", th.border));
        }
        refreshed.push(span(facts.catalog_path.clone(), th.secondary));
    }
    let warning = window_warning(model)
        .map(|text| {
            vec![
                Span::styled(
                    format!("{} ", glyph::ATTENTION),
                    Style::default().fg(th.warning).add_modifier(th.emphasis),
                ),
                span(text, th.warning),
            ]
        })
        .unwrap_or_default();
    if !refreshed.is_empty() || !warning.is_empty() {
        rows.extend(footnote(width, refreshed, warning, th));
    }
    // A row that would push past the sheet is cut, never wrapped, so the
    // table keeps its shape on a narrow window.
    rows.into_iter()
        .map(|line| Line::from(crate::davinci::ui::truncate_run(line.spans, width)))
        .collect()
}

/// `! 47k of context will not fit a 32k window` — said when the catalog
/// holds a model whose window is smaller than what the session already
/// carries, so a switch to it is a known loss before it is made.
fn window_warning(model: &Model) -> Option<String> {
    let (used, _) = model.context;
    let smallest = model
        .catalog
        .iter()
        .filter_map(|row| parse_tokens(&row.window))
        .min()?;
    (smallest < used).then(|| {
        format!(
            "{} of context will not fit a {} window",
            thousands(used),
            thousands(smallest)
        )
    })
}

/// `200k` → 200 000, `1m` → 1 000 000. `None` for anything else.
fn parse_tokens(window: &str) -> Option<u64> {
    let trimmed = window.trim();
    let (number, unit) = trimmed.split_at(trimmed.len().checked_sub(1)?);
    let value: f64 = number.parse().ok()?;
    let scale = match unit {
        "k" => 1_000.0,
        "m" => 1_000_000.0,
        _ => return None,
    };
    Some((value * scale) as u64)
}

/// The sheet's frame (design.md §11): the ring size in the status bar, the
/// keys the sheet answers to, and no composer — the filter box is the input.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let ring = model.catalog.iter().filter(|row| row.ring).count();
    SheetChrome {
        header_right: Vec::new(),
        status_third: (ring > 0).then(|| vec![span(format!("ring of {ring}"), th.muted)]),
        status_right: None,
        hints: vec![
            hint(th, "↑↓ move"),
            hint(th, "enter select"),
            hint(th, "ctrl+p cycle ring"),
            hint(th, "s scope to ring"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
    }
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

fn catalog_row(entry: &CatalogRow, selected: bool, width: u16, th: &Theme) -> Line<'static> {
    let absent = entry.credential == Credential::Absent;
    let band = selected.then_some(th.surface);
    // A row with no credential drops its ramp rather than leaving the list.
    let ink = if absent { th.dim() } else { *th };

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
        ink.muted
    } else {
        th.text
    };

    let mut left = vec![
        selection_bar(selected, th),
        strong_on(
            format!("{} ", state.glyph()),
            th.state_color(state),
            band,
            th,
        ),
    ];
    let fixed = LEAD + WINDOW + 1 + THINKING + 1 + PRICE + 1 + CREDENTIAL;
    let name_column = width.saturating_sub(fixed).saturating_sub(1);
    // The ring and a local router are said in words after the name, as the
    // artboard writes them, in border ink so the name stays the loud part.
    let mut suffix = String::new();
    if entry.ring {
        suffix.push_str(" · in ctrl+p ring");
    }
    if !entry.detail.is_empty() {
        suffix.push_str(" · ");
        suffix.push_str(&entry.detail);
    }
    let name = clip_ellipsis(&entry.name, name_column);
    left.push(span_on(name.clone(), name_color, band));
    let used = run_width(&left) - LEAD;
    let room = name_column.saturating_sub(used);
    if !suffix.is_empty() && room > 0 {
        left.push(span_on(clip_ellipsis(&suffix, room), ink.border, band));
    }
    let gap = (LEAD + name_column + 1)
        .saturating_sub(run_width(&left))
        .max(1);
    left.push(pad(gap, band));

    let (credential_glyph, credential_color) = match entry.credential {
        Credential::Ready | Credential::Local => (glyph::DONE, ink.success),
        Credential::Pending => (glyph::ACTIVE, ink.primary),
        Credential::Expired => (glyph::ATTENTION, ink.warning),
        Credential::Absent => (glyph::QUEUED, ink.border),
    };
    let thinking = if entry.thinking == "none" {
        format!("{} none", glyph::QUEUED)
    } else {
        entry.thinking.clone()
    };
    let detail = if absent { ink.muted } else { th.muted };
    let figure = if absent { ink.muted } else { th.text };
    left.extend([
        span_on(
            format!("{:>w$} ", entry.window, w = WINDOW as usize),
            figure,
            band,
        ),
        span_on(
            format!("{:<w$} ", thinking, w = THINKING as usize),
            detail,
            band,
        ),
        span_on(
            format!("{:>w$} ", entry.price, w = PRICE as usize),
            figure,
            band,
        ),
        span_on(
            format!(
                "{:<w$}",
                format!("{credential_glyph} {}", entry.note),
                w = CREDENTIAL as usize
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
                detail: String::new(),
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
                detail: String::new(),
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
                detail: String::new(),
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
        // Dropped ramp: name in the dim ramp, credential in queued glyph.
        assert!(absent
            .spans
            .iter()
            .any(|span| span.content.as_ref().contains("xai / grok")
                && span.style.fg == Some(m.theme.dim().muted)));
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
        // Selection bar and glyph at the head, the ring said in words after
        // the name, as the artboard writes it.
        assert!(selected.starts_with("▌  ◉"), "{selected}");
        assert!(selected.contains("· in ctrl+p ring"), "{selected}");
        // The legend explains the dimmed rows and the way in.
        assert!(drawn
            .iter()
            .any(|row| row.contains("dimmed rows have no credential")));
        assert!(drawn.iter().any(|row| row.contains("/login")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "3a");
        let c = chrome(&m);
        assert!(c.header_right.is_empty(), "3a keeps cwd │ branch │ model");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "ring of 3");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Hidden);
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("↑↓ move │ enter select │ ctrl+p cycle ring │ s scope to ring"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }

    #[test]
    fn the_catalog_opens_on_its_filter_box_and_states_what_it_shows() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "3a");
        let drawn: Vec<String> = catalog(&m).iter().map(text).collect();
        assert!(drawn[1].contains("filter models…"), "{}", drawn[1]);
        assert!(
            drawn[1].contains("12 of 63 shown · 6 of 10 providers ready"),
            "{}",
            drawn[1]
        );
        assert!(drawn
            .iter()
            .any(|row| row.contains("PROVIDER / MODEL") && row.contains("$/Mtok in · out")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("qwen-coder · router :8080")));
        assert!(drawn.iter().any(|row| row.contains("! token expired")));
        assert!(drawn.iter().any(|row| row.contains("○ no credential")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("catalog refreshed ✓ 2h ago")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("47k of context will not fit a 32k window")));
        // No hint line of its own: the frame draws the hint row.
        assert!(!drawn.iter().any(|row| row.contains("esc close")));
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
        use unicode_width::UnicodeWidthStr;
        for width in [72u16, 80, 100, 120, 160] {
            let cap = width;
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
