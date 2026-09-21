//! The optional graph is a view, not the owner of the user's conversation.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app, fixtures,
    model::{Model, Screen},
    theme::{ColorDepth, Theme},
};

fn model() -> Model {
    let mut m = Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        120,
        40,
        false,
    );
    m.graph_run = Some(fixtures::blueprint_graph());
    m
}
fn key(m: &mut Model, code: KeyCode) -> app::Flow {
    app::handle_key(m, KeyEvent::new(code, KeyModifiers::NONE))
}
fn control(m: &mut Model, ch: char) -> app::Flow {
    app::handle_key(m, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
}
fn frame(m: &Model) -> String {
    app::compose(m, m.height)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn graph_keeps_the_conversation_draft_visible() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    m.composer.set_text("Draft for the main conversation");
    assert!(frame(&m).contains("Draft for the main conversation"));
}

#[test]
fn graph_tab_moves_to_input_without_running_graph_shortcuts() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    m.composer.set_text("Draft ");
    key(&mut m, KeyCode::Tab);
    for ch in "xpr".chars() {
        assert_eq!(key(&mut m, KeyCode::Char(ch)), app::Flow::Continue);
    }
    assert_eq!(m.composer.editor().get_text(), "Draft xpr");
    assert_eq!(
        m.graph_run.as_ref().unwrap().phase,
        fixtures::blueprint_graph().phase
    );
}

#[test]
fn graph_input_submits_to_the_conversation_not_the_selected_worker() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    key(&mut m, KeyCode::Tab);
    m.composer.set_text("Explain the existing tests");
    assert_eq!(
        key(&mut m, KeyCode::Enter),
        app::Flow::Submit("Explain the existing tests".into())
    );
    assert!(m.graph_run.is_some());
}

#[test]
fn closing_graph_does_not_cancel_it_or_destroy_input() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    let original = m.graph_run.as_ref().unwrap().phase.clone();
    m.composer.set_text("Keep this 界 draft");
    assert_eq!(key(&mut m, KeyCode::Esc), app::Flow::Continue);
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "Keep this 界 draft");
    assert_eq!(m.graph_run.as_ref().unwrap().phase, original);
}

#[test]
fn background_status_does_not_replace_the_prompt() {
    let mut m = model();
    m.screen = Screen::Agent;
    m.composer.set_text("Continue another question");
    let text = frame(&m);
    assert!(
        text.contains("Background"),
        "missing background status: {text}"
    );
    assert!(text.contains("Continue another question"));
    assert_eq!(m.screen, Screen::Agent);
}

#[test]
fn graph_view_never_overflows_after_resize() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    m.composer.set_text("Do not lose this draft 👩‍💻");
    for (width, height) in [
        (40, 12),
        (80, 24),
        (100, 30),
        (120, 40),
        (160, 50),
        (1, 1),
        (0, 0),
    ] {
        m.width = width;
        m.height = height;
        let rows = app::compose(&m, height);
        assert_eq!(rows.len(), usize::from(height));
        for row in rows {
            assert!(davinci_tui::davinci::ui::run_width(&row.spans) <= width);
        }
        assert_eq!(m.composer.editor().get_text(), "Do not lose this draft 👩‍💻");
    }
}

#[test]
fn ctrl_d_cannot_exit_with_a_nonempty_draft() {
    let mut m = model();
    m.composer.set_text("unfinished work");
    assert_ne!(control(&mut m, 'd'), app::Flow::Quit);
    assert_eq!(m.composer.editor().get_text(), "unfinished work");
}

#[test]
fn ctrl_e_is_a_line_motion_not_an_unrelated_panel() {
    let mut m = model();
    m.composer.set_text("unfinished work");
    control(&mut m, 'a');
    control(&mut m, 'e');
    assert_eq!(m.composer.cursor(), "unfinished work".len());
    assert!(!m.codex_open());
}

#[test]
fn ctrl_o_expands_tool_output_instead_of_changing_models() {
    let mut m = model();
    let expanded = m.show_tool_output;
    control(&mut m, 'o');
    assert_eq!(m.show_tool_output, !expanded);
    assert!(m.overlay.is_none());
}

#[test]
fn graph_input_accepts_multiline_paste_without_submitting() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    key(&mut m, KeyCode::Tab);
    m.paste("line one\r\nline two 👩‍💻");
    assert_eq!(m.composer.editor().get_text(), "line one\nline two 👩‍💻");
    assert!(m.graph_run.is_some());
}

#[test]
fn graph_navigation_rejects_paste_into_an_unfocused_conversation() {
    let mut m = model();
    m.screen = Screen::GraphRun;
    m.composer.set_text("Keep existing draft");
    m.paste("do not insert");
    assert_eq!(m.composer.editor().get_text(), "Keep existing draft");
}
