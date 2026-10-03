//! MCP server status, using actual transports, tool counts and errors.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::ui::{section_detail, section_state, span};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = &model.mcp else {
        return section_detail(
            width,
            th,
            "MCP server status is unavailable. Check your MCP configuration.",
        );
    };
    let mut rows = Vec::new();
    if sheet.servers.is_empty() {
        rows.extend(section_detail(width, th, "No MCP servers configured."));
    }
    for server in &sheet.servers {
        let state = match server.status.as_str() {
            "connected" => State::Done,
            "disabled" => State::Skipped,
            "connecting" | "pending" => State::Active,
            _ => State::Failed,
        };
        rows.extend(section_state(width, th, state, &server.name));
        rows.extend(section_detail(
            width,
            th,
            &format!(
                "{} · {} · {} tools",
                server.status, server.transport, server.tools
            ),
        ));
        if let Some(error) = &server.error {
            let mut error = section_detail(width, th, error);
            for row in &mut error {
                for span in &mut row.spans {
                    span.style.fg = Some(th.error);
                }
            }
            rows.extend(error);
        }
    }
    if !sheet.config_path.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            &format!("Config: {}", sheet.config_path),
        ));
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    SheetChrome {
        header_right: model
            .mcp
            .as_ref()
            .map(|sheet| {
                vec![span(
                    format!(
                        "{} servers · {} tools",
                        sheet.servers.len(),
                        sheet
                            .servers
                            .iter()
                            .map(|server| server.tools)
                            .sum::<usize>()
                    ),
                    th.muted,
                )]
            })
            .unwrap_or_default(),
        status_third: model.mcp.as_ref().map(|sheet| {
            vec![span(
                format!(
                    "{} connected",
                    sheet
                        .servers
                        .iter()
                        .filter(|server| server.status == "connected")
                        .count()
                ),
                th.muted,
            )]
        }),
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
        model::{McpServerRow, McpSheet},
        theme::{ColorDepth, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.mcp = Some(McpSheet {
            config_path: "project/mcp.json".into(),
            servers: vec![McpServerRow {
                name: "long-server-name".into(),
                transport: "stdio".into(),
                status: "failed".into(),
                tools: 0,
                error: Some("Cannot start server: executable not found".into()),
            }],
        });
        m
    }
    #[test]
    fn statuses_and_errors_keep_actual_names_counts_and_config_paths() {
        let m = model(80);
        let drawn = lines(&m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for value in [
            "long-server-name",
            "stdio",
            "failed",
            "0 tools",
            "executable not found",
            "project/mcp.json",
        ] {
            assert!(drawn.contains(value));
        }
        assert!(!drawn.contains("~/.pi"));
        assert!(ui::focused_row(&lines(&m)).is_none());
    }
    #[test]
    fn unavailable_empty_and_narrow_servers_are_bounded() {
        let mut m = model(80);
        m.mcp = None;
        assert!(lines(&m)[0].to_string().contains("unavailable"));
        m.mcp = Some(McpSheet::default());
        assert!(lines(&m)[0].to_string().contains("No MCP servers"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
