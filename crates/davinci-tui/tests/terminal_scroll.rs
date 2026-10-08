//! The conversation scrolls: wheel, PgUp/PgDn and a scrollbar of its own,
//! since the alternate screen has no terminal scrollbar.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use davinci_tui::davinci::{
    app,
    model::{Entry, Model, Overlay, Screen},
    runtime::{route_scroll_mouse, strip_scrollbar},
    theme::{ColorDepth, State, Theme},
    views::scrollbar::{Scrollbar, THUMB, TRACK, WHEEL_ROWS},
};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

fn model(turns: usize) -> Model {
    let mut m = Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        WIDTH,
        HEIGHT,
        false,
    );
    for turn in 0..turns {
        m.transcript.push(Entry::user(&format!("turn {turn:03}")));
        m.transcript.push(Entry::Gap);
    }
    m
}

fn frame(m: &Model) -> Vec<String> {
    app::compose(m, m.height)
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn shows(m: &Model, needle: &str) -> bool {
    frame(m).iter().any(|row| row.contains(needle))
}

/// Rows scrolled up from the newest, as the next frame draws it.
fn offset(m: &Model) -> usize {
    app::transcript_scrollbar(m).map_or(0, |bar| bar.offset)
}

fn bar(m: &Model) -> Scrollbar {
    app::transcript_scrollbar(m).expect("scrollbar")
}

fn press(m: &mut Model, code: KeyCode) -> app::Flow {
    app::handle_key(m, KeyEvent::new(code, KeyModifiers::NONE))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// What the session does with a pointer event: hit-test against the bar as
/// last drawn.
fn point(m: &mut Model, grab: &mut Option<u16>, event: MouseEvent) -> bool {
    let drawn = app::transcript_scrollbar(m);
    route_scroll_mouse(m, grab, drawn, event)
}

fn push_turns(m: &mut Model, from: usize, to: usize) {
    for turn in from..to {
        m.transcript.push(Entry::user(&format!("turn {turn:03}")));
        m.transcript.push(Entry::Gap);
    }
}

#[test]
fn a_long_conversation_draws_a_scrollbar_in_the_last_column() {
    let m = model(60);
    let bar = bar(&m);
    assert_eq!(bar.column, WIDTH - 1);
    assert_eq!(bar.offset, 0);
    assert!(bar.drawn);
    let rows = frame(&m);
    let column: Vec<String> = rows[usize::from(bar.top)..usize::from(bar.top + bar.height)]
        .iter()
        .map(|row| row.chars().nth(usize::from(WIDTH - 1)).unwrap().to_string())
        .collect();
    assert!(column.iter().all(|cell| cell == TRACK || cell == THUMB));
    // Following the newest, the thumb sits at the bottom of the track.
    assert_eq!(column.last().unwrap(), THUMB);
    assert_eq!(column.first().unwrap(), TRACK);
}

#[test]
fn a_conversation_that_fits_has_no_scrollbar_and_ignores_the_wheel() {
    let m = model(2);
    assert!(app::compose_frame(&m, HEIGHT).scrollbar.is_none());
    assert!(!frame(&m).iter().any(|row| row.contains(THUMB)));
    // Grown one row at a time through the point where it stops fitting:
    // a bar only ever appears with something to scroll.
    let mut m = model(0);
    m.transcript.push(Entry::user("start"));
    let mut saw_bar = false;
    for _ in 0..60 {
        m.transcript.push(Entry::Gap);
        match app::compose_frame(&m, HEIGHT).scrollbar {
            Some(bar) => {
                saw_bar = true;
                assert!(bar.max_offset() > 0, "{bar:?}");
            }
            None => assert!(!app::scroll_transcript(&mut m.clone(), 1)),
        }
    }
    assert!(saw_bar);
}

#[test]
fn the_wheel_scrolls_back_and_forth() {
    let mut m = model(60);
    assert!(shows(&m, "turn 059"));
    let mut grab = None;
    for _ in 0..10 {
        assert!(point(
            &mut m,
            &mut grab,
            mouse(MouseEventKind::ScrollUp, 10, 5)
        ));
    }
    assert_eq!(offset(&m), 10 * WHEEL_ROWS as usize);
    assert!(!shows(&m, "turn 059"));
    assert!(shows(&m, "lines below"));
    for _ in 0..10 {
        point(&mut m, &mut grab, mouse(MouseEventKind::ScrollDown, 10, 5));
    }
    assert_eq!(offset(&m), 0);
    assert!(shows(&m, "turn 059"));
    assert!(!shows(&m, "lines below"));
}

#[test]
fn scrolling_to_the_top_reaches_the_banner_and_first_turn() {
    let mut m = model(60);
    assert!(!shows(&m, "DaVinci"));
    let max = bar(&m).max_offset();
    app::scroll_transcript(&mut m, max as isize + 50);
    assert_eq!(offset(&m), max);
    assert!(shows(&m, "DaVinci"));
    assert!(shows(&m, "turn 000"));
    assert_eq!(bar(&m).thumb().0, 0);
}

#[test]
fn page_keys_scroll_a_one_row_draft_but_not_a_longer_one() {
    let mut m = model(60);
    m.composer.set_text("draft");
    press(&mut m, KeyCode::PageUp);
    let page = offset(&m);
    // Two rows short of the view: one of overlap, one under the notice.
    assert_eq!(page, usize::from(bar(&m).height) - 2);
    press(&mut m, KeyCode::PageDown);
    assert_eq!(offset(&m), 0);
    assert_eq!(m.composer.editor().get_text(), "draft");

    m.composer.set_text("one\ntwo\nthree");
    press(&mut m, KeyCode::PageUp);
    assert_eq!(offset(&m), 0);
    // A single line long enough to wrap belongs to the editor too.
    m.composer.set_text("word ".repeat(30));
    press(&mut m, KeyCode::PageUp);
    assert_eq!(offset(&m), 0);
}

#[test]
fn a_page_up_keeps_one_row_of_overlap_under_the_notice() {
    let mut m = model(60);
    m.composer.set_text("draft");
    let rows = |m: &Model| -> Vec<String> {
        let bar = bar(m);
        frame(m)[usize::from(bar.top)..usize::from(bar.top + bar.height)].to_vec()
    };
    let before = rows(&m);
    press(&mut m, KeyCode::PageUp);
    let after = rows(&m);
    // The old first row is still on screen, just above the notice line.
    let n = after.len();
    assert_eq!(after[n - 2], before[0]);
    assert!(after[n - 1].contains("lines below"));
}

#[test]
fn new_output_does_not_drag_a_scrolled_view() {
    let mut m = model(60);
    app::scroll_transcript(&mut m, 20);
    let before = frame(&m);
    push_turns(&mut m, 60, 70);
    let after = frame(&m);
    assert_eq!(before[..10], after[..10]);
    assert!(after.iter().any(|row| row.contains("lines below")));
}

#[test]
fn calls_settling_below_the_view_do_not_move_it() {
    let mut m = model(60);
    // A turn is running: a group of reads in flight, drawn as two rows.
    m.running = true;
    for file in ["a.rs", "b.rs", "c.rs"] {
        m.transcript.push(Entry::tool(
            State::Read,
            "instrumenta",
            &format!("read src/{file}"),
            None,
        ));
    }
    app::scroll_transcript(&mut m, 20);
    let before = frame(&m);
    // The reads come back and the turn ends: the group collapses to a row.
    for entry in m.transcript.iter_mut() {
        if let Entry::Tool { duration, .. } = entry {
            *duration = Some("0.1s".into());
        }
    }
    m.running = false;
    let after = frame(&m);
    assert_eq!(before[..10], after[..10]);
    assert!(offset(&m) > 0);
}

#[test]
fn sending_a_turn_returns_to_the_newest() {
    let mut m = model(60);
    app::scroll_transcript(&mut m, 30);
    m.composer.set_text("next");
    assert_eq!(
        press(&mut m, KeyCode::Enter),
        app::Flow::Submit("next".into())
    );
    assert_eq!(offset(&m), 0);
    assert!(shows(&m, "❯ next"));
}

#[test]
fn a_resize_drops_the_scroll() {
    let mut m = model(60);
    app::scroll_transcript(&mut m, 30);
    m.width = 100;
    assert_eq!(offset(&m), 0);
}

#[test]
fn a_conversation_replaced_in_one_step_opens_at_its_newest() {
    let mut m = model(60);
    app::scroll_transcript(&mut m, 30);
    // No frame is drawn between the swap and the next one.
    let mut other = model(90).transcript;
    other[0] = Entry::user("another conversation");
    m.transcript = other;
    assert_eq!(offset(&m), 0);
    assert!(shows(&m, "turn 089"));
}

#[test]
fn a_cleared_conversation_does_not_come_back_scrolled() {
    let mut m = model(60);
    app::scroll_transcript(&mut m, 30);
    m.transcript.truncate(10);
    // The runtime draws after every event; that frame sees the shorter
    // conversation and drops the scroll.
    let _ = frame(&m);
    push_turns(&mut m, 100, 180);
    assert_eq!(offset(&m), 0);
    assert!(shows(&m, "turn 179"));
}

#[test]
fn a_narrow_window_scrolls_without_drawing_the_bar() {
    let mut m = model(60);
    m.width = 16;
    let bar = bar(&m);
    assert!(!bar.drawn);
    assert!(app::scroll_transcript(&mut m, 10));
    assert_eq!(offset(&m), 10);
    assert!(frame(&m)
        .iter()
        .all(|row| !row.contains(THUMB) && !row.ends_with(TRACK)));
    // Nothing to press on.
    let mut grab = None;
    let column = bar.column;
    assert!(!point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::Down(MouseButton::Left), column, bar.top)
    ));
}

#[test]
fn pressing_the_track_jumps_and_dragging_the_thumb_follows() {
    let mut m = model(60);
    let bar = bar(&m);
    let mut grab = None;
    // Press the top of the track: the thumb centres there, at the top.
    assert!(point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::Down(MouseButton::Left), bar.column, bar.top)
    ));
    assert!(grab.is_some());
    assert_eq!(offset(&m), bar.max_offset());
    // Drag past the bottom: back to the newest.
    point(
        &mut m,
        &mut grab,
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.column,
            bar.top + bar.height + 5,
        ),
    );
    assert_eq!(offset(&m), 0);
    assert!(point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::Up(MouseButton::Left), bar.column, 0)
    ));
    assert!(grab.is_none());
}

#[test]
fn a_press_off_the_scrollbar_is_left_to_text_selection() {
    let mut m = model(60);
    // A grab left over from a release the terminal never reported.
    let mut grab = Some(1);
    assert!(!point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::Down(MouseButton::Left), 5, 5)
    ));
    assert!(grab.is_none());
    assert!(!point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::Drag(MouseButton::Left), 9, 9)
    ));
    assert_eq!(offset(&m), 0);
}

#[test]
fn copied_rows_leave_the_scrollbar_out() {
    let m = model(60);
    let bar = bar(&m);
    let mut rows = frame(&m);
    strip_scrollbar(&mut rows, &bar);
    for row in &rows[usize::from(bar.top)..usize::from(bar.top + bar.height)] {
        let trimmed = row.trim_end();
        assert!(
            !trimmed.ends_with(TRACK) && !trimmed.ends_with(THUMB),
            "{row:?}"
        );
    }
    assert!(rows.iter().any(|row| row.trim_end() == "❯ turn 059"));
}

#[test]
fn over_a_list_the_wheel_moves_the_selection() {
    let mut m = model(60);
    davinci_tui::davinci::fixtures::dress(&mut m);
    m.toggle_overlay(Overlay::Cogitator);
    assert!(app::transcript_scrollbar(&m).is_none());
    let selected = |m: &Model| frame(m).into_iter().find(|row| row.contains('❯'));
    let before = selected(&m);
    assert!(before.is_some());
    let mut grab = None;
    point(&mut m, &mut grab, mouse(MouseEventKind::ScrollDown, 10, 5));
    assert_ne!(selected(&m), before);
    assert_eq!(m.screen, Screen::Agent);
}

/// WOR-41: the pointer decides which surface owns the wheel while a
/// completion list floats over the conversation.
#[test]
fn the_wheel_over_the_completion_list_steps_it_and_elsewhere_scrolls_the_conversation() {
    let mut m = model(60);
    davinci_tui::davinci::fixtures::dress(&mut m);
    m.transcript.clear();
    push_turns(&mut m, 0, 60);
    m.slash_commands = ["settings", "sessions", "model", "compact"]
        .into_iter()
        .map(|name| davinci_tui::SlashCommandSpec {
            name: name.into(),
            ..Default::default()
        })
        .collect();
    m.type_char("/");
    assert!(m.suggestions.is_some(), "a bare slash opens the list");
    app::scroll_transcript(&mut m, 12);
    let scrolled = offset(&m);
    assert_eq!(scrolled, 12);

    let list = app::compose_frame(&m, m.height)
        .suggestion_rows
        .expect("the open list reports the rows it covers");
    assert!(!list.is_empty());
    let mut grab = None;

    // Over the list: the selection moves, the conversation stays put.
    let picked = m.suggestion_index;
    assert!(point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::ScrollDown, 10, list.start)
    ));
    assert_ne!(m.suggestion_index, picked, "the wheel steps the list");
    assert_eq!(offset(&m), scrolled, "the conversation did not scroll");
    let after_down = m.suggestion_index;
    point(
        &mut m,
        &mut grab,
        mouse(MouseEventKind::ScrollUp, 10, list.end - 1),
    );
    assert_ne!(m.suggestion_index, after_down);
    assert_eq!(offset(&m), scrolled);

    // Over the conversation: it scrolls and the list keeps its selection.
    let kept = m.suggestion_index;
    point(&mut m, &mut grab, mouse(MouseEventKind::ScrollUp, 10, 2));
    assert_eq!(offset(&m), scrolled + WHEEL_ROWS as usize);
    assert_eq!(m.suggestion_index, kept);
    assert!(m.suggestions.is_some());
}

#[test]
fn without_a_completion_list_no_row_is_a_list_row() {
    let m = model(60);
    assert!(app::compose_frame(&m, m.height).suggestion_rows.is_none());
}

/// WOR-42: folding or unfolding tool output changes how many rows every call
/// takes, so a stored row number no longer points at what the reader was on.
#[test]
fn folding_tool_output_while_scrolled_does_not_keep_a_stale_row_anchor() {
    let mut m = model(60);
    for n in 0..20 {
        m.transcript.insert(
            4 + n,
            Entry::tool(State::Done, "manus", &format!("ls dir{n}"), Some("0.1s")),
        );
    }
    app::scroll_transcript(&mut m, 30);
    assert!(offset(&m) > 0);
    m.show_tool_output = !m.show_tool_output;
    assert_eq!(offset(&m), 0, "re-folded: back to the newest");
    assert!(shows(&m, "turn 059"));
    // Scrolling again anchors against the new fold, and survives frames.
    app::scroll_transcript(&mut m, 10);
    assert_eq!(offset(&m), 10);
    let _ = frame(&m);
    assert_eq!(offset(&m), 10);
    // And back the other way.
    m.show_tool_output = !m.show_tool_output;
    assert_eq!(offset(&m), 0);
}

#[test]
fn a_resize_while_scrolled_through_folded_output_drops_the_anchor() {
    let mut m = model(60);
    app::scroll_transcript(&mut m, 25);
    m.height = 30;
    assert!(offset(&m) > 0, "a height change keeps the anchor");
    m.width = 70;
    assert_eq!(offset(&m), 0);
    assert!(shows(&m, "turn 059"));
}

