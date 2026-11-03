//! P13: pickers and panels. The reference fixes the frame; the content is
//! DaVinci's own (model names, settings, commands, context figures).
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app,
    model::{CatalogRow, ContextSection, ContextUsageView, Credential, Model, Screen},
    theme::{ColorDepth, Theme},
    views::{cogitator, context_usage, sheet},
};

fn shell(reference: bool) -> Model {
    let theme = if reference {
        Theme::reference(ColorDepth::TrueColor, false)
    } else {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    };
    let mut model = Model::new(theme, 120, 46, true);
    model.reference = reference;
    model.thinking_level = "medium".into();
    model
}

fn catalog_row(id: &str, name: &str) -> CatalogRow {
    CatalogRow {
        reasoning_levels: vec!["medium".into(), "high".into()],
        reasoning_index: 0,
        name: name.into(),
        detail: String::new(),
        window: "128k".into(),
        thinking: "budget".into(),
        price: String::new(),
        credential: Credential::Local,
        note: String::new(),
        ring: false,
        provider: "local".into(),
        id: id.into(),
    }
}

fn model_sheet(model: &mut Model) {
    model.catalog = vec![catalog_row("demo", "Demo")];
    model.catalog_index = 0;
    model.catalog_session_only = false;
    model.model_name = "demo".into();
    model.model_title = "Demo".into();
    model.active_provider = "local".into();
    model.screen = Screen::Models;
}

fn text(line: &ratatui::text::Line<'static>) -> String {
    line.to_string()
}

#[test]
fn the_reference_model_rows_put_descriptions_at_column_31() {
    let mut model = shell(true);
    model_sheet(&mut model);
    let rows = cogitator::screen(&model, 12);
    let row = rows
        .iter()
        .map(text)
        .find(|row| row.contains("demo"))
        .expect("a model row");
    assert!(row.starts_with("   ❯ 1. demo"), "{row:?}");
    let at_31: String = row.chars().skip(31).take(4).collect();
    assert_eq!(at_31, "loca", "description column: {row:?}");
    assert!(model.reference);
}

#[test]
fn the_effort_row_names_medium_as_the_default_and_high_as_not() {
    let mut model = shell(true);
    model_sheet(&mut model);
    let rows: Vec<String> = cogitator::screen(&model, 12).iter().map(text).collect();
    assert!(
        rows.iter()
            .any(|row| row.contains("◐ Medium effort (default)")),
        "{rows:?}"
    );
    model.catalog[0].reasoning_index = 1;
    let rows: Vec<String> = cogitator::screen(&model, 12).iter().map(text).collect();
    assert!(
        rows.iter().any(|row| row.contains("● High effort")),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("(default)")),
        "{rows:?}"
    );
    // The off shell keeps its five-stop legend and its own glyph.
    let mut off = shell(false);
    model_sheet(&mut off);
    let rows: Vec<String> = cogitator::screen(&off, 12).iter().map(text).collect();
    assert!(
        rows.iter().any(|row| row.contains("/ to search")),
        "{rows:?}"
    );
}

#[test]
fn the_help_tab_bar_fills_the_active_tab() {
    let mut model = shell(true);
    model.screen = Screen::Keys;
    model.help_tab = Some(0);
    let spans = sheet::help_tabs(&model).expect("help tabs");
    let active = spans
        .iter()
        .find(|span| span.content.contains("General"))
        .expect("the General tab");
    assert_eq!(active.content.as_ref(), " General ");
    assert!(active.style.bg.is_some(), "the active tab is filled");

    let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app::handle_key(&mut model, tab), app::Flow::Continue);
    assert_eq!(model.help_tab, Some(1));
    let spans = sheet::help_tabs(&model).unwrap();
    assert!(spans
        .iter()
        .any(|span| span.content.as_ref() == " Commands "));
    assert_eq!(app::handle_key(&mut model, tab), app::Flow::Continue);
    assert_eq!(model.help_tab, Some(2));
    let back = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(app::handle_key(&mut model, back), app::Flow::Continue);
    assert_eq!(model.help_tab, Some(1));
}

fn section(items: usize) -> ContextSection {
    ContextSection {
        title: "MCP tools".into(),
        command: Some("/mcp".into()),
        items: (0..items)
            .map(|index| (format!("mcp__tool_{index}"), 100))
            .collect(),
    }
}

fn usage(all: bool) -> ContextUsageView {
    ContextUsageView {
        model: "demo".into(),
        window: 100_000,
        categories: Vec::new(),
        free: 100_000,
        buffer: 0,
        sections: vec![section(8)],
        all,
    }
}

#[test]
fn context_all_lists_every_section_item() {
    let theme = Theme::reference(ColorDepth::TrueColor, false);
    let folded: Vec<String> = context_usage::lines(&theme, &usage(false), 120)
        .iter()
        .map(text)
        .collect();
    assert_eq!(
        folded
            .iter()
            .filter(|row| row.contains("mcp__tool_"))
            .count(),
        5,
        "{folded:?}"
    );
    assert!(
        folded.iter().any(|row| row.contains("3 more")),
        "{folded:?}"
    );
    assert!(
        folded
            .iter()
            .any(|row| row.contains("/context all to expand")),
        "{folded:?}"
    );
    let all: Vec<String> = context_usage::lines(&theme, &usage(true), 120)
        .iter()
        .map(text)
        .collect();
    assert_eq!(
        all.iter().filter(|row| row.contains("mcp__tool_")).count(),
        8,
        "{all:?}"
    );
    assert!(!all.iter().any(|row| row.contains("3 more")), "{all:?}");
    assert!(!all.iter().any(|row| row.contains("to expand")), "{all:?}");
}

#[test]
fn the_reference_grid_is_twenty_columns_of_the_reference_glyphs() {
    let theme = Theme::reference(ColorDepth::TrueColor, false);
    let mut view = usage(false);
    view.sections.clear();
    let rows: Vec<String> = context_usage::lines(&theme, &view, 120)
        .iter()
        .map(text)
        .collect();
    let grid: Vec<&String> = rows.iter().filter(|row| row.contains("⛶")).collect();
    assert_eq!(grid.len(), 10, "{rows:?}");
    assert_eq!(grid[0].matches('⛶').count(), 20, "{:?}", grid[0]);
    assert!(!rows.iter().any(|row| row.contains('□')), "old glyph kept");
}

fn permission_row(
    kind: &str,
    label: &str,
    detail: &str,
    source: &str,
) -> davinci_tui::davinci::model::PermissionRow {
    davinci_tui::davinci::model::PermissionRow {
        label: label.into(),
        detail: detail.into(),
        current: false,
        kind: kind.into(),
        key: label.into(),
        source: source.into(),
    }
}

#[test]
fn the_permissions_tabs_filter_and_never_apply_from_a_read_only_tab() {
    let mut model = shell(true);
    model.screen = Screen::Permissions;
    model.permission_rows = vec![
        permission_row("rule", "bash(git *)", "allow · user", "user"),
        permission_row("rule", "bash(rm *)", "deny · user", "user"),
        permission_row("rule", "write(src/*)", "allow · project", "project"),
    ];
    model.permission_tab = 0;
    let tabs = sheet::permissions_tabs(&model).expect("permission tabs");
    let active = tabs
        .iter()
        .find(|span| span.content.contains("Permissions"))
        .unwrap();
    assert!(active.style.bg.is_some(), "the active tab is filled");
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(
        app::handle_key(&mut model, enter),
        app::Flow::Choose(_)
    ));
    assert_eq!(model.permission_tab, 0);

    let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(app::handle_key(&mut model, right), app::Flow::Continue);
    assert_eq!(model.permission_tab, 1);
    assert_eq!(app::handle_key(&mut model, right), app::Flow::Continue);
    assert_eq!(model.permission_tab, 2);
    let rows: Vec<String> = davinci_tui::davinci::views::permissions::lines(&model)
        .iter()
        .map(text)
        .collect();
    assert!(
        rows.iter().any(|row| row.contains("bash(git *)")),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("bash(rm *)")),
        "{rows:?}"
    );
    // Enter on a filtered tab is read-only: no Choice reaches the host.
    assert_eq!(app::handle_key(&mut model, enter), app::Flow::Continue);

    model.permission_query = "rm".into();
    let rows: Vec<String> = davinci_tui::davinci::views::permissions::lines(&model)
        .iter()
        .map(text)
        .collect();
    assert!(
        rows.iter().any(|row| row.contains("bash(rm *)")),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("bash(git *)")),
        "{rows:?}"
    );
    assert_eq!(app::handle_key(&mut model, enter), app::Flow::Continue);

    // Off the reference the sheet keeps its old list and no search box.
    let mut off = shell(false);
    off.screen = Screen::Permissions;
    off.permission_rows = model.permission_rows.clone();
    let rows: Vec<String> = davinci_tui::davinci::views::permissions::lines(&off)
        .iter()
        .map(text)
        .collect();
    assert!(!rows.iter().any(|row| row.contains("Search…")), "{rows:?}");
}

#[test]
fn the_viewer_names_the_model_and_says_it_is_the_detailed_transcript() {
    let mut model = shell(true);
    model.model_title = "Demo".into();
    model.transcript = vec![davinci_tui::davinci::model::Entry::User("hello".into())].into();
    let viewer = davinci_tui::davinci::views::viewer::TranscriptViewer::open(&model);
    let rows: Vec<String> = davinci_tui::davinci::views::viewer::render(&model, &viewer, 20)
        .iter()
        .map(text)
        .collect();
    assert!(rows[0].contains("Transcript"), "{}", rows[0]);
    assert!(rows[0].contains("Demo"), "{}", rows[0]);
    assert!(
        rows.last().unwrap().contains("Showing detailed transcript"),
        "{:?}",
        rows.last()
    );
}

fn agent(
    id: &str,
    name: &str,
    state: davinci_tui::davinci::theme::State,
) -> davinci_tui::davinci::model::AgentRow {
    let mut row = davinci_tui::davinci::model::AgentRow::new(id, name, state);
    row.status = match state {
        davinci_tui::davinci::theme::State::Active => "working",
        davinci_tui::davinci::theme::State::Done => "stopped",
        _ => "waiting",
    }
    .into();
    row
}

fn agents_model() -> Model {
    let mut model = shell(true);
    model.screen = Screen::Agents;
    let mut sheet = davinci_tui::davinci::model::AgentsSheet::default();
    let mut needs = agent(
        "n1",
        "needs-one",
        davinci_tui::davinci::theme::State::Attention,
    );
    needs.waiting_on = Some("approve".into());
    sheet.agents = vec![
        needs,
        agent(
            "w1",
            "working-one",
            davinci_tui::davinci::theme::State::Active,
        ),
        agent(
            "c1",
            "completed-one",
            davinci_tui::davinci::theme::State::Done,
        ),
        agent("p1", "pinned-one", davinci_tui::davinci::theme::State::Done),
    ];
    sheet.pinned = vec!["p1".into()];
    sheet.selected_index = 2;
    model.agents = Some(sheet);
    model
}

#[test]
fn the_agents_dashboard_groups_rows_and_counts_them() {
    let model = agents_model();
    let rows: Vec<String> = davinci_tui::davinci::views::agents::lines(&model)
        .iter()
        .map(text)
        .collect();
    for heading in ["Pinned 1", "Needs input 1", "Working 1", "Completed 1"] {
        assert!(
            rows.iter().any(|row| row.contains(heading)),
            "{heading}: {rows:?}"
        );
    }
    let chrome = davinci_tui::davinci::views::agents::chrome(&model);
    let header: String = chrome
        .header_right
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert_eq!(
        header, "1 awaiting input · 1 working · 2 completed",
        "{header}"
    );
}

#[test]
fn the_agents_dashboard_pins_finds_renames_and_deletes_in_two_steps() {
    let mut model = agents_model();
    let ctrl = |ch: char| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
    assert_eq!(app::handle_key(&mut model, ctrl('t')), app::Flow::Continue);
    assert_eq!(
        model.agents.as_ref().unwrap().pinned,
        ["p1".to_string(), "c1".to_string()]
    );
    // Find filters the list; Esc clears it before closing.
    assert_eq!(app::handle_key(&mut model, ctrl('f')), app::Flow::Continue);
    for ch in "working".chars() {
        app::handle_key(
            &mut model,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    let rows: Vec<String> = davinci_tui::davinci::views::agents::lines(&model)
        .iter()
        .map(text)
        .collect();
    assert!(
        rows.iter().any(|row| row.contains("Find: working")),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("needs-one")),
        "{rows:?}"
    );
    app::handle_key(&mut model, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(model.agents.as_ref().unwrap().query.is_empty());
    assert_eq!(
        model.screen,
        Screen::Agents,
        "the first Esc cleared find, not the sheet"
    );
    // Rename the selected row.
    assert_eq!(app::handle_key(&mut model, ctrl('r')), app::Flow::Continue);
    for ch in "-renamed".chars() {
        app::handle_key(
            &mut model,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    app::handle_key(
        &mut model,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    let names: Vec<&str> = model
        .agents
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert!(names.contains(&"completed-one-renamed"), "{names:?}");
    // Two Ctrl+X presses delete the completed row; the working row is never removed.
    let before = model.agents.as_ref().unwrap().agents.len();
    app::handle_key(&mut model, ctrl('x'));
    app::handle_key(&mut model, ctrl('x'));
    assert_eq!(model.agents.as_ref().unwrap().agents.len(), before - 1);
    model.agents.as_mut().unwrap().selected_index = 1;
    app::handle_key(&mut model, ctrl('x'));
    app::handle_key(&mut model, ctrl('x'));
    assert_eq!(model.agents.as_ref().unwrap().agents.len(), before - 1);
}

#[test]
fn screen_reader_mode_is_plain_lines_in_reading_order() {
    let mut model = shell(true);
    model.screen_reader = true;
    model.model_title = "Demo".into();
    model.provider_title = "local".into();
    model.cwd = "~\\work".into();
    let rows: Vec<String> = app::compose(&model, 12).iter().map(text).collect();
    assert_eq!(rows[0], "[Screen Reader Mode: on via flag]");
    assert!(
        rows.iter().any(|row| row.starts_with("DaVinci v")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|row| row == "Demo · local"), "{rows:?}");
    assert!(rows.iter().any(|row| row == "$ "), "{rows:?}");
    assert!(
        rows.iter().any(|row| row.starts_with("effort: ")),
        "{rows:?}"
    );
    assert!(
        !rows
            .iter()
            .any(|row| row.contains('▐') || row.contains('─')),
        "box art leaked into screen-reader mode: {rows:?}"
    );
}

#[test]
fn reduce_motion_is_a_settings_row() {
    let list = davinci_tui::interactive_settings_list(&davinci_tui::InteractiveSettingsConfig {
        reduce_motion: true,
        ..Default::default()
    });
    let row = list
        .items
        .iter()
        .find(|item| item.id == "reduce-motion")
        .expect("a reduce-motion row");
    assert_eq!(row.label, "Reduce motion");
    assert_eq!(row.current_value, "true");
}

#[test]
fn every_screen_renders_on_the_shared_frame() {
    let screens = [
        Screen::Agent,
        Screen::Plan,
        Screen::Grafo,
        Screen::Memoria,
        Screen::Mensura,
        Screen::Models,
        Screen::Settings,
        Screen::Thinking,
        Screen::Login,
        Screen::Keys,
        Screen::Resume,
        Screen::Tree,
        Screen::Compact,
        Screen::Export,
        Screen::GraphRun,
        Screen::GraphResume,
        Screen::Vectors,
        Screen::Governor,
        Screen::Securitas,
        Screen::Trust,
        Screen::Officina,
        Screen::Recovery,
        Screen::Diff,
        Screen::Mcp,
        Screen::Permissions,
        Screen::Workflows,
        Screen::TaskBoard,
        Screen::Agents,
        Screen::ContextInspector,
        Screen::Extensions,
    ];
    for screen in screens {
        assert!(!sheet::title(screen).is_empty(), "{screen:?} has no title");
        let mut model = shell(true);
        model.screen = screen;
        let rows = app::compose(&model, 20);
        assert_eq!(rows.len(), 20, "{screen:?} did not compose a full frame");
        let _ = sheet::chrome(&model);
    }
}
