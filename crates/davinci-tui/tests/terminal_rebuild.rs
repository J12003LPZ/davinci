//! Behavioral contracts for the terminal rebuild. These are NOT claims of
//! pixel/cell parity with Claude Code; that requires independent reference frames.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app,
    model::{Model, Screen},
    theme::{ColorDepth, Theme},
    ui,
};
use ratatui::style::Color;

fn model() -> Model {
    Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 30, false)
}

#[test]
fn rebuild_clip_preserves_graphemes() {
    assert_eq!(ui::clip("👩‍💻x", 2), "👩‍💻");
    assert_eq!(ui::clip("e\u{301}x", 1), "e\u{301}");
    assert_eq!(ui::clip("界x", 1), "");
    assert_eq!(ui::clip_ellipsis("abcdef", 0), "");
}

#[test]
fn rebuild_default_theme_uses_neutral_surfaces() {
    let th = Theme::da_vinci(ColorDepth::TrueColor, false);
    for color in [th.background, th.surface, th.surface_alt] {
        match color {
            Color::Rgb(r, g, b) => assert!(r == g && g == b, "non-neutral surface: {color:?}"),
            _ => panic!("expected truecolor theme"),
        }
    }
    assert_ne!(th.success, th.text, "success needs its own semantic color");
    assert_ne!(th.error, th.text, "errors must not look like ordinary output");
    assert_ne!(th.primary, th.error, "selection must not look like an error");
}

#[test]
fn rebuild_headings_are_text_not_newspaper_labels() {
    let th = model().theme;
    let label = ui::paper_label("Select model", &th, true);
    assert_eq!(label.content.as_ref(), "Select model");
    assert!(label.style.bg.is_none(), "ordinary headings must not be reversed paper labels");
}

#[test]
fn rebuild_rules_are_quiet_and_untextured() {
    let th = model().theme;
    assert_eq!(ui::print_rule(24, &th).to_string(), "─".repeat(24));
    assert_eq!(ui::print_rule(0, &th).to_string(), "");
}

#[test]
fn rebuild_ctrl_u_belongs_to_the_editor() {
    let mut m = model();
    m.composer.set_text("remove this line");
    let flow = app::handle_key(
        &mut m,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(flow, app::Flow::Continue);
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "");
}

#[test]
fn rebuild_open_close_preserves_the_draft() {
    let mut m = model();
    m.composer.set_text("draft with 界 and e\u{301}");
    let cursor = m.composer.cursor();
    m.screen = Screen::Settings;
    app::handle_key(&mut m, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "draft with 界 and e\u{301}");
    assert_eq!(m.composer.cursor(), cursor);
}
