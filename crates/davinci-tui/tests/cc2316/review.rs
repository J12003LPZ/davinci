//! Fixes from the Milestone B review: overlay precedence over the hidden
//! picker and steps, Enter applying what it saves, and the negotiated depth.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::{self, Flow},
    model::{Choice, Model, OnboardStep, Overlay, PickerItem},
    theme::{ColorDepth, Theme},
    views::{onboarding, theme_picker},
};
use ratatui::style::Color;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn reference_model(theme: Theme) -> Model {
    let mut model = Model::new(theme, 120, 46, false);
    model.reference = true;
    model.composer.set_text("a draft in progress");
    model
}

fn on(depth: ColorDepth) -> Theme {
    Theme::reference(depth, false)
}

fn shown(model: &Model) -> String {
    app::compose_frame(model, model.height)
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn ask_over(model: &mut Model) {
    model.ask.title = "A question for you".into();
    model.ask.items = vec![PickerItem::new("first", ""), PickerItem::new("second", "")];
    model.overlay = Some(Overlay::Ask);
}

#[test]
fn a_visible_overlay_outranks_the_picker_for_keys_and_paste() {
    let mut model = reference_model(on(ColorDepth::TrueColor));
    assert!(theme_picker::open(&mut model, "dark"));
    model.overlay = Some(Overlay::Instrumenta);
    let (index, theme) = (model.theme_picker.as_ref().unwrap().index, model.theme);
    model.paste("search words");
    assert_eq!(
        model.query, "search words",
        "the panel in view took the paste"
    );
    app::handle_key(&mut model, key(KeyCode::Char('x')));
    assert_eq!(model.query, "search wordsx");
    app::handle_key(&mut model, key(KeyCode::Down));
    assert_eq!(model.theme_picker.as_ref().unwrap().index, index);
    assert_eq!(model.theme, theme, "the picker saw none of it");
    app::handle_key(&mut model, key(KeyCode::Esc));
    assert!(model.overlay.is_none(), "esc closed the panel in view");
    assert!(model.theme_picker.is_some(), "and left the picker open");
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

#[test]
fn a_visible_question_is_drawn_and_answered_over_the_picker() {
    let mut model = reference_model(on(ColorDepth::TrueColor));
    assert!(theme_picker::open(&mut model, "dark"));
    ask_over(&mut model);
    let frame = shown(&model);
    assert!(frame.contains("A question for you"), "{frame}");
    assert!(!frame.contains(theme_picker::HEADING_SENTENCE));
    app::handle_key(&mut model, key(KeyCode::Down));
    assert_eq!(model.ask_index, 1, "the arrow moved the question");
    assert_eq!(
        model.theme_picker.as_ref().unwrap().index,
        1,
        "not the picker"
    );
}

#[test]
fn a_visible_overlay_outranks_the_steps_for_keys_and_paste() {
    for step in [OnboardStep::Theme, OnboardStep::Security] {
        let mut model = reference_model(on(ColorDepth::TrueColor));
        assert!(onboarding::start(&mut model, "dark"));
        model.onboarding = Some(step);
        ask_over(&mut model);
        assert!(shown(&model).contains("A question for you"), "{step:?}");
        app::handle_key(&mut model, key(KeyCode::Down));
        assert_eq!(model.ask_index, 1, "{step:?}: the question took the arrow");
        assert_eq!(model.onboarding, Some(step));
        model.overlay = Some(Overlay::Instrumenta);
        model.paste("typed words");
        assert_eq!(model.query, "typed words", "{step:?}");
        app::handle_key(&mut model, key(KeyCode::Esc));
        assert!(model.overlay.is_none(), "{step:?}: esc closed the panel");
        assert_eq!(model.onboarding, Some(step), "{step:?}: the steps stay");
    }
}

#[test]
fn enter_in_the_picker_applies_what_it_saves() {
    // A failed save left a light preview; reopening highlights Dark.
    let mut model = reference_model(on(ColorDepth::TrueColor));
    model.theme = model.theme.with_name("light");
    assert!(theme_picker::open(&mut model, "dark"));
    assert_eq!(
        app::handle_key(&mut model, key(KeyCode::Enter)),
        Flow::Choose(Choice::Theme("dark".into()))
    );
    assert_eq!(model.theme, on(ColorDepth::TrueColor).with_name("dark"));
}

#[test]
fn enter_on_the_theme_step_applies_what_it_chooses() {
    let mut model = reference_model(on(ColorDepth::TrueColor));
    model.theme = model.theme.with_name("light");
    assert!(onboarding::start(&mut model, "dark"));
    app::handle_key(&mut model, key(KeyCode::Enter));
    assert_eq!(model.first_run_theme.as_deref(), Some("dark"));
    assert_eq!(model.theme, on(ColorDepth::TrueColor).with_name("dark"));
}

#[test]
fn the_steps_open_on_the_stored_theme_and_show_it_from_the_first_frame() {
    for name in ["light", "ansi", "daltonized"] {
        let mut model = reference_model(on(ColorDepth::TrueColor).with_name(name));
        assert!(onboarding::start(&mut model, name));
        let picker = model.theme_picker.as_ref().unwrap();
        assert_eq!(theme_picker::ENTRIES[picker.index].0, name);
        assert_eq!(model.theme, on(ColorDepth::TrueColor).with_name(name));
    }
}

fn is_named(color: Color) -> bool {
    !matches!(color, Color::Rgb(..) | Color::Indexed(_))
}

#[test]
fn narrowing_to_ansi_keeps_the_terminals_depth_in_any_construction_order() {
    let tc = ColorDepth::TrueColor;
    let truth = on(tc).with_name("dark");
    // The chain the startup path builds: the stored name first, the
    // reference switch after, the stored name again.
    let chain = Theme::da_vinci(tc, false)
        .with_name("ansi")
        .with_reference(true)
        .with_name("dark");
    assert_eq!(
        chain.with_name("dark"),
        truth,
        "ansi, then reference, then dark"
    );
    assert!(
        matches!(chain.cc().permission, Color::Rgb(..)),
        "leaving ansi gives the terminal's own depth back"
    );
    let startup = Theme::da_vinci(tc, false)
        .with_name("ansi")
        .with_reference(true)
        .with_name("ansi");
    assert!(is_named(startup.cc().permission));
    let mut model = reference_model(startup);
    assert!(theme_picker::open(&mut model, "ansi"));
    for _ in 0..4 {
        app::handle_key(&mut model, key(KeyCode::Up));
    }
    assert_eq!(
        theme_picker::ENTRIES[model.theme_picker.as_ref().unwrap().index].0,
        "dark"
    );
    app::handle_key(&mut model, key(KeyCode::Enter));
    assert_eq!(model.theme, truth);
}

fn result_row(reference: bool) -> String {
    use davinci_tui::davinci::model::Entry;
    let theme = if reference {
        on(ColorDepth::TrueColor)
    } else {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    };
    let mut model = Model::new(theme, 120, 44, false);
    model.reference = reference;
    model.transcript.push(Entry::user("/theme"));
    model.confirm("Theme set to light", None);
    app::compose(&model, 44)
        .iter()
        .map(|line| line.to_string())
        .find(|row| row.contains("Theme set to light"))
        .expect("the result is on screen")
}

#[test]
fn the_security_notes_leave_the_link_row_blank_and_keep_the_prompt_row() {
    // D7: no external link, so the row where the reference draws one stays blank
    // and the closing line sits on the row it has there (0-based rows 24 and 26).
    let mut model = reference_model(on(ColorDepth::TrueColor));
    model.height = 44;
    assert!(onboarding::start(&mut model, "dark"));
    app::handle_key(&mut model, key(KeyCode::Enter));
    assert_eq!(model.onboarding, Some(OnboardStep::Security));
    let rows: Vec<String> = app::compose(&model, 44)
        .iter()
        .map(|l| l.to_string())
        .collect();
    assert!(
        rows[23].starts_with(" 2. ") && rows[23].contains("prompt injection"),
        "{:?}",
        rows[23]
    );
    assert!(rows[24].trim().is_empty(), "{:?}", rows[24]);
    assert_eq!(rows[26].trim_end(), " Press Enter to continue…");
}

#[test]
fn a_narrow_window_wraps_the_second_note_rather_than_cut_the_advice() {
    for width in [100u16, 80, 60, 40, 24] {
        let mut model = reference_model(on(ColorDepth::TrueColor));
        model.width = width;
        model.height = 44;
        assert!(onboarding::start(&mut model, "dark"));
        app::handle_key(&mut model, key(KeyCode::Enter));
        let rows: Vec<String> = app::compose(&model, 44)
            .iter()
            .map(|l| l.to_string())
            .collect();
        let notes = rows.iter().map(|r| r.trim()).collect::<Vec<_>>().join(" ");
        assert!(
            notes.contains("so open only code you trust."),
            "{width}: {notes}"
        );
    }
}

fn ansi_screen(reference: bool) -> (Model, Vec<ratatui::text::Line<'static>>) {
    let tc = ColorDepth::TrueColor;
    let theme = if reference {
        on(tc).with_name("ansi")
    } else {
        Theme::da_vinci(tc, false).with_name("ansi")
    };
    let mut model = Model::new(theme, 120, 44, true);
    model.reference = reference;
    model.model_name = "demo-model".into();
    model.model_title = "Demo-Model".into();
    model.provider_title = "Fixture".into();
    model.startup.cwd = "/srv/project".into();
    model.thinking_level = "medium".into();
    let lines = app::compose(&model, 44);
    (model, lines)
}

fn dim_of(line: &ratatui::text::Line<'_>, from: usize) -> Vec<(String, bool)> {
    use ratatui::style::Modifier;
    let mut col = 0;
    let mut found = Vec::new();
    for span in &line.spans {
        let width = unicode_width::UnicodeWidthStr::width(span.content.as_ref());
        if col + width > from && !span.content.trim().is_empty() {
            found.push((
                span.content.to_string(),
                span.style.add_modifier.contains(Modifier::DIM),
            ));
        }
        col += width;
    }
    found
}

#[test]
fn at_ansi_depth_the_reference_header_and_effort_label_are_dim_and_the_rules_white() {
    use ratatui::style::Modifier;
    let (model, lines) = ansi_screen(true);
    // The name is the terminal's own foreground, bold; the rest is dim.
    let name = lines[1]
        .spans
        .iter()
        .find(|s| s.content.contains("DaVinci"))
        .unwrap();
    assert_eq!(name.style.fg.unwrap_or(Color::Reset), Color::Reset);
    assert!(name.style.add_modifier.contains(Modifier::BOLD));
    let version = dim_of(&lines[1], 12);
    assert!(
        version.iter().any(|(t, dim)| t.contains('v') && *dim),
        "{version:?}"
    );
    for row in [2, 3] {
        let shown = dim_of(&lines[row], 11);
        assert!(
            !shown.is_empty() && shown.iter().all(|(_, dim)| *dim),
            "row {row}: {shown:?}"
        );
    }
    let effort = dim_of(&lines[39], 90);
    assert!(
        effort.iter().any(|(t, dim)| t.contains("medium") && *dim),
        "{effort:?}"
    );
    for row in [40, 42] {
        assert_eq!(lines[row].spans[0].style.fg, Some(Color::Gray), "row {row}");
    }
    assert_eq!(model.theme.cc().prompt_border, Color::Gray);
}

#[test]
fn off_mode_ansi_keeps_its_text_ink_and_rules() {
    let (model, lines) = ansi_screen(false);
    assert_eq!(model.theme.text, Color::White);
    assert_ne!(model.theme.cc().prompt_border, Color::Gray);
    let _ = lines;
}

#[test]
fn a_command_result_hangs_from_plain_spaces_in_the_reference_and_is_unchanged_otherwise() {
    assert!(
        result_row(true).starts_with("  \u{23bf}  Theme set"),
        "{:?}",
        result_row(true)
    );
    assert!(
        result_row(false).starts_with("  \u{23bf} \u{a0}Theme set"),
        "{:?}",
        result_row(false)
    );
}
