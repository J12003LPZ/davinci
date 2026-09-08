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

use crate::interaction::{apply_editor_key, input_owner, key_event_bytes, InputOwner};

use super::model::{Choice, Model, Overlay, Screen};
use super::ui::{self, blank, pad_to, tail};
use super::views::chrome::{self, Hint};
use super::views::sheet::{self, Composer};
use super::views::{
    ask, codex, cogitator, compact, diff, disegno, export, governor, grafo, graph_run, instrumenta,
    keys, login, mcp, memoria, mensura, officina, opera, permissions, recovery, resume, securitas,
    settings, startup, transcript, tree, trust, vectors, workflows,
};

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
}

/// Compose exactly `height` rows. Conversation chrome follows the content;
/// command sheets and overlays retain their full-height frame.
pub fn compose(model: &Model, height: u16) -> Vec<Line<'static>> {
    compose_frame(model, height).lines
}

pub struct ComposedFrame {
    pub lines: Vec<Line<'static>>,
    pub mic_rect: Option<ratatui::layout::Rect>,
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
        };
    }
    if height == 0 {
        return ComposedFrame {
            lines: Vec::new(),
            mic_rect: None,
        };
    }
    if model.screen == Screen::Models && model.overlay.is_none() {
        let picker = cogitator::screen(model, cogitator::screen_height(model).min(height));
        let picker_height = picker.len();
        let mut lines = transcript::tail_lines(
            model,
            &model.transcript,
            model.width,
            height.saturating_sub(picker_height),
        );
        while lines.len() < height.saturating_sub(picker_height) {
            lines.insert(0, blank());
        }
        lines.extend(picker);
        return ComposedFrame {
            lines: pad_to(lines, height),
            mic_rect: None,
        };
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
        + 1;
    let body_height = height.saturating_sub(reserved);

    let mut rows = Vec::with_capacity(height);
    if !conversation {
        rows.push(chrome::header(chrome_model));
    }
    rows.extend(top);
    rows.extend(body(model, body_height));
    rows.extend(bottom);
    rows.extend(above);
    rows.extend(working);
    rows.extend(notice);
    rows.extend(offered);
    let mic_rect = chrome::mic_geometry(model).and_then(|(x, width)| {
        if height >= 4 && !composer_rows.is_empty() && rows.len() + 1 < height {
            Some(ratatui::layout::Rect::new(x, rows.len() as u16, width, 1))
        } else {
            None
        }
    });
    rows.extend(composer_rows);
    rows.extend(below);
    rows.push(chrome::status(chrome_model));
    rows.truncate(height);
    if !conversation {
        rows = rows
            .into_iter()
            .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
            .collect();
    }
    ComposedFrame {
        lines: pad_to(rows, height),
        mic_rect,
    }
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
    let hint = if model.overlay.is_some() || model.height < 8 || model.codex_open() {
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
        Screen::Keys => Some(keys::lines(model)),
        Screen::Agent => None,
    }
}

fn body(model: &Model, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if let Some(overlay) = model.overlay {
        return overlay_body(model, overlay, height);
    }
    if let Some(rows) = section_rows(model) {
        return panel(model, rows, height);
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
fn panel(model: &Model, rows: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
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
        Overlay::Ask => ask::lines(model),
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
    if model.voice.blocks_send && voice_send_key(model, &key) {
        model.voice.notice = "Finish or cancel voice before sending".into();
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
        model.screen != Screen::Agent || model.codex_open(),
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
        // several of them (`ctrl+u` mensura, `ctrl+b` codex, `ctrl+d` quit)
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
            let sent = model.composer.to_string();
            model.submit();
            return if sent.trim().is_empty() {
                Flow::Continue
            } else {
                Flow::Submit(sent)
            };
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
    // Tab and enter both take the marked row: tab because that is what the
    // hint row promises, enter because a list in hand means the user is
    // choosing a command, not sending one.
    let tab = bindings.matches(data, "tui.input.tab");
    let model_command = model.suggestions.as_ref().is_some_and(|found| {
        found.prefix.starts_with('/')
            && found
                .items
                .get(model.suggestion_index)
                .is_some_and(|item| item.value == "model")
    });
    // Choosing the `/model` command itself opens the full picker for both
    // advertised acceptance keys. Letting Tab merely expand to `/model `
    // dropped into argument autocomplete, whose compact list cannot own the
    // selected model's reasoning level.
    if (tab || bindings.matches(data, "tui.input.submit")) && model_command {
        model.composer.set_text("/model");
        model.submit();
        return Some(Flow::Submit("/model".into()));
    }
    if tab || bindings.matches(data, "tui.input.submit") {
        if model.accept_suggestion() || tab {
            return Some(Flow::Continue);
        }
        // Enter on a row the composer already holds sends it instead.
        return None;
    }
    if bindings.matches(data, "tui.select.cancel") {
        model.dismiss_suggestions();
        return Some(Flow::Continue);
    }
    None
}

fn action_matches(model: &Model, data: Option<&str>, action: &str) -> bool {
    data.is_some_and(|data| model.keybindings.matches(data, action))
}

fn handle_global_key(model: &mut Model, data: &str) -> Option<Flow> {
    if model.keybindings.matches(data, "davinci.quit") {
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
    if model.codex_open() {
        if action_matches(model, data, "davinci.codex.toggle")
            || action_matches(model, data, "tui.select.cancel")
        {
            model.toggle_codex();
        }
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
    )
}

fn screen_move(model: &mut Model, delta: isize) {
    model.section_offset = None;
    use super::model::wrap_index;
    match model.screen {
        Screen::Models => {
            model.catalog_index = wrap_index(model.catalog_index, delta, model.catalog.len());
        }
        Screen::Settings => {
            model.settings_index =
                wrap_index(model.settings_index, delta, model.settings_rows.len());
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
            if nodes.is_empty() {
                return;
            }
            let at = nodes
                .iter()
                .position(|&index| index == model.tree_index)
                .unwrap_or(0);
            model.tree_index = nodes[wrap_index(at, delta, nodes.len())];
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
        Screen::GraphRun | Screen::Governor | Screen::Vectors | Screen::Workflows => {
            let count = match model.screen {
                Screen::GraphRun => graph_run::lines(model).len(),
                Screen::Governor => governor::lines(model).len(),
                Screen::Workflows => workflows::lines(model).len(),
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
        _ => {
            let last = section_rows(model)
                .map(|rows| rows.len().saturating_sub(1))
                .unwrap_or(0);
            model.feature_scroll = model.feature_scroll.saturating_add_signed(delta).min(last);
        }
    }
}

/// What enter means on the open sheet, if it means anything.
fn screen_accept(model: &Model) -> Option<Choice> {
    let pick = |index: usize, len: usize| (len > 0).then(|| index % len);
    match model.screen {
        Screen::Models => pick(model.catalog_index, model.catalog.len()).map(Choice::Catalog),
        Screen::Settings => {
            pick(model.settings_index, model.settings_rows.len()).map(Choice::Setting)
        }
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
        _ => None,
    }
}

fn handle_overlay_key(
    model: &mut Model,
    overlay: Overlay,
    key: KeyEvent,
    data: Option<&str>,
) -> Flow {
    let toggle_action = match overlay {
        Overlay::Instrumenta => Some("davinci.instrumenta.toggle"),
        Overlay::Sessions => Some("davinci.sessions.toggle"),
        Overlay::Cogitator => Some("davinci.cogitator.toggle"),
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
    if action_matches(model, data, "tui.select.confirm") {
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
        let rows = compose(&m, 44);
        let body_first = row_text(&rows[1]);
        assert!(
            !body_first.trim().is_empty(),
            "first body row is blank: {body_first:?}"
        );
        // 3b draws no composer, so the hint row sits directly above the
        // status bar.
        let hint = rows.iter().rev().nth(1).map(row_text).unwrap();
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
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
            .expect("the conversation remains visible");
        let command = rows
            .iter()
            .position(|row| row.contains("> /model"))
            .expect("the picker command is visible");
        let bottom = rows
            .iter()
            .rposition(|row| row.contains('╰'))
            .expect("the picker has a bottom border");

        assert!(conversation < command, "{rows:?}");
        assert_eq!(bottom, rows.len() - 1, "the picker is bottom anchored");
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
        assert!(text(&rows[19]).chars().all(|ch| "━╸┄╺".contains(ch)));
        assert!(text(&rows[20]).contains("❯"));
        assert!(!text(&rows[20]).contains("…"), "no placeholder prose");
        assert!(text(&rows[21]).chars().all(|ch| "━╸┄╺".contains(ch)));
        assert!(text(&rows[22]).contains("/help for shortcuts"));
        assert!(text(&rows[23]).starts_with("  Manual · main"));
    }

    #[test]
    fn sheet_header_and_status_bar_fill_one_row_each_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let mut m = model(width, 30);
            m.screen = Screen::Settings;
            let rows = compose(&m, 30);
            assert_eq!(run_width(&rows[0].spans), width);
            assert_eq!(run_width(&rows[29].spans), width);
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
        assert!(text(&rows[1]).contains("DAVINCI"));
        let turn = rows
            .iter()
            .position(|row| text(row).contains("> run the tests"))
            .unwrap();
        let prompt = rows.iter().position(|row| text(row).contains("❯")).unwrap();
        assert!(turn < prompt && prompt < 10);
        assert!(text(rows.last().unwrap()).is_empty());
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
            // ctrl+m in the spec; see the note in `handle_key`.
            ('r', Screen::Memoria),
        ] {
            handle_key(&mut m, ctrl(ch));
            assert_eq!(m.screen, expected, "ctrl+{ch}");
            handle_key(&mut m, key(KeyCode::Esc));
            assert_eq!(m.screen, Screen::Agent);
        }
        for (ch, expected) in [
            ('p', Overlay::Instrumenta),
            ('s', Overlay::Sessions),
            ('o', Overlay::Cogitator),
        ] {
            handle_key(&mut m, ctrl(ch));
            assert_eq!(m.overlay, Some(expected), "ctrl+{ch}");
            handle_key(&mut m, key(KeyCode::Esc));
            assert_eq!(m.overlay, None);
        }
        handle_key(&mut m, ctrl('e'));
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
        assert!(rows.iter().any(|row| row.contains("❯ first")), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("second")));
        // With more than one row in hand, the hint says how to end it.
        assert!(rows.iter().any(|row| row.contains("shift+enter newline")));
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
        assert!(rows.iter().any(|row| row.contains("DAVINCI")));
        assert!(rows.iter().any(|row| row.contains("23% context")));
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
            .position(|row| row.contains("/se"))
            .expect("the composer is drawn");
        assert!(offered < composer, "the list sits above the composer");

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
        for glyph in ['✓', '×', '○', 'Δ', '↳', '⌕'] {
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
        // The spinner's four frames and the caret's two states, nothing else.
        for ch in &moving {
            assert!("◜◝◞◟ ".contains(*ch), "{ch:?} animates, and it should not");
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

        for screen in [Screen::GraphRun, Screen::Governor, Screen::Vectors] {
            let mut m = model(100, 30);
            let screen_code = match screen {
                Screen::GraphRun => "5a",
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
                ..Default::default()
            }];
            model.model_names = vec!["openai-codex / gpt-6-astra".into()];
            model.composer.set_text(input);
            model.refresh_suggestions();
            assert_eq!(
                handle_key(&mut model, KeyEvent::new(key, KeyModifiers::NONE)),
                Flow::Submit("/model".into())
            );
            assert_eq!(model.composer.to_string(), "");
        }
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
                !text.contains('╭'),
                "{id} still renders a framed command screen: {text}"
            );
            assert!(text.contains("esc"), "{id} has no visible exit");
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
            assert!(rendered.contains(&format!(
                "Reasoning: ◀ {} ▶",
                model.catalog[0].reasoning_levels[expected]
            )));
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
        assert!(model_text.contains("SELECT A MODEL"), "{model_text}");
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
        assert!(settings_text.contains("SETTING DETAILS"), "{settings_text}");
        assert!(settings_text.contains(&selected_setting), "{settings_text}");
    }

    #[test]
    fn expanded_settings_remain_readable_at_narrow_widths() {
        let mut model = fixture("3b", 32, 24);
        model.settings_rows[0].value = "a-long-current-value".into();
        model.settings_rows[0].description = "Focused setting details".into();
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
        assert!(chrome::header(&m).to_string().contains("SELECT MODEL"));
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
    #[test]
    fn paging_reads_expanded_details_without_changing_the_pending_selection() {
        let mut m = model("3b");
        m.settings_index = 0;
        m.settings_rows[0].description =
            "A long setting description with important detail. ".repeat(40);
        press(&mut m, KeyCode::PageDown);
        assert_eq!(m.settings_index, 0);
        assert!(m.section_offset.is_some());
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
        assert!(drawn.contains("esc close"));
        assert!(!drawn.contains("saved draft"));
        press(&mut m, KeyCode::Esc);
        assert_eq!(&*m.composer, "saved draft");
    }
}
