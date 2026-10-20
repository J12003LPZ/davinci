//! The agent roster and worker focus: TUI-only state.
//!
//! Focus is presentation. Viewing a worker changes what is drawn and where the
//! composer's text will go if it is sent; it never cancels, queues, approves or
//! reroutes anything on its own. Worker identity is the agent id the host
//! gave, never a row index, so a reorder cannot send a click or a message to
//! the wrong worker.

use std::collections::{HashMap, VecDeque};

use super::model::{AgentRow, Model};

/// Activity lines kept per worker.
pub const WORKER_RING: usize = 200;
/// One line's bytes.
pub const WORKER_LINE_BYTES: usize = 2048;
/// Message receipts kept per worker.
pub const RECEIPT_RING: usize = 20;
/// The id the roster gives the main agent's row.
pub const MAIN_ROW: &str = "main";

/// What the main area shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ViewScope {
    #[default]
    Main,
    Worker(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLine {
    pub sequence: u64,
    /// One line of plain text; the host already removed controls and secrets
    /// and the view removes them again before drawing.
    pub text: String,
    pub kind: WorkerLineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerLineKind {
    Started,
    Call,
    Finished,
    Failed,
}

/// A message to a worker and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// Local order, so the row exists before the host has an id for it.
    pub local: u64,
    /// The stable message id, once the host journaled it.
    pub id: Option<String>,
    /// First characters of the text.
    pub text: String,
    /// `sending`, `queued`, `delivered`, `applied`, `rejected`, `unknown`.
    pub state: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerViewState {
    pub agent_id: String,
    /// Rows scrolled back from the newest line.
    pub scroll: usize,
    /// Highest activity sequence applied: the host asks for what came after.
    pub cursor: u64,
    pub lines: VecDeque<WorkerLine>,
    /// Lines the host or this ring dropped; shown as a gap marker.
    pub dropped: u64,
    /// Composer text kept while another scope is shown.
    pub draft: String,
    pub receipts: VecDeque<Receipt>,
    /// Read-only changed files of the worker's worktree, when probed.
    pub changed_paths: Vec<String>,
}

/// A message the user sent from a worker's composer, waiting for the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerOutbound {
    pub local: u64,
    pub agent_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterState {
    /// The `agentFocusUi` setting. Off draws exactly what drew before.
    pub enabled: bool,
    pub scope: ViewScope,
    pub workers: Vec<AgentRow>,
    /// Keyboard focus in the roster list: 0 is main, then the workers.
    pub focus: Option<usize>,
    pub views: HashMap<String, WorkerViewState>,
    /// The main composer's text while a worker is shown.
    pub main_draft: String,
    pub outbox: Vec<WorkerOutbound>,
    next_local: u64,
    /// Agent ids of the roster rows drawn last frame, in row order. A click
    /// resolves through this, so it names the worker the user saw.
    pub hit: std::cell::RefCell<Vec<String>>,
}

impl Default for RosterState {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: ViewScope::Main,
            workers: Vec::new(),
            focus: None,
            views: HashMap::new(),
            main_draft: String::new(),
            outbox: Vec::new(),
            next_local: 0,
            hit: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// A line made safe for any cell: no control characters, one row.
pub fn plain(text: &str) -> String {
    let stripped = super::sanitize::terminal_safe(text);
    let mut out: String = stripped
        .chars()
        .map(|ch| if ch == '\n' || ch == '\t' { ' ' } else { ch })
        .collect();
    if out.len() > WORKER_LINE_BYTES {
        let mut cut = WORKER_LINE_BYTES - '…'.len_utf8();
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push('…');
    }
    out
}

impl Model {
    /// The roster is drawn: the setting is on and there is a worker to show
    /// (or one is being viewed).
    pub fn roster_visible(&self) -> bool {
        self.roster.enabled
            && (!self.roster.workers.is_empty() || self.roster.scope != ViewScope::Main)
    }

    pub fn viewing_worker(&self) -> Option<&str> {
        match &self.roster.scope {
            ViewScope::Worker(id) => Some(id.as_str()),
            ViewScope::Main => None,
        }
    }

    /// The row of the worker in view, when the roster still lists it.
    pub fn viewed_row(&self) -> Option<&AgentRow> {
        let id = self.viewing_worker()?;
        self.roster.workers.iter().find(|row| row.id == id)
    }

    /// `Message @name` while a worker is in view.
    pub fn recipient_label(&self) -> Option<String> {
        let id = self.viewing_worker()?;
        let name = self
            .viewed_row()
            .map(|row| plain(&row.name))
            .unwrap_or_else(|| id.chars().take(8).collect());
        Some(format!("Message @{name}"))
    }

    /// Replace the list of workers. The view stays on its worker by id even
    /// if the list reorders or no longer carries it (a finished worker's
    /// lines stay readable).
    pub fn set_workers(&mut self, rows: Vec<AgentRow>) {
        self.roster.workers = rows;
        let len = self.roster.workers.len() + 1;
        if let Some(focus) = self.roster.focus.as_mut() {
            *focus = (*focus).min(len - 1);
        }
        self.dirty = true;
    }

    /// Show a worker's transcript. The composer's text is set aside and the
    /// worker's own draft takes its place; nothing is sent or cancelled.
    pub fn focus_agent(&mut self, agent_id: &str) {
        if agent_id == MAIN_ROW {
            self.focus_main();
            return;
        }
        if self.viewing_worker() == Some(agent_id) {
            self.roster.focus = None;
            return;
        }
        self.park_composer();
        self.roster.scope = ViewScope::Worker(agent_id.to_string());
        self.roster.focus = None;
        let draft = self
            .roster
            .views
            .entry(agent_id.to_string())
            .or_insert_with(|| WorkerViewState {
                agent_id: agent_id.to_string(),
                ..WorkerViewState::default()
            })
            .draft
            .clone();
        self.composer.set_text(draft);
        self.dismiss_suggestions();
        self.dirty = true;
    }

    /// Back to the main agent. Never interrupts, never clears a queue.
    pub fn focus_main(&mut self) {
        self.roster.focus = None;
        if self.roster.scope == ViewScope::Main {
            return;
        }
        self.park_composer();
        self.roster.scope = ViewScope::Main;
        let draft = std::mem::take(&mut self.roster.main_draft);
        self.composer.set_text(draft);
        self.dismiss_suggestions();
        self.dirty = true;
    }

    /// Put the composer's text where it belongs for the scope being left.
    fn park_composer(&mut self) {
        let text = self.composer.editor().get_text().to_string();
        match self.roster.scope.clone() {
            ViewScope::Main => self.roster.main_draft = text,
            ViewScope::Worker(id) => {
                if let Some(view) = self.roster.views.get_mut(&id) {
                    view.draft = text;
                }
            }
        }
    }

    /// Rows in the roster list: main, then the workers.
    pub fn roster_len(&self) -> usize {
        self.roster.workers.len() + 1
    }

    /// Where the roster's own cursor starts: the row in view.
    fn roster_current(&self) -> usize {
        match self.viewing_worker() {
            None => 0,
            Some(id) => self
                .roster
                .workers
                .iter()
                .position(|row| row.id == id)
                .map_or(0, |at| at + 1),
        }
    }

    pub fn roster_focus_enter(&mut self) {
        self.roster.focus = Some(self.roster_current());
        self.dirty = true;
    }

    pub fn roster_focus_move(&mut self, delta: isize) {
        let current = self.roster.focus.unwrap_or_else(|| self.roster_current());
        let last = self.roster_len() - 1;
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            (current + delta as usize).min(last)
        };
        self.roster.focus = Some(next);
        self.dirty = true;
    }

    /// Open the row the keyboard is on.
    pub fn roster_focus_activate(&mut self) {
        let Some(at) = self.roster.focus else {
            return;
        };
        if at == 0 {
            self.focus_main();
        } else if let Some(row) = self.roster.workers.get(at - 1) {
            let id = row.id.clone();
            self.focus_agent(&id);
        } else {
            self.roster.focus = None;
        }
    }

    /// Apply activity lines the host read after this view's cursor.
    pub fn push_worker_lines(&mut self, agent_id: &str, lines: Vec<WorkerLine>, dropped: u64) {
        let view = self
            .roster
            .views
            .entry(agent_id.to_string())
            .or_insert_with(|| WorkerViewState {
                agent_id: agent_id.to_string(),
                ..WorkerViewState::default()
            });
        let scrolled_back = view.scroll > 0;
        view.dropped = view.dropped.max(dropped);
        for line in lines {
            if line.sequence <= view.cursor {
                continue;
            }
            view.cursor = line.sequence;
            let line = WorkerLine {
                text: plain(&line.text),
                ..line
            };
            view.lines.push_back(line);
            // A reader scrolled back keeps their place as new lines arrive.
            if scrolled_back {
                view.scroll += 1;
            }
            while view.lines.len() > WORKER_RING {
                view.lines.pop_front();
                view.dropped += 1;
                view.scroll = view.scroll.saturating_sub(usize::from(scrolled_back));
            }
        }
        self.dirty = true;
    }

    pub fn set_worker_changed_paths(&mut self, agent_id: &str, paths: Vec<String>) {
        if let Some(view) = self.roster.views.get_mut(agent_id) {
            view.changed_paths = paths.iter().map(|path| plain(path)).collect();
            self.dirty = true;
        }
    }

    /// Scroll the viewed worker's transcript; positive is back in time.
    pub fn scroll_worker(&mut self, rows: isize) {
        let Some(id) = self.viewing_worker().map(str::to_string) else {
            return;
        };
        if let Some(view) = self.roster.views.get_mut(&id) {
            let max = view.lines.len().saturating_sub(1);
            view.scroll = if rows >= 0 {
                (view.scroll + rows as usize).min(max)
            } else {
                view.scroll.saturating_sub(rows.unsigned_abs())
            };
            self.dirty = true;
        }
    }

    /// Send the composer's text to the worker in view. It goes to the outbox
    /// for the host, which journals it before answering `queued`. Returns
    /// whether anything was taken. Text that is a slash command stays with
    /// the main agent and is not taken here.
    pub fn submit_to_worker(&mut self) -> bool {
        let Some(id) = self.viewing_worker().map(str::to_string) else {
            return false;
        };
        let text = self.composer.editor().get_expanded_text();
        if text.trim().is_empty() || text.trim_start().starts_with('/') {
            return false;
        }
        self.composer.editor_mut().submit();
        self.dismiss_suggestions();
        self.roster.next_local += 1;
        let local = self.roster.next_local;
        let view = self
            .roster
            .views
            .entry(id.clone())
            .or_insert_with(|| WorkerViewState {
                agent_id: id.clone(),
                ..WorkerViewState::default()
            });
        view.receipts.push_back(Receipt {
            local,
            id: None,
            text: plain(&text).chars().take(80).collect(),
            state: "sending".into(),
            reason: None,
        });
        while view.receipts.len() > RECEIPT_RING {
            view.receipts.pop_front();
        }
        view.scroll = 0;
        self.roster.outbox.push(WorkerOutbound {
            local,
            agent_id: id,
            text,
        });
        self.dirty = true;
        true
    }

    pub fn take_worker_outbox(&mut self) -> Vec<WorkerOutbound> {
        std::mem::take(&mut self.roster.outbox)
    }

    /// The host's answer for a message: its stable id and where it stands.
    pub fn set_worker_receipt(
        &mut self,
        agent_id: &str,
        local: u64,
        id: Option<String>,
        state: &str,
        reason: Option<String>,
    ) {
        if let Some(view) = self.roster.views.get_mut(agent_id) {
            if let Some(receipt) = view.receipts.iter_mut().find(|r| r.local == local) {
                if id.is_some() {
                    receipt.id = id;
                }
                receipt.state = state.to_string();
                receipt.reason = reason.map(|reason| plain(&reason));
                self.dirty = true;
            }
        }
    }

    /// Move a receipt by its stable id (a later delivered/applied/rejected).
    pub fn advance_worker_receipt(
        &mut self,
        agent_id: &str,
        id: &str,
        state: &str,
        reason: Option<String>,
    ) {
        if let Some(view) = self.roster.views.get_mut(agent_id) {
            if let Some(receipt) = view
                .receipts
                .iter_mut()
                .find(|receipt| receipt.id.as_deref() == Some(id))
            {
                receipt.state = state.to_string();
                receipt.reason = reason.map(|reason| plain(&reason));
                self.dirty = true;
            }
        }
    }
}
