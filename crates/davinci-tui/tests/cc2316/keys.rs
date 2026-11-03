//! P09b: the key conflicts the plan names. The reference gives ctrl+t to the task band
//! and ctrl+o to the verbose transcript; DaVinci's tool-output toggle keeps its
//! `/config` setting and the legacy shell's own bindings.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::{self, Flow},
    model::{Model, Screen},
    theme::{ColorDepth, Theme},
};

fn shell(reference: bool) -> Model {
    let theme = if reference {
        Theme::reference(ColorDepth::TrueColor, false)
    } else {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    };
    let mut model = Model::new(theme, 120, 44, true);
    model.reference = reference;
    model.thinking_level = "medium".into();
    model
}

fn ctrl(model: &mut Model, ch: char) -> Flow {
    app::handle_key(
        model,
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
    )
}

#[test]
fn ctrl_t_toggles_the_task_band_and_leaves_thinking_alone() {
    for reference in [true, false] {
        let mut model = shell(reference);
        let level = model.thinking_level.clone();
        assert!(!model.checklist_open);
        assert_eq!(ctrl(&mut model, 't'), Flow::Continue);
        assert!(model.checklist_open, "ctrl+t did not open the task band");
        assert_eq!(model.thinking_level, level, "ctrl+t changed the effort");
        assert_eq!(ctrl(&mut model, 't'), Flow::Continue);
        assert!(!model.checklist_open, "ctrl+t did not close the task band");
    }
}

#[test]
fn ctrl_o_opens_the_verbose_transcript_and_leaves_tool_output_alone() {
    for reference in [true, false] {
        let mut model = shell(reference);
        let show = model.show_tool_output;
        assert!(model.viewer.is_none());
        assert_eq!(ctrl(&mut model, 'o'), Flow::Continue);
        assert!(model.viewer.is_some(), "ctrl+o did not open the transcript");
        assert_eq!(
            model.show_tool_output, show,
            "ctrl+o changed the tool-output setting"
        );
        app::handle_key(&mut model, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(model.viewer.is_none(), "Esc did not close the transcript");
    }
}

#[test]
fn the_task_band_draws_nothing_without_plan_steps() {
    // keys_panels__05_todos was captured with no todos: ctrl+t leaves the screen as it
    // was. The band must not invent rows.
    let mut model = shell(true);
    let before: Vec<String> = app::compose(&model, model.height)
        .iter()
        .map(|line| line.to_string())
        .collect();
    ctrl(&mut model, 't');
    let after: Vec<String> = app::compose(&model, model.height)
        .iter()
        .map(|line| line.to_string())
        .collect();
    assert_eq!(before, after, "an empty task band changed the screen");
}

#[test]
fn the_append_only_transcript_viewer_is_not_a_modal() {
    // The viewer is a surface, not an overlay: the composer keeps its text and the
    // transcript is what changes.
    let mut model = shell(true);
    model.composer.set_text("a draft");
    ctrl(&mut model, 'o');
    assert_eq!(model.composer.to_string(), "a draft");
    assert_eq!(model.screen, Screen::Agent);
}
