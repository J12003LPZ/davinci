//! Compact welcome screen with workspace facts and useful entry points.
//!
//! Keeps startup discovery from the native shell; upstream interaction lives in
//! vendor/davinci/packages/coding-agent/src/modes/interactive/interactive-mode.ts.

use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, Startup};
use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{blank, indent, paper_label, span, span_strong, truncate_run};

/// Compact identity block, also kept above a short conversation. The path and
/// selected model come from the running session, never from sample copy.
pub fn banner(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let facts = [
        vec![
            paper_label("DaVinci", th, true),
            span(format!(" v{}", env!("CARGO_PKG_VERSION")), th.muted),
        ],
        vec![
            span(model.model_name.clone(), th.text),
            span(format!(" · {}", model.thinking_level), th.muted),
        ],
        vec![span(info.cwd.clone(), th.muted)],
    ];
    facts
        .into_iter()
        .map(|run| {
            indent(
                1.min(model.width),
                truncate_run(run, model.width.saturating_sub(1)),
            )
        })
        .collect()
}

pub fn lines(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let content_width = width.saturating_sub(2);
    let mut rows = vec![Line::from(restored_row(th, info.restored))];
    for found in &info.found {
        rows.push(Line::from(vec![span(found.clone(), th.muted)]));
    }
    rows.push(blank());
    for (command, description) in [
        ("/graph", "Plan a task and follow its progress"),
        ("/governor-status", "View compression and token savings"),
        ("/memory-status", "Browse the memory index"),
        ("/model", "Choose and manage the active model"),
        ("/resume", "Resume a previous session"),
        ("/help", "Show all commands and shortcuts"),
    ] {
        rows.push(Line::from(vec![
            span_strong(format!("{command:<19}"), th.primary, th),
            span(if width >= 64 { description } else { "" }, th.muted),
        ]));
    }
    let mut out = vec![blank()];
    out.extend(banner(model, info));
    out.push(blank());
    out.extend(
        rows.into_iter()
            .map(|row| indent(1.min(width), truncate_run(row.spans, content_width))),
    );
    out.push(blank());
    out
}

fn restored_row(theme: &Theme, restored: bool) -> Vec<Span<'static>> {
    if restored {
        vec![
            span_strong(format!("{} ", glyph::DONE), theme.success, theme),
            span("session restored", theme.muted),
        ]
    } else {
        vec![
            span_strong(format!("{} ", glyph::QUEUED), theme.muted, theme),
            span("new session", theme.muted),
        ]
    }
}

/// Row count includes discovered resources.
pub fn height(model: &Model) -> usize {
    14 + model.startup.found.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::ColorDepth;
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
    fn welcome_keeps_session_facts_and_working_commands() {
        let mut m = model(100);
        m.startup.restored = true;
        m.startup.found = vec!["loaded 1 context file · 41 skills".into()];
        let rows: Vec<String> = lines(&m, &m.startup).iter().map(text).collect();
        let restored = rows
            .iter()
            .position(|row| row.contains("session restored"))
            .unwrap();
        assert!(rows[restored + 1].contains("loaded 1 context file · 41 skills"));
        for command in [
            "/graph",
            "/governor-status",
            "/memory-status",
            "/model",
            "/resume",
            "/help",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(command)),
                "missing {command}"
            );
        }
        assert!(!rows.iter().any(|row| row.contains("/session")));
        assert!(!rows.iter().any(|row| row.contains("/sessions")));
        m.startup.restored = false;
        assert!(lines(&m, &m.startup)
            .iter()
            .any(|row| text(row).contains("new session")));
    }

    #[test]
    fn banner_uses_compact_identity_without_editorial_masthead() {
        let m = model(100);
        let art = banner(&m, &m.startup)
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!art.contains("▓▓▓▓▓╸"));
        assert!(art.contains("DaVinci"));
        assert!(!art.contains("CODE / TOOLS / CONTEXT"));
        assert!(!art.contains("▐▛███▜▌"));
        assert!(!art.contains("▝▜█████▛▘"));
    }
}
