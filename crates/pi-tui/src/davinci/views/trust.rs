//! `6a` — `/trust`. The decision a project asks for the first time you open
//! it.
//!
//! The rows are sorted by what they can do to you, not alphabetically: files
//! that execute code first, files that change limits next, prose last. The
//! composer is drawn disabled because nothing has been loaded yet — the screen
//! is the only thing on it, and it states the choice as four keys rather than
//! a yes/no.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/trust.ex`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, TrustFile};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{blank, clip_ellipsis, span, span_strong, wrap, Surface, MEASURE};

/// Cells the path column takes, and the right-aligned risk word.
const PATH: usize = 30;
const RISK: usize = 14;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(project) = model.project_trust.as_ref() else {
        return vec![Line::from(vec![span(
            "nothing is waiting on a trust decision",
            th.muted,
        )])];
    };

    let mut out: Vec<Line<'static>> = wrap(
        "This project ships files that would change how davinci behaves. \
         Nothing here has been read yet.",
        MEASURE,
    )
    .into_iter()
    .map(|row| Line::from(vec![span(row, th.text)]))
    .collect();
    out.push(blank());

    // Below 88 the description goes; what the file can do to you does not.
    let detail = model.width >= 88;
    for file in &project.files {
        out.push(row(file, detail, th));
    }
    out.push(blank());

    let executes = project
        .files
        .iter()
        .filter(|file| file.risk_label.contains("executes"))
        .count();
    let mut body: Vec<Vec<Span<'static>>> = wrap(
        &format!(
            "! {executes} of these run code you have not read. A project you cloned \
             from a stranger can register a tool that reads your keys the \
             first time you say hello."
        ),
        width.saturating_sub(6),
    )
    .into_iter()
    .map(|row| vec![span(row, th.text)])
    .collect();
    body.push(Vec::new());
    body.push(vec![
        span("decision is per path ", th.muted),
        span(project.path.clone(), th.text),
    ]);
    body.push(vec![
        span("changeable later with ", th.muted),
        span("/trust", th.text),
        span("   --approve trusts one run", th.border),
    ]);
    body.push(Vec::new());
    body.push(vec![
        span("[t]", th.primary),
        span(" trust, and remember", th.text),
        span("   [o]", th.primary),
        span(" this run only", th.muted),
    ]);
    body.push(vec![
        span("[p]", th.primary),
        span(" prompts only, no code", th.muted),
        span("   [n]", th.border),
        span(" ignore them", th.border),
    ]);
    out.extend(
        Surface::new(width, th)
            .border(th.warning)
            .title(vec![span("DECIDE ONCE", th.warning)])
            .rows(body)
            .lines(),
    );
    out.push(blank());

    out.push(Line::from(vec![
        span("trusted so far ", th.muted),
        span(project.trusted.clone(), th.text),
        span(" · ignored ", th.muted),
        span(project.ignored.clone(), th.text),
        span(" · asked again when a path moves", th.border),
    ]));
    out.push(Line::from(vec![
        span(project.store.clone(), th.border),
        span("  paths and decisions, nothing else", th.border),
    ]));
    // Loose rows are cut at the window, never wrapped: the sheet reports one
    // height and keeps it.
    out.into_iter()
        .map(|line| Line::from(crate::davinci::ui::truncate_run(line.spans, model.width)))
        .collect()
}

fn row(file: &TrustFile, detail: bool, th: &Theme) -> Line<'static> {
    let color = risk_color(&file.risk_label, th);
    let mut spans = vec![
        span_strong(
            format!("{} ", file.state.glyph()),
            th.state_color(file.state),
            th,
        ),
        span(
            format!(
                "{:<1$}",
                clip_ellipsis(&file.path, (PATH - 2) as u16),
                PATH - 1
            ),
            th.text,
        ),
    ];
    if detail {
        spans.push(span(
            format!("{:<41}", clip_ellipsis(&file.detail, 40)),
            th.muted,
        ));
    }
    spans.push(span(
        format!("{:>1$}", file.risk_label.clone(), RISK),
        color,
    ));
    Line::from(spans)
}

/// The risk word decides the ink: code and limits in warning, prose muted,
/// everything else quiet.
fn risk_color(risk_label: &str, th: &Theme) -> Color {
    if risk_label.contains("executes") || risk_label.contains("limits") {
        th.warning
    } else if risk_label.contains("prompt") {
        th.muted
    } else {
        th.border
    }
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::ProjectTrustSheet;
    use crate::davinci::theme::{ColorDepth, State};
    use crate::davinci::ui::run_width;

    fn sheet() -> ProjectTrustSheet {
        ProjectTrustSheet {
            files: vec![
                TrustFile {
                    state: State::Attention,
                    path: ".davinci\\extensions\\lint.js".into(),
                    detail: "runs as node, no sandbox".into(),
                    risk_label: "executes code".into(),
                },
                TrustFile {
                    state: State::Attention,
                    path: ".davinci\\settings.json".into(),
                    detail: "3 keys, incl. transport".into(),
                    risk_label: "changes limits".into(),
                },
                TrustFile {
                    state: State::Read,
                    path: "AGENTS.md · CLAUDE.md".into(),
                    detail: "1,208 lines, prepended".into(),
                    risk_label: "prompt text".into(),
                },
                TrustFile {
                    state: State::Queued,
                    path: ".davinci\\themes\\ (1)".into(),
                    detail: "colours only".into(),
                    risk_label: "harmless".into(),
                },
            ],
            path: "C:\\dev\\clones\\vendor-cli".into(),
            trusted: "14 projects".into(),
            ignored: "2".into(),
            store: "%USERPROFILE%\\.davinci\\trust.json".into(),
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.project_trust = Some(sheet());
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_file_states_what_it_can_do_and_the_choice_is_four_keys() {
        let rows: Vec<String> = lines(&model(100)).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("lint.js") && row.contains("executes code")));
        assert!(rows.iter().any(|row| row.contains("prompt text")));
        assert!(rows.iter().any(|row| row.contains("DECIDE ONCE")));
        assert!(rows
            .iter()
            .any(|row| row.contains("[t] trust, and remember")));
        assert!(rows.iter().any(|row| row.contains("[n] ignore them")));
        assert!(rows
            .iter()
            .any(|row| row.contains("trusted so far 14 projects")));
        // The warning counts the files that execute code, from the data.
        assert!(rows.iter().any(|row| row.contains("1 of these run code")));
    }

    #[test]
    fn below_eighty_eight_the_description_goes_but_the_risk_never() {
        let rows: Vec<String> = lines(&model(80)).iter().map(text).collect();
        assert!(!rows.iter().any(|row| row.contains("runs as node")));
        assert!(rows.iter().any(|row| row.contains("executes code")));
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
    fn a_session_with_no_pending_decision_says_so() {
        let mut m = model(100);
        m.project_trust = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("nothing is waiting on a trust decision")));
    }
}
