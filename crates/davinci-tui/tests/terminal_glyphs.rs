//! Every glyph a screen draws must be one Windows Terminal's default font can
//! draw. A glyph Cascadia Mono lacks falls back to a symbol font whose cells
//! are wider than the grid, so dense figures (the `/context` grid was the
//! case that bit) turn into overlapping boxes.
use std::collections::{BTreeMap, BTreeSet};

use davinci_tui::davinci::{
    app, fixtures,
    model::{
        ContextCategory, ContextKind, ContextSection, ContextUsageView, Entry, Model, Overlay,
        Screen,
    },
    theme::{ColorDepth, Theme},
};

/// Single glyphs inherited from Claude Code that Cascadia Mono lacks. Each
/// stands alone in its row, so the fallback is legible; a new entry here
/// needs the same argument.
const FALLBACK_OK: &[char] = &[
    '⎿', // transcript elbow
    '⏸', // permission mode in the footer
    '↳', // sub-task marker
    '⌕', // search box
    '✢', '✶', '✻', '✽', // the working spinner
];

fn cascadia_mono() -> Vec<(u32, u32)> {
    include_str!("fixtures/cascadia_mono.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let (low, high) = line.split_once('-').unwrap();
            (
                u32::from_str_radix(low, 16).unwrap(),
                u32::from_str_radix(high, 16).unwrap(),
            )
        })
        .collect()
}

fn context_usage() -> ContextUsageView {
    let category = |kind, label: &str, tokens| ContextCategory {
        kind,
        label: label.into(),
        tokens,
    };
    ContextUsageView {
        model: "gpt-5.6-luna".into(),
        window: 272_000,
        categories: vec![
            category(ContextKind::SystemPrompt, "System prompt", 1_000),
            category(ContextKind::SystemTools, "System tools", 3_400),
            category(ContextKind::McpTools, "MCP tools", 2_100),
            category(ContextKind::CustomAgents, "Custom agents", 19_000),
            category(ContextKind::MemoryFiles, "Memory files", 900),
            category(ContextKind::Messages, "Messages", 200),
        ],
        free: 109_400,
        buffer: 136_000,
        sections: vec![ContextSection {
            title: "Custom agents".into(),
            command: Some("/agents".into()),
            items: (0..80)
                .map(|index| (format!("agent-{index:02}"), 40 + index as u64))
                .collect(),
        }],
    }
}

/// Every fixture screen, every sheet and overlay, and `/context`.
fn screens(width: u16, height: u16, theme: &str) -> Vec<(String, Model)> {
    let fresh = || {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false).with_name(theme),
            width,
            height,
            false,
        );
        m.width = width;
        m.height = height;
        m
    };
    let mut out = Vec::new();
    for id in [
        "1a",
        "1b",
        "1c",
        "1d",
        "1e",
        "1f",
        "1f-cogitator",
        "1g",
        "1h",
        "2a",
        "2b",
        "2c",
        "3a",
        "3b",
        "3c",
        "3d",
        "3e",
        "4a",
        "4b",
        "4c",
        "4d",
        "5a",
        "blueprint",
        "command-center",
        "5b",
        "5c",
        "5d",
        "6a",
        "6b",
        "6c",
        "6d",
        "plugin",
    ] {
        let mut m = fresh();
        fixtures::dress_screen(&mut m, id);
        out.push((id.to_string(), m));
    }
    for screen in [
        Screen::Mcp,
        Screen::Permissions,
        Screen::Workflows,
        Screen::TaskBoard,
        Screen::Agents,
        Screen::ContextInspector,
        Screen::Extensions,
    ] {
        let mut m = fresh();
        fixtures::dress(&mut m);
        m.task_board = Some(fixtures::task_board_fixture());
        m.context_inspector = Some(fixtures::context_inspector_fixture());
        m.extension_manager = Some(fixtures::extension_manager());
        m.screen = screen;
        out.push((format!("{screen:?}"), m));
    }
    for overlay in [
        Overlay::Instrumenta,
        Overlay::Sessions,
        Overlay::Cogitator,
        Overlay::Ask,
    ] {
        let mut m = fresh();
        fixtures::dress(&mut m);
        m.toggle_overlay(overlay);
        out.push((format!("{overlay:?}"), m));
    }
    let mut m = fresh();
    fixtures::dress_screen(&mut m, "1a");
    m.transcript.push(Entry::User("/context".into()));
    m.transcript.push(Entry::ContextUsage(context_usage()));
    out.push(("/context".into(), m));
    // A fresh session: no sections, so the grid is the last thing drawn.
    let mut m = fresh();
    fixtures::dress_screen(&mut m, "1a");
    let mut fresh_usage = context_usage();
    fresh_usage.sections.clear();
    m.transcript.push(Entry::User("/context".into()));
    m.transcript.push(Entry::ContextUsage(fresh_usage));
    out.push(("/context fresh".into(), m));
    out
}

#[test]
fn every_screen_draws_only_glyphs_cascadia_mono_has() {
    let ranges = cascadia_mono();
    let drawable = |ch: char| {
        let cp = ch as u32;
        cp <= 0x7e
            || ranges
                .iter()
                .any(|(low, high)| (*low..=*high).contains(&cp))
    };
    let mut missing: BTreeMap<char, BTreeSet<String>> = BTreeMap::new();
    for theme in ["dark", "light", "vox"] {
        for (width, height) in [(80, 24), (120, 40)] {
            for (name, m) in screens(width, height, theme) {
                for row in app::compose(&m, height) {
                    for ch in row.to_string().chars() {
                        if !drawable(ch) && !FALLBACK_OK.contains(&ch) {
                            missing.entry(ch).or_default().insert(name.clone());
                        }
                    }
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "glyphs Cascadia Mono cannot draw: {:?}",
        missing
            .iter()
            .map(|(ch, screens)| format!("U+{:04X} {ch} in {screens:?}", *ch as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn context_grid_and_legend_stay_on_screen_with_many_agents() {
    // 40 rows is the screenshot that bit: 80 agents pushed the whole grid off.
    // At 24 rows the grid alone outgrows the transcript, so it scrolls.
    for (width, height) in [(120, 40), (160, 40)] {
        let screens = screens(width, height, "dark");
        let (_, m) = screens.iter().find(|(name, _)| name == "/context").unwrap();
        let text: Vec<String> = app::compose(m, height)
            .iter()
            .map(ToString::to_string)
            .collect();
        let at = |needle: &str| text.iter().position(|row| row.contains(needle));
        let heading = at("Context Usage");
        assert!(heading.is_some(), "{width}x{height}:\n{}", text.join("\n"));
        assert!(at("Free space").is_some(), "{width}x{height}");
        assert!(at("72 more").is_some(), "{width}x{height}");
    }
}

#[test]
fn the_allowlist_holds_only_glyphs_the_font_lacks() {
    let ranges = cascadia_mono();
    for ch in FALLBACK_OK {
        let cp = *ch as u32;
        assert!(
            !ranges
                .iter()
                .any(|(low, high)| (*low..=*high).contains(&cp)),
            "{ch} is in Cascadia Mono; drop it from FALLBACK_OK"
        );
    }
}
