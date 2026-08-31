//! `1f` — the model and provider picker. An overlay over a dimmed transcript;
//! it states its own exits in its footer (design.md §9).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/cogitator.ex`.

use ratatui::text::{Line, Span};

use super::memoria::picker_row;
use crate::davinci::model::Model;
use crate::davinci::ui::{span, Surface};

pub fn lines(model: &Model, config_path: &str) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let width = model.width;
    let inner = width.saturating_sub(inset).saturating_sub(4);
    let selected = model.selection(model.models.len());

    let mut body: Vec<Vec<Span<'static>>> = model
        .models
        .iter()
        .enumerate()
        .map(|(index, item)| {
            picker_row(th, inner, &item.name, &item.window, Some(index) == selected)
        })
        .collect();

    if body.is_empty() {
        // No catalog reached this session — offline, or no credentials. Say
        // which, rather than showing a picker with nothing to pick.
        body.push(vec![span(
            "no models available — /login to add a provider",
            th.muted,
        )]);
    }

    body.push(Vec::new());
    body.push(vec![
        span("configured in ", th.muted),
        span(config_path.to_string(), th.secondary),
    ]);
    body.push(vec![
        span("↑↓ move", th.border),
        span(" · ", th.border),
        span("enter select", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]);

    Surface::new(width, th)
        .inset(inset)
        .title(vec![
            span("COGITATOR", th.primary),
            span(" · ", th.border),
            span("MODEL", th.muted),
        ])
        .right(vec![span("ctrl+o", th.border)])
        .rows(body)
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Overlay;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_overlay(Overlay::Cogitator);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_session_with_no_catalog_is_told_how_to_get_one() {
        let mut m = model(100);
        m.models.clear();
        let rows: Vec<String> = lines(&m, "config.json").iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no models available")));
        assert!(rows.iter().any(|row| row.contains("/login")));
        assert!(rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn the_picker_names_itself_its_key_and_where_it_is_configured() {
        let m = model(100);
        let rows = lines(&m, "%USERPROFILE%\\.pi\\config.toml");
        let top = text(&rows[0]);
        assert!(top.contains("╭─ COGITATOR · MODEL ─"), "{top}");
        assert!(top.ends_with("─ ctrl+o ─╮"), "{top}");
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("configured in %USERPROFILE%\\.pi\\config.toml")));
        assert!(drawn.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn every_model_row_carries_its_context_window() {
        let m = model(100);
        let rows: Vec<String> = lines(&m, "config.toml").iter().map(text).collect();
        for item in &m.models {
            assert!(
                rows.iter()
                    .any(|row| row.contains(&item.name) && row.contains(&item.window)),
                "{} has no window",
                item.name
            );
        }
    }

    #[test]
    fn the_model_in_hand_is_marked_with_a_glyph() {
        let m = model(100);
        let rows = lines(&m, "config.toml");
        assert!(text(&rows[1]).contains("◉ "));
        assert!(text(&rows[2]).contains("○ "));
    }

    #[test]
    fn the_picker_is_row_exact_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m, "config.toml") {
                assert_eq!(run_width(&row.spans), width, "at {width}");
            }
        }
    }
}
