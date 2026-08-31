//! Application state, breakpoints and composer reducers.
//!
//! One instrument at a time (design.md §1): a `Screen` replaces the transcript,
//! an `Overlay` floats over it with the ramp dropped, and only Codex is a
//! persistent split — opt-in at ≥120 columns.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/model.ex`.

use super::theme::{State, Theme};

/// The transcript is the interface; a screen is what replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// `1b` — the transcript.
    Agent,
    /// `1c` — Disegno, the plan sheet.
    Plan,
    /// `2a` — Grafo, the dependency study.
    Grafo,
    /// `2b` — Memoria, vector recall.
    Memoria,
    /// `2c` — Mensura, the token governor.
    Mensura,
}

/// An instrument summoned over the transcript, dismissed with esc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// `1d` — Instrumenta, the command palette.
    Instrumenta,
    /// `1f` — Memoria, sessions.
    Sessions,
    /// `1f` — Cogitator, the model picker.
    Cogitator,
    /// The one-question instrument: a titled list, one row chosen. Trust,
    /// thinking level and stored credentials all borrow it rather than each
    /// growing a panel of its own (design.md §1 — one panel at a time).
    Ask,
}

/// One row of the `Ask` instrument: what it is, and what is worth knowing
/// about it on the same line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerItem {
    pub label: String,
    pub detail: String,
}

impl PickerItem {
    pub fn new(label: &str, detail: &str) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.to_string(),
        }
    }
}

/// A question put to the user as a list. `title` is the paired name the panel
/// wears (design.md §5), `key` the right-hand run, `note` the line above the
/// footer that says what is being decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ask {
    pub title: String,
    pub name: String,
    pub key: String,
    pub note: String,
    pub items: Vec<PickerItem>,
}

/// What enter meant while an instrument was open. The shell knows which row
/// was highlighted; only the caller knows what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// A row of Instrumenta, named as it was listed.
    Command { name: String, kind: String },
    /// A row of Memoria sessions, as an index into `sessions`.
    Session(usize),
    /// A row of Cogitator, as an index into `models`.
    Model(usize),
    /// A row of the `Ask` instrument, as an index into `ask.items`.
    Ask(usize),
}

/// A block of rows an extension owns. Extensions get rows, not colours and
/// not a layout: the theme is the shell's (design.md §2) and the panel budget
/// is the design's (§1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget {
    pub key: String,
    pub lines: Vec<String>,
    /// `belowEditor` in the extension API; anything else sits above.
    pub below: bool,
}

/// Everything the loaded extensions have asked to put on screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extensions {
    pub header: Vec<String>,
    pub footer: Vec<String>,
    pub widgets: Vec<Widget>,
    /// `key -> text`, joined into one row above the composer. It does not go
    /// in the status bar, because the meter there states a number against its
    /// cap (§9) and must not be crowded out.
    pub status: Vec<(String, String)>,
}

impl Extensions {
    /// Add, replace or (with no lines) remove a widget, keyed as the
    /// extension keyed it.
    pub fn set_widget(&mut self, key: &str, lines: Vec<String>, below: bool) {
        self.widgets.retain(|widget| widget.key != key);
        if lines.is_empty() {
            return;
        }
        self.widgets.push(Widget {
            key: key.to_string(),
            lines,
            below,
        });
    }

    pub fn set_status(&mut self, key: &str, text: Option<&str>) {
        self.status.retain(|(existing, _)| existing != key);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            self.status.push((key.to_string(), text.to_string()));
        }
    }

    /// The rows that sit above the composer: widgets first, then the status
    /// line the extensions share.
    pub fn above(&self) -> Vec<String> {
        let mut rows: Vec<String> = self
            .widgets
            .iter()
            .filter(|widget| !widget.below)
            .flat_map(|widget| widget.lines.clone())
            .collect();
        if !self.status.is_empty() {
            rows.push(
                self.status
                    .iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
        }
        rows
    }

    pub fn below(&self) -> Vec<String> {
        self.widgets
            .iter()
            .filter(|widget| widget.below)
            .flat_map(|widget| widget.lines.clone())
            .collect()
    }
}

/// One Studio step: a ledger row of ✓ / ◉ / ○ (design.md §6).
#[derive(Debug, Clone)]
pub struct Step {
    pub state: State,
    pub verb: String,
    pub target: Option<String>,
}

impl Step {
    pub fn new(state: State, verb: &str, target: Option<&str>) -> Self {
        Self {
            state,
            verb: verb.to_string(),
            target: target.map(str::to_string),
        }
    }
}

/// One row of a Δ block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Add,
    Del,
    Context,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub kind: HunkKind,
    pub text: String,
}

impl Hunk {
    pub fn new(kind: HunkKind, text: &str) -> Self {
        Self {
            kind,
            text: text.to_string(),
        }
    }
}

/// A block in the transcript. Blocks are separated by one blank row; nothing
/// inside a block is (design.md §3).
#[derive(Debug, Clone)]
pub enum Entry {
    Gap,
    User(String),
    Agent(String),
    Tool {
        state: State,
        instrument: String,
        target: String,
        duration: Option<String>,
    },
    Detail(String),
    Prose(String),
    Studio(Vec<Step>),
    Delta {
        path: String,
        adds: u32,
        dels: u32,
        hunks: Vec<Hunk>,
    },
}

impl Entry {
    pub fn user(text: &str) -> Self {
        Entry::User(text.to_string())
    }

    pub fn agent(name: &str) -> Self {
        Entry::Agent(name.to_string())
    }

    pub fn prose(text: &str) -> Self {
        Entry::Prose(text.to_string())
    }

    pub fn detail(text: &str) -> Self {
        Entry::Detail(text.to_string())
    }

    pub fn tool(state: State, instrument: &str, target: &str, duration: Option<&str>) -> Self {
        Entry::Tool {
            state,
            instrument: instrument.to_string(),
            target: target.to_string(),
            duration: duration.map(str::to_string),
        }
    }
}

/// One row of the Instrumenta corpus (`1d`): tools, sessions, files, modes.
#[derive(Debug, Clone)]
pub struct CorpusItem {
    pub name: String,
    pub description: String,
    pub kind: String,
}

impl CorpusItem {
    pub fn new(name: &str, description: &str, kind: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            kind: kind.to_string(),
        }
    }

    /// The haystack a query is matched against.
    fn haystack(&self) -> String {
        format!("{} {} {}", self.name, self.description, self.kind).to_lowercase()
    }
}

/// One row of Memoria sessions (`1f`).
#[derive(Debug, Clone)]
pub struct SessionItem {
    pub name: String,
    pub age: String,
    /// The JSONL file this row stands for, so choosing the row can open it.
    /// Empty for the mockup fixtures, which stand for no file on disk.
    pub path: String,
}

impl SessionItem {
    pub fn new(name: &str, age: &str) -> Self {
        Self {
            name: name.to_string(),
            age: age.to_string(),
            path: String::new(),
        }
    }

    pub fn at(mut self, path: &str) -> Self {
        self.path = path.to_string();
        self
    }
}

/// One row of Cogitator (`1f`): a provider and model, with its context window.
#[derive(Debug, Clone)]
pub struct ModelItem {
    pub name: String,
    pub window: String,
    /// The provider and model id behind the row, so choosing it can switch the
    /// agent. Empty for the mockup fixtures.
    pub provider: String,
    pub id: String,
    /// The context window in tokens, for the meter after a switch.
    pub window_tokens: u64,
}

impl ModelItem {
    pub fn new(name: &str, window: &str) -> Self {
        Self {
            name: name.to_string(),
            window: window.to_string(),
            provider: String::new(),
            id: String::new(),
            window_tokens: 0,
        }
    }

    pub fn of(mut self, provider: &str, id: &str, window_tokens: u64) -> Self {
        self.provider = provider.to_string();
        self.id = id.to_string();
        self.window_tokens = window_tokens;
        self
    }
}

/// One step of a Disegno plan (`1c`), numbered in Roman.
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub numeral: String,
    pub state: State,
    pub verb: String,
    pub target: Option<String>,
}

impl PlanStep {
    pub fn new(numeral: &str, state: State, verb: &str, target: Option<&str>) -> Self {
        Self {
            numeral: numeral.to_string(),
            state,
            verb: verb.to_string(),
            target: target.map(str::to_string),
        }
    }
}

/// One row of the Codex file tree (`1e`), already flattened.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub depth: u16,
    /// `▾` open, `▸` closed, nothing for a leaf.
    pub twisty: Option<String>,
    pub name: String,
    /// `Δ` when the file or directory has changes, `×` when it is failing.
    pub status: Option<State>,
}

impl TreeRow {
    pub fn new(depth: u16, twisty: Option<&str>, name: &str, status: Option<State>) -> Self {
        Self {
            depth,
            twisty: twisty.map(str::to_string),
            name: name.to_string(),
            status,
        }
    }
}

/// One row of the git changes popover (`1e`).
#[derive(Debug, Clone)]
pub struct ChangeRow {
    /// Porcelain status letter: `M`, `A`, `D`, `?`.
    pub status: String,
    pub path: String,
    pub count: String,
}

impl ChangeRow {
    pub fn new(status: &str, path: &str, count: &str) -> Self {
        Self {
            status: status.to_string(),
            path: path.to_string(),
            count: count.to_string(),
        }
    }
}

/// Which role a run of the graph drawing plays (`2a`). Connectors are drawn in
/// border, names in muted, and the node in hand in copper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphInk {
    Connector,
    Name,
    Current,
}

/// One row of the graph drawing, on a strict column grid.
#[derive(Debug, Clone)]
pub struct GraphRow(pub Vec<(String, GraphInk)>);

/// One row of the impact list under the graph (`2a`).
#[derive(Debug, Clone)]
pub struct ImpactRow {
    pub state: State,
    pub symbol: String,
    pub distance: String,
    pub sites: String,
    /// Untested edges are drawn in warning (design.md §6).
    pub untested: bool,
}

impl ImpactRow {
    pub fn new(state: State, symbol: &str, distance: &str, sites: &str, untested: bool) -> Self {
        Self {
            state,
            symbol: symbol.to_string(),
            distance: distance.to_string(),
            sites: sites.to_string(),
            untested,
        }
    }
}

/// What the graph knows about itself, for the panel's notches (`2a`).
#[derive(Debug, Clone, Default)]
pub struct GraphMeta {
    pub nodes: String,
    pub edges: String,
    pub cycles: String,
    pub subject: String,
    pub fan_in: String,
    pub fan_out: String,
    pub depth: String,
    pub tests: String,
    pub untested: String,
    pub freshness: String,
}

/// One hit from vector recall (`2b`). Two rows each: score, summary and
/// location, then a proportion meter and provenance.
#[derive(Debug, Clone)]
pub struct RecallHit {
    pub score: f64,
    pub summary: String,
    pub location: String,
    pub provenance: String,
    /// Hits below the relevance floor are counted, not drawn, so the retrieval
    /// stays auditable.
    pub above_floor: bool,
}

impl RecallHit {
    pub fn new(
        score: f64,
        summary: &str,
        location: &str,
        provenance: &str,
        above_floor: bool,
    ) -> Self {
        Self {
            score,
            summary: summary.to_string(),
            location: location.to_string(),
            provenance: provenance.to_string(),
            above_floor,
        }
    }
}

/// What the recall index knows about itself (`2b`).
#[derive(Debug, Clone, Default)]
pub struct RecallMeta {
    pub query: String,
    pub vectors: String,
    pub shards: String,
    pub embedding: String,
    pub metric: String,
    pub elapsed: String,
    pub k: String,
    pub floor: f64,
    pub promoted: String,
    pub freshness: String,
}

/// One role's share of the context window (`2c`).
#[derive(Debug, Clone)]
pub struct BudgetRow {
    pub role: String,
    pub tokens: String,
    pub fraction: f64,
    pub note: String,
    /// The breaching row is copper with a warning cap note; the rest verdigris.
    pub breach: bool,
}

impl BudgetRow {
    pub fn new(role: &str, tokens: &str, fraction: f64, note: &str, breach: bool) -> Self {
        Self {
            role: role.to_string(),
            tokens: tokens.to_string(),
            fraction,
            note: note.to_string(),
            breach,
        }
    }
}

/// A governor proposal (`2c`). It always states what it recovers, what it
/// keeps, what it costs and whether it is reversible, and it never acts
/// silently (design.md §6).
#[derive(Debug, Clone, Default)]
pub struct Proposal {
    pub summary: String,
    pub recovers: String,
    pub keeps: String,
    pub cost: String,
    pub reversible: bool,
    /// `(key, what it does)`, in the order they are offered.
    pub actions: Vec<(String, String)>,
}

/// What the governor knows about the session's spend (`2c`).
#[derive(Debug, Clone, Default)]
pub struct BudgetMeta {
    pub policy: String,
    pub in_use: String,
    pub window: String,
    pub headroom: String,
    /// How much of the window is in use, as a proportion.
    pub in_use_fraction: f64,
    pub rate: String,
    pub session_spend: String,
    pub daily_cap: String,
    pub daily_fraction: f64,
    pub history: String,
}

/// What the session found when it opened — the empty state, screen `1a`.
#[derive(Debug, Clone, Default)]
pub struct Startup {
    pub cwd: String,
    pub branch: String,
    pub language: String,
    pub crates: String,
    pub restored: bool,
}

/// Where the session lives, and what it costs. Every field here is shown as a
/// meter or a labelled unit, never as a bare number (design.md §9).
#[derive(Debug, Clone)]
pub struct Model {
    pub width: u16,
    pub height: u16,
    /// One clock, 250ms per step, driving both animations (design.md §8).
    pub tick: u64,
    pub animate: bool,
    pub theme: Theme,

    pub screen: Screen,
    pub overlay: Option<Overlay>,
    pub codex: bool,

    pub composer: String,
    /// Turns typed while one was already running, waiting their place. They
    /// sit in the composer box above the line being typed, so what is waiting
    /// is visible rather than remembered.
    pub queued: Vec<String>,
    pub query: String,
    pub transcript: Vec<Entry>,
    /// Rows the loaded extensions have asked for.
    pub extensions: Extensions,
    pub running: bool,

    pub palette_index: usize,
    pub session_index: usize,
    pub model_index: usize,
    pub recall_index: usize,

    /// The question in hand, when `Overlay::Ask` is open.
    pub ask: Ask,
    pub ask_index: usize,

    pub cwd: String,
    pub branch: String,
    pub model_name: String,
    /// `Δn +a -d` for the status bar.
    pub changes: (u32, u32, u32),
    /// `(used, cap)` in tokens.
    pub context: (u64, u64),
    /// The empty state, shown while the transcript has nothing in it.
    pub startup: Startup,

    /// Everything Instrumenta can reach, and how much of it there is.
    pub corpus: Vec<CorpusItem>,
    pub corpus_total: usize,
    /// Workspace-relative paths, for completing a file name in the composer.
    /// Supplied by the shell; the view layer never walks a disk.
    pub paths: Vec<String>,
    pub sessions: Vec<SessionItem>,
    pub models: Vec<ModelItem>,
    /// Where the model picker says the configuration lives (`1f`).
    pub config_path: String,

    /// `1c` — the plan in hand.
    pub plan: Vec<PlanStep>,
    /// `1e` — the workspace tree and the git changes beside it.
    pub tree: Vec<TreeRow>,
    pub changes_list: Vec<ChangeRow>,

    /// `2a` — the dependency study.
    pub graph: Vec<GraphRow>,
    pub graph_meta: GraphMeta,
    pub impact: Vec<ImpactRow>,
    /// `2b` — vector recall.
    pub recall: Vec<RecallHit>,
    pub recall_meta: RecallMeta,
    /// `2c` — the token governor.
    pub budget: Vec<BudgetRow>,
    pub budget_meta: BudgetMeta,
    pub proposal: Option<Proposal>,
}

impl Model {
    pub fn new(theme: Theme, width: u16, height: u16, animate: bool) -> Self {
        Self {
            width,
            height,
            tick: 0,
            animate,
            theme,
            screen: Screen::Agent,
            overlay: None,
            codex: false,
            composer: String::new(),
            queued: Vec::new(),
            extensions: Extensions::default(),
            query: String::new(),
            transcript: Vec::new(),
            running: false,
            palette_index: 0,
            session_index: 0,
            model_index: 0,
            recall_index: 0,
            ask: Ask::default(),
            ask_index: 0,
            cwd: String::new(),
            branch: String::new(),
            model_name: String::new(),
            changes: (0, 0, 0),
            context: (0, 200_000),
            startup: Startup::default(),
            corpus: Vec::new(),
            corpus_total: 0,
            paths: Vec::new(),
            sessions: Vec::new(),
            models: Vec::new(),
            config_path: String::new(),
            plan: Vec::new(),
            tree: Vec::new(),
            changes_list: Vec::new(),
            graph: Vec::new(),
            graph_meta: GraphMeta::default(),
            impact: Vec::new(),
            recall: Vec::new(),
            recall_meta: RecallMeta::default(),
            budget: Vec::new(),
            budget_meta: BudgetMeta::default(),
            proposal: None,
        }
    }

    /// The corpus, narrowed by the palette query. Matching is subsequence
    /// matching over name, description and kind, so `git` finds `/git status`
    /// and `fix-git-hooks` alike.
    pub fn filtered_corpus(&self) -> Vec<&CorpusItem> {
        if self.query.is_empty() {
            return self.corpus.iter().collect();
        }
        let needle = self.query.to_lowercase();
        self.corpus
            .iter()
            .filter(|item| subsequence(&needle, &item.haystack()))
            .collect()
    }

    /// The selected row of whichever list is open, clamped to what is there.
    pub fn selection(&self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        let index = match self.overlay {
            Some(Overlay::Instrumenta) => self.palette_index,
            Some(Overlay::Sessions) => self.session_index,
            Some(Overlay::Cogitator) => self.model_index,
            Some(Overlay::Ask) => self.ask_index,
            None => self.recall_index,
        };
        Some(index % len)
    }

    /// What the highlighted row of the open instrument stands for. `None` when
    /// no instrument is open, or when the one that is has nothing in it — an
    /// empty list must not answer enter with a choice it cannot make.
    pub fn accept(&self) -> Option<Choice> {
        match self.overlay {
            Some(Overlay::Instrumenta) => {
                let rows = self.filtered_corpus();
                let index = self.selection(rows.len())?;
                let row = rows[index];
                Some(Choice::Command {
                    name: row.name.clone(),
                    kind: row.kind.clone(),
                })
            }
            Some(Overlay::Sessions) => self.selection(self.sessions.len()).map(Choice::Session),
            Some(Overlay::Cogitator) => self.selection(self.models.len()).map(Choice::Model),
            Some(Overlay::Ask) => self.selection(self.ask.items.len()).map(Choice::Ask),
            None => None,
        }
    }

    /// Move the selection in whichever list is open.
    pub fn move_selection(&mut self, delta: isize) {
        match self.overlay {
            Some(Overlay::Instrumenta) => {
                let len = self.filtered_corpus().len();
                self.palette_index = wrap_index(self.palette_index, delta, len);
            }
            Some(Overlay::Sessions) => {
                self.session_index = wrap_index(self.session_index, delta, self.sessions.len());
            }
            Some(Overlay::Cogitator) => {
                self.model_index = wrap_index(self.model_index, delta, self.models.len());
            }
            Some(Overlay::Ask) => {
                self.ask_index = wrap_index(self.ask_index, delta, self.ask.items.len());
            }
            None => {}
        }
    }

    // --- breakpoints (design.md §7) ------------------------------------------

    /// Below 100: Studio collapses and annotations are dropped (screen `1g`).
    pub fn narrow(&self) -> bool {
        self.width < 100
    }

    /// At 80 and below: the header drops the model, paths shorten to
    /// crate-relative, and the status bar abbreviates to `^p` (screen `1g`).
    pub fn minimal(&self) -> bool {
        self.width <= 80
    }

    /// Below 80: transcript and composer only, every panel full-screen.
    /// Nothing requires a large window (design.md §7).
    pub fn bare(&self) -> bool {
        self.width < 80
    }

    /// The Codex sidebar is opt-in, and only offered at ≥120 columns.
    pub fn sidebar_allowed(&self) -> bool {
        self.width >= 120
    }

    /// At ≥150 the git changes popover fits under the transcript.
    pub fn wide(&self) -> bool {
        self.width >= 150
    }

    /// Decoration — the identity mark, the compass, annotations.
    pub fn decoration(&self) -> bool {
        self.width >= 100
    }

    pub fn codex_open(&self) -> bool {
        self.codex && self.sidebar_allowed()
    }

    /// How far an overlay is inset from the window edge (design.md §7).
    pub fn overlay_inset(&self) -> u16 {
        if self.bare() {
            0
        } else if self.width >= 120 {
            8
        } else {
            6
        }
    }

    /// The word under the identity mark in the header.
    pub fn mode(&self) -> &'static str {
        match self.screen {
            Screen::Agent => "agent",
            Screen::Plan => "plan",
            Screen::Grafo => "grafo",
            Screen::Memoria => "memoria",
            Screen::Mensura => "mensura",
        }
    }

    /// The caret blinks at ~1s, step-end, off the same clock as the spinner.
    pub fn blink(&self) -> bool {
        if !self.animate {
            return true;
        }
        (self.tick / 4) % 2 == 0
    }

    pub fn context_fraction(&self) -> f64 {
        let (used, cap) = self.context;
        if cap == 0 {
            return 0.0;
        }
        (used as f64 / cap as f64).clamp(0.0, 1.0)
    }

    // --- reducers ------------------------------------------------------------

    pub fn type_char(&mut self, text: &str) {
        if self.overlay == Some(Overlay::Instrumenta) {
            self.query.push_str(text);
            self.palette_index = 0;
        } else {
            self.composer.push_str(text);
        }
    }

    /// A newline inside the composer. The hint line promises `shift+enter
    /// newline`; this is what keeps that promise.
    pub fn newline(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        self.composer.push('\n');
    }

    /// Complete the token under the caret against the commands and the
    /// workspace. Returns whether anything changed, so the caller can leave
    /// tab alone when there is nothing to complete.
    ///
    /// Completion goes as far as every candidate agrees and no further: a
    /// prefix shared by four files is not a choice the user has made.
    pub fn complete(&mut self) -> bool {
        if self.overlay.is_some() {
            return false;
        }
        let start = self
            .composer
            .rfind(|ch: char| ch.is_whitespace())
            .map(|index| index + 1)
            .unwrap_or(0);
        let token = &self.composer[start..];
        if token.is_empty() {
            return false;
        }

        let candidates: Vec<&str> = if token.starts_with('/') {
            self.corpus
                .iter()
                .filter(|item| item.kind == "command" && item.name.starts_with(token))
                .map(|item| item.name.as_str())
                .collect()
        } else {
            self.paths
                .iter()
                .filter(|path| path.starts_with(token))
                .map(String::as_str)
                .collect()
        };
        let Some(common) = common_prefix(&candidates) else {
            return false;
        };
        if common.len() <= token.len() {
            return false;
        }
        let only_one = candidates.len() == 1;
        self.composer.truncate(start);
        self.composer.push_str(&common);
        if only_one {
            self.composer.push(' ');
        }
        true
    }

    pub fn backspace(&mut self) {
        let target = if self.overlay == Some(Overlay::Instrumenta) {
            &mut self.query
        } else {
            &mut self.composer
        };
        target.pop();
        if self.overlay == Some(Overlay::Instrumenta) {
            self.palette_index = 0;
        }
    }

    /// Enter sends. An empty composer sends nothing.
    pub fn submit(&mut self) {
        if self.composer.trim().is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.composer);
        if !self.transcript.is_empty() {
            self.transcript.push(Entry::Gap);
        }
        self.transcript.push(Entry::user(&text));
        self.transcript.push(Entry::Gap);
        self.transcript.push(Entry::agent("davinci"));
        self.running = true;
    }

    /// Set the composer aside to be sent when the running turn is over.
    /// Returns whether anything was queued.
    pub fn queue(&mut self) -> bool {
        if self.composer.trim().is_empty() {
            return false;
        }
        self.queued.push(std::mem::take(&mut self.composer));
        true
    }

    /// ctrl+c interrupts the run, never the app (design.md §6). What was
    /// waiting to be sent goes with it: the user stopped this train of
    /// thought, not just this turn.
    pub fn interrupt(&mut self) {
        self.running = false;
        self.queued.clear();
    }

    /// esc closes the instrument in hand and returns to the transcript.
    pub fn close(&mut self) {
        self.screen = Screen::Agent;
        self.overlay = None;
    }

    pub fn toggle_overlay(&mut self, overlay: Overlay) {
        if self.overlay == Some(overlay) {
            self.overlay = None;
        } else {
            self.overlay = Some(overlay);
            self.screen = Screen::Agent;
        }
    }

    pub fn toggle_screen(&mut self, screen: Screen) {
        if self.screen == screen {
            self.screen = Screen::Agent;
        } else {
            self.screen = screen;
            self.overlay = None;
        }
    }

    pub fn toggle_codex(&mut self) {
        self.codex = !self.codex;
        self.overlay = None;
        self.screen = Screen::Agent;
    }
}

/// Whether `needle` appears in `haystack` in order, not necessarily adjacent.
fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|wanted| chars.any(|candidate| candidate == wanted))
}

/// Move a selection index by `delta`, wrapping at both ends.
/// The longest prefix every candidate shares, or `None` when there are none.
/// Byte-wise, but only ever cut at a character boundary.
pub fn common_prefix(candidates: &[&str]) -> Option<String> {
    let first = candidates.first()?;
    let mut end = first.len();
    for candidate in &candidates[1..] {
        let shared = first
            .bytes()
            .zip(candidate.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        end = end.min(shared);
    }
    while end > 0 && !first.is_char_boundary(end) {
        end -= 1;
    }
    Some(first[..end].to_string())
}

pub fn wrap_index(index: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    let next = (index as isize + delta).rem_euclid(len);
    next as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::ColorDepth;

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        )
    }

    #[test]
    fn breakpoints_follow_the_responsive_table() {
        // Screen `1g` is authored at exactly 80: abbreviated, but still a
        // full transcript with panels available.
        let narrow = model(80);
        assert!(narrow.narrow());
        assert!(narrow.minimal());
        assert!(!narrow.bare());
        assert!(!narrow.decoration());
        assert!(!narrow.sidebar_allowed());

        let tight = model(72);
        assert!(tight.bare());
        assert!(tight.minimal());
        assert!(tight.narrow());

        let between = model(90);
        assert!(between.narrow());
        assert!(!between.minimal(), "81..99 keeps the fuller status bar");

        let standard = model(100);
        assert!(!standard.narrow());
        assert!(standard.decoration());
        assert!(!standard.sidebar_allowed());

        let sidebar = model(120);
        assert!(sidebar.sidebar_allowed());
        assert!(!sidebar.wide());

        let wide = model(160);
        assert!(wide.sidebar_allowed());
        assert!(wide.wide());
    }

    #[test]
    fn the_codex_sidebar_is_opt_in_and_only_offered_when_wide_enough() {
        let mut narrow = model(100);
        narrow.toggle_codex();
        assert!(narrow.codex, "the preference is remembered");
        assert!(!narrow.codex_open(), "but the split never opens below 120");

        let mut wide = model(160);
        assert!(!wide.codex_open(), "and it is off until asked for");
        wide.toggle_codex();
        assert!(wide.codex_open());
    }

    #[test]
    fn overlays_inset_further_as_the_window_grows() {
        assert_eq!(model(72).overlay_inset(), 0);
        assert_eq!(model(100).overlay_inset(), 6);
        assert_eq!(model(120).overlay_inset(), 8);
        assert_eq!(model(160).overlay_inset(), 8);
    }

    #[test]
    fn one_panel_at_a_time() {
        let mut m = model(160);
        m.toggle_screen(Screen::Grafo);
        assert_eq!(m.screen, Screen::Grafo);

        m.toggle_overlay(Overlay::Instrumenta);
        assert_eq!(m.overlay, Some(Overlay::Instrumenta));
        assert_eq!(m.screen, Screen::Agent, "an overlay closes the screen");

        m.toggle_screen(Screen::Mensura);
        assert_eq!(m.overlay, None, "a screen closes the overlay");

        m.toggle_codex();
        assert_eq!(m.screen, Screen::Agent);
        assert_eq!(m.overlay, None);
    }

    #[test]
    fn esc_closes_whatever_is_in_hand() {
        let mut m = model(120);
        m.toggle_overlay(Overlay::Sessions);
        m.close();
        assert_eq!(m.overlay, None);
        assert_eq!(m.screen, Screen::Agent);

        m.toggle_screen(Screen::Plan);
        m.close();
        assert_eq!(m.screen, Screen::Agent);
    }

    #[test]
    fn toggling_the_same_instrument_dismisses_it() {
        let mut m = model(120);
        m.toggle_overlay(Overlay::Cogitator);
        m.toggle_overlay(Overlay::Cogitator);
        assert_eq!(m.overlay, None);

        m.toggle_screen(Screen::Memoria);
        m.toggle_screen(Screen::Memoria);
        assert_eq!(m.screen, Screen::Agent);
    }

    #[test]
    fn typing_goes_to_the_palette_when_the_palette_is_open() {
        let mut m = model(120);
        m.type_char("gi");
        assert_eq!(m.composer, "gi");

        m.toggle_overlay(Overlay::Instrumenta);
        m.type_char("t");
        assert_eq!(m.query, "t");
        assert_eq!(m.composer, "gi", "the composer is not disturbed");

        m.backspace();
        assert_eq!(m.query, "");
        assert_eq!(m.composer, "gi");
    }

    #[test]
    fn submit_appends_a_turn_and_clears_the_composer() {
        let mut m = model(120);
        m.type_char("explain how the agent runtime works");
        m.submit();
        assert_eq!(m.composer, "");
        assert!(m.running);
        assert!(matches!(m.transcript[0], Entry::User(_)));
        assert!(matches!(m.transcript.last(), Some(Entry::Agent(_))));
    }

    #[test]
    fn an_empty_composer_sends_nothing() {
        let mut m = model(120);
        m.submit();
        m.type_char("   ");
        m.submit();
        assert!(m.transcript.is_empty());
        assert!(!m.running);
    }

    #[test]
    fn interrupt_stops_the_run_and_leaves_the_app_alone() {
        let mut m = model(120);
        m.type_char("run the tests");
        m.submit();
        m.interrupt();
        assert!(!m.running);
        assert!(!m.transcript.is_empty(), "the transcript survives");
    }

    #[test]
    fn the_caret_blinks_off_the_shared_clock_and_freezes_when_still() {
        let mut m = model(120);
        let phases: Vec<bool> = (0..9)
            .map(|tick| {
                m.tick = tick;
                m.blink()
            })
            .collect();
        assert_eq!(
            phases,
            vec![true, true, true, true, false, false, false, false, true]
        );

        m.animate = false;
        m.tick = 6;
        assert!(m.blink(), "a still caret is drawn, not hidden");
    }

    #[test]
    fn the_mode_word_follows_the_screen() {
        let mut m = model(120);
        assert_eq!(m.mode(), "agent");
        m.toggle_screen(Screen::Plan);
        assert_eq!(m.mode(), "plan");
        m.toggle_screen(Screen::Mensura);
        assert_eq!(m.mode(), "mensura");
    }

    #[test]
    fn the_context_fraction_is_bounded() {
        let mut m = model(120);
        m.context = (47_000, 200_000);
        assert!((m.context_fraction() - 0.235).abs() < 1e-9);
        m.context = (250_000, 200_000);
        assert_eq!(m.context_fraction(), 1.0);
        m.context = (10, 0);
        assert_eq!(m.context_fraction(), 0.0);
    }

    #[test]
    fn enter_answers_with_the_highlighted_row_of_whatever_is_open() {
        let mut m = model(120);
        m.corpus = vec![
            CorpusItem::new("/compact", "summarise the session", "command"),
            CorpusItem::new("read", "instrumenta", "tool"),
        ];
        m.sessions = vec![
            SessionItem::new("first", "3m").at("a.jsonl"),
            SessionItem::new("second", "1h").at("b.jsonl"),
        ];
        m.models = vec![
            ModelItem::new("anthropic / opus", "200k").of("anthropic", "opus", 200_000),
            ModelItem::new("openai / gpt", "128k").of("openai", "gpt", 128_000),
        ];
        m.ask.items = vec![PickerItem::new("trust", ""), PickerItem::new("ask", "")];

        m.toggle_overlay(Overlay::Instrumenta);
        assert_eq!(
            m.accept(),
            Some(Choice::Command {
                name: "/compact".into(),
                kind: "command".into()
            })
        );
        m.move_selection(1);
        assert_eq!(
            m.accept(),
            Some(Choice::Command {
                name: "read".into(),
                kind: "tool".into()
            })
        );

        m.toggle_overlay(Overlay::Sessions);
        m.move_selection(1);
        assert_eq!(m.accept(), Some(Choice::Session(1)));

        m.toggle_overlay(Overlay::Cogitator);
        assert_eq!(m.accept(), Some(Choice::Model(0)));

        m.toggle_overlay(Overlay::Ask);
        m.move_selection(-1);
        assert_eq!(m.accept(), Some(Choice::Ask(1)));
    }

    #[test]
    fn an_empty_list_answers_with_nothing_rather_than_a_row_that_is_not_there() {
        let mut m = model(120);
        m.toggle_overlay(Overlay::Sessions);
        assert_eq!(m.accept(), None);
        m.toggle_overlay(Overlay::Ask);
        assert_eq!(m.accept(), None);
        // With no instrument open, enter is the composer's, not a choice.
        m.close();
        assert_eq!(m.accept(), None);
    }

    #[test]
    fn a_query_that_narrows_the_palette_still_answers_with_the_row_on_screen() {
        let mut m = model(120);
        m.corpus = vec![
            CorpusItem::new("/compact", "summarise the session", "command"),
            CorpusItem::new("/export", "write the session out", "command"),
        ];
        m.toggle_overlay(Overlay::Instrumenta);
        m.type_char("exp");
        assert_eq!(m.filtered_corpus().len(), 1);
        assert_eq!(
            m.accept(),
            Some(Choice::Command {
                name: "/export".into(),
                kind: "command".into()
            })
        );
    }

    #[test]
    fn tab_completes_a_command_as_far_as_every_candidate_agrees() {
        let mut m = model(120);
        m.corpus = vec![
            CorpusItem::new("/compact", "", "command"),
            CorpusItem::new("/copy", "", "command"),
            CorpusItem::new("/clone", "", "command"),
            CorpusItem::new("read", "", "tool"),
        ];

        m.type_char("/c");
        // `/compact`, `/copy` and `/clone` share `/c` and nothing more, so
        // tab reports that it changed nothing rather than choosing.
        assert!(!m.complete());
        assert_eq!(m.composer, "/c");

        m.composer = "/co".into();
        assert!(!m.complete(), "/compact and /copy still disagree");
        assert_eq!(m.composer, "/co");

        m.composer = "/com".into();
        assert!(m.complete());
        assert_eq!(m.composer, "/compact ", "one candidate completes in full");
    }

    #[test]
    fn tab_completes_a_path_from_the_workspace_and_leaves_prose_alone() {
        let mut m = model(120);
        m.paths = vec![
            "crates/".into(),
            "crates/pi-tui/".into(),
            "crates/pi-tui/src/lib.rs".into(),
        ];

        m.type_char("read crates/pi-tui/s");
        assert!(m.complete());
        assert_eq!(m.composer, "read crates/pi-tui/src/lib.rs ");

        m.composer = "explain the runtime".into();
        assert!(!m.complete(), "prose has nothing to complete against");

        m.composer = String::new();
        assert!(!m.complete(), "an empty composer completes to nothing");
    }

    #[test]
    fn tab_is_the_palettes_business_only_when_no_instrument_is_open() {
        let mut m = model(120);
        m.corpus = vec![CorpusItem::new("/compact", "", "command")];
        m.toggle_overlay(Overlay::Instrumenta);
        m.composer = "/comp".into();
        assert!(!m.complete());
        assert_eq!(m.composer, "/comp");
    }

    #[test]
    fn a_newline_grows_the_composer_without_sending_it() {
        let mut m = model(120);
        m.type_char("first");
        m.newline();
        m.type_char("second");
        assert_eq!(m.composer, "first\nsecond");
        assert!(!m.running, "a newline is not a send");
        m.submit();
        assert_eq!(m.composer, "");
        assert!(m.running);
    }

    #[test]
    fn a_follow_up_typed_mid_turn_waits_its_place_and_an_interrupt_drops_it() {
        let mut m = model(120);
        m.type_char("run the tests");
        m.submit();
        assert!(m.running);

        m.type_char("then commit");
        assert!(m.queue());
        assert_eq!(m.queued, vec!["then commit".to_string()]);
        assert_eq!(m.composer, "", "queuing clears the line being typed");
        assert!(!m.queue(), "an empty composer queues nothing");

        // ctrl+c stops the train of thought, not only the turn in flight.
        m.interrupt();
        assert!(!m.running);
        assert!(m.queued.is_empty());
    }

    #[test]
    fn the_common_prefix_is_the_longest_all_candidates_share() {
        assert_eq!(common_prefix(&[]), None);
        assert_eq!(common_prefix(&["only"]).as_deref(), Some("only"));
        assert_eq!(
            common_prefix(&["/compact", "/copy", "/clone"]).as_deref(),
            Some("/c")
        );
        // Never cut a multi-byte character in half.
        assert_eq!(common_prefix(&["éa", "éb"]).as_deref(), Some("é"));
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        assert_eq!(wrap_index(0, -1, 5), 4);
        assert_eq!(wrap_index(4, 1, 5), 0);
        assert_eq!(wrap_index(2, 1, 5), 3);
        assert_eq!(wrap_index(0, 1, 0), 0);
    }
}
