//! `/export`: a receipt for the output the runtime actually produced.
//! The legacy `gist` field can contain a local path; it is not proof of upload.

use super::sheet::{Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(ledger) = &model.export_ledger else {
        return section_detail(
            width,
            th,
            "No export result. Use /export <path> from the conversation.",
        );
    };
    let mut rows = section_detail(width, th, "Export result");
    // A path supplied by the export operation is authoritative. Keep a distinct
    // link label for an actual remote destination, without claiming clipboard use.
    if !ledger.file.is_empty() {
        rows.extend(section_detail(width, th, &format!("File: {}", ledger.file)));
    }
    if !ledger.gist.is_empty() && ledger.gist != ledger.file {
        let remote = ledger.gist.starts_with("https://") || ledger.gist.starts_with("http://");
        rows.extend(section_detail(
            width,
            th,
            &format!("{}: {}", if remote { "Link" } else { "File" }, ledger.gist),
        ));
        if remote {
            rows.extend(section_detail(
                width,
                th,
                "Review link visibility before sharing this session.",
            ));
        }
    }
    for (label, value) in [
        ("Size", &ledger.size),
        ("Elapsed", &ledger.elapsed),
        ("Session", &ledger.session),
        ("Turns", &ledger.turns),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    if !ledger.included.is_empty() {
        rows.extend(section_detail(width, th, "Included"));
        for text in &ledger.included {
            rows.extend(section_detail(width, th, &format!("  {text}")));
        }
    }
    if !ledger.excluded.is_empty() {
        rows.extend(section_detail(width, th, "Content and privacy notes"));
        for (state, text) in &ledger.excluded {
            let mut detail = section_detail(width, th, &format!("{} {text}", state.glyph()));
            for row in &mut detail {
                for s in &mut row.spans {
                    s.style.fg = Some(th.state_color(*state));
                }
            }
            rows.extend(detail);
        }
    }
    rows.extend(section_detail(width, th, "Review exported content before sharing; paths and conversation text may identify your environment."));
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    SheetChrome {
        status_third: Some(vec![span("export result", model.theme.muted)]),
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::ExportLedger,
        theme::{ColorDepth, State, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.export_ledger = Some(ExportLedger {
            gist: "session.html".into(),
            size: "4 KB".into(),
            elapsed: "0.1s".into(),
            included: vec!["2 turns".into()],
            excluded: vec![(State::Attention, "absolute paths retained".into())],
            ..Default::default()
        });
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
    fn receipt_keeps_output_and_privacy_facts_without_claiming_upload() {
        let m = model(80);
        let drawn = text(&m);
        for value in [
            "File: session.html",
            "4 KB",
            "0.1s",
            "2 turns",
            "absolute paths retained",
        ] {
            assert!(drawn.contains(value));
        }
        for false_claim in ["uploaded", "clipboard", "[o]", "[c]", "[d]", "secret gist"] {
            assert!(!drawn.contains(false_claim));
        }
    }
    #[test]
    fn a_real_link_is_named_without_inventing_visibility_or_clipboard_state() {
        let mut m = model(80);
        m.export_ledger.as_mut().unwrap().gist = "https://example.test/session".into();
        assert!(text(&m).contains("Link: https://example.test/session"));
        assert!(!text(&m).contains("clipboard"));
    }
    #[test]
    fn empty_and_narrow_receipts_remain_bounded() {
        let mut m = model(80);
        m.export_ledger = None;
        assert!(text(&m).contains("No export result"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
