//! The one-question instrument: a titled list, one row chosen, esc to leave.
//!
//! Trust (`/trust`), thinking level (`/thinking`) and stored credentials
//! (`/logout`) all wear this panel rather than each growing one of their own —
//! design.md §1 asks for one panel at a time, not one panel per question. Its
//! shape is Cogitator's (`1f`); only the name and the rows differ.

use ratatui::text::{Line, Span};

use super::memoria::picker_row;
use crate::davinci::model::Model;
use crate::davinci::ui::{span, Surface};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let ask = &model.ask;
    let inset = model.overlay_inset();
    let width = model.width;
    let inner = width.saturating_sub(inset).saturating_sub(4);
    let selected = model.selection(ask.items.len());

    let mut body: Vec<Vec<Span<'static>>> = ask
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            picker_row(
                th,
                inner,
                &item.label,
                &item.detail,
                Some(index) == selected,
            )
        })
        .collect();

    if body.is_empty() {
        body.push(vec![span("nothing to choose", th.muted)]);
    }

    if !ask.note.is_empty() {
        body.push(Vec::new());
        body.push(vec![span(ask.note.clone(), th.secondary)]);
    }
    body.push(vec![
        span("↑↓ move", th.border),
        span(" · ", th.border),
        span("enter select", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]);

    let mut title = vec![span(ask.title.clone(), th.primary)];
    if !ask.name.is_empty() {
        title.push(span(" · ", th.border));
        title.push(span(ask.name.clone(), th.muted));
    }

    Surface::new(width, th)
        .inset(inset)
        .title(title)
        .right(vec![span(ask.key.clone(), th.border)])
        .rows(body)
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{Ask, Overlay, PickerItem};
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.ask = Ask {
            title: "FIDES".into(),
            name: "TRUST".into(),
            key: "/trust".into(),
            note: "C:\\work\\pi-rust".into(),
            items: vec![
                PickerItem::new("trust this folder", "tools run without asking"),
                PickerItem::new("ask every time", "the safe default"),
            ],
        };
        model.toggle_overlay(Overlay::Ask);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_question_names_itself_and_states_its_exits() {
        let m = model(100);
        let rows = lines(&m);
        let top = text(&rows[0]);
        assert!(top.contains("╭─ FIDES · TRUST ─"), "{top}");
        assert!(top.ends_with("─ /trust ─╮"), "{top}");
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("esc close")));
        assert!(drawn.iter().any(|row| row.contains("C:\\work\\pi-rust")));
    }

    #[test]
    fn the_highlighted_row_is_marked_by_glyph_not_only_by_colour() {
        let m = model(100);
        let rows = lines(&m);
        assert!(text(&rows[1]).contains("◉ "), "{}", text(&rows[1]));
        assert!(text(&rows[2]).contains("○ "), "{}", text(&rows[2]));
    }

    #[test]
    fn a_question_with_no_answers_still_draws_and_still_says_how_to_leave() {
        let mut m = model(100);
        m.ask.items.clear();
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("nothing to choose")));
        assert!(rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn the_panel_is_row_exact_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m) {
                assert_eq!(run_width(&row.spans), width, "at {width}");
            }
        }
    }
}
