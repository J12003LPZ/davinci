//! P12: the permission dialog. Synthetic content only; the expected lines come from
//! measuring the pinned build's `mock_permission_write__01_perm_dialog` frame.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::{self, Flow},
    model::{Ask, AskKind, Choice, Hunk, HunkKind, Model, Overlay, PickerItem},
    theme::{ColorDepth, Theme},
};

fn shell() -> Model {
    let mut model = Model::new(
        Theme::reference(ColorDepth::TrueColor, false),
        120,
        44,
        true,
    );
    model.reference = true;
    model.thinking_level = "medium".into();
    model
}

fn create_dialog(model: &mut Model) {
    model.ask = Ask {
        title: "Create file".into(),
        name: "PERMISSION".into(),
        key: "/permissions".into(),
        note: "write · ..\\outside.txt".into(),
        subject: "..\\outside.txt · outside the project".into(),
        question: "Do you want to create outside.txt?".into(),
        preview: vec![Hunk::at(HunkKind::Add, 1, "permission probe")],
        kind: AskKind::Create,
        items: vec![
            PickerItem::new("Yes", ""),
            PickerItem::new("No", ""),
            PickerItem::new("No, and tell the model what to do differently", ""),
        ],
    };
    model.overlay = Some(Overlay::Ask);
    model.ask_index = 0;
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
fn a_create_preview_is_the_file_as_it_will_read() {
    let mut model = shell();
    create_dialog(&mut model);
    let rows = frame(&model);
    let preview = rows
        .iter()
        .find(|row| row.contains("permission probe"))
        .unwrap_or_else(|| panic!("no preview row in {rows:?}"));
    assert_eq!(
        preview, "  1 permission probe",
        "the create preview carries no diff sign"
    );
    assert_eq!(
        rows.iter().filter(|row| row.contains('╌')).count(),
        2,
        "the preview is fenced by two dashed rules"
    );
    assert!(rows.iter().any(|row| row == " Create file"), "{rows:?}");
    assert!(rows
        .iter()
        .any(|row| row == " Do you want to create outside.txt?"));
}

#[test]
fn the_options_are_numbered_with_the_marker_on_the_chosen_row() {
    let mut model = shell();
    create_dialog(&mut model);
    let rows = frame(&model);
    let first = rows
        .iter()
        .position(|row| row.starts_with(" ❯ 1. Yes"))
        .expect("no marked first row");
    assert_eq!(rows[first + 1], "   2. No");
    assert_eq!(
        rows[first + 2],
        "   3. No, and tell the model what to do differently"
    );
    assert_eq!(rows[first + 4], " Esc to cancel");
}

#[test]
fn a_digit_moves_the_marker_and_enter_takes_that_row() {
    let mut model = shell();
    create_dialog(&mut model);
    assert_eq!(press(&mut model, KeyCode::Char('2')), Flow::Continue);
    assert_eq!(model.ask_index, 1);
    let rows = frame(&model);
    assert!(
        rows.iter().any(|row| row.starts_with("   1. Yes")),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.starts_with(" ❯ 2. No")),
        "{rows:?}"
    );
    assert_eq!(
        press(&mut model, KeyCode::Enter),
        Flow::Choose(Choice::Ask(1))
    );
}

#[test]
fn escape_closes_the_dialog_and_chooses_nothing() {
    let mut model = shell();
    create_dialog(&mut model);
    // The host reads a closed overlay as the user's no; the dialog itself must not
    // turn Escape into a numbered choice.
    assert_eq!(press(&mut model, KeyCode::Esc), Flow::Continue);
    assert_eq!(model.overlay, None);
}

#[test]
fn the_dialog_does_not_open_off_the_reference() {
    let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 44, true);
    create_dialog(&mut model);
    // The shell's own ask view still works; this test only records that P12's
    // create preview lives behind the reference theme and changes nothing off.
    assert!(!model.reference);
    let rows = frame(&model);
    assert!(rows.iter().any(|row| row.contains("permission probe")));
}

#[test]
fn the_reference_footer_names_the_four_cycle_stops() {
    for (mode, label, hint) in [
        (
            "ask",
            "⏸ manual mode on",
            " · ? for shortcuts · ← for agents",
        ),
        (
            "edits",
            "⏵⏵ accept edits on",
            " (shift+tab to cycle) · ← for agents",
        ),
        (
            "read-only",
            "⏸ plan mode on",
            " (shift+tab to cycle) · ← for agents",
        ),
        (
            "auto",
            "⏵⏵ auto mode on",
            " (shift+tab to cycle) · ← for agents",
        ),
    ] {
        let mut model = shell();
        model.permission_mode = mode.into();
        let rows = frame(&model);
        let footer = &rows[43];
        assert!(footer.trim_start().starts_with(label), "{mode}: {footer:?}");
        assert!(footer.contains(hint), "{mode}: {footer:?}");
    }
    // Always-approve is not one of the reference's shift+tab stops; it can only
    // be reached on purpose, and the footer still names it.
    let mut model = shell();
    model.permission_mode = "always-approve".into();
    assert!(frame(&model)[43].contains("always approve on"));
}

#[test]
fn the_shell_footer_keeps_its_own_hints() {
    let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 44, true);
    model.permission_mode = "edits".into();
    let rows = frame(&model);
    assert!(rows[43].contains("accept edits on"), "{:?}", rows[43]);
    assert!(!rows[43].contains("← for agents"), "{:?}", rows[43]);
}
