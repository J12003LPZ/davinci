//! `3e` — `/hotkeys`. The whole keymap, grouped by the surface a key belongs
//! to.
//!
//! The mockup sets this in two columns; on a character grid one column per
//! surface reads better and keeps every row inside the measure (design.md §3),
//! so the groups stack and the sheet scrolls with ↑↓ instead. The point the
//! screen has to make survives either way: a key means one thing per surface,
//! and ctrl+d means three different things depending on what has the keyboard.
//!
//! Unlike the transcript, this sheet is windowed from the top and says how
//! much is below it — dropping the first rows of a reference sheet to fit the
//! window would hide exactly what someone opened it to read.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/keys.ex`.

use ratatui::text::Line;

use crate::davinci::model::Model;
use crate::davinci::ui::{blank, indent, span, spread, MEASURE};

/// The key column, wide enough for `ctrl+d t u l a`.
const KEY_COLUMN: usize = 18;

/// The destructive bindings are marked wherever they are listed, so the sheet
/// never reads as a flat list of equals.
fn destructive(key: &str) -> bool {
    matches!(key, "ctrl+d" | "ctrl+backspace")
}

pub fn lines(model: &Model, rows: usize) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);

    let mut body: Vec<Line<'static>> = Vec::new();
    for group in &model.keymap {
        let right = if group.note.is_empty() {
            Vec::new()
        } else {
            vec![span(group.note.clone(), th.border)]
        };
        body.push(spread(
            width,
            vec![span(group.title.clone(), th.primary)],
            right,
        ));
        for (key, description) in &group.rows {
            let ink = if destructive(key) {
                th.warning
            } else {
                th.muted
            };
            body.push(indent(
                2,
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

    let footer: Vec<Line<'static>> = vec![
        Line::from(vec![span("a key means one thing per surface", th.muted)]),
        Line::from(vec![span(
            "ctrl+d quits here, deletes in the session list",
            th.border,
        )]),
        Line::from(vec![
            span("rebind in ", th.muted),
            span("%USERPROFILE%\\.pi\\agent\\keybindings.json", th.secondary),
        ]),
        Line::from(vec![span("esc close", th.border)]),
    ];

    // Windowed from the top: the sheet keeps its first group on screen and
    // says how much sits above and below the window.
    let room = rows.saturating_sub(footer.len() + 1).max(4);
    if body.len() <= room {
        let mut out = body;
        out.extend(footer);
        return out;
    }
    let offset = model.keys_offset.min(body.len() - room);
    let below = body.len() - offset - room;
    let mut counts: Vec<String> = Vec::new();
    if offset > 0 {
        counts.push(format!("{offset} above"));
    }
    if below > 0 {
        counts.push(format!("{below} below"));
    }
    let mut out: Vec<Line<'static>> = body[offset..offset + room].to_vec();
    out.push(Line::from(vec![
        span("↑↓ scrolls", th.border),
        span(format!("  {}", counts.join(" · ")), th.muted),
    ]));
    out.extend(footer);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn every_group_and_binding_is_listed_when_there_is_room() {
        let m = model(100);
        let rows: Vec<String> = lines(&m, 40).iter().map(text).collect();
        for expected in [
            "INSTRUMENTS",
            "over the transcript",
            "ctrl+p",
            "RUN",
            "COMPOSER",
            "shift+enter",
            "esc close",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is missing"
            );
        }
    }

    #[test]
    fn destructive_keys_are_marked_in_warning_ink() {
        let m = model(100);
        let rows = lines(&m, 40);
        let quit = rows
            .iter()
            .find(|row| text(row).contains("ctrl+d") && text(row).contains("quit"))
            .expect("the quit row is drawn");
        let description = quit.spans.last().expect("a description span");
        assert_eq!(description.style.fg, Some(m.theme.warning));
    }

    #[test]
    fn the_sheet_scrolls_from_the_top_and_counts_what_is_hidden() {
        let mut m = model(100);
        let rows: Vec<String> = lines(&m, 10).iter().map(text).collect();
        assert!(rows.len() <= 10, "windowed to the room it was given");
        assert!(
            rows.iter().any(|row| row.contains("INSTRUMENTS")),
            "the first group stays on screen at offset 0"
        );
        assert!(rows.iter().any(|row| row.contains("below")), "{rows:?}");

        m.keys_offset = 6;
        let scrolled: Vec<String> = lines(&m, 10).iter().map(text).collect();
        assert!(
            !scrolled.iter().any(|row| row.contains("INSTRUMENTS")),
            "the first group scrolled away"
        );
        assert!(
            scrolled.iter().any(|row| row.contains("above")),
            "{scrolled:?}"
        );
    }

    #[test]
    fn the_output_never_exceeds_the_rows_it_was_given() {
        let m = model(100);
        for rows in [8usize, 10, 12, 20] {
            let drawn = lines(&m, rows);
            assert!(
                drawn.len() <= rows.max(4 + 5),
                "{} lines for {rows} rows",
                drawn.len()
            );
        }
    }

    #[test]
    fn an_empty_keymap_still_says_something() {
        let mut m = model(100);
        m.keymap.clear();
        let rows: Vec<String> = lines(&m, 40).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no bindings loaded")));
    }

    #[test]
    fn no_row_overflows_the_window() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m, 40) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }

    #[test]
    fn section_headers_are_row_exact_at_the_sheet_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            let sheet = width.min(MEASURE + 14);
            let rows = lines(&m, 40);
            let header = rows
                .iter()
                .find(|row| text(row).contains("INSTRUMENTS"))
                .expect("the first group header");
            assert_eq!(run_width(&header.spans), sheet, "at {width}");
        }
    }
}
