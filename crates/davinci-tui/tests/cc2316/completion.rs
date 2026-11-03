//! P09: the completion list and shell mode. Every text here is synthetic; the geometry
//! comes from measuring the pinned build (the slash_only goldens).
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::{self, Flow},
    model::{Model, Screen},
    theme::{ColorDepth, Theme},
};
use davinci_tui::{
    suggestions, AutocompleteItem, AutocompleteSuggestions, SlashCommandSpec, SuggestionQuery,
};

fn shell(reference: bool, width: u16, height: u16) -> Model {
    let theme = if reference {
        Theme::reference(ColorDepth::TrueColor, false)
    } else {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    };
    let mut model = Model::new(theme, width, height, true);
    model.reference = reference;
    model.thinking_level = "medium".into();
    model
}

fn item(name: &str, description: &str) -> AutocompleteItem {
    AutocompleteItem {
        value: name.into(),
        label: name.into(),
        description: Some(description.into()),
    }
}

fn with_list(model: &mut Model) {
    model.composer.set_text("/th");
    model.suggestions = Some(AutocompleteSuggestions {
        items: vec![
            item("theme", "Change the theme"),
            item("alpha", "A first thing to do"),
            item("beta", "A second thing to do"),
            item("gamma", "A third thing to do"),
            item("delta", "A fourth thing to do"),
            item("epsilon", "A sixth thing beyond the window"),
        ],
        prefix: "/th".into(),
    });
    model.suggestion_index = 0;
}

fn frame(model: &Model) -> Vec<String> {
    app::compose(model, model.height)
        .iter()
        .map(|line| line.to_string())
        .collect()
}

fn press(model: &mut Model, code: KeyCode) -> Flow {
    app::handle_key(model, KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn the_list_sits_above_the_composer_with_the_marker_and_columns() {
    let mut model = shell(true, 120, 44);
    with_list(&mut model);
    let rows = frame(&model);
    // Five rows, the last where the effort row would sit; the effort row is not drawn.
    assert_eq!(
        rows[35], "  ❯ /theme                      Change the theme",
        "{:?}",
        rows[35]
    );
    assert!(rows[36].starts_with("    /alpha"), "{:?}", rows[36]);
    assert!(rows[39].starts_with("    /delta"), "{:?}", rows[39]);
    assert!(!rows.iter().any(|row| row.contains("/effort")));
    assert!(rows[40].starts_with('─'));
    // The description starts at column 32 on every row.
    for row in &rows[35..=39] {
        assert_eq!(&row[32..33], &row[32..33].to_ascii_uppercase(), "{row:?}");
        assert!(
            !row[32..].trim().is_empty(),
            "no description at 32: {row:?}"
        );
    }
    // The sixth item is past the five-row window.
    assert!(!rows.iter().any(|row| row.contains("/epsilon")));
}

#[test]
fn the_effort_row_comes_back_once_the_list_closes() {
    let mut model = shell(true, 120, 44);
    with_list(&mut model);
    model.dismiss_suggestions();
    let rows = frame(&model);
    assert!(rows[39].contains("/effort"), "{:?}", rows[39]);
    assert_eq!(rows[41].trim_end(), "❯\u{a0}/th");
}

#[test]
fn off_mode_keeps_its_own_list_layout() {
    let mut model = shell(false, 120, 44);
    with_list(&mut model);
    let rows = frame(&model);
    // The shell's list has no marker and no effort-row swap; its effort row is blank
    // (the reference label is not drawn off).
    assert!(!rows
        .iter()
        .any(|row| row.contains('❯') && row.contains("/theme")));
    assert!(
        rows.iter()
            .any(|row| row.trim_start().starts_with("/theme")),
        "{:?}",
        rows[34]
    );
    assert_eq!(rows[39].trim(), "", "{:?}", rows[39]);
}

#[test]
fn tab_fills_the_marked_command_and_closes_the_list() {
    let mut model = shell(true, 120, 44);
    with_list(&mut model);
    assert_eq!(press(&mut model, KeyCode::Tab), Flow::Continue);
    assert_eq!(model.composer.to_string(), "/theme ");
    assert!(model.suggestions.is_none(), "the list stayed open");
}

#[test]
fn enter_fills_and_sends_the_marked_command_once() {
    let mut model = shell(true, 120, 44);
    with_list(&mut model);
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not take the marked command");
    };
    assert_eq!(sent, "/theme");
    assert_eq!(model.composer.to_string(), "");
}

#[test]
fn escape_closes_the_list_and_keeps_the_typed_text() {
    let mut model = shell(true, 120, 44);
    with_list(&mut model);
    assert_eq!(press(&mut model, KeyCode::Esc), Flow::Continue);
    assert!(model.suggestions.is_none());
    assert_eq!(model.composer.to_string(), "/th");
}

#[test]
fn the_reference_list_scores_a_description_hit_and_the_shell_list_does_not() {
    let commands = [
        SlashCommandSpec {
            name: "theme".into(),
            description: "Change the theme".into(),
            argument_hint: None,
            argument_items: Vec::new(),
        },
        SlashCommandSpec {
            name: "setup".into(),
            description: "Write a thing".into(),
            argument_hint: None,
            argument_items: Vec::new(),
        },
    ];
    let query = |reference| SuggestionQuery {
        text: "/th",
        commands: &commands,
        models: &[],
        thinking_levels: &[],
        login_providers: &[],
        extra_providers: &[],
        cwd: Path::new("."),
        force_path: false,
        reference,
    };
    let reference = suggestions(query(true)).expect("the reference list");
    let names: Vec<_> = reference
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(
        names,
        ["theme", "setup"],
        "a description hit follows the name hit"
    );
    let shell = suggestions(query(false)).expect("the shell list");
    let names: Vec<_> = shell.items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(
        names,
        ["theme"],
        "off, a description is not a command match"
    );
}

#[test]
fn a_completion_never_reaches_the_composer_under_a_modal() {
    let mut model = shell(true, 120, 44);
    with_list(&mut model);
    model.overlay = Some(davinci_tui::davinci::model::Overlay::Cogitator);
    // A modal owns the keys; the list behind it is not accepted or dismissed by Tab.
    let before = model.composer.to_string();
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.composer.to_string(), before);
    model.dismiss_suggestions();
    assert_eq!(model.screen, Screen::Agent);
}
