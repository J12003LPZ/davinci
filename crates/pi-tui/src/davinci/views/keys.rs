//! `3e` — `/hotkeys`. The whole keymap, grouped by the surface a key belongs
//! to.
//!
//! The artboard sets this in two columns; on a character grid one column per
//! surface reads better and keeps every row inside the measure (design.md
//! §3), so the groups stack and the sheet scrolls with ↑↓ instead. The point
//! the screen has to make survives either way: a key means one thing per
//! surface, and ctrl+d means three different things depending on what has the
//! keyboard.
//!
//! Mirrors artboard `3e` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::Line;

use super::sheet::{facts, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{blank, footnote, indent, span, truncate_run};

/// The key column, wide enough for `ctrl+d t u l a`.
const KEY_COLUMN: usize = 18;
/// The rows sit where a selection bar would, so the groups line up with the
/// other sheets' tables.
const LEAD: u16 = 5;

/// The destructive bindings are marked wherever they are listed, so the sheet
/// never reads as a flat list of equals.
fn destructive(key: &str) -> bool {
    matches!(key, "ctrl+d" | "ctrl+backspace")
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;

    let mut body: Vec<Line<'static>> = Vec::new();
    for group in &model.keymap {
        // `INSTRUMENTS · OVER THE TRANSCRIPT`: the surface, then where it is.
        let mut title = vec![span(group.title.clone(), th.primary)];
        if !group.note.is_empty() {
            title.push(span(" · ", th.border));
            title.push(span(group.note.to_uppercase(), th.border));
        }
        body.push(Line::from(title));
        for (key, description) in &group.rows {
            let ink = if destructive(key) {
                th.warning
            } else {
                th.muted
            };
            body.push(indent(
                LEAD,
                vec![
                    span(format!("{key:<KEY_COLUMN$}"), th.text),
                    span(description.clone(), ink),
                ],
            ));
        }
        body.push(blank());
    }
    if model.keymap.is_empty() {
        body.push(Line::from(vec![span(
            "no bindings loaded — the defaults apply",
            th.muted,
        )]));
        body.push(blank());
    }

    body.extend(footnote(
        width,
        vec![span("a key means one thing per surface", th.muted)],
        vec![span(
            "ctrl+d quits the shell, deletes in the session list",
            th.border,
        )],
        th,
    ));
    body.push(Line::from(vec![
        span("rebind in ", th.muted),
        span("%USERPROFILE%\\.pi\\agent\\keybindings.json", th.secondary),
    ]));

    body.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): how many bindings over how many
/// surfaces, where they come from, and that `/reload` re-reads them. The
/// artboard draws no hint row beyond the exit; the sheet scrolls with ↑↓.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    // The keymap file may hold more than the sheet lists; the file's count
    // wins when the opener read it.
    let bindings = if model.facts.keys_count > 0 {
        model.facts.keys_count
    } else {
        model.keymap.iter().map(|group| group.rows.len()).sum()
    };
    let surfaces = if model.facts.keys_surfaces > 0 {
        model.facts.keys_surfaces
    } else {
        model.keymap.len()
    };
    SheetChrome {
        header_right: facts(
            th,
            vec![
                (bindings > 0)
                    .then(|| vec![span(format!("{bindings} bindings"), th.muted)])
                    .unwrap_or_default(),
                (surfaces > 0)
                    .then(|| vec![span(format!("{surfaces} surfaces"), th.muted)])
                    .unwrap_or_default(),
                vec![span("keybindings.json", th.secondary)],
            ],
        ),
        status_third: Some(vec![span("/reload re-reads them", th.muted)]),
        status_right: None,
        hints: Vec::new(),
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::KeymapGroup;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.keymap = vec![
            KeymapGroup {
                title: "INSTRUMENTS".into(),
                note: "over the transcript".into(),
                rows: vec![
                    ("ctrl+p".into(), "instrumenta · palette".into()),
                    ("ctrl+s".into(), "memoria · sessions".into()),
                    ("esc".into(), "close whichever is open".into()),
                ],
            },
            KeymapGroup {
                title: "RUN".into(),
                note: "while the agent works".into(),
                rows: vec![
                    ("ctrl+c".into(), "interrupt the run · never the app".into()),
                    ("ctrl+d".into(), "quit".into()),
                ],
            },
            KeymapGroup {
                title: "COMPOSER".into(),
                note: String::new(),
                rows: vec![
                    ("enter".into(), "send".into()),
                    ("shift+enter".into(), "newline · also ctrl+j".into()),
                ],
            },
        ];
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_group_and_binding_is_listed() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        for expected in [
            "INSTRUMENTS · OVER THE TRANSCRIPT",
            "ctrl+p",
            "RUN · WHILE THE AGENT WORKS",
            "COMPOSER",
            "shift+enter",
            "a key means one thing per surface",
            "keybindings.json",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is missing"
            );
        }
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn destructive_keys_are_marked_in_warning_ink() {
        let m = model(100);
        let rows = lines(&m);
        let quit = rows
            .iter()
            .find(|row| text(row).contains("ctrl+d") && text(row).contains("quit"))
            .expect("the quit row is drawn");
        let description = quit.spans.last().expect("a description span");
        assert_eq!(description.style.fg, Some(m.theme.warning));
    }

    #[test]
    fn an_empty_keymap_still_says_something() {
        let mut m = model(100);
        m.keymap.clear();
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no bindings loaded")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "3e");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "39 bindings │ 4 surfaces │ keybindings.json");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "/reload re-reads them");
        assert!(c.hints.is_empty());
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Hidden);
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
        assert_eq!(hint.trim_start(), "esc close");
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
