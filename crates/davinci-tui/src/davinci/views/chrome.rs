//! AppShell: header, composer, status bar (design.md §6, screen `1b`).
//!
//! Header and status bar are one line each at every width; both abbreviate
//! rather than wrap. Neutral rules frame the composer; the prompt and caret
//! carry focus, and muted keybind hints remain readable below it.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/chrome.ex`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::davinci::model::{Model, Overlay, Screen};
use crate::davinci::theme::glyph;
use crate::davinci::ui::{
    clip_ellipsis, meter, pad, paper_label, run_width, span, span_strong, spread, Surface,
};

use super::sheet::{self, Composer};

/// Examples shown in an empty conversation composer, one per session.
pub const PLACEHOLDERS: [&str; 3] = [
    "fix lint errors",
    "fix typecheck errors",
    "refactor <filepath>",
];

/// Which hints sit under the composer. Every panel states its own exits (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    Default,
    Multiline,
    /// A sheet is open: the composer still sends, and esc closes the sheet.
    Closable,
    None,
}

/// `✻ davinci · agent` on the left, `path │ branch │ model` on the right.
/// Memoria recall and Mensura claim the right run for their own facts, as the
/// mockups set them (`2b`, `2c`).
pub fn header(model: &Model) -> Line<'static> {
    let th = &model.theme;
    if let Some(section) = sheet::chrome(model) {
        return spread(
            model.width,
            vec![paper_label(sheet::title(model.screen), th, false)],
            section.header_right,
        );
    }
    let mut left = vec![paper_label("DaVinci", th, true)];
    if !model.minimal() {
        left.push(span(" · ", th.border));
        left.push(span(model.mode().to_uppercase(), th.text));
    }

    // A command sheet claims the right run for its own facts (design.md §11).
    if let Some(chrome) = sheet::chrome(model) {
        if !chrome.header_right.is_empty() {
            return spread(model.width, left, chrome.header_right);
        }
    }

    let right = if model.minimal() {
        vec![
            span(short_cwd(&model.cwd), th.muted),
            span(" │ ", th.border),
            span(model.branch.clone(), th.secondary),
        ]
    } else if model.screen == Screen::Memoria {
        let meta = &model.recall_meta;
        vec![
            span(format!("{} vectors", meta.vectors), th.muted),
            span(" │ ", th.border),
            span(format!("{} shards", meta.shards), th.muted),
            span(" │ ", th.border),
            span(meta.embedding.clone(), th.muted),
        ]
    } else if model.screen == Screen::Mensura {
        let meta = &model.budget_meta;
        vec![
            span("policy ", th.muted),
            span(meta.policy.clone(), th.text),
            span(" │ ", th.border),
            span(format!("window {}", meta.window), th.muted),
            span(" │ ", th.border),
            span(model_run(model), th.muted),
        ]
    } else {
        let mut right = vec![
            span(short_cwd(&model.cwd), th.muted),
            span(" │ ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" │ ", th.border),
            span(model_run(model), th.muted),
        ];
        if model.codex_open() {
            right.push(span(" │ ", th.border));
            right.push(span(format!("{}×{}", model.width, model.height), th.border));
        }
        right
    };

    spread(model.width, left, right)
}

/// `mode · branch · Δn +a -d` on the left, a context meter on the right.
pub fn status(model: &Model) -> Line<'static> {
    if model.permission_mode == "always-approve"
        || (model.screen == Screen::Agent && model.overlay.is_none() && !model.codex_open())
    {
        // Keep the no-prompts warning visible even while a sheet owns input.
        return conversation_status(model);
    }
    spread(model.width, status_left(model), status_right(model))
}

/// One quiet footer. Permissions remain explicit even on narrow terminals.
fn conversation_status(model: &Model) -> Line<'static> {
    let cc = model.theme.cc();
    // The warning takes priority over padding and cycle hints on narrow screens.
    if model.permission_mode == "always-approve" && model.width < 36 {
        let label = if model.width >= 28 {
            "always approve · no prompts"
        } else {
            "no prompts"
        };
        return Line::from(super::super::ui::truncate_run(
            vec![span(label, cc.error)],
            model.width,
        ));
    }
    if model.exit_armed {
        return Line::from(span("  Press Ctrl-C again to exit", cc.inactive));
    }
    if model.screen == Screen::Agent && model.composer.starts_with('!') {
        return Line::from(vec![
            span("  ", cc.inactive),
            span("! for shell mode", cc.bash),
        ]);
    }

    let cycle = key_label(model, "app.permissions.cycle").unwrap_or_else(|| "shift+tab".into());
    let cycle_note = format!(" ({cycle} to cycle)");
    let (label, color, tail) = match model.permission_mode.as_str() {
        "edits" => ("⏵⏵ accept edits on", cc.accept_edits, cycle_note),
        "read-only" => ("⏸ plan mode on", cc.plan_mode, cycle_note),
        "auto" => ("⏵⏵ auto mode on", cc.auto_mode, cycle_note),
        "always-approve" => ("⏵⏵ always approve on · no prompts", cc.error, cycle_note),
        _ => ("⏸ manual mode on", cc.inactive, String::new()),
    };
    let mut left = vec![span("  ", cc.inactive), span(label, color)];
    if !tail.is_empty() {
        left.push(span(tail, cc.inactive));
    }
    if model.running {
        left.push(span(" · esc to interrupt", cc.inactive));
    } else if !matches!(
        model.permission_mode.as_str(),
        "edits" | "read-only" | "auto" | "always-approve"
    ) {
        left.push(span(" · ? for shortcuts", cc.inactive));
    }
    let agents = model.agents.as_ref().map_or(0, |sheet| sheet.agents.len());
    if agents > 0 {
        let noun = if agents == 1 { "agent" } else { "agents" };
        left.push(span(format!(" · ← {agents} {noun}"), cc.inactive));
    }
    if !model.voice.notice.is_empty() {
        left.push(span(format!(" · {}", model.voice.notice), cc.inactive));
    }
    if let Some(jobs) = jobs_note(model) {
        left.push(span(format!(" · {jobs}"), cc.inactive));
    }
    let right = if model.context_fraction() >= 0.8 && model.width >= 80 {
        vec![span(
            format!("{}% context ", (model.context_fraction() * 100.0) as u32),
            cc.auto_mode,
        )]
    } else {
        Vec::new()
    };
    spread(model.width, left, right)
}

fn key_label(model: &Model, action: &str) -> Option<String> {
    model
        .keybindings
        .keys_for(action)
        .first()
        .map(|key| key.to_string())
}

/// Rows below the conversation composer: the regular status or the shortcuts panel.
pub fn footer(model: &Model) -> Vec<Line<'static>> {
    if model.shortcuts_open && model.screen == Screen::Agent && model.overlay.is_none() {
        return shortcut_rows(model);
    }
    vec![status(model)]
}

fn shortcut_rows(model: &Model) -> Vec<Line<'static>> {
    let quiet = model.theme.cc().inactive;
    let spaced = |action: &str, text: &str| {
        key_label(model, action).map(|key| format!("{} {text}", key.replace('+', " + ")))
    };
    let columns: [Vec<String>; 3] = [
        vec![
            "! for shell mode".into(),
            "/ for commands".into(),
            "@ for file paths".into(),
        ],
        [
            spaced("app.permissions.cycle", "to cycle modes"),
            spaced("davinci.tools.expand", "for verbose output"),
            spaced("davinci.composer.newLine", "for newline"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        [
            spaced("app.model.select", "to switch model"),
            spaced("app.editor.external", "to edit in $EDITOR"),
            Some("/hotkeys to customize".to_string()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    ];
    if model.width < 100 {
        return columns
            .iter()
            .flatten()
            .map(|text| Line::from(span(format!("  {text}"), quiet)))
            .collect();
    }
    let height = columns.iter().map(Vec::len).max().unwrap_or(0);
    (0..height)
        .map(|row| {
            let cell = |column: usize| columns[column].get(row).cloned().unwrap_or_default();
            Line::from(span(
                format!("  {:<24}{:<35}{}", cell(0), cell(1), cell(2)),
                quiet,
            ))
        })
        .collect()
}

/// Quiet, focus-preserving feedback above the composer; the ledger stays
/// available after the notice expires, including with animation disabled.
pub fn governor_notice(model: &Model) -> Vec<Line<'static>> {
    let Some((message, when)) = &model.governor_notice else {
        return Vec::new();
    };
    if when.elapsed() >= std::time::Duration::from_secs(8) || model.height < 12 {
        return Vec::new();
    }
    vec![spread(
        model.width,
        vec![
            span_strong("✓ Governor  ", model.theme.success, &model.theme),
            span(message.clone(), model.theme.text),
        ],
        if model.width >= 100 {
            vec![span(" /governor-status", model.theme.muted)]
        } else {
            Vec::new()
        },
    )]
}

fn status_left(model: &Model) -> Vec<Span<'static>> {
    let th = &model.theme;
    let (delta, adds, dels) = model.changes;

    // With an instrument floating over the transcript, the status bar
    // abbreviates and dims behind it (`1d`, `1f`).
    if model.overlay.is_some() {
        return vec![
            span(model.mode(), th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{}{delta}", glyph::DELTA), th.primary),
        ];
    }

    // A section reports its own state, not the underlying conversation mode.
    if let Some(chrome) = sheet::chrome(model) {
        let mut run = vec![span("  ", th.muted)];
        run.extend(
            chrome
                .status_third
                .unwrap_or_else(|| vec![span(sheet::title(model.screen), th.muted)]),
        );
        return run;
    }

    match model.screen {
        Screen::Grafo => vec![
            span("grafo", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span("impact view", th.muted),
        ],
        Screen::Memoria => {
            let meta = &model.recall_meta;
            vec![
                span("memoria", th.secondary),
                span(" · ", th.border),
                span(format!("recall {} of {}", meta.k, meta.vectors), th.muted),
            ]
        }
        Screen::Mensura => {
            let mut left = vec![
                span("mensura", th.warning),
                span(" · ", th.border),
                span(model.branch.clone(), th.secondary),
            ];
            if model.proposal.is_some() {
                left.push(span(" · ", th.border));
                left.push(span("1 proposal", th.muted));
            }
            left
        }
        Screen::Plan => vec![
            span("plan", th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{} steps", model.plan.len()), th.muted),
        ],
        Screen::Agent if model.minimal() => vec![
            span(model.branch.clone(), th.secondary),
            span(" · ", th.border),
            span(format!("{}{delta}", glyph::DELTA), th.primary),
        ],
        // Before anything has changed there is no Δ to count; the model in
        // hand takes its place (`1a`).
        Screen::Agent if delta == 0 => {
            let mut left = vec![
                span(model.mode(), th.primary),
                span(" · ", th.border),
                span(model.branch.clone(), th.secondary),
                span(" · ", th.border),
                span(model_run(model), th.muted),
            ];
            if let Some(jobs) = jobs_note(model) {
                left.push(span(" · ", th.border));
                left.push(span(jobs, th.muted));
            }
            left
        }
        Screen::Agent => {
            let mut left = vec![
                span(model.mode(), th.primary),
                span(" · ", th.border),
                span(model.branch.clone(), th.secondary),
                span(" · ", th.border),
                span(format!("{}{delta} ", glyph::DELTA), th.primary),
                span(format!("+{adds} "), th.success),
                span(format!("-{dels}"), th.error),
            ];
            if model.codex_open() {
                left.push(span(" · ", th.border));
                left.push(span("codex open", th.muted));
            }
            if let Some(jobs) = jobs_note(model) {
                left.push(span(" · ", th.border));
                left.push(span(jobs, th.muted));
            }
            left
        }
        // The command-opened sheets (`3a`–`6d`): the mode word carries the
        // instrument, the branch stays for orientation.
        _ => vec![
            span(model.mode(), th.primary),
            span(" · ", th.border),
            span(model.branch.clone(), th.secondary),
        ],
    }
}

fn status_right(model: &Model) -> Vec<Span<'static>> {
    let th = &model.theme;
    let fraction = model.context_fraction();
    // Truncated, not rounded: a meter must never claim a cap has been reached
    // before it has (47k of 200k reads 23%, screen `1g`).
    let percent = (fraction * 100.0) as u32;
    let (used, cap) = model.context;

    // Section keyboard guidance belongs to its actual input owner, not the
    // conversation footer or historical instrument mockups.
    if model.overlay.is_none() {
        if let Some(section) = sheet::chrome(model) {
            return section.status_right.unwrap_or_default();
        }
    }

    // Behind an overlay the meter abbreviates; the pickers also state the
    // shared exit here rather than each growing a footer row (`1d`, `1f`).
    match model.overlay {
        Some(Overlay::Instrumenta) => {
            return vec![span(
                format!("{}/{}", thousands(used), thousands(cap)),
                th.muted,
            )];
        }
        Some(_) => {
            return vec![
                span("context ", th.muted),
                span(th.pie(fraction), th.primary),
                span(format!(" {percent}%"), th.muted),
                span(" · ", th.border),
                span("esc close", th.border),
            ];
        }
        None => {}
    }

    // A command sheet with a meter of its own (§11).
    if let Some(chrome) = sheet::chrome(model) {
        if let Some(right) = chrome.status_right {
            return right;
        }
    }

    if model.minimal() {
        // Still a meter, never a bare number (design.md §6, §9).
        return vec![
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("^p", th.border),
        ];
    }

    if model.narrow() {
        return vec![
            span("context ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("^p", th.border),
        ];
    }

    // The empty state points at the palette instead of metering nothing (`1a`).
    if model.screen == Screen::Agent && model.transcript.is_empty() && !model.codex_open() {
        return vec![
            span("context ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("ctrl+alt+p commands", th.muted),
        ];
    }

    // The plan sheet reads its budget as a proportion first (`1c`).
    if model.screen == Screen::Plan {
        return vec![
            span("context ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span(format!("{}/{}", thousands(used), thousands(cap)), th.muted),
        ];
    }

    let mut right = vec![span("context ", th.muted)];
    right.extend(meter(fraction, 12, th, None));
    right.push(span(
        format!(" {}/{}", thousands(used), thousands(cap)),
        th.muted,
    ));
    if model.codex_open() {
        right.push(span(" · ", th.border));
        right.push(span("ctrl+p", th.border));
    }
    right
}

/// Claude-style effort affordance shared by the conversation composer and
/// command sheets. The session's real effort label is preserved.
pub fn effort_line(model: &Model) -> Line<'static> {
    let quiet = model.theme.cc().inactive;
    spread(
        model.width,
        Vec::new(),
        vec![
            span("● ", quiet),
            span(model.thinking_level.to_lowercase(), quiet),
            span(" · /effort  ", quiet),
        ],
    )
}

pub fn effort_rule(model: &Model) -> Line<'static> {
    let cc = model.theme.cc();
    let label = format!(" ● {} · /effort ▔", model.thinking_level.to_lowercase());
    let label_width = UnicodeWidthStr::width(label.as_str()).min(model.width as usize);
    let left = "▔".repeat((model.width as usize).saturating_sub(label_width));
    Line::from(crate::davinci::ui::truncate_run(
        vec![span(left, cc.permission), span(label, cc.inactive)],
        model.width,
    ))
}
/// Rows for the composer plus its hint row. Grows with content. The box takes
/// two neutral rules and no side borders. The prompt and caret carry
/// focus; under an open instrument the theme dims. Recall replaces the input
/// with its own keys (`2b`).
pub fn composer(model: &Model, lines: Option<&[String]>, hint: Hint) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let editor = model.composer.editor();
    editor.set_layout_width(usize::MAX);
    editor.set_terminal_rows(model.height as usize);

    if model.screen == Screen::Memoria {
        let keys = if model.minimal() {
            "enter pin · r reindex · esc close"
        } else {
            "enter pin to context · f raise floor · r reindex · esc close"
        };
        return Surface::new(model.width, th)
            .row(vec![
                span(format!("{} ", glyph::PROMPT), th.secondary),
                span(keys, th.muted),
            ])
            .lines();
    }

    let owned = lines.map(<[String]>::to_vec);
    let entries: Vec<String> =
        owned.unwrap_or_else(|| model.composer.split('\n').map(str::to_string).collect());
    let last = entries.len().saturating_sub(1);
    let overlaid = !model.composer_owns_focus();
    let shell_mode = model.screen == Screen::Agent && model.composer.starts_with('!');
    let border = if shell_mode {
        cc.bash
    } else {
        cc.prompt_border
    };
    let placeholder = sheet::chrome(model)
        .and_then(|chrome| match chrome.composer {
            Composer::Prompt(text) => Some(text.to_string()),
            Composer::PromptOwned(text) => Some(text),
            Composer::Hidden | Composer::Disabled(_) => None,
        })
        .or_else(|| screen_placeholder(model.screen).map(str::to_string))
        .or_else(|| {
            (model.screen == Screen::Agent && lines.is_none()).then(|| {
                format!(
                    "Try \"{}\"",
                    PLACEHOLDERS[model.placeholder % PLACEHOLDERS.len()]
                )
            })
        });

    let lit = model.blink();
    let caret_color = if th.text == Color::Reset {
        th.primary
    } else {
        th.text
    };
    let caret_style = if lit {
        Style::default().bg(caret_color).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };
    let caret_at = if lines.is_none() && !overlaid {
        Some(model.composer.editor().get_cursor())
    } else {
        None
    };
    let caret_row = caret_at.map_or(last, |(row, _)| row.min(last));
    let visible = (model.height as usize / 3).clamp(1, 8);
    let start = caret_row
        .saturating_sub(visible - 1)
        .min(entries.len().saturating_sub(visible));
    let end = (start + visible).min(entries.len());
    let mut rows = if model.screen == Screen::Agent && model.overlay.is_none() {
        vec![
            effort_line(model),
            composer_rule(model, border, start, "above"),
        ]
    } else {
        vec![composer_rule(model, border, start, "above")]
    };

    for (index, raw_entry) in entries.into_iter().enumerate().take(end).skip(start) {
        let ink = th.text;
        let mut column = caret_at
            .filter(|(row, _)| *row == index)
            .map(|(_, col)| col);
        let entry = if shell_mode && index == 0 {
            column = column.map(|col| col.saturating_sub(1));
            raw_entry
                .strip_prefix('!')
                .unwrap_or(&raw_entry)
                .to_string()
        } else {
            raw_entry
        };
        let (shown, column) = composer_view(&entry, column, model.width.saturating_sub(6));
        let split = column.and_then(|col| split_at_caret(&shown, col));
        let caret_here = split.is_some();
        let body = if entry.is_empty() {
            let mut hint_span = span(placeholder.clone().unwrap_or_default(), th.text);
            hint_span.style = hint_span.style.add_modifier(Modifier::DIM);
            vec![hint_span]
        } else if let Some((before, under, after)) = split {
            vec![
                span(before, ink),
                if lit {
                    Span::styled(under, caret_style)
                } else {
                    span(under, ink)
                },
                span(after, ink),
            ]
        } else {
            vec![span(shown, ink)]
        };
        let prompt = if index == start {
            let glyph_text = if shell_mode { "!" } else { glyph::PROMPT };
            format!("{glyph_text}\u{a0}")
        } else {
            "  ".into()
        };
        let prompt_color = if overlaid {
            th.border
        } else if shell_mode {
            cc.bash
        } else if model.running {
            cc.inactive
        } else {
            th.text
        };
        let mut run = vec![span(prompt, prompt_color)];
        run.extend(body);
        if index == caret_row && !overlaid && !caret_here {
            if lit {
                run.push(Span::styled(" ", caret_style));
            } else {
                run.push(Span::raw(" "));
            }
        }
        if index == 0 && !shell_mode {
            if let Some(name) = known_command(model, &entry) {
                run = recolor_leading(run, 2 + 1 + name.chars().count(), cc.permission);
                let spec = model.slash_commands.iter().find(|spec| spec.name == name);
                if entry == format!("/{name} ") {
                    if let Some(argument_hint) = spec.and_then(|spec| spec.argument_hint.clone()) {
                        run.push(span(argument_hint, cc.inactive));
                    }
                }
            }
        }
        rows.push(Line::from(crate::davinci::ui::truncate_run(
            run,
            model.width,
        )));
    }

    let rows_typed = last + 1;
    rows.push(composer_rule(model, border, last + 1 - end, "below"));
    if hint != Hint::None {
        rows.push(hint_line(model, hint, rows_typed));
    }
    rows
}

fn known_command(model: &Model, entry: &str) -> Option<String> {
    let word = entry.strip_prefix('/')?.split_whitespace().next()?;
    model
        .slash_commands
        .iter()
        .any(|spec| spec.name == word)
        .then(|| word.to_string())
}

fn recolor_leading(spans: Vec<Span<'static>>, cells: usize, color: Color) -> Vec<Span<'static>> {
    let mut left = cells;
    let mut out = Vec::with_capacity(spans.len() + 1);
    for (index, mut item) in spans.into_iter().enumerate() {
        if index == 0 || left == 0 {
            left = left.saturating_sub(UnicodeWidthStr::width(item.content.as_ref()));
            out.push(item);
            continue;
        }
        let text = item.content.to_string();
        let width = UnicodeWidthStr::width(text.as_str());
        if width <= left {
            left -= width;
            item.style = item.style.fg(color);
            out.push(item);
            continue;
        }
        let mut split = 0;
        let mut used = 0;
        for (at, ch) in text.char_indices() {
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + width > left {
                split = at;
                break;
            }
            used += width;
            split = at + ch.len_utf8();
        }
        let (head, tail) = text.split_at(split);
        out.push(Span::styled(head.to_string(), item.style.fg(color)));
        out.push(Span::styled(tail.to_string(), item.style));
        left = 0;
    }
    out
}

fn composer_rule(
    model: &Model,
    color: ratatui::style::Color,
    hidden: usize,
    direction: &str,
) -> Line<'static> {
    if direction == "above" {
        if let Some((x, _)) = mic_geometry(model) {
            let left = if hidden > 0 {
                format!("─ {hidden} lines above ")
            } else {
                String::new()
            };
            let left = clip_ellipsis(&left, x);
            let used = UnicodeWidthStr::width(left.as_str());
            return Line::from(vec![
                span(
                    format!("{left}{}", "─".repeat((x as usize).saturating_sub(used))),
                    color,
                ),
                span(mic_label(model), model.theme.primary),
            ]);
        }
    }
    if hidden == 0 {
        return Line::from(span("─".repeat(model.width as usize), color));
    }
    let label = if hidden > 0 {
        format!("─ {hidden} lines {direction} ")
    } else {
        String::new()
    };
    let used = UnicodeWidthStr::width(label.as_str());
    Line::from(crate::davinci::ui::truncate_run(
        vec![span(
            format!(
                "{label}{}",
                "─".repeat((model.width as usize).saturating_sub(used))
            ),
            color,
        )],
        model.width,
    ))
}

fn mic_label(model: &Model) -> String {
    if model.width >= 52 {
        format!("[{}]", model.voice.label)
    } else if model.voice.label.starts_with("REC ") {
        "[REC / Esc cancel]".into()
    } else if model.voice.active {
        "[voice / Esc]".into()
    } else {
        "[mic]".into()
    }
}

pub fn mic_geometry(model: &Model) -> Option<(u16, u16)> {
    if !model.voice.enabled || !model.voice_eligible() || model.height < 4 {
        return None;
    }
    let width = UnicodeWidthStr::width(mic_label(model).as_str()) as u16;
    (width <= model.width).then_some((model.width.saturating_sub(width), width))
}

/// Scroll a long logical row to keep the editor's byte cursor visible. Display
/// width and grapheme boundaries keep CJK text and composed characters intact.
fn composer_view(entry: &str, column: Option<usize>, width: u16) -> (String, Option<usize>) {
    use unicode_segmentation::UnicodeSegmentation;
    let Some(column) = column.filter(|&c| c <= entry.len() && entry.is_char_boundary(c)) else {
        return (clip_ellipsis(entry, width), None);
    };
    let mut start = 0;
    if UnicodeWidthStr::width(&entry[..column]) >= width as usize {
        let mut used = 0;
        start = column;
        for (at, grapheme) in entry[..column].grapheme_indices(true).rev() {
            let cells = UnicodeWidthStr::width(grapheme);
            if used + cells > width.saturating_sub(2) as usize {
                break;
            }
            used += cells;
            start = at;
        }
    }
    let prefix = if start > 0 { "…" } else { "" };
    let shown = format!(
        "{prefix}{}",
        clip_ellipsis(&entry[start..], width.saturating_sub(u16::from(start > 0)))
    );
    (shown, Some(column - start + prefix.len()))
}

/// The session change tally shown beside the active model and permission label.
/// A composer that takes no input: border rule, the sheet's reason in the dim
/// ramp, no caret (`6a` — the composer is disabled until you decide).
pub fn disabled_composer(model: &Model, text: &str) -> Vec<Line<'static>> {
    let th = &model.theme;
    let dim = th.dim();
    Surface::new(model.width, th)
        .border(th.border)
        .row(vec![
            span(format!("{} ", glyph::PROMPT), dim.border),
            span(text.to_string(), dim.muted),
        ])
        .lines()
}

/// `1 job` / `2 jobs` while background commands run; nothing otherwise, so
/// the bar says it only when there is something to know.
fn jobs_note(model: &Model) -> Option<String> {
    match model.jobs_running {
        0 => None,
        1 => Some("1 job".into()),
        n => Some(format!("{n} jobs")),
    }
}

fn model_run(model: &Model) -> String {
    format!(
        "{} · {} · {}",
        model.model_name,
        model.thinking_level,
        model.permission_label()
    )
}

/// Split a drawn composer row into `(before, under, after)` around the caret,
/// where `col` is a byte offset into the row's text.
///
/// `None` means the caret has no character to sit on in this row and the caller
/// should give it a cell of its own: the cursor is at end of line, or past what
/// survived `clip_ellipsis` on an over-long row (davinci's composer does not
/// scroll horizontally, so a caret out beyond the `…` has nowhere to land).
fn split_at_caret(shown: &str, col: usize) -> Option<(String, String, String)> {
    if col >= shown.len() || !shown.is_char_boundary(col) {
        return None;
    }
    use unicode_segmentation::UnicodeSegmentation;
    let under = shown[col..].graphemes(true).next()?;
    let end = col + under.len();
    Some((
        shown[..col].to_string(),
        under.to_string(),
        shown[end..].to_string(),
    ))
}

/// What the composer suggests while a sheet is open: the command that
/// summoned the sheet, as the mockups set each one (`2a`–`6d`).
fn screen_placeholder(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Grafo => Some("/graph path …"),
        Screen::Mensura => Some("/mensura policy frugal"),
        Screen::Models => Some("/model anthropic/claude-opus"),
        Screen::Settings => Some("/settings"),
        Screen::Thinking => Some("/thinking high"),
        Screen::Login => Some("/login openai"),
        Screen::Keys => Some("/hotkeys"),
        Screen::Resume => Some("/resume provider-parity"),
        Screen::Tree => Some("/tree"),
        Screen::Compact => Some("/compact keep the store.rs decisions"),
        Screen::Export => Some("/share"),
        Screen::GraphRun => Some("/graph-view t6"),
        Screen::Vectors => Some("/memory-search <query>"),
        Screen::Governor => Some("/governor-status"),
        Screen::Securitas => Some("/sec-report --severity high"),
        Screen::Officina => Some("/reload"),
        Screen::Trust => Some("decide first"),
        Screen::Mcp => Some("/mcp"),
        Screen::Permissions => Some("/permissions ask"),
        Screen::Workflows => Some("/workflow <goal>"),
        Screen::TaskBoard => Some("/tasks"),
        Screen::Agents => Some("/agents"),
        Screen::ContextInspector => Some("/context"),
        Screen::Extensions => Some("/plugin"),
        Screen::Recovery | Screen::Diff | Screen::Agent | Screen::Plan | Screen::Memoria => None,
    }
}

/// What the composer is offering, drawn directly above it: the marked row
/// carries the same 3-cell copper bar and tinted ground Instrumenta uses, so a
/// selection reads without color (design.md §6). Nothing is drawn when there is
/// nothing on offer.
pub fn suggestions(model: &Model) -> Vec<Line<'static>> {
    let Some(found) = &model.suggestions else {
        return Vec::new();
    };
    if found.items.is_empty() {
        return Vec::new();
    }

    let cc = model.theme.cc();
    let name_column = ((model.width as usize * 2 / 5).max(20)) as u16;
    let items: Vec<Vec<(String, String)>> = found
        .items
        .iter()
        .map(|item| {
            let label = suggestion_label(&found.prefix, model, &item.label);
            suggestion_item_rows(
                &label,
                item.description.as_deref(),
                name_column,
                model.width,
            )
        })
        .collect();
    let heights: Vec<usize> = items.iter().map(Vec::len).collect();
    let selected = model.suggestion_index.min(items.len() - 1);
    let (start, end) = visible_items(&heights, selected, 5);
    let mut out = Vec::new();
    for (index, rows) in items.iter().enumerate().take(end).skip(start) {
        let color = if index == selected {
            cc.permission
        } else {
            cc.inactive
        };
        for (left, description) in rows {
            let mut spans = vec![span(left.clone(), color)];
            if !description.is_empty() {
                spans.push(span(description.clone(), color));
            }
            out.push(Line::from(crate::davinci::ui::truncate_run(
                spans,
                model.width,
            )));
        }
    }
    out
}

fn suggestion_label(prefix: &str, model: &Model, label: &str) -> String {
    if prefix.starts_with('@') {
        let path = label.trim_start_matches('@');
        return format!("+ {}", path.replace('/', std::path::MAIN_SEPARATOR_STR));
    }
    if prefix.starts_with('/') && !model.composer.contains(' ') {
        return format!("/{}", label.trim_start_matches('/'));
    }
    label.to_string()
}

fn suggestion_item_rows(
    label: &str,
    description: Option<&str>,
    name_column: u16,
    width: u16,
) -> Vec<(String, String)> {
    let label = clip_ellipsis(label, name_column.saturating_sub(2));
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    let Some(description) = description else {
        return vec![(format!("  {label}"), String::new())];
    };
    let room = width.saturating_sub(2 + name_column + 2).max(8);
    let mut wrapped = crate::davinci::ui::wrap(description, room);
    if wrapped.len() > 2 {
        let rest = wrapped[1..].join(" ");
        wrapped.truncate(1);
        wrapped.push(rest);
    }
    if let Some(second) = wrapped.get_mut(1) {
        if UnicodeWidthStr::width(second.as_str()) > room as usize {
            *second = clip_ellipsis(second, room);
        }
    }
    let pad = |text: &str| {
        let used = UnicodeWidthStr::width(text);
        format!(
            "{text}{}",
            " ".repeat((2 + name_column as usize).saturating_sub(used))
        )
    };
    let mut rows = vec![(pad(&format!("  {label}")), wrapped[0].clone())];
    if let Some(second) = wrapped.get(1) {
        rows.push((pad(""), second.clone()));
    }
    rows
}

fn visible_items(heights: &[usize], selected: usize, max_rows: usize) -> (usize, usize) {
    if heights.is_empty() {
        return (0, 0);
    }
    let selected = selected.min(heights.len() - 1);
    let mut start = 0;
    while start < selected && heights[start..=selected].iter().sum::<usize>() > max_rows {
        start += 1;
    }
    let mut end = selected + 1;
    while end < heights.len() && heights[start..=end].iter().sum::<usize>() <= max_rows {
        end += 1;
    }
    (start, end)
}

/// How many rows [`suggestions`] will occupy, known before it is built.
pub fn suggestions_height(model: &Model) -> u16 {
    suggestions(model).len() as u16
}

/// How many rows [`composer`] will occupy, known before it is built.
pub fn composer_height(lines: Option<&[String]>, hinted: bool) -> u16 {
    let entries = lines.map(<[String]>::len).unwrap_or(1).max(1);
    // Conversation composer: effort row + top rule + entries + bottom rule,
    // then the optional hint row. Command sheets use their actual row count.
    entries as u16 + 3 + u16::from(hinted)
}

/// The keybind row under the composer: hints left, the closing act right,
/// split by hairline bars (`1b`, `1c`).
fn hint_line(model: &Model, hint: Hint, rows_typed: usize) -> Line<'static> {
    let th = &model.theme;
    if model.voice.enabled && model.voice_eligible() && !model.voice.notice.is_empty() {
        return Line::from(span(
            clip_ellipsis(&model.voice.notice, model.width),
            th.muted,
        ));
    }
    if model.exit_armed {
        return Line::from(vec![span("ctrl+c again to exit", th.primary)]);
    }
    if model.screen == Screen::Agent && hint == Hint::Default && !model.running {
        let cycle = model.keybindings.keys_for("app.permissions.cycle").first();
        let help = match cycle {
            Some(binding) => {
                let binding = if binding == "shift+tab" {
                    "Shift+Tab"
                } else {
                    binding
                };
                if model.width >= 72 {
                    format!("  /help for shortcuts · ctrl+alt+p commands · {binding} mode")
                } else if model.width >= 40 {
                    format!("  /help · {binding} mode")
                } else {
                    format!("{binding} mode")
                }
            }
            None => "  /help for shortcuts · ctrl+alt+p commands".into(),
        };
        return spread(
            model.width,
            vec![span(help, th.muted)],
            if model.width >= 80 {
                vec![span("shift+enter for newline ", th.muted)]
            } else {
                Vec::new()
            },
        );
    }
    let bar = || span(" · ", th.border);
    let (left, right) = match hint {
        Hint::None => (Vec::new(), Vec::new()),
        Hint::Closable => (
            vec![span("enter send", th.muted)],
            vec![span("esc close", th.muted)],
        ),
        Hint::Multiline if model.width < 64 => (
            vec![span(format!("{rows_typed} lines"), th.muted)],
            vec![span("enter send", th.muted)],
        ),
        Hint::Multiline => (
            vec![
                span("shift+enter newline", th.muted),
                bar(),
                span(format!("{rows_typed} lines"), th.muted),
            ],
            vec![span("enter send", th.muted)],
        ),
        Hint::Default if model.minimal() => (
            vec![span("enter send", th.muted)],
            vec![span("esc cancel", th.muted)],
        ),
        Hint::Default => (
            vec![
                span("enter send", th.muted),
                bar(),
                span("shift+enter newline", th.muted),
                bar(),
                span("tab complete", th.muted),
            ],
            vec![span("esc cancel", th.muted)],
        ),
    };
    let mut spans = spread(model.width.saturating_sub(4), left, right).spans;
    spans.insert(0, pad(2, None));
    Line::from(spans)
}

/// `47000` → `47k`. Numbers carry their unit (design.md §9).
pub fn thousands(value: u64) -> String {
    if value >= 1_000_000 {
        let millions = value as f64 / 1_000_000.0;
        if millions >= 10.0 {
            return format!("{}m", millions.round() as u64);
        }
        return format!("{millions:.1}m");
    }
    if value >= 1_000 {
        return format!("{}k", (value as f64 / 1_000.0).round() as u64);
    }
    value.to_string()
}

/// Paths shorten to crate-relative below 80 columns (design.md §7).
fn short_cwd(cwd: &str) -> String {
    cwd.rsplit(['/', '\\'])
        .next()
        .filter(|tail| !tail.is_empty())
        .unwrap_or(cwd)
        .to_string()
}

/// Width of a row, for tests and for the responsive audit.
pub fn line_width(line: &Line<'_>) -> u16 {
    run_width(&line.spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Overlay;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.cwd = "C:\\dev\\oss\\davinci-rust".into();
        model.branch = "main".into();
        model.model_name = "sonnet".into();
        model.changes = (3, 42, 11);
        model.context = (47_000, 200_000);
        // A turn under way: the untouched empty state has its own quieter
        // composer, tested separately.
        model
            .transcript
            .push(crate::davinci::model::Entry::user("run the tests"));
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_sheet_with_facts_owns_the_header_right_run_and_the_status_third() {
        let mut m = model(100);
        crate::davinci::fixtures::dress_screen(&mut m, "6d");
        let h = text(&header(&m));
        assert!(
            h.contains("Review changes") && h.contains("7 files · +145 -127"),
            "{h}"
        );
        let s = text(&status(&m));
        assert!(s.starts_with("  review only"), "{s}");
        assert!(!s.contains("agent · main"));
    }

    #[test]
    fn a_long_completion_list_is_windowed_and_every_row_stays_reachable() {
        let mut m = model(100);
        m.login_providers = (b'a'..=b'z')
            .map(|c| format!("provider-{}", c as char))
            .collect();
        m.suggestion_rows = 6;
        m.composer.push_str("/login ");
        m.refresh_suggestions();
        let total = m
            .suggestions
            .as_ref()
            .expect("providers offered")
            .items
            .len();
        assert_eq!(total, 26, "nothing past the cap is discarded");

        let drawn: Vec<String> = suggestions(&m).iter().map(text).collect();
        assert_eq!(drawn.len(), 5, "compact completion window: {drawn:?}");
        assert!(drawn[0].contains("provider-a"));
        assert!(drawn[4].contains("provider-e"));
        assert_eq!(
            suggestions_height(&m) as usize,
            suggestions(&m).len(),
            "the promised height is the drawn height"
        );

        // The last provider is reachable, and the window follows.
        for _ in 0..25 {
            m.suggestion_move(1);
        }
        let drawn: Vec<String> = suggestions(&m).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("provider-z")),
            "the selection walked past the fold: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|row| row.contains("provider-a")),
            "{drawn:?}"
        );
        assert_eq!(suggestions_height(&m) as usize, suggestions(&m).len());
    }

    #[test]
    fn model_arguments_use_the_plain_completion_list() {
        let mut m = model(140);
        m.model_names = vec![
            "openai-codex / gpt-6-astra".into(),
            "openai-codex / gpt-5.6-luna".into(),
        ];
        m.composer.set_text("/model gpt");
        m.refresh_suggestions();
        let rows = suggestions(&m);
        let drawn = rows.iter().map(text).collect::<Vec<_>>().join("\n");
        for label in ["gpt-6-astra", "gpt-5.6-luna"] {
            assert!(drawn.contains(label), "{label}: {drawn}");
        }
        // The argument list is a completion list, not a second model picker
        // with its own header and effort rule above the composer's.
        for absent in ["Select model", "effort", "▔"] {
            assert!(!drawn.contains(absent), "{absent}: {drawn}");
        }
        assert_eq!(suggestions_height(&m) as usize, rows.len());
    }

    #[test]
    fn a_completed_command_name_waits_for_its_argument() {
        let mut m = model(140);
        m.model_names = vec!["openai-codex / gpt-6-astra".into()];
        m.thinking_levels = vec!["low".into(), "high".into()];
        m.slash_commands = ["model", "thinking"]
            .into_iter()
            .map(|name| crate::autocomplete::SlashCommandSpec {
                name: name.into(),
                argument_hint: Some("<value>".into()),
                ..Default::default()
            })
            .collect();
        for text in ["/model ", "/model   ", "/thinking "] {
            m.composer.set_text(text);
            m.refresh_suggestions();
            assert!(m.suggestions.is_none(), "{text:?} opened a list");
            assert!(suggestions(&m).is_empty(), "{text:?} drew a list");
        }
        // No hint to show: the value list is the only discovery surface.
        m.slash_commands[1].argument_hint = None;
        m.composer.set_text("/thinking ");
        m.refresh_suggestions();
        assert!(m.suggestions.is_some(), "hintless command lost its values");
        // A newline anywhere means it is not a bare command line.
        m.composer.set_text("/model \n");
        m.refresh_suggestions();
    }

    #[test]
    fn the_header_is_one_line_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let line = header(&model(width));
            assert_eq!(line_width(&line), width, "header at {width}");
        }
    }

    #[test]
    fn the_status_bar_is_one_line_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let line = status(&model(width));
            assert_eq!(line_width(&line), width, "status bar at {width}");
        }
    }

    #[test]
    fn the_header_carries_active_reasoning_next_to_the_model() {
        let mut m = model(100);
        m.thinking_level = "high".into();

        let drawn = text(&header(&m));
        assert!(drawn.contains("sonnet · high"), "{drawn}");
    }

    #[test]
    fn the_header_distinguishes_auto_from_always_approve() {
        let mut m = model(120);
        m.thinking_level = "high".into();
        assert!(text(&header(&m)).contains("· Manual"));
        m.permission_mode = "auto".into();
        assert!(text(&header(&m)).contains("sonnet · high · Auto Mode"));
        assert!(!text(&status(&m)).contains("no prompts"));
        m.permission_mode = "always-approve".into();
        assert!(text(&header(&m)).contains("· Always Approve"));
        assert!(text(&status(&m)).contains("no prompts"));
    }

    #[test]
    fn the_header_carries_path_branch_and_model_when_there_is_room() {
        let drawn = text(&header(&model(100)));
        assert!(drawn.starts_with("DaVinci · AGENT"), "{drawn}");
        assert!(drawn.contains("davinci-rust │ main │ sonnet"));
    }

    #[test]
    fn the_header_shortens_the_path_below_eighty_columns() {
        let drawn = text(&header(&model(72)));
        assert!(drawn.contains("davinci-rust │ main"), "{drawn}");
        assert!(!drawn.contains("C:\\dev"), "{drawn}");
        assert!(!drawn.contains("sonnet"), "{drawn}");
    }

    #[test]
    fn the_header_reports_the_window_size_only_when_codex_is_open() {
        let mut m = model(160);
        assert!(!text(&header(&m)).contains("160×44"));
        m.toggle_codex();
        assert!(text(&header(&m)).contains("160×44"));
    }

    #[test]
    fn permission_status_uses_all_five_exact_labels() {
        for (id, label) in [
            ("ask", "manual mode on"),
            ("edits", "accept edits on"),
            ("read-only", "plan mode on"),
            ("auto", "auto mode on"),
            ("always-approve", "always approve on"),
        ] {
            let mut m = model(100);
            m.permission_mode = id.into();
            let drawn = text(&status(&m));
            assert!(drawn.contains(label), "{id}: {drawn}");
        }
    }

    #[test]
    fn always_approve_warning_survives_narrow_status_and_monochrome() {
        for width in [10, 20, 32, 40, 48, 72, 100] {
            for no_color in [false, true] {
                let mut m = model(width);
                m.permission_mode = "always-approve".into();
                m.theme = Theme::da_vinci(ColorDepth::TrueColor, no_color);
                let row = status(&m);
                let drawn = text(&row);
                assert!(drawn.contains("no prompts"), "width {width}: {drawn}");
                if width >= 32 {
                    assert!(drawn.contains("always approve"), "{drawn}");
                }
                assert!(line_width(&row) <= width);
                assert!(row
                    .spans
                    .iter()
                    .any(|s| s.style.fg == Some(m.theme.cc().error)));
            }
        }
    }

    #[test]
    fn conversation_status_names_permissions_and_context_usage() {
        let mut m = model(100);
        let drawn = text(&status(&m));
        assert!(drawn.starts_with("  ⏸ manual mode on"));
        assert!(drawn.contains("? for shortcuts"));
        assert!(!drawn.contains("23% context"));
        m.context = (180_000, 200_000);
        assert!(text(&status(&m)).contains("90% context"));
    }

    #[test]
    fn a_narrow_status_bar_keeps_permissions_and_labels_the_percentage() {
        for width in [72u16, 90] {
            let drawn = text(&status(&model(width)));
            assert!(drawn.contains("? for shortcuts"), "{drawn}");
            assert!(drawn.contains("manual mode on"), "{drawn}");
        }
    }

    #[test]
    fn the_status_bar_left_names_the_screen_in_hand() {
        let mut m = model(120);
        assert!(text(&status(&m)).starts_with("  ⏸ manual mode on · ? for shortcuts"));
        m.toggle_screen(Screen::Grafo);
        let drawn = text(&status(&m));
        assert!(drawn.contains("Code graph"), "{drawn}");
        assert!(!drawn.contains("enter open node") && !drawn.contains("x expand"));
        assert_eq!(sheet::chrome(&m).unwrap().escape, Some("esc close"));
    }

    #[test]
    fn the_composer_uses_plain_rules_a_prompt_and_a_hint_row() {
        let m = model(100);
        let rows = composer(&m, None, Hint::Default);
        assert_eq!(rows.len() as u16, composer_height(None, true));
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].spans[0].style.fg, Some(m.theme.border));
        assert_eq!(rows[2].style.bg, None);
        assert!(text(&rows[1]).chars().all(|ch| ch == '─'));
        let prompt_row = text(&rows[2]);
        assert!(prompt_row.contains("❯"), "{prompt_row}");
        assert!(
            !prompt_row.contains("…"),
            "an empty composer carries no placeholder prose: {prompt_row}"
        );
        assert!(text(&rows[4]).contains("/help for shortcuts · ctrl+alt+p commands"));
        assert!(text(&rows[4])
            .trim_end()
            .ends_with("shift+enter for newline"));
    }

    #[test]
    fn the_untouched_empty_state_is_a_quiet_rule_with_no_prose() {
        let mut m = model(100);
        m.transcript.clear();
        let rows = composer(&m, None, Hint::None);
        assert_eq!(rows[1].spans[0].style.fg, Some(m.theme.border));
        let prompt_row = text(&rows[2]);
        assert!(prompt_row.contains("❯"), "{prompt_row}");
        assert!(!prompt_row.contains("…"), "{prompt_row}");
    }

    #[test]
    fn a_sheet_suggests_its_summoning_command_and_the_chat_suggests_nothing() {
        // The agent chat carries no placeholder prose; an open sheet is the
        // one exception, and its hint is dimmed so nothing about it reads as
        // typed text.
        let mut m = model(100);
        m.composer = String::new().into();
        m.screen = crate::davinci::model::Screen::Models;
        let rows = composer(&m, None, Hint::None);
        let hint = rows[1]
            .spans
            .iter()
            .find(|span| span.content.starts_with('/'))
            .expect("the sheet hint");
        assert_eq!(hint.style.fg, Some(m.theme.text), "{:?}", hint.content);
        assert!(hint
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::DIM));

        m.screen = crate::davinci::model::Screen::Agent;
        let drawn = text(&composer(&m, None, Hint::None)[1]);
        assert!(
            !drawn.chars().any(char::is_alphabetic),
            "the empty chat row is just the prompt: {drawn}"
        );
    }

    #[test]
    fn the_composer_grows_with_its_content() {
        let m = model(100);
        let entered = vec![
            "keep step IV, but generate the fixtures from the".to_string(),
            "existing TS golden files under tests\\golden\\".to_string(),
        ];
        let rows = composer(&m, Some(&entered), Hint::Multiline);
        assert_eq!(rows.len(), 6);
        assert_eq!(composer_height(Some(&entered), true), 6);
        assert!(text(&rows[2]).contains("keep step IV"));
        assert!(text(&rows[3]).contains("existing TS golden files"));
        let hint = text(&rows[5]);
        assert!(hint.contains("shift+enter newline · 2 lines"), "{hint}");
        assert!(hint.trim_end().ends_with("enter send"), "{hint}");
    }

    #[test]
    fn the_caret_is_drawn_on_the_last_row_only_and_blinks() {
        let mut m = model(100);
        let entered = vec!["one".to_string(), "two".to_string()];
        let rows = composer(&m, Some(&entered), Hint::Multiline);
        let caret = |line: &Line<'_>| {
            line.spans
                .iter()
                .any(|span| span.style.bg.is_some() && span.style.bg != Some(m.theme.background))
        };
        assert!(!caret(&rows[2]), "no caret on the first row");
        assert!(caret(&rows[3]), "caret on the last row");

        m.tick = 4;
        let off = composer(&m, Some(&entered), Hint::Multiline);
        assert!(!caret(&off[3]), "the caret blinks off");
    }

    #[test]
    fn the_hint_row_abbreviates_below_eighty_columns() {
        let mut m = model(72);
        m.running = true;
        let drawn = text(&composer(&m, None, Hint::Default)[4]);
        assert!(drawn.contains("enter send"), "{drawn}");
        assert!(drawn.contains("esc cancel"), "{drawn}");
        assert!(!drawn.contains("tab complete"), "{drawn}");
    }

    #[test]
    fn recall_replaces_the_composer_with_memorias_own_keys() {
        let mut m = model(100);
        m.toggle_screen(Screen::Memoria);
        let rows = composer(&m, None, Hint::None);
        let drawn = text(&rows[1]);
        assert!(drawn.contains("enter pin to context"), "{drawn}");
        assert!(drawn.contains("esc close"), "{drawn}");
        assert!(
            !rows
                .iter()
                .flat_map(|row| row.spans.iter())
                .any(|span| span.style.bg == Some(m.theme.text)),
            "no caret while memoria owns the keys"
        );
    }

    #[test]
    fn the_palette_query_does_not_reach_the_composer_row() {
        let mut m = model(100);
        m.toggle_overlay(Overlay::Instrumenta);
        m.type_char("git");
        let drawn = text(&composer(&m, None, Hint::Default)[1]);
        assert!(
            !drawn.contains("…"),
            "no placeholder under an overlay: {drawn}"
        );
    }

    /// Find the caret after the four-cell prompt/continuation gutter.
    fn caret_column(model: &Model, row: usize) -> Option<usize> {
        let line = composer(model, None, Hint::Default).remove(row + 1);
        let caret_color = if model.theme.text == ratatui::style::Color::Reset {
            model.theme.primary
        } else {
            model.theme.text
        };
        let mut column = 0usize;
        let mut caret = None;
        for span in &line.spans {
            if caret.is_none() && span.style.bg == Some(caret_color) {
                caret = Some(column);
            }
            column += UnicodeWidthStr::width(span.content.as_ref());
        }
        caret.map(|at| at.saturating_sub(2))
    }

    #[test]
    fn the_caret_sits_where_the_cursor_is_not_at_the_end_of_the_row() {
        let mut m = model(100);
        m.type_char("what llm model are you");
        assert_eq!(
            caret_column(&m, 1),
            Some(22),
            "end of line takes its own cell"
        );

        for _ in 0..5 {
            m.composer.editor_mut().move_left();
        }
        assert_eq!(
            caret_column(&m, 1),
            Some(17),
            "five left of the end is the `e` of `are`, not the end of the row"
        );

        m.composer.editor_mut().move_line_start();
        assert_eq!(caret_column(&m, 1), Some(0), "line start is column zero");
    }

    #[test]
    fn the_caret_follows_the_cursor_onto_an_earlier_composer_row() {
        let mut m = model(100);
        m.type_char("alpha");
        m.newline();
        m.type_char("bravo");
        assert_eq!(
            caret_column(&m, 2),
            Some(5),
            "the caret starts on the row being typed"
        );
        assert_eq!(caret_column(&m, 1), None, "and only on that row");

        m.composer.editor_mut().cursor_up();
        assert_eq!(
            caret_column(&m, 1),
            Some(5),
            "up moves the caret onto `alpha`"
        );
        assert_eq!(caret_column(&m, 2), None, "and off `bravo`");
    }

    #[test]
    fn a_caret_past_the_clip_keeps_its_own_cell() {
        // Never index clipped text with the original editor's byte column.
        // Horizontal scrolling translates the column before splitting it.
        assert_eq!(split_at_caret("abc…", 9), None);
        assert_eq!(
            split_at_caret("abc", 3),
            None,
            "end of line has no character"
        );
        assert_eq!(
            split_at_caret("héllo", 1),
            Some((String::from("h"), String::from("é"), String::from("llo"))),
            "a multi-byte character is taken whole"
        );
        assert_eq!(split_at_caret("héllo", 2), None, "never splits a character");
    }

    #[test]
    fn numbers_carry_their_unit() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(47_000), "47k");
        assert_eq!(thousands(128_400), "128k");
        assert_eq!(thousands(200_000), "200k");
        assert_eq!(thousands(1_200_000), "1.2m");
        assert_eq!(thousands(18_402_000), "18m");
    }

    #[test]
    fn governor_notice_renders_and_respects_constraints() {
        let mut m = model(100);
        assert!(governor_notice(&m).is_empty());

        m.governor_notice = Some((
            "compressed bash output · saved 14k tokens".into(),
            std::time::Instant::now(),
        ));
        let lines = governor_notice(&m);
        assert_eq!(lines.len(), 1);
        let rendered = text(&lines[0]);
        assert!(rendered.contains("✓ Governor"));
        assert!(rendered.contains("compressed bash output · saved 14k tokens"));
        assert!(rendered.contains("/governor-status"));

        // Omitted when window height is below 12 rows
        m.height = 10;
        assert!(governor_notice(&m).is_empty());
        m.height = 44;

        // At narrow width (< 100 cols), the status shortcut is dropped
        m.width = 80;
        let narrow = text(&governor_notice(&m)[0]);
        assert!(narrow.contains("✓ Governor"));
        assert!(!narrow.contains("/governor-status"));

        // Notice expires after 8 seconds
        m.width = 100;
        m.governor_notice = Some((
            "compressed bash output · saved 14k tokens".into(),
            std::time::Instant::now() - std::time::Duration::from_secs(9),
        ));
        assert!(governor_notice(&m).is_empty());
    }
}
