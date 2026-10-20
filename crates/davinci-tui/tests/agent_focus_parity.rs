//! The agent roster and worker focus contract.
//!
//! No screenshot of the reference workflow is in the repository, so this is a
//! semantic contract taken from the design spec (section 3): which cells a
//! roster row carries, what a worker's view shows, and how focus moves. The
//! comparison against a real screenshot is not made and is not claimed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app,
    fixtures::agent_parity,
    model::Model,
    roster::{ViewScope, MAIN_ROW},
    views::agent_focus,
};

fn key(m: &mut Model, code: KeyCode) -> app::Flow {
    app::handle_key(m, KeyEvent::new(code, KeyModifiers::NONE))
}

fn frame(m: &Model) -> Vec<String> {
    app::compose(m, m.height)
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn text(m: &Model) -> String {
    frame(m).join("\n")
}

fn type_text(m: &mut Model, s: &str) {
    for ch in s.chars() {
        key(m, KeyCode::Char(ch));
    }
}

#[test]
fn claude_reference_roster_and_focus_contract() {
    let m = agent_parity().model(120, 40);
    let rows = frame(&m);
    let screen = rows.join("\n");

    // The main row and one row per worker, in a roster at the bottom.
    let main = rows
        .iter()
        .position(|r| r.contains("main"))
        .expect("main row");
    let ci = rows.iter().position(|r| r.contains("ci-watch")).unwrap();
    assert!(ci > main, "workers are listed under main");
    assert!(main > rows.len() / 2, "the roster sits at the bottom");

    // Each worker row: status glyph, name, one-line activity, elapsed, tokens,
    // workspace indicator.
    let row_of = |name: &str| rows.iter().find(|r| r.contains(name)).unwrap().clone();
    let map = row_of("map-auth");
    assert!(map.contains("Read(src/auth/login.rs)"), "{map}");
    assert!(map.contains("1:23"), "{map}");
    assert!(map.contains("23k"), "{map}");
    assert!(map.contains("shared"), "{map}");
    let ci_row = row_of("ci-watch");
    assert!(ci_row.contains("Bash(cargo test)"), "{ci_row}");
    assert!(ci_row.contains("12:05"), "{ci_row}");
    assert!(ci_row.contains("~8k"), "estimates carry a tilde: {ci_row}");
    assert!(ci_row.contains("⎇ davinci/agent/ci"), "{ci_row}");
    let reviewer = row_of("reviewer");
    assert!(reviewer.contains("4:40"), "{reviewer}");
    assert!(
        reviewer.contains("unknown") && !reviewer.contains(" 0 "),
        "no usage is unknown, never zero: {reviewer}"
    );

    // Running and finished workers read differently at a glance.
    assert_ne!(map.chars().nth(2), reviewer.chars().nth(2));
    assert!(!screen.contains("Viewing:"));

    // Focus a worker: its own transcript, its workspace, the way back.
    let mut m = m;
    m.focus_agent("w-ci-watch");
    let focused = text(&m);
    assert!(focused.contains("Viewing: ci-watch"), "{focused}");
    assert!(focused.contains("esc back to main"), "{focused}");
    assert!(focused.contains("started ci-watch"), "{focused}");
    assert!(
        focused.contains("davinci/agent/ci @ 1a2b3c4 · dirty"),
        "{focused}"
    );
    assert!(focused.contains("cache 52000 read"), "{focused}");
    assert!(
        !focused.contains("Search(\"login\")"),
        "no other worker's lines"
    );
    assert!(focused.contains("Message @ci-watch"));
    // The roster stays visible while a worker is in view.
    assert!(focused.contains("map-auth") && focused.contains("reviewer"));
}

#[test]
fn one_shot_workers_render_when_teams_disabled() {
    // A one-shot worker needs nothing but a row: the roster has no setting of
    // its own that depends on agentTeams.
    let mut m = agent_parity().model(100, 30);
    let one = vec![m.roster.workers[0].clone()];
    m.set_workers(one);
    let screen = text(&m);
    assert!(
        screen.contains("map-auth") && screen.contains("main"),
        "{screen}"
    );
    assert!(m.roster.enabled, "on by default");
}

#[test]
fn worker_roster_visible_during_main_turn() {
    let mut m = agent_parity().model(110, 36);
    m.running = true;
    let screen = text(&m);
    for name in ["main", "map-auth", "ci-watch", "reviewer"] {
        assert!(screen.contains(name), "{name} missing:\n{screen}");
    }
}

#[test]
fn worker_focus_preserves_main_transcript_and_draft() {
    let mut m = agent_parity().model(110, 36);
    m.composer.set_text("main draft in progress");
    let transcript = m.transcript.len();
    m.focus_agent("w-map-auth");
    assert_eq!(
        m.composer.editor().get_text(),
        "",
        "the worker has its own draft"
    );
    type_text(&mut m, "check the token");
    m.focus_main();
    assert_eq!(m.composer.editor().get_text(), "main draft in progress");
    assert_eq!(m.transcript.len(), transcript);
    m.focus_agent("w-map-auth");
    assert_eq!(m.composer.editor().get_text(), "check the token");
}

#[test]
fn finished_worker_remains_readable() {
    let mut m = agent_parity().model(110, 36);
    m.focus_agent("w-reviewer");
    let screen = text(&m);
    assert!(screen.contains("Viewing: reviewer"), "{screen}");
    assert!(screen.contains("Read(docs/spec.md)") && screen.contains("finished"));
}

#[test]
fn narrow_terminal_keeps_composer() {
    for (width, height) in [
        (40, 8),
        (40, 12),
        (40, 24),
        (80, 10),
        (80, 24),
        (160, 30),
        (30, 6),
    ] {
        let mut m = agent_parity().model(width, height);
        m.composer.set_text("typing here");
        for focused in [false, true] {
            if focused {
                m.focus_agent("w-ci-watch");
                m.composer.set_text("typing here");
            }
            let rows = frame(&m);
            assert_eq!(rows.len(), usize::from(height), "{width}x{height}");
            assert!(
                rows.iter().any(|r| r.contains("typing here")),
                "{width}x{height} focused={focused} lost the composer:\n{}",
                rows.join("\n")
            );
            for row in &rows {
                assert!(
                    unicode_width::UnicodeWidthStr::width(row.as_str()) <= usize::from(width),
                    "{width}x{height}: {row:?} is too wide"
                );
            }
        }
    }
}

#[test]
fn a_tiny_terminal_collapses_the_roster_to_one_row() {
    let m = agent_parity().model(80, 9);
    let rows = frame(&m);
    let summary = rows
        .iter()
        .filter(|r| r.contains("agents (2 running)"))
        .count();
    assert_eq!(summary, 1, "{}", rows.join("\n"));
    assert!(!rows.iter().any(|r| r.contains("reviewer")));
}

#[test]
fn esc_from_worker_returns_main_without_abort_or_queue_clear() {
    let mut m = agent_parity().model(110, 36);
    m.running = true;
    m.queued = vec!["then commit".into(), "then push".into()];
    m.composer.set_text("main draft");
    m.focus_agent("w-ci-watch");
    type_text(&mut m, "to worker");

    let flow = key(&mut m, KeyCode::Esc);
    assert_eq!(
        flow,
        app::Flow::Continue,
        "Esc in a worker's view is not an interrupt"
    );
    assert_eq!(m.roster.scope, ViewScope::Main);
    assert!(m.running, "the run was not stopped");
    assert_eq!(
        m.queued,
        vec!["then commit".to_string(), "then push".to_string()]
    );
    assert_eq!(m.composer.editor().get_text(), "main draft");
    // The worker's draft is kept for next time.
    m.focus_agent("w-ci-watch");
    assert_eq!(m.composer.editor().get_text(), "to worker");
}

#[test]
fn esc_main_retains_interrupt_behavior() {
    let mut m = agent_parity().model(110, 36);
    m.running = true;
    m.queued = vec!["then commit".into()];
    let flow = key(&mut m, KeyCode::Esc);
    assert_eq!(flow, app::Flow::Interrupt);
    assert!(!m.running);
    assert_eq!(
        m.queued,
        vec!["then commit".to_string()],
        "the queue survives it"
    );
}

#[test]
fn click_main_returns_main() {
    let mut m = agent_parity().model(110, 36);
    m.focus_agent("w-map-auth");
    app::compose_frame(&m, m.height); // draw: the roster publishes its rows
    let main_row = m
        .roster
        .hit
        .borrow()
        .iter()
        .position(|id| id == MAIN_ROW)
        .unwrap();
    assert!(agent_focus::click(&mut m, main_row));
    assert_eq!(m.roster.scope, ViewScope::Main);
}

#[test]
fn click_worker_opens_correct_identity_after_roster_reorder() {
    let mut m = agent_parity().model(110, 36);
    let frame = app::compose_frame(&m, m.height);
    // The click lands on the row drawn for ci-watch...
    let drawn: Vec<String> = m.roster.hit.borrow().clone();
    let index = drawn.iter().position(|id| id == "w-ci-watch").unwrap();
    assert!(frame
        .targets
        .iter()
        .any(|hit| hit.target == app::Target::Roster(index)));
    // ...even though the list was reordered before the click was handled.
    let mut reordered = m.roster.workers.clone();
    reordered.reverse();
    m.set_workers(reordered);
    assert!(agent_focus::click(&mut m, index));
    assert_eq!(m.roster.scope, ViewScope::Worker("w-ci-watch".into()));
}

#[test]
fn click_and_keyboard_navigation_match() {
    let mut by_key = agent_parity().model(110, 36);
    assert_eq!(key(&mut by_key, KeyCode::Down), app::Flow::Continue);
    key(&mut by_key, KeyCode::Down); // main -> map-auth
    key(&mut by_key, KeyCode::Down); // -> ci-watch
    key(&mut by_key, KeyCode::Enter);

    let mut by_click = agent_parity().model(110, 36);
    app::compose_frame(&by_click, by_click.height);
    let at = by_click
        .roster
        .hit
        .borrow()
        .iter()
        .position(|id| id == "w-ci-watch")
        .unwrap();
    agent_focus::click(&mut by_click, at);

    assert_eq!(by_key.roster.scope, by_click.roster.scope);
    assert_eq!(by_key.roster.scope, ViewScope::Worker("w-ci-watch".into()));
    // Up from the top of the roster leaves it, and Esc does too.
    let mut m = agent_parity().model(110, 36);
    key(&mut m, KeyCode::Down);
    assert!(m.roster.focus.is_some());
    key(&mut m, KeyCode::Up);
    assert!(m.roster.focus.is_none());
    key(&mut m, KeyCode::Down);
    assert_eq!(key(&mut m, KeyCode::Esc), app::Flow::Continue);
    assert!(m.roster.focus.is_none());
}

#[test]
fn worker_composer_sends_to_the_worker_and_slash_commands_stay_with_main() {
    let mut m = agent_parity().model(110, 36);
    m.composer.set_text("main draft");
    m.focus_agent("w-map-auth");
    assert!(text(&m).contains("Message @map-auth"));

    type_text(&mut m, "look at the parser");
    assert_eq!(key(&mut m, KeyCode::Enter), app::Flow::Continue);
    let outbox = m.take_worker_outbox();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].agent_id, "w-map-auth");
    assert_eq!(outbox[0].text, "look at the parser");
    assert!(
        text(&m).contains("sending"),
        "a local receipt shows at once"
    );
    assert!(m.queued.is_empty(), "nothing was queued for main");

    // The host answers; the receipt moves, by stable id.
    m.set_worker_receipt(
        "w-map-auth",
        outbox[0].local,
        Some("abcdef0123".into()),
        "queued",
        None,
    );
    assert!(text(&m).contains("queued"));
    m.advance_worker_receipt("w-map-auth", "abcdef0123", "applied", None);
    assert!(text(&m).contains("applied"));

    // A slash command is the main agent's, whichever view is open.
    type_text(&mut m, "/agents");
    assert_eq!(
        key(&mut m, KeyCode::Enter),
        app::Flow::Submit("/agents".into())
    );
    assert!(m.take_worker_outbox().is_empty());
    // And the main draft was not touched by any of it.
    m.focus_main();
    assert_eq!(m.composer.editor().get_text(), "main draft");
}

#[test]
fn hostile_agent_text_cannot_inject_terminal_controls_or_fake_rows() {
    let mut m = agent_parity().model(110, 36);
    let mut rows = m.roster.workers.clone();
    rows[0].name = "x\u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[2J\r\nmain".into();
    rows[0].activity = "go\u{1b}[31m\u{8}\r\nreviewer approved".into();
    m.set_workers(rows);
    m.push_worker_lines(
        "w-map-auth",
        vec![davinci_tui::davinci::roster::WorkerLine {
            sequence: 99,
            kind: davinci_tui::davinci::roster::WorkerLineKind::Call,
            text: "Bash(\u{1b}[2Jrm -rf)\nSEND".into(),
        }],
        0,
    );
    for focused in [false, true] {
        if focused {
            m.focus_agent("w-map-auth");
        }
        for row in frame(&m) {
            assert!(!row.chars().any(char::is_control), "{row:?}");
        }
    }
}

#[test]
fn terminal_resize_click_targets_not_stale() {
    let mut m = agent_parity().model(120, 40);
    let wide = app::compose_frame(&m, m.height);
    let wide_ids: Vec<String> = m.roster.hit.borrow().clone();
    m.width = 40;
    m.height = 12;
    let narrow = app::compose_frame(&m, m.height);
    let narrow_ids: Vec<String> = m.roster.hit.borrow().clone();
    let rows_wide: Vec<_> = wide
        .targets
        .iter()
        .filter(|hit| matches!(hit.target, app::Target::Roster(_)))
        .map(|hit| hit.rows.clone())
        .collect();
    let rows_narrow: Vec<_> = narrow
        .targets
        .iter()
        .filter(|hit| matches!(hit.target, app::Target::Roster(_)))
        .map(|hit| hit.rows.clone())
        .collect();
    assert_ne!(rows_wide, rows_narrow, "targets follow the new layout");
    assert!(narrow_ids.len() <= wide_ids.len());
    for hit in &narrow.targets {
        assert!(hit.rows.end <= 12, "{hit:?} lies outside the 12-row window");
    }
}

#[test]
fn the_setting_off_draws_no_roster_and_takes_no_keys() {
    let mut m = agent_parity().model(110, 36);
    m.roster.enabled = false;
    let screen = text(&m);
    assert!(!screen.contains("ci-watch"), "{screen}");
    // Down in an empty composer is the footer's again, and Esc on an idle
    // composer is the old no-op.
    assert_eq!(key(&mut m, KeyCode::Down), app::Flow::Continue);
    assert!(m.roster.focus.is_none());
}
