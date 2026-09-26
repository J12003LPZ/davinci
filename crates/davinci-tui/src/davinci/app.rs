//! The app shell: compose a window's worth of rows, route a key, paint.
//!
//! Layout is assembled as a flat list of rows rather than through nested
//! ratatui `Layout` splits. The composer follows a short conversation and
//! reaches the bottom as it fills the window. Overlays dim the transcript
//! behind them with the dropped ramp (design.md §1, §2).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/app.ex`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Line;
use zeroize::Zeroizing;

use crate::interaction::{apply_editor_key, input_owner, key_event_bytes, InputOwner};

use super::model::{Choice, Model, Overlay, Screen};
use super::ui::{self, blank, pad_to, tail};
use super::views::chrome::{self, Hint};
use super::views::sheet::{self, Composer};
use super::views::{
    agents, ask, codex, cogitator, compact, context_inspector, decision_modal, diff, disegno,
    export, extensions, governor, grafo, graph_run, instrumenta, keys, login, mcp, memoria,
    mensura, officina, opera, permissions, recovery, resume, rewind, secret_input, securitas,
    settings, startup, task_board, transcript, tree, trust, vectors, workflows,
};

/// Secret value carried from the masked input overlay to the host.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretInputValue(Zeroizing<String>);

impl SecretInputValue {
    pub(crate) fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn into_inner(self) -> String {
        let mut value = self.0;
        std::mem::take(&mut *value)
    }
}

impl std::fmt::Debug for SecretInputValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretInputValue")
            .finish_non_exhaustive()
    }
}

/// What the runtime should do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    /// Keep going.
    Continue,
    /// Leave the TUI. `ctrl+c` never produces this — it interrupts the run.
    Quit,
    /// The user asked to interrupt the run in progress.
    Interrupt,
    /// The composer was sent; the caller owns what happens next.
    Submit(String),
    /// A row of the open instrument was chosen; the caller owns the action.
    Choose(Choice),
    /// Request the next permission mode. Only the runtime may apply it and
    /// synchronize `Model::permission_mode`; the draft and UI state stay put.
    CyclePermissionMode,
    /// A secret was submitted from the dedicated masked input. The host must
    /// validate it immediately and must not put it in the transcript.
    SecretInputSubmitted(SecretInputValue),
}

/// Computes the next 0-indexed permission mode in the 5-mode cycle.
pub fn next_mode_index(index: usize) -> usize {
    (index + 1) % 5
}

/// Compose exactly `height` rows. Conversation chrome follows the content;
/// command sheets and overlays retain their full-height frame.
pub fn compose(model: &Model, height: u16) -> Vec<Line<'static>> {
    compose_frame(model, height).lines
}

pub struct ComposedFrame {
    pub lines: Vec<Line<'static>>,
    pub mic_rect: Option<ratatui::layout::Rect>,
    pub graph: Option<super::views::graph_nav::GraphFrame>,
}

pub fn compose_frame(model: &Model, height: u16) -> ComposedFrame {
    let height = height as usize;
    if model.voice.setup {
        let rows = model
            .voice
            .setup_rows
            .iter()
            .map(|text| {
                Line::from(ui::span(
                    ui::clip_ellipsis(text, model.width),
                    model.theme.text,
                ))
            })
            .collect();
        return ComposedFrame {
            lines: pad_to(rows, height),
            mic_rect: None,
            graph: None,
        };
    }
    if height == 0 {
        return ComposedFrame {
            lines: Vec::new(),
            mic_rect: None,
            graph: None,
        };
    }
    if matches!(model.screen, Screen::Models | Screen::Settings) && model.overlay.is_none() {
        let picker = if model.screen == Screen::Settings {
            settings::screen(model, settings::screen_height(model).min(height))
        } else {
            cogitator::screen(model, cogitator::screen_height(model).min(height))
        };
        let picker_height = picker.len();
        let mut context = Model {
            screen: Screen::Agent,
            overlay: None,
            ..model.clone()
        };
        context.graph_run = None;
        let mut lines = body(&context, height.saturating_sub(picker_height));
        lines = pad_to(lines, height.saturating_sub(picker_height));
        lines.extend(picker);
        return ComposedFrame {
            lines: pad_to(lines, height),
            mic_rect: None,
            graph: None,
        };
    }
    // Command surfaces use the same bottom-anchored, unboxed language as the
    // reference. The graph retains its optional navigable canvas and composer.
    if model.overlay.is_none() && model.screen != Screen::Agent && model.screen != Screen::GraphRun
    {
        if let Some(content) = section_rows(model) {
            return ComposedFrame {
                lines: command_panel_frame(model, content, height),
                mic_rect: None,
                graph: None,
            };
        }
    }
    // While an instrument floats over the transcript, the chrome around it
    // drops its ramp too — only the panel keeps full ink (`1d`, `1f`).
    let dimmed = model.overlay.map(|_| Model {
        theme: model.theme.dim(),
        ..model.clone()
    });
    let chrome_model = dimmed.as_ref().unwrap_or(model);
    let conversation =
        model.screen == Screen::Agent && model.overlay.is_none() && !model.codex_open();

    let composer_rows = composer_rows(chrome_model);
    // What the composer offers sits directly above it, inside the same stack,
    // so the list moves with the composer at any window height.
    let offered = chrome::suggestions(chrome_model);
    let top = extension_rows(model, &model.extensions.header);
    let bottom = extension_rows(model, &model.extensions.footer);
    let above = extension_rows(model, &model.extensions.above());
    let below = extension_rows(model, &model.extensions.below());
    let notice = chrome::governor_notice(chrome_model);
    // The working line is a block of its own, so it never runs into the last
    // transcript row (design.md §3). It is pinned here rather than pushed into
    // the transcript so what a running turn has cost stays put while the
    // transcript scrolls under it.
    let mut working = if height >= 8 {
        opera::lines(chrome_model)
    } else {
        Vec::new()
    };
    if model.screen == Screen::Agent && model.overlay.is_none() && height >= 10 {
        working.extend(graph_run::background_lines(chrome_model));
    }
    if !working.is_empty() {
        working.insert(0, blank());
    }
    let reserved = usize::from(!conversation)
        + top.len()
        + bottom.len()
        + above.len()
        + working.len()
        + notice.len()
        + offered.len()
        + composer_rows.len()
        + below.len()
        + chrome::footer(chrome_model).len();
    let body_height = height.saturating_sub(reserved);

    let mut rows = Vec::with_capacity(height);
    if !conversation {
        rows.push(chrome::header(chrome_model));
    }
    rows.extend(top);
    let body_start = rows.len() as u16;
    let mut graph = None;
    let content = body_with_graph(model, body_height, &mut graph);
    rows.extend(if conversation {
        pad_to(content, body_height)
    } else {
        content
    });
    if let Some(frame) = &mut graph {
        frame.origin_y = frame.origin_y.saturating_add(body_start);
    }
    rows.extend(bottom);
    rows.extend(above);
    rows.extend(working);
    rows.extend(notice);
    rows.extend(offered);
    let mic_rect = chrome::mic_geometry(model).and_then(|(x, width)| {
        // The conversation composer now starts with an effort row; the mic is
        // rendered on the rule immediately below it. Hit testing must point at
        // that rendered row rather than the start of the composer stack.
        let composer_rule_offset =
            usize::from(model.screen == Screen::Agent && model.overlay.is_none());
        let y = rows.len().saturating_add(composer_rule_offset);
        (height >= 4 && !composer_rows.is_empty() && y < height)
            .then_some(ratatui::layout::Rect::new(x, y as u16, width, 1))
    });
    rows.extend(composer_rows);
    rows.extend(below);
    rows.extend(chrome::footer(chrome_model));
    rows.truncate(height);
    if !conversation {
        rows = rows
            .into_iter()
            .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
            .collect();
    }
    ComposedFrame {
        lines: pad_to(rows, height),
        graph,
        mic_rect,
    }
}

/// A shared command panel for authentication, history, help, policies and
/// DaVinci-only utilities. Data/actions remain in their existing owners.
fn command_panel_frame(
    model: &Model,
    content: Vec<Line<'static>>,
    height: usize,
) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let chrome = sheet::chrome(model);
    let notices: Vec<Line<'static>> = model
        .section_notice
        .as_ref()
        .map(|text| {
            ui::wrap(text, model.width.saturating_sub(3))
                .into_iter()
                .take(3)
                .map(|text| {
                    Line::from(ui::truncate_run(
                        vec![ui::span(format!("   {text}"), th.warning)],
                        model.width,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let safety = usize::from(model.permission_mode == "always-approve");
    let panel_height =
        (content.len() + 5 + notices.len() + safety).min(height.saturating_sub(2).max(1));
    let rows_room = panel_height.saturating_sub(4 + notices.len() + safety);
    let selected = ui::focused_row(&content).unwrap_or(model.sheet_anchor());
    let anchor = model.section_offset.unwrap_or(selected);
    let context = Model {
        screen: Screen::Agent,
        overlay: None,
        graph_run: None,
        ..model.clone()
    };
    let mut rows = pad_to(
        body(&context, height.saturating_sub(panel_height)),
        height.saturating_sub(panel_height),
    );
    if panel_height < 5 {
        rows.extend(ui::window(content, panel_height, anchor, th));
        return pad_to(
            rows.into_iter()
                .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
                .collect(),
            height,
        );
    }
    if matches!(
        model.screen,
        Screen::TaskBoard | Screen::Agents | Screen::Mcp | Screen::Extensions
    ) {
        rows.push(Line::from(ui::span(
            "▔".repeat(usize::from(model.width)),
            cc.permission,
        )));
    } else {
        rows.push(chrome::effort_rule(model));
    }
    rows.push(Line::from(ui::truncate_run(
        vec![
            ui::span("   ", cc.inactive),
            ui::span_strong(sheet::title(model.screen), cc.permission, th),
        ],
        model.width,
    )));
    rows.push(blank());
    rows.extend(ui::window(content, rows_room, anchor, th));
    rows.extend(notices);
    if safety > 0 {
        rows.push(chrome::status(model));
    }
    let hint = chrome
        .as_ref()
        .and_then(|chrome| sheet::hint_row(model, chrome))
        .unwrap_or_else(|| Line::from(ui::span("   Esc to close", cc.inactive)));
    rows.push(hint);
    pad_to(
        rows.into_iter()
            .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
            .collect(),
        height,
    )
}

/// Rows an extension supplied. They are drawn as plain text in the shell's own
/// muted ink and clipped to the window: an extension contributes words, not a
/// palette and not a layout (design.md §2).
fn extension_rows(model: &Model, lines: &[String]) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|line| {
            let text = ui::clip_ellipsis(line, model.width.saturating_sub(2));
            ui::indent(2, vec![ui::span(text, model.theme.muted)])
        })
        .collect()
}

fn composer_rows(model: &Model) -> Vec<Line<'static>> {
    if model.overlay.is_some() {
        return Vec::new();
    }
    // Conversation hints stay close to the prompt and abbreviate on narrow
    // windows. Overlays and the Codex split own their keyboard guidance.
    // A command sheet says what sits under it; its hint row is the only hint
    // row (design.md §11).
    if let Some(sheet) = sheet::chrome(model) {
        match sheet.composer {
            Composer::Hidden => return Vec::new(),
            Composer::Disabled(text) => return chrome::disabled_composer(model, text),
            Composer::Prompt(_) | Composer::PromptOwned(_) => {
                if model.queued.is_empty() {
                    return chrome::composer(model, None, Hint::None);
                }
                let mut rows: Vec<String> = model.queued.clone();
                rows.extend(model.composer.split('\n').map(str::to_string));
                return chrome::composer(model, Some(&rows), Hint::None);
            }
        }
    }
    let hint = if model.overlay.is_some()
        || model.height < 8
        || model.codex_open()
        || model.screen == Screen::Agent
    {
        Hint::None
    } else {
        match model.screen {
            Screen::Memoria | Screen::Trust => Hint::None,
            // Once the composer holds more than one row, the hint that
            // matters is how to end it, not the full list.
            _ if model.composer.contains('\n') || !model.queued.is_empty() => Hint::Multiline,
            // A sheet is open: the way out is worth a hint. After an
            // interrupt (`6c`) the composer is the way forward instead, so it
            // keeps the full send hints.
            Screen::Agent | Screen::Plan | Screen::Recovery => Hint::Default,
            _ => Hint::Closable,
        }
    };
    if model.queued.is_empty() {
        return chrome::composer(model, None, hint);
    }
    // What is waiting sits above what is being typed, in the same box.
    let mut rows: Vec<String> = model.queued.clone();
    rows.extend(model.composer.split('\n').map(str::to_string));
    chrome::composer(model, Some(&rows), hint)
}

/// A short conversation starts under its welcome banner. As it fills the
/// window, older rows fall off the top and the composer reaches the bottom.
fn section_rows(model: &Model) -> Option<Vec<Line<'static>>> {
    match model.screen {
        Screen::Plan => Some(disegno::lines(model)),
        Screen::Grafo => Some(grafo::lines(model)),
        Screen::Memoria => Some(memoria::recall(model)),
        Screen::Mensura => Some(mensura::lines(model)),
        Screen::Models => Some(cogitator::catalog(model)),
        Screen::Settings => Some(settings::lines(model)),
        Screen::Thinking => Some(super::views::thinking::lines(model)),
        Screen::Login => Some(login::lines(model)),
        Screen::Resume => Some(resume::lines(model)),
        Screen::Tree => Some(tree::lines(model)),
        Screen::Compact => Some(compact::lines(model)),
        Screen::Export => Some(export::lines(model)),
        Screen::GraphRun => Some(graph_run::lines(model)),
        Screen::Vectors => Some(vectors::lines(model)),
        Screen::Governor => Some(governor::lines(model)),
        Screen::Securitas => Some(securitas::lines(model)),
        Screen::Trust => Some(trust::lines(model)),
        Screen::Officina => Some(officina::lines(model)),
        Screen::Recovery => Some(recovery::lines(model)),
        Screen::Diff => Some(diff::lines(model)),
        Screen::Mcp => Some(mcp::lines(model)),
        Screen::Permissions => Some(permissions::lines(model)),
        Screen::Workflows => Some(workflows::lines(model)),
        Screen::TaskBoard => Some(task_board::lines(model)),
        Screen::Agents => Some(agents::lines(model)),
        Screen::ContextInspector => Some(context_inspector::lines(model)),
        Screen::Extensions => Some(extensions::lines(model)),
        Screen::Keys => Some(keys::lines(model)),
        Screen::Agent => None,
    }
}

fn body(model: &Model, height: usize) -> Vec<Line<'static>> {
    body_with_graph(model, height, &mut None)
}

fn body_with_graph(
    model: &Model,
    height: usize,
    graph: &mut Option<super::views::graph_nav::GraphFrame>,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if let Some(overlay) = model.overlay {
        return overlay_body(model, overlay, height);
    }
    if model.screen == Screen::GraphRun {
        return panel(model, Vec::new(), height, graph);
    }
    if let Some(rows) = section_rows(model) {
        return panel(model, rows, height, graph);
    }
    if model.codex_open() {
        return codex::lines(model, height);
    }
    if model.transcript.is_empty() {
        return empty_state(model, height);
    }
    let width = model.width;
    let mut rows = transcript::tail_lines(model, &model.transcript, width, height);
    let banner = startup::banner(model, &model.startup);
    if rows.len() + banner.len() + 3 <= height {
        let mut above = vec![blank()];
        above.extend(banner);
        above.push(blank());
        above.extend(rows);
        rows = above;
    }
    if rows.len() < height {
        rows.push(blank());
    }
    rows
}

/// The welcome starts at the top, just as the first conversation will.
fn empty_state(model: &Model, height: usize) -> Vec<Line<'static>> {
    startup::lines(model, &model.startup)
        .into_iter()
        .take(height)
        .collect()
}

/// A screen that takes over the body. A command sheet (`3a`–`6d`) fills it
/// from the header down: the command that summoned it echoed first, the
/// rows windowed around the selection, its hint row pinned last (design.md
/// §11). The screens with a frame of their own (`1c`, `2a`–`2c`) keep the
/// turn that produced them visible above and anchor to the composer.
fn panel(
    model: &Model,
    mut rows: Vec<Line<'static>>,
    height: usize,
    graph: &mut Option<super::views::graph_nav::GraphFrame>,
) -> Vec<Line<'static>> {
    let Some(chrome) = sheet::chrome(model) else {
        let panel = tail(rows, height);
        let room = height - panel.len();
        let mut above = transcript::tail_lines(model, &model.transcript, model.width, room);
        while above.len() < room {
            above.insert(0, blank());
        }
        above.extend(panel);
        return above;
    };
    let hint = sheet::hint_row(model, &chrome);
    let th = &model.theme;
    let mut out = Vec::with_capacity(height);
    if let Some(echo) = &chrome.echo {
        out.push(Line::from(ui::truncate_run(
            vec![
                ui::span(format!("{} ", super::theme::glyph::USER), th.muted),
                ui::span(echo.clone(), th.muted),
            ],
            model.width,
        )));
        out.push(blank());
    }
    let hint_rows = usize::from(hint.is_some());
    let mut notice = model
        .section_notice
        .as_ref()
        .map(|text| ui::section_detail(model.width, th, &format!("! {text}")))
        .unwrap_or_default();
    notice.truncate(3.min(height.saturating_sub(hint_rows + 1)));
    for row in &mut notice {
        for span in &mut row.spans {
            span.style.fg = Some(th.warning);
        }
    }
    let room = height
        .saturating_sub(out.len())
        .saturating_sub(hint_rows + notice.len());
    if model.screen == Screen::GraphRun {
        if let Some(layout) = graph_run::layout_for(model, room as u16) {
            rows = graph_run::lines_with_layout(model, room as u16, &layout);
            let offset = super::views::graph_nav::viewport(
                &layout,
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
            );
            *graph = Some(super::views::graph_nav::GraphFrame {
                layout,
                origin_y: out.len() as u16 + graph_run::HEADER_ROWS,
                offset,
            });
            out.extend(rows);
            out = pad_to(out, height.saturating_sub(hint_rows + notice.len()));
            out.extend(notice);
            if let Some(hint) = hint {
                out.push(hint);
            }
            out.truncate(height);
            return out;
        }
        rows = graph_run::lines_in(model, room as u16);
    }
    let picking = matches!(
        model.screen,
        Screen::Models
            | Screen::Settings
            | Screen::Thinking
            | Screen::Login
            | Screen::Resume
            | Screen::Tree
            | Screen::Permissions
            | Screen::Diff
            | Screen::Securitas
            | Screen::Agents
            | Screen::ContextInspector
            | Screen::Extensions
    );
    let anchor = model.section_offset.unwrap_or_else(|| {
        if picking {
            ui::focused_row(&rows).unwrap_or(model.sheet_anchor())
        } else {
            model.sheet_anchor()
        }
    });
    let pinned = match model.screen {
        Screen::Settings => settings::PINNED_DETAIL_ROWS,
        _ => 0,
    };
    if pinned > 0 && room > pinned && rows.len() > pinned {
        let pinned = pinned.min(rows.len());
        out.extend(rows[..pinned].iter().cloned());
        out.extend(ui::window(
            rows[pinned..].to_vec(),
            room - pinned,
            anchor.saturating_sub(pinned),
            th,
        ));
    } else {
        out.extend(ui::window(rows, room, anchor, th));
    }
    let mut out = pad_to(out, height.saturating_sub(hint_rows + notice.len()));
    out.extend(notice);
    if let Some(hint) = hint {
        out.push(hint);
    }
    out.truncate(height);
    out
}

/// An instrument in hand: the transcript stays visible behind it with the ramp
/// dropped, and the panel is drawn over it, anchored above the composer
/// (design.md §2, screens `1d` and `1f`).
fn overlay_rows(model: &Model, overlay: Overlay) -> Vec<Line<'static>> {
    match overlay {
        Overlay::Instrumenta => instrumenta::all_lines(model),
        Overlay::Sessions => memoria::session_lines(model),
        Overlay::Cogitator => cogitator::lines(model, &model.config_path),
        Overlay::SecretInput => secret_input::lines(model),
        Overlay::Ask => {
            if let Some(rewind) = &model.rewind_modal {
                rewind::lines(rewind, model.width, &model.theme)
            } else if let Some(decision) = &model.decision_modal {
                decision_modal::lines(decision, model.width, &model.theme)
            } else {
                ask::lines(model)
            }
        }
    }
}

fn overlay_body(model: &Model, overlay: Overlay, height: usize) -> Vec<Line<'static>> {
    let dimmed = Model {
        theme: model.theme.dim(),
        overlay: None,
        screen: Screen::Agent,
        ..model.clone()
    };
    let panel = ui::window_section(
        overlay_rows(model, overlay),
        height,
        if overlay == Overlay::Instrumenta {
            2
        } else {
            1
        },
        model.overlay_offset,
        &model.theme,
    );
    let mut behind = body(&dimmed, height.saturating_sub(panel.len()));
    behind = pad_to(behind, height.saturating_sub(panel.len()));
    behind.extend(panel);
    behind
}

/// Route one key. `esc` closes the instrument in hand, `ctrl+c` interrupts the
/// run and never the app (design.md §6).
pub fn handle_key(model: &mut Model, key: KeyEvent) -> Flow {
    if model.overlay == Some(Overlay::SecretInput) {
        let data = key_event_bytes(&key);
        return handle_overlay_key(model, Overlay::SecretInput, key, data.as_deref());
    }
    if model.voice.blocks_send && voice_send_key(model, &key) {
        model.voice.notice = "Finish or cancel voice before sending".into();
        return Flow::Continue;
    }
    if key.kind == KeyEventKind::Release {
        return Flow::Continue;
    }
    // ? in an empty conversation composer opens the shortcuts panel. Any
    // other key closes it and is then handled normally.
    if model.shortcuts_open {
        model.shortcuts_open = false;
        if key.code == KeyCode::Char('?') && (key.modifiers - KeyModifiers::SHIFT).is_empty() {
            return Flow::Continue;
        }
    } else if key.code == KeyCode::Char('?')
        && (key.modifiers - KeyModifiers::SHIFT).is_empty()
        && model.composer.is_empty()
        && model.screen == Screen::Agent
        && model.overlay.is_none()
    {
        model.shortcuts_open = true;
        return Flow::Continue;
    }
    if model.screen == Screen::GraphRun
        && model.overlay.is_none()
        && key.kind != KeyEventKind::Release
        && super::views::graph_nav::handle_command_center_key(model, key)
    {
        return Flow::Continue;
    }
    let data = key_event_bytes(&key);

    if action_matches(model, data.as_deref(), "davinci.interrupt")
        || (model.running
            && model.overlay.is_none()
            && action_matches(model, data.as_deref(), "app.interrupt"))
    {
        model.interrupt();
        return Flow::Interrupt;
    }

    match input_owner(
        model.overlay.is_some(),
        model.suggestions.is_some(),
        !model.composer_owns_focus(),
    ) {
        InputOwner::Modal => {
            return handle_overlay_key(
                model,
                model.overlay.expect("modal owner requires overlay"),
                key,
                data.as_deref(),
            )
        }
        InputOwner::Surface => return handle_screen_key(model, key, data.as_deref()),
        // An open completion list gets first refusal on the keys that steer it;
        // everything else falls through to the composer underneath.
        InputOwner::Autocomplete => {
            if let Some(flow) = handle_suggestion_key(model, data.as_deref()) {
                return flow;
            }
        }
        InputOwner::Composer => {}
    }

    if let Some(data) = data.as_deref() {
        // Active surfaces get first refusal above. Autocomplete may decline
        // this chord without losing its suggestions or changing the draft.
        if model.keybindings.matches(data, "app.permissions.cycle") {
            let plain_tab_modifiers = !matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                || (key.modifiers - KeyModifiers::SHIFT).is_empty()
                || (key.code == KeyCode::Tab && key.modifiers == KeyModifiers::CONTROL);
            return if key.kind == KeyEventKind::Press
                && !model.running
                && !model.voice.setup
                && model.screen == Screen::Agent
                && !model.codex_open()
                && plain_tab_modifiers
            {
                Flow::CyclePermissionMode
            } else {
                Flow::Continue
            };
        }
        // The shell's own shortcuts are checked before the editor's, because
        // some of them (explicit surface shortcuts and `ctrl+d` quit)
        // spell the same bytes as a readline binding. design.md §5 gives those
        // keys to the instruments, so the editor never sees them.
        if let Some(flow) = handle_global_key(model, data) {
            return flow;
        }
        if apply_editor_key(model.composer.editor_mut(), &model.keybindings, data) {
            // Deleting a word or moving the caret changes what is on offer.
            model.refresh_suggestions();
            // Arrows, word motions and history recall all land the caret
            // somewhere new: it stays solid rather than blinking mid-move.
            model.mark_caret_moved();
            return Flow::Continue;
        }
        if model.keybindings.matches(data, "davinci.composer.newLine") {
            model.newline();
            return Flow::Continue;
        }
        if model.keybindings.matches(data, "tui.input.tab") {
            model.complete();
            return Flow::Continue;
        }
        if model.keybindings.matches(data, "tui.input.submit") {
            let sent = model.composer.editor().get_expanded_text();
            if sent.trim().is_empty() {
                return Flow::Continue;
            }
            model.submit();
            return Flow::Submit(sent);
        }
        if model.keybindings.matches(data, "tui.select.cancel") {
            model.close();
            return Flow::Continue;
        }
    }

    if let KeyCode::Char(ch) = key.code {
        if !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            model.type_char(&ch.to_string());
        }
    }
    Flow::Continue
}

/// Shared preflight runs before extensions and autocomplete in the live loops.
pub fn voice_send_key(model: &Model, key: &KeyEvent) -> bool {
    let Some(data) = key_event_bytes(key) else {
        return false;
    };
    !model.keybindings.matches(&data, "davinci.composer.newLine")
        && (model.keybindings.matches(&data, "tui.input.submit")
            || model.keybindings.matches(&data, "app.message.followUp")
            || key.code == KeyCode::Enter)
}

/// Steer the open completion list. `None` hands the key back to the composer,
/// so ordinary typing keeps narrowing the list rather than being swallowed by
/// it (design.md §6).
fn handle_suggestion_key(model: &mut Model, data: Option<&str>) -> Option<Flow> {
    let data = data?;
    let bindings = &model.keybindings;
    if bindings.matches(data, "tui.select.up") {
        model.suggestion_move(-1);
        return Some(Flow::Continue);
    }
    if bindings.matches(data, "tui.select.down") {
        model.suggestion_move(1);
        return Some(Flow::Continue);
    }
    let tab = bindings.matches(data, "tui.input.tab");
    if tab || bindings.matches(data, "tui.input.submit") {
        let run = !tab && runs_on_enter(model);
        if model.accept_suggestion() || tab {
            if run {
                let sent = model
                    .composer
                    .editor()
                    .get_expanded_text()
                    .trim_end()
                    .to_string();
                model.dismiss_suggestions();
                model.submit();
                return Some(Flow::Submit(sent));
            }
            return Some(Flow::Continue);
        }
        return None;
    }
    if bindings.matches(data, "tui.select.cancel") {
        model.dismiss_suggestions();
        return Some(Flow::Continue);
    }
    None
}

fn runs_on_enter(model: &Model) -> bool {
    model.composer.trim_start().starts_with('/')
        && model
            .suggestions
            .as_ref()
            .is_some_and(|found| !found.prefix.starts_with(['@', '#']))
}

fn action_matches(model: &Model, data: Option<&str>, action: &str) -> bool {
    data.is_some_and(|data| model.keybindings.matches(data, action))
}

fn handle_global_key(model: &mut Model, data: &str) -> Option<Flow> {
    if model.keybindings.matches(data, "davinci.quit") && model.composer.is_empty() {
        return Some(Flow::Quit);
    }
    if model
        .keybindings
        .matches(data, "davinci.instrumenta.toggle")
    {
        model.toggle_overlay(Overlay::Instrumenta);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.sessions.toggle") {
        model.toggle_overlay(Overlay::Sessions);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.cogitator.toggle") {
        model.toggle_overlay(Overlay::Cogitator);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.plan.toggle") {
        model.toggle_screen(Screen::Plan);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.grafo.toggle") {
        model.toggle_screen(Screen::Grafo);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.mensura.toggle") {
        model.toggle_screen(Screen::Mensura);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.memoria.toggle") {
        model.toggle_screen(Screen::Memoria);
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.codex.toggle") {
        model.toggle_codex();
        return Some(Flow::Continue);
    }
    if model.keybindings.matches(data, "davinci.tools.expand") {
        model.show_tool_output = !model.show_tool_output;
        return Some(Flow::Continue);
    }
    None
}

fn handle_screen_key(model: &mut Model, key: KeyEvent, data: Option<&str>) -> Flow {
    if model.screen == Screen::GraphRun && super::views::graph_nav::handle_key(model, key) {
        return Flow::Continue;
    }
    if model.codex_open() {
        if action_matches(model, data, "davinci.codex.toggle")
            || action_matches(model, data, "tui.select.cancel")
        {
            model.toggle_codex();
        }
        return Flow::Continue;
    }

    if model.screen == Screen::Settings {
        return settings::handle_key(model, key);
    }
    if model.screen == Screen::Models && key.modifiers.is_empty() {
        if key.code == KeyCode::Char('/') && !model.catalog_search {
            model.catalog_search = true;
            return Flow::Continue;
        }
        if key.code == KeyCode::Char('s') && !model.catalog_search && model.catalog_query.is_empty()
        {
            if let Some(choice) = screen_accept(model) {
                model.catalog_session_only = true;
                return Flow::Choose(choice);
            }
            return Flow::Continue;
        }
        if key.code == KeyCode::Enter {
            model.catalog_session_only = false;
        }
    }
    if handle_picker_search(model, &key) {
        return Flow::Continue;
    }

    let toggle_action = match model.screen {
        Screen::Plan => Some("davinci.plan.toggle"),
        Screen::Grafo => Some("davinci.grafo.toggle"),
        Screen::Memoria => Some("davinci.memoria.toggle"),
        Screen::Mensura => Some("davinci.mensura.toggle"),
        Screen::Agent => return Flow::Continue,
        // The command-opened sheets (`3a`–`6d`) have no toggle chord of their
        // own; esc is their way out.
        _ => None,
    };
    if toggle_action.is_some_and(|action| action_matches(model, data, action))
        || action_matches(model, data, "tui.select.cancel")
    {
        model.close();
        return Flow::Continue;
    }
    if model.screen == Screen::Plan && action_matches(model, data, "davinci.instrumenta.toggle") {
        model.toggle_overlay(Overlay::Instrumenta);
        return Flow::Continue;
    }

    if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) && key.modifiers.is_empty() {
        let page = model.height.saturating_sub(6).max(1) as isize;
        let delta = if key.code == KeyCode::PageUp {
            -page
        } else {
            page
        };
        if is_picker(model.screen) {
            let rows = section_rows(model).unwrap_or_default();
            let anchor = model
                .section_offset
                .or_else(|| ui::focused_row(&rows))
                .unwrap_or(0);
            model.section_offset = Some(
                anchor
                    .saturating_add_signed(delta)
                    .min(rows.len().saturating_sub(1)),
            );
        } else {
            screen_move(model, delta);
        }
        return Flow::Continue;
    }

    if model.screen == Screen::Models
        && key.modifiers.is_empty()
        && matches!(key.code, KeyCode::Left | KeyCode::Right)
    {
        let len = model.catalog.len();
        if let Some(row) = model.catalog.get_mut(model.catalog_index % len.max(1)) {
            row.reasoning_index = if key.code == KeyCode::Left {
                row.reasoning_index.saturating_sub(1)
            } else {
                row.reasoning_index.saturating_add(1)
            }
            .min(row.reasoning_levels.len().saturating_sub(1));
        }
        model.section_offset = None;
        return Flow::Continue;
    }

    if model.screen == Screen::Extensions && key.kind == KeyEventKind::Press {
        if let Some(flow) = handle_extensions_key(model, key) {
            return flow;
        }
    }

    // A sheet with a selection owns the arrows and enter.
    if action_matches(model, data, "tui.select.up") {
        screen_move(model, -1);
        return Flow::Continue;
    }
    if action_matches(model, data, "tui.select.down") {
        screen_move(model, 1);
        return Flow::Continue;
    }
    if action_matches(model, data, "tui.select.confirm") {
        if model.section_offset.take().is_some() {
            return Flow::Continue;
        }
        if let Some(choice) = screen_accept(model) {
            return Flow::Choose(choice);
        }
        return Flow::Continue;
    }

    if model.screen == Screen::Agents && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
    {
        let len = model.agents.as_ref().map(|s| s.agents.len()).unwrap_or(0);
        if len > 0 {
            let index = model.agents_index % len;
            match key.code {
                KeyCode::Char('s') => {
                    return Flow::Choose(Choice::AgentAction {
                        action: "steer",
                        index,
                    })
                }
                KeyCode::Char('x') => {
                    return Flow::Choose(Choice::AgentAction {
                        action: "stop",
                        index,
                    })
                }
                KeyCode::Char('r') => {
                    return Flow::Choose(Choice::AgentAction {
                        action: "retry",
                        index,
                    })
                }
                KeyCode::Char('d') => {
                    return Flow::Choose(Choice::AgentAction {
                        action: "diff",
                        index,
                    })
                }
                _ => {}
            }
        }
    }

    if model.screen == Screen::GraphRun
        && key.modifiers.is_empty()
        && key.kind == KeyEventKind::Press
    {
        let len = model.graph_run.as_ref().map(|s| s.tasks.len()).unwrap_or(0);
        let raw_index = model
            .graph_run
            .as_ref()
            .map(|s| s.selected_index)
            .unwrap_or(0);
        let index = if len > 0 { raw_index % len } else { raw_index };
        match key.code {
            KeyCode::Char('p') => {
                return Flow::Choose(Choice::GraphAction {
                    action: "pause_resume",
                    index,
                });
            }
            KeyCode::Char('s') => {
                return Flow::Choose(Choice::GraphAction {
                    action: "resume",
                    index,
                });
            }
            KeyCode::Char('x') => {
                return Flow::Choose(Choice::GraphAction {
                    action: "stop",
                    index,
                });
            }
            KeyCode::Char('r') => {
                return Flow::Choose(Choice::GraphAction {
                    action: "retry",
                    index,
                });
            }
            KeyCode::Char('d') => {
                return Flow::Choose(Choice::GraphAction {
                    action: "diff",
                    index,
                });
            }
            _ => {}
        }
    }

    if model.screen == Screen::ContextInspector
        && key.modifiers.is_empty()
        && key.kind == KeyEventKind::Press
    {
        let len = model
            .context_inspector
            .as_ref()
            .map(|s| s.rows.len())
            .unwrap_or(0);
        if len > 0 {
            let index = model.context_inspector_index % len;
            match key.code {
                KeyCode::Char('p') => {
                    return Flow::Choose(Choice::ContextInspectorAction {
                        action: "pin",
                        index,
                    })
                }
                KeyCode::Char('x') => {
                    return Flow::Choose(Choice::ContextInspectorAction {
                        action: "exclude",
                        index,
                    })
                }
                KeyCode::Char('r') => {
                    return Flow::Choose(Choice::ContextInspectorAction {
                        action: "refresh",
                        index,
                    })
                }
                KeyCode::Tab => {
                    return Flow::Choose(Choice::ContextInspectorAction {
                        action: "toggle_pending",
                        index,
                    })
                }
                _ => {}
            }
        }
    }

    // The active surface owns every other key. Surface-specific actions are
    // deliberately added here rather than falling through into the composer.
    let _ = key;
    Flow::Continue
}

/// Move the selection of whichever sheet is open. The session tree steps over
/// its spacer rows, which carry only the trunk.
fn is_picker(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Models
            | Screen::Settings
            | Screen::Thinking
            | Screen::Login
            | Screen::Resume
            | Screen::Tree
            | Screen::Permissions
            | Screen::Diff
            | Screen::Securitas
            | Screen::Agents
            | Screen::ContextInspector
    )
}

#[cfg(test)]
mod graph_input_tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
        views::graph_nav,
    };
    fn graph_model() -> Model {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 120, 40, false);
        model.screen = Screen::GraphRun;
        model.graph_run = Some(fixtures::blueprint_graph());
        model
    }
    fn press(model: &mut Model, code: KeyCode) -> Flow {
        handle_key(model, KeyEvent::new(code, KeyModifiers::NONE))
    }
    #[test]
    fn graph_view_action_keyboard_is_local_and_keeps_controls() {
        let mut model = graph_model();
        press(&mut model, KeyCode::Right);
        assert!(!model.graph_canvas.follow_live);
        assert!(model.graph_run.as_ref().unwrap().selected_node_id.is_some());
        press(&mut model, KeyCode::Char('f'));
        assert!(model.graph_canvas.follow_live);
        press(&mut model, KeyCode::Char('v'));
        assert_eq!(
            model.graph_canvas.view_mode,
            super::super::model::GraphViewMode::Focus
        );
        press(&mut model, KeyCode::Enter);
        assert!(model.graph_run.as_ref().unwrap().inspecting_node);
        press(&mut model, KeyCode::Esc);
        assert!(!model.graph_run.as_ref().unwrap().inspecting_node);
        assert_eq!(model.screen, Screen::GraphRun);
        for (key, action) in [
            ('p', "pause_resume"),
            ('s', "resume"),
            ('x', "stop"),
            ('r', "retry"),
            ('d', "diff"),
        ] {
            assert!(
                matches!(press(&mut model, KeyCode::Char(key)), Flow::Choose(Choice::GraphAction { action: actual, .. }) if actual == action)
            );
        }
        model.screen = Screen::Agent;
        press(&mut model, KeyCode::Char('f'));
        press(&mut model, KeyCode::Char('v'));
        assert_eq!(model.composer.to_string(), "fv");
    }

    #[test]
    fn graph_mouse_uses_composed_geometry_with_notices_and_echo() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut model = graph_model();
        model.section_notice = Some("notice".into());
        let composed = compose_frame(&model, model.height);
        let frame = composed
            .graph
            .expect("graph geometry accompanies rendered lines");
        let node = frame
            .layout
            .nodes
            .iter()
            .find(|n| n.id == "writer")
            .unwrap();
        let x = node.rect.x as i32 - frame.offset.0 + 2;
        let y = node.rect.y as i32 - frame.offset.1 + frame.origin_y as i32 + 1;
        assert!(composed.lines[y as usize].to_string().contains("writer"));
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x as u16,
            row: y as u16,
            modifiers: KeyModifiers::NONE,
        };
        assert!(graph_nav::handle_mouse(&mut model, mouse, &frame));
        assert_eq!(
            model
                .graph_run
                .as_ref()
                .unwrap()
                .selected_node_id
                .as_deref(),
            Some("writer")
        );
        assert!(!model.graph_canvas.follow_live);
        let selected = model.graph_run.as_ref().unwrap().selected_node_id.clone();
        graph_nav::handle_mouse(
            &mut model,
            MouseEvent {
                column: 0,
                row: frame.origin_y,
                ..mouse
            },
            &frame,
        );
        assert_eq!(model.graph_run.as_ref().unwrap().selected_node_id, selected);
        assert!(!graph_nav::handle_mouse(
            &mut model,
            MouseEvent { row: 0, ..mouse },
            &frame
        ));
    }
}

/// Search input belongs to the selector; it must never overwrite the chat draft.
fn handle_picker_search(model: &mut Model, key: &KeyEvent) -> bool {
    if !matches!(model.screen, Screen::Models | Screen::Settings)
        || key.kind == KeyEventKind::Release
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return false;
    }
    let query = if model.screen == Screen::Models {
        &mut model.catalog_query
    } else {
        &mut model.settings_query
    };
    match key.code {
        KeyCode::Char(ch) => query.push(ch),
        KeyCode::Backspace => {
            use unicode_segmentation::UnicodeSegmentation;
            let start = query
                .grapheme_indices(true)
                .last()
                .map(|(at, _)| at)
                .unwrap_or(0);
            query.truncate(start);
        }
        _ => return false,
    }
    let (indices, selected) = if model.screen == Screen::Models {
        (cogitator::visible_indices(model), &mut model.catalog_index)
    } else {
        (settings::visible_indices(model), &mut model.settings_index)
    };
    if !indices.contains(selected) {
        if let Some(first) = indices.first() {
            *selected = *first;
        }
    }
    model.section_offset = None;
    true
}

fn screen_move(model: &mut Model, delta: isize) {
    model.section_offset = None;
    use super::model::wrap_index;
    match model.screen {
        Screen::Models => {
            model.catalog_index = super::views::picker::step(
                &cogitator::visible_indices(model),
                model.catalog_index,
                delta,
            );
        }
        Screen::Settings => {
            model.settings_index = super::views::picker::step(
                &settings::visible_indices(model),
                model.settings_index,
                delta,
            );
        }
        Screen::Thinking => {
            model.thinking_index =
                wrap_index(model.thinking_index, delta, model.thinking_rows.len());
        }
        Screen::Login => {
            model.login_index = wrap_index(model.login_index, delta, model.providers.len());
        }
        Screen::Resume => {
            model.resume_index = wrap_index(model.resume_index, delta, model.resume_sessions.len());
        }
        Screen::Tree => {
            let nodes: Vec<usize> = model
                .session_tree
                .iter()
                .enumerate()
                .filter(|(_, row)| row.id.is_some())
                .map(|(index, _)| index)
                .collect();
            if !nodes.is_empty() {
                let current_pos = nodes
                    .iter()
                    .position(|&idx| idx == model.tree_index)
                    .unwrap_or(0);
                let next_pos = wrap_index(current_pos, delta, nodes.len());
                model.tree_index = nodes[next_pos];
            }
        }
        Screen::Securitas => {
            let len = model
                .security
                .as_ref()
                .map(|scan| scan.findings.len())
                .unwrap_or(0);
            model.security_index = wrap_index(model.security_index, delta, len);
        }
        Screen::Diff => {
            let len = model
                .review
                .as_ref()
                .map(|review| review.files.len())
                .unwrap_or(0);
            model.diff_index = wrap_index(model.diff_index, delta, len);
        }
        Screen::Keys => {
            model.keys_offset = model
                .keys_offset
                .saturating_add_signed(delta)
                .min(keys::lines(model).len().saturating_sub(1));
        }
        Screen::GraphRun => {
            if let Some(sheet) = model.graph_run.as_mut() {
                let len = sheet.tasks.len();
                if len > 0 {
                    sheet.selected_index = wrap_index(sheet.selected_index, delta, len);
                    sheet.selected_node_id =
                        sheet.tasks.get(sheet.selected_index).map(|t| t.id.clone());
                }
            }
            let count = graph_run::lines(model).len();
            model.feature_scroll = model
                .feature_scroll
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1));
        }
        Screen::Governor | Screen::Vectors | Screen::Workflows | Screen::TaskBoard => {
            let count = match model.screen {
                Screen::Governor => governor::lines(model).len(),
                Screen::Workflows => workflows::lines(model).len(),
                Screen::TaskBoard => task_board::lines(model).len(),
                _ => vectors::lines(model).len(),
            };
            model.feature_scroll = model
                .feature_scroll
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1));
        }
        Screen::Permissions => {
            model.permission_index =
                wrap_index(model.permission_index, delta, model.permission_rows.len());
        }
        Screen::Agents => {
            let len = model.agents.as_ref().map(|s| s.agents.len()).unwrap_or(0);
            model.agents_index = wrap_index(model.agents_index, delta, len);
            if let Some(s) = model.agents.as_mut() {
                s.selected_index = model.agents_index;
            }
        }
        Screen::Extensions => {
            if let Some(sheet) = model.extension_manager.as_mut() {
                sheet.move_selection(delta);
            }
        }
        Screen::ContextInspector => {
            let len = model
                .context_inspector
                .as_ref()
                .map(|s| s.rows.len())
                .unwrap_or(0);
            model.context_inspector_index = wrap_index(model.context_inspector_index, delta, len);
            if let Some(s) = model.context_inspector.as_mut() {
                s.selected_index = model.context_inspector_index;
            }
        }
        _ => {
            let last = section_rows(model)
                .map(|rows| rows.len().saturating_sub(1))
                .unwrap_or(0);
            model.feature_scroll = model.feature_scroll.saturating_add_signed(delta).min(last);
        }
    }
}

/// The `/plugin` manager's own keys: tabs, and the actions the selected row
/// allows. Delete and hook approval arm first and run only on `y`, a key
/// that auto-repeat of the arming key cannot produce; any other key
/// disarms. Arming approval also asks the host for the plugin's details, so
/// the hook commands are on screen before `y`.
fn handle_extensions_key(model: &mut Model, key: KeyEvent) -> Option<Flow> {
    let sheet = model.extension_manager.as_mut()?;
    let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    match key.code {
        KeyCode::Left | KeyCode::BackTab => {
            sheet.switch_tab(-1);
            model.section_offset = None;
            return Some(Flow::Continue);
        }
        KeyCode::Right | KeyCode::Tab => {
            sheet.switch_tab(1);
            model.section_offset = None;
            return Some(Flow::Continue);
        }
        _ => {}
    }
    let row = sheet.current().cloned()?;
    let armed = sheet.armed_here();
    sheet.armed = None;
    let action = match key.code {
        KeyCode::Char('y') if plain && armed.is_some() => armed.unwrap_or_default(),
        KeyCode::Char('u') if plain && row.can_update => "update",
        KeyCode::Char('e') if plain && row.can_toggle => "toggle",
        KeyCode::Char('r') if plain && row.can_revoke => "revoke",
        KeyCode::Char('a') if plain && row.can_approve => {
            sheet.armed = Some((row.key.clone(), "approve"));
            "info"
        }
        KeyCode::Char('d') if plain && row.can_delete => {
            sheet.armed = Some((row.key, "delete"));
            return Some(Flow::Continue);
        }
        _ => return None,
    };
    Some(Flow::Choose(Choice::ExtensionAction {
        action,
        tab: sheet.tab,
        key: row.key,
    }))
}

fn extension_choice(model: &Model, action: &'static str) -> Option<Choice> {
    let sheet = model.extension_manager.as_ref()?;
    sheet.current().map(|row| Choice::ExtensionAction {
        action,
        tab: sheet.tab,
        key: row.key.clone(),
    })
}

/// What enter means on the open sheet, if it means anything.
fn screen_accept(model: &Model) -> Option<Choice> {
    let pick = |index: usize, len: usize| (len > 0).then(|| index % len);
    match model.screen {
        Screen::Models => cogitator::visible_indices(model)
            .contains(&model.catalog_index)
            .then_some(Choice::Catalog(model.catalog_index)),
        Screen::Settings => settings::visible_indices(model)
            .contains(&model.settings_index)
            .then_some(Choice::Setting(model.settings_index)),
        Screen::Thinking => {
            pick(model.thinking_index, model.thinking_rows.len()).map(Choice::ThinkingLevel)
        }
        Screen::Login => pick(model.login_index, model.providers.len()).map(Choice::Provider),
        Screen::Resume => {
            pick(model.resume_index, model.resume_sessions.len()).map(Choice::ResumeSession)
        }
        Screen::Tree => model
            .session_tree
            .get(model.tree_index)
            .filter(|row| row.id.is_some())
            .map(|_| Choice::TreeEntry(model.tree_index)),
        // The sheet is for reading; enter moves on to the decision.
        Screen::Trust => model.project_trust.as_ref().map(|_| Choice::TrustDecide),
        Screen::Permissions => {
            pick(model.permission_index, model.permission_rows.len()).map(Choice::Permission)
        }
        Screen::Agents => {
            let len = model.agents.as_ref().map(|s| s.agents.len()).unwrap_or(0);
            (len > 0).then(|| Choice::AgentAction {
                action: "inspect",
                index: model.agents_index % len,
            })
        }
        Screen::ContextInspector => {
            let len = model
                .context_inspector
                .as_ref()
                .map(|s| s.rows.len())
                .unwrap_or(0);
            (len > 0).then(|| Choice::ContextInspectorAction {
                action: "preview",
                index: model.context_inspector_index % len,
            })
        }
        Screen::Extensions => extension_choice(model, "info"),
        Screen::GraphRun => {
            let len = model.graph_run.as_ref().map(|s| s.tasks.len()).unwrap_or(0);
            (len > 0).then(|| Choice::GraphAction {
                action: "inspect",
                index: model
                    .graph_run
                    .as_ref()
                    .map(|s| s.selected_index)
                    .unwrap_or(0)
                    % len,
            })
        }
        _ => None,
    }
}

fn handle_overlay_key(
    model: &mut Model,
    overlay: Overlay,
    key: KeyEvent,
    data: Option<&str>,
) -> Flow {
    if overlay == Overlay::SecretInput {
        let Some(secret) = model.secret_input.as_mut() else {
            model.overlay = None;
            return Flow::Continue;
        };
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            secret.cancel();
            model.secret_input = None;
            model.overlay = None;
            return Flow::Continue;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
        {
            let Some(candidate) = secret.begin_validation() else {
                return Flow::Continue;
            };
            model.secret_input = None;
            model.overlay = None;
            return Flow::SecretInputSubmitted(SecretInputValue::new(candidate));
        }
        if key.code == KeyCode::Backspace {
            secret.backspace();
            return Flow::Continue;
        }
        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
            if let KeyCode::Char(character) = key.code {
                secret.insert_text(&character.to_string());
            }
        }
        return Flow::Continue;
    }
    if overlay == Overlay::Ask && model.rewind_modal.is_some() {
        let rewind = model.rewind_modal.as_mut().unwrap();
        if key.code == KeyCode::Esc {
            let _ = rewind.handle_key("escape");
            model.overlay = None;
            return Flow::Continue;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
        {
            let action = rewind.handle_key("enter");
            if action == "confirm" {
                model.overlay = None;
            }
            return Flow::Continue;
        }
        if key.modifiers.is_empty() {
            match key.code {
                KeyCode::Char('1') | KeyCode::Char('c') => {
                    let _ = rewind.handle_key("1");
                    return Flow::Continue;
                }
                KeyCode::Char('2') | KeyCode::Char('t') => {
                    let _ = rewind.handle_key("2");
                    return Flow::Continue;
                }
                KeyCode::Char('3') | KeyCode::Char('s') => {
                    let _ = rewind.handle_key("3");
                    return Flow::Continue;
                }
                _ => {}
            }
        }
        return Flow::Continue;
    }
    if overlay == Overlay::Ask && model.decision_modal.is_some() {
        let decision = model.decision_modal.as_mut().unwrap();
        if key.code == KeyCode::BackTab
            || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT))
        {
            return Flow::Continue;
        }
        if key.code == KeyCode::Esc {
            let _ = decision.cancel();
            model.overlay = None;
            return Flow::Continue;
        }
        if key.code == KeyCode::Tab && key.modifiers.is_empty() {
            decision.toggle_custom();
            return Flow::Continue;
        }
        if key.code == KeyCode::Up {
            decision.move_selection(-1);
            return Flow::Continue;
        }
        if key.code == KeyCode::Down {
            decision.move_selection(1);
            return Flow::Continue;
        }
        if (key.code == KeyCode::Char('i') && !decision.custom_focused)
            || (key.code == KeyCode::Char('i') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            decision.toggle_inspect();
            return Flow::Continue;
        }
        if (key.code == KeyCode::Char('d') && !decision.custom_focused)
            || (key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            let _ = decision.defer();
            model.overlay = None;
            return Flow::Continue;
        }
        if !decision.custom_focused && key.modifiers.is_empty() {
            if let KeyCode::Char(num @ '1'..='9') = key.code {
                let digit = (num as u8 - b'0') as usize;
                decision.select_number(digit);
                return Flow::Continue;
            }
        }
        if decision.custom_focused {
            match key.code {
                KeyCode::Backspace => {
                    decision.backspace();
                    return Flow::Continue;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    decision.insert_char(c);
                    return Flow::Continue;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
        {
            if decision_modal::commits_decision("enter", decision.is_valid()) {
                let _ = decision.commit();
                model.overlay = None;
            }
            return Flow::Continue;
        }
        return Flow::Continue;
    }
    let permission_approval = overlay == Overlay::Ask && model.ask.key == "/permissions";
    if permission_approval && key.modifiers.is_empty() {
        if let KeyCode::Char(number @ '1'..='5') = key.code {
            let index = (number as u8 - b'1') as usize;
            if key.kind == KeyEventKind::Press && index < model.ask.items.len() {
                model.ask_index = index;
                model.overlay_offset = None;
            }
            return Flow::Continue;
        }
    }
    let toggle_action = match overlay {
        Overlay::Instrumenta => Some("davinci.instrumenta.toggle"),
        Overlay::Sessions => Some("davinci.sessions.toggle"),
        Overlay::Cogitator => Some("davinci.cogitator.toggle"),
        Overlay::SecretInput => None,
        Overlay::Ask => None,
    };
    if toggle_action.is_some_and(|action| action_matches(model, data, action))
        || action_matches(model, data, "tui.select.cancel")
    {
        model.overlay = None;
        model.overlay_offset = None;
        model.query.clear();
        return Flow::Continue;
    }
    if key.modifiers.is_empty() && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        let rows = overlay_rows(model, overlay);
        let anchor = model
            .overlay_offset
            .or_else(|| ui::focused_row(&rows))
            .unwrap_or(0);
        let page = model.height.saturating_sub(6).max(1) as isize;
        let delta = if key.code == KeyCode::PageUp {
            -page
        } else {
            page
        };
        model.overlay_offset = Some(
            anchor
                .saturating_add_signed(delta)
                .min(rows.len().saturating_sub(3)),
        );
        return Flow::Continue;
    }
    if action_matches(model, data, "tui.select.up") {
        model.move_selection(-1);
        return Flow::Continue;
    }
    if action_matches(model, data, "tui.select.down") {
        model.move_selection(1);
        return Flow::Continue;
    }
    let confirms = if permission_approval {
        key.code == KeyCode::Enter && key.modifiers.is_empty() && key.kind == KeyEventKind::Press
    } else {
        action_matches(model, data, "tui.select.confirm")
    };
    if confirms {
        if model.overlay_offset.take().is_some() {
            return Flow::Continue;
        }
        let chosen = model.accept();
        if chosen.is_some() {
            model.overlay = None;
            model.query.clear();
        }
        return chosen.map(Flow::Choose).unwrap_or(Flow::Continue);
    }
    if overlay == Overlay::Instrumenta {
        model.overlay_offset = None;
        if action_matches(model, data, "tui.editor.deleteCharBackward") {
            model.backspace();
            return Flow::Continue;
        }
        if let KeyCode::Char(ch) = key.code {
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                model.type_char(&ch.to_string());
            }
        }
    }
    Flow::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Entry;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    #[test]
    fn voice_hit_rectangle_tracks_the_rendered_composer_rule() {
        for width in [20, 40, 100] {
            let mut m = model(width, 30);
            m.screen = Screen::Agent;
            m.overlay = None;
            m.voice.enabled = true;
            m.voice.label = "mic ctrl+t".into();
            let frame = compose_frame(&m, 30);
            let rect = frame.mic_rect.expect("visible mic");
            assert!(frame.lines[rect.y as usize].to_string().contains("[mic"));
            assert_eq!(rect.right(), width);
            assert!(rect.y < 28, "short transcript follows its content");
            assert!(compose_frame(&m, 1).mic_rect.is_none());
            m.voice.setup = true;
            assert!(compose_frame(&m, 30).mic_rect.is_none());
        }
    }

    fn model(width: u16, height: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            height,
            true,
        );
        model.cwd = "C:\\dev\\oss\\davinci-rust".into();
        model.branch = "main".into();
        model.model_name = "sonnet".into();
        model.context = (47_000, 200_000);
        model
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn row_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn a_sheet_starts_under_the_header_and_ends_with_its_hint_row() {
        let mut m = model(100, 44);
        crate::davinci::fixtures::dress_screen(&mut m, "3b");
        m.transcript = vec![Entry::user("keep this conversation visible")];
        let rows = compose(&m, 44);
        assert_eq!(rows.len(), 44);
        let title = rows
            .iter()
            .position(|row| row_text(row).trim().starts_with("Settings  "))
            .unwrap();
        let context = rows
            .iter()
            .position(|row| row_text(row).contains("keep this conversation visible"))
            .unwrap();
        assert!(context < title);
        let hint = rows
            .iter()
            .map(row_text)
            .find(|row| row.contains("Type to filter"))
            .unwrap();
        assert!(hint.contains("Esc to clear"), "{hint}");
    }

    #[test]
    fn model_picker_is_content_sized_and_keeps_the_conversation_visible_above_it() {
        let mut m = model(108, 30);
        crate::davinci::fixtures::dress_screen(&mut m, "3a");
        m.transcript = vec![Entry::user("keep this conversation visible")];
        let rows: Vec<String> = compose(&m, 30).iter().map(text).collect();
        let conversation = rows
            .iter()
            .position(|row| row.contains("keep this conversation visible"))
            .unwrap();
        let title = rows
            .iter()
            .position(|row| row.contains("Select model"))
            .unwrap();
        assert!(conversation < title, "{rows:?}");
        assert!(rows.last().unwrap().contains("Enter to set as default"));
        assert!(!rows.iter().any(|row| row.contains('╰')));
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    #[ignore = "audit dump, run by hand"]
    fn dump_every_screen_for_the_mockup_audit() {
        use crate::davinci::fixtures;

        let screens: &[(&str, u16, u16, bool)] = &[
            ("1a", 100, 44, false),
            ("1b", 100, 44, false),
            ("1c", 100, 44, false),
            ("1d", 100, 44, false),
            ("1e", 160, 44, false),
            ("1f", 100, 44, false),
            ("1f-cogitator", 100, 44, false),
            ("1g", 80, 30, false),
            ("1h", 80, 30, true),
            ("2a", 100, 44, false),
            ("2b", 100, 44, false),
            ("2c", 100, 44, false),
            ("3a", 100, 44, false),
            ("3b", 100, 44, false),
            ("3c", 100, 44, false),
            ("3d", 100, 44, false),
            ("3e", 100, 44, false),
            ("4a", 100, 44, false),
            ("4b", 100, 44, false),
            ("4c", 100, 44, false),
            ("4d", 100, 44, false),
            ("5a", 100, 44, false),
            ("5b", 100, 44, false),
            ("5c", 100, 44, false),
            ("5d", 100, 44, false),
            ("6a", 100, 44, false),
            ("6b", 100, 44, false),
            ("6c", 100, 44, false),
            ("6d", 100, 44, false),
        ];
        for &(screen, width, height, no_color) in screens {
            let mut m = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, no_color),
                width,
                height,
                true,
            );
            fixtures::dress_screen(&mut m, screen);
            m.config_path = "%USERPROFILE%\\.pi\\config.json".into();
            println!("===== {screen} {width}x{height} =====");
            for row in compose(&m, height) {
                println!("{}", text(&row));
            }
        }
    }

    #[test]
    fn the_window_is_filled_exactly_at_every_height() {
        for height in [10u16, 24, 44, 60] {
            let rows = compose(&model(100, height), height);
            assert_eq!(rows.len(), height as usize, "at height {height}");
        }
    }

    #[test]
    fn a_full_conversation_anchors_the_prompt_above_the_status_bar() {
        let mut m = model(100, 24);
        m.transcript = (0..40).map(|i| Entry::user(&format!("turn {i}"))).collect();
        let rows = compose(&m, 24);
        assert!(text(&rows[20]).chars().all(|ch| ch == '─'));
        assert!(text(&rows[21]).starts_with("❯"));
        assert!(!text(&rows[21]).contains("…"));
        assert!(text(&rows[22]).chars().all(|ch| ch == '─'));
        assert!(text(&rows[23]).starts_with("  ⏸ manual mode on"));
        assert!(text(&rows[23]).contains("? for shortcuts"));
    }

    #[test]
    fn sheet_header_and_status_bar_fill_one_row_each_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let mut m = model(width, 30);
            m.screen = Screen::Thinking;
            let rows = compose(&m, 30);
            let divider = rows
                .iter()
                .find(|row| row.to_string().starts_with('▔'))
                .unwrap();
            assert_eq!(run_width(&divider.spans), width);
            assert!(rows.iter().all(|row| run_width(&row.spans) <= width));
            assert!(rows.iter().any(|row| row.to_string().contains("esc close")));
        }
    }

    #[test]
    fn a_long_transcript_scrolls_off_the_top_like_a_scrollback() {
        let mut m = model(100, 12);
        m.transcript = (0..40).map(|i| Entry::user(&format!("turn {i}"))).collect();
        let rows = compose(&m, 12);
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("turn 0")));
        assert!(drawn.iter().any(|row| row.contains("turn 39")));
    }

    #[test]
    fn a_short_conversation_follows_the_banner_and_keeps_spare_space_below() {
        let mut m = model(100, 20);
        m.transcript = vec![Entry::user("run the tests")];
        let rows = compose(&m, 20);
        assert!(text(&rows[1]).contains("DaVinci"));
        let turn = rows
            .iter()
            .position(|row| text(row).contains("❯ run the tests"))
            .unwrap();
        let prompt = rows
            .iter()
            .rposition(|row| text(row).contains("❯"))
            .unwrap();
        assert!(turn < prompt && prompt == rows.len() - 3);
        assert!(!text(rows.last().unwrap()).is_empty());
    }

    #[test]
    fn active_overlays_own_text_and_backspace_before_the_composer() {
        for overlay in [Overlay::Sessions, Overlay::Cogitator, Overlay::Ask] {
            let mut m = model(120, 30);
            m.composer = "draft".into();
            m.overlay = Some(overlay);

            assert_eq!(handle_key(&mut m, key(KeyCode::Char('x'))), Flow::Continue);
            assert_eq!(m.composer, "draft", "{overlay:?} leaked text into composer");

            assert_eq!(handle_key(&mut m, key(KeyCode::Backspace)), Flow::Continue);
            assert_eq!(
                m.composer, "draft",
                "{overlay:?} leaked backspace into composer"
            );
        }
    }

    #[test]
    fn active_overlay_owns_surface_shortcuts_until_it_is_closed() {
        let mut m = model(120, 30);
        m.overlay = Some(Overlay::Sessions);

        assert_eq!(handle_key(&mut m, ctrl('p')), Flow::Continue);
        assert_eq!(m.overlay, Some(Overlay::Sessions));

        assert_eq!(handle_key(&mut m, ctrl('s')), Flow::Continue);
        assert_eq!(
            m.overlay, None,
            "the Sessions close chord still belongs to Sessions"
        );
    }

    #[test]
    fn active_screens_own_text_backspace_and_enter_before_the_composer() {
        for screen in [
            Screen::Plan,
            Screen::Grafo,
            Screen::Memoria,
            Screen::Mensura,
        ] {
            let mut m = model(120, 30);
            m.composer = "draft".into();
            m.screen = screen;

            assert_eq!(handle_key(&mut m, key(KeyCode::Char('x'))), Flow::Continue);
            assert_eq!(m.composer, "draft", "{screen:?} leaked text into composer");

            assert_eq!(handle_key(&mut m, key(KeyCode::Backspace)), Flow::Continue);
            assert_eq!(
                m.composer, "draft",
                "{screen:?} leaked backspace into composer"
            );

            assert_eq!(handle_key(&mut m, key(KeyCode::Enter)), Flow::Continue);
            assert_eq!(m.composer, "draft", "{screen:?} submitted the composer");
        }
    }

    #[test]
    fn ctrl_tab_cycles_modes_without_editing_or_submitting_the_draft() {
        let mut m = model(120, 30);
        m.running = false;
        m.composer.set_text("keep this café 🦀 draft");
        let current = m.permission_mode.clone();
        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL);
        assert_eq!(handle_key(&mut m, key), Flow::CyclePermissionMode);
        assert_eq!(
            m.permission_mode, current,
            "runtime must commit the transition first"
        );
        assert_eq!(m.composer, "keep this café 🦀 draft");
        m.running = true;
        assert_eq!(handle_key(&mut m, key), Flow::Continue);
        m.running = false;
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            let mut repeated = key;
            repeated.kind = kind;
            assert_eq!(handle_key(&mut m, repeated), Flow::Continue);
        }
        m.overlay = Some(Overlay::Ask);
        assert_eq!(handle_key(&mut m, key), Flow::Continue);
    }

    #[test]
    fn shift_tab_requests_mode_cycle_without_submitting_or_erasing_draft() {
        for key in [
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
        ] {
            let mut m = model(120, 30);
            m.running = false;
            m.composer.set_text("keep this café 🦀\ndraft");
            m.composer.editor_mut().move_left();
            let cursor = m.composer.editor().get_cursor();
            let thinking = m.thinking_level.clone();
            let permission = m.permission_mode.clone();
            let transcript = format!("{:?}", m.transcript);
            assert_eq!(handle_key(&mut m, key), Flow::CyclePermissionMode);
            assert_eq!(m.composer, "keep this café 🦀\ndraft");
            assert_eq!(m.composer.editor().get_cursor(), cursor);
            assert_eq!(m.thinking_level, thinking);
            assert_eq!(
                m.permission_mode, permission,
                "wait for runtime confirmation"
            );
            assert_eq!(format!("{:?}", m.transcript), transcript);
        }
    }

    #[test]
    fn shift_tab_does_not_escalate_running_modal_or_repeated_input() {
        use crossterm::event::KeyEventKind;
        let mut m = model(120, 30);
        let mut key = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        m.running = true;
        assert_eq!(handle_key(&mut m, key), Flow::Continue);
        m.running = false;
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            key.kind = kind;
            assert_eq!(handle_key(&mut m, key), Flow::Continue);
        }
        key.kind = KeyEventKind::Press;
        for overlay in [
            Overlay::Instrumenta,
            Overlay::Sessions,
            Overlay::Cogitator,
            Overlay::Ask,
        ] {
            m.overlay = Some(overlay);
            assert_eq!(handle_key(&mut m, key), Flow::Continue, "{overlay:?}");
            assert_eq!(m.overlay, Some(overlay));
        }
        m.overlay = None;
        for screen in [
            Screen::Plan,
            Screen::Models,
            Screen::Settings,
            Screen::Permissions,
            Screen::Login,
        ] {
            m.screen = screen;
            assert_eq!(handle_key(&mut m, key), Flow::Continue, "{screen:?}");
            assert_eq!(m.screen, screen);
        }
        m.screen = Screen::Agent;
        m.toggle_codex();
        assert_eq!(handle_key(&mut m, key), Flow::Continue);
        m.toggle_codex();
        m.voice.setup = true;
        assert_eq!(handle_key(&mut m, key), Flow::Continue);
    }

    #[test]
    fn unbound_modified_tab_chords_do_not_cycle_permissions() {
        for code in [KeyCode::Tab, KeyCode::BackTab] {
            for modifiers in [
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
                KeyModifiers::SHIFT | KeyModifiers::CONTROL,
                KeyModifiers::SHIFT | KeyModifiers::ALT,
                KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
            ] {
                let mut m = model(120, 30);
                m.running = false;
                m.composer.set_text("keep this draft");
                let expected = if code == KeyCode::Tab && modifiers == KeyModifiers::CONTROL {
                    Flow::CyclePermissionMode
                } else {
                    Flow::Continue
                };
                assert_eq!(handle_key(&mut m, KeyEvent::new(code, modifiers)), expected);
                assert_eq!(m.composer, "keep this draft");
                assert_eq!(m.permission_mode, "ask");
            }
        }
    }

    #[test]
    fn composer_inserts_at_the_cursor_instead_of_appending_only() {
        let mut m = model(120, 30);
        handle_key(&mut m, key(KeyCode::Char('a')));
        handle_key(&mut m, key(KeyCode::Char('c')));
        handle_key(&mut m, key(KeyCode::Left));
        handle_key(&mut m, key(KeyCode::Char('b')));

        assert_eq!(m.composer, "abc");
    }

    #[test]
    fn composer_honors_configured_editor_keybindings() {
        let mut m = model(120, 30);
        m.keybindings = crate::Keybindings::from_json(r#"{"tui.editor.cursorLeft":"ctrl+h"}"#);
        handle_key(&mut m, key(KeyCode::Char('a')));
        handle_key(&mut m, key(KeyCode::Char('c')));
        handle_key(&mut m, ctrl('h'));
        handle_key(&mut m, key(KeyCode::Char('b')));

        assert_eq!(m.composer, "abc");
    }

    #[test]
    fn davinci_surface_shortcuts_honor_configured_keybindings() {
        let mut m = model(120, 30);
        m.keybindings = crate::Keybindings::from_json(r#"{"davinci.instrumenta.toggle":"ctrl+i"}"#);

        assert_eq!(handle_key(&mut m, ctrl('p')), Flow::Continue);
        assert_eq!(
            m.overlay, None,
            "the overridden default must no longer fire"
        );

        assert_eq!(handle_key(&mut m, ctrl('i')), Flow::Continue);
        assert_eq!(m.overlay, Some(Overlay::Instrumenta));
    }

    #[test]
    fn ctrl_c_interrupts_the_run_and_never_the_app() {
        let mut m = model(100, 24);
        m.type_char("cargo test");
        m.submit();
        assert!(m.running);
        assert_eq!(handle_key(&mut m, ctrl('c')), Flow::Interrupt);
        assert!(!m.running);
        assert!(!m.transcript.is_empty());
    }

    #[test]
    fn esc_interrupts_the_run_when_working() {
        let mut m = model(100, 24);
        m.type_char("cargo test");
        m.submit();
        assert!(m.running);
        assert_eq!(handle_key(&mut m, key(KeyCode::Esc)), Flow::Interrupt);
        assert!(!m.running);
        assert!(!m.transcript.is_empty());
    }

    #[test]
    fn every_instrument_has_a_key_and_esc_closes_it() {
        let mut m = model(160, 44);
        for (ch, expected) in [
            ('l', Screen::Plan),
            ('g', Screen::Grafo),
            ('u', Screen::Mensura),
            ('r', Screen::Memoria),
        ] {
            handle_key(
                &mut m,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL | KeyModifiers::ALT),
            );
            assert_eq!(m.screen, expected);
            handle_key(&mut m, key(KeyCode::Esc));
            assert_eq!(m.screen, Screen::Agent);
        }
        for (ch, modifiers, expected) in [
            (
                'p',
                KeyModifiers::CONTROL | KeyModifiers::ALT,
                Overlay::Instrumenta,
            ),
            ('s', KeyModifiers::CONTROL, Overlay::Sessions),
            ('p', KeyModifiers::ALT, Overlay::Cogitator),
        ] {
            handle_key(&mut m, KeyEvent::new(KeyCode::Char(ch), modifiers));
            assert_eq!(m.overlay, Some(expected));
            handle_key(&mut m, key(KeyCode::Esc));
            assert_eq!(m.overlay, None);
        }
        handle_key(
            &mut m,
            KeyEvent::new(
                KeyCode::Char('e'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        );
        assert!(m.codex_open());
    }

    #[test]
    fn ctrl_m_stays_enter_so_the_composer_keeps_its_send_key() {
        let mut m = model(120, 30);
        m.type_char("run the tests");
        // The terminal delivers ctrl+m as enter; recall must not steal it.
        let flow = handle_key(&mut m, ctrl('m'));
        assert_eq!(flow, Flow::Continue);
        assert_eq!(m.screen, Screen::Agent, "ctrl+m did not open a screen");

        let flow = handle_key(&mut m, key(KeyCode::Enter));
        assert_eq!(flow, Flow::Submit("run the tests".to_string()));
    }

    #[test]
    fn submit_flow_expands_the_paste_marker_for_the_host() {
        let mut m = model(120, 30);
        let text = format!("{}\nsecond line\n", "界".repeat(1100));
        m.paste(&text);
        assert!(m.composer.contains("[paste #1"));
        assert!(!m.running);
        assert_eq!(handle_key(&mut m, key(KeyCode::Enter)), Flow::Submit(text));
        assert!(m.composer.is_empty());
    }

    #[test]
    fn whitespace_paste_does_not_start_a_phantom_turn() {
        let mut m = model(120, 30);
        m.paste(&" \n".repeat(600));
        let draft = m.composer.to_string();
        assert_eq!(handle_key(&mut m, key(KeyCode::Enter)), Flow::Continue);
        assert!(!m.running);
        assert_eq!(m.composer.to_string(), draft);
    }

    #[test]
    fn the_composer_keeps_the_three_promises_its_hint_line_makes() {
        let mut m = model(120, 30);
        m.corpus = vec![crate::davinci::model::CorpusItem::new(
            "/compact", "", "command",
        )];

        // tab complete
        for ch in "/comp".chars() {
            handle_key(&mut m, key(KeyCode::Char(ch)));
        }
        handle_key(&mut m, key(KeyCode::Tab));
        assert_eq!(m.composer, "/compact ");

        // shift+enter newline — and it must not send
        m.composer = "first".into();
        let flow = handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(flow, Flow::Continue);
        assert_eq!(m.composer, "first\n");
        // alt+enter and ctrl+j spell the same key on terminals that send them
        handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        handle_key(&mut m, ctrl('j'));
        assert_eq!(m.composer, "first\n\n\n");

        // enter send
        m.composer = "first\nsecond".into();
        let flow = handle_key(&mut m, key(KeyCode::Enter));
        assert_eq!(flow, Flow::Submit("first\nsecond".to_string()));
    }

    #[test]
    fn a_multiline_composer_is_drawn_as_the_rows_that_were_typed() {
        let mut m = model(120, 30);
        m.composer = "first\nsecond".into();
        let rows: Vec<String> = compose(&m, 30).iter().map(text).collect();
        assert!(
            rows.iter().any(|row| row.contains("❯\u{a0}first")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.contains("second")));
        assert!(rows.iter().any(|row| row.contains("? for shortcuts")));
    }

    #[test]
    fn what_is_waiting_to_be_sent_is_visible_above_what_is_being_typed() {
        let mut m = model(120, 30);
        m.running = true;
        m.queued = vec!["then commit".into(), "then push".into()];
        m.composer = "and open a pr".into();
        let rows: Vec<String> = compose(&m, 30).iter().map(text).collect();
        for expected in ["then commit", "then push", "and open a pr"] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is not on screen"
            );
        }
        // Still exactly one window's worth of rows.
        assert_eq!(compose(&m, 30).len(), 30);
    }

    #[test]
    fn extension_rows_take_their_place_without_costing_the_window_its_height() {
        let mut m = model(120, 30);
        m.transcript.push(Entry::user("run the tests"));
        m.extensions.header = vec!["branch: rust-rewrite".into()];
        m.extensions.footer = vec!["2 checks pending".into()];
        m.extensions
            .set_widget("todo", vec!["3 open todos".into()], false);
        m.extensions
            .set_widget("keys", vec!["ctrl+k commands".into()], true);
        m.extensions.set_status("sync", Some("synced"));

        let rows: Vec<String> = compose(&m, 30).iter().map(text).collect();
        assert_eq!(rows.len(), 30);
        for expected in [
            "branch: rust-rewrite",
            "3 open todos",
            "synced",
            "2 checks pending",
            "ctrl+k commands",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is not on screen"
            );
        }
        assert!(rows.iter().any(|row| row.contains("DaVinci")));
        assert!(rows.iter().any(|row| row.contains("? for shortcuts")));
    }

    #[test]
    fn typing_and_sending_a_turn() {
        let mut m = model(100, 24);
        for ch in "run the tests".chars() {
            handle_key(&mut m, key(KeyCode::Char(ch)));
        }
        assert_eq!(m.composer, "run the tests");
        handle_key(&mut m, key(KeyCode::Backspace));
        assert_eq!(m.composer, "run the test");

        let flow = handle_key(&mut m, key(KeyCode::Enter));
        assert_eq!(flow, Flow::Submit("run the test".to_string()));
        assert_eq!(m.composer, "");
    }

    #[test]
    fn enter_on_an_empty_composer_does_nothing() {
        let mut m = model(100, 24);
        assert_eq!(handle_key(&mut m, key(KeyCode::Enter)), Flow::Continue);
        assert!(m.transcript.is_empty());
    }

    #[test]
    fn what_the_composer_offers_sits_above_it_and_costs_the_window_nothing() {
        let mut m = model(100, 24);
        m.slash_commands = ["settings", "sessions"]
            .into_iter()
            .map(|name| crate::autocomplete::SlashCommandSpec {
                name: name.to_string(),
                description: format!("the {name} command"),
                argument_hint: None,
                argument_items: Vec::new(),
            })
            .collect();
        for ch in ['/', 's', 'e'] {
            handle_key(&mut m, key(KeyCode::Char(ch)));
        }
        assert_eq!(
            m.suggestions.as_ref().map(|found| found.items.len()),
            Some(2)
        );

        let rows = compose(&m, 24);
        assert_eq!(rows.len(), 24, "the window is still filled exactly");
        let text: Vec<String> = rows.iter().map(text).collect();
        let offered = text
            .iter()
            .position(|row| row.contains("settings"))
            .expect("the offered command is drawn");
        let composer = text
            .iter()
            .position(|row| row.contains("❯\u{a0}/se"))
            .expect("the composer is drawn");
        assert!(
            offered < composer,
            "the list sits above the composer: {text:?}"
        );

        // Mode cycling leaves the completion owner and Unicode caret alone;
        // ordinary Tab must still accept exactly the previously selected row.
        handle_key(&mut m, key(KeyCode::Down));
        m.running = false;
        let found = format!("{:?}", m.suggestions);
        let selected = m.suggestion_index;
        let cursor = m.composer.editor().get_cursor();
        assert_eq!(
            handle_key(&mut m, KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)),
            Flow::CyclePermissionMode
        );
        assert_eq!(m.composer, "/se");
        assert_eq!(m.composer.editor().get_cursor(), cursor);
        assert_eq!(m.suggestion_index, selected);
        assert_eq!(format!("{:?}", m.suggestions), found);
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        ] {
            let expected = if modifiers == KeyModifiers::CONTROL {
                Flow::CyclePermissionMode
            } else {
                Flow::Continue
            };
            assert_eq!(
                handle_key(&mut m, KeyEvent::new(KeyCode::Tab, modifiers)),
                expected
            );
            assert_eq!(m.composer, "/se");
        }
        handle_key(&mut m, key(KeyCode::Tab));
        assert_eq!(m.composer, "/sessions ");
    }

    #[test]
    fn a_key_that_belongs_to_the_composer_is_never_a_quit() {
        let mut m = model(100, 24);
        for ch in ['q', 'x', 'Q'] {
            assert_eq!(handle_key(&mut m, key(KeyCode::Char(ch))), Flow::Continue);
        }
        assert_eq!(m.composer, "qxQ");
        assert_eq!(handle_key(&mut m, ctrl('d')), Flow::Continue);
        assert_eq!(m.composer, "qxQ");
        m.composer.clear();
        assert_eq!(handle_key(&mut m, ctrl('d')), Flow::Quit);
    }

    /// Every state the shell can be in, so the responsive and NO_COLOR audits
    /// can walk all of them.
    fn every_surface(width: u16, height: u16) -> Vec<(String, Model)> {
        use crate::davinci::fixtures;

        let base = |screen: &str| {
            let mut model = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, false),
                width,
                height,
                true,
            );
            fixtures::dress_screen(&mut model, screen);
            model.config_path = "%USERPROFILE%\\.pi\\config.json".into();
            model
        };

        let mut all: Vec<(String, Model)> = [
            "1a", "1b", "1c", "1d", "1e", "1f", "2a", "2b", "2c", "3a", "3b", "3c", "3d", "3e",
            "4a", "4b", "4c", "4d", "5a", "5b", "5c", "5d", "6a", "6b", "6c", "6d",
        ]
        .iter()
        .map(|screen| (screen.to_string(), base(screen)))
        .collect();
        all.push(("1f-cogitator".into(), base("1f-cogitator")));
        all
    }

    #[test]
    fn no_screen_overflows_its_window_at_any_breakpoint() {
        for (width, height) in [(60u16, 20u16), (80, 24), (100, 30), (120, 40), (160, 44)] {
            for (screen, model) in every_surface(width, height) {
                let rows = compose(&model, height);
                assert_eq!(rows.len(), height as usize, "{screen} at {width}");
                for row in &rows {
                    assert!(
                        run_width(&row.spans) <= width,
                        "{screen} overflows {width}: {:?}",
                        text(row)
                    );
                }
            }
        }
    }

    #[test]
    fn below_eighty_columns_there_is_only_a_transcript_and_a_composer() {
        for (screen, model) in every_surface(60, 20) {
            assert!(!model.codex_open(), "{screen} opened a split below 80");
            assert_eq!(
                model.overlay_inset(),
                0,
                "{screen} inset a panel below 80 instead of filling the window"
            );
        }
    }

    #[test]
    fn the_codex_split_never_opens_below_a_hundred_and_twenty_columns() {
        for width in [60u16, 80, 100, 119] {
            let mut m = model(width, 30);
            m.toggle_codex();
            assert!(!m.codex_open(), "a split opened at {width}");
        }
        let mut m = model(120, 30);
        m.toggle_codex();
        assert!(m.codex_open());
    }

    #[test]
    fn no_color_leaves_every_state_readable_by_glyph_alone() {
        use crate::davinci::fixtures;

        for screen in [
            "1a", "1b", "1c", "1d", "1e", "1f", "2a", "2b", "2c", "3a", "3b", "3c", "3d", "3e",
            "4a", "4b", "4c", "4d", "5a", "5b", "5c", "5d", "6a", "6b", "6c", "6d",
        ] {
            let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 120, 40, true);
            fixtures::dress_screen(&mut m, screen);
            let rows = compose(&m, 40);

            for row in &rows {
                for span in &row.spans {
                    if let Some(ratatui::style::Color::Rgb(r, g, b)) = span.style.fg {
                        assert!(
                            r == g && g == b,
                            "{screen} drew a colored run under NO_COLOR: {:?}",
                            span.content
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_transcript_still_states_every_outcome_under_no_color() {
        use crate::davinci::fixtures;

        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 100, 40, true);
        fixtures::dress_screen(&mut m, "1b");
        let drawn: String = compose(&m, 40).iter().map(|row| text(row)).collect();

        // Screen 1h: done, failed, in progress, queued, change and read all
        // read from the glyph alone.
        for glyph in ['✓', '×', '○', '⎿'] {
            assert!(drawn.contains(glyph), "{glyph} is missing under NO_COLOR");
        }
    }

    #[test]
    fn animations_stop_under_no_animation() {
        use crate::davinci::fixtures;

        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            120,
            40,
            false,
        );
        fixtures::dress_screen(&mut m, "1b");

        let frames: Vec<String> = (0..8)
            .map(|tick| {
                m.tick = tick;
                compose(&m, 40).iter().map(|row| text(row)).collect()
            })
            .collect();
        assert!(
            frames.windows(2).all(|pair| pair[0] == pair[1]),
            "something moved with --no-animation"
        );
    }

    #[test]
    fn exactly_two_things_move_when_animation_is_on() {
        use crate::davinci::fixtures;

        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 120, 40, true);
        fixtures::dress_screen(&mut m, "1b");

        let mut moving: Vec<char> = Vec::new();
        let base: Vec<String> = {
            m.tick = 0;
            compose(&m, 40).iter().map(|row| text(row)).collect()
        };
        for tick in 1..8u64 {
            m.tick = tick;
            let frame: Vec<String> = compose(&m, 40).iter().map(|row| text(row)).collect();
            for (before, after) in base.iter().zip(&frame) {
                if before == after {
                    continue;
                }
                for (a, b) in before.chars().zip(after.chars()) {
                    if a != b {
                        moving.push(a);
                        moving.push(b);
                    }
                }
            }
        }
        moving.sort_unstable();
        moving.dedup();
        // The reference spinner and caret are the only moving glyphs.
        for ch in &moving {
            assert!(
                "◜◝◞◟·✢*✶✻✽ ".contains(*ch),
                "{ch:?} animates, and it should not"
            );
        }
    }

    #[test]
    fn nothing_breaks_below_eighty_columns() {
        let mut m = model(60, 18);
        m.transcript = vec![
            Entry::user("run the tests"),
            Entry::Gap,
            Entry::agent("davinci"),
        ];
        let rows = compose(&m, 18);
        assert_eq!(rows.len(), 18);
        for row in &rows {
            assert!(
                run_width(&row.spans) <= 60,
                "row overflows 60 columns: {:?}",
                text(row)
            );
        }
    }

    #[test]
    fn arrow_keys_and_page_up_down_scroll_feature_sheets() {
        use crate::davinci::fixtures;

        // GraphRun has spatial selection/panning, covered by graph_input_tests.
        for screen in [Screen::Governor, Screen::Vectors] {
            let mut m = model(100, 30);
            let screen_code = match screen {
                Screen::Vectors => "5b",
                Screen::Governor => "5c",
                _ => unreachable!(),
            };
            fixtures::dress_screen(&mut m, screen_code);
            assert_eq!(m.screen, screen);
            assert_eq!(m.feature_scroll, 0);

            // Down arrow scrolls by 1
            handle_key(&mut m, key(KeyCode::Down));
            assert_eq!(m.feature_scroll, 1);

            handle_key(&mut m, key(KeyCode::Down));
            assert_eq!(m.feature_scroll, 2);

            // Up arrow scrolls back by 1
            handle_key(&mut m, key(KeyCode::Up));
            assert_eq!(m.feature_scroll, 1);

            handle_key(&mut m, key(KeyCode::Up));
            assert_eq!(m.feature_scroll, 0);

            // Cannot scroll above 0
            handle_key(&mut m, key(KeyCode::Up));
            assert_eq!(m.feature_scroll, 0);

            // PageDown uses the visible section reading budget.
            handle_key(&mut m, key(KeyCode::PageDown));
            assert_eq!(m.feature_scroll, m.height.saturating_sub(6) as usize);

            // PageUp scrolls back by page
            handle_key(&mut m, key(KeyCode::PageUp));
            assert_eq!(m.feature_scroll, 0);
        }
    }
}

#[cfg(test)]
mod section_regressions {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
    };

    #[test]
    fn accepting_model_command_opens_the_picker_instead_of_argument_completion() {
        for (input, key) in [
            ("/model", KeyCode::Enter),
            ("/mo", KeyCode::Enter),
            ("/model", KeyCode::Tab),
            ("/mo", KeyCode::Tab),
        ] {
            let mut model = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, false),
                100,
                32,
                false,
            );
            model.slash_commands = vec![crate::autocomplete::SlashCommandSpec {
                name: "model".into(),
                argument_hint: Some("<provider/model>".into()),
                ..Default::default()
            }];
            model.model_names = vec!["openai-codex / gpt-6-astra".into()];
            model.composer.set_text(input);
            model.refresh_suggestions();
            let flow = handle_key(&mut model, KeyEvent::new(key, KeyModifiers::NONE));
            if key == KeyCode::Enter {
                assert_eq!(flow, Flow::Submit("/model".into()));
                assert_eq!(model.composer.to_string(), "");
            } else {
                assert_eq!(flow, Flow::Continue);
                assert_eq!(model.composer.to_string(), "/model ");
                // Tab completes the name only: no argument list pops open.
                assert!(model.suggestions.is_none(), "{input}: list opened");
                // Starting an argument brings its values back.
                handle_key(
                    &mut model,
                    KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                );
                assert!(model.suggestions.is_some(), "{input}: no values");
            }
        }
    }

    #[test]
    fn enter_after_tab_completion_runs_the_command() {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            100,
            32,
            false,
        );
        model.slash_commands = vec![crate::autocomplete::SlashCommandSpec {
            name: "model".into(),
            argument_hint: Some("<provider/model>".into()),
            ..Default::default()
        }];
        model.model_names = vec!["openai-codex / gpt-6-astra".into()];
        model.composer.set_text("/mo");
        model.refresh_suggestions();
        handle_key(&mut model, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let flow = handle_key(
            &mut model,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        let Flow::Submit(sent) = flow else {
            panic!("enter did not submit: {flow:?}");
        };
        assert_eq!(sent.trim(), "/model");
        assert_eq!(model.composer.to_string(), "");
    }

    #[test]
    fn primary_command_screens_use_open_terminal_sections() {
        for id in ["3b", "3c", "3d", "4a"] {
            let mut model =
                Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 32, false);
            fixtures::dress_screen(&mut model, id);
            let text = compose(&model, 32)
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                (!text.contains('╭') || id == "3b"),
                "{id} still renders a framed command screen: {text}"
            );
            assert!(
                text.to_lowercase().contains("esc"),
                "{id} has no visible exit"
            );
        }
    }

    #[test]
    fn model_names_remain_readable_on_narrow_terminals() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 40, 24, false);
        fixtures::dress_screen(&mut model, "3a");
        model.catalog[0].id = "recognizable-model".into();
        model.catalog_index = 0;
        let text = compose(&model, 24)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("recognizable-model"), "{text}");
    }
}

#[cfg(test)]
mod section_behavior_regressions {
    use super::*;
    use crate::davinci::views::thinking;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
    };

    fn fixture(id: &str, width: u16, height: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            height,
            false,
        );
        fixtures::dress_screen(&mut model, id);
        model
    }

    #[test]
    fn model_arrows_preview_supported_reasoning_until_confirmation() {
        let mut model = fixture("3a", 80, 32);
        model.catalog_index = 0;
        model.catalog[0].reasoning_levels = vec!["low".into(), "high".into()];
        model.catalog[0].reasoning_index = 0;
        let current = model.thinking_level.clone();
        for (key, expected) in [
            (KeyCode::Left, 0),
            (KeyCode::Right, 1),
            (KeyCode::Right, 1),
            (KeyCode::Left, 0),
        ] {
            assert!(matches!(
                handle_key(&mut model, KeyEvent::new(key, KeyModifiers::NONE)),
                Flow::Continue
            ));
            assert_eq!(model.catalog[0].reasoning_index, expected);
            assert_eq!(model.thinking_level, current);
            let rendered = cogitator::screen(&model, 30)
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(rendered.contains(&format!("● {} effort ←/→ to adjust", {
                let level = &model.catalog[0].reasoning_levels[expected];
                format!("{}{}", level[..1].to_uppercase(), &level[1..])
            })));
        }
        assert!(matches!(
            handle_key(
                &mut model,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Flow::Choose(Choice::Catalog(0))
        ));
        model.catalog[0].reasoning_levels.clear();
        handle_key(
            &mut model,
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        );
        assert_eq!(model.catalog[0].reasoning_index, 0);
        model.catalog.clear();
        handle_key(&mut model, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    }

    #[test]
    fn enter_on_an_empty_picker_is_a_no_op() {
        for screen in [
            Screen::Models,
            Screen::Settings,
            Screen::Thinking,
            Screen::Login,
            Screen::Resume,
            Screen::Permissions,
        ] {
            let mut model =
                Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 40, 12, false);
            model.screen = screen;
            assert!(screen_accept(&model).is_none(), "{screen:?}");
        }
    }

    #[test]
    fn moving_thinking_focus_does_not_change_the_current_level() {
        let mut model = fixture("3c", 80, 32);
        model.thinking_level = "low".into();
        model.thinking_index = model
            .thinking_rows
            .iter()
            .position(|r| r.level == "high")
            .unwrap();
        let status = thinking::chrome(&model)
            .status_third
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(status.contains("low"), "{status}");
        let rows = thinking::lines(&model);
        assert!(rows
            .iter()
            .any(|r| r.to_string().contains("low") && r.to_string().contains("current")));
        assert!(rows[ui::focused_row(&rows).expect("focus marker")]
            .to_string()
            .contains("high"));
    }

    #[test]
    fn primary_lists_do_not_advertise_inert_filters() {
        for id in ["3a", "4a"] {
            let model = fixture(id, 80, 32);
            let text = compose(&model, 32)
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                !text.contains("filter models") && !text.contains("filter sessions"),
                "{text}"
            );
        }
    }

    #[test]
    fn model_and_settings_keep_their_detail_summary_visible_when_selection_is_deep() {
        let mut models = fixture("3a", 80, 16);
        models.catalog_index = models.catalog.len().saturating_sub(1);
        let selected_model = models.catalog[models.catalog_index].id.clone();
        let model_text = compose(&models, 16)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(model_text.contains("Select model"), "{model_text}");
        assert!(model_text.contains(&selected_model), "{model_text}");

        let mut settings = fixture("3b", 80, 16);
        settings.settings_index = settings.settings_rows.len().saturating_sub(1);
        let selected_setting = settings.settings_rows[settings.settings_index]
            .label
            .clone();
        let settings_text = compose(&settings, 16)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            settings_text.contains("Search settings…"),
            "{settings_text}"
        );
        assert!(settings_text.contains(&selected_setting), "{settings_text}");
    }

    #[test]
    fn expanded_settings_remain_readable_at_narrow_widths() {
        let mut model = fixture("3b", 32, 24);
        model.settings_rows[0].value = "a-long-current-value".into();
        model.settings_rows[0].description = "Focused setting details".into();
        model.settings_details = true;
        model.settings_index = 0;
        let text = settings::lines(&model)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("a-long-current-value"), "{text}");
        assert!(text.contains("Focused setting details"), "{text}");
        assert!(
            !text.contains("%USERPROFILE%"),
            "the renderer must not invent a settings path"
        );
    }
}

#[cfg(test)]
mod section_truth_regressions {
    use super::*;
    use crate::davinci::{
        model::{Compaction, ExportLedger, FailedRun, ProjectTrustSheet},
        theme::{ColorDepth, Theme},
    };
    fn model() -> Model {
        Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false)
    }
    fn text(rows: Vec<Line<'static>>) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_local_export_is_not_presented_as_an_upload_or_clipboard_action() {
        let mut m = model();
        m.export_ledger = Some(ExportLedger {
            gist: "session.html".into(),
            size: "4 KB".into(),
            ..Default::default()
        });
        let drawn = text(export::lines(&m));
        assert!(drawn.contains("session.html"));
        for unsupported in ["uploaded", "clipboard", "[o]", "[c]", "[d]", "exporting"] {
            assert!(!drawn.contains(unsupported), "{drawn}");
        }
    }
    #[test]
    fn completed_compaction_does_not_offer_a_second_unwired_confirmation() {
        let mut m = model();
        m.compaction = Some(Compaction {
            before_tokens: "40k".into(),
            after_tokens: "8k".into(),
            ..Default::default()
        });
        let drawn = text(compact::lines(&m));
        assert!(drawn.contains("40k") && drawn.contains("8k"));
        for unsupported in ["compact now", "[e]", "[t]", "pays a full", "reversible"] {
            assert!(!drawn.contains(unsupported), "{drawn}");
        }
    }
    #[test]
    fn trust_review_does_not_claim_unread_files_or_unwired_decision_letters() {
        let mut m = model();
        m.project_trust = Some(ProjectTrustSheet {
            path: "example-project".into(),
            ..Default::default()
        });
        let drawn = text(trust::lines(&m));
        assert!(drawn.contains("example-project"));
        for unsupported in [
            "Nothing here has been read",
            "[t]",
            "[o]",
            "[p]",
            "[n]",
            "no tools loaded",
        ] {
            assert!(!drawn.contains(unsupported), "{drawn}");
        }
    }
    #[test]
    fn a_failure_does_not_invent_interrupt_persistence_or_retry_actions() {
        let mut m = model();
        m.failed_run = Some(FailedRun {
            error: "provider rejected the request".into(),
            ..Default::default()
        });
        let drawn = text(recovery::lines(&m));
        assert!(drawn.contains("provider rejected the request"));
        for unsupported in [
            "session written",
            "You stopped",
            "ctrl+c",
            "finish on opus",
            "retry now",
        ] {
            assert!(!drawn.contains(unsupported), "{drawn}");
        }
    }
}

#[cfg(test)]
mod section_layout_regressions {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
    };
    const IDS: &[&str] = &[
        "1c",
        "1d",
        "1f",
        "1f-cogitator",
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
        "5b",
        "5c",
        "5d",
        "6a",
        "6b",
        "6c",
        "6d",
    ];
    fn fixture(id: &str, width: u16, height: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            height,
            false,
        );
        fixtures::dress_screen(&mut m, id);
        m.width = width;
        m.height = height;
        m
    }
    #[test]
    fn every_fixture_fits_the_terminal_cells_and_requested_height() {
        for id in IDS {
            for width in [20, 32, 40, 80, 120] {
                for height in [0, 1, 2, 4, 8, 16, 32] {
                    let m = fixture(id, width, height);
                    let rows = compose(&m, height);
                    assert_eq!(rows.len(), height as usize, "{id} {width}x{height}");
                    for row in rows {
                        assert!(
                            ui::run_width(&row.spans) <= width,
                            "{id} {width}x{height}: {row:?}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn section_titles_take_priority_over_metadata_on_narrow_terminals() {
        let mut m = fixture("3a", 32, 16);
        m.catalog.resize(12000, m.catalog[0].clone());
        assert!(chrome::header(&m).to_string().contains("Select model"));
    }
    #[test]
    fn scrolling_a_status_view_is_not_pinned_to_its_running_worker_marker() {
        let mut m = fixture("5a", 80, 12);
        let before = compose(&m, 12);
        screen_move(&mut m, 200);
        assert_ne!(compose(&m, 12), before);
    }
    #[test]
    fn read_only_sections_do_not_advertise_unimplemented_actions_in_the_status_bar() {
        let m = fixture("2a", 80, 24);
        let status = chrome::status(&m).to_string();
        assert!(
            !status.contains("enter open node") && !status.contains("x expand"),
            "{status}"
        );
    }
}

#[cfg(test)]
mod section_content_regressions {
    use super::*;
    use crate::davinci::{
        fixtures,
        model::{Ask, PickerItem, SecurityScan},
        theme::{ColorDepth, Theme},
    };
    fn model(id: &str) -> Model {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 32, false);
        fixtures::dress_screen(&mut m, id);
        m
    }
    fn text(rows: Vec<Line<'static>>) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn confirmation_context_precedes_the_choices() {
        let mut m = model("1a");
        m.ask = Ask {
            title: "Permission".into(),
            note: "Read a file outside this project".into(),
            items: vec![PickerItem::new("Allow once", "This call only")],
            ..Default::default()
        };
        m.overlay = Some(Overlay::Ask);
        let drawn = text(ask::lines(&m));
        assert!(drawn.find("outside this project").unwrap() < drawn.find("Allow once").unwrap());
    }
    #[test]
    fn overlays_only_advertise_implemented_actions() {
        let mut m = model("1d");
        m.width = 120;
        let palette = text(instrumenta::lines(&m, 40));
        assert!(!palette.contains("tab complete"), "{palette}");
        m.overlay = Some(Overlay::Sessions);
        let sessions = text(memoria::sessions(&m, 40));
        assert!(
            !sessions.contains("d delete") && !sessions.contains("f fork"),
            "{sessions}"
        );
        assert!(sessions.contains("esc close"));
    }
    #[test]
    fn read_only_recall_and_worker_status_are_not_fake_pickers() {
        for (id, render) in [
            ("2b", memoria::recall as fn(&Model) -> Vec<Line<'static>>),
            ("5a", graph_run::lines),
        ] {
            let m = model(id);
            let drawn = render(&m);
            assert!(ui::focused_row(&drawn).is_none(), "{id}");
            assert!(!text(drawn).contains('▌'), "{id}");
        }
    }
    #[test]
    fn security_status_does_not_invent_validation_network_or_seal_guarantees() {
        let mut m = model("5d");
        m.security = Some(SecurityScan {
            id: "test-scan".into(),
            state: "running".into(),
            ..Default::default()
        });
        let drawn = text(securitas::lines(&m));
        for claim in [
            "report sealed",
            "never left this machine",
            "allow_network false",
            "not guessed",
        ] {
            assert!(!drawn.contains(claim), "{drawn}");
        }
    }
    #[test]
    fn plans_and_budget_advice_do_not_offer_unhandled_letter_keys() {
        let plan = text(disegno::lines(&model("1c")));
        assert!(!plan.contains("a accept") && !plan.contains("e edit step"));
        let budget = text(mensura::lines(&model("2c")));
        assert!(
            !budget.contains("[a]") && !budget.contains("[d]"),
            "{budget}"
        );
    }
}

#[cfg(test)]
mod section_input_regressions {
    use super::*;
    use crate::davinci::{
        fixtures,
        model::{Ask, PickerItem},
        theme::{ColorDepth, Theme},
    };
    fn model(id: &str) -> Model {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 40, 16, false);
        fixtures::dress_screen(&mut m, id);
        m.width = 40;
        m.height = 16;
        m
    }
    fn press(m: &mut Model, key: KeyCode) -> Flow {
        handle_key(m, KeyEvent::new(key, KeyModifiers::NONE))
    }
    fn press_mods(m: &mut Model, key: KeyCode, mods: KeyModifiers) -> Flow {
        handle_key(m, KeyEvent::new(key, mods))
    }

    #[test]
    fn secret_input_owns_ctrl_c_instead_of_interrupting_the_session() {
        let mut m = model("1a");
        m.secret_input = Some(crate::davinci::views::secret_input::SecretInputState::new());
        m.overlay = Some(Overlay::SecretInput);

        let flow = press_mods(&mut m, KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(flow, Flow::Continue);
        assert!(m.secret_input.is_none());
        assert!(m.overlay.is_none());
    }

    #[test]
    fn secret_input_preserves_shifted_key_characters() {
        let mut m = model("1a");
        m.secret_input = Some(crate::davinci::views::secret_input::SecretInputState::new());
        m.overlay = Some(Overlay::SecretInput);
        for (ch, mods) in [
            ('a', KeyModifiers::NONE),
            ('_', KeyModifiers::SHIFT),
            ('B', KeyModifiers::SHIFT),
        ] {
            press_mods(&mut m, KeyCode::Char(ch), mods);
        }
        let Flow::SecretInputSubmitted(value) = press(&mut m, KeyCode::Enter) else {
            panic!("credential was not submitted");
        };
        assert_eq!(value.into_inner(), "a_B");
        assert_eq!(&*m.composer, "");
    }

    #[test]
    fn secret_input_accepts_paste_without_putting_it_in_the_composer() {
        let mut m = model("1a");
        m.secret_input = Some(crate::davinci::views::secret_input::SecretInputState::new());
        m.overlay = Some(Overlay::SecretInput);
        m.paste("candidate");

        let flow = press(&mut m, KeyCode::Enter);

        assert!(matches!(&flow, Flow::SecretInputSubmitted(_)));
        assert!(!format!("{flow:?}").contains("candidate"));
        assert_eq!(&*m.composer, "");
    }

    #[test]
    fn f11_five_mode_cycle() {
        let mut mode = 0;
        let mut labels = Vec::new();
        for _ in 0..5 {
            mode = next_mode_index(mode);
            labels.push(mode);
        }
        assert_eq!(labels, vec![1, 2, 3, 4, 0]);
    }

    #[test]
    fn f11_shift_tab_mode_cycling_preserves_draft() {
        let mut m = model("1a");
        m.screen = Screen::Agent;
        m.permission_mode = "ask".into();
        assert_eq!(m.permission_label(), "Manual");

        let unicode_draft = "Fix #42: add 🚀 unicode draft\nsecond line";
        m.composer.set_text(unicode_draft);
        let initial_caret = m.composer.cursor();

        let expected_cycles = [
            ("edits", "Accept Edits"),
            ("read-only", "Plan Mode"),
            ("auto", "Auto Mode"),
            ("always-approve", "Always Approve"),
            ("ask", "Manual"),
        ];

        for (expected_mode, expected_label) in expected_cycles {
            let flow = press_mods(&mut m, KeyCode::BackTab, KeyModifiers::SHIFT);
            assert_eq!(flow, Flow::CyclePermissionMode);

            m.permission_mode = expected_mode.to_string();
            assert_eq!(m.permission_label(), expected_label);

            assert_eq!(&*m.composer, unicode_draft);
            assert_eq!(m.composer.cursor(), initial_caret);
        }
    }

    #[test]
    fn f11_normal_tab_preserved() {
        let mut m = model("1a");
        m.screen = Screen::Agent;
        m.permission_mode = "ask".into();
        m.composer.set_text("help");

        let flow = press(&mut m, KeyCode::Tab);
        assert_ne!(flow, Flow::CyclePermissionMode);
        assert_eq!(m.permission_mode, "ask");
    }

    #[test]
    fn f11_modal_intercepts_mode_cycling() {
        let mut m = model("1a");
        m.screen = Screen::Agent;
        m.permission_mode = "ask".into();
        m.overlay = Some(Overlay::Ask);

        let flow = press_mods(&mut m, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_ne!(flow, Flow::CyclePermissionMode);
        assert_eq!(m.permission_mode, "ask");
    }

    #[test]
    fn f11_running_turn_blocks_mode_cycling() {
        let mut m = model("1a");
        m.screen = Screen::Agent;
        m.running = true;
        m.permission_mode = "ask".into();

        let flow = press_mods(&mut m, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_ne!(flow, Flow::CyclePermissionMode);
        assert_eq!(m.permission_mode, "ask");
    }

    #[test]
    fn f11_ctrl_c_interrupts() {
        let mut m = model("1a");
        let flow = press_mods(&mut m, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(flow, Flow::Interrupt);
    }

    #[test]
    fn f11_narrow_terminal_mode_cycling() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 40, 12, false);
        fixtures::dress_screen(&mut m, "1a");
        m.screen = Screen::Agent;
        m.permission_mode = "ask".into();
        m.composer.set_text("short draft");

        let flow = press_mods(&mut m, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(flow, Flow::CyclePermissionMode);
        m.permission_mode = "edits".into();
        assert_eq!(&*m.composer, "short draft");
    }
    #[test]
    fn paging_reads_expanded_details_without_changing_the_pending_selection() {
        let mut m = model("3b");
        m.settings_index = 0;
        m.settings_rows[0].description =
            "A long setting description with important detail. ".repeat(40);
        press(&mut m, KeyCode::F(1));
        press(&mut m, KeyCode::PageDown);
        assert_eq!(m.settings_index, 0);
        assert!(m.section_offset.is_some());
        press(&mut m, KeyCode::Esc);
        press(&mut m, KeyCode::Down);
        assert_eq!(m.settings_index, 1);
        assert!(m.section_offset.is_none());
    }
    #[test]
    fn paging_a_question_does_not_choose_or_move_its_answer() {
        let mut m = model("1a");
        m.ask = Ask {
            title: "Permission".into(),
            note: "Important context. ".repeat(60),
            items: vec![
                PickerItem::new("Allow", "One call"),
                PickerItem::new("Deny", "Do not run"),
            ],
            ..Default::default()
        };
        m.toggle_overlay(Overlay::Ask);
        press(&mut m, KeyCode::PageUp);
        assert!(m.overlay_offset.is_some());
        assert_eq!(m.ask_index, 0);
        assert_eq!(m.overlay, Some(Overlay::Ask));
        press(&mut m, KeyCode::Down);
        assert_eq!(m.ask_index, 1);
        assert!(m.overlay_offset.is_none());
        press(&mut m, KeyCode::Esc);
        assert!(m.overlay.is_none());
    }
    #[test]
    fn a_sheet_error_is_visible_without_discarding_the_picker_or_composer_draft() {
        let mut m = model("3a");
        m.section_notice = Some("Provider unavailable".into());
        m.composer.push_str("saved draft");
        let drawn = compose(&m, 16)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("Provider unavailable"));
        assert!(
            drawn.contains("Esc"),
            "the cancel key stays visible: {drawn}"
        );
        press(&mut m, KeyCode::Esc);
        assert_eq!(&*m.composer, "saved draft");
    }

    #[test]
    fn graph_run_keys_do_not_edit_composer_draft() {
        let mut m = model("5a");
        m.screen = Screen::GraphRun;
        m.composer.push_str("my draft");

        // Press 'p', 'x', 'r', 'd', 'enter'
        let flow_p = handle_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                KeyCode::Char('p'),
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert!(matches!(
            flow_p,
            Flow::Choose(Choice::GraphAction {
                action: "pause_resume",
                ..
            })
        ));
        assert_eq!(&*m.composer, "my draft");

        let flow_x = handle_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                KeyCode::Char('x'),
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert!(matches!(
            flow_x,
            Flow::Choose(Choice::GraphAction { action: "stop", .. })
        ));
        assert_eq!(&*m.composer, "my draft");

        let flow_r = handle_key(
            &mut m,
            crossterm::event::KeyEvent::new(
                KeyCode::Char('r'),
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert!(matches!(
            flow_r,
            Flow::Choose(Choice::GraphAction {
                action: "retry",
                ..
            })
        ));
        assert_eq!(&*m.composer, "my draft");

        let flow_enter = handle_key(
            &mut m,
            crossterm::event::KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
        );
        assert!(matches!(flow_enter, Flow::Continue));
        assert!(m.graph_run.as_ref().unwrap().inspecting_node);
        assert_eq!(&*m.composer, "my draft");
    }
}

#[cfg(test)]
mod extension_manager_tests {
    use super::*;
    use crate::davinci::model::{ExtensionRow, ExtensionTab, ExtensionsSheet};
    use crate::davinci::theme::{ColorDepth, Theme};

    fn press(model: &mut Model, code: KeyCode) -> Flow {
        handle_key(model, KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn model() -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            100,
            30,
            false,
        );
        m.screen = Screen::Extensions;
        let row = |key: &str| ExtensionRow {
            key: key.into(),
            title: key.into(),
            status: "enabled".into(),
            can_toggle: true,
            can_delete: true,
            ..ExtensionRow::default()
        };
        m.extension_manager = Some(ExtensionsSheet {
            plugins: vec![row("one@m"), row("two@m")],
            skills: vec![ExtensionRow {
                can_delete: false,
                ..row("skill")
            }],
            ..ExtensionsSheet::default()
        });
        m
    }

    fn chosen(flow: Flow) -> Option<(&'static str, ExtensionTab, String)> {
        match flow {
            Flow::Choose(Choice::ExtensionAction { action, tab, key }) => Some((action, tab, key)),
            _ => None,
        }
    }

    #[test]
    fn arrows_and_tab_move_between_rows_and_tabs() {
        let mut m = model();
        press(&mut m, KeyCode::Down);
        assert_eq!(
            chosen(press(&mut m, KeyCode::Char('e'))).unwrap().2,
            "two@m"
        );
        press(&mut m, KeyCode::Right);
        assert_eq!(
            m.extension_manager.as_ref().unwrap().tab,
            ExtensionTab::Skills
        );
        press(&mut m, KeyCode::Tab);
        press(&mut m, KeyCode::Tab);
        assert_eq!(
            m.extension_manager.as_ref().unwrap().tab,
            ExtensionTab::Plugins
        );
        let (action, tab, key) = chosen(press(&mut m, KeyCode::Enter)).unwrap();
        assert_eq!(
            (action, tab, key.as_str()),
            ("info", ExtensionTab::Plugins, "two@m")
        );
        // Esc still closes the sheet.
        press(&mut m, KeyCode::Esc);
        assert_eq!(m.screen, Screen::Agent);
    }

    fn armed(m: &Model) -> Option<(String, &'static str)> {
        m.extension_manager.as_ref().unwrap().armed.clone()
    }

    #[test]
    fn delete_is_confirmed_only_by_y_and_other_keys_disarm_it() {
        let mut m = model();
        assert!(chosen(press(&mut m, KeyCode::Char('d'))).is_none());
        assert_eq!(armed(&m), Some(("one@m".into(), "delete")));
        // A held `d` (auto-repeat) never confirms.
        assert!(chosen(press(&mut m, KeyCode::Char('d'))).is_none());
        assert_eq!(armed(&m), Some(("one@m".into(), "delete")));
        press(&mut m, KeyCode::Char('x'));
        assert!(armed(&m).is_none());
        // `y` with nothing armed does nothing.
        assert!(chosen(press(&mut m, KeyCode::Char('y'))).is_none());
        press(&mut m, KeyCode::Char('d'));
        press(&mut m, KeyCode::Down);
        assert!(armed(&m).is_none(), "moving away cancels");
        press(&mut m, KeyCode::Char('d'));
        let (action, _, key) = chosen(press(&mut m, KeyCode::Char('y'))).unwrap();
        assert_eq!((action, key.as_str()), ("delete", "two@m"));
        assert!(armed(&m).is_none());
    }

    #[test]
    fn approving_hooks_shows_details_first_then_needs_y() {
        let mut m = model();
        m.extension_manager.as_mut().unwrap().plugins[0].can_approve = true;
        let (action, _, key) = chosen(press(&mut m, KeyCode::Char('a'))).unwrap();
        assert_eq!((action, key.as_str()), ("info", "one@m"));
        assert_eq!(armed(&m), Some(("one@m".into(), "approve")));
        assert!(chosen(press(&mut m, KeyCode::Char('a'))).unwrap().0 == "info");
        let (action, _, _) = chosen(press(&mut m, KeyCode::Char('y'))).unwrap();
        assert_eq!(action, "approve");
    }

    #[test]
    fn keys_a_row_does_not_allow_do_nothing_and_pages_move_the_selection() {
        let mut m = model();
        assert!(chosen(press(&mut m, KeyCode::Char('u'))).is_none());
        press(&mut m, KeyCode::PageDown);
        assert_eq!(
            m.extension_manager.as_ref().unwrap().current().unwrap().key,
            "two@m"
        );
        press(&mut m, KeyCode::PageUp);
        assert_eq!(
            m.extension_manager.as_ref().unwrap().current().unwrap().key,
            "one@m"
        );
        press(&mut m, KeyCode::Right);
        assert!(chosen(press(&mut m, KeyCode::Char('d'))).is_none());
        assert!(armed(&m).is_none());
    }
}
