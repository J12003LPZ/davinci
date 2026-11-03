//! P10: transcript, Markdown, tools and diffs. The expected lines are the synthetic
//! fixture text visible in the pinned build's frames (top_tools2, diff_view, error_view3);
//! nothing of the reference's own prose is stored here.
use davinci_tui::davinci::{
    model::{Entry, Hunk, HunkKind, Model},
    theme::{ColorDepth, State, Theme},
    views::{markdown, transcript},
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

fn shown(model: &Model) -> Vec<String> {
    transcript::lines(model, &model.transcript, 116)
        .iter()
        .map(|line| line.to_string())
        .collect()
}

fn markdown_text(reference: bool, source: &str) -> Vec<String> {
    let theme = if reference {
        Theme::reference(ColorDepth::TrueColor, false)
    } else {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    };
    markdown::lines(&theme, source, 116)
        .iter()
        .map(|line| line.to_string())
        .collect()
}

#[test]
fn the_reference_markdown_table_is_box_drawn_with_numbers_right_aligned() {
    let source = "| Name | Value | Notes |\n|------|-------|-------|\n| alpha | 1 | first |\n| beta | 2 | second |";
    let rows = markdown_text(true, source);
    assert_eq!(rows.len(), 7, "{rows:?}");
    assert_eq!(rows[0], "┌───────┬───────┬────────┐");
    assert_eq!(rows[1], "│ Name  │ Value │ Notes  │");
    assert_eq!(rows[2], "├───────┼───────┼────────┤");
    assert_eq!(rows[3], "│ alpha │     1 │ first  │");
    assert_eq!(rows[4], "├───────┼───────┼────────┤");
    assert_eq!(rows[5], "│ beta  │     2 │ second │");
    assert_eq!(rows[6], "└───────┴───────┴────────┘");
    // Off the reference the shell's own plain table stays.
    let off = markdown_text(false, source);
    assert!(off[0].starts_with("Name"), "{off:?}");
    assert!(!off.iter().any(|row| row.contains('┌')));
}

#[test]
fn the_reference_quote_and_rule_glyphs() {
    let rows = markdown_text(true, "> A block quote for visual testing.\n\n---");
    assert_eq!(rows[0], "▎ A block quote for visual testing.");
    assert_eq!(rows.last().map(String::as_str), Some("---"));
    let off = markdown_text(false, "> quote\n\n---");
    assert!(off[0].starts_with("│ "), "{off:?}");
    assert!(!off.iter().any(|row| row == "---"));
}

#[test]
fn a_failed_bash_call_is_one_red_row_with_its_error_beneath() {
    let mut model = shell(true);
    model.transcript = vec![
        Entry::User("Show a failing command.".into()),
        Entry::Gap,
        Entry::Tool {
            state: State::Failed,
            instrument: "Bash".into(),
            target: "definitely-not-a-real-command".into(),
            duration: None,
            summary: None,
            output: vec!["Error: bash: definitely-not-a-real-command: command not found".into()],
        },
    ]
    .into();
    let rows = shown(&model);
    let header = rows
        .iter()
        .position(|row| row.contains("Bash(definitely-not-a-real-command)"))
        .unwrap_or_else(|| panic!("no tool row in {rows:?}"));
    assert!(rows[header].starts_with('●'), "{:?}", rows[header]);
    let error = rows
        .iter()
        .find(|row| row.contains("command not found"))
        .unwrap_or_else(|| panic!("no error row in {rows:?}"));
    assert!(error.starts_with("  ⎿ \u{a0}"), "{error:?}");
    assert!(error.contains("Error: bash:"), "{error:?}");
    let user = rows
        .iter()
        .find(|row| row.contains("Show a failing command"))
        .unwrap();
    assert!(user.starts_with("❯ "), "{user:?}");
}

#[test]
fn an_update_diff_draws_line_numbers_and_the_change_signs() {
    let mut model = shell(true);
    model.transcript = vec![
        Entry::User("Now edit it to add a fourth line.".into()),
        Entry::Gap,
        Entry::Delta {
            path: r"C:\work\fixture.txt".into(),
            adds: 1,
            dels: 1,
            hunks: vec![
                Hunk::at(HunkKind::Context, 1, "line one"),
                Hunk::at(HunkKind::Context, 2, "line two"),
                Hunk::at(HunkKind::Del, 3, "line three"),
                Hunk::at(HunkKind::Add, 3, "line three changed"),
                Hunk::at(HunkKind::Context, 4, "line four"),
                Hunk::at(HunkKind::Context, 5, "line five"),
            ],
        },
    ]
    .into();
    let rows = shown(&model);
    let joined = rows.join("\n");
    assert!(joined.contains("line three changed"), "{joined}");
    let removed = rows
        .iter()
        .find(|row| row.contains("-line three"))
        .expect("a removed row");
    let added = rows
        .iter()
        .find(|row| row.contains("+line three changed"))
        .expect("an added row");
    assert!(removed.contains('3'), "{removed:?}");
    assert!(added.contains('3'), "{added:?}");
    assert!(
        !rows
            .iter()
            .any(|row| row.contains("line three changed") && !row.contains('+')),
        "a changed row lost its sign: {rows:?}"
    );
}

#[test]
fn a_shell_mode_row_keeps_its_own_prompt_and_error() {
    let mut model = shell(true);
    model.transcript = vec![Entry::Shell {
        command: "definitely-not-a-real-command".into(),
        output: vec!["bash: definitely-not-a-real-command: command not found".into()],
        failed: true,
    }]
    .into();
    let rows = shown(&model);
    assert!(rows[0].starts_with("! "), "{:?}", rows[0]);
    assert!(rows.iter().any(|row| row.contains("command not found")));
}
