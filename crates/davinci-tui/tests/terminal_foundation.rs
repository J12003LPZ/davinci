//! Foundation regressions. These check DaVinci behavior, not external UI parity.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app,
    model::{Model, Screen},
    theme::{ColorDepth, Theme},
    ui,
    views::startup,
};
use ratatui::style::Color;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

fn model() -> Model {
    Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        100,
        30,
        false,
    )
}

#[test]
fn delete_to_start_retains_the_rest_and_can_be_yanked() {
    let mut m = model();
    m.composer.set_text("first line\nsecond");
    app::handle_key(
        &mut m,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "first line\n");
    app::handle_key(
        &mut m,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
    );
    assert_eq!(m.composer.editor().get_text(), "first line\nsecond");
}

#[test]
fn usage_view_has_a_nonconflicting_default_shortcut() {
    let mut m = model();
    m.composer.set_text("keep this");
    let cursor = m.composer.cursor();
    app::handle_key(&mut m, KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT));
    assert_eq!(m.screen, Screen::Mensura);
    app::handle_key(&mut m, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "keep this");
    assert_eq!(m.composer.cursor(), cursor);
}

#[test]
fn explicit_shell_keybindings_are_still_respected() {
    let mut m = model();
    m.keybindings = davinci_tui::Keybindings::from_json(r#"{"davinci.mensura.toggle":"alt+x"}"#);
    app::handle_key(&mut m, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
    assert_eq!(m.screen, Screen::Mensura);
}

#[test]
fn oversized_styled_runs_never_wrap_the_u16_width_counter() {
    let input = vec![Span::raw("x".repeat(70_000)), Span::raw("y".repeat(70_000))];
    assert_eq!(ui::run_width(&input), u16::MAX);
    let clipped = ui::truncate_run(input, 80);
    let text = clipped
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert!(UnicodeWidthStr::width(text.as_str()) <= 80);
    assert!(text.ends_with('…'));
    let input = vec![Span::raw("x".repeat(70_000))];
    let clipped = ui::truncate_run(input, u16::MAX);
    let text = clipped
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert_eq!(UnicodeWidthStr::width(text.as_str()), usize::from(u16::MAX));
    assert!(text.ends_with('…'));
}

#[test]
fn wrapping_never_tears_a_joined_emoji_even_when_it_cannot_fit() {
    assert_eq!(ui::wrap("👩‍💻", 1), vec!["👩‍💻"]);
    for text in ["👩‍💻xyz", "e\u{301}xyz", "🇵🇷xyz", "界xyz"] {
        for width in 0..8 {
            assert!(UnicodeWidthStr::width(ui::clip(text, width).as_str()) <= usize::from(width));
        }
    }
}

fn hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02X}{g:02X}{b:02X}"),
        other => panic!("not truecolor: {other:?}"),
    }
}

#[test]
fn native_and_regular_builtin_palettes_cannot_drift() {
    for regular in davinci_tui::builtin_themes()
        .into_iter()
        .filter(|t| t.palette.is_some())
    {
        let native = Theme::da_vinci(ColorDepth::TrueColor, false).with_name(&regular.name);
        assert_eq!(regular.background, hex(native.background));
        assert_eq!(regular.foreground, hex(native.text));
        assert_eq!(regular.accent, hex(native.primary));
        let palette = regular.palette.unwrap();
        for (actual, expected) in [
            (palette.surface, native.surface),
            (palette.border, native.border),
            (palette.text, native.text),
            (palette.muted, native.muted),
            (palette.primary, native.primary),
            (palette.secondary, native.secondary),
            (palette.success, native.success),
            (palette.warning, native.warning),
            (palette.error, native.error),
        ] {
            assert_eq!(actual, hex(expected), "{}", regular.name);
        }
    }
}

#[test]
fn compact_welcome_preserves_real_identity_and_workspace() {
    let mut m = model();
    m.model_name = "fixture-provider/fixture-model".into();
    m.startup.cwd = "/fixture/project".into();
    for width in [1, 40, 80, 100, 160] {
        m.width = width;
        let rows = startup::banner(&m, &m.startup);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.width() <= usize::from(width)));
        let text = rows
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("▓▓"));
        if width >= 80 {
            assert!(text.contains("DaVinci"));
            assert!(text.contains("fixture-provider/fixture-model"));
            assert!(text.contains("/fixture/project"));
        }
    }
}
