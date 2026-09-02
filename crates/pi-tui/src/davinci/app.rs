//! The app shell: compose a window's worth of rows, route a key, paint.
//!
//! Layout is assembled as a flat list of rows rather than through nested
//! ratatui `Layout` splits, so the composer can be anchored to the bottom of
//! the window at any height and an overlay can dim the transcript behind it by
//! rendering it with the dropped ramp (design.md §1, §2).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/app.ex`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;

use crate::interaction::{apply_editor_key, input_owner, key_event_bytes, InputOwner};

use super::model::{Choice, Model, Overlay, Screen};
use super::ui::{self, blank, pad_to, tail};
use super::views::chrome::{self, Hint};
use super::views::sheet::{self, Composer};
use super::views::{
    ask, codex, cogitator, compact, diff, disegno, export, governor, grafo, graph_run, instrumenta,
    keys, login, mcp, memoria, mensura, officina, opera, permissions, recovery, resume, securitas,
    settings, startup, transcript, tree, trust, vectors,
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
    /// Cycle the active model's thinking/reasoning level.
    CycleThinking,
    /// The composer was sent; the caller owns what happens next.
    Submit(String),
    /// A row of the open instrument was chosen; the caller owns the action.
    Choose(Choice),
}

/// Compose exactly `height` rows: header, body, composer, status bar.
pub fn compose(model: &Model, height: u16) -> Vec<Line<'static>> {
    let height = height.max(4) as usize;
    // While an instrument floats over the transcript, the chrome around it
    // drops its ramp too — only the panel keeps full ink (`1d`, `1f`).
    let dimmed = model.overlay.map(|_| Model {
        theme: model.theme.dim(),
        ..model.clone()
    });
    let chrome_model = dimmed.as_ref().unwrap_or(model);

    let composer_rows = composer_rows(chrome_model);
    // What the composer offers sits directly above it, inside the same stack,
    // so the list moves with the composer at any window height.
    let offered = chrome::suggestions(chrome_model);
    let top = extension_rows(model, &model.extensions.header);
    let bottom = extension_rows(model, &model.extensions.footer);
    let above = extension_rows(model, &model.extensions.above());
    let below = extension_rows(model, &model.extensions.below());
    // The working line is a block of its own, so it never runs into the last
    // transcript row (design.md §3). It is pinned here rather than pushed into
    // the transcript so what a running turn has cost stays put while the
    // transcript scrolls under it.
    let mut working = opera::lines(chrome_model);
    if !working.is_empty() {
        working.insert(0, blank());
    }
    let reserved = 1
        + top.len()
        + bottom.len()
        + above.len()
        + working.len()
        + offered.len()
        + composer_rows.len()
        + below.len()
        + 1;
    let body_height = height.saturating_sub(reserved);

    let mut rows = Vec::with_capacity(height);
    rows.push(chrome::header(chrome_model));
    rows.extend(top);
    rows.extend(body(model, body_height));
    rows.extend(bottom);
    rows.extend(above);
    rows.extend(working);
    rows.extend(offered);
    rows.extend(composer_rows);
    rows.extend(below);
    rows.push(chrome::status(chrome_model));
    rows.truncate(height);
    rows
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
    // The hint row appears only where the mockups draw it: a plain transcript
    // or the plan sheet, wide enough for hints, with something under way. An
    // open instrument, a summoned screen, the Codex split, the narrow window
    // and the untouched empty state all leave the composer bare (`1a`, `1d`,
    // `1e`, `1g`, `2a`–`2c`).
    let untouched =
        model.transcript.is_empty() && model.composer.is_empty() && model.queued.is_empty();
    // A command sheet says what sits under it; its hint row is the only hint
    // row (design.md §11).
    if let Some(sheet) = sheet::chrome(model) {
        match sheet.composer {
            Composer::Hidden => return Vec::new(),
            Composer::Disabled(text) => return chrome::disabled_composer(model, text),
            Composer::Prompt(_) => {
                if model.queued.is_empty() {
                    return chrome::composer(model, None, Hint::None);
                }
                let mut rows: Vec<String> = model.queued.clone();
                rows.extend(model.composer.split('\n').map(str::to_string));
                return chrome::composer(model, Some(&rows), Hint::None);
            }
        }
    }
    let hint = if model.overlay.is_some() || model.minimal() || model.codex_open() {
        Hint::None
    } else {
        match model.screen {
            Screen::Memoria | Screen::Trust => Hint::None,
            Screen::Agent if untouched => Hint::None,
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

/// The transcript, bottom-anchored: older rows fall off the top like a
/// scrollback, and a short transcript is padded so the composer stays put.
/// An empty transcript is the empty state instead, centred in the body (`1a`).
fn body(model: &Model, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if let Some(overlay) = model.overlay {
        return overlay_body(model, overlay, height);
    }
    // The reference sheet is the one surface that needs the height: `tail`
    // keeps the *last* rows, right for a transcript and wrong for a sheet,
    // where it would silently eat the first group.
    if model.screen == Screen::Keys {
        return panel(model, keys::lines(model, height), height);
    }
    let screen_rows = match model.screen {
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
        Screen::Agent | Screen::Keys => None,
    };
    if let Some(rows) = screen_rows {
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
    if rows.len() < height {
        let lead = height - rows.len();
        let mut padded = vec![blank(); lead];
        padded.extend(rows);
        rows = padded;
    }
    pad_to(rows, height)
}

/// The identity mark and what the session found, vertically centred. If the
/// window is too short for the mark, the mark goes rather than the words.
fn empty_state(model: &Model, height: usize) -> Vec<Line<'static>> {
    let mut rows = startup::lines(model, &model.startup);
    if rows.len() > height {
        rows = rows.split_off(rows.len() - height);
    }
    let lead = (height - rows.len()) / 2;
    let mut out = vec![blank(); lead];
    out.extend(rows);
    pad_to(out, height)
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
        out.push(Line::from(vec![
            ui::span(format!("{} ", super::theme::glyph::USER), th.muted),
            ui::span(echo.clone(), th.muted),
        ]));
        out.push(blank());
    }
    let hint_rows = usize::from(hint.is_some());
    let room = height.saturating_sub(out.len()).saturating_sub(hint_rows);
    out.extend(ui::window(rows, room, model.sheet_anchor(), th));
    let mut out = pad_to(out, height.saturating_sub(hint_rows));
    if let Some(hint) = hint {
        out.push(hint);
    }
    out.truncate(height);
    out
}

/// An instrument in hand: the transcript stays visible behind it with the ramp
/// dropped, and the panel is drawn over it, anchored above the composer
/// (design.md §2, screens `1d` and `1f`).
fn overlay_body(model: &Model, overlay: Overlay, height: usize) -> Vec<Line<'static>> {
    let dimmed = Model {
        theme: model.theme.dim(),
        overlay: None,
        ..model.clone()
    };
    let mut behind = body(&dimmed, height);

    let panel = match overlay {
        Overlay::Instrumenta => instrumenta::lines(model, height),
        Overlay::Sessions => memoria::sessions(model, height),
        Overlay::Cogitator => cogitator::lines(model, &model.config_path),
        Overlay::Ask => ask::lines(model),
    };

    let panel = if panel.len() > height {
        tail(panel, height)
    } else {
        panel
    };
    behind.truncate(height - panel.len());
    behind.extend(panel);
    behind
}

/// Route one key. `esc` closes the instrument in hand, `ctrl+c` interrupts the
/// run and never the app (design.md §6).
pub fn handle_key(model: &mut Model, key: KeyEvent) -> Flow {
    let data = key_event_bytes(&key);

    if action_matches(model, data.as_deref(), "davinci.interrupt") {
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
    if model.keybindings.matches(data, "app.thinking.cycle") {
        return Some(Flow::CycleThinking);
    }
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
fn screen_move(model: &mut Model, delta: isize) {
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
            model.keys_offset = model.keys_offset.saturating_add_signed(delta);
        }
        Screen::Permissions => {
            model.permission_index =
                wrap_index(model.permission_index, delta, model.permission_rows.len());
        }
        _ => {}
    }
}

/// What enter means on the open sheet, if it means anything.
fn screen_accept(model: &Model) -> Option<Choice> {
    let pick = |index: usize, len: usize| (len > 0).then_some(index % len);
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
        Screen::Trust => Some(Choice::TrustDecide),
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
        model.query.clear();
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
        let chosen = model.accept();
        model.overlay = None;
        model.query.clear();
        return chosen.map(Flow::Choose).unwrap_or(Flow::Continue);
    }
    if overlay == Overlay::Instrumenta {
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
    #[ignore = "until 3b wears its artboard chrome"]
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
    fn the_composer_is_anchored_to_the_bottom_above_the_status_bar() {
        let mut m = model(100, 24);
        m.transcript.push(Entry::user("run the tests"));
        let rows = compose(&m, 24);
        assert_eq!(text(&rows[19]).chars().next(), Some('╭'));
        assert!(text(&rows[20]).contains("›"));
        assert!(!text(&rows[20]).contains("…"), "no placeholder prose");
        assert_eq!(text(&rows[21]).chars().next(), Some('╰'));
        assert!(text(&rows[22]).contains("enter send"));
        assert!(text(&rows[23]).starts_with("agent · main"));
    }

    #[test]
    fn the_header_and_status_bar_are_one_row_each_at_every_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width, 30);
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
    fn a_short_transcript_sits_above_the_composer_not_under_the_header() {
        let mut m = model(100, 20);
        m.transcript = vec![Entry::user("run the tests")];
        let rows = compose(&m, 20);
        assert!(text(&rows[1]).is_empty(), "the gap is above the turn");
        assert!(
            text(&rows[14]).contains("> run the tests"),
            "the turn sits directly above the composer, not under the header"
        );
        assert_eq!(text(&rows[15]).chars().next(), Some('╭'));
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
    fn shift_tab_requests_thinking_cycle_when_composer_owns_input() {
        let mut m = model(120, 30);

        assert_eq!(
            handle_key(&mut m, KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),),
            Flow::CycleThinking,
        );
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
        assert!(rows.iter().any(|row| row.contains("› first")), "{rows:?}");
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
        // The header stays the shell's, and the status bar keeps its meter.
        assert!(rows[0].contains("davinci"));
        assert!(rows[29].contains("/200k"), "{}", rows[29]);
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

        // Down then tab takes the second row; esc would have closed it.
        handle_key(&mut m, key(KeyCode::Down));
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
}
