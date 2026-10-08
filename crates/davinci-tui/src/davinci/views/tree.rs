//! `/tree`: navigable session entries with a separate focus gutter.
//! Stored entry state is never replaced by cursor focus. Tree connectors stay
//! structural, while the focused entry's full text wraps beneath its label.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{self, section_detail, section_row, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.session_tree.iter().all(|entry| entry.id.is_none()) {
        return section_detail(width, th, "No session turns to navigate yet.");
    }
    let mut rows = Vec::new();
    for (index, entry) in model.session_tree.iter().enumerate() {
        let Some(id) = &entry.id else {
            rows.push(Line::from(ui::truncate_run(
                vec![span(format!("{GUTTER}{}", entry.trunk), th.border)],
                width,
            )));
            continue;
        };
        let focused = index == model.tree_index;
        let state = if entry.state == Some(State::Active) {
            "current"
        } else {
            entry.meta.as_deref().unwrap_or("")
        };
        let label = format!(
            "{}{}  {}",
            entry.trunk,
            id,
            entry.label.as_deref().unwrap_or("")
        );
        rows.push(section_row(width, th, focused, &label, state));
        if focused {
            // Under the label text: gutter, the trunk carried down, then the
            // width of the id and the two spaces after it.
            let prefix = format!(
                "{GUTTER}{}{}",
                continuation(&entry.trunk),
                " ".repeat(id.chars().count() + 2)
            );
            let lead =
                ui::run_width(&[span(prefix.clone(), th.border)]).min(width.saturating_sub(1));
            let details = [
                entry.label.as_deref().unwrap_or(""),
                entry.detail.as_deref().unwrap_or(""),
                entry.meta.as_deref().unwrap_or(""),
            ];
            for detail in details.into_iter().filter(|value| !value.is_empty()) {
                for part in ui::wrap(detail, width.saturating_sub(lead)) {
                    rows.push(Line::from(ui::truncate_run(
                        vec![
                            span(ui::clip_ellipsis(&prefix, lead), th.border),
                            span(part, th.muted),
                        ],
                        width,
                    )));
                }
            }
        }
    }
    if !model.facts.tree_summary.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            &format!("Session: {}", model.facts.tree_summary),
        ));
    }
    rows.extend(section_detail(
        width,
        th,
        "Switching the conversation does not restore working-tree files.",
    ));
    rows.extend(section_detail(width, th, &model.facts.tree_branch_note));
    rows
}

/// The selection gutter every row starts with, so connectors on spacer and
/// detail rows sit in the same column as the ones on entry rows.
const GUTTER: &str = ui::UNSELECTED_BAR;

fn continuation(trunk: &str) -> String {
    trunk.replace("├── ", "│   ").replace("└── ", "    ")
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let count = model.session_tree.iter().filter(|r| r.id.is_some()).count();
    SheetChrome {
        header_right: vec![span(format!("{count} turns"), th.muted)],
        status_third: model
            .session_tree
            .get(model.tree_index)
            .and_then(|r| r.id.as_ref())
            .map(|id| vec![span(format!("turn {id}"), th.muted)]),
        hints: vec![hint(th, "↑↓ move"), hint(th, "enter switch")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress_screen(&mut m, "4b");
        m.width = width;
        m
    }
    #[test]
    fn tree_focus_uses_the_shared_gutter_and_does_not_invent_filters_or_historical_context() {
        let m = model(80);
        let rows = lines(&m);
        let focus = ui::focused_row(&rows).unwrap();
        assert!(rows[focus]
            .to_string()
            .contains(m.session_tree[m.tree_index].id.as_ref().unwrap()));
        let text = rows
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for unsupported in [
            "filter all",
            "nothing lost",
            "context at this point",
            "f fork",
            "working tree is ahead",
        ] {
            assert!(!text.contains(unsupported));
        }
        assert_eq!(continuation("│   ├── "), "│   │   ");
    }
    #[test]
    fn empty_and_narrow_trees_are_cell_bounded() {
        let mut m = model(80);
        m.session_tree.clear();
        assert!(lines(&m)[0].to_string().contains("No session turns"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }

    #[test]
    fn connectors_line_up_across_entry_spacer_and_detail_rows() {
        let m = model(80);
        let rows: Vec<String> = lines(&m).iter().map(Line::to_string).collect();
        // Every vertical stroke continues a connector in the row above it, and
        // the first child connector hangs straight under the root's id.
        let column = |row: &str, ch: char| row.chars().position(|c| c == ch);
        let root = rows.iter().find(|r| r.contains("01  ")).unwrap();
        let first_branch = rows.iter().find(|r| r.contains("├── 02")).unwrap();
        assert_eq!(
            column(first_branch, '├'),
            root.chars().position(|c| c == '0')
        );
        for pair in rows.windows(2) {
            let above: Vec<char> = pair[0].chars().collect();
            // A connector may hang from an entry's id, never from a digit in
            // its label, detail or time.
            let id_at = above.iter().position(|c| c.is_ascii_digit()).filter(|at| {
                above.get(at + 2..at + 4) == Some(&[' ', ' ']) && above[at + 1].is_ascii_digit()
            });
            for (index, ch) in pair[1].chars().enumerate() {
                if ch == '│' || ch == '├' || ch == '└' {
                    let joined = above.get(index).is_some_and(|c| matches!(c, '│' | '├'))
                        || id_at == Some(index);
                    assert!(
                        joined,
                        "loose connector at {index}:
{}
{}",
                        pair[0], pair[1]
                    );
                }
            }
        }
        // The focused entry's text wraps under its label, not under its id.
        let focus = ui::focused_row(&lines(&m)).unwrap();
        let label_at = |row: &str| {
            row.find("fix the store")
                .map(|at| row[..at].chars().count())
        };
        let start = label_at(&rows[focus]).expect("focused label");
        assert_eq!(label_at(&rows[focus + 1]), Some(start));
        // Every detail row under the focus starts at the same column.
        for row in rows[focus + 1..]
            .iter()
            .take_while(|row| !row.contains("── "))
        {
            let text_at = row
                .chars()
                .position(|c| !matches!(c, ' ' | '│'))
                .unwrap_or(start);
            assert_eq!(text_at, start, "{row}");
        }
    }
}
