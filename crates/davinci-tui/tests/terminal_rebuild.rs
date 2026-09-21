//! Behavioral contracts for the terminal rebuild. Shared 120x40 geometry and
//! palette assertions are derived from the validated Claude Code 2.1.278 reference frames.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app, fixtures,
    model::{CatalogRow, Choice, Model, Screen, SettingRow},
    theme::{ColorDepth, Theme},
    ui, views,
};
use ratatui::style::Color;

fn model() -> Model {
    Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        100,
        30,
        false,
    )
}

fn type_text(m: &mut Model, text: &str) {
    for ch in text.chars() {
        app::handle_key(m, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
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
        assert_eq!(
            color,
            Color::Reset,
            "Claude-style terminal chrome must inherit the terminal background"
        );
    }
    assert_ne!(th.success, th.text, "success needs its own semantic color");
    assert_ne!(
        th.error, th.text,
        "errors must not look like ordinary output"
    );
    assert_ne!(
        th.primary, th.error,
        "selection must not look like an error"
    );
}

#[test]
fn rebuild_headings_are_text_not_newspaper_labels() {
    let th = model().theme;
    let label = ui::paper_label("Select model", &th, true);
    assert_eq!(label.content.as_ref(), "Select model");
    assert!(
        label.style.bg.is_none(),
        "ordinary headings must not be reversed paper labels"
    );
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
    assert_eq!(
        m.screen,
        Screen::Settings,
        "first Escape leaves search focus"
    );
    app::handle_key(&mut m, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "draft with 界 and e\u{301}");
    assert_eq!(m.composer.cursor(), cursor);
}

#[test]
fn rebuild_settings_search_selects_the_source_row_and_keeps_chat_draft() {
    let mut m = model();
    m.settings_rows = vec![
        SettingRow {
            key: "autocompact".into(),
            label: "Auto-compact".into(),
            ..Default::default()
        },
        SettingRow {
            key: "transport".into(),
            label: "Transport".into(),
            values: vec!["auto".into(), "sse".into()],
            ..Default::default()
        },
    ];
    m.screen = Screen::Settings;
    m.composer.set_text("unrelated draft");
    type_text(&mut m, "transport");
    assert_eq!(m.settings_index, 1);
    assert_eq!(m.composer.editor().get_text(), "unrelated draft");
    let rows = views::settings::lines(&m)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!rows.contains("Auto-compact"));
    assert_eq!(
        app::handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        app::Flow::Continue
    );
    assert_eq!(
        app::handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        app::Flow::Choose(Choice::Setting(1))
    );
}

#[test]
fn rebuild_model_search_does_not_reindex_equal_model_names() {
    let mut m = model();
    m.catalog = vec![
        CatalogRow {
            provider: "provider-a".into(),
            id: "model".into(),
            name: "Same name".into(),
            ..Default::default()
        },
        CatalogRow {
            provider: "provider-b".into(),
            id: "model".into(),
            name: "Same name".into(),
            ..Default::default()
        },
    ];
    m.screen = Screen::Models;
    type_text(&mut m, "provider-b");
    assert_eq!(
        m.catalog.len(),
        2,
        "filtering must not rewrite the host collection"
    );
    assert_eq!(m.catalog_index, 1);
    assert_eq!(
        app::handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        app::Flow::Choose(Choice::Catalog(1))
    );
}

#[test]
fn rebuild_zero_search_results_cannot_accept_a_hidden_model() {
    let mut m = model();
    m.catalog = vec![CatalogRow {
        name: "Model".into(),
        ..Default::default()
    }];
    m.screen = Screen::Models;
    type_text(&mut m, "nonexistent");
    assert_eq!(
        app::handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        app::Flow::Continue
    );
}

#[test]
fn rebuild_reference_shell_uses_claude_120x40_geometry() {
    let mut m = model();
    m.width = 120;
    m.height = 40;
    m.transcript.clear();
    m.graph_run = None;
    m.running = false;
    m.thinking_level = "high".into();
    m.permission_mode = "ask".into();
    let rows = app::compose(&m, 40);
    assert_eq!(rows.len(), 40);
    assert!(rows[1].to_string().starts_with(" ▐▛███▛█   DaVinci"));
    assert!(rows[2].to_string().starts_with("▝▜██████▀  "));
    assert!(rows[3].to_string().starts_with("  ▝▝ ▝▝    "));
    assert!(rows[35].to_string().contains("● high · /effort"));
    assert_eq!(rows[36].to_string(), "─".repeat(120));
    assert!(rows[37].to_string().starts_with("❯"));
    assert_eq!(rows[38].to_string(), "─".repeat(120));
    assert!(rows[39]
        .to_string()
        .contains("⏸ manual mode on · ? for shortcuts · ← for agents"));
}

#[test]
fn rebuild_model_sheet_matches_reference_height() {
    let mut m = model();
    m.width = 120;
    m.height = 40;
    fixtures::dress_screen(&mut m, "3a");
    m.screen = Screen::Models;
    m.overlay = None;
    assert_eq!(views::cogitator::screen_height(&m), 16);
    let rows = app::compose(&m, 40);
    assert!(rows[24].to_string().contains("/effort"));
    assert!(rows[25].to_string().contains("Select model"));
    assert!(rows[39].to_string().contains("Enter to set as default"));
}

#[test]
fn rebuild_startup_leaves_room_for_the_conversation() {
    let m = model();
    let rows = views::startup::lines(&m, &m.startup);
    let text = rows
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.len() <= 9, "startup consumed {} rows", rows.len());
    assert!(!text.contains("▓"));
    assert!(!text.contains("CODE / TOOLS / CONTEXT"));
    assert!(text.contains(&m.model_name));
}
