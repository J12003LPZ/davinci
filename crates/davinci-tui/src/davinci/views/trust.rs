//! `6a` — `/trust`. The decision a project asks for the first time you open
//! it.
//!
//! The rows are sorted by what they can do to you, not alphabetically: files
//! that execute code first, files that change limits next, prose last. The
//! composer is drawn disabled because nothing has been loaded yet — the screen
//! is the only thing on it, and it states the choice as four keys rather than
//! a yes/no.
//!
//! Mirrors artboard `6a` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use super::chrome::thousands;
use super::sheet::{facts, Composer, SheetChrome};
use crate::davinci::model::{Model, TrustFile};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{
    blank, clip_ellipsis, footnote, run_width, span, span_strong, spread, truncate_run, wrap,
    Surface, MEASURE,
};

/// Cells the path column takes.
const PATH: usize = 30;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(project) = model.project_trust.as_ref() else {
        return vec![Line::from(vec![span(
            "nothing is waiting on a trust decision",
            th.muted,
        )])];
    };

    let mut out: Vec<Line<'static>> = wrap(
        "This project ships files that would change how davinci behaves. \
         Nothing here has been read yet.",
        MEASURE.min(width),
    )
    .into_iter()
    .map(|row| Line::from(vec![span(row, th.text)]))
    .collect();
    out.push(blank());

    // Below 88 the description goes; what the file can do to you does not.
    let detail = width >= 88;
    for file in &project.files {
        out.push(row(file, detail, width, th));
    }
    out.push(blank());

    let executes = project
        .files
        .iter()
        .filter(|file| file.risk_label.contains("executes"))
        .count();
    let inner = width.saturating_sub(4);
    let mut body: Vec<Vec<Span<'static>>> = wrap(
        &format!(
            "! {} of these run code you have not read. A project you cloned \
             from a stranger can register a tool that reads your keys the \
             first time you say hello.",
            spelled(executes)
        ),
        inner,
    )
    .into_iter()
    .map(|row| vec![span(row, th.text)])
    .collect();
    body.push(Vec::new());
    body.push(
        spread(
            inner,
            vec![
                span("decision is per path", th.muted),
                span(" · ", th.border),
                span(project.path.clone(), th.text),
            ],
            vec![
                span("changeable later with ", th.muted),
                span("/trust", th.text),
            ],
        )
        .spans,
    );
    body.push(Vec::new());
    let keys: [(&str, &str, Color); 4] = [
        ("[t]", " trust, and remember", th.text),
        ("[o]", " this run only", th.muted),
        ("[p]", " prompts only, no code", th.muted),
        ("[n]", " ignore the project's files", th.border),
    ];
    let key_spans = |(key, what, color): &(&str, &str, Color)| {
        vec![
            span(
                *key,
                if *color == th.border {
                    th.border
                } else {
                    th.primary
                },
            ),
            span(*what, *color),
        ]
    };
    let one_row: Vec<Span<'static>> = keys
        .iter()
        .enumerate()
        .flat_map(|(i, key)| {
            let mut spans = if i == 0 {
                Vec::new()
            } else {
                vec![span("   ", th.border)]
            };
            spans.extend(key_spans(key));
            spans
        })
        .collect();
    if run_width(&one_row) <= inner {
        body.push(one_row);
    } else {
        for pair in keys.chunks(2) {
            let mut spans = key_spans(&pair[0]);
            if let Some(second) = pair.get(1) {
                spans.push(span("   ", th.border));
                spans.extend(key_spans(second));
            }
            body.push(spans);
        }
    }
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
        span(" · ", th.border),
        span("ignored ", th.muted),
        span(project.ignored.clone(), th.text),
        span(" · ", th.border),
        span("asked again when a path moves", th.border),
    ]));
    out.push(Line::from(vec![
        span(project.store.clone(), th.border),
        span(" · ", th.border),
        span("paths and decisions, nothing else", th.border),
    ]));
    out.extend(footnote(
        width,
        vec![
            span("--approve", th.text),
            span(" trusts for one run without asking", th.muted),
        ],
        vec![
            span("--no-approve", th.text),
            span(" is the safe default for scripts", th.muted),
        ],
        th,
    ));
    // Loose rows are cut at the window, never wrapped: the sheet reports one
    // height and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// `2` → `Two`: the warning reads as a sentence, so small counts are words.
fn spelled(count: usize) -> String {
    const WORDS: [&str; 10] = [
        "None", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine",
    ];
    WORDS
        .get(count)
        .map(|word| (*word).to_string())
        .unwrap_or_else(|| count.to_string())
}

fn row(file: &TrustFile, detail: bool, width: u16, th: &Theme) -> Line<'static> {
    let color = risk_color(&file.risk_label, th);
    let mut left = vec![
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
    let right = vec![span(file.risk_label.clone(), color)];
    if detail {
        let room = width
            .saturating_sub(run_width(&left))
            .saturating_sub(run_width(&right))
            .saturating_sub(3);
        left.push(span(clip_ellipsis(&file.detail, room), th.muted));
    }
    spread(width, left, right)
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

/// The sheet's frame (design.md §11): the project and whether it has been
/// seen before in the header, `untrusted` in the status bar with what has
/// been loaded — nothing — where the meter would go. The decision keys live
/// in the panel, so there is no hint row, and the composer is drawn disabled.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let project = model.project_trust.as_ref();
    SheetChrome {
        header_right: facts(
            th,
            vec![
                project
                    .filter(|p| !p.path.is_empty())
                    .map(|p| vec![span(p.path.clone(), th.text)])
                    .unwrap_or_default(),
                if model.branch.is_empty() {
                    Vec::new()
                } else {
                    vec![span(model.branch.clone(), th.muted)]
                },
                project
                    .filter(|p| p.first_visit)
                    .map(|_| vec![span("first visit", th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: project.map(|_| vec![span("untrusted", th.warning)]),
        status_right: project.map(|_| {
            vec![
                span("no tools loaded", th.muted),
                span(" · ", th.border),
                span(format!("0k/{}", thousands(model.context.1)), th.muted),
            ]
        }),
        hints: Vec::new(),
        escape: None,
        composer: Composer::Disabled("the composer is disabled until you decide"),
        echo: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::ProjectTrustSheet;
    use crate::davinci::theme::{ColorDepth, State};

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
            first_visit: true,
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
        let lint = rows
            .iter()
            .find(|row| row.contains("lint.js"))
            .expect("the lint row");
        assert!(lint.trim_end().ends_with("executes code"), "{lint}");
        assert!(rows.iter().any(|row| row.contains("prompt text")));
        assert!(rows.iter().any(|row| row.contains("DECIDE ONCE")));
        assert!(rows
            .iter()
            .any(|row| row.contains("[t] trust, and remember")));
        assert!(rows
            .iter()
            .any(|row| row.contains("[n] ignore the project's files")));
        assert!(rows.iter().any(|row| row
            .contains("decision is per path · C:\\dev\\clones\\vendor-cli")
            && row.contains("changeable later with /trust")));
        assert!(rows
            .iter()
            .any(|row| row
                == "trusted so far 14 projects · ignored 2 · asked again when a path moves"));
        assert!(rows
            .iter()
            .any(|row| row
                == "%USERPROFILE%\\.davinci\\trust.json · paths and decisions, nothing else"));
        assert!(rows.iter().any(
            |row| row.contains("--approve trusts for one run without asking")
                && row.contains("--no-approve is the safe default for scripts")
        ));
        // The warning counts the files that execute code, from the data, as a word.
        assert!(rows.iter().any(|row| row.contains("One of these run code")));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
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

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "6a");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "C:\\dev\\clones\\vendor-cli │ main │ first visit");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "untrusted");
        let right: String = c
            .status_right
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(right, "no tools loaded · 0k/200k");
        assert_eq!(c.escape, None);
        assert!(c.hints.is_empty());
        assert_eq!(
            c.composer,
            Composer::Disabled("the composer is disabled until you decide")
        );
        assert!(super::super::sheet::hint_row(&m, &c).is_none());
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("Two of these run code")));
    }
}
