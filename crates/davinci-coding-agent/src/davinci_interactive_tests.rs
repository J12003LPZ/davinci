
#[test]
fn security_tui_maps_cancelled_failed_and_admission_without_success() {
    use davinci_tui::davinci::{
        model::Model,
        theme::{ColorDepth, Theme},
        views::securitas,
    };
    fn drawn(sheet: davinci_tui::davinci::model::SecurityScan) -> String {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
        model.security = Some(sheet);
        securitas::lines(&model)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
    for status in ["cancelled", "failed", "interrupted"] {
        let sheet = super::security_sheet(&serde_json::json!({
            "schemaVersion": 2,
            "scanId": "11111111-1111-1111-1111-111111111111",
            "status": status,
            "coverageComplete": false,
            "findings": [],
            "coverage": {"eligibleFiles": 1, "reviewedPaths": [], "skipped": []}
        }));
        assert_eq!(sheet.state, status);
        assert_eq!(sheet.id, "11111111-1111-1111-1111-111111111111");
        let text = drawn(sheet);
        assert!(text.contains(&format!("Status: {status}")), "{text}");
        assert!(text.contains("coverage incomplete"), "{text}");
        for claim in [
            "complete for captured scope",
            "report sealed",
            "never left",
            "coverage complete",
        ] {
            assert!(!text.contains(claim), "{status}: {text}");
        }
    }
    let missing = super::security_sheet(&serde_json::json!({"schemaVersion": 2}));
    assert_eq!(missing.state, "unknown");
    assert!(!drawn(missing).contains("Status: completed"));
    let mut empty = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
    empty.security = None;
    let text = securitas::lines(&empty)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("No security scan available"));
    assert!(!text.contains("Status: completed"));
    let view = crate::native_extensions::security_scan::report::project_legacy_v1(
            br#"{"schemaVersion":1,"validated":true,"findings":[{"severity":"critical","title":"old"}]}"#,
        )
        .unwrap();
    let sheet = super::security_sheet(&view);
    assert_eq!(sheet.state, "legacy-readonly");
    assert_eq!(sheet.fraction, 0.0);
    let text = drawn(sheet);
    assert!(text.contains("Status: legacy-readonly"), "{text}");
    assert!(text.contains("not v2 confirmation"), "{text}");
    assert!(!text.contains("Status: completed"), "{text}");
    assert!(!text.contains("report sealed"), "{text}");
}

#[test]
fn security_sheet_preserves_grouped_occurrence_assessments() {
    let report = serde_json::json!({"schemaVersion":2,"findings":[{
            "claim":{"title":"Shared control","locations":[{"path":"control.rs","startLine":1}]},
            "assessment":{"classification":"confirmed","severity":"high","reason":"First reason"},
            "occurrences":[
                {"claim":{"entrypoint":"First caller"},"assessment":{"classification":"confirmed","severity":"high","reason":"First reason"}},
                {"claim":{"entrypoint":"Second caller","prerequisites":["Second prerequisite"],"locations":[{"path":"second.rs","startLine":2,"endLine":3}]},"assessment":{"classification":"likely","severity":"critical","reason":"Second reason","proofGaps":["Second unknown condition"]}}]}]});
    let sheet = super::security_sheet(&report);
    assert_eq!(sheet.findings.len(), 1);
    assert!(sheet.findings[0].message.contains("2 occurrences"));
    assert!(sheet.findings[0].evidence.contains("First caller"));
    assert!(sheet.findings[0].evidence.contains("likely / critical"));
    assert!(sheet.findings[0].evidence.contains("Second caller"));
    assert!(sheet.findings[0]
        .evidence
        .contains("Second unknown condition"));
    assert!(sheet.findings[0].evidence.contains("Second prerequisite"));
    assert!(sheet.findings[0].evidence.contains("second.rs:2-3"));
}
use super::*;
use serde_json::json;

#[test]
fn a_home_path_is_said_with_the_home_variable() {
    let home = davinci_session::home_dir().expect("a home directory");
    let label = home_label(&home.join(".pi").join("agent").join("auth.json"));
    if cfg!(windows) {
        assert!(label.starts_with("%USERPROFILE%"), "{label}");
    } else {
        assert!(label.starts_with("~/"), "{label}");
    }
    assert!(label.ends_with("auth.json"), "{label}");
    let elsewhere = std::path::Path::new("/srv/pi/auth.json");
    assert_eq!(home_label(elsewhere), elsewhere.display().to_string());
}

#[test]
fn a_missing_models_file_has_no_refreshed_label() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(refreshed_label(&dir.path().join("models.json")), "");
    std::fs::write(dir.path().join("models.json"), "{}").unwrap();
    assert_eq!(refreshed_label(&dir.path().join("models.json")), "just now");
    assert_eq!(age_label(7_200), "2h ago");
}

#[test]
fn thinking_facts_come_from_the_reported_usage() {
    assert_eq!(thinking_facts(&[]), (String::new(), 0.0));
    let entry = |usage: serde_json::Value| {
        let mut entry = davinci_session::SessionEntry::message("assistant", json!([]));
        entry.message = Some(json!({"role": "assistant", "usage": usage}));
        entry
    };
    let entries = vec![
        entry(json!({"output": 1000, "reasoning": 400})),
        entry(json!({"output": 1000})),
        entry(json!({"output": 2000, "reasoning": 1200})),
    ];
    let (last, share) = thinking_facts(&entries);
    assert_eq!(last, "1.2k tokens");
    assert!((share - 0.4).abs() < 1e-9, "{share}");
}

#[test]
fn the_session_disk_figure_sums_the_files_and_leaves_the_cap_unknown() {
    assert_eq!(sessions_disk(&[]), None);
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    let b = dir.path().join("b.jsonl");
    std::fs::write(&a, "12345").unwrap();
    std::fs::write(&b, "12").unwrap();
    assert_eq!(
        sessions_disk(&[a, b, dir.path().join("gone")]),
        Some((7, 0))
    );
}

#[test]
fn the_keys_sheet_counts_its_bindings_and_surfaces() {
    let mut m = model();
    open_keys_sheet(&mut m);
    assert_eq!(m.screen, Screen::Keys);
    assert_eq!(
        m.facts.keys_count,
        m.keymap.iter().map(|group| group.rows.len()).sum::<usize>()
    );
    assert!(!m
        .keymap
        .iter()
        .flat_map(|group| &group.rows)
        .any(|(_, label)| label.contains("thinking cycle") || label.contains("cycle thinking")));
    assert_eq!(m.facts.keys_surfaces, m.keymap.len());
    assert!(m.facts.keys_surfaces >= 3);
}

#[test]
fn the_login_sheet_names_the_auth_file() {
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
    std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
    let mut m = model();
    open_login_sheet(&crate::args::Args::default(), &mut m);
    match previous {
        Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
        None => std::env::remove_var("PI_CODING_AGENT_DIR"),
    }
    assert_eq!(m.screen, Screen::Login);
    assert!(
        m.facts.auth_path.ends_with("auth.json"),
        "{}",
        m.facts.auth_path
    );
    assert_eq!(m.facts.auth_mode.is_empty(), !cfg!(unix));
}

#[test]
fn detached_login_does_not_report_pending_oauth_as_signed_in() {
    let outcome = detached_login_message("anthropic", true);
    assert_eq!(
        outcome,
        Err("authorization required to sign in to anthropic".to_string())
    );
}

#[test]
fn detached_openai_without_key_reports_api_key_requirement() {
    assert_eq!(
        detached_login_message("openai", true),
        Err("API key required for openai. Run /login openai <api-key>.".to_string())
    );
}

#[test]
fn detached_login_reports_completed_login_as_signed_in() {
    assert_eq!(
        detached_login_message("anthropic", false),
        Ok("signed in to anthropic".to_string())
    );
}

fn assistant(text: &str) -> davinci_ai::ChatMessage {
    davinci_ai::ChatMessage {
        role: "assistant".into(),
        content: vec![davinci_ai::MessageContent::Text { text: text.into() }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        extra: Default::default(),
    }
}

#[test]
fn a_second_interrupt_while_one_is_pending_asks_for_the_way_out() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut m = model();
    let abort = Arc::new(AtomicBool::new(false));
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    // The first esc requests the abort and is not a force-quit.
    assert!(!mid_turn_key(&mut m, esc, &abort));
    assert!(abort.load(Ordering::Relaxed));
    // The second, with the flag already up, is: the worker is not
    // answering, and the only way left restores the terminal first.
    assert!(mid_turn_key(&mut m, esc, &abort));
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(mid_turn_key(&mut m, ctrl_c, &abort));
}

#[test]
fn an_interrupted_turn_opens_the_recovery_sheet_with_what_it_ran() {
    // The `6c` sheet is populated from the turn's own log; this pins the
    // log's shape so `end_tool` keeps feeding it.
    let mut m = model();
    let mut turn = Turn::default();
    turn.start_tool(&mut m, "call-1", "read", &json!({"path": "src/lib.rs"}));
    turn.end_tool(
        &mut m,
        "call-1",
        "read",
        &serde_json::Value::String("line".into()),
        false,
        None,
    );
    assert_eq!(turn.log.len(), 1);
    assert_eq!(turn.log[0].0, State::Read);
    assert!(turn.log[0].1.contains("read src/lib.rs"));
}

#[test]
fn the_diff_command_is_findable_in_the_palette() {
    let agent = davinci_agent::Agent::new("test");
    let items = corpus(&agent, &[], &[]);
    assert!(items
        .iter()
        .any(|item| item.name == "/diff" && item.kind == "command"));
}

#[test]
fn switching_models_resolves_unsupported_thinking_before_optional_confirmation() {
    use davinci_protocol::ThinkingLevel::*;
    assert_eq!(
        supported_thinking_choice(Max, &[Off, Low, Medium, High]),
        High
    );
    assert_eq!(
        supported_thinking_choice(Low, &[Off, Low, Medium, High]),
        Low
    );
    assert_eq!(supported_thinking_choice(High, &[Off]), Off);
    assert_eq!(supported_thinking_choice(Off, &[Low, Medium, High]), Low);
}

#[test]
fn theme_setting_persists_and_recolors_the_current_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = model();
    m.composer = "keep this draft".into();
    let dark = m.theme;
    let mut row = SettingRow {
        key: "theme".into(),
        value: "dark".into(),
        values: vec!["dark".into(), "light".into()],
        ..SettingRow::default()
    };
    persist_setting_row(&mut row, |assignment| {
        assert_eq!(assignment, "theme=light");
        let stored = crate::settings::Settings {
            theme: Some("light".into()),
            ..Default::default()
        };
        crate::settings::save_settings(dir.path(), &stored)
    })
    .unwrap();
    let stored = crate::settings::load_settings(dir.path());
    apply_theme_setting(&mut m, &stored);
    assert_ne!(m.theme, dark);
    assert_eq!(m.composer, "keep this draft");
    let mut restarted = model();
    apply_theme_setting(&mut restarted, &stored);
    assert_eq!(restarted.theme, m.theme);
    assert_eq!(row.value, "light");
}

#[test]
fn failed_setting_persistence_does_not_change_the_displayed_row() {
    let mut row = SettingRow {
        label: "Auto compact".into(),
        value: "on".into(),
        values: vec!["on".into(), "off".into()],
        project: true,
        key: "auto-compact".into(),
        ..SettingRow::default()
    };
    let result = persist_setting_row(&mut row, |_| Err("disk full".into()));
    assert_eq!(result.unwrap_err(), "disk full");
    assert_eq!(row.value, "on");
    assert!(row.project);
}

#[test]
fn typesafe_key_clipboard_paste_stays_in_secret_input() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use davinci_tui::davinci::app::{self, Flow};
    let mut model = model();
    open_typesafe_key_input(&mut model);
    assert!(paste_secret_clipboard(
        &mut model,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        || Some("test_KEY".into()),
    ));
    assert_eq!(model.composer.to_string(), "");
    let Flow::SecretInputSubmitted(value) = app::handle_key(
        &mut model,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ) else {
        panic!("credential was not submitted")
    };
    assert_eq!(value.into_inner(), "test_KEY");
    assert!(!paste_secret_clipboard(
        &mut model,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        || panic!("clipboard must not be read without secret overlay"),
    ));
}

#[test]
fn opening_typesafe_key_replacement_uses_the_masked_secret_overlay() {
    let mut model = model();

    open_typesafe_key_input(&mut model);

    assert_eq!(model.overlay, Some(Overlay::SecretInput));
    assert!(model.secret_input.is_some());
    assert_eq!(model.composer.to_string(), "");
}

#[test]
fn typesafe_key_replacement_respects_environment_and_project_overrides() {
    let enabled = SettingRow {
        key: "decision-intelligence".into(),
        value: "on".into(),
        ..SettingRow::default()
    };
    assert_eq!(typesafe_key_replacement_blocker(&[enabled], false), None);

    let project_disabled = SettingRow {
        key: "decision-intelligence".into(),
        value: "off".into(),
        project: true,
        ..SettingRow::default()
    };
    assert!(typesafe_key_replacement_blocker(&[project_disabled], false)
        .unwrap()
        .contains("project settings"));
    assert!(typesafe_key_replacement_blocker(&[], true)
        .unwrap()
        .contains("TYPESAFE_API_KEY"));
}

#[test]
fn section_notices_are_only_attached_to_open_sheets() {
    let mut sheet = model();
    sheet.screen = Screen::Settings;
    attach_section_notice(&mut sheet, "could not save");
    assert_eq!(sheet.section_notice.as_deref(), Some("could not save"));

    let mut agent = model();
    agent.screen = Screen::Agent;
    attach_section_notice(&mut agent, "conversation warning");
    assert!(agent.section_notice.is_none());
}

#[test]
fn opening_an_ask_overlay_resets_only_overlay_reading_state() {
    let mut m = model();
    m.section_offset = Some(5);
    m.overlay_offset = Some(8);
    m.ask_index = 3;
    open_ask_overlay(&mut m);
    assert_eq!(m.section_offset, Some(5));
    assert!(m.overlay_offset.is_none());
    assert_eq!(m.ask_index, 0);
    assert_eq!(m.overlay, Some(Overlay::Ask));
}

#[test]
fn opening_a_sheet_clears_section_reading_state() {
    let mut m = model();
    m.section_offset = Some(9);
    m.overlay_offset = Some(7);
    m.section_notice = Some("stale notice".into());
    open_sheet(&mut m, Screen::Settings);
    assert!(m.section_offset.is_none());
    assert!(m.overlay_offset.is_none());
    assert!(m.section_notice.is_none());
}

fn model() -> Model {
    Model::new(
        davinci_tui::davinci::theme::Theme::da_vinci(
            davinci_tui::davinci::theme::ColorDepth::TrueColor,
            false,
        ),
        100,
        40,
        true,
    )
}

#[test]
fn shift_tab_cycle_commits_policy_prompt_and_status_without_submitting_the_draft() {
    let mut agent = Agent::new("base prompt");
    let mut m = model();
    change_permission_mode(&mut agent, &mut m, PermissionMode::Ask);
    m.composer.set_text("unfinished request");
    for expected in [
        PermissionMode::Edits,
        PermissionMode::ReadOnly,
        PermissionMode::Auto,
        PermissionMode::AlwaysApprove,
        PermissionMode::Ask,
    ] {
        let flow = davinci_tui::davinci::app::handle_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::BackTab,
                crossterm::event::KeyModifiers::SHIFT,
            ),
        );
        assert_eq!(flow, davinci_tui::davinci::app::Flow::CyclePermissionMode);
        cycle_permission_mode(&mut agent, &mut m);
        assert_eq!(agent.permission_mode(), expected);
        assert_eq!(m.permission_mode, expected.as_str());
        assert_eq!(agent.is_plan_mode(), expected == PermissionMode::ReadOnly);
        assert_eq!(
            agent
                .system_prompt
                .contains(davinci_agent::PLAN_MODE_APPENDIX),
            agent.is_plan_mode()
        );
        assert_eq!(m.composer.to_string(), "unfinished request");
    }
}

#[test]
fn context_refresh_resynchronizes_mode_after_session_restore() {
    let mut agent = Agent::new("base prompt");
    let mut m = model();
    change_permission_mode(&mut agent, &mut m, PermissionMode::AlwaysApprove);
    // Session loading can choose Plan Mode without a keyboard event.
    agent.set_permission_mode(PermissionMode::ReadOnly);
    refresh_context(&mut m, &agent);
    assert_eq!(m.permission_label(), "Plan Mode");
    assert!(m.is_plan_mode());
}

#[test]
fn switching_to_plan_from_always_approve_blocks_real_tool_permissions() {
    let mut agent = Agent::new("base prompt");
    let mut m = model();
    change_permission_mode(&mut agent, &mut m, PermissionMode::AlwaysApprove);
    change_permission_mode(&mut agent, &mut m, PermissionMode::ReadOnly);
    let policy = agent.permissions.lock().unwrap();
    let verdict = policy.decide(
        "plan-transition",
        "write",
        &serde_json::json!({"path":"src/new.rs", "content":"mutation"}),
        &agent.cwd,
    );
    assert!(matches!(
        verdict,
        davinci_agent::PermissionVerdict::Deny { .. }
    ));
    assert_eq!(m.permission_label(), "Plan Mode");
}

fn partial(text: &str) -> std::sync::Arc<davinci_ai::AssistantMessage> {
    std::sync::Arc::new(davinci_ai::AssistantMessage {
        id: "m".into(),
        role: "assistant".into(),
        content: vec![davinci_ai::ContentBlock::Text { text: text.into() }],
        model: "fixture".into(),
        usage: None,
        stop_reason: None,
        error_message: None,
    })
}

fn update(event: davinci_ai::AssistantMessageEvent) -> AgentEvent {
    AgentEvent::MessageUpdate {
        message: Arc::new(davinci_ai::assistant_to_chat(event.message())),
        assistant_message_event: event,
    }
}

#[test]
fn text_deltas_stream_into_one_prose_entry_that_message_end_keeps() {
    use davinci_ai::AssistantMessageEvent as Ev;
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::MessageStart {
            message: davinci_ai::ChatMessage::text("assistant", ""),
        },
    );
    apply(
        &mut m,
        &mut turn,
        &update(Ev::TextStart {
            content_index: 0,
            partial: partial(""),
        }),
    );
    apply(
        &mut m,
        &mut turn,
        &update(Ev::TextDelta {
            content_index: 0,
            delta: "Hel".into(),
            partial: partial("Hel"),
        }),
    );
    apply(
        &mut m,
        &mut turn,
        &update(Ev::TextDelta {
            content_index: 0,
            delta: "lo".into(),
            partial: partial("Hello"),
        }),
    );
    let prose: Vec<&Entry> = m
        .transcript
        .iter()
        .filter(|entry| matches!(entry, Entry::Prose(_)))
        .collect();
    assert_eq!(prose.len(), 1, "{:?}", m.transcript);
    assert!(matches!(prose[0], Entry::Prose(text) if text == "Hello"));

    apply(
        &mut m,
        &mut turn,
        &AgentEvent::MessageEnd {
            message: davinci_ai::ChatMessage::text("assistant", "Hello"),
        },
    );
    let prose: Vec<&Entry> = m
        .transcript
        .iter()
        .filter(|entry| matches!(entry, Entry::Prose(_)))
        .collect();
    assert_eq!(
        prose.len(),
        1,
        "message end must not repeat the streamed text"
    );
    assert!(turn.said_something);
}

#[test]
fn a_message_that_never_streamed_still_lands_at_message_end() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::MessageEnd {
            message: davinci_ai::ChatMessage::text("assistant", "whole reply"),
        },
    );
    assert!(matches!(
        m.transcript.last(),
        Some(Entry::Prose(text)) if text == "whole reply"
    ));
}

#[test]
fn reasoning_streams_live_and_collapses_when_the_text_starts() {
    use davinci_ai::AssistantMessageEvent as Ev;
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &update(Ev::ThinkingDelta {
            content_index: 0,
            delta: "Need the file first.".into(),
            partial: partial(""),
        }),
    );
    assert!(matches!(
        m.transcript.last(),
        Some(Entry::Thinking { text, live: true, .. }) if text == "Need the file first."
    ));
    apply(
        &mut m,
        &mut turn,
        &update(Ev::TextDelta {
            content_index: 1,
            delta: "Reading.".into(),
            partial: partial("Reading."),
        }),
    );
    let thinking = m
        .transcript
        .iter()
        .find(|entry| matches!(entry, Entry::Thinking { .. }))
        .expect("thinking row");
    assert!(matches!(thinking, Entry::Thinking { live: false, .. }));
    assert!(matches!(m.transcript.last(), Some(Entry::Prose(text)) if text == "Reading."));
}

#[test]
fn hidden_reasoning_never_reaches_the_transcript() {
    use davinci_ai::AssistantMessageEvent as Ev;
    let mut m = model();
    let mut turn = Turn {
        hide_thinking: true,
        ..Turn::default()
    };
    apply(
        &mut m,
        &mut turn,
        &update(Ev::ThinkingDelta {
            content_index: 0,
            delta: "secret".into(),
            partial: partial(""),
        }),
    );
    assert!(!m
        .transcript
        .iter()
        .any(|entry| matches!(entry, Entry::Thinking { .. })));
}

#[test]
fn the_catalogue_opens_with_newer_models_before_the_current_older_model() {
    let row = |provider: &str, id: &str, ready: bool| CatalogRow {
        name: format!("{provider}/{id}"),
        detail: String::new(),
        window: "200k".into(),
        thinking: "none".into(),
        price: "1.00 · 2.00".into(),
        credential: if ready {
            Credential::Ready
        } else {
            Credential::Absent
        },
        note: String::new(),
        ring: false,
        provider: provider.into(),
        id: id.into(),
        ..Default::default()
    };
    let mut catalog = vec![
        row("amazon-bedrock", "nova", false),
        row("anthropic", "claude", true),
        row("openai-codex", "gpt-5", true),
        row("openai-codex", "gpt-5-mini", true),
        row("openai-codex", "gpt-5.6-luna", true),
        row("openai-codex", "gpt-6-astra", true),
        row("xai", "grok", false),
    ];
    let index = order_catalog(&mut catalog, "openai-codex", "gpt-5-mini");
    assert_eq!(index, 0);
    let names: Vec<&str> = catalog.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "openai-codex/gpt-6-astra",
            "openai-codex/gpt-5.6-luna",
            "openai-codex/gpt-5",
            "openai-codex/gpt-5-mini",
            "anthropic/claude",
        ]
    );
}

#[test]
fn what_the_session_found_is_said_under_the_mark_not_in_the_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
    std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
    let mut agent = davinci_agent::Agent::new("test");
    agent.cwd = dir.path().to_path_buf();
    agent.context_files.push(davinci_agent::ContextFile {
        path: dir.path().join("AGENTS.md"),
        name: "AGENTS.md".into(),
        body: "be kind".into(),
    });
    let found = opening_found(&crate::args::Args::default(), &agent);
    assert_eq!(found, ["loaded 1 context file"]);
    let block = opening_block(&crate::args::Args::default(), &agent, &[]);
    assert!(!block
        .iter()
        .any(|entry| matches!(entry, Entry::Prose(text) if text.starts_with("loaded"))));
    match previous {
        Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
        None => std::env::remove_var("PI_CODING_AGENT_DIR"),
    }
}

#[test]
fn f01_native_denial_editor_requires_confirmation_and_keeps_draft() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    for cancel in [false, true] {
        let request = approval("write", "ordinary.txt", false);
        let choices = request.host_choices(true);
        let mut m = model();
        m.width = 40;
        m.height = 12;
        m.composer.set_text("original cafe\u{301}");
        m.composer.editor_mut().move_left();
        let draft = m.composer.to_string();
        let cursor = m.composer.editor().get_cursor();
        m.ask = permission_ask(&request, true);
        open_ask_overlay(&mut m);
        let abort = Arc::new(AtomicBool::new(false));
        let press = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(approval_answer_key(
            &mut m,
            press(KeyCode::Char('5')),
            &choices,
            &request,
            &abort
        )
        .is_none());
        assert!(m.approval_instructions.is_none());
        assert!(
            approval_answer_key(&mut m, press(KeyCode::Enter), &choices, &request, &abort)
                .is_none()
        );
        assert!(m.approval_instructions.is_some());
        assert!(
            approval_answer_key(&mut m, press(KeyCode::Enter), &choices, &request, &abort)
                .is_none()
        );
        for ch in "try cafe\u{301}".chars() {
            assert!(approval_answer_key(
                &mut m,
                press(KeyCode::Char(ch)),
                &choices,
                &request,
                &abort
            )
            .is_none());
        }
        m.paste("must not enter either buffer");
        assert!(!m.insert_dictation("late voice", m.composer_epoch).unwrap());
        assert_eq!(
            m.approval_instructions.as_ref().unwrap().to_string(),
            "try cafe\u{301}"
        );
        let mut repeat = press(KeyCode::Enter);
        repeat.kind = KeyEventKind::Repeat;
        assert!(approval_answer_key(&mut m, repeat, &choices, &request, &abort).is_none());
        m.width = 120;
        m.height = 40;
        let rendered = davinci_tui::davinci::views::ask::lines(&m);
        assert!(rendered
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.content.contains("try cafe")));
        let answer = approval_answer_key(
            &mut m,
            press(if cancel { KeyCode::Esc } else { KeyCode::Enter }),
            &choices,
            &request,
            &abort,
        )
        .unwrap();
        assert_eq!(answer.decision, ToolApprovalDecision::Deny);
        assert_eq!(
            answer.instructions.as_deref(),
            if cancel {
                None
            } else {
                Some("try cafe\u{301}")
            }
        );
        assert!(m.approval_instructions.is_none());
        assert!(m.overlay.is_none());
        assert_eq!(m.composer.to_string(), draft);
        assert_eq!(m.composer.editor().get_cursor(), cursor);
    }
}

#[test]
fn f01_native_denial_editor_bounds_utf8_and_cancels() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let request = approval("write", "ordinary.txt", false);
    let choices = request.host_choices(true);
    let mut m = model();
    m.ask = permission_ask(&request, true);
    open_ask_overlay(&mut m);
    m.approval_instructions = Some("x".repeat(4095).into());
    let abort = Arc::new(AtomicBool::new(false));
    assert!(approval_answer_key(
        &mut m,
        KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
        &choices,
        &request,
        &abort
    )
    .is_none());
    assert_eq!(m.approval_instructions.as_ref().unwrap().len(), 4095);
    assert!(approval_answer_key(
        &mut m,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        &choices,
        &request,
        &abort
    )
    .is_none());
    assert_eq!(m.approval_instructions.as_ref().unwrap().len(), 4096);
    let answer = approval_answer_key(
        &mut m,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &choices,
        &request,
        &abort,
    )
    .unwrap();
    assert_eq!(answer, ToolApprovalDecision::Deny.into());
    assert!(abort.load(Ordering::Relaxed));
    assert!(m.approval_instructions.is_none());
    assert!(m.overlay.is_none());
}

#[test]
fn f01_native_offers_policy_owned_denial_instructions() {
    let mut request = approval("write", "ordinary.txt", false);
    assert_eq!(
        permission_ask(&request, true).items.last().unwrap().label,
        "deny with instructions"
    );
    request
        .legal_choices
        .retain(|choice| choice.scope != davinci_agent::approval::GrantScope::DenyWithInstructions);
    assert_eq!(
        permission_ask(&request, true).items.last().unwrap().label,
        "deny"
    );
}

#[test]
fn f01_modal_numeric_focus_preserves_draft_until_enter() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for (width, height) in [(40, 12), (120, 40)] {
        let mut m = model();
        m.width = width;
        m.height = height;
        m.composer.set_text("keep cafe\u{301} draft");
        m.composer.editor_mut().move_left();
        let cursor = m.composer.editor().get_cursor();
        let original = m.composer.to_string();
        let permission = m.permission_mode.clone();
        let request = approval("write", "ordinary.txt", false);
        let choices = request.host_choices(true);
        m.ask = permission_ask(&request, true);
        open_ask_overlay(&mut m);
        let abort = Arc::new(AtomicBool::new(false));
        for key in [
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
        ] {
            assert_eq!(approval_key(&mut m, key, &choices, &abort), None);
        }
        assert_eq!(m.ask_index, 1);
        m.keybindings = davinci_tui::Keybindings::from_json(r#"{"tui.select.confirm":"x"}"#);
        assert_eq!(
            approval_key(
                &mut m,
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                &choices,
                &abort
            ),
            None
        );
        m.paste("\r\npasted instruction");
        assert!(!m
            .insert_dictation("voice instruction", m.composer_epoch)
            .unwrap());
        assert_eq!(m.composer.to_string(), original);
        assert_eq!(m.composer.editor().get_cursor(), cursor);
        assert_eq!(m.permission_mode, permission);
        assert_eq!(
            approval_key(
                &mut m,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &choices,
                &abort
            ),
            Some(ToolApprovalDecision::AllowForSession)
        );
        assert_eq!(m.composer.to_string(), original);
        assert_eq!(m.composer.editor().get_cursor(), cursor);
        open_ask_overlay(&mut m);
        assert_eq!(
            approval_key(
                &mut m,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &choices,
                &abort
            ),
            Some(ToolApprovalDecision::Deny)
        );
        assert_eq!(m.composer.to_string(), original);
        assert_eq!(m.composer.editor().get_cursor(), cursor);
    }
}

#[test]
fn f01_native_approval_wait_releases_on_cancel_expiry_and_disconnect() {
    for case in [
        "allow",
        "instructions",
        "cancel",
        "expired",
        "disconnect",
        "ui_error",
        "expires_waiting",
    ] {
        let (tx, rx) = mpsc::channel();
        let abort = Arc::new(AtomicBool::new(case == "cancel"));
        let expires = if case == "expired" {
            0
        } else if case == "expires_waiting" {
            davinci_session::now_ms() + 250
        } else {
            davinci_session::now_ms() + 60_000
        };
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                wait_native_approval(
                    &approval("write", "ordinary.txt", false),
                    expires,
                    &tx,
                    &abort,
                )
            });
            if matches!(
                case,
                "allow" | "instructions" | "ui_error" | "expires_waiting"
            ) {
                let pending = rx.recv_timeout(Duration::from_secs(1)).unwrap();
                if case == "allow" {
                    pending
                        .reply
                        .send(ToolApprovalDecision::AllowOnce.into())
                        .unwrap();
                } else if case == "instructions" {
                    pending
                        .reply
                        .send(NativeApprovalAnswer {
                            decision: ToolApprovalDecision::Deny,
                            instructions: Some("read the docs first".into()),
                        })
                        .unwrap();
                } else if case == "ui_error" {
                    let _exit = ApprovalUiExit::new(abort.clone());
                }
                let start = Instant::now();
                while !worker.is_finished() && start.elapsed() < Duration::from_secs(1) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(worker.is_finished());
                assert!(!pending.live.load(Ordering::Relaxed));
            }
            drop(rx);
            let answer = worker.join().unwrap();
            assert_eq!(
                answer.decision,
                if case == "allow" {
                    ToolApprovalDecision::AllowOnce
                } else {
                    ToolApprovalDecision::Deny
                }
            );
            let challenge: davinci_agent::approval::ApprovalChallenge = serde_json::from_value(json!({
                    "schema_version": 1, "id": "00000000-0000-0000-0000-000000000001", "call_id": "fixture",
                    "action_digest": "fixture", "policy_revision": 0, "contract_revision": null,
                    "mode": "ask", "action_label": "write", "display_target": "ordinary.txt", "reason": "fixture",
                    "legal_choices": [], "expires_at_ms": expires
                })).unwrap();
            let reply = answer.into_reply(&challenge);
            assert_eq!(reply.challenge_id, challenge.id);
            assert_eq!(
                reply.choice_id,
                if case == "instructions" {
                    "deny_with_instructions"
                } else if case == "allow" {
                    "once"
                } else {
                    "deny"
                }
            );
            assert_eq!(
                reply.instructions.as_deref(),
                if case == "instructions" {
                    Some("read the docs first")
                } else {
                    None
                }
            );
        });
    }
}

fn approval(tool: &str, subject: &str, outside: bool) -> ToolApprovalRequest {
    ToolApprovalRequest {
        legal_choices: davinci_agent::approval::offer_scopes(false, true, true),
        tool_call_id: "call_1".into(),
        tool: tool.into(),
        args: json!({}),
        subject: subject.into(),
        summary: davinci_agent::summary_of(tool, subject),
        session_rule: davinci_agent::session_rule_for(tool, subject).to_string(),
        outside_project: outside,
        mode: PermissionMode::Ask,
    }
}

#[test]
fn the_permission_panel_offers_always_only_in_a_trusted_project() {
    let ask = permission_ask(&approval("bash", "git status --short", false), true);
    assert_eq!(ask.title, "Permission");
    assert_eq!(ask.name, "PERMISSION");
    assert_eq!(ask.key, "/permissions");
    assert_eq!(ask.note, "bash · git status --short");
    let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "allow once",
            "allow for this session",
            "always allow here",
            "deny",
            "deny with instructions"
        ]
    );
    assert_eq!(ask.items[1].detail, "bash(git status *) until pi exits");
    assert_eq!(
        ask.items[2].detail,
        "bash(git status *) saved to .pi/settings.json"
    );

    let ask = permission_ask(&approval("write", "../out.txt", true), false);
    let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "allow once",
            "allow for this session",
            "deny",
            "deny with instructions"
        ]
    );
    assert_eq!(ask.note, "write · ../out.txt · outside the project");
}

#[test]
fn f05_scope_expansion_panel_has_only_host_authority_choices() {
    let contract = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-panel",
        7,
        davinci_agent::TaskId::new(),
        3,
        vec!["src/".into()],
        vec!["secrets/".into()],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = contract
        .preview_scope_expansion("migrations/001.sql", "write migration")
        .unwrap();
    let ask = scope_expansion_ask(&preview);
    assert_eq!(ask.title, "Scope expansion required");
    assert_eq!(ask.key, "/scope-expansion");
    let labels: Vec<&str> = ask.items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "approve expansion",
            "request in-scope approach",
            "deny with instructions"
        ]
    );
    assert!(ask.note.contains("migrations/001.sql"));
    assert!(ask.note.contains("revision 7 → 8"));
    assert!(ask.note.contains(&preview.preview_digest));
    assert_eq!(
        scope_expansion_choice(0),
        Some(davinci_agent::runtime::contracts::ScopeExpansionDecision::Approve)
    );
    assert_eq!(
        scope_expansion_choice(1),
        Some(davinci_agent::runtime::contracts::ScopeExpansionDecision::RequestInScopeApproach)
    );
    assert_eq!(
        scope_expansion_choice(2),
        Some(davinci_agent::runtime::contracts::ScopeExpansionDecision::Deny)
    );
    assert_eq!(scope_expansion_choice(3), None);
}

#[test]
fn f05_scope_expansion_deny_collects_bounded_host_instructions() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let contract = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-keys",
        1,
        davinci_agent::TaskId::new(),
        1,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = contract
        .preview_scope_expansion("outside.rs", "blocked write")
        .unwrap();
    let mut m = model();
    m.ask = scope_expansion_ask(&preview);
    open_ask_overlay(&mut m);
    m.ask_index = 2;
    let press = |code| KeyEvent::new(code, KeyModifiers::NONE);

    assert!(scope_expansion_answer_key(&mut m, press(KeyCode::Enter)).is_none());
    assert!(m.approval_instructions.is_some());
    for ch in "stay in src".chars() {
        assert!(scope_expansion_answer_key(&mut m, press(KeyCode::Char(ch))).is_none());
    }
    let answer = scope_expansion_answer_key(&mut m, press(KeyCode::Enter)).unwrap();
    assert_eq!(
        answer.0,
        davinci_agent::runtime::contracts::ScopeExpansionDecision::Deny
    );
    assert_eq!(answer.1.as_deref(), Some("stay in src"));
    assert!(m.overlay.is_none());
}

#[test]
fn f05_scope_expansion_commit_is_durable_exact_and_invalidates_session_grants() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.session =
        Some(davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap());
    let current = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-commit",
        4,
        davinci_agent::TaskId::new(),
        2,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = current
        .preview_scope_expansion("migrations/001.sql", "write migration")
        .unwrap();
    agent.set_active_contract(current.clone());
    agent.permissions.lock().unwrap().remember("write:**");
    assert!(!agent.permissions.lock().unwrap().session_allow.is_empty());

    commit_scope_expansion(&mut agent, &preview, None).unwrap();

    assert_eq!(agent.active_contract(), Some(preview.proposed.clone()));
    assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
    let session = agent.session.as_ref().unwrap();
    let entry = session.entries.last().expect("durable audit entry");
    assert_eq!(
        entry.custom_type.as_deref(),
        Some("task_contract_scope_expansion")
    );
    assert_eq!(
        entry.extra["data"]["preview_digest"],
        preview.preview_digest
    );
    assert_eq!(
        entry.extra["data"]["proposed"]["digest"],
        preview.proposed.digest
    );
}

#[test]
fn f05_scope_expansion_commit_updates_bound_task_digest_before_live_swap() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent
        .load_from_session(
            davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap(),
        )
        .unwrap();
    let runtime = agent.runtime_for_session().unwrap().clone();
    let task_id = davinci_agent::TaskId::new();
    let current = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-bound",
        5,
        task_id,
        2,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = current
        .preview_scope_expansion("migrations/001.sql", "write migration")
        .unwrap();
    let mut task = davinci_agent::TaskRecord::new(runtime.run_id, "bound scope task")
        .with_assigned(runtime.agent_id)
        .with_contract_digest(current.digest.clone());
    task.id = task_id;
    runtime.task_registry.create_task(task).unwrap();
    agent.set_active_contract(current.clone());
    let guard = capture_scope_expansion_task_guard(&agent, &preview).unwrap();

    commit_scope_expansion(&mut agent, &preview, guard.as_ref()).unwrap();

    let rebound = runtime.task_registry.get_task(&task_id).unwrap();
    assert_eq!(
        rebound.contract_digest.as_deref(),
        Some(preview.proposed.digest.as_str())
    );
    assert_eq!(agent.active_contract(), Some(preview.proposed));
}

#[test]
fn f05_scope_expansion_rejects_task_revision_changed_after_preview() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent
        .load_from_session(
            davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap(),
        )
        .unwrap();
    let runtime = agent.runtime_for_session().unwrap().clone();
    let task_id = davinci_agent::TaskId::new();
    let current = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-bound-stale",
        5,
        task_id,
        2,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = current
        .preview_scope_expansion("migrations/001.sql", "write migration")
        .unwrap();
    let mut task = davinci_agent::TaskRecord::new(runtime.run_id, "bound scope task")
        .with_assigned(runtime.agent_id)
        .with_contract_digest(current.digest.clone());
    task.id = task_id;
    runtime.task_registry.create_task(task).unwrap();
    agent.set_active_contract(current.clone());
    agent.permissions.lock().unwrap().remember("write:**");
    let guard = capture_scope_expansion_task_guard(&agent, &preview).unwrap();
    runtime
        .task_registry
        .assign_task(task_id, runtime.agent_id)
        .unwrap();
    let changed = runtime.task_registry.get_task(&task_id).unwrap();

    let error = commit_scope_expansion(&mut agent, &preview, guard.as_ref()).unwrap_err();

    assert!(error.contains("bound task contract digest"));
    assert_eq!(agent.active_contract(), Some(current));
    assert_eq!(
        runtime
            .task_registry
            .get_task(&task_id)
            .unwrap()
            .contract_digest,
        changed.contract_digest
    );
    assert!(!agent.permissions.lock().unwrap().session_allow.is_empty());
}

#[test]
fn f05_scope_expansion_bound_task_update_failure_keeps_live_contract_and_grants() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent
        .load_from_session(
            davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap(),
        )
        .unwrap();
    let runtime = agent.runtime_for_session().unwrap().clone();
    let task_id = davinci_agent::TaskId::new();
    let current = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-bound-fail",
        5,
        task_id,
        2,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = current
        .preview_scope_expansion("migrations/001.sql", "write migration")
        .unwrap();
    let other_agent = davinci_agent::AgentId::new();
    let mut task = davinci_agent::TaskRecord::new(runtime.run_id, "bound scope task")
        .with_assigned(other_agent)
        .with_contract_digest(current.digest.clone());
    task.id = task_id;
    runtime.task_registry.create_task(task).unwrap();
    agent.set_active_contract(current.clone());
    agent.permissions.lock().unwrap().remember("write:**");
    let before = runtime.task_registry.get_task(&task_id).unwrap();
    let guard = capture_scope_expansion_task_guard(&agent, &preview).unwrap();

    let error = commit_scope_expansion(&mut agent, &preview, guard.as_ref()).unwrap_err();

    assert!(error.contains("bound task contract digest"));
    assert_eq!(agent.active_contract(), Some(current));
    assert_eq!(runtime.task_registry.get_task(&task_id), Some(before));
    assert!(!agent.permissions.lock().unwrap().session_allow.is_empty());
    let audit = agent.session.as_ref().unwrap().entries.last().unwrap();
    assert_eq!(
        audit.custom_type.as_deref(),
        Some("task_contract_scope_expansion")
    );
}

#[test]
fn f05_scope_expansion_storage_failure_and_stale_preview_keep_current_contract() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    let mut session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
    session.path = dir.path().join("missing-parent/session.jsonl");
    agent.session = Some(session);
    let current = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-fail",
        9,
        davinci_agent::TaskId::new(),
        2,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = current
        .preview_scope_expansion("migrations/001.sql", "write migration")
        .unwrap();
    agent.set_active_contract(current.clone());
    let prior_leaf = agent.session.as_ref().unwrap().leaf_id.clone();

    assert!(commit_scope_expansion(&mut agent, &preview, None).is_err());
    assert_eq!(agent.active_contract(), Some(current.clone()));
    assert_eq!(agent.session.as_ref().unwrap().leaf_id, prior_leaf);

    let newer = current.expand_scope(vec!["docs/".into()], vec![]).unwrap();
    agent.set_active_contract(newer.clone());
    assert!(commit_scope_expansion(&mut agent, &preview, None).is_err());
    assert_eq!(agent.active_contract(), Some(newer));
}

#[test]
fn f05_scope_expansion_reject_and_in_scope_guidance_keep_contract_unchanged() {
    let current = davinci_agent::runtime::contracts::TaskContract::new(
        "scope-reject",
        3,
        davinci_agent::TaskId::new(),
        1,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let preview = current
        .preview_scope_expansion("outside/file.rs", "requested write")
        .unwrap();

    let mut always_agent = Agent::new("fixture");
    always_agent.set_active_contract(current.clone());
    always_agent.set_permission_mode(PermissionMode::AlwaysApprove);
    assert_eq!(always_agent.active_contract(), Some(current.clone()));

    for (decision, instructions) in [
        (
            davinci_agent::runtime::contracts::ScopeExpansionDecision::RequestInScopeApproach,
            None,
        ),
        (
            davinci_agent::runtime::contracts::ScopeExpansionDecision::Deny,
            Some("do not modify generated files".to_string()),
        ),
    ] {
        let mut agent = Agent::new("fixture");
        agent.set_active_contract(current.clone());
        let guidance = apply_scope_expansion_decision(
            &mut agent,
            &preview,
            None,
            decision,
            instructions.as_deref(),
        )
        .unwrap()
        .expect("non-approval guidance");
        assert_eq!(agent.active_contract(), Some(current.clone()));
        assert!(guidance.contains("outside/file.rs"));
        if let Some(instructions) = instructions.as_deref() {
            assert!(guidance.contains(instructions));
        }
    }
}

#[test]
fn f09_watchdog_ask_and_choice_navigation() {
    use davinci_agent::runtime::progress_watchdog::LoopSignal;
    let signal = LoopSignal::RepeatedTestFailureWithoutNewDiagnosis {
        signature: "test_foo failed: assertion failed".into(),
        count: 3,
    };
    let ask = watchdog_ask(&signal);
    assert_eq!(ask.title, "Progress watchdog");
    assert_eq!(ask.name, "WATCHDOG");
    assert_eq!(ask.key, "/watchdog");
    assert_eq!(ask.items.len(), 3);
    assert_eq!(watchdog_choice(0), Some("continue"));
    assert_eq!(watchdog_choice(1), Some("plan"));
    assert_eq!(watchdog_choice(2), Some("stop"));
    assert_eq!(watchdog_choice(3), None);

    let mut model = Model::new(
        davinci_tui::davinci::theme::Theme::da_vinci(
            davinci_tui::davinci::theme::ColorDepth::TrueColor,
            false,
        ),
        80,
        24,
        false,
    );
    model.ask = ask;
    model.ask_index = 0;
    model.overlay = Some(Overlay::Ask);

    // Press '1' -> continue
    assert_eq!(
        watchdog_answer_key(
            &mut model,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('1'),
                crossterm::event::KeyModifiers::empty(),
            ),
        ),
        Some("continue")
    );

    // Press '2' -> plan
    assert_eq!(
        watchdog_answer_key(
            &mut model,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('2'),
                crossterm::event::KeyModifiers::empty(),
            ),
        ),
        Some("plan")
    );

    // Press '3' -> stop
    assert_eq!(
        watchdog_answer_key(
            &mut model,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('3'),
                crossterm::event::KeyModifiers::empty(),
            ),
        ),
        Some("stop")
    );

    // Press Esc -> stop
    assert_eq!(
        watchdog_answer_key(
            &mut model,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::empty(),
            ),
        ),
        Some("stop")
    );

    // Test apply_watchdog_choice transitions
    let mut agent = Agent::new("test_agent");
    let res_continue = apply_watchdog_choice(&mut agent, "continue", &signal);
    assert!(res_continue.is_ok());

    let res_plan = apply_watchdog_choice(&mut agent, "plan", &signal);
    assert!(res_plan.is_ok());
    assert_eq!(
        agent.permission_mode(),
        davinci_agent::PermissionMode::ReadOnly
    );

    let res_stop = apply_watchdog_choice(&mut agent, "stop", &signal);
    assert!(res_stop.is_ok());
}

#[test]
fn interrupted_recovery_copy_avoids_unverified_guarantees() {
    let rows = interrupted_recovery_aftermath();
    let copy = rows
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(copy.contains("interrupted"));
    assert!(!copy.contains("nothing to recover"));
    assert!(!copy.contains("transcript written"));
    assert!(!copy.contains("abort was delivered"));
    assert!(!copy.contains("esc esc"));
}

#[test]
fn permission_panel_uses_policy_choices_even_in_a_trusted_project() {
    let dir = tempfile::tempdir().unwrap();
    let policy = davinci_agent::PermissionPolicy::new(PermissionMode::Ask);
    let davinci_agent::PermissionVerdict::Ask(request) = policy.decide(
        "risky",
        "write",
        &json!({"path":".env", "content":"fixture"}),
        dir.path(),
    ) else {
        panic!("expected Ask")
    };
    let mut m = model();
    m.ask = permission_ask(&request, true);
    assert_eq!(
        m.ask
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["allow once", "deny", "deny with instructions"]
    );
    open_ask_overlay(&mut m);
    m.ask_index = 1;
    let abort = Arc::new(AtomicBool::new(false));
    assert_eq!(
        approval_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE
            ),
            &request.host_choices(true),
            &abort
        ),
        Some(ToolApprovalDecision::Deny)
    );
    let mut trusted = true;
    assert_eq!(
        persist_permission_choice(
            &mut m,
            &request,
            ToolApprovalDecision::AllowAlways,
            dir.path(),
            &mut trusted
        ),
        Some(ToolApprovalDecision::Deny)
    );
    assert!(!dir.path().join(".davinci/settings.json").exists());
}

#[test]
fn permission_save_failure_reopens_panel_without_granting() {
    use ToolApprovalDecision::*;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".davinci"), "not a directory").unwrap();
    let request = approval("write", "file.txt", false);
    let mut m = model();
    let mut project_allowed = true;
    assert_eq!(
        persist_permission_choice(
            &mut m,
            &request,
            AllowAlways,
            dir.path(),
            &mut project_allowed
        ),
        None
    );
    assert!(!project_allowed);
    assert!(m.ask.note.contains("could not be saved"));
    assert_eq!(m.ask.items.len(), 4);
    let abort = Arc::new(AtomicBool::new(false));
    let decision = approval_key(
        &mut m,
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ),
        &request.host_choices(project_allowed),
        &abort,
    );
    assert_eq!(decision, Some(Deny));
    assert_eq!(
        persist_permission_choice(&mut m, &request, Deny, dir.path(), &mut project_allowed),
        Some(Deny)
    );
    for (index, expected) in [(0, AllowOnce), (1, AllowForSession), (2, Deny)] {
        project_allowed = true;
        assert_eq!(
            persist_permission_choice(
                &mut m,
                &request,
                AllowAlways,
                dir.path(),
                &mut project_allowed
            ),
            None
        );
        assert!(matches!(m.overlay, Some(Overlay::Ask)));
        m.ask_index = index;
        let choice = approval_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
            &request.host_choices(project_allowed),
            &abort,
        )
        .unwrap();
        assert_eq!(
            persist_permission_choice(&mut m, &request, choice, dir.path(), &mut project_allowed),
            Some(expected)
        );
    }
    assert_eq!(
        persist_permission_choice(
            &mut m,
            &request,
            AllowAlways,
            dir.path(),
            &mut project_allowed
        ),
        Some(Deny)
    );
    let saved = tempfile::tempdir().unwrap();
    project_allowed = true;
    assert_eq!(
        persist_permission_choice(
            &mut m,
            &request,
            AllowAlways,
            saved.path(),
            &mut project_allowed
        ),
        Some(AllowAlways)
    );
    let settings: serde_json::Value = serde_json::from_slice(
        &std::fs::read(saved.path().join(".davinci/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["permissions"]["allow"],
        json!([request.session_rule])
    );
}

#[test]
fn permission_rows_map_to_decisions_in_both_shapes() {
    use ToolApprovalDecision::*;
    let request = approval("bash", "git status", false);
    let trusted = request.host_choices(true);
    let untrusted = request.host_choices(false);
    assert_eq!(permission_choice(0, &trusted), Some(AllowOnce));
    assert_eq!(permission_choice(1, &trusted), Some(AllowForSession));
    assert_eq!(permission_choice(2, &trusted), Some(AllowAlways));
    assert_eq!(permission_choice(3, &trusted), Some(Deny));
    assert_eq!(permission_choice(4, &trusted), None);
    assert_eq!(permission_choice(2, &untrusted), Some(Deny));
    assert_eq!(permission_choice(3, &untrusted), None);
}

#[test]
fn under_the_permission_panel_esc_denies_and_enter_chooses_the_row() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
    let abort = Arc::new(AtomicBool::new(false));
    let choices = approval("bash", "git status", false).host_choices(true);
    let open = || {
        let mut m = model();
        m.running = true;
        m.ask = permission_ask(&approval("bash", "git status", false), true);
        m.ask_index = 0;
        m.overlay = Some(Overlay::Ask);
        m
    };

    let mut m = open();
    assert_eq!(
        approval_key(&mut m, key(KeyCode::Esc), &choices, &abort),
        Some(ToolApprovalDecision::Deny)
    );
    assert_eq!(m.overlay, None);
    assert!(
        !abort.load(Ordering::Relaxed),
        "esc refuses; it does not interrupt"
    );

    let mut m = open();
    assert_eq!(
        approval_key(&mut m, key(KeyCode::Down), &choices, &abort),
        None
    );
    assert_eq!(
        m.overlay,
        Some(Overlay::Ask),
        "moving keeps the question up"
    );
    assert_eq!(
        approval_key(&mut m, key(KeyCode::Enter), &choices, &abort),
        Some(ToolApprovalDecision::AllowForSession)
    );
    assert_eq!(m.overlay, None);

    let mut m = open();
    assert_eq!(
        approval_key(
            &mut m,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &choices,
            &abort
        ),
        Some(ToolApprovalDecision::Deny)
    );
    assert!(
        abort.load(Ordering::Relaxed),
        "ctrl+c interrupts the turn as well"
    );
}

#[test]
fn a_waiting_call_says_so_on_its_ledger_row_and_is_quiet_again_once_answered() {
    let mut m = model();
    let mut turn = Turn::default();
    turn.start_tool(&mut m, "call_1", "bash", &json!({"command": "git status"}));
    let request = approval("bash", "git status", false);
    turn.await_approval(&mut m, &request);
    let studio_target = |m: &Model| {
        m.transcript
            .iter()
            .find_map(|entry| match entry {
                Entry::Studio(steps) => steps.last().and_then(|step| step.target.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    let tool_summary = |m: &Model| {
        m.transcript
            .iter()
            .find_map(|entry| match entry {
                Entry::Tool { summary, .. } => Some(summary.clone()),
                _ => None,
            })
            .flatten()
    };
    assert!(
        studio_target(&m).ends_with(" · awaiting approval"),
        "{}",
        studio_target(&m)
    );
    assert_eq!(tool_summary(&m).as_deref(), Some("awaiting approval"));

    turn.settle_approval(&mut m, &request, Some("bash(git status *)"));
    assert!(
        !studio_target(&m).contains("awaiting"),
        "{}",
        studio_target(&m)
    );
    assert_eq!(tool_summary(&m), None);
    assert!(matches!(
        m.transcript.last(),
        Some(Entry::Tool { target, .. }) if target == "remembered bash(git status *) · .pi/settings.json"
    ));
}

#[test]
fn shell_is_manus_and_everything_else_is_instrumenta() {
    assert_eq!(instrument_of("bash"), "manus");
    assert_eq!(instrument_of("powershell"), "manus");
    assert_eq!(instrument_of("read"), "instrumenta");
    assert_eq!(instrument_of("grep"), "instrumenta");
    assert_eq!(instrument_of("memory_search"), "memoria");
    assert_eq!(instrument_of("graph_impact"), "grafo");
}

#[test]
fn every_tool_gets_a_glyph_and_a_failure_overrides_it() {
    assert_eq!(state_of("read", false), State::Read);
    assert_eq!(state_of("grep", false), State::Search);
    assert_eq!(state_of("edit", false), State::Delta);
    assert_eq!(state_of("bash", false), State::Done);
    for tool in ["read", "grep", "edit", "bash"] {
        assert_eq!(state_of(tool, true), State::Failed, "{tool}");
    }
}

#[test]
fn targets_read_as_the_mockups_do() {
    assert_eq!(
        target_of("read", &json!({"path": "crates/davinci-tui/src/lib.rs"})),
        "read crates/davinci-tui/src/lib.rs"
    );
    assert_eq!(
        target_of("grep", &json!({"pattern": "SessionManager"})),
        "search \"SessionManager\""
    );
    assert_eq!(
        target_of("bash", &json!({"command": "cargo test -p pi-session"})),
        "cargo test -p pi-session"
    );
    assert_eq!(target_of("read", &json!({})), "read ");
}

#[test]
fn a_long_target_is_clipped_and_marked() {
    let long = "x".repeat(200);
    let drawn = target_of("bash", &json!({ "command": long }));
    assert!(drawn.chars().count() <= 61, "{}", drawn.chars().count());
    assert!(drawn.ends_with('…'));
}

#[test]
fn only_the_first_line_of_a_multiline_command_is_shown() {
    let drawn = target_of("bash", &json!({"command": "cargo fmt\ncargo clippy"}));
    assert_eq!(drawn, "cargo fmt");
}

#[test]
fn a_failure_keeps_at_most_four_lines() {
    let body = (0..12)
        .map(|i| format!("frame {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lines = failure_lines(&json!(body));
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "frame 0");

    let structured = json!({"output": "error[E0308] mismatched types\nstore.rs:118"});
    assert_eq!(
        failure_lines(&structured),
        vec![
            "error[E0308] mismatched types".to_string(),
            "store.rs:118".to_string()
        ]
    );
}

#[test]
fn a_tool_call_becomes_one_line_and_gains_its_duration() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            args: json!({"command": "cargo fmt"}),
        },
    );
    match &m.transcript[0] {
        Entry::Tool {
            state,
            instrument,
            target,
            duration,
            ..
        } => {
            assert_eq!(*state, State::Done);
            assert_eq!(instrument, "manus");
            assert_eq!(target, "cargo fmt");
            assert!(duration.is_none(), "no duration while it runs");
        }
        other => panic!("{other:?}"),
    }

    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionEnd {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            result: json!("ok"),
            is_error: false,
            details: None,
        },
    );
    match &m.transcript[0] {
        Entry::Tool { duration, .. } => {
            assert!(duration.as_deref().unwrap().ends_with('s'))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_denied_call_keeps_the_done_glyph() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            args: json!({"command": "rm -rf /"}),
        },
    );
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionEnd {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            result: json!("Permission denied: `bash · rm -rf /` matches the deny rule `bash`."),
            is_error: true,
            details: None,
        },
    );
    match &m.transcript[0] {
        Entry::Tool {
            state: State::Done,
            summary,
            ..
        } => assert_eq!(summary.as_deref(), Some("denied")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failing_tool_keeps_its_glyph_and_its_detail_directly_beneath() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            args: json!({"command": "cargo test"}),
        },
    );
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionEnd {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            result: json!("error[E0308] mismatched types\nstore.rs:118"),
            is_error: true,
            details: None,
        },
    );

    match &m.transcript[0] {
        Entry::Tool {
            state: State::Failed,
            output,
            ..
        } => {
            assert!(
                output.iter().any(|line| line.contains("E0308")),
                "{output:?}"
            );
            assert!(
                output.iter().any(|line| line.contains("store.rs:118")),
                "{output:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        !m.transcript
            .iter()
            .any(|entry| matches!(entry, Entry::Detail(_))),
        "the failure rides on the tool line, not as Detail siblings: {:?}",
        m.transcript
    );
}

#[test]
fn concurrent_tools_each_find_their_own_line() {
    let mut m = model();
    let mut turn = Turn::default();
    for (id, command) in [("1", "cargo fmt"), ("2", "cargo clippy")] {
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: id.into(),
                tool_name: "bash".into(),
                args: json!({ "command": command }),
            },
        );
    }
    // The second one finishes first.
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionEnd {
            tool_call_id: "2".into(),
            tool_name: "bash".into(),
            result: json!("ok"),
            is_error: false,
            details: None,
        },
    );

    let durations: Vec<bool> = m
        .transcript
        .iter()
        .filter_map(|entry| match entry {
            Entry::Tool { duration, .. } => Some(duration.is_some()),
            _ => None,
        })
        .collect();
    assert_eq!(durations, vec![false, true], "the right line was closed");
}

#[test]
fn the_studio_ledger_tracks_the_step_in_hand() {
    let mut m = model();
    let mut turn = Turn::default();
    for (id, tool, args) in [
        ("1", "read", json!({"path": "store.rs"})),
        ("2", "bash", json!({"command": "cargo test"})),
    ] {
        apply(
            &mut m,
            &mut turn,
            &AgentEvent::ToolExecutionStart {
                tool_call_id: id.into(),
                tool_name: tool.into(),
                args,
            },
        );
    }

    let studio = m
        .transcript
        .iter()
        .find_map(|entry| match entry {
            Entry::Studio(steps) => Some(steps.clone()),
            _ => None,
        })
        .expect("a ledger");
    assert_eq!(studio.len(), 2);
    assert_eq!(studio[0].state, State::Done, "the earlier step is settled");
    assert_eq!(studio[1].state, State::Active);
    assert_eq!(studio[0].verb, "studying");
    assert_eq!(studio[1].verb, "testing");
}

#[test]
fn an_interrupted_turn_marks_its_open_step_skipped_rather_than_done() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "1".into(),
            tool_name: "bash".into(),
            args: json!({"command": "cargo test"}),
        },
    );
    turn.close(&mut m, true);

    let studio = m
        .transcript
        .iter()
        .find_map(|entry| match entry {
            Entry::Studio(steps) => Some(steps.clone()),
            _ => None,
        })
        .expect("a ledger");
    assert_eq!(studio[0].state, State::Skipped);
}

#[test]
fn an_assistant_reply_becomes_prose_after_a_gap() {
    let mut m = model();
    let mut turn = Turn::default();
    let message = assistant("A request enters the agent as a Turn.");
    apply(&mut m, &mut turn, &AgentEvent::MessageEnd { message });

    assert!(matches!(m.transcript[0], Entry::Gap));
    assert!(matches!(&m.transcript[1], Entry::Prose(text) if text.starts_with("A request")));
    assert!(turn.said_something);
}

#[test]
fn an_empty_reply_is_not_pushed() {
    let mut m = model();
    let mut turn = Turn::default();
    let message = assistant("   \n ");
    apply(&mut m, &mut turn, &AgentEvent::MessageEnd { message });
    assert!(m.transcript.is_empty());
    assert!(!turn.said_something);
}

#[test]
fn a_plain_line_is_a_prompt_and_slash_quit_leaves() {
    assert!(
        matches!(classify("explain the runtime"), Sent::Prompt(text) if text == "explain the runtime")
    );
    assert!(matches!(classify("/quit"), Sent::Quit));
    assert!(matches!(classify("/exit"), Sent::Quit));
}

#[test]
fn commands_with_a_home_reach_perform_which_opens_their_sheet() {
    use crate::slash::SlashAction;
    // Each of these opens its designed sheet (screens 3a–4a) from
    // `perform`, with live data behind it, rather than a bare overlay.
    assert!(matches!(
        classify("/model"),
        Sent::Command(SlashAction::OpenModel)
    ));
    assert!(matches!(
        classify("/resume"),
        Sent::Command(SlashAction::Resume)
    ));
    assert!(matches!(
        classify("/settings"),
        Sent::Command(SlashAction::Settings)
    ));
    assert!(matches!(
        classify("/hotkeys"),
        Sent::Command(SlashAction::Hotkeys)
    ));
}

#[test]
fn help_answers_without_asking_the_model() {
    assert!(matches!(classify("/help"), Sent::Say(text) if text.contains('/')));
}

#[test]
fn init_starts_a_repository_analysis_prompt() {
    assert!(
        matches!(classify("/init"), Sent::Prompt(text) if text.contains("AGENTS.md") && !text.starts_with('/'))
    );
}

#[test]
fn thinking_command_reaches_the_agent_for_every_supported_level() {
    for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
        let command = format!("/thinking {level}");
        assert!(matches!(
            classify(&command),
            Sent::Command(crate::slash::SlashAction::SetThinking(value)) if value == level
        ));
    }
    assert!(crate::slash::builtin_slash_commands()
        .iter()
        .any(|command| command.name == "thinking"));
}

#[test]
fn every_builtin_command_reaches_the_agent_rather_than_the_model() {
    use crate::slash::SlashAction;
    // Nothing a `/` line can parse to may fall through to the model as
    // prose: either an instrument opens, or the agent carries it out.
    for line in [
        "/compact",
        "/new",
        "/export report.html",
        "/name work",
        "/fork",
        "/clone",
        "/copy",
        "/reload",
        "/import a.jsonl",
        "/share",
        "/session",
        "/thinking high",
        "/mcp",
        "/cost",
        "/status",
        "/agents",
        "/logout",
        "/login openai",
        "/tree",
    ] {
        match classify(line) {
            Sent::Command(_) => {}
            _ => panic!("{line} did not reach the agent"),
        }
    }
    assert!(matches!(
        classify("/export report.html"),
        Sent::Command(SlashAction::Export(Some(path))) if path == "report.html"
    ));
}

#[test]
fn recall_searches_for_what_is_typed_and_falls_back_to_the_last_ask() {
    let mut m = model();
    let mut agent = davinci_agent::Agent::new("test");
    assert_eq!(recall_query(&m, &agent), "", "nothing typed, nothing asked");

    agent.messages.push(davinci_ai::ChatMessage {
        role: "user".into(),
        content: vec![davinci_ai::MessageContent::Text {
            text: "how does the session store work".into(),
        }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        extra: Default::default(),
    });
    agent.messages.push(assistant("it appends to a jsonl file"));
    assert_eq!(recall_query(&m, &agent), "how does the session store work");

    m.composer = "  branch cache  ".into();
    assert_eq!(recall_query(&m, &agent), "branch cache");
}

#[test]
fn extensions_get_rows_and_a_title_but_never_the_palette() {
    let mut m = model();
    let title = apply_ui_calls(
        &mut m,
        &[
            json!({"op": "setHeader", "lines": ["branch: rust-rewrite"]}),
            json!({"op": "setFooter", "lines": ["2 checks pending"]}),
            json!({"op": "setWidget", "key": "todo", "lines": ["3 open todos"]}),
            json!({"op": "setWidget", "key": "hint", "content": "one\ntwo", "placement": "belowEditor"}),
            json!({"op": "setStatus", "key": "sync", "text": "synced"}),
            json!({"op": "notify", "message": "the index is stale"}),
            json!({"op": "setTitle", "title": "pi · rust-rewrite"}),
            // Ignored: davinci has one palette and two animations.
            json!({"op": "setTheme", "theme": "solarized"}),
            json!({"op": "setWorkingIndicator", "frames": ["-", "\\"]}),
        ],
    );

    assert_eq!(title.as_deref(), Some("pi · rust-rewrite"));
    assert_eq!(
        m.extensions.header,
        vec!["branch: rust-rewrite".to_string()]
    );
    assert_eq!(m.extensions.footer, vec!["2 checks pending".to_string()]);
    assert_eq!(m.extensions.above(), vec!["3 open todos", "synced"]);
    assert_eq!(m.extensions.below(), vec!["one", "two"]);
    assert!(m.transcript.iter().any(|entry| {
        matches!(entry, Entry::Tool { state, target, .. }
                if *state == State::Attention && target == "the index is stale")
    }));

    // A widget with no lines is a removal, keyed as the extension keyed it.
    apply_ui_calls(&mut m, &[json!({"op": "setWidget", "key": "todo"})]);
    assert_eq!(m.extensions.above(), vec!["synced"]);
    apply_ui_calls(&mut m, &[json!({"op": "setStatus", "key": "sync"})]);
    assert!(m.extensions.above().is_empty());
}

#[test]
fn an_extension_can_fill_the_composer_and_add_to_it() {
    let mut m = model();
    apply_ui_calls(&mut m, &[json!({"op": "setEditorText", "text": "review "})]);
    assert_eq!(m.composer, "review ");
    apply_ui_calls(
        &mut m,
        &[json!({"op": "pasteToEditor", "text": "the diff"})],
    );
    assert_eq!(m.composer, "review the diff");
}

#[test]
fn a_question_wears_a_named_panel_and_one_row_per_answer() {
    let mut agent = davinci_agent::Agent::new("test");
    agent.thinking_level = davinci_protocol::ThinkingLevel::Medium;

    let trust = Question::Trust {
        path: "C:\\work\\pi-rust".into(),
        options: crate::trust::get_project_trust_options(std::path::Path::new("."), false),
    };
    let panel = trust.ask(&agent);
    assert_eq!(panel.title, "Trust");
    assert!(panel.note.contains("C:\\work\\pi-rust"), "{}", panel.note);
    assert!(!panel.items.is_empty());

    let credentials = Question::Logout {
        providers: vec!["anthropic".into(), "openai".into()],
    }
    .ask(&agent);
    assert_eq!(credentials.title, "Credentials");
    assert_eq!(credentials.items.len(), 2);
    assert_eq!(credentials.items[0].label, "anthropic");
}

#[test]
fn first_run_asks_only_what_davinci_cannot_decide_for_itself() {
    let agent = davinci_agent::Agent::new("test");
    let panel = Question::FirstRun.ask(&agent);
    // The old setup asked for a theme too; there is one palette here, so
    // the only question left is the one about the user, not the terminal.
    assert_eq!(panel.title, "Welcome");
    assert_eq!(panel.items.len(), 2);
    assert!(panel.items[0].label.contains("share"));
    assert!(panel.note.contains("never required"), "{}", panel.note);
}

#[test]
fn an_untrusted_project_with_pi_resources_is_warned_about_by_glyph() {
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
    std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
    let project = dir.path().join("project");
    std::fs::create_dir_all(project.join(".pi").join("skills")).unwrap();

    let mut agent = davinci_agent::Agent::new("test");
    agent.cwd = project.clone();
    let block = opening_block(&crate::args::Args::default(), &agent, &[]);

    let warned = block.iter().any(|entry| {
        matches!(entry, Entry::Tool { state, target, .. }
                if *state == State::Attention && target.contains("not trusted"))
    });
    assert!(warned, "an untrusted project must say so");

    match previous {
        Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
        None => std::env::remove_var("PI_CODING_AGENT_DIR"),
    }
}

#[test]
fn a_trusted_project_opens_without_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
    std::env::set_var("PI_CODING_AGENT_DIR", dir.path().join("agent"));
    let project = dir.path().join("plain");
    std::fs::create_dir_all(&project).unwrap();

    let mut agent = davinci_agent::Agent::new("test");
    agent.cwd = project;
    let block = opening_block(&crate::args::Args::default(), &agent, &[]);
    assert!(!block.iter().any(|entry| {
        matches!(entry, Entry::Tool { target, .. } if target.contains("not trusted"))
    }));

    match previous {
        Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
        None => std::env::remove_var("PI_CODING_AGENT_DIR"),
    }
}

#[test]
fn a_question_with_nothing_in_it_still_produces_a_panel() {
    let agent = davinci_agent::Agent::new("test");
    let panel = Question::Logout { providers: vec![] }.ask(&agent);
    assert!(panel.items.is_empty());
    assert_eq!(panel.key, "/logout");
}

#[test]
fn an_unknown_slash_command_is_still_a_prompt() {
    // Skills and templates arrive this way; the agent expands them.
    // Extension commands never reach here — `on_line` runs them first.
    assert!(matches!(classify("/skill:review"), Sent::Prompt(_)));
}

#[test]
fn a_native_command_is_run_by_the_shell_not_sent_to_the_model() {
    // `/graph-view` used to fall through `classify` as prose, reach the
    // provider verbatim, and come back as "the model returned no text".
    let host = crate::extension_host::ExtensionHost::default();
    let result = host
        .execute_native_command("graph-status", "")
        .expect("graph-status is a native command");
    assert!(result.is_some(), "the host must claim it");

    let mut model = Model::new(
        davinci_tui::davinci::theme::Theme::da_vinci(
            davinci_tui::davinci::theme::ColorDepth::TrueColor,
            false,
        ),
        100,
        44,
        true,
    );
    push_command_result(&mut model, "graph-status", &result.unwrap());
    let said: Vec<String> = model
        .transcript
        .iter()
        .filter_map(|entry| match entry {
            Entry::Tool { target, .. } => Some(target.clone()),
            Entry::Detail(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        said.iter().any(|line| line == "/graph-status"),
        "the command names itself: {said:?}"
    );
}

#[test]
fn a_failed_request_says_why_instead_of_blaming_the_model() {
    let target = |entries: &[Entry]| -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Tool { target, .. } => Some(target.clone()),
                Entry::Detail(text) => Some(text.clone()),
                Entry::Prose(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    };

    // The provider's own words, not "the model returned no text". This is
    // the whole point: the reason used to be dropped with the worker's
    // return value, and every failure read as an empty answer.
    let out = turn_outcome(
        false,
        false,
        false,
        Some("No credential for openai-codex. Run /login openai-codex.".into()),
        "",
    );
    assert!(target(&out).iter().any(|line| line == "the request failed"));
    assert!(target(&out)
        .iter()
        .any(|line| line.contains("Run /login openai-codex")));
    assert!(out
        .iter()
        .any(|entry| matches!(entry, Entry::Tool { state, .. } if *state == State::Failed)));

    // A turn that genuinely said nothing still says so.
    let out = turn_outcome(false, false, false, None, "");
    assert!(target(&out)
        .iter()
        .any(|line| line.contains("the model returned no text")));

    // Text the events never carried is said rather than dropped.
    let out = turn_outcome(false, false, false, None, "  late answer  ");
    assert_eq!(target(&out), vec!["late answer".to_string()]);

    // A turn that spoke owes nothing more.
    assert!(turn_outcome(false, false, true, None, "").is_empty());

    // A crash and an interrupt outrank a failure message.
    let out = turn_outcome(true, false, false, Some("ignored".into()), "");
    assert!(target(&out).iter().any(|line| line.contains("crashed")));
    let out = turn_outcome(false, true, false, Some("ignored".into()), "");
    assert!(target(&out).iter().any(|line| line.contains("interrupted")));

    // An empty failure string is not a failure.
    let out = turn_outcome(false, false, false, Some("   ".into()), "");
    assert!(target(&out)
        .iter()
        .any(|line| line.contains("the model returned no text")));
}

#[test]
fn a_slash_nobody_owns_is_named_rather_than_sent_to_the_model() {
    let mut m = Model::new(
        davinci_tui::davinci::theme::Theme::da_vinci(
            davinci_tui::davinci::theme::ColorDepth::TrueColor,
            false,
        ),
        100,
        44,
        true,
    );
    m.slash_commands = ["graph-view", "graph-status", "compact"]
        .into_iter()
        .map(|name| davinci_tui::SlashCommandSpec {
            name: name.into(),
            description: String::new(),
            argument_hint: None,
            argument_items: Vec::new(),
        })
        .collect();

    // A transposition must not be answered with the shorter command it
    // happens to begin with.
    let note = unknown_command(&m, "/graph-veiw").expect("not a command");
    assert!(note.contains("did you mean /graph-view?"), "{note}");

    // The typo in the screenshot: a trailing full stop.
    let note = unknown_command(&m, "/graph-view.").expect("not a command");
    assert!(note.contains("did you mean /graph-view?"), "{note}");

    // A prefix offers the shortest name it could still become.
    let note = unknown_command(&m, "/graph-s").expect("not a command");
    assert!(note.contains("did you mean /graph-status?"), "{note}");

    // A real command is left alone.
    assert!(unknown_command(&m, "/graph-view").is_none());
    assert!(unknown_command(&m, "/compact").is_none());

    // A slash line with arguments is a prompt, not a mistyped command.
    assert!(unknown_command(&m, "/explain this file").is_none());
    assert!(unknown_command(&m, "plain prose").is_none());

    // Nothing close: still say what to press.
    let note = unknown_command(&m, "/zzzz").expect("not a command");
    assert!(note.contains("ctrl+p"), "{note}");
}

#[test]
fn a_command_result_becomes_rows_rather_than_a_json_dump() {
    let value = serde_json::json!({
        "runId": "run-7",
        "tasks": [{"id": "review-1", "status": "running"}],
        "pending": [],
    });
    let rows = command_result_rows(&value);
    assert!(rows.iter().any(|row| row == "run id · run-7"), "{rows:?}");
    assert!(rows.iter().any(|row| row == "tasks · 1"), "{rows:?}");
    assert!(
        rows.iter().any(|row| row == "  review-1 · running"),
        "{rows:?}"
    );
    assert!(rows.iter().any(|row| row == "pending · none"), "{rows:?}");
    assert!(
        !rows.iter().any(|row| row.contains('{')),
        "no raw JSON: {rows:?}"
    );
}

#[test]
fn an_error_result_carries_the_attention_glyph() {
    let mut model = Model::new(
        davinci_tui::davinci::theme::Theme::da_vinci(
            davinci_tui::davinci::theme::ColorDepth::TrueColor,
            false,
        ),
        100,
        44,
        true,
    );
    push_command_result(
        &mut model,
        "graph-view",
        &serde_json::json!({"error": "No graph runs in this project."}),
    );
    let failed = model.transcript.iter().any(|entry| {
        matches!(
            entry,
            Entry::Tool { state, instrument, .. }
                if *state == State::Attention && instrument == "grafo"
        )
    });
    assert!(failed, "{:?}", model.transcript);
}

#[test]
fn the_palette_lists_every_command_the_composer_completes() {
    let agent = Agent::new("x");
    let commands = vec![
        davinci_tui::SlashCommandSpec {
            name: "graph-view".into(),
            description: "Tail a graph worker's live transcript.".into(),
            argument_hint: None,
            argument_items: Vec::new(),
        },
        davinci_tui::SlashCommandSpec {
            name: "quit".into(),
            description: "Leave".into(),
            argument_hint: None,
            argument_items: Vec::new(),
        },
    ];
    let items = corpus(&agent, &commands, &[]);
    let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
    assert!(names.contains(&"/graph-view"), "{names:?}");
    assert!(names.contains(&"/quit"), "{names:?}");
    assert!(names.contains(&"/todo"), "{names:?}");
    assert!(names.contains(&"/jobs"), "{names:?}");
    assert!(names.contains(&"/mcp"), "{names:?}");
    assert!(names.contains(&"/plan"), "{names:?}");
    assert!(names.contains(&"/act"), "{names:?}");
    assert!(names.contains(&"/cost"), "{names:?}");
    assert!(names.contains(&"/status"), "{names:?}");
    assert!(names.contains(&"/agents"), "{names:?}");
    assert!(names.contains(&"/workflow"), "{names:?}");
    assert!(names.contains(&"/workflows"), "{names:?}");
    assert!(names.contains(&"/workflow-stop"), "{names:?}");
    assert!(names.contains(&"/workflow-resume"), "{names:?}");
}

#[test]
fn a_tool_row_says_what_the_tool_does_not_which_instrument_ran_it() {
    // design.md §3: `instrumenta` is the default instrument and is never
    // named. It used to fill the middle column of nearly every tool row,
    // which made the column say nothing at all.
    let mut agent = Agent::new("x");
    agent.tools = vec!["bash".into(), "read".into()];
    let items = corpus(&agent, &[], &[]);
    for item in &items {
        assert_ne!(item.description, "instrumenta", "{item:?}");
    }
    let bash = items.iter().find(|item| item.name == "bash").unwrap();
    assert!(!bash.description.is_empty(), "{bash:?}");
}

#[test]
fn extension_ui_calls_are_taken_rather_than_replayed_each_turn() {
    let host = Arc::new(Mutex::new(crate::extension_host::ExtensionHost::default()));
    host.lock().unwrap().ui_calls = vec![serde_json::json!({
        "op": "notify",
        "message": "index rebuilt",
    })];
    let mut model = Model::new(
        davinci_tui::davinci::theme::Theme::da_vinci(
            davinci_tui::davinci::theme::ColorDepth::TrueColor,
            false,
        ),
        100,
        44,
        true,
    );
    drain_ui_calls(&mut model, &host);
    let notices = |model: &Model| {
        model
            .transcript
            .iter()
            .filter(
                |entry| matches!(entry, Entry::Tool { target, .. } if target == "index rebuilt"),
            )
            .count()
    };
    assert_eq!(notices(&model), 1);

    // The host is shared across every turn, so reading the queue without
    // taking it said the same thing again on each one.
    drain_ui_calls(&mut model, &host);
    assert_eq!(notices(&model), 1);
}

#[test]
fn a_resumed_session_opens_where_it_left_off() {
    let messages = vec![
        assistant("first reply"),
        davinci_ai::ChatMessage {
            role: "user".into(),
            content: vec![davinci_ai::MessageContent::Text {
                text: "and then?".into(),
            }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            extra: Default::default(),
        },
    ];
    let entries = transcript_from(&messages);
    assert!(matches!(&entries[0], Entry::Agent(name) if name == "davinci"));
    assert!(matches!(&entries[2], Entry::Prose(text) if text == "first reply"));
    assert!(matches!(&entries[4], Entry::User(text) if text == "and then?"));
}

#[test]
fn a_resumed_session_keeps_the_calls_the_turn_made() {
    let messages = vec![
        davinci_ai::ChatMessage {
            role: "assistant".into(),
            content: vec![
                davinci_ai::MessageContent::Text {
                    text: "looking".into(),
                },
                davinci_ai::MessageContent::ToolCall {
                    id: "call-1".into(),
                    name: "read".into(),
                    arguments: json!({"path": "src/lib.rs"}),
                },
                davinci_ai::MessageContent::ToolCall {
                    id: "call-2".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "cargo test"}),
                },
            ],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            extra: Default::default(),
        },
        davinci_ai::ChatMessage {
            role: "tool".into(),
            content: vec![davinci_ai::MessageContent::Text {
                text: "one\ntwo\nthree".into(),
            }],
            tool_call_id: Some("call-1".into()),
            tool_name: Some("read".into()),
            is_error: None,
            extra: Default::default(),
        },
        davinci_ai::ChatMessage {
            role: "tool".into(),
            content: vec![davinci_ai::MessageContent::Text {
                text: "error[E0308] mismatched types".into(),
            }],
            tool_call_id: Some("call-2".into()),
            tool_name: Some("bash".into()),
            is_error: Some(true),
            extra: Default::default(),
        },
    ];

    let entries = transcript_from(&messages);
    let read = entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Tool {
                state,
                target,
                summary,
                ..
            } if target.starts_with("read ") => Some((*state, summary.clone())),
            _ => None,
        })
        .expect("the read is drawn");
    assert_eq!(read, (State::Read, Some("3 lines".to_string())));

    let failed = entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Tool {
                state,
                target,
                summary,
                output,
                ..
            } if target == "cargo test" => Some((*state, summary.clone(), output.clone())),
            _ => None,
        })
        .expect("the command is drawn");
    assert_eq!(failed.0, State::Failed);
    assert_eq!(failed.1, None, "a failure states no outcome");
    assert!(
        failed.2.iter().any(|line| line.contains("E0308")),
        "the failure rides on the tool line: {:?}",
        failed.2
    );
    assert!(
        !entries
            .iter()
            .any(|entry| matches!(entry, Entry::Detail(_))),
        "a resumed failure is not a Detail sibling"
    );
}

#[test]
fn an_outcome_is_stated_in_the_fewest_words_that_say_it() {
    assert_eq!(
        summary_of("grep", &json!({"pattern": "x"}), &json!("a\nb")),
        Some("2 matches".into())
    );
    assert_eq!(
        summary_of("grep", &json!({"pattern": "x"}), &json!("")),
        Some("0 matches".into())
    );
    assert_eq!(
        summary_of("read", &json!({"path": "a"}), &json!("only")),
        Some("1 line".into())
    );
    assert_eq!(
        summary_of(
            "edit",
            &json!({"edits": [{"oldText": "a\nb", "newText": "a\nb\nc\nd"}]}),
            &json!("Edited a")
        ),
        Some("+4 -2".into())
    );
    assert_eq!(summary_of("ask", &json!({}), &json!("whatever")), None);
    assert_eq!(
        summary_of(
            "todo",
            &json!({"items": [
                {"text": "survey", "status": "completed"},
                {"text": "edit", "status": "in_progress"}
            ]}),
            &json!("ok")
        ),
        Some("1 of 2 done".into())
    );
    assert_eq!(
        summary_of(
            "web_search",
            &json!({"query": "ratatui"}),
            &json!("1. ratatui.rs\n2. docs.rs/ratatui")
        ),
        Some("2 results".into())
    );
}

#[test]
fn an_empty_session_opens_on_the_empty_state() {
    assert!(transcript_from(&[]).is_empty());
    let blank = assistant("   ");
    assert!(transcript_from(&[blank]).is_empty());
}

#[test]
fn durations_read_in_seconds() {
    assert_eq!(duration_of(Duration::from_millis(1_840)), "1.84s");
    assert_eq!(duration_of(Duration::from_millis(420)), "0.42s");
}

#[test]
fn an_edit_draws_its_delta_from_the_details_the_tool_returned() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "1".into(),
            tool_name: "edit".into(),
            args: json!({"path": "src/lib.rs"}),
        },
    );
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionEnd {
            tool_call_id: "1".into(),
            tool_name: "edit".into(),
            result: json!("Edited src/lib.rs"),
            is_error: false,
            details: Some(json!({
                "path": "src/lib.rs",
                "diff": "+ 1 pub fn foo() {\n- 1 pub fn bar() {\n"
            })),
        },
    );
    assert!(matches!(
        &m.transcript[0],
        Entry::Tool {
            state: State::Delta,
            ..
        }
    ));
    assert!(matches!(m.transcript[1], Entry::Gap));
    match &m.transcript[2] {
        Entry::Delta {
            path,
            adds,
            dels,
            hunks,
        } => {
            assert_eq!(path, "src/lib.rs");
            assert_eq!((*adds, *dels), (1, 1));
            assert_eq!(hunks[0].kind, HunkKind::Add);
            assert!(hunks[0].text.contains("foo"));
            assert_eq!(hunks[1].kind, HunkKind::Del);
            assert!(hunks[1].text.contains("bar"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_todo_call_becomes_the_studio_ledger_and_the_plan_sheet() {
    let mut m = model();
    let mut turn = Turn::default();
    apply(
        &mut m,
        &mut turn,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "1".into(),
            tool_name: "todo".into(),
            args: json!({"items": [
                {"text": "survey", "status": "completed"},
                {"text": "edit", "status": "in_progress"},
                {"text": "test", "status": "pending"}
            ]}),
        },
    );
    assert!(
        m.transcript
            .iter()
            .any(|entry| matches!(entry, Entry::Studio(steps) if steps.len() == 3)),
        "{:?}",
        m.transcript
    );
    assert_eq!(m.plan.len(), 3);
    assert_eq!(m.plan[0].state, State::Done);
    assert_eq!(m.plan[1].state, State::Active);
    assert_eq!(m.plan[2].state, State::Queued);
}

#[test]
fn a_finished_job_is_a_manus_row_with_its_tail_behind_the_line() {
    let ok = davinci_agent::JobNotice {
        id: 1,
        command: "cargo build".into(),
        status: davinci_agent::JobStatus::Exited(0),
        elapsed: Duration::from_millis(31_200),
        tail: vec!["Compiling pi".into(), "Finished".into()],
    };
    match job_row(&ok) {
        Entry::Tool {
            state: State::Done,
            instrument,
            target,
            summary,
            output,
            duration,
        } => {
            assert_eq!(instrument, "manus");
            assert!(target.contains("job 1 finished"), "{target}");
            assert!(target.contains("cargo build"), "{target}");
            assert_eq!(summary.as_deref(), Some("exit 0"));
            assert_eq!(duration.as_deref(), Some("31.2s"));
            assert!(output.iter().any(|line| line.contains("Compiling pi")));
        }
        other => panic!("{other:?}"),
    }
    let fail = davinci_agent::JobNotice {
        id: 2,
        command: "cargo test".into(),
        status: davinci_agent::JobStatus::Exited(1),
        elapsed: Duration::from_millis(400),
        tail: vec!["FAILED".into()],
    };
    match job_row(&fail) {
        Entry::Tool {
            state: State::Failed,
            summary,
            ..
        } => assert_eq!(summary.as_deref(), Some("exit 1")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn new_tools_name_their_instrument_state_verb_and_target() {
    assert_eq!(instrument_of("web_fetch"), "instrumenta");
    assert_eq!(instrument_of("job_output"), "manus");
    assert_eq!(state_of("web_fetch", false), State::Search);
    assert_eq!(state_of("web_search", false), State::Search);
    assert_eq!(state_of("todo", false), State::Done);
    assert_eq!(state_of("job_output", false), State::Read);
    assert_eq!(state_of("notebook_edit", false), State::Delta);
    assert_eq!(verb_of("todo"), "planning");
    assert_eq!(verb_of("web_search"), "surveying");
    assert_eq!(verb_of("notebook_edit"), "constructing");
    assert_eq!(
        target_of("web_fetch", &json!({"url": "https://docs.rs/ratatui/"})),
        "fetch docs.rs/ratatui"
    );
    assert_eq!(
        target_of("web_search", &json!({"query": "myers diff"})),
        "search web \"myers diff\""
    );
    assert_eq!(
        target_of("todo", &json!({"items": [{"text": "a"}, {"text": "b"}]})),
        "plan · 2 items"
    );
    assert_eq!(
        target_of("job_output", &json!({"jobId": 3})),
        "job 3 output"
    );
    assert_eq!(
        target_of("notebook_edit", &json!({"path": "n.ipynb", "cell": 2})),
        "edit n.ipynb · cell 2"
    );
    assert_eq!(instrument_of("mcp_read"), "instrumenta");
    assert_eq!(instrument_of("mcp__memory__echo"), "instrumenta");
    assert_eq!(state_of("mcp_read", false), State::Read);
    assert_eq!(state_of("mcp__memory__echo", false), State::Done);
    assert_eq!(verb_of("mcp_read"), "studying");
    assert_eq!(
        target_of(
            "mcp_read",
            &json!({"server": "memory", "uri": "fixture://note"})
        ),
        "mcp memory fixture://note"
    );
    assert_eq!(
        target_of("mcp__memory__echo", &json!({"text": "hi"})),
        "mcp memory echo hi"
    );
    assert_eq!(verb_of("agent"), "delegating");
    assert_eq!(
        target_of(
            "agent",
            &json!({"prompt": "scan the crate", "description": "survey"})
        ),
        "agent survey"
    );
}

#[test]
fn a_persisted_background_job_replays_as_a_tool_row_not_a_user_echo() {
    let mut extra = serde_json::Map::new();
    extra.insert("customType".into(), json!(davinci_agent::JOB_NOTICE_TYPE));
    extra.insert("jobId".into(), json!(1));
    let notice = davinci_agent::JobNotice {
        id: 1,
        command: "cargo build".into(),
        status: davinci_agent::JobStatus::Exited(0),
        elapsed: Duration::from_millis(31_200),
        tail: vec!["Finished".into()],
    };
    let message = davinci_ai::ChatMessage {
        role: "user".into(),
        content: vec![davinci_ai::MessageContent::Text {
            text: notice.message_text(),
        }],
        extra,
        ..davinci_ai::ChatMessage::default()
    };
    let entries = transcript_from(&[message]);
    match &entries[0] {
        Entry::Tool {
            state: State::Done,
            instrument,
            target,
            summary,
            output,
            ..
        } => {
            assert_eq!(instrument, "manus");
            assert!(target.contains("job 1 finished"), "{target}");
            assert_eq!(summary.as_deref(), Some("exit 0"));
            assert!(output.iter().any(|line| line.contains("Finished")));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        !entries.iter().any(|entry| matches!(entry, Entry::User(_))),
        "{entries:?}"
    );
}

#[test]
fn hunks_from_diff_strips_the_number_column_and_counts() {
    let (adds, dels, hunks) = hunks_from_diff("+12 added\n- 8 removed\n  4 kept\n    ...\n");
    assert_eq!((adds, dels), (1, 1));
    assert_eq!(hunks[0].kind, HunkKind::Add);
    assert_eq!(hunks[0].text, "added");
    assert_eq!(hunks[1].kind, HunkKind::Del);
    assert_eq!(hunks[1].text, "removed");
    assert_eq!(hunks[2].kind, HunkKind::Context);
    assert_eq!(hunks[2].text, "kept");
    assert_eq!(hunks[3].text, "…");
}

#[test]
fn open_resume_sheet_shows_all_sessions_without_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions");
    let work_dir = dir.path().join("work");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::create_dir_all(&work_dir).unwrap();

    // Create 35 sessions (exceeding the old 30-item limit)
    for i in 0..35 {
        davinci_session::JsonlSession::create(
            &session_dir,
            &work_dir.to_string_lossy(),
            Some(&format!("session-{i}")),
        )
        .unwrap();
    }

    let parsed = crate::args::Args {
        session_dir: Some(session_dir.to_string_lossy().into_owned()),
        ..crate::args::Args::default()
    };
    let mut agent = davinci_agent::Agent::new("test");
    agent.cwd = work_dir;

    let mut m = model();
    open_resume_sheet(&parsed, &agent, &mut m);

    assert_eq!(m.screen, Screen::Resume);
    assert_eq!(m.session_count, 35);
    assert_eq!(
        m.resume_sessions.len(),
        35,
        "all 35 sessions must be present without a 30-item truncation limit"
    );
}

#[test]
fn session_commands_classify_to_resume_or_stats() {
    use crate::slash::SlashAction;
    assert!(matches!(
        classify("/session"),
        Sent::Command(SlashAction::Resume)
    ));
    assert!(matches!(
        classify("/sessions"),
        Sent::Command(SlashAction::Resume)
    ));
    assert!(matches!(
        classify("/resume"),
        Sent::Command(SlashAction::Resume)
    ));
    assert!(matches!(
        classify("/session info"),
        Sent::Command(SlashAction::SessionInfo)
    ));
    assert!(matches!(
        classify("/session stats"),
        Sent::Command(SlashAction::SessionInfo)
    ));
}

#[test]
fn test_workflow_commands_and_sheet() {
    let mut agent = davinci_agent::Agent::new("test");
    let bus = davinci_agent::RuntimeBus::new();
    let runtime = davinci_agent::RuntimeHandle::new(
        davinci_agent::RunId::new(),
        davinci_agent::AgentId::new(),
        bus,
    );
    let store = davinci_agent::WorkflowStateStore::new();
    let executor = std::sync::Arc::new(davinci_agent::WorkflowExecutor::new(
        runtime.clone(),
        store,
        None,
    ));
    let runtime = runtime.with_workflow_executor(executor.clone());
    agent.runtime = Some(runtime);

    let mut m = model();
    open_workflows_sheet(&agent, &mut m);
    assert_eq!(m.screen, Screen::Workflows);
    assert!(m.workflows.is_some());
    assert_eq!(m.workflows.unwrap().workflows.len(), 0);
}

#[test]
fn f02_wait_native_decision_timeout() {
    let (tx, rx) = mpsc::channel();
    let abort = AtomicBool::new(false);
    let q = davinci_agent::decisions::DecisionQuestion {
        id: "q-timeout".into(),
        kind: davinci_agent::decisions::DecisionKind::Architecture,
        title: "Timeout Test".into(),
        question: "Will it time out?".into(),
        materiality: "High".into(),
        evidence_refs: vec![],
        evidence_fingerprints: std::collections::BTreeMap::new(),
        options: vec![],
        allow_custom: true,
        custom_only: true,
        plan_revision: 1,
        state: davinci_agent::decisions::DecisionState::Open,
        answer: None,
    };
    let req = davinci_agent::DecisionHostRequest { question: q };
    // Expired deadline
    let expired_ms = davinci_session::now_ms().saturating_sub(10);
    let resp = wait_native_decision(req, expired_ms, &tx, &abort);
    assert_eq!(resp, davinci_agent::DecisionHostResponse::Cancelled);
    assert!(rx.try_recv().is_err());
}

#[test]
fn f02_wait_native_decision_abort_cancelled() {
    let (tx, _rx) = mpsc::channel();
    let abort = AtomicBool::new(true);
    let q = davinci_agent::decisions::DecisionQuestion {
        id: "q-abort".into(),
        kind: davinci_agent::decisions::DecisionKind::Architecture,
        title: "Abort Test".into(),
        question: "Will it cancel?".into(),
        materiality: "High".into(),
        evidence_refs: vec![],
        evidence_fingerprints: std::collections::BTreeMap::new(),
        options: vec![],
        allow_custom: true,
        custom_only: true,
        plan_revision: 1,
        state: davinci_agent::decisions::DecisionState::Open,
        answer: None,
    };
    let req = davinci_agent::DecisionHostRequest { question: q };
    let resp = wait_native_decision(req, davinci_session::now_ms() + 10_000, &tx, &abort);
    assert_eq!(resp, davinci_agent::DecisionHostResponse::Cancelled);
}

#[test]
fn f02_open_decision_modal_mapping() {
    let mut m = model();
    let q = davinci_agent::decisions::DecisionQuestion {
        id: "q-map".into(),
        kind: davinci_agent::decisions::DecisionKind::Behavior,
        title: "Map Test".into(),
        question: "Map Question?".into(),
        materiality: "Medium".into(),
        evidence_refs: vec!["ref1".into()],
        evidence_fingerprints: std::collections::BTreeMap::new(),
        options: vec![davinci_agent::decisions::DecisionOptionInput {
            id: "opt1".into(),
            label: "Option 1".into(),
            explanation: "Expl 1".into(),
            recommended: true,
        }],
        allow_custom: true,
        custom_only: false,
        plan_revision: 3,
        state: davinci_agent::decisions::DecisionState::Open,
        answer: None,
    };
    open_decision_modal(&mut m, &q);
    assert_eq!(m.overlay, Some(Overlay::Ask));
    assert!(m.decision_modal.is_some());
    let modal = m.decision_modal.as_ref().unwrap();
    assert_eq!(modal.question_id, "q-map");
    assert_eq!(modal.title, "Map Test");
    assert_eq!(modal.options.len(), 1);
    assert_eq!(modal.options[0].id, "opt1");
    assert!(modal.options[0].recommended);
    assert_eq!(modal.plan_revision, 3);
}

#[test]
fn f04_open_rewind_modal_mapping() {
    let mut m = model();
    let preview = davinci_agent::runtime::rewind::RewindPreview {
        checkpoint_id: "cp-123".into(),
        preview_digest: "digest-abc".into(),
        files: vec![davinci_agent::runtime::rewind::FileRewindPlan {
            path: "src/main.rs".into(),
            classification: "inverse".into(),
            resolved_content: Some(b"fn main() {}".to_vec()),
            is_conflict: false,
            conflict_reason: None,
            pre_rewind_hash: None,
        }],
        conflict_count: 0,
        irreversible_effects: vec![davinci_agent::runtime::effects::ExternalEffectReceipt {
            receipt_id: "rcpt-1".into(),
            task_id: davinci_agent::TaskId::new(),
            operation_id: "op-1".into(),
            kind: "deploy".into(),
            details: serde_json::json!({
                "secret_token": "secret123",
                "target": "staging"
            }),
            timestamp_ms: 1000,
            reversible: false,
        }],
    };
    open_rewind_modal(&mut m, &preview, "Pre-Task 1", "12:00:00");
    assert_eq!(m.overlay, Some(Overlay::Ask));
    assert!(m.rewind_modal.is_some());
    let modal = m.rewind_modal.as_ref().unwrap();
    assert_eq!(modal.checkpoint_id, "cp-123");
    assert_eq!(modal.checkpoint_name, "Pre-Task 1");
    assert_eq!(modal.checkpoint_time, "12:00:00");
    assert_eq!(modal.preview_digest, "digest-abc");
    assert_eq!(modal.files.len(), 1);
    assert_eq!(modal.files[0].path, "src/main.rs");
    assert_eq!(modal.files[0].classification, "inverse");
    assert_eq!(modal.conflict_count, 0);
    assert_eq!(modal.irreversible_effects.len(), 1);
    assert_eq!(modal.irreversible_effects[0].operation_id, "op-1");
    assert!(modal.irreversible_effects[0].details.contains("[REDACTED]"));
    assert!(!modal.irreversible_effects[0].details.contains("secret123"));
}

#[cfg(test)]
mod terminal_rebuild_host_contracts {
    use super::*;
    #[test]
    fn starting_a_graph_does_not_force_the_canvas_open() {
        assert!(!graph_command_opens_view(
            "graph",
            "Inspect the parser and implement recovery"
        ));
        assert!(!graph_command_opens_view("graph", "run saved-example"));
        assert!(graph_command_opens_view("graph-status", ""));
        assert!(graph_command_opens_view("graph-view", ""));
        assert!(!graph_command_opens_view("model", ""));
    }
}
