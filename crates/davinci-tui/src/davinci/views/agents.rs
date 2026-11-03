//! Live task and agent control panel (`/agents`).
//!
//! Provides an inspectable control surface for every active worker:
//! what it is doing, what it owns, what it is waiting on, and how to intervene safely.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, section_row, span};
use ratatui::text::Line;

/// `24k`, `~24k` when part of it is an estimate, `unknown` when nothing was
/// reported or estimated. Never `0` for a worker that has not been counted.
pub fn tokens_cell(agent: &crate::davinci::model::AgentRow) -> String {
    match (agent.usage_unknown, agent.tokens) {
        (true, _) | (false, None) => "unknown".to_string(),
        (false, Some(tokens)) => {
            let count = if tokens >= 1000 {
                format!("{}k", tokens / 1000)
            } else {
                tokens.to_string()
            };
            if agent.tokens_estimated {
                format!("~{count}")
            } else {
                count
            }
        }
    }
}

/// Names, activity and paths come from model-chosen tool arguments and agent
/// names: one line of plain text, no control bytes.
fn safe(text: &str) -> String {
    crate::davinci::sanitize::terminal_safe(text).replace('\n', " ")
}

fn classical_lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model
        .agents
        .as_ref()
        .filter(|sheet| !sheet.agents.is_empty() || !sheet.profiles.is_empty())
    else {
        return section_detail(
            width,
            th,
            "No agents defined. Add profiles under .davinci/agents/ or ~/.davinci/agent/agents/.",
        );
    };

    let mut rows = Vec::new();
    if sheet.agents.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            "No workers active. Workers appear when multi-agent tasks execute.",
        ));
    }
    for (i, agent) in sheet.agents.iter().enumerate() {
        let is_selected = i == sheet.selected_index;

        let title = if !agent.activity.is_empty() {
            format!("{} · {}", safe(&agent.name), safe(&agent.activity))
        } else if !agent.role.is_empty() {
            format!("{} · {}", safe(&agent.name), safe(&agent.role))
        } else {
            safe(&agent.name)
        };

        rows.push(section_row(
            width,
            th,
            is_selected,
            &title,
            &safe(&agent.elapsed),
        ));

        let owns_str = if agent.owned_paths.is_empty() {
            "owns: (none)".to_string()
        } else {
            format!("owns: {}", safe(&agent.owned_paths.join(", ")))
        };
        rows.extend(section_detail(width, th, &format!("  {owns_str}")));

        let tokens_str = tokens_cell(agent);

        let status_str = if agent.disconnected {
            format!("{} [disconnected]", safe(&agent.status))
        } else {
            safe(&agent.status)
        };

        let stats_str = format!(
            "  tools: {} · tokens: {} · status: {}",
            agent.tool_count, tokens_str, status_str
        );
        rows.extend(section_detail(width, th, &stats_str));

        if let Some(waiting) = &agent.waiting_on {
            let mut waiting_rows =
                section_detail(width, th, &format!("  waiting on: {}", safe(waiting)));
            for row in &mut waiting_rows {
                for span in &mut row.spans {
                    span.style.fg = Some(th.warning);
                }
            }
            rows.extend(waiting_rows);
        }
    }
    if !sheet.profiles.is_empty() {
        if !rows.is_empty() {
            rows.push(Line::default());
        }
        rows.extend(section_detail(
            width,
            th,
            &format!("Available agents ({})", sheet.profiles.len()),
        ));
        for profile in &sheet.profiles {
            rows.push(section_row(width, th, false, &safe(&profile.name), ""));
            let description = if profile.description.is_empty() {
                "(no description)".to_string()
            } else {
                safe(&profile.description)
            };
            rows.extend(section_detail(width, th, &format!("  {description}")));
        }
    }
    rows
}

fn classical_chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let total = model.agents.as_ref().map_or(0, |sheet| sheet.agents.len());
    let defined = model
        .agents
        .as_ref()
        .map_or(0, |sheet| sheet.profiles.len());
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
        header_right: vec![span(
            if defined == 0 {
                format!("{total} workers")
            } else {
                format!("{total} workers · {defined} defined")
            },
            th.muted,
        )],
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

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    if !model.reference {
        return classical_lines(model);
    }
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model.agents.as_ref() else {
        return empty_state(width, th);
    };
    let matches = |agent: &crate::davinci::model::AgentRow| sheet.matches(agent);
    if sheet.agents.is_empty() && sheet.profiles.is_empty() {
        let mut rows = counter_rows(model, sheet, width, th);
        if sheet.query.is_empty() {
            rows.extend(empty_state(width, th));
        } else {
            rows.extend(section_detail(width, th, &format!("Find: {}", sheet.query)));
            rows.extend(section_detail(
                width,
                th,
                &format!("No agents match {:?}.", sheet.query),
            ));
        }
        return rows;
    }
    let pinned: Vec<&crate::davinci::model::AgentRow> = sheet
        .agents
        .iter()
        .filter(|agent| sheet.pinned.contains(&agent.id) && matches(agent))
        .collect();
    let needs: Vec<&crate::davinci::model::AgentRow> = sheet
        .agents
        .iter()
        .filter(|agent| {
            !sheet.pinned.contains(&agent.id)
                && (agent.waiting_on.is_some() || agent.state == State::Attention)
                && matches(agent)
        })
        .collect();
    let working: Vec<&crate::davinci::model::AgentRow> = sheet
        .agents
        .iter()
        .filter(|agent| {
            !sheet.pinned.contains(&agent.id)
                && agent.waiting_on.is_none()
                && agent.state != State::Attention
                && matches!(agent.state, State::Active | State::Queued)
                && matches(agent)
        })
        .collect();
    let completed: Vec<&crate::davinci::model::AgentRow> = sheet
        .agents
        .iter()
        .filter(|agent| {
            !sheet.pinned.contains(&agent.id)
                && agent.waiting_on.is_none()
                && agent.state != State::Attention
                && !matches!(agent.state, State::Active | State::Queued)
                && matches(agent)
        })
        .collect();
    if pinned.is_empty()
        && needs.is_empty()
        && working.is_empty()
        && completed.is_empty()
        && !sheet.query.is_empty()
    {
        return section_detail(width, th, &format!("No agents match {:?}.", sheet.query));
    }

    let mut rows = counter_rows(model, sheet, width, th);
    if !sheet.query.is_empty() {
        rows.extend(section_detail(width, th, &format!("Find: {}", sheet.query)));
    }
    for (heading, group) in [
        ("Pinned", pinned),
        ("Needs input", needs),
        ("Working", working),
        ("Completed", completed),
    ] {
        if group.is_empty() {
            continue;
        }
        rows.extend(section_detail(
            width,
            th,
            &format!("{heading} {}", group.len()),
        ));
        for agent in group {
            render_agent(&mut rows, sheet, agent, width, th);
        }
    }

    if !sheet.profiles.is_empty() {
        if !rows.is_empty() {
            rows.push(Line::default());
        }
        rows.extend(section_detail(
            width,
            th,
            &format!("Available agents ({})", sheet.profiles.len()),
        ));
        for profile in &sheet.profiles {
            rows.push(section_row(width, th, false, &safe(&profile.name), ""));
            let description = if profile.description.is_empty() {
                "(no description)".to_string()
            } else {
                safe(&profile.description)
            };
            rows.extend(section_detail(width, th, &format!("  {description}")));
        }
    }
    rows
}

/// The reference dashboard's header counters. They are drawn as the first body
/// row because the reference shell's banner has no sheet header run.
fn counter_rows(
    model: &Model,
    sheet: &crate::davinci::model::AgentsSheet,
    width: u16,
    th: &crate::davinci::theme::Theme,
) -> Vec<Line<'static>> {
    if !model.reference {
        return Vec::new();
    }
    let needs = sheet
        .agents
        .iter()
        .filter(|a| a.waiting_on.is_some() || a.state == State::Attention)
        .count();
    let working = sheet
        .agents
        .iter()
        .filter(|a| {
            a.waiting_on.is_none()
                && a.state != State::Attention
                && matches!(a.state, State::Active | State::Queued)
        })
        .count();
    let done = sheet.agents.len().saturating_sub(needs + working);
    section_detail(
        width,
        th,
        &format!("{needs} awaiting input · {working} working · {done} completed"),
    )
}

fn empty_state(width: u16, th: &crate::davinci::theme::Theme) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (heading, description) in [
        (
            "Needs input",
            "Sessions with a question or a decision for you",
        ),
        (
            "Working",
            "Sessions still running; closing this view never stops them",
        ),
        ("Completed", "Finished sessions wait here for you to review"),
    ] {
        rows.extend(section_detail(width, th, heading));
        rows.extend(section_detail(width, th, description));
        rows.push(Line::default());
    }
    rows.extend(section_detail(
        width,
        th,
        "Describe a task below to start a session, or open /agents from a run.",
    ));
    rows
}

fn render_agent(
    rows: &mut Vec<Line<'static>>,
    sheet: &crate::davinci::model::AgentsSheet,
    agent: &crate::davinci::model::AgentRow,
    width: u16,
    th: &crate::davinci::theme::Theme,
) {
    let index = sheet
        .agents
        .iter()
        .position(|candidate| candidate.id == agent.id)
        .unwrap_or(0);
    let is_selected = index == sheet.selected_index;
    let title = if !agent.activity.is_empty() {
        format!("{} · {}", safe(&agent.name), safe(&agent.activity))
    } else if !agent.role.is_empty() {
        format!("{} · {}", safe(&agent.name), safe(&agent.role))
    } else {
        safe(&agent.name)
    };
    rows.push(section_row(
        width,
        th,
        is_selected,
        &title,
        &safe(&agent.elapsed),
    ));
    let owns = if agent.owned_paths.is_empty() {
        "owns: (none)".to_string()
    } else {
        format!("owns: {}", safe(&agent.owned_paths.join(", ")))
    };
    rows.extend(section_detail(width, th, &format!("  {owns}")));
    let status = if agent.disconnected {
        format!("{} [disconnected]", safe(&agent.status))
    } else {
        safe(&agent.status)
    };
    rows.extend(section_detail(
        width,
        th,
        &format!(
            "  tools: {} · tokens: {} · status: {}",
            agent.tool_count,
            tokens_cell(agent),
            status
        ),
    ));
    if let Some(waiting) = &agent.waiting_on {
        rows.extend(section_detail(
            width,
            th,
            &format!("  waiting on: {}", safe(waiting)),
        ));
    }
    if let Some((id, name)) = &sheet.renaming {
        if id == &agent.id {
            rows.extend(section_detail(width, th, &format!("  rename: {name}█")));
        }
    }
}

pub fn chrome(model: &Model) -> SheetChrome {
    if !model.reference {
        return classical_chrome(model);
    }
    let th = &model.theme;
    let total = model.agents.as_ref().map_or(0, |sheet| sheet.agents.len());
    let defined = model
        .agents
        .as_ref()
        .map_or(0, |sheet| sheet.profiles.len());
    let needs = model.agents.as_ref().map_or(0, |sheet| {
        sheet
            .agents
            .iter()
            .filter(|a| a.waiting_on.is_some() || a.state == State::Attention)
            .count()
    });
    let active = model.agents.as_ref().map_or(0, |sheet| {
        sheet
            .agents
            .iter()
            .filter(|a| {
                a.waiting_on.is_none()
                    && a.state != State::Attention
                    && matches!(a.state, State::Active | State::Queued)
            })
            .count()
    });
    let done = model.agents.as_ref().map_or(0, |sheet| {
        let needs = sheet
            .agents
            .iter()
            .filter(|a| a.waiting_on.is_some() || a.state == State::Attention)
            .count();
        let active = sheet
            .agents
            .iter()
            .filter(|a| {
                a.waiting_on.is_none()
                    && a.state != State::Attention
                    && matches!(a.state, State::Active | State::Queued)
            })
            .count();
        sheet.agents.len().saturating_sub(needs + active)
    });

    SheetChrome {
        header_right: vec![span(
            if model.reference {
                format!("{needs} awaiting input · {active} working · {done} completed")
            } else if defined == 0 {
                format!("{total} workers")
            } else {
                format!("{total} workers · {defined} defined")
            },
            th.muted,
        )],
        status_third: Some(vec![span(
            format!("{active} active · {done} finished"),
            th.muted,
        )]),
        hints: if model.reference {
            vec![
                hint(th, "ctrl+r rename"),
                hint(th, "ctrl+t pin"),
                hint(th, "ctrl+f find"),
                hint(th, "ctrl+x delete"),
            ]
        } else {
            vec![
                hint(th, "enter inspect"),
                hint(th, "s steer"),
                hint(th, "x stop"),
                hint(th, "r retry"),
                hint(th, "d diff"),
                hint(th, "esc close"),
            ]
        },
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{AgentProfileRow, AgentRow, AgentsSheet},
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
            ..Default::default()
        });
        m
    }

    #[test]
    fn hostile_agent_text_cannot_inject_terminal_controls() {
        let mut m = sample_model(80);
        let hostile = "x\u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[2J\r\nfake: approved";
        let sheet = m.agents.as_mut().unwrap();
        sheet.agents[0].name = hostile.into();
        sheet.agents[0].activity = hostile.into();
        sheet.agents[0].owned_paths = vec![hostile.into()];
        sheet.agents[0].status = hostile.into();
        sheet.agents[1].waiting_on = Some(hostile.into());
        sheet.profiles = vec![AgentProfileRow {
            name: hostile.into(),
            description: hostile.into(),
        }];
        for line in lines(&m) {
            for span in &line.spans {
                assert!(
                    !span.content.chars().any(|ch| ch.is_control()),
                    "control byte reached a cell: {:?}",
                    span.content
                );
            }
        }
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
        assert!(joined.contains("No agents defined"));
    }

    #[test]
    fn defined_profiles_show_when_no_worker_runs() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
        m.agents = Some(AgentsSheet {
            profiles: vec![AgentProfileRow {
                name: "code-improver".into(),
                description: "Reviews diffs for simplifications".into(),
            }],
            ..Default::default()
        });
        let joined: String = lines(&m)
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("No workers active"));
        assert!(joined.contains("Available agents (1)"));
        assert!(joined.contains("code-improver"));
        assert!(joined.contains("Reviews diffs for simplifications"));
        let header: String = chrome(&m)
            .header_right
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(header, "0 workers · 1 defined");
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
