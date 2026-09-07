//! `/resume`: session names and recency, with focused session details.
//! The runtime retains discovery, ordering, paths, and resume semantics.

use ratatui::text::Line;

use super::sheet::{facts, hint, status_meter, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_row, span};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.resume_sessions.is_empty() {
        return section_detail(width, th, "No saved sessions found. A conversation creates a session when session saving is enabled.");
    }
    let selected = model.resume_index % model.resume_sessions.len();
    let mut rows = Vec::new();
    for (index, session) in model.resume_sessions.iter().enumerate() {
        let focused = index == selected;
        let value = if session.warning.is_some() {
            format!("! {}", session.touched)
        } else {
            session.touched.clone()
        };
        rows.push(section_row(width, th, focused, &session.name, &value));
        if focused {
            rows.extend(section_detail(width, th, &session.name));
            let mut details = Vec::new();
            if !session.turns.is_empty() {
                details.push(format!("{} messages", session.turns));
            }
            if !session.tokens.is_empty() {
                details.push(format!("{} tokens", session.tokens));
            }
            if !session.model.is_empty() {
                details.push(session.model.clone());
            }
            if !session.branch.is_empty() {
                details.push(format!("branch {}", session.branch));
            }
            if !session.touched.is_empty() {
                details.push(format!("updated {}", session.touched));
            }
            rows.extend(section_detail(width, th, &details.join(" · ")));
            rows.extend(section_detail(width, th, &session.note));
            if !session.commit.is_empty() {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Commit: {}", session.commit),
                ));
            }
            if !session.last.is_empty() {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Last message: {}", session.last),
                ));
            }
            rows.extend(section_detail(width, th, &session.path));
            rows.extend(section_detail(width, th, &session.size));
        }
        if let Some(warning) = &session.warning {
            let mut warning = section_detail(width, th, warning);
            for row in &mut warning {
                for s in &mut row.spans {
                    s.style.fg = Some(th.warning);
                }
            }
            rows.extend(warning);
        }
    }
    rows
}

pub fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 10.0 || (gb - gb.round()).abs() < 0.05 {
        format!("{}G", gb.round() as u64)
    } else {
        format!("{gb:.1}G")
    }
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let count = model.session_count.max(model.resume_sessions.len());
    let disk = model.facts.sessions_disk.filter(|(_, cap)| *cap > 0);
    SheetChrome {
        header_right: facts(
            th,
            vec![
                vec![span(format!("{count} sessions"), th.muted)],
                disk.map(|(used, _)| vec![span(format!("{} on disk", gigabytes(used)), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: Some(vec![span("sessions", th.muted)]),
        status_right: disk.map(|(used, cap)| {
            status_meter(
                th,
                "disk",
                used as f64 / cap as f64,
                &gigabytes(used),
                &gigabytes(cap),
            )
        }),
        hints: vec![hint(th, "↑↓ move"), hint(th, "enter resume")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::ResumeRow,
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
        m.resume_sessions = vec![
            ResumeRow {
                name: "recognizable-session".into(),
                touched: "3m ago".into(),
                turns: "42".into(),
                tokens: "~128k".into(),
                branch: "main".into(),
                last: "Fix the parser".into(),
                note: "forked from earlier work".into(),
                path: "sessions/example.jsonl".into(),
                size: "1.8 MB".into(),
                ..ResumeRow::default()
            },
            ResumeRow {
                name: "another-session".into(),
                warning: Some("! branch no longer exists".into()),
                ..ResumeRow::default()
            },
        ];
        m
    }
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn names_and_recency_are_primary_and_focus_keeps_real_details() {
        let m = model(80);
        let rows = lines(&m);
        assert!(
            rows[0].to_string().contains("recognizable-session")
                && rows[0].to_string().contains("3m ago")
        );
        for value in [
            "42 messages",
            "~128k tokens",
            "branch main",
            "Fix the parser",
            "sessions/example.jsonl",
            "1.8 MB",
            "forked from earlier work",
        ] {
            assert!(text(&m).contains(value), "{value}");
        }
        assert!(!text(&m).contains("filter sessions") && !text(&m).contains("f forks"));
    }

    #[test]
    fn warnings_remain_visible_for_unfocused_sessions() {
        let m = model(80);
        let rows = lines(&m);
        let warning = rows
            .iter()
            .find(|r| r.to_string().contains("branch no longer exists"))
            .unwrap();
        assert!(warning
            .spans
            .iter()
            .any(|s| s.style.fg == Some(m.theme.warning)));
    }

    #[test]
    fn no_disk_cap_means_no_invented_meter() {
        let mut m = model(80);
        m.facts.sessions_disk = Some((1000, 0));
        assert!(chrome(&m).status_right.is_none());
        assert_eq!(gigabytes(1_288_490_188), "1.2G");
        assert_eq!(gigabytes(8_589_934_592), "8G");
    }

    #[test]
    fn empty_and_unicode_sessions_are_width_safe() {
        let mut m = model(80);
        m.resume_sessions.clear();
        assert!(text(&m).contains("No saved sessions"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.resume_sessions[0].name = "会话 café 🦀 recognizable-session".into();
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
