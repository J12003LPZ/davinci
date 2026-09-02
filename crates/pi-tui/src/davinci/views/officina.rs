//! `6b` — `/reload`. What is loaded, what failed to load, and what it costs
//! every turn.
//!
//! The reload result is written as ordinary tool lines (design.md §3): the
//! elbow, the state glyph, what it did, then how long it took. A failed
//! extension keeps its error and says which of its tools are missing as a
//! result, rather than disappearing quietly.
//!
//! Mirrors artboard `6b` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, footnote, indent, meter, run_width, span, span_strong, spread, truncate_run, wrap,
    Surface, MEASURE,
};

const LABEL: usize = 18;
const COUNT: usize = 4;
const METER: u16 = 20;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(workshop) = model.workshop.as_ref() else {
        return vec![Line::from(vec![span(
            "nothing has been reloaded yet — /reload reads the workshop",
            th.muted,
        )])];
    };

    let mut out: Vec<Line<'static>> = Vec::new();
    for (state, text, duration, detail) in &workshop.reload {
        out.push(reload_line(*state, text, duration, th));
        if let Some(detail) = detail {
            // The first detail row is the error itself; the rest is what it
            // cost, in a quieter ink.
            for (index, part) in detail.split('\n').enumerate() {
                let color = if index == 0 { th.error } else { th.muted };
                for row in wrap(part, MEASURE.min(width).saturating_sub(6)) {
                    out.push(indent(4, vec![span(row, color)]));
                }
            }
        }
    }
    out.push(blank());

    let native_rows = |inner: u16| -> Vec<Vec<Span<'static>>> {
        let mut rows: Vec<Vec<Span<'static>>> = workshop
            .native
            .iter()
            .map(|(state, name, detail)| extension_row(inner, *state, name, detail, th))
            .collect();
        rows.push(
            spread(
                inner,
                vec![
                    span("· ", th.border),
                    span("built in — no node, no install", th.border),
                ],
                vec![span("0ms", th.border)],
            )
            .spans,
        );
        rows
    };
    let javascript_rows = |inner: u16| -> Vec<Vec<Span<'static>>> {
        let mut rows: Vec<Vec<Span<'static>>> = workshop
            .javascript
            .iter()
            .map(|(state, name, detail)| extension_row(inner, *state, name, detail, th))
            .collect();
        let mut node = vec![
            span("· ", th.border),
            span(workshop.node.clone(), th.border),
        ];
        if !workshop.node_note.is_empty() {
            node.push(span(" · ", th.border));
            node.push(span(workshop.node_note.clone(), th.border));
        }
        rows.push(
            spread(
                inner,
                node,
                vec![span(workshop.node_elapsed.clone(), th.border)],
            )
            .spans,
        );
        rows
    };
    let native_title = vec![
        span("NATIVE", th.primary),
        span(" · ", th.border),
        span("RUST, ALWAYS ON", th.muted),
    ];
    let javascript_title = vec![
        span("JAVASCRIPT", th.primary),
        span(" · ", th.border),
        span("NODE SUBPROCESS", th.muted),
    ];
    // Side by side where the window allows it, as the artboard sets them;
    // stacked below 100 columns (design.md §7).
    if width >= 100 {
        let half = (width - 2) / 2;
        let right_width = width - half - 2;
        let left = Surface::new(half, th)
            .title(native_title)
            .rows(native_rows(half.saturating_sub(4)))
            .lines();
        let right = Surface::new(right_width, th)
            .title(javascript_title)
            .rows(javascript_rows(right_width.saturating_sub(4)))
            .lines();
        let rows = left.len().max(right.len());
        let blank_left = Line::from(vec![span(" ".repeat(half as usize), th.border)]);
        for index in 0..rows {
            let mut spans = left.get(index).unwrap_or(&blank_left).spans.clone();
            spans.push(span("  ", th.border));
            if let Some(row) = right.get(index) {
                spans.extend(row.spans.iter().cloned());
            }
            out.push(Line::from(spans));
        }
    } else {
        out.extend(
            Surface::new(width, th)
                .title(native_title)
                .rows(native_rows(width.saturating_sub(4)))
                .lines(),
        );
        out.push(blank());
        out.extend(
            Surface::new(width, th)
                .title(javascript_title)
                .rows(javascript_rows(width.saturating_sub(4)))
                .lines(),
        );
    }
    out.push(blank());

    let (schema_tokens, schema_share) = workshop
        .schema
        .split_once(" · ")
        .map(|(tokens, share)| (tokens.to_string(), share.to_string()))
        .unwrap_or_else(|| (workshop.schema.clone(), String::new()));
    let mut schema = vec![
        span(schema_tokens, th.muted),
        span(" of the window is tool schema", th.muted),
    ];
    if !schema_share.is_empty() {
        schema.push(span(" · ", th.border));
        schema.push(span(schema_share, th.warning));
    }
    out.push(spread(
        width,
        vec![span("what every turn carries", th.text)],
        schema,
    ));
    for (label, count, fraction, note) in &workshop.tools {
        let mut left = vec![
            span(format!("{label:<LABEL$}"), th.muted),
            span(format!("{count:>COUNT$}  "), th.text),
        ];
        left.extend(meter(*fraction, METER, th, Some(th.secondary)));
        let right = if note.is_empty() {
            Vec::new()
        } else {
            vec![span(note.clone(), th.border)]
        };
        out.push(spread(width, left, right));
    }
    out.push(blank());

    out.extend(footnote(
        width,
        vec![
            span("-nt", th.text),
            span(" disables all tools", th.muted),
            span(" · ", th.border),
            span("-t read,grep,ls", th.text),
            span(" keeps three", th.muted),
            span(" · ", th.border),
            span("-xt bash", th.text),
            span(" drops one", th.muted),
        ],
        vec![span(
            "/reload does not restart the session or lose the transcript",
            th.border,
        )],
        th,
    ));
    // A long footer or extension detail is cut at the window, never wrapped:
    // the sheet reports one height and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

fn reload_line(state: State, text: &str, duration: &str, th: &Theme) -> Line<'static> {
    let elbow = if state == State::Failed {
        Vec::new()
    } else {
        vec![span(format!("{} ", glyph::BRANCH), th.border)]
    };
    let mut spans = elbow;
    spans.push(span_strong(
        format!("{} ", state.glyph()),
        th.state_color(state),
        th,
    ));
    spans.push(span(
        text.to_string(),
        if state == State::Failed {
            th.text
        } else {
            th.muted
        },
    ));
    spans.push(span(format!("   {duration}"), th.border));
    indent(2, spans)
}

/// One extension inside its panel: the name at the left, what it brought at
/// the right edge; a failure is named in the error colour.
fn extension_row(
    inner: u16,
    state: State,
    name: &str,
    detail: &str,
    th: &Theme,
) -> Vec<Span<'static>> {
    let failed = state == State::Failed;
    let (base, suffix) = name.split_once(" · ").unwrap_or((name, ""));
    let mut left = vec![
        span_strong(format!("{} ", state.glyph()), th.state_color(state), th),
        span(base.to_string(), if failed { th.muted } else { th.text }),
    ];
    if !suffix.is_empty() {
        left.push(span(format!(" · {suffix}"), th.border));
    }
    let right = vec![span(
        detail.to_string(),
        if failed { th.error } else { th.border },
    )];
    if run_width(&left) + run_width(&right) + 1 > inner {
        let mut row = left;
        row.push(span("  ", th.border));
        row.extend(right);
        return row;
    }
    spread(inner, left, right).spans
}

/// `21400` → `21.4k`.
fn tenths_k(tokens: u64) -> String {
    format!("{:.1}k", tokens as f64 / 1_000.0)
}

/// The sheet's frame (design.md §11): what is loaded and what it costs in
/// the header, how many failed in the status bar, the composer ready.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let workshop = model.workshop.as_ref();
    let facts_ = &model.facts;
    let failed = workshop.map(|w| {
        let listed = w
            .native
            .iter()
            .chain(w.javascript.iter())
            .filter(|(state, _, _)| *state == State::Failed)
            .count();
        if listed > 0 {
            listed
        } else {
            w.reload
                .iter()
                .filter(|(state, _, _, _)| *state == State::Failed)
                .count()
        }
    });
    SheetChrome {
        header_right: facts(
            th,
            vec![
                if facts_.tool_count == 0 {
                    Vec::new()
                } else {
                    vec![span(format!("{} tools", facts_.tool_count), th.muted)]
                },
                if facts_.command_count == 0 {
                    Vec::new()
                } else {
                    vec![span(format!("{} commands", facts_.command_count), th.muted)]
                },
                if facts_.tool_schema_tokens == 0 {
                    Vec::new()
                } else {
                    vec![span(
                        format!("{} of schema", tenths_k(facts_.tool_schema_tokens)),
                        th.muted,
                    )]
                },
            ],
        ),
        status_third: failed.map(|n| {
            if n > 0 {
                vec![span(format!("{n} failed"), th.error)]
            } else {
                vec![span("all loaded", th.muted)]
            }
        }),
        status_right: None,
        hints: vec![
            hint_dim(th, "enter open the source"),
            hint(th, "r reload again"),
            hint_dim(th, "d disable one"),
            hint_dim(th, "e show its error"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("ask davinci…"),
        echo: Some("/reload".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::WorkshopSheet;
    use crate::davinci::theme::ColorDepth;

    fn sheet() -> WorkshopSheet {
        WorkshopSheet {
            reload: vec![
                (
                    State::Done,
                    "keybindings · 39 bindings, 2 yours".into(),
                    "3ms".into(),
                    None,
                ),
                (
                    State::Failed,
                    "extensions · deploy.js failed to register".into(),
                    "318ms".into(),
                    Some(
                        "TypeError: hooks.preTool is not a function · deploy.js:41\n\
                         its 3 tools are not registered; everything else loaded"
                            .into(),
                    ),
                ),
            ],
            native: vec![
                (
                    State::Done,
                    "vector-memory".into(),
                    "4 tools · 4 cmds".into(),
                ),
                (State::Done, "graph".into(), "1 tool · 5 cmds".into()),
            ],
            javascript: vec![
                (State::Done, "plan-mode".into(), "1 tool · --plan".into()),
                (State::Failed, "deploy.js · project".into(), "failed".into()),
            ],
            node: "node v24.19.0".into(),
            node_note: "one process, reused".into(),
            node_elapsed: "318ms".into(),
            schema: "21.4k · 11%".into(),
            tools: vec![
                (
                    "built-in tools".into(),
                    "8".into(),
                    0.33,
                    "read write edit bash powershell grep find ls".into(),
                ),
                (
                    "native tools".into(),
                    "14".into(),
                    0.58,
                    "memory, governor".into(),
                ),
            ],
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.workshop = Some(sheet());
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_reload_reads_as_tool_lines_and_a_failure_keeps_its_error() {
        let rows: Vec<String> = lines(&model(100)).iter().map(text).collect();
        assert!(!rows.iter().any(|row| row.contains("> /reload")));
        assert!(rows
            .iter()
            .any(|row| row.contains("⎿ ✓ keybindings · 39 bindings")));
        assert!(rows
            .iter()
            .any(|row| row.contains("TypeError: hooks.preTool is not a function · deploy.js:41")));
        assert!(rows
            .iter()
            .any(|row| row.contains("its 3 tools are not registered; everything else loaded")));
        let native = rows
            .iter()
            .find(|row| row.contains("NATIVE"))
            .expect("the native panel");
        assert!(native.contains("JAVASCRIPT"), "{native}");
        assert!(rows
            .iter()
            .any(|row| row.contains("built in — no node, no install")
                && row.contains("0ms")
                && row.contains("node v24.19.0 · one process, reused")
                && row.trim_end().ends_with("318ms │")));
        assert!(rows
            .iter()
            .any(|row| row.contains("vector-memory") && row.contains("4 tools · 4 cmds")));
        assert!(rows
            .iter()
            .any(|row| row.contains("deploy.js · project") && row.contains("failed")));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("what every turn carries")
                && row
                    .trim_end()
                    .ends_with("21.4k of the window is tool schema · 11%")));
        let builtin = rows
            .iter()
            .find(|row| row.starts_with("built-in tools"))
            .expect("the built-in row");
        assert!(builtin.contains("◸"), "{builtin}");
        assert!(builtin
            .trim_end()
            .ends_with("read write edit bash powershell grep find ls"));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("-nt disables all tools")));
        assert!(
            rows.iter()
                .any(|row| row
                    .contains("/reload does not restart the session or lose the transcript"))
        );
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn below_one_hundred_the_panels_stack() {
        let rows: Vec<String> = lines(&model(90)).iter().map(text).collect();
        let native = rows
            .iter()
            .find(|row| row.contains("NATIVE"))
            .expect("the native panel");
        assert!(!native.contains("JAVASCRIPT"));
        assert!(rows.iter().any(|row| row.contains("JAVASCRIPT")));
    }

    #[test]
    fn no_row_overflows_the_window_at_any_breakpoint() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(run_width(&row.spans) <= width, "overflow at {width}");
            }
        }
    }

    #[test]
    fn a_session_that_never_reloaded_says_so() {
        let mut m = model(100);
        m.workshop = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("nothing has been reloaded yet")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "6b");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "24 tools │ 37 commands │ 21.4k of schema");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "1 failed");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Prompt("ask davinci…"));
        assert_eq!(c.echo.as_deref(), Some("/reload"));
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with(
                "enter open the source │ r reload again │ d disable one │ e show its error"
            ),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }
}
