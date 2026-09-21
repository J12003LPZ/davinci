//! Geometry and interaction checks grounded in the captured native CLI panels.
//! Product/account values deliberately come from DaVinci, never from the reference.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app, fixtures,
    model::{Model, Screen, SettingRow},
    theme::{ColorDepth, Theme},
    views,
};
fn model() -> Model {
    let mut m = Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        120,
        40,
        false,
    );
    m.settings_rows = vec![SettingRow {
        label: "Auto-compact".into(),
        key: "autocompact".into(),
        value: "true".into(),
        values: vec!["true".into(), "false".into()],
        ..Default::default()
    }];
    m
}
#[test]
fn config_uses_reference_tabs_search_box_and_adjacent_value_column() {
    let m = model();
    let text = views::settings::screen(&m, 40)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(text
        .iter()
        .any(|r| r.contains("Settings  Status   Config   Usage   Stats")));
    assert!(text.iter().any(|r| r.contains("⌕ Search settings…")));
    let setting = text.iter().find(|r| r.contains("Auto-compact")).unwrap();
    assert_eq!(setting.find("Auto-compact"), Some(5));
    assert_eq!(setting.find("true"), Some(48));
}
#[test]
fn settings_tabs_switch_without_mutating_a_setting_or_chat_draft() {
    let mut m = model();
    m.screen = Screen::Settings;
    m.composer.set_text("Preserved draft");
    let event = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        app::handle_key(&mut m, event(KeyCode::Up)),
        app::Flow::Continue
    );
    assert_eq!(
        app::handle_key(&mut m, event(KeyCode::Left)),
        app::Flow::Continue
    );
    let text = views::settings::screen(&m, 40)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Version:"));
    assert_eq!(m.settings_rows[0].value, "true");
    assert_eq!(m.composer.editor().get_text(), "Preserved draft");
}
#[test]
fn shipped_vox_selection_upgrades_without_changing_settings_on_disk() {
    let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
    assert_eq!(theme.with_name("vox"), theme.with_name("dark"));
}
#[test]
fn startup_keeps_the_reference_three_row_identity_geometry() {
    let mut m = model();
    m.model_name = "Actual provider model".into();
    let rows = views::startup::banner(&m, &m.startup);
    assert_eq!(rows.len(), 3);
    let title = rows[0].to_string();
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(&title[..title.find("DaVinci").unwrap()]),
        11
    );
    let model = rows[1].to_string();
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(
            &model[..model.find("Actual provider model").unwrap()]
        ),
        11
    );
}
#[test]
fn main_footer_has_one_hint_row_not_two_status_dashboards() {
    let mut m = model();
    fixtures::dress_screen(&mut m, "1a");
    let rows = app::compose(&m, 40)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let prompt = rows.iter().rposition(|row| row.starts_with("❯")).unwrap();
    assert_eq!(prompt, 37);
    assert!(rows[39].contains("? for shortcuts"));
}

#[test]
fn model_panel_uses_a_numbered_list_and_unboxed_reference_header() {
    let mut m = model();
    fixtures::dress_screen(&mut m, "3a");
    let rows = views::cogitator::screen(&m, 24);
    let text = rows
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows[0].to_string().starts_with('▔'));
    assert!(text.contains("   Select model"));
    assert!(text.contains("1. "));
    assert!(!text.contains("╭"));
    assert!(rows.last().unwrap().to_string().contains("Enter"));
}
#[test]
fn voice_does_not_take_the_verbose_output_shortcut() {
    let (keys, _) = davinci_tui::Keybindings::defaults().with_voice(true);
    assert!(keys.matches("\x0f", "davinci.tools.expand"));
}

#[test]
fn utility_screens_share_the_unboxed_bottom_panel_and_keep_the_draft() {
    for screen in [
        Screen::Thinking,
        Screen::Login,
        Screen::Keys,
        Screen::Resume,
        Screen::Tree,
        Screen::Compact,
        Screen::Export,
        Screen::Vectors,
        Screen::Governor,
        Screen::Securitas,
        Screen::Trust,
        Screen::Officina,
        Screen::Recovery,
        Screen::Diff,
        Screen::Mcp,
        Screen::Permissions,
        Screen::Workflows,
        Screen::TaskBoard,
        Screen::Agents,
        Screen::ContextInspector,
    ] {
        let mut m = model();
        m.screen = screen;
        m.composer.set_text("untouched draft");
        let rows = app::compose(&m, 40);
        assert!(
            rows.iter().any(|row| row.to_string().starts_with('▔')),
            "{screen:?}"
        );
        assert_eq!(rows.len(), 40);
        assert_eq!(m.composer.editor().get_text(), "untouched draft");
    }
}

#[test]
fn user_messages_use_the_reference_prompt_without_a_chat_bubble() {
    use davinci_tui::davinci::model::Entry;
    let m = model();
    let rows = views::transcript::lines(&m, &[Entry::user("Explain this code")], 120);
    assert!(rows[0].to_string().starts_with("❯ Explain this code"));
    assert!(rows[0].style.bg.is_none());
}

#[test]
fn task_checklist_is_readable_at_normal_terminal_widths() {
    use davinci_tui::davinci::{model::Step, theme::State};
    let mut m = model();
    m.width = 80;
    let steps = vec![
        Step::new(State::Done, "Inspect files", None),
        Step::new(State::Active, "Run tests", None),
    ];
    let rows = views::studio::lines(&m, &steps);
    let text = rows
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Inspect files") && text.contains("Run tests"));
    assert!(!text.contains("STUDIO") && !text.contains('╭'));
    assert_eq!(rows.len(), views::studio::height(&m, &steps));
}

#[test]
fn model_session_only_choice_does_not_request_a_default_save() {
    use davinci_tui::davinci::model::Choice;
    let mut m = model();
    fixtures::dress_screen(&mut m, "3a");
    assert_eq!(
        app::handle_key(
            &mut m,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)
        ),
        app::Flow::Choose(Choice::Catalog(m.catalog_index))
    );
    assert!(m.catalog_session_only);
    // The host owns persistence, so the view only specifies the requested scope.
    // The matching host contract is tested alongside the persistence guard.
}

#[test]
fn every_screen_and_overlay_is_bounded_in_both_themes() {
    use davinci_tui::davinci::model::Overlay;
    use davinci_tui::davinci::views::secret_input::SecretInputState;
    let screens = [
        Screen::Agent,
        Screen::Plan,
        Screen::Grafo,
        Screen::Memoria,
        Screen::Mensura,
        Screen::Models,
        Screen::Settings,
        Screen::Thinking,
        Screen::Login,
        Screen::Keys,
        Screen::Resume,
        Screen::Tree,
        Screen::Compact,
        Screen::Export,
        Screen::GraphRun,
        Screen::Vectors,
        Screen::Governor,
        Screen::Securitas,
        Screen::Trust,
        Screen::Officina,
        Screen::Recovery,
        Screen::Diff,
        Screen::Mcp,
        Screen::Permissions,
        Screen::Workflows,
        Screen::TaskBoard,
        Screen::Agents,
        Screen::ContextInspector,
    ];
    let overlays = [
        Overlay::Instrumenta,
        Overlay::Sessions,
        Overlay::Cogitator,
        Overlay::SecretInput,
        Overlay::Ask,
    ];
    for theme_name in ["dark", "light", "vox"] {
        for (width, height) in [(0, 0), (1, 1), (40, 12), (80, 24), (120, 40), (160, 50)] {
            for (screen, overlay) in screens.iter().map(|screen| (*screen, None)).chain(
                overlays
                    .iter()
                    .map(|overlay| (Screen::Agent, Some(*overlay))),
            ) {
                let mut m = model();
                m.width = width;
                m.height = height;
                m.theme = m.theme.with_name(theme_name);
                m.screen = screen;
                m.overlay = overlay;
                if overlay == Some(Overlay::SecretInput) {
                    m.secret_input = Some(SecretInputState::new());
                }
                let rows = app::compose(&m, height);
                assert_eq!(rows.len(), height as usize, "{screen:?}/{overlay:?}");
                for row in rows {
                    assert!(
                        davinci_tui::davinci::ui::run_width(&row.spans) <= width,
                        "{screen:?}/{overlay:?} at {width}x{height}: {}",
                        row
                    );
                }
            }
        }
    }
}

#[test]
fn selector_paste_updates_search_without_touching_the_conversation() {
    let mut m = model();
    m.screen = Screen::Settings;
    m.composer.set_text("Keep my draft");
    m.paste("Auto\r\ncompact");
    assert_eq!(m.settings_query, "Auto compact");
    assert_eq!(m.composer.editor().get_text(), "Keep my draft");
    m.screen = Screen::Models;
    m.paste("sonnet");
    assert_eq!(m.catalog_query, "sonnet");
    assert!(m.catalog_search);
    assert_eq!(m.composer.editor().get_text(), "Keep my draft");
}
