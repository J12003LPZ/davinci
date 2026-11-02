//! First run in the reference shell: a welcome band with DaVinci's own art, the
//! theme step, the security notes, and then the shell's existing analytics
//! question (an `Ask`, opened by the host once `enter` ends these steps).
//!
//! Nothing here writes settings and nothing here trusts or allows anything. The
//! theme chosen is only held in `Model::first_run_theme`; the host stores it in
//! the same write that stores the analytics answer, so leaving partway leaves
//! no settings file and the first-run question is asked again next time.
use crate::davinci::{
    app::Flow,
    model::{wrap_index, Choice, Model, OnboardStep},
    ui::{self, span},
    views::{
        startup,
        theme_picker::{self, ENTRIES},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The list stops before DaVinci's own Vox: the first-run list is the shared
/// themes only, and Vox stays one `/theme` away.
pub const LIST: usize = 7;
/// Where the step's own rows begin; the band and art take the rows above.
pub const BODY_TOP: usize = 17;
const BAND: usize = 58;

/// Open the steps on `current` (the stored theme name). Only the reference
/// shell has them: `false` leaves the first run as it was.
pub fn start(model: &mut Model, current: &str) -> bool {
    if !model.reference || !theme_picker::open(model, current) {
        return false;
    }
    model.onboarding = Some(OnboardStep::Theme);
    model.first_run_theme = None;
    true
}

fn leave(model: &mut Model) {
    if let Some(picker) = model.theme_picker.take() {
        model.theme = picker.previous;
    }
    model.onboarding = None;
    model.first_run_theme = None;
}

/// Keys while a step is open. `esc` leaves the steps and writes nothing, as it
/// does on the analytics panel; `ctrl+c` is the shell's interrupt, which the
/// host counts towards quitting, and leaves the step as it was.
pub fn handle_key(model: &mut Model, key: KeyEvent) -> Flow {
    let Some(step) = model.onboarding else {
        return Flow::Continue;
    };
    if matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'c'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return Flow::Interrupt;
    }
    if key.code == KeyCode::Esc {
        leave(model);
        return Flow::Continue;
    }
    match (step, key.code) {
        (OnboardStep::Theme, KeyCode::Up | KeyCode::Down) => {
            if let Some(picker) = model.theme_picker.as_mut() {
                let delta = if key.code == KeyCode::Up { -1 } else { 1 };
                picker.index = wrap_index(picker.index, delta, LIST);
                let previous = picker.previous;
                model.theme = previous.with_name(ENTRIES[picker.index].0);
            }
        }
        (OnboardStep::Theme, KeyCode::Char('t'))
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if let Some(picker) = model.theme_picker.as_mut() {
                picker.syntax = !picker.syntax;
            }
        }
        (OnboardStep::Theme, KeyCode::Enter) => {
            if let Some(picker) = model.theme_picker.as_ref() {
                model.first_run_theme = Some(ENTRIES[picker.index.min(LIST - 1)].0.to_string());
                model.onboarding = Some(OnboardStep::Security);
            }
        }
        (OnboardStep::Security, KeyCode::Enter) => {
            model.theme_picker = None;
            model.onboarding = None;
            return Flow::Choose(Choice::OnboardingDone);
        }
        _ => {}
    }
    Flow::Continue
}

fn text(model: &Model, indent: usize, words: &str, style: Style) -> Line<'static> {
    Line::from(ui::truncate_run(
        vec![
            span(" ".repeat(indent), model.theme.text),
            Span::styled(words.to_string(), style),
        ],
        model.width,
    ))
}

/// Rows 0 to 16: the title, the band of dots, and DaVinci's own painter
/// sprite standing on the lower rows.
fn welcome(model: &Model) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let ink = Style::default().fg(cc.claude);
    let dots = "·".repeat(BAND);
    let mut rows = vec![
        Line::from(vec![
            Span::styled("Welcome to DaVinci", ink),
            Span::styled(
                format!(" v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(cc.inactive),
            ),
        ]),
        Line::from(dots.clone()),
    ];
    rows.resize(10, ui::blank());
    let sprite = startup::sprite_rows(model);
    for row in 0..5 {
        let mut spans = vec![span("     ", model.theme.text)];
        if let Some(art) = sprite.get(row) {
            spans.extend(art.iter().cloned());
        }
        rows.push(Line::from(ui::truncate_run(spans, model.width)));
    }
    rows.push(Line::from(dots));
    rows.push(ui::blank());
    rows.into_iter()
        .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
        .collect()
}

fn theme_rows(model: &Model) -> Vec<Line<'static>> {
    let Some(picker) = &model.theme_picker else {
        return Vec::new();
    };
    let cc = model.theme.cc();
    let quiet = Style::default()
        .fg(cc.inactive)
        .add_modifier(cc.inactive_modifier);
    let mut rows = vec![
        text(model, 1, "Pick a look first.", Style::default()),
        ui::blank(),
        text(
            model,
            1,
            theme_picker::HEADING_SENTENCE,
            Style::default().add_modifier(Modifier::BOLD),
        ),
        text(model, 1, "So switch when wanted via /theme", quiet),
        ui::blank(),
    ];
    rows.extend(theme_picker::entries(model, 1, LIST));
    rows.push(ui::blank());
    rows.extend(theme_picker::sample(model, picker.syntax, 1));
    rows
}

fn security_rows(model: &Model) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let quiet = Style::default().fg(cc.inactive);
    let clip = |spans: Vec<Span<'static>>| Line::from(ui::truncate_run(spans, model.width));
    // `n.` in the quiet ink, then the note's first line in the plain one.
    let note = |number: &str, first: &str| {
        clip(vec![
            span(" ", model.theme.text),
            Span::styled(number.to_string(), quiet),
            Span::raw(format!(" {first}")),
        ])
    };
    let more = |words: &str| {
        clip(vec![
            span("    ", model.theme.text),
            Span::styled(words.to_string(), quiet),
        ])
    };
    let ask = Style::default().fg(cc.permission);
    vec![
        text(
            model,
            1,
            "Security notes:",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        ui::blank(),
        note("1.", "DaVinci can get things wrong."),
        more("You decide what runs: read each change and each command"),
        more("before you approve it, most of all when code will execute."),
        ui::blank(),
        note(
            "2.",
            "Files and web pages can hide instructions for the agent",
        ),
        more("(prompt injection), so open only code and content you trust."),
        ui::blank(),
        clip(vec![
            span(" ", model.theme.text),
            Span::styled("Press ", ask),
            Span::styled("Enter", ask.add_modifier(Modifier::BOLD)),
            Span::styled(" to continue…", ask),
        ]),
    ]
}

/// The whole screen, `height` rows. A screen too short for it drops the
/// top rows, the band and art first, and always keeps the highlighted theme.
pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    let mut rows = welcome(model);
    let selected = match model.onboarding {
        Some(OnboardStep::Theme) => {
            rows.extend(theme_rows(model));
            model
                .theme_picker
                .as_ref()
                .map(|picker| BODY_TOP + 5 + picker.index.min(LIST - 1))
        }
        Some(OnboardStep::Security) => {
            rows.extend(security_rows(model));
            None
        }
        None => None,
    };
    let mut rows: Vec<Line<'static>> = rows
        .into_iter()
        .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
        .collect();
    if rows.len() > height {
        let mut start = rows.len() - height;
        if let Some(selected) = selected {
            start = start.min(selected);
        }
        rows = rows.split_off(start);
        rows.truncate(height);
    }
    rows
}
