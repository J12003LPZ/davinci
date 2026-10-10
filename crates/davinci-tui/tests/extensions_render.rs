//! `/plugins`, `/skills` and `/mcp` drawn whole, through `app::compose`: the
//! tab bar and search box stay pinned while a long registry scrolls, the
//! selected row is always on screen, and nothing is wider than the terminal.
//! Set `DUMP_EXTENSIONS=1` and run with `--nocapture` to see the screens.

use davinci_tui::davinci::{
    app,
    model::{
        DiscoverState, ExtensionRow, ExtensionTab, ExtensionView, ExtensionsSheet, Model, Screen,
    },
    theme::{ColorDepth, State, Theme},
    ui,
};

fn registry(count: usize) -> Vec<ExtensionRow> {
    (0..count)
        .map(|n| ExtensionRow {
            key: format!("registry:{n}"),
            title: format!("io.github.owner{n}/server-{n}"),
            status: "featured · MCP Registry · v1.10.1".into(),
            detail: format!(
                "Description of server {n}, long enough that it must be clipped on a narrow \
                 terminal instead of wrapping over several rows and pushing the list around."
            ),
            note: Some(format!(
                "Adds server-{n}: runs npx -y @owner{n}/mcp@1.10.1."
            )),
            ..ExtensionRow::default()
        })
        .collect()
}

fn model(width: u16, height: u16, sheet: ExtensionsSheet) -> Model {
    let mut m = Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        width,
        height,
        false,
    );
    m.width = width;
    m.height = height;
    m.screen = Screen::Extensions;
    m.extension_manager = Some(sheet);
    m
}

fn discover_sheet(count: usize, selected: usize) -> ExtensionsSheet {
    ExtensionsSheet {
        tab: ExtensionTab::Mcp,
        view: ExtensionView::Discover,
        discover: DiscoverState {
            results: registry(count),
            selected,
            ..DiscoverState::default()
        },
        ..ExtensionsSheet::default()
    }
}

fn draw(m: &Model) -> Vec<String> {
    let rows: Vec<String> = app::compose(m, m.height)
        .iter()
        .map(ToString::to_string)
        .collect();
    if std::env::var_os("DUMP_EXTENSIONS").is_some() {
        eprintln!("--- {}x{} ---\n{}", m.width, m.height, rows.join("\n"));
    }
    rows
}

#[test]
fn a_long_registry_keeps_its_header_and_the_selected_row_on_screen() {
    for selected in [0, 1, 40, 109] {
        let m = model(100, 30, discover_sheet(110, selected));
        let rows = draw(&m);
        let text = rows.join("\n");
        assert!(
            text.contains("Installed (0)"),
            "tab bar scrolled away:\n{text}"
        );
        assert!(text.contains("Search the MCP Registry"), "{text}");
        assert!(text.contains("110 results"), "{text}");
        assert!(
            text.contains(&format!("{}/110", selected + 1)),
            "no position:\n{text}"
        );
        assert!(
            text.contains(&format!("server-{selected}")),
            "selected row {selected} off screen:\n{text}"
        );
        assert!(text.contains("❯"), "{text}");
    }
}

#[test]
fn rows_are_never_wider_than_the_terminal() {
    for width in [0u16, 1, 10, 20, 32, 40, 60, 80, 100, 140, 200] {
        for height in [3u16, 8, 24, 40] {
            for view in [ExtensionView::Installed, ExtensionView::Discover] {
                let mut sheet = discover_sheet(30, 5);
                sheet.view = view;
                sheet.mcp = registry(4);
                sheet.discover.query = "a query that is far wider than a narrow screen".into();
                let m = model(width, height, sheet);
                for row in app::compose(&m, height) {
                    assert!(
                        ui::run_width(&row.spans) <= width,
                        "{view:?} {width}x{height}: {row}"
                    );
                }
            }
        }
    }
}

#[test]
fn rows_are_separated_and_the_selected_one_shows_more() {
    let m = model(100, 40, discover_sheet(6, 2));
    let rows = draw(&m);
    let text = rows.join(
        "
",
    );
    // Status follows the name on the same line, not in a far column.
    assert!(
        rows.iter()
            .any(|r| r.contains("owner2/server-2 · featured · MCP Registry · v1.10.1")),
        "{text}"
    );
    // The selected row opens its description and install preview.
    assert!(text.contains("Adds server-2: runs npx"), "{text}");
    // Others clip to one line and hide the preview.
    assert!(!text.contains("Adds server-3"), "{text}");
    let clipped = rows.iter().find(|r| r.contains("Description of server 3"));
    assert!(clipped.is_some_and(|r| r.contains('…')), "{text}");
    // A blank row separates one entry from the next.
    let at = rows.iter().position(|r| r.contains("server-3")).unwrap();
    assert!(rows[at - 1].trim().is_empty(), "{text}");
}

#[test]
fn a_failed_row_keeps_its_error_visible_when_not_selected() {
    let sheet = ExtensionsSheet {
        tab: ExtensionTab::Mcp,
        mcp: vec![
            ExtensionRow {
                key: "a".into(),
                title: "a".into(),
                status: "connected · 4 tools".into(),
                ..ExtensionRow::default()
            },
            ExtensionRow {
                key: "b".into(),
                title: "b".into(),
                status: "failed".into(),
                state: State::Failed,
                note: Some("Cannot start server: executable not found".into()),
                ..ExtensionRow::default()
            },
        ],
        ..ExtensionsSheet::default()
    };
    let text = draw(&model(100, 24, sheet)).join("\n");
    assert!(text.contains("executable not found"), "{text}");
    assert!(text.contains("connected · 4 tools"), "{text}");
}

fn problem_sheet(tab: ExtensionTab) -> ExtensionsSheet {
    let row = |title: &str, status: &str, state: State, note: &str| ExtensionRow {
        key: title.into(),
        title: title.into(),
        status: status.into(),
        state,
        detail: format!("detail of {title}"),
        note: Some(note.into()),
        ..ExtensionRow::default()
    };
    let rows = vec![
        row("fine", "connected · 4 tools", State::Done, "all good"),
        row(
            "broken",
            "error",
            State::Failed,
            "Cannot start server: spawn ENOENT",
        ),
        row("locked", "error", State::Failed, "HTTP 401 Unauthorized"),
        row(
            "stale",
            "enabled · not running",
            State::Attention,
            "Starts with the next session.",
        ),
    ];
    let mut sheet = ExtensionsSheet {
        tab,
        ..ExtensionsSheet::default()
    };
    match tab {
        ExtensionTab::Skills => sheet.skills = rows,
        ExtensionTab::Mcp => sheet.mcp = rows,
        ExtensionTab::Plugins => sheet.plugins = rows,
    }
    sheet
}

#[test]
fn errors_and_missing_auth_are_badged_and_counted_for_mcp_and_plugins() {
    for tab in [ExtensionTab::Mcp, ExtensionTab::Plugins] {
        let text = draw(&model(100, 40, problem_sheet(tab))).join(
            "
",
        );
        assert!(
            text.contains("broken  × error"),
            "{tab:?}:
{text}"
        );
        assert!(
            text.contains("locked  ! needs auth"),
            "{tab:?}:
{text}"
        );
        assert!(
            text.contains("stale  ! needs attention"),
            "{tab:?}:
{text}"
        );
        assert!(
            text.contains("1 error"),
            "{tab:?}:
{text}"
        );
        assert!(
            text.contains("1 needs auth"),
            "{tab:?}:
{text}"
        );
        assert!(
            !text.contains("fine  !"),
            "{tab:?}:
{text}"
        );
    }
}

#[test]
fn the_problem_summary_follows_you_into_discover() {
    let mut sheet = problem_sheet(ExtensionTab::Mcp);
    sheet.view = ExtensionView::Discover;
    let text = draw(&model(100, 30, sheet)).join(
        "
",
    );
    assert!(text.contains("1 error"), "{text}");
}

#[test]
fn skills_never_show_a_problem() {
    let text = draw(&model(100, 40, problem_sheet(ExtensionTab::Skills))).join(
        "
",
    );
    assert!(!text.contains('×') && !text.contains("  !"), "{text}");
    for word in ["× error", "needs auth", "needs attention", "1 error"] {
        assert!(
            !text.contains(word),
            "{word}:
{text}"
        );
    }
}

#[test]
fn an_armed_hook_approval_shows_every_line_it_asks_you_to_trust() {
    let hooks: Vec<String> = (0..8)
        .map(|n| format!("PreToolUse hook {n}: sh -c run-{n}"))
        .collect();
    let sheet = ExtensionsSheet {
        tab: ExtensionTab::Plugins,
        plugins: vec![ExtensionRow {
            key: "p@m".into(),
            title: "p@m".into(),
            status: "enabled".into(),
            state: State::Attention,
            detail: "line one of the detail
line two
line three of the detail"
                .into(),
            note: Some(hooks.join(
                "
",
            )),
            can_approve: true,
            ..ExtensionRow::default()
        }],
        armed: Some(("p@m".into(), "approve")),
        ..ExtensionsSheet::default()
    };
    for width in [40u16, 100] {
        let text = draw(&model(width, 60, sheet.clone())).join(
            "
",
        );
        for n in 0..8 {
            assert!(
                text.contains(&format!("run-{n}")),
                "{width}: hook {n}:
{text}"
            );
        }
        assert!(
            text.contains("line three"),
            "{width}:
{text}"
        );
        assert!(
            text.contains("Any other key"),
            "{width}:
{text}"
        );
    }
}

fn skill_sheet() -> ExtensionsSheet {
    let skill = |title: String, group: &str, status: &str| ExtensionRow {
        key: format!("{group}/{title}"),
        title,
        status: status.into(),
        detail: "does a thing".into(),
        group: group.into(),
        ..ExtensionRow::default()
    };
    let mut rows = vec![
        skill("mine-a".into(), "Your skills", "user"),
        skill("mine-b".into(), "Your skills", "user"),
    ];
    rows.extend((0..293).map(|n| skill(format!("ecc-skill-{n:03}"), "ecc@ecc", "plugin")));
    rows.extend((0..15).map(|n| skill(format!("sp-{n:02}"), "superpowers", "plugin")));
    ExtensionsSheet {
        tab: ExtensionTab::Skills,
        skills: rows,
        ..ExtensionsSheet::default()
    }
}

#[test]
fn skills_are_filed_under_their_source_and_big_plugins_start_folded() {
    let text = draw(&model(100, 60, skill_sheet())).join("\n");
    assert!(
        text.contains("Skills (310)") || text.contains("Installed (310)"),
        "{text}"
    );
    assert!(text.contains("▾ Your skills · 2 skills"), "{text}");
    assert!(text.contains("mine-a"), "{text}");
    assert!(text.contains("▸ ecc@ecc · 293 skills"), "{text}");
    assert!(text.contains("▸ superpowers · 15 skills"), "{text}");
    // A folded group says what is inside, in one clipped line.
    assert!(text.contains("ecc-skill-000, ecc-skill-001"), "{text}");
    // 308 plugin cards would bury the screen; only the two of the open group show.
    assert_eq!(text.matches("does a thing").count(), 2, "{text}");
}

#[test]
fn a_selected_group_heading_stays_on_screen_on_a_short_terminal() {
    let mut sheet = skill_sheet();
    sheet.selected[1] = 3; // Your skills, mine-a, mine-b, then the ecc@ecc heading
    let rows = draw(&model(100, 24, sheet));
    let text = rows.join("\n");
    assert!(
        rows.iter().any(|row| row.starts_with("❯ ▸ ecc@ecc")),
        "{text}"
    );
}

#[test]
fn a_filter_opens_every_group_it_matches_and_counts_them() {
    let mut sheet = skill_sheet();
    sheet.filter = "sp-0".into();
    let text = draw(&model(100, 40, sheet)).join("\n");
    assert!(text.contains("10 of 310"), "{text}");
    assert!(text.contains("▾ superpowers · 10 of 15 skills"), "{text}");
    assert!(!text.contains("ecc@ecc"), "{text}");
    let mut none = skill_sheet();
    none.filter = "zzzz".into();
    let text = draw(&model(100, 40, none)).join("\n");
    assert!(text.contains("Nothing matches \"zzzz\"."), "{text}");
}

#[test]
fn grouped_rows_are_never_wider_than_the_terminal() {
    for width in [0u16, 1, 10, 20, 40, 80, 140] {
        for height in [3u16, 8, 24, 60] {
            let mut filtered = skill_sheet();
            filtered.filter = "e".into();
            filtered.filtering = true;
            for sheet in [skill_sheet(), filtered] {
                for row in app::compose(&model(width, height, sheet), height) {
                    assert!(
                        ui::run_width(&row.spans) <= width,
                        "{width}x{height}: {row}"
                    );
                }
            }
        }
    }
}
