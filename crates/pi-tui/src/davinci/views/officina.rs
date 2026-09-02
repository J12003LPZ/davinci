//! `6b` — `/reload`. What is loaded, what failed to load, and what it costs
//! every turn.
//!
//! The reload result is written as ordinary tool lines (design.md §3): the
//! elbow, the state glyph, what it did, then how long it took. A failed
//! extension keeps its error and says which of its tools are missing as a
//! result, rather than disappearing quietly.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/officina.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, indent, meter, span, span_strong, truncate_run, wrap, Surface, MEASURE,
};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(workshop) = model.workshop.as_ref() else {
        return vec![Line::from(vec![span(
            "nothing has been reloaded yet — /reload reads the workshop",
            th.muted,
        )])];
    };

    let mut out = vec![
        Line::from(vec![
            span(format!("{} ", glyph::USER), th.primary),
            span("/reload", th.muted),
        ]),
        blank(),
    ];

    for (state, text, duration, detail) in &workshop.reload {
        out.push(reload_line(*state, text, duration, th));
        if let Some(detail) = detail {
            for row in wrap(detail, MEASURE.saturating_sub(6)) {
                out.push(indent(6, vec![span(row, th.error)]));
            }
        }
    }
    out.push(blank());

    out.extend(
        Surface::new(width, th)
            .title(vec![
                span("NATIVE", th.primary),
                span(" · ", th.border),
                span("RUST, ALWAYS ON", th.muted),
            ])
            .right(vec![span("0ms", th.border)])
            .rows(
                workshop
                    .native
                    .iter()
                    .map(|(state, name, detail)| extension_row(*state, name, detail, th))
                    .collect(),
            )
            .lines(),
    );
    out.push(blank());
    out.extend(
        Surface::new(width, th)
            .title(vec![
                span("JAVASCRIPT", th.primary),
                span(" · ", th.border),
                span("NODE SUBPROCESS", th.muted),
            ])
            .right(vec![span(workshop.node.clone(), th.border)])
            .rows(
                workshop
                    .javascript
                    .iter()
                    .map(|(state, name, detail)| extension_row(*state, name, detail, th))
                    .collect(),
            )
            .lines(),
    );
    out.push(blank());

    out.push(Line::from(vec![
        span("what every turn carries", th.text),
        span("   ", th.border),
        span(workshop.schema.clone(), th.warning),
        span(" of the window is tool schema", th.muted),
    ]));
    for (label, count, fraction, note) in &workshop.tools {
        let mut row = vec![
            span(format!("{label:<16}"), th.muted),
            span(format!("{count:>4}  "), th.text),
        ];
        row.extend(meter(*fraction, 14, th, Some(th.secondary)));
        row.push(span(format!("  {note}"), th.border));
        out.push(Line::from(row));
    }
    out.push(blank());

    out.push(Line::from(vec![
        span("-nt disables all tools", th.border),
        span(" · ", th.border),
        span("-t read,grep,ls keeps three", th.border),
        span(" · ", th.border),
        span("-xt bash drops one", th.border),
    ]));
    out.push(Line::from(vec![
        span("/reload keeps the session and the transcript", th.muted),
        span(" · ", th.border),
        span("e show the error", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));
    // A long footer or extension detail is cut at the window, never wrapped:
    // the sheet reports one height and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

fn reload_line(state: State, text: &str, duration: &str, th: &Theme) -> Line<'static> {
    indent(
        2,
        vec![
            span(format!("{} ", glyph::BRANCH), th.border),
            span_strong(format!("{} ", state.glyph()), th.state_color(state), th),
            span(
                text.to_string(),
                if state == State::Failed {
                    th.text
                } else {
                    th.muted
                },
            ),
            span(format!("   {duration}"), th.border),
        ],
    )
}

fn extension_row(state: State, name: &str, detail: &str, th: &Theme) -> Vec<Span<'static>> {
    vec![
        span_strong(format!("{} ", state.glyph()), th.state_color(state), th),
        span(
            format!("{name:<20}"),
            if state == State::Failed {
                th.border
            } else {
                th.muted
            },
        ),
        span(detail.to_string(), th.border),
    ]
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::WorkshopSheet;
    use crate::davinci::theme::ColorDepth;
    use crate::davinci::ui::run_width;

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
                        "TypeError: hooks.preTool is not a function · deploy.js:41 · its 3 tools are missing"
                            .into(),
                    ),
                ),
            ],
            native: vec![
                (State::Done, "vector-memory".into(), "4 tools · 4 commands".into()),
                (State::Done, "graph".into(), "1 tool · 5 commands".into()),
            ],
            javascript: vec![
                (State::Done, "plan-mode".into(), "1 tool · registers --plan".into()),
                (State::Failed, "deploy.js · project".into(), "failed to register".into()),
            ],
            node: "node v24.19.0".into(),
            schema: "21.4k · 11%".into(),
            tools: vec![
                (
                    "built-in tools".into(),
                    "8".into(),
                    0.33,
                    "read write edit grep find ls bash pwsh".into(),
                ),
                ("native tools".into(), "14".into(), 0.58, "memory, governor".into()),
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
        assert!(rows.iter().any(|row| row.contains("> /reload")));
        assert!(rows
            .iter()
            .any(|row| row.contains("⎿ ✓ keybindings · 39 bindings")));
        assert!(rows
            .iter()
            .any(|row| row.contains("TypeError: hooks.preTool is not a function")));
        assert!(rows.iter().any(|row| row.contains("NATIVE")));
        assert!(rows.iter().any(|row| row.contains("node v24.19.0")));
        assert!(rows.iter().any(|row| row.contains("vector-memory")));
        assert!(rows
            .iter()
            .any(|row| row.contains("21.4k · 11%") && row.contains("tool schema")));
        assert!(rows.iter().any(|row| row.contains("built-in tools")));
        assert!(rows
            .iter()
            .any(|row| row.contains("/reload keeps the session")));
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
}
