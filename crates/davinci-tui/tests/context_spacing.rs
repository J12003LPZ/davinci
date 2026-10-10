//! The last thing the transcript says (an error, a command's output) must not
//! sit flush against the context meter's header: a row of air separates them.

use davinci_tui::davinci::{
    app,
    model::{ContextBarMode, ContextCategory, ContextKind, ContextUsageView, Entry, Model},
    theme::{ColorDepth, Theme},
};

fn meter() -> ContextUsageView {
    ContextUsageView {
        model: "gpt-5.6-luna".into(),
        window: 272_000,
        categories: vec![ContextCategory {
            kind: ContextKind::Messages,
            label: "Messages".into(),
            tokens: 19_800,
        }],
        free: 116_000,
        buffer: 136_000,
        sections: Vec::new(),
    }
}

#[test]
fn a_blank_row_separates_the_last_message_from_the_context_meter() {
    for height in [20u16, 30, 44] {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            100,
            height,
            true,
        );
        m.width = 100;
        m.height = height;
        m.context_bar = ContextBarMode::Compact;
        m.context_meter = Some(meter());
        // Enough history that the last message fills the row above the meter.
        for n in 0..80 {
            m.transcript.push(Entry::User(format!("message {n}")));
        }
        let rows: Vec<String> = app::compose(&m, height)
            .iter()
            .map(ToString::to_string)
            .collect();
        let at = rows
            .iter()
            .position(|row| row.contains("◆ context"))
            .unwrap_or_else(|| panic!("no meter at {height}:\n{}", rows.join("\n")));
        assert!(
            at > 0 && rows[at - 1].trim().is_empty() && rows[at - 2].contains("message"),
            "{height}:\n{}",
            rows.join("\n")
        );
    }
}
