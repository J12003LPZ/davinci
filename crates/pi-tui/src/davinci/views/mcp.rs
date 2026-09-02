//! `/mcp`. Connected MCP servers, their transport, and whether they answered.
//!
//! No TypeScript counterpart. Phase 4 spec:
//! `docs/superpowers/specs/2026-09-01-native-mcp-design.md`.

use ratatui::text::Line;

use super::sheet::{facts, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{blank, span, span_strong, wrap, MEASURE};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(sheet) = model.mcp.as_ref() else {
        return vec![Line::from(vec![span(
            "no MCP servers configured — edit ~/.pi/agent/mcp.json",
            th.muted,
        )])];
    };

    let mut out: Vec<Line<'static>> = Vec::new();

    if sheet.servers.is_empty() {
        out.push(Line::from(vec![span("no servers in mcp.json", th.muted)]));
    } else {
        for server in &sheet.servers {
            out.push(row(server, th));
            if let Some(error) = &server.error {
                for line in wrap(error, MEASURE.saturating_sub(6)) {
                    out.push(Line::from(vec![
                        span("      ", th.muted),
                        span(line, th.error),
                    ]));
                }
            }
        }
    }

    out.push(blank());
    let connected = sheet
        .servers
        .iter()
        .filter(|server| server.status == "connected")
        .count();
    let tools: usize = sheet.servers.iter().map(|server| server.tools).sum();
    out.push(Line::from(vec![span(
        format!(
            "{connected} of {} connected · {tools} tools",
            sheet.servers.len()
        ),
        th.muted,
    )]));
    if !sheet.config_path.is_empty() {
        out.push(Line::from(vec![
            span("edit ", th.muted),
            span(sheet.config_path.clone(), th.text),
        ]));
    }
    out
}

fn row(server: &crate::davinci::model::McpServerRow, th: &Theme) -> Line<'static> {
    let (state, color) = match server.status.as_str() {
        "connected" => (State::Done, th.success),
        "disabled" => (State::Skipped, th.border),
        _ => (State::Failed, th.error),
    };
    Line::from(vec![
        span(format!("{} ", glyph::BRANCH), th.border),
        span_strong(format!("{} ", state.glyph()), color, th),
        span(format!("{:<16}", server.name), th.text),
        span(format!("{:<8}", server.transport), th.muted),
        span(
            if server.status == "connected" {
                if server.tools == 1 {
                    "1 tool".into()
                } else {
                    format!("{} tools", server.tools)
                }
            } else {
                server.status.clone()
            },
            color,
        ),
    ])
}

/// The sheet's frame (design.md §11): servers and tools in the header, how
/// many answered in the status bar, the command echoed and offered again.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let sheet = model.mcp.as_ref();
    let connected = sheet.map(|s| {
        s.servers
            .iter()
            .filter(|server| server.status == "connected")
            .count()
    });
    let tools: usize = sheet
        .map(|s| s.servers.iter().map(|server| server.tools).sum())
        .unwrap_or(0);
    SheetChrome {
        header_right: facts(
            th,
            vec![
                sheet
                    .map(|s| vec![span(format!("{} servers", s.servers.len()), th.muted)])
                    .unwrap_or_default(),
                sheet
                    .map(|_| vec![span(format!("{tools} tools"), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: connected.map(|n| vec![span(format!("{n} connected"), th.muted)]),
        status_right: None,
        hints: Vec::new(),
        escape: Some("esc close"),
        composer: Composer::Prompt("/mcp"),
        echo: Some("/mcp".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{McpServerRow, McpSheet, Model};
    use crate::davinci::theme::{ColorDepth, Theme};

    #[test]
    fn a_connected_and_an_error_row_draw() {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            100,
            24,
            false,
        );
        model.mcp = Some(McpSheet {
            servers: vec![
                McpServerRow {
                    name: "memory".into(),
                    transport: "stdio".into(),
                    status: "connected".into(),
                    tools: 3,
                    error: None,
                },
                McpServerRow {
                    name: "docs".into(),
                    transport: "http".into(),
                    status: "error".into(),
                    tools: 0,
                    error: Some("connection refused".into()),
                },
            ],
            config_path: "~/.pi/agent/mcp.json".into(),
        });
        let drawn: Vec<String> = lines(&model)
            .into_iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        let blob = drawn.join("\n");
        assert!(!blob.contains("> /mcp"), "{blob}");
        assert!(blob.contains("memory"), "{blob}");
        assert!(blob.contains("3 tools"), "{blob}");
        assert!(blob.contains("docs"), "{blob}");
        assert!(blob.contains("error"), "{blob}");
        assert!(blob.contains("connection refused"), "{blob}");
        assert!(blob.contains("1 of 2 connected"), "{blob}");
        let c = chrome(&model);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "2 servers │ 3 tools");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "1 connected");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Prompt("/mcp"));
        assert_eq!(c.echo.as_deref(), Some("/mcp"));
    }
}
