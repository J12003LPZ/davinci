//! Compact welcome screen with workspace facts and useful entry points.
//!
//! Keeps startup discovery from the native shell; upstream interaction lives in
//! vendor/davinci/packages/coding-agent/src/modes/interactive/interactive-mode.ts.

use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

use crate::davinci::model::{Model, Startup};
use crate::davinci::ui::{blank, clip_ellipsis, mix, span, truncate_run};

/// The masthead D, drawn in quadrant blocks on a 16×6 sub-cell grid: a
/// two-cell stem, hairline bars, and a bowl that rounds off through ▜ ▌ ▟
/// instead of meeting the stem at square corners. Every row is the same
/// eleven columns, so the text beside it lines up.
pub const MARK: [&str; 3] = [" ██▀▀▀▜▄   ", " ██    █▌  ", " ██▄▄▄▟▀   "];

/// Terracotta lit from above: the top row catches the light, the bottom
/// sits in shade. Truecolor only; other palettes draw the flat accent.
fn mark_inks(model: &Model) -> [Color; 3] {
    let cc = model.theme.cc();
    const SHADE: Color = Color::Rgb(0x8E, 0x3F, 0x27);
    [
        mix(cc.claude, cc.claude_shimmer, 0.45),
        cc.claude,
        mix(cc.claude, SHADE, 0.4),
    ]
}

/// Compact identity block, also kept above a short conversation. The path and
/// selected model come from the running session, never from sample copy.
pub fn banner(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let mut name = span("DaVinci", th.text);
    name.style = name.style.add_modifier(Modifier::BOLD);
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let cwd = if !home.is_empty() && info.cwd.starts_with(&home) {
        format!("~{}", &info.cwd[home.len()..])
    } else {
        info.cwd.clone()
    };
    let [top, middle, bottom] = mark_inks(model);
    let rows = [
        vec![
            span(MARK[0], top),
            name,
            span(format!(" v{}", env!("CARGO_PKG_VERSION")), cc.inactive),
        ],
        vec![
            span(MARK[1], middle),
            span(
                format!("{} with {} effort", model.model_name, model.thinking_level),
                cc.inactive,
            ),
        ],
        vec![
            span(MARK[2], bottom),
            span(
                clip_ellipsis(&cwd, model.width.saturating_sub(12)),
                cc.inactive,
            ),
        ],
        vec![],
        vec![
            span("  Switch models anytime with ", th.text),
            span("/model", cc.permission),
            span(". Type ", th.text),
            span("?", cc.permission),
            span(" for shortcuts.", th.text),
        ],
    ];
    rows.into_iter()
        .map(|row| Line::from(truncate_run(row, model.width)))
        .collect()
}

pub fn lines(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let mut out = vec![blank()];
    out.extend(banner(model, info));
    out
}

/// Row count includes discovered resources.
pub fn height(model: &Model) -> usize {
    lines(model, &model.startup).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            52,
            true,
        )
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn welcome_is_bounded_and_reports_its_actual_height() {
        for width in [1, 40, 64, 80, 100, 160] {
            let mut m = model(width);
            m.startup.found = vec!["loaded 1 context file · 41 skills".into()];
            let rows = lines(&m, &m.startup);
            assert_eq!(rows.len(), height(&m));
            assert!(rows.iter().all(|row| run_width(&row.spans) <= width));
        }
    }

    #[test]
    fn welcome_matches_reference_masthead_with_model_hint() {
        let mut m = model(100);
        m.startup.restored = true;
        m.startup.found = vec!["loaded 1 context file · 41 skills".into()];
        let rows: Vec<String> = lines(&m, &m.startup).iter().map(text).collect();
        assert_eq!(rows.len(), 6);
        assert!(rows[0].is_empty());
        let drawn = rows.join("\n");
        assert!(drawn.contains("DaVinci"));
        assert!(drawn.contains(&m.model_name));
        assert!(drawn.contains(&m.startup.cwd));
        assert!(!drawn.contains("session restored"));
        assert!(!drawn.contains("new session"));
        assert!(!drawn.contains("/help"));
        assert!(drawn.contains("/model"));
    }

    #[test]
    fn the_mark_is_a_d_of_even_width_lit_from_above() {
        use unicode_width::UnicodeWidthStr;
        assert!(MARK.iter().all(|row| row.width() == 11), "{MARK:?}");
        let m = model(100);
        let rows = banner(&m, &m.startup);
        let inks: Vec<_> = rows[..3].iter().map(|row| row.spans[0].style.fg).collect();
        assert_ne!(inks[0], inks[1]);
        assert_ne!(inks[1], inks[2]);
        assert_eq!(inks[1], Some(m.theme.cc().claude));

        // A 256-color palette has no in-between shades: the D is flat.
        let flat = Model::new(Theme::da_vinci(ColorDepth::Ansi256, false), 100, 52, true);
        let rows = banner(&flat, &flat.startup);
        let accent = Some(flat.theme.cc().claude);
        assert!(rows[..3].iter().all(|row| row.spans[0].style.fg == accent));
    }

    #[test]
    fn banner_uses_compact_identity_and_model_hint() {
        let m = model(100);
        let rows = banner(&m, &m.startup);
        let art = rows.iter().map(text).collect::<Vec<_>>().join("\n");
        assert_eq!(rows.len(), 5);
        assert!(art.contains("DaVinci"));
        assert!(art.contains(&m.model_name));
        assert!(!art.contains("▓"));
        assert!(!art.contains("CODE / TOOLS / CONTEXT"));
    }
}
