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
            text.contains("Installed 0"),
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
fn a_selected_card_opens_and_the_others_stay_two_lines() {
    let m = model(100, 40, discover_sheet(6, 2));
    let rows = draw(&m);
    let text = rows.join("\n");
    // The selected row shows its whole description and its install preview.
    assert!(text.contains("pushing the list around."), "{text}");
    assert!(text.contains("Adds server-2: runs npx"), "{text}");
    // An unselected row shows neither: its description is clipped.
    assert!(!text.contains("Adds server-3"), "{text}");
    let clipped = rows.iter().find(|r| r.contains("Description of server 3"));
    assert!(clipped.is_some_and(|r| r.contains('…')), "{text}");
}

#[test]
fn status_chips_drop_whole_segments_instead_of_cutting_one() {
    let m = model(60, 30, discover_sheet(3, 0));
    let text = draw(&m).join("\n");
    assert!(text.contains("featured"), "{text}");
    assert!(!text.contains("v…"), "a version cut to `v…`:\n{text}");
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
