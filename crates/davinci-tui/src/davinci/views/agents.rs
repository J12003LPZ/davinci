//! Live task and agent control panel (`/agents`).
//!
//! Provides an inspectable control surface for every active worker:
//! what it is doing, what it owns, what it is waiting on, and how to intervene safely.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, section_row, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model
        .agents
        .as_ref()
        .filter(|sheet| !sheet.agents.is_empty())
    else {
        return section_detail(
            width,
            th,
            "No workers active. Workers appear when multi-agent tasks execute.",
        );
    };

    let mut rows = Vec::new();
    for (i, agent) in sheet.agents.iter().enumerate() {
        let is_selected = i == sheet.selected_index;

        let title = if !agent.activity.is_empty() {
            format!("{} · {}", agent.name, agent.activity)
        } else if !agent.role.is_empty() {
            format!("{} · {}", agent.name, agent.role)
        } else {
            agent.name.clone()
        };

        rows.push(section_row(width, th, is_selected, &title, &agent.elapsed));

        let owns_str = if agent.owned_paths.is_empty() {
            "owns: (none)".to_string()
        } else {
            format!("owns: {}", agent.owned_paths.join(", "))
        };
        rows.extend(section_detail(width, th, &format!("  {owns_str}")));

        let tokens_str = match (agent.usage_unknown, agent.tokens) {
            (true, _) | (false, None) => "unknown".to_string(),
            (false, Some(tok)) => {
                if tok >= 1000 {
                    format!("{}k", tok / 1000)
                } else {
                    tok.to_string()
                }
            }
        };

        let status_str = if agent.disconnected {
            format!("{} [disconnected]", agent.status)
        } else {
            agent.status.clone()
        };

        let stats_str = format!(
            "  tools: {} · tokens: {} · status: {}",
            agent.tool_count, tokens_str, status_str
        );
        rows.extend(section_detail(width, th, &stats_str));

        if let Some(waiting) = &agent.waiting_on {
            let mut waiting_rows = section_detail(width, th, &format!("  waiting on: {waiting}"));
            for row in &mut waiting_rows {
                for span in &mut row.spans {
                    span.style.fg = Some(th.warning);
                }
            }
            rows.extend(waiting_rows);
        }
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let total = model.agents.as_ref().map_or(0, |sheet| sheet.agents.len());
    let active = model.agents.as_ref().map_or(0, |sheet| {
        sheet
            .agents
            .iter()
            .filter(|a| a.state == State::Active)
            .count()
    });
    let done = model.agents.as_ref().map_or(0, |sheet| {
        sheet
            .agents
            .iter()
            .filter(|a| a.state == State::Done)
            .count()
    });

    SheetChrome {
        header_right: vec![span(format!("{total} workers"), th.muted)],
        status_third: Some(vec![span(
            format!("{active} active · {done} finished"),
            th.muted,
        )]),
        hints: vec![
            hint(th, "enter inspect"),
            hint(th, "s steer"),
            hint(th, "x stop"),
            hint(th, "r retry"),
            hint(th, "d diff"),
            hint(th, "esc close"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{AgentRow, AgentsSheet},
        theme::{ColorDepth, Theme},
    };

    fn sample_model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        let mut a1 = AgentRow::new("agent-1", "permission-engine", State::Active);
        a1.activity = "implementing classifier".into();
        a1.elapsed = "00:42".into();
        a1.owned_paths = vec!["crates/davinci-agent/src/permission*.rs".into()];
        a1.tool_count = 18;
        a1.tokens = Some(24000);
        a1.status = "working".into();

        let mut a2 = AgentRow::new("agent-2", "host-review", State::Queued);
        a2.status = "waiting on implementation".into();
        a2.waiting_on = Some("implementation".into());
        a2.usage_unknown = true;

        m.agents = Some(AgentsSheet {
            agents: vec![a1, a2],
            selected_index: 0,
        });
        m
    }

    #[test]
    fn test_empty_agents_panel() {
        let m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
        let rendered = lines(&m);
        assert!(!rendered.is_empty());
        let joined: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("No workers active"));
    }

    #[test]
    fn test_populated_agents_panel() {
        let m = sample_model(80);
        let rendered = lines(&m);
        let joined: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("permission-engine"));
        assert!(joined.contains("implementing classifier"));
        assert!(joined.contains("crates/davinci-agent/src/permission*.rs"));
        assert!(joined.contains("tools: 18"));
        assert!(joined.contains("tokens: 24k"));
        assert!(joined.contains("host-review"));
        assert!(joined.contains("waiting on: implementation"));
        assert!(joined.contains("tokens: unknown"));
    }

    #[test]
    fn test_narrow_viewport() {
        let m = sample_model(40);
        let rendered = lines(&m);
        assert!(!rendered.is_empty());
    }

    #[test]
    fn test_chrome_hints_and_counts() {
        let m = sample_model(80);
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("2 workers"));
        let status = c.status_third.unwrap();
        let status_text: String = status.iter().map(|s| s.content.as_ref()).collect();
        assert!(status_text.contains("1 active · 0 finished"));
        let hints_text: String = c
            .hints
            .iter()
            .flat_map(|h| h.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(hints_text.contains("enter inspect"));
        assert!(hints_text.contains("s steer"));
        assert!(hints_text.contains("x stop"));
        assert!(hints_text.contains("r retry"));
        assert!(hints_text.contains("d diff"));
    }
}
