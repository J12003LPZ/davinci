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
    // Find moved the selection onto its one result; pick the completed row again.
    assert_eq!(model.agents_index, 1);
    model.agents_index = 2;
    model.agents.as_mut().unwrap().selected_index = 2;
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

#[test]
fn the_agents_find_filter_governs_selection_navigation_and_actions() {
    use davinci_tui::davinci::model::Choice;
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let find = |model: &mut Model, text: &str| {
        app::handle_key(
            model,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        );
        for ch in text.chars() {
            app::handle_key(model, key(KeyCode::Char(ch)));
        }
        app::handle_key(model, key(KeyCode::Enter));
    };

    // The selection moves to the visible result, and Enter and x act on it.
    let mut model = agents_model();
    model.agents_index = 0;
    find(&mut model, "working");
    assert_eq!(model.agents_index, 1, "selection left on a hidden agent");
    assert_eq!(model.agents.as_ref().unwrap().selected_index, 1);
    assert!(matches!(
        app::handle_key(&mut model, key(KeyCode::Enter)),
        app::Flow::Choose(Choice::AgentAction {
            action: "inspect",
            index: 1
        })
    ));
    assert!(matches!(
        app::handle_key(&mut model, key(KeyCode::Char('x'))),
        app::Flow::Choose(Choice::AgentAction {
            action: "stop",
            index: 1
        })
    ));

    // Navigation stays inside the visible rows.
    let mut model = agents_model();
    model.agents_index = 0;
    find(&mut model, "stopped");
    assert_eq!(model.agents_index, 2);
    app::handle_key(&mut model, key(KeyCode::Down));
    assert_eq!(model.agents_index, 3);
    app::handle_key(&mut model, key(KeyCode::Down));
    assert_eq!(model.agents_index, 2, "wraps over the two visible rows");

    // With no match, no key acts on any agent.
    let mut model = agents_model();
    find(&mut model, "zzz");
    for code in [
        KeyCode::Enter,
        KeyCode::Char('s'),
        KeyCode::Char('x'),
        KeyCode::Char('r'),
        KeyCode::Char('d'),
    ] {
        let flow = app::handle_key(&mut model, key(code));
        assert!(
            !matches!(flow, app::Flow::Choose(_)),
            "{code:?} acted on a hidden agent"
        );
    }
}

#[test]
fn screen_reader_mode_draws_the_open_question_and_its_choices_at_any_height() {
    use davinci_tui::davinci::model::{Ask, Overlay, PickerItem};
    let mut model = shell(true);
    model.screen_reader = true;
    model.ask = Ask {
        title: "Permission".into(),
        question: "Run git status?".into(),
        items: vec![
            PickerItem::new("Yes", ""),
            PickerItem::new("Yes, always", "this project"),
            PickerItem::new("No", ""),
        ],
        ..Ask::default()
    };
    model.overlay = Some(Overlay::Ask);
    model.ask_index = 1;
    for height in [4u16, 5, 6, 8, 24] {
        let rows: Vec<String> = app::compose(&model, height).iter().map(text).collect();
        assert_eq!(rows.len(), height as usize);
        for needle in ["1. Yes", "> 2. Yes, always - this project", "3. No"] {
            assert!(
                rows.iter().any(|row| row.contains(needle)),
                "height {height} lost {needle:?}: {rows:?}"
            );
        }
    }
    let rows: Vec<String> = app::compose(&model, 24).iter().map(text).collect();
    assert!(rows.iter().any(|row| row == "Run git status?"), "{rows:?}");
}

#[test]
fn screen_reader_mode_windows_a_long_list_around_its_first_row_and_marks_it() {
    use davinci_tui::davinci::model::{Overlay, SessionItem};
    let mut model = shell(true);
    model.screen_reader = true;
    model.sessions = (0..40)
        .map(|n| SessionItem::new(&format!("session {n:02}"), "1d"))
        .collect();
    model.overlay = Some(Overlay::Sessions);
    for height in [4u16, 8, 24] {
        let rows: Vec<String> = app::compose(&model, height).iter().map(text).collect();
        assert_eq!(rows.len(), height as usize);
        assert!(
            rows.iter()
                .any(|row| row.trim_start().starts_with("> ") && row.contains("session 00")),
            "height {height}: the selected first row must show with a plain marker: {rows:?}"
        );
        assert!(
            !rows.iter().any(|row| row.contains("session 39")),
            "height {height} showed the tail while the first row is selected: {rows:?}"
        );
    }
    // Moving the selection moves the window with it.
    for _ in 0..30 {
        model.move_selection(1);
    }
    let rows: Vec<String> = app::compose(&model, 8).iter().map(text).collect();
    assert!(
        rows.iter()
            .any(|row| row.trim_start().starts_with("> ") && row.contains("session 30")),
        "{rows:?}"
    );
}

fn approval_with_preview() -> Model {
    use davinci_tui::davinci::model::{Ask, AskKind, Hunk, HunkKind, Overlay, PickerItem};
    let mut model = shell(true);
    model.screen_reader = true;
    let mut preview: Vec<Hunk> = (0..30)
        .map(|n| Hunk::new(HunkKind::Context, &format!("context line {n}")))
        .collect();
    preview[10] = Hunk::new(HunkKind::Del, "old value");
    preview[11] = Hunk::new(HunkKind::Add, "new value");
    model.ask = Ask {
        title: "Edit file".into(),
        key: "/permissions".into(),
        question: "Do you want to make this edit?".into(),
        subject: "src/lib.rs".into(),
        kind: AskKind::File,
        preview,
        items: vec![
            PickerItem::new("Yes", ""),
            PickerItem::new("Yes, always", ""),
            PickerItem::new("No", ""),
        ],
        ..Ask::default()
    };
    model.overlay = Some(Overlay::Ask);
    model.ask_index = 1;
    model
}

#[test]
fn screen_reader_approval_keeps_what_it_approves_and_prints_the_preview() {
    let mut model = approval_with_preview();
    for height in [4u16, 8, 24] {
        let rows: Vec<String> = app::compose(&model, height).iter().map(text).collect();
        assert_eq!(rows.len(), height as usize);
        for needle in ["src/lib.rs", "> 2. Yes, always"] {
            assert!(
                rows.iter().any(|row| row.contains(needle)),
                "height {height} lost {needle:?}: {rows:?}"
            );
        }
        assert!(
            rows.iter().any(|row| row.contains("rows hidden")),
            "height {height} must say that rows are hidden: {rows:?}"
        );
        if height >= 8 {
            assert!(
                rows.iter()
                    .any(|row| row.contains("Do you want to make this edit?")),
                "height {height}: {rows:?}"
            );
            assert!(
                rows.iter().any(|row| row.contains("context line 0")),
                "height {height} must print the preview: {rows:?}"
            );
        }
    }
    // The preview is reachable: paging moves it while the decision stays put.
    model.overlay_offset = Some(10);
    let rows: Vec<String> = app::compose(&model, 8).iter().map(text).collect();
    assert!(rows.iter().any(|row| row == "- old value"), "{rows:?}");
    assert!(rows.iter().any(|row| row.contains("src/lib.rs")));
    assert!(rows.iter().any(|row| row.contains("> 2. Yes, always")));
}

#[test]
fn deleting_the_last_selected_agent_keeps_enter_and_x_on_the_highlighted_row() {
    use davinci_tui::davinci::model::{AgentsSheet, Choice};
    use davinci_tui::davinci::theme::State;
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let ctrl = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
    let mut model = shell(true);
    model.screen = Screen::Agents;
    model.agents = Some(AgentsSheet {
        agents: vec![
            agent("a0", "zero", State::Done),
            agent("a1", "one", State::Done),
            agent("a2", "two", State::Done),
        ],
        selected_index: 2,
        ..AgentsSheet::default()
    });
    model.agents_index = 2;

    // Ctrl+X arms, the second Ctrl+X deletes agent 2 of 3.
    app::handle_key(&mut model, ctrl('x'));
    app::handle_key(&mut model, ctrl('x'));
    let sheet = model.agents.as_ref().unwrap();
    assert_eq!(sheet.agents.len(), 2);
    assert_eq!(sheet.selected_index, 1, "the view highlights agent 1");
    assert_eq!(model.agents_index, 1, "the action target follows it");
    assert_eq!(model.selected_agent(), Some(1));
    assert!(matches!(
        app::handle_key(&mut model, key(KeyCode::Enter)),
        app::Flow::Choose(Choice::AgentAction {
            action: "inspect",
            index: 1
        })
    ));
    assert!(matches!(
        app::handle_key(&mut model, key(KeyCode::Char('x'))),
        app::Flow::Choose(Choice::AgentAction {
            action: "stop",
            index: 1
        })
    ));
}

#[test]
fn screen_reader_approval_keeps_every_line_of_a_multi_line_command() {
    use davinci_tui::davinci::model::{Ask, AskKind, Hunk, HunkKind, Overlay, PickerItem};
    let mut model = shell(true);
    model.screen_reader = true;
    model.ask = Ask {
        title: "Bash".into(),
        key: "/permissions".into(),
        question: "Run this command?".into(),
        subject: "echo safe # comment\r\nrm important.txt".into(),
        kind: AskKind::File,
        preview: (0..10)
            .map(|n| Hunk::new(HunkKind::Context, &format!("detail {n}")))
            .collect(),
        items: vec![
            PickerItem::new("Yes", ""),
            PickerItem::new("Yes, always", ""),
            PickerItem::new("No", ""),
        ],
        ..Ask::default()
    };
    model.overlay = Some(Overlay::Ask);
    model.ask_index = 1;
    let rows_at = |model: &Model, height: u16| -> Vec<String> {
        app::compose(model, height).iter().map(text).collect()
    };

    let rows = rows_at(&model, 24);
    assert!(
        rows.iter().any(|row| row == "echo safe # comment"),
        "{rows:?}"
    );
    assert!(rows.iter().any(|row| row == "rm important.txt"), "{rows:?}");

    for height in [4u16, 8] {
        let rows = rows_at(&model, height);
        assert!(
            rows.iter().any(|row| row == "echo safe # comment"),
            "height {height}: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("rows hidden")),
            "height {height}: {rows:?}"
        );
        // Paging reaches the hidden subject line, not only the preview.
        model.overlay_offset = Some(0);
        let paged = rows_at(&model, height);
        assert!(
            paged.iter().any(|row| row == "rm important.txt"),
            "height {height} paged: {paged:?}"
        );
        assert!(
            paged.iter().any(|row| row == "echo safe # comment"),
            "height {height} paged: {paged:?}"
        );
        assert!(
            paged.iter().any(|row| row.contains("> 2. Yes, always")),
            "height {height} paged: {paged:?}"
        );
        model.overlay_offset = None;
    }
    // A bare `\r` is a break too, and the question and note keep theirs.
    model.ask.subject = "a\rb".into();
    model.ask.question = "first\nsecond".into();
    model.ask.note = "n1\nn2".into();
    let rows = rows_at(&model, 24);
    for row in ["a", "b", "first", "second", "n1", "n2"] {
        assert!(rows.iter().any(|r| r == row), "{row}: {rows:?}");
    }
}

fn screen_reader_ask(subject: &str, note: &str, choices: usize, height: u16) -> Model {
    use davinci_tui::davinci::model::{Ask, AskKind, Overlay, PickerItem};
    let mut model = shell(true);
    model.screen_reader = true;
    model.height = height;
    model.ask = Ask {
        title: "Bash".into(),
        key: "/permissions".into(),
        question: "Run this command?".into(),
        subject: subject.into(),
        note: note.into(),
        kind: AskKind::File,
        items: (0..choices)
            .map(|n| PickerItem::new(&format!("Choice {n}"), ""))
            .collect(),
        ..Ask::default()
    };
    model.overlay = Some(Overlay::Ask);
    model.ask_index = 1;
    model
}

fn page_down(model: &mut Model) {
    app::handle_key(model, KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
}

#[test]
fn screen_reader_approval_pages_to_every_note_line_and_keeps_the_choice_at_height_one() {
    let note = (0..6)
        .map(|n| format!("note {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut model = screen_reader_ask("cmd", &note, 2, 4);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..12 {
        let rows: Vec<String> = app::compose(&model, 4).iter().map(text).collect();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().any(|row| row == "cmd"), "{rows:?}");
        assert!(rows.iter().any(|row| row == "> 2. Choice 1"), "{rows:?}");
        for n in 0..6 {
            if rows.contains(&format!("note {n}")) {
                seen.insert(n);
            }
        }
        page_down(&mut model);
    }
    assert_eq!(seen.len(), 6, "every note line is reachable: {seen:?}");

    // One row: Enter acts on the selected choice, so it is the row shown.
    let model = screen_reader_ask("cmd", &note, 2, 1);
    let rows: Vec<String> = app::compose(&model, 1).iter().map(text).collect();
    assert_eq!(rows, vec!["> 2. Choice 1".to_string()]);
}

#[test]
fn screen_reader_approval_pages_to_the_tail_of_a_bare_cr_subject() {
    let subject = (0..40)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\r");
    let mut model = screen_reader_ask(&subject, "", 3, 8);
    let mut reached = false;
    for _ in 0..60 {
        let rows: Vec<String> = app::compose(&model, 8).iter().map(text).collect();
        if rows.iter().any(|row| row == "line 39") {
            reached = true;
            break;
        }
        page_down(&mut model);
    }
    assert!(reached, "the subject's last line is unreachable by paging");
    for _ in 0..80 {
        page_down(&mut model);
    }
    // 39 subject lines past the first are pageable: the offset stops inside them.
    assert!(
        model.overlay_offset.unwrap() <= 38,
        "{:?}",
        model.overlay_offset
    );
    let rows: Vec<String> = app::compose(&model, 8).iter().map(text).collect();
    assert!(rows.iter().any(|row| row == "line 39"), "{rows:?}");
    assert!(rows.iter().any(|row| row == "line 0"), "{rows:?}");
}

#[test]
fn deleting_an_agent_under_a_filter_moves_the_selection_to_a_visible_row() {
    use davinci_tui::davinci::model::{AgentsSheet, Choice};
    use davinci_tui::davinci::theme::State;
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let ctrl = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
    let mut model = shell(true);
    model.screen = Screen::Agents;
    model.agents = Some(AgentsSheet {
        agents: vec![
            agent("a0", "foo-first", State::Done),
            agent("a1", "bar", State::Done),
            agent("a2", "foo-last", State::Done),
        ],
        selected_index: 2,
        query: "foo".into(),
        ..AgentsSheet::default()
    });
    model.agents_index = 2;
    app::handle_key(&mut model, ctrl('x'));
    app::handle_key(&mut model, ctrl('x'));
    let sheet = model.agents.as_ref().unwrap();
    assert_eq!(sheet.agents.len(), 2);
    assert_eq!(sheet.selected_index, 0, "the hidden `bar` row is skipped");
    assert_eq!(model.agents_index, 0);
    assert_eq!(model.selected_agent(), Some(0));
    assert!(matches!(
        app::handle_key(&mut model, key(KeyCode::Enter)),
        app::Flow::Choose(Choice::AgentAction {
            action: "inspect",
            index: 0
        })
    ));
}
