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
use unicode_width::UnicodeWidthStr;

use crate::davinci::model::{Model, Overlay, Screen};
use crate::davinci::theme::glyph;
use crate::davinci::ui::{
    clip_ellipsis, meter, pad, run_width, span, span_on, span_strong, spread, Surface,
};

use super::instrumenta::SELECTION_BAR;

/// The three cells an unmarked completion row spends to stay aligned with the
/// marked one.
const UNSELECTED_BAR: &str = "   ";

/// Which hints sit under the composer. Every panel states its own exits (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    Default,
    Multiline,
    /// A sheet is open: the composer still sends, and esc closes the sheet.
    Closable,
    None,
}

/// `D davinci · agent` on the left, `path │ branch │ model` on the right.
/// Memoria recall and Mensura claim the right run for their own facts, as the
/// mockups set them (`2b`, `2c`).
pub fn header(model: &Model) -> Line<'static> {
    let th = &model.theme;
    let mut left = vec![span_strong("D", th.primary, th), span(" davinci", th.text)];
    if !model.minimal() {
        left.push(span(" · ", th.border));
        left.push(span(model.mode(), th.primary));
    }

    let right = if model.minimal() {
        vec![
            span(short_cwd(&model.cwd), th.muted),
            span(" │ ", th.border),
            span(model.branch.clone(), th.secondary),
        ]
    } else if model.screen == Screen::Memoria {
        let meta = &model.recall_meta;
        vec![
            span(format!("{} vectors", meta.vectors), th.muted),
            span(" │ ", th.border),
            span(format!("{} shards", meta.shards), th.muted),
            span(" │ ", th.border),
            span(meta.embedding.clone(), th.muted),
        ]
    } else if model.screen == Screen::Mensura {
        let meta = &model.budget_meta;
        vec![
            span("policy ", th.muted),
            span(meta.policy.clone(), th.text),
            span(" │ ", th.border),
            span(format!("window {}", meta.window), th.muted),
            span(" │ ", th.border),
            span(
                format!("{} · {}", model.model_name, model.thinking_level),
                th.muted,
            ),
        ]
    } else {
        let mut right = vec![
            span(model.cwd.clone(), th.muted),
            span(" │ ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" │ ", th.border),
            span(
                format!("{} · {}", model.model_name, model.thinking_level),
                th.muted,
            ),
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

    // With an instrument floating over the transcript, the status bar
    // abbreviates and dims behind it (`1d`, `1f`).
    if model.overlay.is_some() {
        return vec![
            span(model.mode(), th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{}{delta}", glyph::DELTA), th.primary),
        ];
    }

    match model.screen {
        Screen::Grafo => vec![
            span("grafo", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span("impact view", th.muted),
        ],
        Screen::Memoria => {
            let meta = &model.recall_meta;
            vec![
                span("memoria", th.secondary),
                span(" · ", th.border),
                span(format!("recall {} of {}", meta.k, meta.vectors), th.muted),
            ]
        }
        Screen::Mensura => {
            let mut left = vec![
                span("mensura", th.warning),
                span(" · ", th.border),
                span(model.branch.clone(), th.secondary),
            ];
            if model.proposal.is_some() {
                left.push(span(" · ", th.border));
                left.push(span("1 proposal", th.muted));
            }
            left
        }
        Screen::Plan => vec![
            span("plan", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{} steps", model.plan.len()), th.muted),
        ],
        Screen::Agent if model.minimal() => vec![
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{}{delta}", glyph::DELTA), th.primary),
        ],
        // Before anything has changed there is no Δ to count; the model in
        // hand takes its place (`1a`).
        Screen::Agent if delta == 0 => vec![
            span(model.mode(), th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(
                format!("{} · {}", model.model_name, model.thinking_level),
                th.muted,
            ),
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
        // The command-opened sheets (`3a`–`6d`): the mode word carries the
        // instrument, the branch stays for orientation.
        _ => vec![
            span(model.mode(), th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
        ],
    }
}

fn status_right(model: &Model) -> Vec<Span<'static>> {
    let th = &model.theme;
    let fraction = model.context_fraction();
    // Truncated, not rounded: a meter must never claim a cap has been reached
    // before it has (47k of 200k reads 23%, screen `1g`).
    let percent = (fraction * 100.0) as u32;
    let (used, cap) = model.context;

    if model.screen == Screen::Grafo {
        return vec![
            span("enter open node", th.border),
            span(" · ", th.border),
            span("x expand", th.border),
            span(" · ", th.border),
            span("esc close", th.border),
        ];
    }

    // Behind an overlay the meter abbreviates; the pickers also state the
    // shared exit here rather than each growing a footer row (`1d`, `1f`).
    match model.overlay {
        Some(Overlay::Instrumenta) => {
            return vec![span(
                format!("{}/{}", thousands(used), thousands(cap)),
                th.muted,
            )];
        }
        Some(_) => {
            return vec![
                span("mensura ", th.muted),
                span(th.pie(fraction), th.primary),
                span(format!(" {percent}%"), th.muted),
                span(" · ", th.border),
                span("esc close", th.border),
            ];
        }
        None => {}
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

    // The empty state points at the palette instead of metering nothing (`1a`).
    if model.screen == Screen::Agent && model.transcript.is_empty() && !model.codex_open() {
        return vec![
            span("mensura ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("ctrl+p instrumenta", th.border),
        ];
    }

    // The plan sheet reads its budget as a proportion first (`1c`).
    if model.screen == Screen::Plan {
        return vec![
            span("mensura ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span(format!("{}/{}", thousands(used), thousands(cap)), th.muted),
        ];
    }

    let mut right = vec![span("context ", th.muted)];
    right.extend(meter(fraction, 12, th, None));
    right.push(span(
        format!(" {}/{}", thousands(used), thousands(cap)),
        th.muted,
    ));
    if model.codex_open() {
        right.push(span(" · ", th.border));
        right.push(span("ctrl+p", th.border));
    }
    right
}

/// Rows for the composer plus its hint row. Grows with content. The box takes
/// its whole look from the model: copper rule while it is the active input,
/// border rule at rest — the untouched empty state (`1a`) and under an open
/// instrument (`1d`) — and the recall screen replaces the line being typed
/// with its own keys (`2b`).
pub fn composer(model: &Model, lines: Option<&[String]>, hint: Hint) -> Vec<Line<'static>> {
    let th = &model.theme;

    // `2b` — the composer carries Memoria's keys while recall is open.
    if model.screen == Screen::Memoria {
        let keys = if model.minimal() {
            "enter pin · r reindex · esc close"
        } else {
            "enter pin to context · f raise floor · r reindex · esc close"
        };
        return Surface::new(model.width, th)
            .row(vec![
                span(format!("{} ", glyph::PROMPT), th.secondary),
                span(keys, th.muted),
            ])
            .lines();
    }

    let owned = lines.map(<[String]>::to_vec);
    // A composer holding newlines is drawn as the rows the user typed; an
    // empty one is still one row, so the box never collapses.
    let entries: Vec<String> =
        owned.unwrap_or_else(|| model.composer.split('\n').map(str::to_string).collect());
    let last = entries.len().saturating_sub(1);
    let overlaid = model.overlay.is_some();
    let untouched = model.screen == Screen::Agent
        && model.transcript.is_empty()
        && model.composer.is_empty()
        && model.queued.is_empty();
    let border = if overlaid || untouched {
        th.border
    } else {
        th.primary
    };
    // An empty composer carries no placeholder prose — the prompt glyph and
    // the caret are the whole invitation, as in every terminal agent. An open
    // sheet is the one exception: its row suggests the command that summoned
    // it, which is a hint, not chat.
    let placeholder = screen_placeholder(model.screen);
    let lit = model.blink();
    let caret_style = if lit {
        Style::default().bg(th.primary).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };
    // Where the editor's cursor actually sits, as `(row, byte column)`. Only
    // the composer's own text can be indexed this way: a caller-supplied row
    // set (a sheet's suggestion, recall's keys) is not what the editor holds,
    // so those keep the caret parked at the end.
    let caret_at = if lines.is_none() && !overlaid {
        Some(model.composer.editor().get_cursor())
    } else {
        None
    };
    let caret_row = caret_at.map_or(last, |(row, _)| row.min(last));

    let mut surface = Surface::new(model.width, th).border(border);
    for (index, entry) in entries.into_iter().enumerate() {
        // An echoed command reads muted, prose bright (`2a`, `2c`).
        let ink = if entry.starts_with('/') {
            th.muted
        } else {
            th.text
        };
        let shown = clip_ellipsis(&entry, model.width.saturating_sub(10));
        // The caret sits *on* the character it is in front of, not after the
        // whole row: parking a block at end-of-line made the arrow keys look
        // dead even though the editor had moved.
        let split = match caret_at {
            Some((row, col)) if row == index && !overlaid => split_at_caret(&shown, col),
            _ => None,
        };
        let caret_here = split.is_some();
        let body = if entry.is_empty() {
            vec![span(placeholder.unwrap_or("").to_string(), th.muted)]
        } else if let Some((before, under, after)) = split {
            vec![
                span(before, ink),
                // Unlit, the character under the caret is still the user's
                // text — painting it background-on-background would blink the
                // letter itself out of the line.
                if lit {
                    Span::styled(under, caret_style)
                } else {
                    span(under, ink)
                },
                span(after, ink),
            ]
        } else {
            vec![span(shown, ink)]
        };
        // The prompt is copper on the opening row; continuation rows carry a
        // quiet one, as the mockup's wrapped composer does (`1c`).
        let prompt_ink = if index == 0 && !overlaid {
            th.primary
        } else {
            th.border
        };
        let mut row = vec![span(format!("{} ", glyph::PROMPT), prompt_ink)];
        row.extend(body);
        // The caret belongs to whatever owns the keyboard; an open instrument
        // owns it, so the composer's goes with it (`1d`, `1f`). At end of line
        // it has no character to sit on, so it takes a cell of its own.
        if index == caret_row && !overlaid && !caret_here {
            row.push(Span::styled(" ", caret_style));
        }
        surface = surface.row(row);
    }

    let rows_typed = last + 1;
    let mut rows = surface.lines();
    if hint != Hint::None {
        rows.push(hint_line(model, hint, rows_typed));
    }
    rows
}

/// Split a drawn composer row into `(before, under, after)` around the caret,
/// where `col` is a byte offset into the row's text.
///
/// `None` means the caret has no character to sit on in this row and the caller
/// should give it a cell of its own: the cursor is at end of line, or past what
/// survived `clip_ellipsis` on an over-long row (davinci's composer does not
/// scroll horizontally, so a caret out beyond the `…` has nowhere to land).
fn split_at_caret(shown: &str, col: usize) -> Option<(String, String, String)> {
    if col >= shown.len() || !shown.is_char_boundary(col) {
        return None;
    }
    let under = shown[col..].chars().next()?;
    let end = col + under.len_utf8();
    Some((
        shown[..col].to_string(),
        under.to_string(),
        shown[end..].to_string(),
    ))
}

/// What the composer suggests while a sheet is open: the command that
/// summoned the sheet, as the mockups set each one (`2a`–`6d`).
fn screen_placeholder(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Grafo => Some("/graph path …"),
        Screen::Mensura => Some("/mensura policy frugal"),
        Screen::Models => Some("/model anthropic/claude-opus"),
        Screen::Settings => Some("/settings"),
        Screen::Thinking => Some("/thinking high"),
        Screen::Login => Some("/login openai"),
        Screen::Keys => Some("/hotkeys"),
        Screen::Resume => Some("/resume provider-parity"),
        Screen::Tree => Some("/tree"),
        Screen::Compact => Some("/compact keep the store.rs decisions"),
        Screen::Export => Some("/share"),
        Screen::GraphRun => Some("/graph-view t6"),
        Screen::Vectors => Some("/memory-search interrupt handling"),
        Screen::Governor => Some("/governor-status"),
        Screen::Securitas => Some("/sec-report --severity high"),
        Screen::Officina => Some("/reload"),
        Screen::Trust => Some("decide first"),
        Screen::Recovery | Screen::Diff | Screen::Agent | Screen::Plan | Screen::Memoria => None,
    }
}

/// What the composer is offering, drawn directly above it: the marked row
/// carries the same 3-cell copper bar and tinted ground Instrumenta uses, so a
/// selection reads without color (design.md §6). Nothing is drawn when there is
/// nothing on offer.
pub fn suggestions(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(found) = &model.suggestions else {
        return Vec::new();
    };
    if found.items.is_empty() {
        return Vec::new();
    }
    let inner = model.width.saturating_sub(4);
    // The value column is wide enough for a command name and its arguments;
    // whatever is left describes it. On a narrow window the description goes
    // rather than the name (§9).
    let name_column = (inner / 3).clamp(12, 28);
    // Only a window of the list is drawn, around the selection; the fold is
    // counted either side so six visible providers never read as the whole
    // list.
    let (start, end) = model.suggestion_window();
    let total = found.items.len();
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    if start > 0 {
        rows.push(vec![
            span_on(UNSELECTED_BAR, th.border, None),
            span(format!("… {start} above"), th.border),
        ]);
    }
    rows.extend(
        found.items[start..end]
            .iter()
            .enumerate()
            .map(|(offset, item)| {
                let index = start + offset;
                let selected = index == model.suggestion_index;
                let tint = if selected { Some(th.surface) } else { None };
                let bar = if selected {
                    span_on(SELECTION_BAR, th.primary, tint)
                } else {
                    span_on(UNSELECTED_BAR, th.border, tint)
                };
                let label = clip_ellipsis(&item.label, name_column);
                let gap =
                    name_column.saturating_sub(UnicodeWidthStr::width(label.as_str()) as u16) + 2;
                let mut row = vec![
                    bar,
                    span_on(label, if selected { th.text } else { th.muted }, tint),
                ];
                match item.description.as_deref().filter(|_| !model.minimal()) {
                    Some(description) if !description.is_empty() => {
                        row.push(span_on(" ".repeat(gap as usize), th.border, tint));
                        row.push(span_on(
                            clip_ellipsis(
                                description,
                                inner
                                    .saturating_sub(name_column)
                                    .saturating_sub(gap)
                                    .saturating_sub(3),
                            ),
                            th.border,
                            tint,
                        ));
                    }
                    _ => {}
                }
                row
            }),
    );
    if end < total {
        rows.push(vec![
            span_on(UNSELECTED_BAR, th.border, None),
            span(format!("… {} below", total - end), th.border),
        ]);
    }

    Surface::new(model.width, th)
        .border(th.border)
        .title(vec![span("COMPLETIONS", th.border)])
        .right(vec![span(
            format!("{total} · ↑↓ move · tab take · esc close"),
            th.border,
        )])
        .rows(rows)
        .lines()
}

/// How many rows [`suggestions`] will occupy, known before it is built.
pub fn suggestions_height(model: &Model) -> u16 {
    match &model.suggestions {
        Some(found) if !found.items.is_empty() => {
            let (start, end) = model.suggestion_window();
            let mut rows = (end - start) as u16 + 2;
            if start > 0 {
                rows += 1;
            }
            if end < found.items.len() {
                rows += 1;
            }
            rows
        }
        _ => 0,
    }
}

/// How many rows [`composer`] will occupy, known before it is built.
pub fn composer_height(lines: Option<&[String]>, hinted: bool) -> u16 {
    let entries = lines.map(<[String]>::len).unwrap_or(1).max(1);
    // surface top + entries + surface bottom, then the hint row if any.
    entries as u16 + 2 + u16::from(hinted)
}

/// The keybind row under the composer: hints left, the closing act right,
/// split by hairline bars (`1b`, `1c`).
fn hint_line(model: &Model, hint: Hint, rows_typed: usize) -> Line<'static> {
    let th = &model.theme;
    let bar = || span(" │ ", th.border);
    let (left, right) = match hint {
        Hint::None => (Vec::new(), Vec::new()),
        Hint::Closable => (
            vec![span("enter send", th.border)],
            vec![span("esc close", th.border)],
        ),
        Hint::Multiline => (
            vec![
                span("shift+enter newline", th.border),
                bar(),
                span(format!("{rows_typed} lines"), th.border),
            ],
            vec![span("enter send", th.border)],
        ),
        Hint::Default if model.minimal() => (
            vec![span("enter send", th.border)],
            vec![span("esc cancel", th.border)],
        ),
        Hint::Default => (
            vec![
                span("enter send", th.border),
                bar(),
                span("shift+enter newline", th.border),
                bar(),
                span("tab complete", th.border),
            ],
            vec![span("esc cancel", th.border)],
        ),
    };
    let mut spans = spread(model.width.saturating_sub(4), left, right).spans;
    spans.insert(0, pad(2, None));
    Line::from(spans)
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
        // A turn under way: the untouched empty state has its own quieter
        // composer, tested separately.
        model
            .transcript
            .push(crate::davinci::model::Entry::user("run the tests"));
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_long_completion_list_is_windowed_and_every_row_stays_reachable() {
        let mut m = model(100);
        m.login_providers = (b'a'..=b'z')
            .map(|c| format!("provider-{}", c as char))
            .collect();
        m.suggestion_rows = 6;
        m.composer.push_str("/login ");
        m.refresh_suggestions();
        let total = m
            .suggestions
            .as_ref()
            .expect("providers offered")
            .items
            .len();
        assert_eq!(total, 26, "nothing past the cap is discarded");

        let drawn: Vec<String> = suggestions(&m).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("below")),
            "the fold is counted: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|row| row.contains("26 ·")),
            "the header states the real total: {drawn:?}"
        );
        assert_eq!(
            suggestions_height(&m) as usize,
            suggestions(&m).len(),
            "the promised height is the drawn height"
        );

        // The last provider is reachable, and the window follows.
        for _ in 0..25 {
            m.suggestion_move(1);
        }
        let drawn: Vec<String> = suggestions(&m).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("provider-z")),
            "the selection walked past the fold: {drawn:?}"
        );
        assert!(drawn.iter().any(|row| row.contains("above")), "{drawn:?}");
        assert_eq!(suggestions_height(&m) as usize, suggestions(&m).len());
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
    fn the_header_carries_active_reasoning_next_to_the_model() {
        let mut m = model(100);
        m.thinking_level = "high".into();

        let drawn = text(&header(&m));
        assert!(drawn.contains("sonnet · high"), "{drawn}");
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
        assert_eq!(rows.len() as u16, composer_height(None, true));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].spans[0].style.fg, Some(m.theme.primary));
        let prompt_row = text(&rows[1]);
        assert!(prompt_row.contains("›"), "{prompt_row}");
        assert!(
            !prompt_row.contains("…"),
            "an empty composer carries no placeholder prose: {prompt_row}"
        );
        assert!(text(&rows[3]).contains("enter send │ shift+enter newline"));
        assert!(text(&rows[3]).trim_end().ends_with("esc cancel"));
    }

    #[test]
    fn the_untouched_empty_state_is_a_quiet_rule_with_no_prose() {
        let mut m = model(100);
        m.transcript.clear();
        let rows = composer(&m, None, Hint::None);
        assert_eq!(rows[0].spans[0].style.fg, Some(m.theme.border));
        let prompt_row = text(&rows[1]);
        assert!(prompt_row.contains("›"), "{prompt_row}");
        assert!(!prompt_row.contains("…"), "{prompt_row}");
    }

    #[test]
    fn a_sheet_suggests_its_summoning_command_and_the_chat_suggests_nothing() {
        // The agent chat carries no placeholder prose; an open sheet is the
        // one exception, and its hint is muted so nothing about it reads as
        // typed text.
        let mut m = model(100);
        m.composer = String::new().into();
        m.screen = crate::davinci::model::Screen::Models;
        let rows = composer(&m, None, Hint::None);
        let hint = rows[1]
            .spans
            .iter()
            .find(|span| span.content.starts_with('/'))
            .expect("the sheet hint");
        assert_eq!(hint.style.fg, Some(m.theme.muted), "{:?}", hint.content);

        m.screen = crate::davinci::model::Screen::Agent;
        let drawn = text(&composer(&m, None, Hint::None)[1]);
        assert!(
            !drawn.chars().any(char::is_alphabetic),
            "the empty chat row is just the prompt: {drawn}"
        );
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
        assert_eq!(composer_height(Some(&entered), true), 5);
        assert!(text(&rows[1]).contains("keep step IV"));
        assert!(text(&rows[2]).contains("existing TS golden files"));
        let hint = text(&rows[4]);
        assert!(hint.contains("shift+enter newline │ 2 lines"), "{hint}");
        assert!(hint.trim_end().ends_with("enter send"), "{hint}");
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
        assert!(drawn.contains("enter send"), "{drawn}");
        assert!(drawn.contains("esc cancel"), "{drawn}");
        assert!(!drawn.contains("tab complete"), "{drawn}");
    }

    #[test]
    fn recall_replaces_the_composer_with_memorias_own_keys() {
        let mut m = model(100);
        m.toggle_screen(Screen::Memoria);
        let rows = composer(&m, None, Hint::None);
        let drawn = text(&rows[1]);
        assert!(drawn.contains("enter pin to context"), "{drawn}");
        assert!(drawn.contains("esc close"), "{drawn}");
        assert!(
            !rows
                .iter()
                .flat_map(|row| row.spans.iter())
                .any(|span| span.style.bg == Some(m.theme.primary)),
            "no caret while memoria owns the keys"
        );
    }

    #[test]
    fn the_palette_query_does_not_reach_the_composer_row() {
        let mut m = model(100);
        m.toggle_overlay(Overlay::Instrumenta);
        m.type_char("git");
        let drawn = text(&composer(&m, None, Hint::Default)[1]);
        assert!(
            !drawn.contains("…"),
            "no placeholder under an overlay: {drawn}"
        );
    }

    /// The caret cell is the one span drawn on the theme's primary; find where
    /// it sits within a composer row, in columns after the `› ` prompt.
    fn caret_column(model: &Model, row: usize) -> Option<usize> {
        let line = composer(model, None, Hint::Default).remove(row);
        // Everything the surface draws before the text: the box rule and the
        // `› ` prompt.
        let mut prefix = 0usize;
        let mut column = 0usize;
        let mut caret = None;
        for span in &line.spans {
            if caret.is_none() && span.style.bg == Some(model.theme.primary) {
                caret = Some(column);
            }
            column += UnicodeWidthStr::width(span.content.as_ref());
            if span.content.contains(glyph::PROMPT) {
                prefix = column;
            }
        }
        caret.map(|at| at.saturating_sub(prefix))
    }

    #[test]
    fn the_caret_sits_where_the_cursor_is_not_at_the_end_of_the_row() {
        let mut m = model(100);
        m.type_char("what llm model are you");
        assert_eq!(
            caret_column(&m, 1),
            Some(22),
            "end of line takes its own cell"
        );

        for _ in 0..5 {
            m.composer.editor_mut().move_left();
        }
        assert_eq!(
            caret_column(&m, 1),
            Some(17),
            "five left of the end is the `e` of `are`, not the end of the row"
        );

        m.composer.editor_mut().move_line_start();
        assert_eq!(caret_column(&m, 1), Some(0), "line start is column zero");
    }

    #[test]
    fn the_caret_follows_the_cursor_onto_an_earlier_composer_row() {
        let mut m = model(100);
        m.type_char("alpha");
        m.newline();
        m.type_char("bravo");
        assert_eq!(
            caret_column(&m, 2),
            Some(5),
            "the caret starts on the row being typed"
        );
        assert_eq!(caret_column(&m, 1), None, "and only on that row");

        m.composer.editor_mut().cursor_up();
        assert_eq!(
            caret_column(&m, 1),
            Some(5),
            "up moves the caret onto `alpha`"
        );
        assert_eq!(caret_column(&m, 2), None, "and off `bravo`");
    }

    #[test]
    fn a_caret_past_the_clip_keeps_its_own_cell() {
        // No horizontal scrolling: a cursor out beyond the `…` has no
        // character to sit on, so the row falls back to a trailing caret
        // rather than indexing into text that was never drawn.
        assert_eq!(split_at_caret("abc…", 9), None);
        assert_eq!(
            split_at_caret("abc", 3),
            None,
            "end of line has no character"
        );
        assert_eq!(
            split_at_caret("héllo", 1),
            Some((String::from("h"), String::from("é"), String::from("llo"))),
            "a multi-byte character is taken whole"
        );
        assert_eq!(split_at_caret("héllo", 2), None, "never splits a character");
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
