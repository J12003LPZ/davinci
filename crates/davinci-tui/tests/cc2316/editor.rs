//! P08: the composer. Every text here is synthetic; positions and limits come from
//! measuring the pinned build, and nothing of the reference is stored here.
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::{self, Flow},
    model::{Model, Overlay, Screen, VimMode, VimState, PASTE_COLLAPSE_CHARS},
    theme::{ColorDepth, Theme},
};
use unicode_width::UnicodeWidthStr;

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

fn press(model: &mut Model, code: KeyCode) -> Flow {
    app::handle_key(model, KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(model: &mut Model, ch: char) -> Flow {
    app::handle_key(
        model,
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
    )
}

fn type_text(model: &mut Model, text: &str) {
    for ch in text.chars() {
        press(model, KeyCode::Char(ch));
    }
}

fn frame(model: &Model) -> Vec<String> {
    app::compose(model, model.height)
        .iter()
        .map(|line| line.to_string())
        .collect()
}

fn footer(model: &Model) -> String {
    frame(model).last().cloned().unwrap_or_default()
}

fn draft(model: &Model) -> String {
    model.composer.to_string()
}

// ------------------------------------------------------------------ paste

#[test]
fn a_paste_collapses_above_ten_thousand_characters_and_not_at_ten_thousand() {
    assert_eq!(PASTE_COLLAPSE_CHARS, 10_000);
    let mut at = shell(true, 120, 44);
    at.paste(&"a".repeat(10_000));
    assert_eq!(
        draft(&at),
        "a".repeat(10_000),
        "10,000 characters stay in the draft"
    );
    assert!(!at.composer.has_collapsed_paste());

    let mut over = shell(true, 120, 44);
    over.paste(&"a".repeat(10_001));
    assert_eq!(draft(&over), "[Pasted text #1]");
    assert!(over.composer.has_collapsed_paste());
    assert_eq!(over.composer.expanded_text(), "a".repeat(10_001));
}

#[test]
fn the_limit_counts_characters_not_bytes_cells_or_carriage_returns() {
    // Wide glyphs: 10,000 of them are 30,000 bytes and 20,000 cells, and still fit.
    let mut wide = shell(true, 120, 44);
    wide.paste(&"你".repeat(10_000));
    assert!(!wide.composer.has_collapsed_paste());
    wide.paste("你");
    assert_eq!(
        draft(&wide).chars().count(),
        10_001,
        "a second paste adds to the draft"
    );
    let mut wide_over = shell(true, 120, 44);
    wide_over.paste(&"你".repeat(10_001));
    assert!(wide_over.composer.has_collapsed_paste());

    // Combining marks count one each: 5,000 of "e" + U+0301 are 10,000 characters.
    let mut marks = shell(true, 120, 44);
    marks.paste(&"e\u{301}".repeat(5_000));
    assert!(!marks.composer.has_collapsed_paste());
    let mut marks_over = shell(true, 120, 44);
    marks_over.paste(&format!("{}x", "e\u{301}".repeat(5_000)));
    assert!(marks_over.composer.has_collapsed_paste());

    // CRLF is one newline: 5,000 of them are 5,000 characters, and a chip counts lines.
    let mut crlf = shell(true, 120, 44);
    crlf.paste(&"\r\n".repeat(5_000));
    assert!(!crlf.composer.has_collapsed_paste());
    let mut lines = shell(true, 120, 44);
    lines.paste(&format!("{}\r\n{}", "a".repeat(6_000), "b".repeat(5_000)));
    assert_eq!(draft(&lines), "[Pasted text #1 +1 lines]");
}

#[test]
fn pasting_the_same_text_again_puts_it_in_the_draft_with_the_middle_cut() {
    let mut model = shell(true, 120, 44);
    let text = format!(
        "{}{}{}",
        "h".repeat(600),
        "m".repeat(9_000),
        "t".repeat(600)
    );
    model.paste(&text);
    assert_eq!(footer(&model).trim(), "paste again to expand");
    model.paste(&text);
    let shown = draft(&model);
    assert!(shown.starts_with(&"h".repeat(500)) && shown.ends_with(&"t".repeat(500)));
    assert!(
        shown.contains("[...Truncated text #2 +0 lines...]"),
        "{shown:?}"
    );
    assert_eq!(shown.chars().count(), 500 + 34 + 500);
    assert!(!footer(&model).contains("paste again to expand"));
    // Another text is a paste of its own, beside what is already there.
    model.paste("other");
    assert!(draft(&model).ends_with("other"));
}

#[test]
fn a_collapsed_paste_is_sent_whole_and_recalled_whole() {
    let mut model = shell(true, 120, 44);
    let text = "z".repeat(10_050);
    type_text(&mut model, "see ");
    model.paste(&text);
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, format!("see {text}"));
    assert_eq!(draft(&model), "");
    assert!(!model.composer.has_collapsed_paste());
    press(&mut model, KeyCode::Up);
    assert_eq!(
        draft(&model),
        format!("see {text}"),
        "history holds what was sent"
    );
}

#[test]
fn undo_across_a_collapsed_paste_takes_the_chip_out_and_a_new_paste_gets_a_new_number() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "ab");
    model.paste(&"q".repeat(10_001));
    assert_eq!(draft(&model), "ab[Pasted text #1]");
    model.composer.editor_mut().undo();
    assert_eq!(draft(&model), "ab");
    assert!(!model.composer.has_collapsed_paste());
    model.paste(&"q".repeat(10_001));
    assert_eq!(draft(&model), "ab[Pasted text #2]");
    assert_eq!(
        model.composer.expanded_text(),
        format!("ab{}", "q".repeat(10_001))
    );
}

#[test]
fn control_characters_and_escape_sequences_never_reach_the_draft() {
    let mut model = shell(true, 120, 44);
    model.paste("a\u{1b}[31mb\u{1b}]0;title\u{7}c\u{7}d\0e\u{7f}f\u{85}g\u{1b}[200~h");
    assert_eq!(draft(&model), "abcdefgh");
    // A tab is four spaces and a lone carriage return a newline.
    let mut model = shell(true, 120, 44);
    model.paste("x\ty\rz");
    assert_eq!(draft(&model), "x    y\nz");
    // A paste of nothing but controls adds nothing.
    let mut model = shell(true, 120, 44);
    model.paste("\u{1b}[2J\u{7}");
    assert_eq!(draft(&model), "");
}

#[test]
fn a_hundred_thousand_character_paste_stays_responsive() {
    let mut model = shell(true, 120, 44);
    let text = "word ".repeat(20_000);
    let started = Instant::now();
    model.paste(&text);
    let pasted = started.elapsed();
    let started = Instant::now();
    let rows = frame(&model);
    let drawn = started.elapsed();
    let started = Instant::now();
    model.paste(&text);
    let again = started.elapsed();
    println!("100,000 characters: paste {pasted:?}, first frame {drawn:?}, paste again {again:?}");
    assert!(rows.iter().any(|row| row.contains("[Pasted text #1]")));
    for (what, took) in [("paste", pasted), ("frame", drawn), ("paste again", again)] {
        assert!(took < Duration::from_secs(2), "{what} took {took:?}");
    }
    // The same text again is the second paste, which opens the first (cut in the
    // middle on screen, whole when sent).
    assert!(draft(&model).chars().count() < 1_100);
    assert_eq!(model.composer.expanded_text(), text);
}

#[test]
fn a_wide_glyph_or_a_combining_mark_on_the_clip_edge_keeps_every_row_inside_the_window() {
    for width in [60_u16, 61, 80] {
        let room = usize::from(width) - 4;
        // The wide glyph would straddle the edge; the mark must stay with its letter.
        let text = format!("{}{}", "a".repeat(room - 1), "你好e\u{301}\u{301}x");
        let mut model = shell(true, width, 30);
        model.paste(&text);
        let rows = frame(&model);
        assert!(
            rows.iter()
                .all(|row| UnicodeWidthStr::width(row.as_str()) <= usize::from(width)),
            "{width}: a row is wider than the window"
        );
        let joined: String = rows.join("\n");
        assert!(
            joined.contains("e\u{301}\u{301}"),
            "{width}: the marks left their letter"
        );
        assert!(
            joined.contains('你') && joined.contains('好'),
            "{width}: a glyph was lost"
        );
    }
}

#[test]
fn off_the_shell_collapses_as_it_did() {
    let mut model = shell(false, 120, 44);
    model.paste(&"a".repeat(1_500));
    assert!(
        draft(&model).starts_with("[paste #1"),
        "{:?}",
        draft(&model)
    );
    let mut model = shell(false, 120, 44);
    model.paste(&"a".repeat(10_001));
    assert!(!footer(&model).contains("paste again"));
}

#[test]
fn a_paste_with_a_modal_open_changes_nothing_and_is_not_kept() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "keep");
    model.overlay = Some(Overlay::Cogitator);
    model.paste(&"x".repeat(12_000));
    assert_eq!(draft(&model), "keep");
    model.overlay = None;
    assert!(!model.composer.has_collapsed_paste());
    // The same with a wide, combining draft and a resize while the modal is open.
    let mut model = shell(true, 100, 30);
    model.paste(&"你好🙂e\u{301}".repeat(2_500));
    let before = model.composer.expanded_text();
    model.overlay = Some(Overlay::Cogitator);
    model.width = 50;
    model.height = 12;
    model.paste("late");
    type_text(&mut model, "zz");
    model.overlay = None;
    assert_eq!(
        model.composer.expanded_text(),
        before,
        "text, chip and cursor survive"
    );
    assert!(model.composer.has_collapsed_paste());
}

// ------------------------------------------------------------------ editing

#[test]
fn multiline_word_movement_deletion_and_undo() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "one two");
    ctrl(&mut model, 'j');
    type_text(&mut model, "three");
    assert_eq!(draft(&model), "one two\nthree");
    ctrl(&mut model, 'w');
    assert_eq!(draft(&model), "one two\n");
    app::handle_key(
        &mut model,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
    );
    app::handle_key(
        &mut model,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT),
    );
    model.composer.editor_mut().undo();
    assert!(draft(&model).starts_with("one two"));
}

#[test]
fn history_comes_back_to_a_multiline_draft() {
    let mut model = shell(true, 120, 44);
    for sent in ["first", "second"] {
        type_text(&mut model, sent);
        press(&mut model, KeyCode::Enter);
    }
    type_text(&mut model, "alpha");
    ctrl(&mut model, 'j');
    type_text(&mut model, "beta");
    // Up moves through the draft's lines first, then into the history.
    press(&mut model, KeyCode::Up);
    assert_eq!(
        draft(&model),
        "alpha\nbeta",
        "the first Up moved within the draft"
    );
    for _ in 0..4 {
        if draft(&model) != "alpha\nbeta" {
            break;
        }
        press(&mut model, KeyCode::Up);
    }
    assert_eq!(draft(&model), "second");
    press(&mut model, KeyCode::Up);
    assert_eq!(draft(&model), "first");
    press(&mut model, KeyCode::Down);
    press(&mut model, KeyCode::Down);
    assert_eq!(
        draft(&model),
        "alpha\nbeta",
        "the draft in progress came back"
    );
}

#[test]
fn the_prompt_search_opens_over_no_history_and_says_so() {
    let mut model = shell(true, 120, 44);
    ctrl(&mut model, 'r');
    assert!(model.history_search.is_some());
    let rows = frame(&model);
    assert!(rows
        .iter()
        .any(|row| row.trim() == "Search prompts · everywhere"));
    assert!(rows.iter().any(|row| row.trim() == "No matching prompts"));
    assert!(footer(&model).contains("ctrl+s to scope"));
    ctrl(&mut model, 's');
    assert!(frame(&model).iter().any(|row| row.contains("this session")));
    type_text(&mut model, "find");
    assert!(frame(&model).iter().any(|row| row.contains("⌕ find")));
    press(&mut model, KeyCode::Esc);
    assert!(model.history_search.is_none());
    // Off, a search with nothing to search does not open, as before.
    let mut off = shell(false, 120, 44);
    ctrl(&mut off, 'r');
    assert!(off.history_search.is_none());
}

// ------------------------------------------------------------------ escape

#[test]
fn a_second_escape_clears_the_draft_into_history_within_the_reference_window() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "hello");
    press(&mut model, KeyCode::Esc);
    assert!(model.clear_pending());
    assert!(
        frame(&model)
            .iter()
            .any(|row| row.trim_end().ends_with("Esc again to clear")),
        "the label sits on the effort row"
    );
    assert!(!footer(&model).contains("Esc again to clear"));
    // 700 ms later the reference still takes the second Esc; the shell's 500 ms would not.
    model.clear_armed = Some(Instant::now() - Duration::from_millis(700));
    assert!(model.clear_pending());
    press(&mut model, KeyCode::Esc);
    assert_eq!(draft(&model), "");
    press(&mut model, KeyCode::Up);
    assert_eq!(
        draft(&model),
        "hello",
        "the cleared draft is in the history"
    );

    let mut off = shell(false, 120, 44);
    type_text(&mut off, "hello");
    press(&mut off, KeyCode::Esc);
    off.clear_armed = Some(Instant::now() - Duration::from_millis(700));
    press(&mut off, KeyCode::Esc);
    assert_eq!(draft(&off), "hello", "off, 700 ms is too late, as before");
}

#[test]
fn escape_on_a_draft_ignores_the_double_escape_action_setting() {
    // The setting (rewind, fork, none) belongs to an empty draft and is read by the
    // shell loop; a draft is cleared by two Escapes whatever it says.
    for action in ["rewind", "fork", "none"] {
        let mut model = shell(true, 120, 44);
        model.double_escape_action = action.into();
        type_text(&mut model, "draft");
        press(&mut model, KeyCode::Esc);
        press(&mut model, KeyCode::Esc);
        assert_eq!(draft(&model), "", "{action}");
    }
    // Two Escapes on an empty draft clear nothing and are left to the setting.
    let mut model = shell(true, 120, 44);
    press(&mut model, KeyCode::Esc);
    assert!(!model.clear_pending());
}

#[test]
fn the_reference_caret_is_a_steady_reversed_cell_and_the_empty_draft_suggests_nothing() {
    let mut model = shell(true, 120, 44);
    let rows = app::compose(&model, model.height);
    let composer = &rows[rows.len() - 3];
    assert_eq!(composer.to_string().trim_end_matches(' '), "❯\u{a0}");
    assert!(
        composer.spans.iter().any(|span| span
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)),
        "the caret is a reversed cell"
    );
    // It does not wink out however long the draft sits still.
    model.caret_blink = true;
    for _ in 0..40 {
        model.advance_tick();
    }
    assert!(model.caret_lit());
    let mut off = shell(false, 120, 44);
    assert!(frame(&off)[off.height as usize - 3].contains("Try"));
    off.caret_blink = true;
}

// ------------------------------------------------------------------ vim

fn vim() -> Model {
    let mut model = shell(true, 120, 44);
    model.vim = Some(VimState::default());
    model
}

#[test]
fn vim_starts_in_insert_and_dw_takes_a_word_and_the_spaces_after_it() {
    let mut model = vim();
    assert_eq!(model.vim.map(|v| v.mode), Some(VimMode::Insert));
    assert!(footer(&model).contains("-- INSERT --"));
    // 'i' is text in INSERT.
    type_text(&mut model, "ihello vim");
    assert_eq!(draft(&model), "ihello vim");
    press(&mut model, KeyCode::Esc);
    assert_eq!(model.vim.map(|v| v.mode), Some(VimMode::Normal));
    assert!(!footer(&model).contains("INSERT"));
    type_text(&mut model, "0dw");
    assert_eq!(draft(&model), "vim");
    // Keys that are not commands type nothing in NORMAL.
    type_text(&mut model, "qz");
    assert_eq!(draft(&model), "vim");
    // x, u, dd, A and o.
    type_text(&mut model, "x");
    assert_eq!(draft(&model), "im");
    type_text(&mut model, "u");
    assert_eq!(draft(&model), "vim");
    type_text(&mut model, "$a!");
    assert_eq!(draft(&model), "vim!");
    assert_eq!(model.vim.map(|v| v.mode), Some(VimMode::Insert));
    press(&mut model, KeyCode::Esc);
    type_text(&mut model, "ddi");
    assert_eq!(draft(&model), "");
    type_text(&mut model, "ab");
    press(&mut model, KeyCode::Esc);
    type_text(&mut model, "oc");
    assert_eq!(draft(&model), "ab\nc");
}

#[test]
fn vim_is_off_unless_the_setting_gave_it_and_never_runs_under_a_modal() {
    let mut off = shell(false, 120, 44);
    off.vim = Some(VimState::default());
    type_text(&mut off, "abc");
    press(&mut off, KeyCode::Esc);
    type_text(&mut off, "x");
    assert_eq!(draft(&off), "abcx", "reference off: no vim keys");

    let mut model = vim();
    type_text(&mut model, "keep");
    press(&mut model, KeyCode::Esc);
    model.overlay = Some(Overlay::Cogitator);
    type_text(&mut model, "0dwxx");
    assert_eq!(draft(&model), "keep");
    assert_eq!(model.vim.map(|v| v.mode), Some(VimMode::Normal));
}

// ------------------------------------------------------------------ image token

#[test]
fn the_image_token_trails_the_draft_and_what_is_typed_next_lands_before_it() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "look at this ");
    model.composer.insert_after_cursor("[Image #1]");
    type_text(&mut model, "please describe");
    assert_eq!(draft(&model), "look at this please describe[Image #1]");
}

// ------------------------------------------------------------------ focus

#[test]
fn no_modal_or_graph_view_takes_composer_keys() {
    for overlay in [Overlay::Cogitator, Overlay::Sessions, Overlay::Instrumenta] {
        let mut model = shell(true, 120, 44);
        type_text(&mut model, "draft");
        model.overlay = Some(overlay);
        let before = draft(&model);
        for code in [
            KeyCode::Char('z'),
            KeyCode::Backspace,
            KeyCode::Left,
            KeyCode::Delete,
        ] {
            press(&mut model, code);
        }
        ctrl(&mut model, 'w');
        ctrl(&mut model, 'u');
        model.paste("pasted");
        // The palette keeps its own query; the draft behind it is untouched.
        assert_eq!(draft(&model), before, "{overlay:?}");
    }
    let mut graph = shell(true, 120, 44);
    type_text(&mut graph, "draft");
    graph.screen = Screen::GraphRun;
    graph.graph_canvas.input_focus = false;
    type_text(&mut graph, "zz");
    graph.paste("pasted");
    assert_eq!(draft(&graph), "draft");
}

#[test]
fn keys_during_a_running_turn_edit_the_draft_and_enter_queues_it() {
    let mut model = shell(true, 120, 44);
    model.running = true;
    type_text(&mut model, "next");
    assert_eq!(draft(&model), "next");
    model.paste(&"y".repeat(10_001));
    assert!(model.composer.has_collapsed_paste());
    assert!(model.queue());
    assert_eq!(model.queued, vec![format!("next{}", "y".repeat(10_001))]);
    assert_eq!(draft(&model), "");
}

// ------------------------------------------------------------------ chips keep their text

/// `n` different characters, so a lost or reordered stretch shows.
fn distinct(n: u32) -> String {
    (0..n)
        .map(|i| char::from_u32(0x4e00 + i).expect("in the CJK block"))
        .collect()
}

#[test]
fn a_second_paste_still_sends_every_character() {
    let text = distinct(12_000);
    let mut model = shell(true, 120, 44);
    model.paste(&text);
    model.paste(&text);
    assert!(draft(&model).contains("[...Truncated text #2"));
    assert_eq!(model.composer.expanded_text(), text);
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, text);

    let mut queued = shell(true, 120, 44);
    queued.running = true;
    queued.paste(&text);
    queued.paste(&text);
    assert!(queued.queue());
    assert_eq!(queued.queued, vec![text]);
}

#[test]
fn undo_after_a_path_that_sets_the_text_brings_the_chip_back_whole() {
    let text = distinct(10_500);
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "a ");
    model.paste(&text);
    model.composer.set_text("a ");
    model.composer.editor_mut().undo();
    assert_eq!(draft(&model), "a [Pasted text #1]");
    assert_eq!(model.composer.expanded_text(), format!("a {text}"));
}

#[test]
fn history_ctrl_g_and_a_scope_switch_keep_the_chip() {
    let text = distinct(10_500);
    // History: away from the draft and back.
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "old");
    press(&mut model, KeyCode::Enter);
    model.paste(&text);
    for _ in 0..4 {
        press(&mut model, KeyCode::Up);
    }
    assert_eq!(draft(&model), "old");
    for _ in 0..4 {
        press(&mut model, KeyCode::Down);
    }
    assert_eq!(draft(&model), "[Pasted text #1]");
    assert_eq!(model.composer.expanded_text(), text);

    // Ctrl+G hands the marker back unchanged.
    let mut model = shell(true, 120, 44);
    model.paste(&text);
    let marker = draft(&model);
    model.replace_composer(format!("{marker} edited"));
    assert_eq!(model.composer.expanded_text(), format!("{text} edited"));

    // Another view's draft in between, even one that is sent.
    let mut model = shell(true, 120, 44);
    model.paste(&text);
    model.focus_agent("worker-1");
    type_text(&mut model, "to the worker");
    model.composer.submit();
    model.focus_main();
    assert_eq!(draft(&model), "[Pasted text #1]");
    assert_eq!(model.composer.expanded_text(), text);
}

#[test]
fn markers_expand_in_one_pass_and_a_typed_marker_stays_as_typed() {
    // A chip whose own text names a later chip's marker is not expanded twice.
    let first = format!("{} [Pasted text #2] {}", distinct(5_000), distinct(5_100));
    let second = distinct(10_200);
    let mut model = shell(true, 120, 44);
    model.paste(&first);
    model.paste(&second);
    assert_eq!(draft(&model), "[Pasted text #1][Pasted text #2]");
    assert_eq!(model.composer.expanded_text(), format!("{first}{second}"));

    // No such chip: what was typed stays.
    let mut typed = shell(true, 120, 44);
    type_text(&mut typed, "see [Pasted text #1] here");
    assert_eq!(typed.composer.expanded_text(), "see [Pasted text #1] here");
    let Flow::Submit(sent) = press(&mut typed, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, "see [Pasted text #1] here");
}

// ------------------------------------------------------------------ chips belong to a draft

#[test]
fn a_deleted_marker_leaves_no_chip_to_expand_a_marker_typed_later() {
    let text = distinct(12_000);
    let mut model = shell(true, 120, 44);
    model.paste(&text);
    let marker = draft(&model);
    for _ in marker.chars() {
        press(&mut model, KeyCode::Backspace);
    }
    assert_eq!(draft(&model), "");
    type_text(&mut model, "hi");
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, "hi", "the deleted paste is not sent");

    type_text(&mut model, &marker);
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, marker, "a literal marker stays as typed");
}

#[test]
fn another_views_chip_does_not_expand_a_marker_typed_in_this_one() {
    let text = distinct(12_000);
    let mut model = shell(true, 120, 44);
    model.paste(&text);
    model.focus_agent("worker-1");
    type_text(&mut model, "[Pasted text #1]");
    assert_eq!(model.composer.expanded_text(), "[Pasted text #1]");
    model.focus_main();
    assert_eq!(draft(&model), "[Pasted text #1]");
    assert_eq!(
        model.composer.expanded_text(),
        text,
        "the main draft still holds its paste"
    );
}

// ------------------------------------------------------------------ stash

fn effort_row(model: &Model) -> String {
    frame(model)
        .into_iter()
        .find(|row| row.contains("/effort"))
        .expect("the effort row is drawn")
}

#[test]
fn ctrl_s_stashes_the_draft_and_a_second_ctrl_s_brings_it_back() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "half a thought");
    assert!(!effort_row(&model).contains("stashed"));

    ctrl(&mut model, 's');
    assert_eq!(model.overlay, None, "ctrl+s does not open Sessions here");
    assert_eq!(draft(&model), "");
    assert_eq!(model.composer.editor().get_cursor(), (0, 0));
    let row = effort_row(&model);
    assert!(row.ends_with("◐ medium · /effort · › stashed  "), "{row:?}");
    assert_eq!(UnicodeWidthStr::width(row.as_str()), 120);
    model.composer.editor_mut().undo();
    assert_eq!(draft(&model), "", "undo cannot bring a stashed draft back");

    ctrl(&mut model, 's');
    assert_eq!(draft(&model), "half a thought");
    assert!(!effort_row(&model).contains("stashed"));
}

#[test]
fn ctrl_s_with_nothing_to_stash_does_nothing_and_off_it_keeps_opening_sessions() {
    let mut model = shell(true, 120, 44);
    ctrl(&mut model, 's');
    assert_eq!(model.overlay, None);
    assert!(!effort_row(&model).contains("stashed"));

    let mut off = shell(false, 120, 44);
    type_text(&mut off, "x");
    ctrl(&mut off, 's');
    assert_eq!(off.overlay, Some(Overlay::Sessions));
    assert_eq!(draft(&off), "x", "nothing was stashed");
}

#[test]
fn restoring_over_a_draft_swaps_the_two_so_nothing_is_lost() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "first");
    ctrl(&mut model, 's');
    type_text(&mut model, "second");
    ctrl(&mut model, 's');
    assert_eq!(draft(&model), "first");
    assert!(effort_row(&model).contains("stashed"), "second is held now");
    ctrl(&mut model, 's');
    assert_eq!(draft(&model), "second");
    assert!(
        effort_row(&model).contains("stashed"),
        "first is held again"
    );
}

#[test]
fn a_stashed_draft_survives_a_send_and_its_paste_is_still_expanded_on_restore() {
    let text = distinct(12_000);
    let mut model = shell(true, 120, 44);
    model.paste(&text);
    ctrl(&mut model, 's');
    assert_eq!(draft(&model), "");
    type_text(&mut model, "other");
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, "other");
    assert!(
        effort_row(&model).contains("stashed"),
        "the send left the stash alone"
    );

    ctrl(&mut model, 's');
    assert_eq!(draft(&model), "[Pasted text #1]");
    let Flow::Submit(sent) = press(&mut model, KeyCode::Enter) else {
        panic!("Enter did not send");
    };
    assert_eq!(sent, text);

    // A paste made while another is stashed has its own chip, even with the same number.
    let other = distinct(11_000);
    model.paste(&text);
    ctrl(&mut model, 's');
    model.paste(&other);
    assert_eq!(draft(&model), "[Pasted text #1]");
    ctrl(&mut model, 's');
    assert_eq!(model.composer.expanded_text(), text);
    ctrl(&mut model, 's');
    assert_eq!(model.composer.expanded_text(), other);
}

#[test]
fn ctrl_s_in_the_prompt_search_still_cycles_its_scope_and_stashes_nothing() {
    let mut model = shell(true, 120, 44);
    type_text(&mut model, "x");
    ctrl(&mut model, 'r');
    assert!(model.history_search.is_some());
    ctrl(&mut model, 's');
    assert!(model.history_search.is_some(), "the search stays open");
    press(&mut model, KeyCode::Esc);
    assert_eq!(draft(&model), "x");
    assert!(!effort_row(&model).contains("stashed"));
}
