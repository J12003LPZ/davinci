//! Shared confirmation and question view. Context precedes the answers;
//! focus does not imply approval, and the runtime still owns every decision.

use super::sheet::hint;
use crate::davinci::model::{AskKind, Hunk, HunkKind, Model};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{
    self, blank, clip_ellipsis, run_width, section_detail, section_row, span, truncate_run, Surface,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn instruction_window(before: &str, after: &str, width: usize) -> String {
    let budget = width.saturating_sub(3);
    let mut used = 0;
    let mut left = Vec::new();
    for grapheme in before.graphemes(true).rev() {
        if used + grapheme.width() > budget / 2 {
            break;
        }
        used += grapheme.width();
        left.push(grapheme);
    }
    left.reverse();
    let left = left.concat();
    let mut right = String::new();
    for grapheme in after.graphemes(true) {
        if used + grapheme.width() > budget {
            break;
        }
        used += grapheme.width();
        right.push_str(grapheme);
    }
    format!(
        "{}{left}▏{right}{}",
        if left.len() < before.len() { "…" } else { "" },
        if right.len() < after.len() { "…" } else { "" }
    )
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let ask = &model.ask;
    let inset = model.overlay_inset();
    let inner = model.width.saturating_sub(inset * 2).saturating_sub(4);
    if ask.key == "/permissions" {
        if let Some(input) = &model.approval_instructions {
            let editor = input.editor();
            let (before, after) = editor.buffer.split_at(editor.cursor);
            let mut body = section_detail(
                inner,
                th,
                "Deny this call and tell the model what to do instead:",
            );
            body.extend(section_detail(
                inner,
                th,
                &instruction_window(before, after, usize::from(inner)),
            ));
            body.push(ui::hint_row(
                inner,
                &[hint(th, "enter confirm denial")],
                Some("esc deny without instructions"),
                th,
            ));
            return Surface::section(model.width, th)
                .inset(inset)
                .title(vec![span("Permission · instructions", th.primary)])
                .rows(body.into_iter().map(|r| r.spans).collect())
                .lines();
        }
        return approval_lines(model);
    }
    let selected = model.selection(ask.items.len());
    let mut body = section_detail(inner, th, &ask.note);
    if ask.items.is_empty() {
        body.extend(section_detail(inner, th, "nothing to choose"));
    }
    for (index, item) in ask.items.iter().enumerate() {
        let focused = Some(index) == selected;
        let label = if ask.key == "/permissions" {
            format!("{}. {}", index + 1, item.label)
        } else {
            item.label.clone()
        };
        body.push(section_row(inner, th, focused, &label, ""));
        if focused {
            if ui::run_width(&[span(label.clone(), th.text)]) > inner.saturating_sub(3) {
                body.extend(section_detail(inner, th, &label));
            }
            body.extend(section_detail(inner, th, &item.detail));
        }
    }
    let exit = if ask.key == "/permissions" {
        "esc deny"
    } else {
        "esc close"
    };
    body.push(ui::hint_row(
        inner,
        &[
            hint(
                th,
                if ask.key == "/permissions" {
                    "↑↓/1-5 focus"
                } else {
                    "↑↓ move"
                },
            ),
            hint(
                th,
                if model.overlay_offset.is_some() {
                    "enter back"
                } else {
                    "enter select"
                },
            ),
            hint(th, "pgup/pgdn read"),
        ],
        Some(exit),
        th,
    ));
    let mut title = vec![span(ask.title.clone(), th.primary)];
    if !ask.name.is_empty() && ask.name != ask.title {
        title.push(span(format!(" · {}", ask.name), th.muted));
    }
    Surface::section(model.width, th)
        .inset(inset)
        .title(title)
        .rows(body.into_iter().map(|r| r.spans).collect())
        .lines()
}

fn approval_lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let ask = &model.ask;
    let width = model.width;
    let mut rows = vec![Line::from(span("─".repeat(width as usize), cc.permission))];

    let mut title = span(format!(" {}", ask.title), cc.permission);
    title.style = title.style.add_modifier(Modifier::BOLD);
    rows.push(Line::from(title));

    match ask.kind {
        AskKind::Shell => {
            rows.push(blank());
            for line in ask.subject.lines() {
                rows.push(Line::from(truncate_run(
                    vec![span(format!("   {line}"), th.text)],
                    width,
                )));
            }
            rows.push(blank());
        }
        AskKind::File => {
            rows.push(Line::from(span(format!(" {}", ask.subject), cc.inactive)));
            if !ask.preview.is_empty() {
                let dashes = Line::from(span("╌".repeat(width as usize), cc.subtle));
                rows.push(dashes.clone());
                let digits = ask
                    .preview
                    .iter()
                    .filter_map(|hunk| hunk.line)
                    .max()
                    .map_or(1, |line| line.to_string().len());
                rows.extend(
                    ask.preview
                        .iter()
                        .map(|hunk| approval_hunk(th, hunk, digits, width)),
                );
                rows.push(dashes);
            }
        }
        AskKind::List => {
            if !ask.note.is_empty() {
                rows.push(Line::from(span(format!(" {}", ask.note), cc.inactive)));
            }
            rows.push(blank());
        }
    }

    rows.push(question_line(th, &ask.question, &ask.subject));
    let selected = model.selection(ask.items.len());
    for (index, item) in ask.items.iter().enumerate() {
        let focused = Some(index) == selected;
        let mut row = vec![span(if focused { " ❯ " } else { "   " }, cc.permission)];
        row.push(span(format!("{}. ", index + 1), cc.inactive));
        row.push(span(
            item.label.clone(),
            if focused { cc.permission } else { th.text },
        ));
        rows.push(Line::from(truncate_run(row, width)));
    }
    rows.push(blank());
    rows.push(Line::from(span(" Esc to cancel", cc.inactive)));
    rows
}

fn question_line(theme: &Theme, question: &str, subject: &str) -> Line<'static> {
    let clean_subject = subject
        .strip_suffix(" · outside the project")
        .unwrap_or(subject);
    let name = std::path::Path::new(clean_subject)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    match (!name.is_empty()).then(|| question.find(&name)).flatten() {
        Some(at) => {
            let mut bold = span(name.clone(), theme.text);
            bold.style = bold.style.add_modifier(Modifier::BOLD);
            Line::from(vec![
                span(format!(" {}", &question[..at]), theme.text),
                bold,
                span(question[at + name.len()..].to_string(), theme.text),
            ])
        }
        None => Line::from(span(format!(" {question}"), theme.text)),
    }
}

fn approval_hunk(theme: &Theme, hunk: &Hunk, digits: usize, width: u16) -> Line<'static> {
    let cc = theme.cc();
    let number = hunk.line.map_or(String::new(), |line| line.to_string());
    let (sign, foreground, background) = match hunk.kind {
        HunkKind::Add => ("+", cc.diff_add, Some(cc.diff_add_bg)),
        HunkKind::Del => ("-", cc.diff_del, Some(cc.diff_del_bg)),
        HunkKind::Context => (" ", cc.diff_text, None),
    };
    let paint = |foreground: Color| match background {
        Some(background) => Style::default().fg(foreground).bg(background),
        None => Style::default().fg(foreground),
    };
    let mut gutter = paint(foreground);
    if background.is_none() {
        gutter = gutter.add_modifier(Modifier::DIM);
    }
    let text = clip_ellipsis(&hunk.text, width.saturating_sub(digits as u16 + 4));
    let mut spans = vec![
        Span::styled(format!(" {number:>digits$} "), gutter),
        Span::styled(sign.to_string(), paint(foreground)),
        Span::styled(text, paint(cc.diff_text)),
    ];
    if let Some(background) = background {
        let used = run_width(&spans);
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used) as usize),
            Style::default().bg(background),
        ));
    }
    Line::from(truncate_run(spans, width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{Ask, Overlay, PickerItem},
        theme::{ColorDepth, Theme},
    };
    #[test]
    fn f01_denial_editor_keeps_a_long_unicode_caret_visible() {
        let mut m = model(40);
        m.ask.key = "/permissions".into();
        m.approval_instructions = Some(format!("{}tail", "界e\u{301}".repeat(300)).into());
        let rows = lines(&m);
        assert!(rows.len() <= 10);
        let shown: String = rows
            .iter()
            .flat_map(|row| row.spans.iter().map(|span| span.content.as_ref()))
            .collect();
        assert!(shown.contains("tail▏"));
        m.approval_instructions
            .as_mut()
            .unwrap()
            .editor_mut()
            .move_line_start();
        assert!(text(&m).contains("▏界e\u{301}"));
    }
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.ask = Ask {
            title: "Project trust".into(),
            key: "/trust".into(),
            note: "Review this project's resources".into(),
            items: vec![
                PickerItem::new("Trust this folder", "Load project resources"),
                PickerItem::new("Do not trust", "Ignore project resources"),
            ],
            ..Default::default()
        };
        m.toggle_overlay(Overlay::Ask);
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
    fn context_precedes_answers_and_only_focus_expands() {
        let m = model(80);
        let text = text(&m);
        assert!(text.contains("Project trust") && !text.contains('╭'));
        assert!(
            text.find("Review this project").unwrap() < text.find("Trust this folder").unwrap()
        );
        assert!(
            text.contains("Load project resources") && !text.contains("Ignore project resources")
        );
        let rows = lines(&m);
        assert!(rows[ui::focused_row(&rows).unwrap()]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(m.theme.surface)));
        assert!(text.contains("enter select") && text.contains("esc close"));
    }
    #[test]
    fn permissions_name_the_real_cancel_semantics() {
        let mut m = model(80);
        m.ask.key = "/permissions".into();
        let shown = text(&m);
        assert!(shown.contains("Esc to cancel"));
        assert!(shown.contains("1. Trust this folder"));
        assert!(shown.contains("2. Do not trust"));
    }
    #[test]
    fn empty_choices_still_offer_an_exit() {
        let mut m = model(80);
        m.ask.items.clear();
        assert!(text(&m).contains("nothing to choose") && text(&m).contains("esc close"));
    }
    #[test]
    fn long_unicode_questions_and_options_wrap_within_the_terminal() {
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.ask.note = "项目/café/🦀/long-path ".repeat(8);
            m.ask.items[0].label = "Trust parent folder 项目/café/🦀 ".repeat(6);
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width, "{width}: {row:?}");
            }
        }
    }
}
