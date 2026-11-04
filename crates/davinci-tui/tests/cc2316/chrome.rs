//! P07 and P07b: the header, the pinned rows and the first-run screens.
//! Positions come from measuring the pinned build's captures; every text here
//! is synthetic, so nothing of the reference is stored in this repository.
use davinci_tui::davinci::{
    app,
    model::{Model, Working},
    theme::{ColorDepth, Theme},
    views::{startup, theme_picker},
};
use unicode_width::UnicodeWidthStr;

/// The sizes of the `layout_sizes` goldens, plus the boot one.
const SIZES: [(u16, u16); 6] = [
    (120, 44),
    (100, 30),
    (80, 24),
    (60, 20),
    (160, 50),
    (120, 18),
];

fn reference(width: u16, height: u16) -> Model {
    let mut model = Model::new(
        Theme::reference(ColorDepth::TrueColor, false),
        width,
        height,
        true,
    );
    model.reference = true;
    model.model_name = "demo-model".into();
    model.model_title = "Demo-Model".into();
    model.provider_title = "Fixture".into();
    model.startup.cwd = "/srv/project".into();
    model.thinking_level = "medium".into();
    model
}

fn rows(model: &Model) -> Vec<String> {
    app::compose(model, model.height)
        .iter()
        .map(|line| line.to_string())
        .collect()
}

fn chars(row: &str) -> Vec<char> {
    row.chars().collect()
}

#[test]
fn the_header_is_three_rows_with_the_text_in_column_eleven() {
    for (width, height) in SIZES {
        let frame = rows(&reference(width, height));
        assert_eq!(frame.len(), usize::from(height));
        assert!(frame[0].trim().is_empty(), "{width}x{height}: row 0");
        let name = chars(&frame[1]);
        let text: String = name[11..].iter().collect();
        assert!(
            text.starts_with(&format!("DaVinci v{}", env!("CARGO_PKG_VERSION"))),
            "{width}x{height}: {:?}",
            frame[1]
        );
        assert!(chars(&frame[2])[11..]
            .iter()
            .collect::<String>()
            .starts_with("Demo Model · Fixture"));
        assert!(chars(&frame[3])[11..]
            .iter()
            .collect::<String>()
            .starts_with("/srv/project"));
        assert!(frame[4].trim().is_empty(), "{width}x{height}: row 4 is air");
        // Column 10 and everything left of the text is the mark's box or air.
        for row in &frame[1..=3] {
            assert_eq!(chars(row)[10], ' ', "{width}x{height}: {row:?}");
        }
    }
}

#[test]
fn the_mark_stays_in_its_box_and_does_not_move() {
    let mut model = reference(120, 44);
    let still = rows(&model);
    for row in &still[1..=3] {
        let mark: Vec<char> = chars(row)[..10].to_vec();
        assert!(mark.iter().any(|c| *c != ' '), "no mark in {row:?}");
    }
    // Rows 4 and 5 hold no part of a taller mark.
    assert!(still[4].trim().is_empty() && still[5].trim().is_empty());
    for tick in 0..80 {
        model.tick = tick;
        model.working = (tick % 2 == 0).then(Working::default);
        let frame = rows(&model);
        for row in 1..=3 {
            assert_eq!(
                chars(&frame[row])[..10],
                chars(&still[row])[..10],
                "the mark changed at tick {tick}"
            );
        }
    }
}

#[test]
fn the_pinned_rows_sit_where_the_goldens_put_them() {
    for (width, height) in SIZES {
        let frame = rows(&reference(width, height));
        let h = usize::from(height);
        let w = usize::from(width);
        let rule = "─".repeat(w);
        assert_eq!(frame[h - 4], rule, "{width}x{height}: top rule");
        assert_eq!(frame[h - 2], rule, "{width}x{height}: bottom rule");
        assert!(
            frame[h - 3].starts_with("❯\u{a0}"),
            "{width}x{height}: {:?}",
            frame[h - 3]
        );
        let label = "◐ medium · /effort";
        let effort = chars(&frame[h - 5]);
        let at = w - 2 - label.chars().count();
        assert_eq!(
            effort[at..at + 18].iter().collect::<String>(),
            label,
            "{width}x{height}"
        );
        assert!(effort[..at].iter().all(|c| *c == ' '));
        assert!(frame[h - 5].trim_end().ends_with("/effort"));
        assert!(
            frame[h - 1].starts_with("  "),
            "{width}x{height}: footer indent"
        );
        assert!(
            !frame[h - 1].starts_with("   "),
            "{width}x{height}: footer indent"
        );
    }
}

#[test]
fn an_armed_exit_replaces_the_footer_and_nothing_else() {
    let mut model = reference(120, 44);
    let before = rows(&model);
    model.exit_armed = true;
    let after = rows(&model);
    assert_eq!(after[43].trim_end(), "  Press Ctrl-C again to exit");
    assert_eq!(&before[..43], &after[..43]);
}

#[test]
fn without_an_effort_the_row_is_empty() {
    let mut model = reference(120, 44);
    model.thinking_level = "off".into();
    assert!(rows(&model)[39].trim().is_empty());
}

#[test]
fn the_home_folder_reads_as_a_tilde() {
    use startup::home_relative as home;
    assert_eq!(home(r"C:\Users\ana\proj", Some(r"C:\Users\ana")), r"~\proj");
    assert_eq!(home("/home/ana/proj", Some("/home/ana")), "~/proj");
    assert_eq!(home("/home/ana", Some("/home/ana")), "~");
    assert_eq!(home("/home/ana/", Some("/home/ana/")), "~");
    assert_eq!(home(r"c:\users\ANA\proj", Some(r"C:\Users\ana")), r"~\proj");
    // A multibyte home makes the byte split a non-char boundary.
    assert_eq!(
        home(r"C:\Users\José\proj", Some(r"C:\Users\José")),
        r"~\proj"
    );
    assert_eq!(home("/home/José/proj", Some("/home/José")), "~/proj");
    assert_eq!(
        home("/home/Joséphine/proj", Some("/home/José")),
        "/home/Joséphine/proj"
    );
    // A sibling that merely starts with the home path is not under it.
    assert_eq!(
        home("/home/anaconda/proj", Some("/home/ana")),
        "/home/anaconda/proj"
    );
    assert_eq!(home("/srv/proj", Some("/home/ana")), "/srv/proj");
    assert_eq!(home("/srv/proj", None), "/srv/proj");
    assert_eq!(home("/srv/proj", Some("")), "/srv/proj");
}

#[test]
fn a_draft_of_any_size_leaves_the_header_and_the_footer_in_place() {
    let drafts = [
        String::new(),
        "short".to_string(),
        (1..=60)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "漢字漢字🙂é".repeat(80),
    ];
    for (width, height) in SIZES {
        let base = rows(&reference(width, height));
        let h = usize::from(height);
        for draft in &drafts {
            let mut model = reference(width, height);
            model.composer.set_text(draft);
            let frame = rows(&model);
            assert_eq!(frame.len(), h);
            assert_eq!(&frame[..4], &base[..4], "{width}x{height}: header moved");
            assert_eq!(frame[h - 1], base[h - 1], "{width}x{height}: footer moved");
            assert!(frame.iter().all(|row| row.width() <= usize::from(width)));
            assert!(
                frame[h - 2].starts_with('─'),
                "{width}x{height}: bottom rule"
            );
        }
    }
}

#[test]
fn wide_glyphs_on_the_clip_edge_are_whole_or_absent() {
    for width in 20..=27u16 {
        let mut model = reference(width, 12);
        model.composer.set_text("漢".repeat(40));
        for row in rows(&model) {
            assert!(row.width() <= usize::from(width), "{width}: {row:?}");
            assert!(!row.contains('\u{fffd}'));
        }
    }
}

#[test]
fn windows_smaller_than_the_goldens_do_not_panic() {
    for (width, height) in [
        (1, 1),
        (2, 2),
        (5, 2),
        (10, 3),
        (20, 4),
        (21, 5),
        (30, 5),
        (40, 8),
    ] {
        let mut model = reference(width, height);
        model.composer.set_text("a draft\nwith two lines");
        let frame = rows(&model);
        assert_eq!(frame.len(), usize::from(height), "{width}x{height}");
        assert!(frame.iter().all(|row| row.width() <= usize::from(width)));
        model.exit_armed = true;
        let _ = rows(&model);
    }
}

#[test]
fn resizing_with_a_draft_and_with_the_picker_open_keeps_both() {
    let mut model = reference(120, 44);
    model.composer.set_text("a draft in progress");
    for (width, height) in [
        (80, 24),
        (60, 20),
        (20, 4),
        (160, 50),
        (120, 18),
        (30, 6),
        (120, 44),
    ] {
        model.width = width;
        model.height = height;
        let frame = rows(&model);
        assert_eq!(frame.len(), usize::from(height));
        assert_eq!(model.composer.editor().get_text(), "a draft in progress");
    }
    assert!(theme_picker::open(&mut model, "dark"));
    for (width, height) in [(80, 24), (20, 4), (60, 20), (160, 50), (1, 1), (120, 44)] {
        model.width = width;
        model.height = height;
        let frame = rows(&model);
        assert_eq!(frame.len(), usize::from(height));
        assert!(frame.iter().all(|row| row.width() <= usize::from(width)));
    }
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

#[test]
fn the_rules_file_names_the_version_the_binary_reports() {
    let rules = include_str!("../../../../scripts/ui/cc2316_rules.json");
    assert!(
        rules.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
        "update the identity rule for the header's version in scripts/ui/cc2316_rules.json"
    );
}

#[test]
fn with_the_setting_off_the_masthead_is_the_painter_and_its_hint() {
    let mut model = reference(120, 44);
    model.reference = false;
    let frame = rows(&model);
    assert!(frame
        .iter()
        .any(|row| row.contains("Switch models anytime")));
    assert!(!frame[39].contains("/effort"));
    assert!(frame.len() == 44 && startup::PET_ROWS == 5);
}

// ---- P07b: the first-run steps ----------------------------------------------

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::Flow,
    model::{Choice, OnboardStep},
    views::onboarding,
};

fn press(model: &mut Model, code: KeyCode) -> Flow {
    app::handle_key(model, KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl_key(model: &mut Model, ch: char) -> Flow {
    app::handle_key(
        model,
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
    )
}

fn welcome(width: u16, height: u16) -> Model {
    let mut model = reference(width, height);
    model.composer.set_text("a draft in progress");
    assert!(onboarding::start(&mut model, "dark"));
    model
}

#[test]
fn the_steps_only_open_for_the_reference() {
    let mut model = reference(120, 44);
    model.reference = false;
    assert!(!onboarding::start(&mut model, "dark"));
    assert!(model.onboarding.is_none() && model.theme_picker.is_none());
}

#[test]
fn the_welcome_walks_theme_then_security_then_hands_over_to_the_question() {
    let mut model = welcome(120, 44);
    let theme = rows(&model).join("\n");
    assert!(
        theme.contains("/theme"),
        "the theme step says where to change it later"
    );
    assert!(theme.contains("Dark mode (ANSI colors only)"));
    assert!(theme.contains("Light mode (ANSI colors only)"));
    assert!(!theme.contains("Vox"), "the list is the shared themes");
    press(&mut model, KeyCode::Down);
    assert_eq!(model.first_run_theme, None, "moving chooses nothing");
    assert_eq!(press(&mut model, KeyCode::Enter), Flow::Continue);
    assert_eq!(model.onboarding, Some(OnboardStep::Security));
    assert_eq!(model.first_run_theme.as_deref(), Some("light"));
    let notes = rows(&model).join("\n");
    assert!(notes.contains("1. ") && notes.contains("2. "));
    assert!(notes.contains("prompt injection") && notes.contains("Press Enter to continue"));
    assert_eq!(
        press(&mut model, KeyCode::Enter),
        Flow::Choose(Choice::OnboardingDone)
    );
    assert!(model.onboarding.is_none() && model.theme_picker.is_none());
    assert_eq!(
        model.first_run_theme.as_deref(),
        Some("light"),
        "held for the answer"
    );
}

#[test]
fn escape_leaves_the_steps_and_ctrl_c_is_the_shells_interrupt() {
    let mut model = welcome(120, 44);
    let before = model.theme;
    press(&mut model, KeyCode::Down);
    press(&mut model, KeyCode::Enter);
    assert_eq!(ctrl_key(&mut model, 'c'), Flow::Interrupt);
    assert_eq!(
        model.onboarding,
        Some(OnboardStep::Security),
        "ctrl+c leaves the step as it was"
    );
    assert_eq!(press(&mut model, KeyCode::Esc), Flow::Continue);
    assert!(model.onboarding.is_none() && model.theme_picker.is_none());
    assert_eq!(model.first_run_theme, None);
    assert_eq!(model.theme, before);
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

#[test]
fn nothing_in_the_steps_trusts_submits_or_grants() {
    // Any key at either step: nothing but a move comes back, never a submit, a
    // trust choice or a permission change.
    let keys = [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Tab,
        KeyCode::Char('y'),
        KeyCode::Char('1'),
        KeyCode::Right,
        KeyCode::BackTab,
    ];
    for step in [OnboardStep::Theme, OnboardStep::Security] {
        let mut model = welcome(120, 44);
        model.onboarding = Some(step);
        let mode = model.permission_mode.clone();
        for code in keys {
            assert_eq!(press(&mut model, code), Flow::Continue, "{step:?} {code:?}");
        }
        assert_eq!(model.permission_mode, mode);
        assert_eq!(model.composer.editor().get_text(), "a draft in progress");
        assert!(model.overlay.is_none());
    }
}

#[test]
fn a_paste_during_a_step_reaches_nothing() {
    for step in [OnboardStep::Theme, OnboardStep::Security] {
        let mut model = welcome(120, 44);
        model.onboarding = Some(step);
        model.paste("rm -rf /\nPress Enter");
        assert_eq!(model.composer.editor().get_text(), "a draft in progress");
        assert_eq!(model.onboarding, Some(step));
    }
}

#[test]
fn every_step_survives_any_resize_and_a_short_screen_keeps_the_highlighted_theme() {
    for step in [OnboardStep::Theme, OnboardStep::Security] {
        let mut model = welcome(120, 44);
        model.onboarding = Some(step);
        for (width, height) in [
            (160, 50),
            (80, 24),
            (60, 20),
            (20, 4),
            (1, 1),
            (120, 3),
            (120, 44),
        ] {
            model.width = width;
            model.height = height;
            let frame = rows(&model);
            assert_eq!(
                frame.len(),
                usize::from(height),
                "{step:?} {width}x{height}"
            );
            assert!(frame.iter().all(|row| row.width() <= usize::from(width)));
        }
    }
    for index in 0..onboarding::LIST {
        let mut model = welcome(120, 12);
        model.theme_picker.as_mut().unwrap().index = index;
        let frame = rows(&model);
        assert!(
            frame.iter().any(|row| row.contains('❯')),
            "index {index}: {frame:?}"
        );
    }
}

#[test]
fn the_welcome_band_and_the_steps_sit_on_the_pinned_rows() {
    let frame = rows(&welcome(120, 44));
    assert!(frame[0].starts_with("Welcome to DaVinci v"));
    assert!(frame[1].starts_with('·') && frame[15].contains('·'));
    assert!(frame[22].contains("Auto (match terminal)"));
    assert!(frame[23].starts_with(" ❯ "));
}
