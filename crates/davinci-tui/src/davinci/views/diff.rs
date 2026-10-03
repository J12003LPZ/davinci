//! Read-only file review. Each focused file owns its adjacent hunk; paths,
//! test results and whitespace-preserving code continuations fit narrow screens.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::ui::{self, section_detail, section_row, section_state, span};
use crate::davinci::{
    model::{HunkKind, Model},
    theme::Theme,
};
use ratatui::{style::Color, text::Line};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(review) = model
        .review
        .as_ref()
        .filter(|review| !review.files.is_empty())
    else {
        return section_detail(width, th, "There are no changes to review.");
    };
    let selected = model.diff_index % review.files.len();
    let mut rows = Vec::new();
    for (index, file) in review.files.iter().enumerate() {
        let focused = index == selected;
        rows.push(section_row(
            width,
            th,
            focused,
            &file.path,
            &format!("{} {}", plus(file.adds), minus(file.dels)),
        ));
        if !focused {
            continue;
        }
        rows.extend(section_detail(width, th, &file.path));
        rows.extend(section_detail(
            width,
            th,
            &format!(
                "Changes: {} {} · {}",
                plus(file.adds),
                minus(file.dels),
                file.state.glyph()
            ),
        ));
        if !file.tests.is_empty() {
            rows.extend(section_state(width, th, file.test_state, &file.tests));
        }
        rows.extend(section_detail(width, th, &file.hunk_note));
        if file.hunk.is_empty() {
            rows.extend(section_detail(
                width,
                th,
                "No hunk content available for this file.",
            ));
        }
        rows.extend(section_detail(width, th, &file.hunk_header));
        let language = super::highlight::language_of(&file.path);
        for hunk in &file.hunk {
            let room = width.saturating_sub(5);
            for (part_index, part) in code_chunks(&hunk.text, room).into_iter().enumerate() {
                let prefix = if part_index == 0 {
                    marker(hunk.kind)
                } else {
                    "↳ "
                };
                let mut spans = vec![
                    span("   ", th.muted),
                    span(prefix, marker_color(hunk.kind, th)),
                ];
                if hunk.kind == HunkKind::Context {
                    spans.push(span(part, th.muted));
                } else {
                    spans.extend(super::highlight::spans(th, language, &part, th.text));
                }
                rows.push(Line::from(ui::truncate_run(spans, width)));
            }
        }
    }
    if !review.warning.is_empty() {
        rows.extend(section_detail(width, th, &format!("! {}", review.warning)));
    }
    for (label, value) in [
        ("Tests", &review.tests),
        ("Branch", &review.branch),
        ("Branch status", &review.behind),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows
}

fn plus(count: Option<u32>) -> String {
    count
        .map(|count| format!("+{count}"))
        .unwrap_or_else(|| "—".into())
}
fn minus(count: Option<u32>) -> String {
    count
        .map(|count| format!("-{count}"))
        .unwrap_or_else(|| "—".into())
}
fn marker(kind: HunkKind) -> &'static str {
    match kind {
        HunkKind::Add => "+ ",
        HunkKind::Del => "- ",
        HunkKind::Context => "  ",
    }
}
fn marker_color(kind: HunkKind, th: &Theme) -> Color {
    match kind {
        HunkKind::Add => th.success,
        HunkKind::Del => th.error,
        HunkKind::Context => th.border,
    }
}

// Unlike prose wrapping this preserves spaces and blank lines. When a terminal
// cannot fit even one wide grapheme, replace that cell rather than overflowing.
fn code_chunks(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut result = Vec::new();
    for line in text.split('\n') {
        let mut current = String::new();
        let mut cells = 0;
        for grapheme in line.graphemes(true) {
            let n = UnicodeWidthStr::width(grapheme);
            if cells + n > width as usize && !current.is_empty() {
                result.push(current);
                current = String::new();
                cells = 0;
            }
            if n > width as usize {
                current.push('�');
                cells += 1;
            } else {
                current.push_str(grapheme);
                cells += n;
            }
        }
        result.push(current);
    }
    result
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    SheetChrome {
        header_right: model
            .review
            .as_ref()
            .map(|review| {
                vec![span(
                    format!(
                        "{} files · +{} -{}",
                        review.files.len(),
                        review.adds,
                        review.dels
                    ),
                    th.muted,
                )]
            })
            .unwrap_or_default(),
        status_third: Some(vec![span("review only", th.muted)]),
        hints: vec![hint(th, "↑↓ file"), hint(th, "pgup/pgdn read")],
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
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress_screen(&mut m, "6d");
        m.width = width;
        m
    }
    #[test]
    fn the_focused_file_expands_its_own_hunk_and_test_result() {
        let mut m = model(160);
        for index in 0..m.review.as_ref().unwrap().files.len() {
            m.diff_index = index;
            let rows = lines(&m);
            let drawn = rows
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            let file = &m.review.as_ref().unwrap().files[index];
            for value in [&file.path, &file.tests, &file.hunk_note] {
                assert!(drawn.contains(value), "{value}");
            }
            assert!(rows[ui::focused_row(&rows).unwrap()]
                .to_string()
                .contains(&file.path));
            for hunk in &file.hunk {
                assert!(drawn.contains(&hunk.text), "{}: {}", file.path, hunk.text);
            }
            assert!(
                !drawn.contains("j k")
                    && !drawn.contains("revert is per")
                    && !drawn.contains("until you say so")
            );
        }
    }
    #[test]
    fn code_wrapping_preserves_spaces_unicode_and_blank_lines() {
        let input = "    let café = \"模型 🦀\";  ";
        for width in [4, 10, 20, 80] {
            let chunks = code_chunks(input, width);
            assert_eq!(chunks.concat(), input);
            for chunk in chunks {
                assert!(UnicodeWidthStr::width(chunk.as_str()) <= width as usize);
            }
        }
        assert_eq!(code_chunks("", 20), vec![""]);
        assert_eq!(plus(None), "—");
        assert_eq!(minus(Some(7)), "-7");
    }
    #[test]
    fn empty_and_narrow_reviews_remain_cell_bounded() {
        let mut m = model(80);
        m.review = None;
        assert!(lines(&m)[0].to_string().contains("no changes"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
