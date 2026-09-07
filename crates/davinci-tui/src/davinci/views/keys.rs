//! The loaded keymap, grouped by its actual input context. Bindings and
//! descriptions wrap instead of hiding behind a fixed keyboard column.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_heading, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.keymap.is_empty() {
        return section_detail(width, th, "No keymap details available.");
    }
    let mut rows = Vec::new();
    for group in &model.keymap {
        rows.extend(section_heading(width, th, &group.title));
        rows.extend(section_detail(width, th, &group.note));
        for (key, description) in &group.rows {
            rows.extend(section_detail(width, th, &format!("{key}  {description}")));
        }
    }
    rows.extend(section_detail(
        width,
        th,
        "Bindings apply to the input context listed above.",
    ));
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let bindings = model
        .facts
        .keys_count
        .max(model.keymap.iter().map(|group| group.rows.len()).sum());
    let groups = model.facts.keys_surfaces.max(model.keymap.len());
    SheetChrome {
        header_right: vec![span(
            format!("{bindings} bindings · {groups} contexts"),
            th.muted,
        )],
        status_third: Some(vec![span("keyboard reference", th.muted)]),
        hints: vec![hint(th, "↑↓ scroll")],
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
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress_screen(&mut m, "3e");
        m.width = width;
        m
    }
    #[test]
    fn the_keymap_uses_loaded_bindings_without_inventing_paths_or_reload_behavior() {
        let m = model(160);
        let drawn = lines(&m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for group in &m.keymap {
            assert!(drawn.contains(&group.title));
            for (key, description) in &group.rows {
                assert!(drawn.contains(key) && drawn.contains(description));
            }
        }
        assert!(!drawn.contains("%USERPROFILE%") && !drawn.contains("/reload re-reads"));
    }
    #[test]
    fn empty_and_narrow_references_are_bounded() {
        let mut m = model(80);
        m.keymap.clear();
        assert!(lines(&m)[0].to_string().contains("No keymap"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
