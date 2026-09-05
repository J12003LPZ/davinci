//! `5c` — `governor-status`. What the governor did to your tool output, which
//! is a different question from `2c`'s "where is the budget going".
//!
//! Counters carry their denominator, and the screen shows one compressed
//! result in full so the elision marker and the retrieval id are visible
//! rather than described: nothing is deleted, the tail is on disk, and the
//! model can ask for any range of it back.
//!
//! Mirrors artboard `5c` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Color;
use ratatui::text::Line;

use super::sheet::{facts, hint, Composer, SheetChrome};
use crate::davinci::model::{Model, Tone};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{
    blank, clip_ellipsis, column_header, footnote, indent, pad, run_width, span, span_strong,
    spread, truncate_run, wrap, Surface,
};

/// A counter tile narrower than this cannot hold its figure and its reason;
/// below it the counters stack as two rows each.
const MIN_TILE: u16 = 18;

const ID: u16 = 12;
const TOOL: u16 = 12;
const SIZE: u16 = 18;

fn tone_color(tone: Tone, theme: &Theme) -> Color {
    match tone {
        Tone::Primary => theme.primary,
        Tone::Secondary => theme.secondary,
        Tone::Warning => theme.warning,
        Tone::Success => theme.success,
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(status) = model.governor.as_ref() else {
        return vec![Line::from(vec![span(
            "the governor has nothing to report — no tool output was compressed",
            th.muted,
        )])];
    };

    // The artboard sets the counters side by side, one bordered tile each:
    // the figure with its denominator, what was done, and why. A terminal
    // too narrow for four tiles stacks them as two rows each instead.
    let counters = counter_tiles(status, width, th).unwrap_or_else(|| counter_rows(status, th));

    let latest = status.stored.iter().find(|entry| !entry.stale);
    let sample_rows = if let Some(entry) = latest {
        vec![
            vec![
                span_strong(entry.tool.clone(), th.text, th),
                span(format!("  {}", entry.call), th.muted),
            ],
            vec![
                span("Original  ", th.muted),
                span(entry.size.clone(), th.text),
            ],
            vec![
                span("Saved as  ", th.muted),
                span(entry.id.clone(), th.secondary),
            ],
            vec![span(
                "Ask the agent to retrieve this output by ID, with an optional line range.",
                th.muted,
            )],
        ]
    } else {
        vec![
            vec![span("No outputs compressed in this session yet.", th.muted)],
            vec![span(
                "Large results appear here after their originals are saved.",
                th.muted,
            )],
        ]
    };
    let sample = Surface::new(width, th)
        .title(vec![span_strong("LATEST SAVED OUTPUT", th.primary, th)])
        .rows(sample_rows)
        .lines();

    let held = footnote(
        width,
        vec![span("held on disk, retrievable by id", th.text)],
        if status.outputs_note.is_empty() {
            Vec::new()
        } else {
            vec![span(status.outputs_note.clone(), th.border)]
        },
        th,
    );
    let header = column_header(
        width,
        &[
            ("", ID, false),
            ("TOOL", TOOL, false),
            ("CALL", 0, false),
            ("LINES · SIZE", SIZE, true),
        ],
        th,
    );

    let stored: Vec<Line<'static>> = status
        .stored
        .iter()
        .map(|entry| {
            let color = if entry.stale { th.border } else { th.muted };
            let left = vec![
                span(
                    format!("{:<w$} ", entry.id, w = ID as usize),
                    if entry.stale { th.border } else { th.warning },
                ),
                span(format!("{:<w$} ", entry.tool, w = TOOL as usize), color),
            ];
            let right = vec![span(entry.size.clone(), th.border)];
            let room = width
                .saturating_sub(run_width(&left))
                .saturating_sub(run_width(&right))
                .saturating_sub(1);
            let mut spans = left;
            spans.push(span(clip_ellipsis(&entry.call, room), color));
            spread(width, spans, right)
        })
        .collect();

    // The thresholds the governor runs with, from its status: the sheet
    // never describes a policy the process does not have.
    let mut footer: Vec<Line<'static>> = Vec::new();
    if !status.policy.is_empty() {
        let (above, keeps) = status
            .policy
            .split_once(" · ")
            .unwrap_or((status.policy.as_str(), ""));
        let mut spans = vec![span(above.to_string(), th.muted)];
        if !keeps.is_empty() {
            spans.push(span(" · ", th.border));
            spans.push(span(keeps.to_string(), th.border));
        }
        footer.push(Line::from(spans));
    }
    for row in wrap(
        "nothing is deleted — the full output is on disk and the model can ask \
         for any range of it",
        width,
    ) {
        footer.push(Line::from(vec![span(row, th.muted)]));
    }
    footer.extend(footnote(
        width,
        vec![span(status.store_dir.clone(), th.secondary)],
        vec![span(
            "governor-reset clears the counters, not the store",
            th.border,
        )],
        th,
    ));

    let mut out = counters;
    out.push(blank());
    out.extend(sample);
    out.push(blank());
    out.extend(held);
    out.extend(header);
    out.extend(stored);
    out.push(blank());
    out.extend(footer);
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The counters as one row of bordered tiles, `None` when the width cannot
/// hold them.
fn counter_tiles(
    status: &crate::davinci::model::GovernorSheet,
    width: u16,
    th: &Theme,
) -> Option<Vec<Line<'static>>> {
    let count = status.counters.len() as u16;
    if count == 0 {
        return Some(Vec::new());
    }
    let gap = 1u16;
    let tile = width.saturating_sub(gap * (count - 1)) / count;
    if tile < MIN_TILE {
        return None;
    }
    // Every tile has the same number of rows, so the bottom rules align.
    let inner = tile.saturating_sub(4);
    let mut bodies: Vec<Vec<Vec<ratatui::text::Span<'static>>>> = status
        .counters
        .iter()
        .map(|counter| {
            let mut rows = vec![
                vec![
                    span_strong(counter.number.clone(), tone_color(counter.tone, th), th),
                    span(format!(" {}", counter.of), th.muted),
                ],
                vec![span(counter.verb.clone(), th.text)],
            ];
            rows.extend(
                wrap(&counter.note, inner)
                    .into_iter()
                    .map(|row| vec![span(row, th.border)]),
            );
            rows
        })
        .collect();
    let rows = bodies.iter().map(Vec::len).max().unwrap_or(0);
    for body in &mut bodies {
        while body.len() < rows {
            body.push(Vec::new());
        }
    }
    let tiles: Vec<Vec<Line<'static>>> = bodies
        .into_iter()
        .map(|body| Surface::new(tile, th).rows(body).lines())
        .collect();
    let height = tiles.iter().map(Vec::len).max().unwrap_or(0);
    Some(
        (0..height)
            .map(|row| {
                let mut spans = Vec::new();
                for (index, lines) in tiles.iter().enumerate() {
                    if index > 0 {
                        spans.push(pad(gap, None));
                    }
                    match lines.get(row) {
                        Some(line) => {
                            let used = run_width(&line.spans);
                            spans.extend(line.spans.iter().cloned());
                            if used < tile {
                                spans.push(pad(tile - used, None));
                            }
                        }
                        None => spans.push(pad(tile, None)),
                    }
                }
                Line::from(spans)
            })
            .collect(),
    )
}

/// The counters as two rows each: the figure with its denominator, then
/// what was done and why.
fn counter_rows(status: &crate::davinci::model::GovernorSheet, th: &Theme) -> Vec<Line<'static>> {
    let mut counters: Vec<Line<'static>> = Vec::new();
    for counter in &status.counters {
        counters.push(Line::from(vec![
            span(counter.number.clone(), tone_color(counter.tone, th)),
            span(format!(" {}", counter.of), th.muted),
        ]));
        counters.push(indent(
            2,
            vec![
                span(counter.verb.clone(), th.text),
                span(format!("  {}", counter.note), th.border),
            ],
        ));
    }
    counters
}

/// The sheet's frame (design.md §11): the governor and its session in the
/// header, what it compressed in the status bar.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let status = model.governor.as_ref();
    let compressed = status
        .and_then(|s| s.counters.first())
        .map(|counter| counter.number.clone());
    SheetChrome {
        header_right: facts(
            th,
            vec![
                status
                    .map(|s| {
                        vec![
                            span("governor ", th.muted),
                            span(
                                match s.enabled {
                                    Some(true) => "on",
                                    Some(false) => "off",
                                    None => "status unknown",
                                },
                                th.text,
                            ),
                        ]
                    })
                    .unwrap_or_default(),
                status
                    .filter(|s| !s.session_id.is_empty())
                    .map(|s| vec![span(format!("session {}", s.session_id), th.muted)])
                    .unwrap_or_default(),
                status
                    .filter(|s| !s.since.is_empty())
                    .map(|s| vec![span(format!("since {}", s.since), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: compressed.map(|n| vec![span(format!("{n} compressed"), th.muted)]),
        status_right: None,
        hints: vec![
            hint(th, "/governor-status refresh"),
            hint(th, "/governor-reset reset counters"),
            hint(th, "↑↓ scroll"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("/governor-status"),
        echo: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.governor = Some(fixtures::governor_sheet());
        model.toggle_screen(Screen::Governor);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_counters_are_four_tiles_side_by_side() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let figures = rows
            .iter()
            .find(|row| row.contains("31 of 96 results"))
            .expect("the figures row");
        assert!(figures.contains("9 of 61 reads"), "{figures}");
        assert!(figures.contains("4 of 96 calls"), "{figures}");
        assert!(figures.contains("96.2k of 200k"), "{figures}");
        assert!(rows.iter().any(|row| row.contains("compressed")
            && row.contains("deduplicated")
            && row.contains("tokens never sent")));
        assert!(rows.iter().any(|row| row.contains("head 40 · tail 40")));
        assert!(rows
            .iter()
            .any(|row| row.contains("compresses above 8 KB or 300 lines")));
    }

    #[test]
    fn a_narrow_terminal_stacks_the_counters_two_rows_each() {
        let m = model(60);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert_eq!(rows[0], "31 of 96 results");
        assert!(rows[1].contains("compressed") && rows[1].contains("head 40 · tail 40"));
        assert!(rows.iter().any(|row| row == "96.2k of 200k"));
    }

    #[test]
    fn the_sample_and_the_store_are_visible_not_described() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("LATEST SAVED OUTPUT")));
        assert!(rows
            .iter()
            .any(|row| row.contains("Saved as") && row.contains("out-9f21c4")));
        let held = rows
            .iter()
            .position(|row| row.contains("held on disk, retrievable by id"))
            .expect("the held row");
        assert!(rows[held..=held + 1].iter().any(|row| row
            .contains("31 outputs · 2.8 MB · 4 newest shown · dropped when the session ends")));
        let stored = rows
            .iter()
            .find(|row| row.starts_with("out-9f21c4"))
            .expect("the stored row");
        assert!(stored.trim_end().ends_with("1,184 ln · 84 KB"), "{stored}");
        assert!(rows
            .iter()
            .any(|row| row.contains("%USERPROFILE%\\.pi\\outputs\\01JB2K\\")
                && row.contains("governor-reset clears the counters, not the store")));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn nothing_compressed_says_so() {
        let mut m = model(100);
        m.governor = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("nothing to report")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "5c");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "governor on │ session 01JB2K │ since 11:04");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "31 compressed");
        assert_eq!(c.composer, Composer::Prompt("/governor-status"));
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(hint.starts_with("/governor-status refresh"), "{hint}");
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
