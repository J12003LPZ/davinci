//! AppShell: header, composer, status bar (design.md §6, screen `1b`).
//!
//! Header and status bar are one line each at every width; both abbreviate
//! rather than wrap. The composer is the loudest element on screen: copper
//! rule, `›` prompt, blinking block caret, keybind hints below it in border
//! color.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/chrome.ex`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, Screen};
use crate::davinci::theme::glyph;
use crate::davinci::ui::{clip, indent, meter, run_width, span, span_strong, spread, Surface};

/// Which hints sit under the composer. Every panel states its own exits (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    Default,
    Closable,
    Recall,
    Multiline,
    None,
}

/// `D davinci · agent` on the left, `path │ branch │ model` on the right.
pub fn header(model: &Model) -> Line<'static> {
    let th = &model.theme;
    let left = vec![
        span_strong("D", th.primary, th),
        span(" davinci", th.text),
        span(" · ", th.border),
        span(model.mode(), th.primary),
    ];

    let right = if model.minimal() {
        vec![
            span(short_cwd(&model.cwd), th.muted),
            span(" │ ", th.border),
            span(model.branch.clone(), th.secondary),
        ]
    } else {
        let mut right = vec![
            span(model.cwd.clone(), th.muted),
            span(" │ ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" │ ", th.border),
            span(model.model_name.clone(), th.muted),
        ];
        if model.codex_open() {
            right.push(span(" │ ", th.border));
            right.push(span(format!("{}×{}", model.width, model.height), th.border));
        }
        right
    };

    spread(model.width, left, right)
}

/// `mode · branch · Δn +a -d` on the left, a context meter on the right.
pub fn status(model: &Model) -> Line<'static> {
    spread(model.width, status_left(model), status_right(model))
}

fn status_left(model: &Model) -> Vec<Span<'static>> {
    let th = &model.theme;
    let (delta, adds, dels) = model.changes;

    match model.screen {
        Screen::Grafo => vec![
            span("grafo", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span("impact view", th.muted),
        ],
        Screen::Memoria => vec![
            span("memoria", th.primary),
            span(" · ", th.border),
            span("recall", th.muted),
        ],
        Screen::Mensura => vec![
            span("mensura", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
        ],
        Screen::Plan => vec![
            span("plan", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
        ],
        Screen::Agent if model.minimal() => vec![
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{}{delta}", glyph::DELTA), th.primary),
        ],
        Screen::Agent => {
            let mut left = vec![
                span(model.mode(), th.primary),
                span(" · ", th.border),
                span(model.branch.clone(), th.secondary),
                span(" · ", th.border),
                span(format!("{}{delta} ", glyph::DELTA), th.primary),
                span(format!("+{adds} "), th.success),
                span(format!("-{dels}"), th.error),
            ];
            if model.codex_open() {
                left.push(span(" · ", th.border));
                left.push(span("codex open", th.muted));
            }
            left
        }
    }
}

fn status_right(model: &Model) -> Vec<Span<'static>> {
    let th = &model.theme;
    let fraction = model.context_fraction();
    // Truncated, not rounded: a meter must never claim a cap has been reached
    // before it has (47k of 200k reads 23%, screen `1g`).
    let percent = (fraction * 100.0) as u32;

    if model.screen == Screen::Grafo {
        return vec![
            span("enter open node", th.border),
            span(" · ", th.border),
            span("x expand", th.border),
            span(" · ", th.border),
            span("esc close", th.border),
        ];
    }

    if model.minimal() {
        // Still a meter, never a bare number (design.md §6, §9).
        return vec![
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("^p", th.border),
        ];
    }

    if model.narrow() {
        return vec![
            span("mensura ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("^p", th.border),
        ];
    }

    let (used, cap) = model.context;
    let mut right = vec![span("context ", th.muted)];
    right.extend(meter(fraction, 12, th, None));
    right.push(span(
        format!(" {}/{}", thousands(used), thousands(cap)),
        th.muted,
    ));
    right
}

/// Rows for the composer plus its hint row. Grows with content.
pub fn composer(model: &Model, lines: Option<&[String]>, hint: Hint) -> Vec<Line<'static>> {
    composer_with(model, lines, hint, "ask davinci…", 0)
}

/// The composer, drawn against an arbitrary theme and inset — used when it sits
/// under an instrument rather than under the transcript.
pub fn composer_with(
    model: &Model,
    lines: Option<&[String]>,
    hint: Hint,
    placeholder: &str,
    inset: u16,
) -> Vec<Line<'static>> {
    let th = &model.theme;
    let owned = lines.map(<[String]>::to_vec);
    // A composer holding newlines is drawn as the rows the user typed; an
    // empty one is still one row, so the box never collapses.
    let entries: Vec<String> =
        owned.unwrap_or_else(|| model.composer.split('\n').map(str::to_string).collect());
    let last = entries.len().saturating_sub(1);
    let caret_style = if model.blink() {
        Style::default().bg(th.primary).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };

    let mut surface = Surface::new(model.width, th)
        .border(th.primary)
        .inset(inset);
    for (index, entry) in entries.into_iter().enumerate() {
        let body = if entry.is_empty() {
            span(placeholder.to_string(), th.muted)
        } else {
            span(clip(&entry, model.width.saturating_sub(10)), th.text)
        };
        let mut row = vec![span(format!("{} ", glyph::PROMPT), th.primary), body];
        if index == last {
            row.push(Span::styled(" ", caret_style));
        }
        surface = surface.row(row);
    }

    let mut rows = surface.lines();
    rows.push(hint_line(model, hint, inset));
    rows
}

/// How many rows [`composer`] will occupy, known before it is built.
pub fn composer_height(lines: Option<&[String]>) -> u16 {
    let entries = lines.map(<[String]>::len).unwrap_or(1).max(1);
    // surface top + entries + surface bottom + hint row
    entries as u16 + 3
}

fn hint_line(model: &Model, hint: Hint, inset: u16) -> Line<'static> {
    let th = &model.theme;
    let dot = || span(" · ", th.border);
    let spans = match hint {
        Hint::None => Vec::new(),
        Hint::Closable => vec![
            span("enter run", th.border),
            dot(),
            span("esc close", th.border),
        ],
        // Hints abbreviate rather than wrap; the exit is the part that is
        // never dropped (design.md §7, §9).
        Hint::Recall if model.minimal() => vec![
            span("enter pin", th.border),
            dot(),
            span("r reindex", th.border),
            dot(),
            span("esc close", th.border),
        ],
        Hint::Recall => vec![
            span("enter pin to context", th.border),
            dot(),
            span("f raise floor", th.border),
            dot(),
            span("r reindex", th.border),
            dot(),
            span("esc close", th.border),
        ],
        Hint::Multiline if model.minimal() => vec![
            span("enter send", th.border),
            dot(),
            span("esc cancel", th.border),
        ],
        Hint::Multiline => vec![
            span("shift+enter newline", th.border),
            dot(),
            span("enter send", th.border),
            dot(),
            span("esc cancel", th.border),
        ],
        Hint::Default if model.minimal() => {
            vec![span("enter send · esc cancel", th.border)]
        }
        Hint::Default => vec![
            span("enter send", th.border),
            dot(),
            span("shift+enter newline", th.border),
            dot(),
            span("tab complete", th.border),
            dot(),
            span("esc cancel", th.border),
        ],
    };
    indent(inset + 2, spans)
}

/// `47000` → `47k`. Numbers carry their unit (design.md §9).
pub fn thousands(value: u64) -> String {
    if value >= 1_000_000 {
        let millions = value as f64 / 1_000_000.0;
        if millions >= 10.0 {
            return format!("{}m", millions.round() as u64);
        }
        return format!("{millions:.1}m");
    }
    if value >= 1_000 {
        return format!("{}k", (value as f64 / 1_000.0).round() as u64);
    }
    value.to_string()
}

/// Paths shorten to crate-relative below 80 columns (design.md §7).
fn short_cwd(cwd: &str) -> String {
    cwd.rsplit(['/', '\\'])
        .next()
        .filter(|tail| !tail.is_empty())
        .unwrap_or(cwd)
        .to_string()
}

/// Width of a row, for tests and for the responsive audit.
pub fn line_width(line: &Line<'_>) -> u16 {
    run_width(&line.spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Overlay;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.cwd = "C:\\dev\\oss\\davinci-rust".into();
        model.branch = "main".into();
        model.model_name = "sonnet".into();
        model.changes = (3, 42, 11);
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
    fn the_header_is_one_line_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let line = header(&model(width));
            assert_eq!(line_width(&line), width, "header at {width}");
        }
    }

    #[test]
    fn the_status_bar_is_one_line_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let line = status(&model(width));
            assert_eq!(line_width(&line), width, "status bar at {width}");
        }
    }

    #[test]
    fn the_header_carries_path_branch_and_model_when_there_is_room() {
        let drawn = text(&header(&model(100)));
        assert!(drawn.starts_with("D davinci · agent"), "{drawn}");
        assert!(drawn.contains("C:\\dev\\oss\\davinci-rust │ main │ sonnet"));
    }

    #[test]
    fn the_header_shortens_the_path_below_eighty_columns() {
        let drawn = text(&header(&model(72)));
        assert!(drawn.contains("davinci-rust │ main"), "{drawn}");
        assert!(!drawn.contains("C:\\dev"), "{drawn}");
        assert!(!drawn.contains("sonnet"), "{drawn}");
    }

    #[test]
    fn the_header_reports_the_window_size_only_when_codex_is_open() {
        let mut m = model(160);
        assert!(!text(&header(&m)).contains("160×44"));
        m.toggle_codex();
        assert!(text(&header(&m)).contains("160×44"));
    }

    #[test]
    fn the_status_bar_shows_the_context_as_a_meter_with_its_cap() {
        let drawn = text(&status(&model(100)));
        assert!(drawn.contains("context "), "{drawn}");
        assert!(drawn.contains("47k/200k"), "{drawn}");
        assert!(drawn.contains('╸'), "the meter has a tip: {drawn}");
    }

    #[test]
    fn a_narrow_status_bar_is_still_a_meter_never_a_bare_number() {
        for width in [72u16, 90] {
            let drawn = text(&status(&model(width)));
            assert!(drawn.contains("23%"), "{drawn}");
            assert!(
                drawn.contains('◐')
                    || drawn.contains('◑')
                    || drawn.contains('◒')
                    || drawn.contains('◓'),
                "no pie glyph at {width}: {drawn}"
            );
            assert!(drawn.contains("^p"), "{drawn}");
        }
    }

    #[test]
    fn the_status_bar_left_names_the_screen_in_hand() {
        let mut m = model(120);
        assert!(text(&status(&m)).starts_with("agent · main · Δ3 +42 -11"));
        m.toggle_screen(Screen::Grafo);
        let drawn = text(&status(&m));
        assert!(drawn.starts_with("grafo · main · impact view"), "{drawn}");
        assert!(drawn.contains("esc close"), "grafo states its exits");
    }

    #[test]
    fn the_composer_is_a_copper_surface_with_a_prompt_and_a_hint_row() {
        let m = model(100);
        let rows = composer(&m, None, Hint::Default);
        assert_eq!(rows.len() as u16, composer_height(None));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].spans[0].style.fg, Some(m.theme.primary));
        assert!(text(&rows[1]).contains("› ask davinci…"));
        assert!(text(&rows[3]).contains("enter send · shift+enter newline"));
    }

    #[test]
    fn the_composer_grows_with_its_content() {
        let m = model(100);
        let entered = vec![
            "keep step IV, but generate the fixtures from the".to_string(),
            "existing TS golden files under tests\\golden\\".to_string(),
        ];
        let rows = composer(&m, Some(&entered), Hint::Multiline);
        assert_eq!(rows.len(), 5);
        assert_eq!(composer_height(Some(&entered)), 5);
        assert!(text(&rows[1]).contains("keep step IV"));
        assert!(text(&rows[2]).contains("existing TS golden files"));
    }

    #[test]
    fn the_caret_is_drawn_on_the_last_row_only_and_blinks() {
        let mut m = model(100);
        let entered = vec!["one".to_string(), "two".to_string()];
        let rows = composer(&m, Some(&entered), Hint::Multiline);
        let caret = |line: &Line<'_>| {
            line.spans
                .iter()
                .any(|span| span.style.bg == Some(m.theme.primary))
        };
        assert!(!caret(&rows[1]), "no caret on the first row");
        assert!(caret(&rows[2]), "caret on the last row");

        m.tick = 4;
        let off = composer(&m, Some(&entered), Hint::Multiline);
        assert!(!caret(&off[2]), "the caret blinks off");
    }

    #[test]
    fn the_hint_row_abbreviates_below_eighty_columns() {
        let drawn = text(&composer(&model(72), None, Hint::Default)[3]);
        assert!(drawn.contains("enter send · esc cancel"), "{drawn}");
        assert!(!drawn.contains("tab complete"), "{drawn}");
    }

    #[test]
    fn every_hint_states_the_exit() {
        let m = model(100);
        for hint in [Hint::Default, Hint::Closable, Hint::Recall, Hint::Multiline] {
            let rows = composer(&m, None, hint);
            let drawn = text(rows.last().unwrap());
            assert!(drawn.contains("esc"), "{hint:?} does not state its exit");
        }
    }

    #[test]
    fn the_palette_query_does_not_reach_the_composer_row() {
        let mut m = model(100);
        m.toggle_overlay(Overlay::Instrumenta);
        m.type_char("git");
        let drawn = text(&composer(&m, None, Hint::Default)[1]);
        assert!(drawn.contains("ask davinci…"), "{drawn}");
    }

    #[test]
    fn numbers_carry_their_unit() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(47_000), "47k");
        assert_eq!(thousands(128_400), "128k");
        assert_eq!(thousands(200_000), "200k");
        assert_eq!(thousands(1_200_000), "1.2m");
        assert_eq!(thousands(18_402_000), "18m");
    }
}
