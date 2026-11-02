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
