//! Application state, breakpoints and composer reducers.
//!
//! One instrument at a time (design.md §1): a `Screen` replaces the transcript,
//! an `Overlay` floats over it with the ramp dropped, and only Codex is a
//! persistent split — opt-in at ≥120 columns.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/model.ex`.

use std::fmt;
use std::ops::Deref;
use std::path::Path;

use crate::autocomplete::{
    apply_completion, suggestions, AutocompleteSuggestions, ExtraAutocompleteProvider,
    SlashCommandSpec, SuggestionQuery,
};
use crate::{Editor, Keybindings};

use super::theme::{State, Theme};

/// How many completions the composer offers at once. The list sits between the
/// transcript and the composer, so it stays short enough to leave the turn it
/// belongs to visible (design.md §2).
pub const SUGGESTION_ROWS: usize = 6;

/// Da Vinci's composer text backed by the mature shared [`Editor`].
///
/// `Deref<str>` and the string conversions keep the renderer/fixture surface
/// lightweight while all mutations go through editor semantics rather than an
/// append-only `String`.
#[derive(Debug, Clone)]
pub struct Composer {
    editor: Editor,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            editor: Editor::new(),
        }
    }
}

impl Composer {
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.editor.set_text(text);
    }

    pub fn push_str(&mut self, text: &str) {
        self.editor.insert_str(text);
    }

    pub fn push(&mut self, ch: char) {
        self.editor.insert(ch);
    }

    pub fn clear(&mut self) {
        self.editor.set_text(String::new());
    }

    pub fn truncate(&mut self, len: usize) {
        let mut end = len.min(self.editor.buffer.len());
        while end > 0 && !self.editor.buffer.is_char_boundary(end) {
            end -= 1;
        }
        let text = self.editor.buffer[..end].to_string();
        self.editor.set_text(text);
    }

    pub fn into_string(self) -> String {
        self.editor.buffer
    }
}

impl Deref for Composer {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.editor.get_text()
    }
}

impl fmt::Display for Composer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.editor.get_text())
    }
}

impl From<String> for Composer {
    fn from(value: String) -> Self {
        let mut composer = Self::default();
        composer.set_text(value);
        composer
    }
}

impl From<&str> for Composer {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}

impl PartialEq<&str> for Composer {
    fn eq(&self, other: &&str) -> bool {
        self.editor.get_text() == *other
    }
}

impl PartialEq<String> for Composer {
    fn eq(&self, other: &String) -> bool {
        self.editor.get_text() == other
    }
}

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
    /// `3a` — the full model catalog (`/model`).
    Models,
    /// `3b` — settings (`/settings`).
    Settings,
    /// `3c` — thinking levels (`/thinking`).
    Thinking,
    /// `3d` — provider credentials (`/login`).
    Login,
    /// `3e` — the keymap (`/hotkeys`).
    Keys,
    /// `4a` — the session list (`/resume`).
    Resume,
    /// `4b` — the session tree (`/tree`).
    Tree,
    /// `4c` — the compaction preview (`/compact`).
    Compact,
    /// `4d` — the export ledger (`/export`).
    Export,
    /// `5a` — a task running as a graph (`/graph`).
    GraphRun,
    /// `5b` — the vector index (`/memory-status`).
    Vectors,
    /// `5c` — the token governor's ledger (`/governor-status`).
    Governor,
    /// `5d` — the security scan (`/sec-report`).
    Securitas,
    /// `6a` — project trust (`/trust`).
    Trust,
    /// `6b` — the workshop, what `/reload` loaded.
    Officina,
    /// `6c` — the interrupt aftermath.
    Recovery,
    /// `6d` — the Δ review (`/diff`).
    Diff,
    /// Connected MCP servers (`/mcp`). No mockup number; phase 4.
    Mcp,
    /// `/permissions` — mode and rules.
    Permissions,
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
    /// A row of the model catalog screen (`3a`), as an index into `catalog`.
    Catalog(usize),
    /// A setting to advance to its next value (`3b`).
    Setting(usize),
    /// A thinking level (`3c`), as an index into `thinking_rows`.
    ThinkingLevel(usize),
    /// A provider to sign in to (`3d`), as an index into `providers`.
    Provider(usize),
    /// A session of the resume screen (`4a`).
    ResumeSession(usize),
    /// A node of the session tree (`4b`), as an index into `session_tree`.
    TreeEntry(usize),
    /// Enter on the trust sheet (`6a`): read first, then decide.
    TrustDecide,
    /// A row of the `/permissions` sheet.
    Permission(usize),
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

/// What a turn under way has cost so far, for the working line pinned above
/// the composer (design.md §8).
///
/// The shell owns the clock and the counter; the view only reads them, so a
/// frame is a pure function of the model and stays testable.
#[derive(Debug, Clone, Default)]
pub struct Working {
    /// Seconds since the turn was sent.
    pub seconds: u64,
    /// Tokens streamed back so far — output, not context.
    pub tokens: u64,
    /// `high`, `medium`, … — `None` when the model is not thinking.
    pub thinking: Option<String>,
}

impl Working {
    pub fn new() -> Self {
        Self::default()
    }

    /// The word beside the spinner. It is drawn from the elapsed second, not
    /// from a random seed, so a frame is reproducible in a test.
    pub fn verb(&self) -> &'static str {
        // A workshop's vocabulary, one word every three seconds.
        const VERBS: [&str; 20] = [
            "Pondering",
            "Sketching",
            "Drafting",
            "Composing",
            "Measuring",
            "Devising",
            "Chiselling",
            "Layering",
            "Gilding",
            "Mixing",
            "Sculpting",
            "Etching",
            "Burnishing",
            "Contemplating",
            "Refining",
            "Rendering",
            "Studying",
            "Tinkering",
            "Weaving",
            "Distilling",
        ];
        VERBS[((self.seconds / 3) as usize) % VERBS.len()]
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
        /// What the call came back with, in the fewest words that still say it
        /// — `412 lines`, `8 matches`, `+31 -8`. It sits at the end of the tool
        /// line so a finished call states its outcome without a second row.
        summary: Option<String>,
        /// The result itself, line by line, kept on the entry and drawn only
        /// when asked: a failure shows its first four rows, `ctrl+t` shows
        /// twelve of any call (phase 3, "Collapsible tool output"). Clipped
        /// to `TOOL_OUTPUT_KEPT` rows on entry so a long read costs nothing.
        output: Vec<String>,
    },
    Detail(String),
    /// A failure detail: what went wrong, then the subject — `1 failed
    /// store::roundtrip_windows_paths` (`1g`).
    Failure {
        what: String,
        subject: String,
    },
    Prose(String),
    /// The model's reasoning. While `live` the last few rows are shown as
    /// they arrive; once done it collapses to one row — `⟐ reasoned 4s ·
    /// first sentence` — so the thinking is auditable without crowding the
    /// answer.
    Thinking {
        text: String,
        live: bool,
        seconds: u64,
    },
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

    pub fn thinking(text: &str, live: bool, seconds: u64) -> Self {
        Entry::Thinking {
            text: text.to_string(),
            live,
            seconds,
        }
    }

    pub fn detail(text: &str) -> Self {
        Entry::Detail(text.to_string())
    }

    pub fn failure(what: &str, subject: &str) -> Self {
        Entry::Failure {
            what: what.to_string(),
            subject: subject.to_string(),
        }
    }

    pub fn tool(state: State, instrument: &str, target: &str, duration: Option<&str>) -> Self {
        Entry::Tool {
            state,
            instrument: instrument.to_string(),
            target: target.to_string(),
            duration: duration.map(str::to_string),
            summary: None,
            output: Vec::new(),
        }
    }

    /// The same tool line, with what it came back with.
    pub fn summarised(mut self, text: &str) -> Self {
        if let Entry::Tool { summary, .. } = &mut self {
            *summary = Some(text.to_string());
        }
        self
    }

    /// The same tool line, carrying its result for the expanded view.
    pub fn with_output(mut self, text: &str) -> Self {
        if let Entry::Tool { output, .. } = &mut self {
            *output = tool_output_rows(text);
        }
        self
    }
}

/// Rows of tool output kept on an entry; a 2000-line read is clipped here
/// and counted, not carried.
pub const TOOL_OUTPUT_KEPT: usize = 200;

/// A result's non-empty lines, trimmed at the right, clipped to
/// `TOOL_OUTPUT_KEPT` with a last row that counts the rest.
pub fn tool_output_rows(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let mut rows: Vec<String> = lines
        .iter()
        .take(TOOL_OUTPUT_KEPT)
        .map(|line| line.to_string())
        .collect();
    if lines.len() > TOOL_OUTPUT_KEPT {
        rows.push(format!("… {} more lines", lines.len() - TOOL_OUTPUT_KEPT));
    }
    rows
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
    /// What the panel footer says about the highlighted row: `42 turns │
    /// 128k tokens │ forked from provider-parity` (`1f`). Empty runs are
    /// dropped from the footer.
    pub turns: String,
    pub tokens: String,
    pub lineage: String,
}

impl SessionItem {
    pub fn new(name: &str, age: &str) -> Self {
        Self {
            name: name.to_string(),
            age: age.to_string(),
            path: String::new(),
            turns: String::new(),
            tokens: String::new(),
            lineage: String::new(),
        }
    }

    pub fn at(mut self, path: &str) -> Self {
        self.path = path.to_string();
        self
    }

    pub fn facts(mut self, turns: &str, tokens: &str, lineage: &str) -> Self {
        self.turns = turns.to_string();
        self.tokens = tokens.to_string();
        self.lineage = lineage.to_string();
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
    /// The row in hand: a copper bar and a tint, like every other selection.
    pub selected: bool,
}

impl TreeRow {
    pub fn new(depth: u16, twisty: Option<&str>, name: &str, status: Option<State>) -> Self {
        Self {
            depth,
            twisty: twisty.map(str::to_string),
            name: name.to_string(),
            status,
            selected: false,
        }
    }

    pub fn current(mut self) -> Self {
        self.selected = true;
        self
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

/// Whether a credential stands behind a provider or a catalog row (`3a`,
/// `3d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Credential {
    Ready,
    Pending,
    Expired,
    #[default]
    Absent,
    Local,
}

/// The facts a command sheet states in its header, status bar and
/// footnotes (design.md §11). Every field is optional in spirit: an empty
/// string or zero means the fact is not known live and its segment is
/// omitted, never invented.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// `3a` — `12 of 63 shown · 6 of 10 providers ready`.
    pub catalog_shown: usize,
    pub catalog_total: usize,
    pub providers_ready: usize,
    pub providers_total: usize,
    /// `3a` — `catalog refreshed ✓ 2h ago`.
    pub catalog_refreshed: String,
    /// `3a` — where the catalog lives.
    pub catalog_path: String,
    /// `3b` — how many keys the settings file knows.
    pub settings_keys: usize,
    /// `3c` — `reserve 10k`, the tokens held back from thinking.
    pub thinking_reserve: String,
    /// `3c` — `5.1k of 8k`, what the last turn thought.
    pub thinking_last_turn: String,
    /// `3c` — share of this session's output tokens spent thinking.
    pub thinking_output_share: f64,
    /// `3d` — where credentials are kept, and the mode they are written with.
    pub auth_path: String,
    pub auth_mode: String,
    /// `3e` — `39 bindings │ 4 surfaces`.
    pub keys_count: usize,
    pub keys_surfaces: usize,
    /// `4a` — `(used, cap)` bytes of the session store; cap 0 omits the meter.
    pub sessions_disk: Option<(u64, u64)>,
    /// `4b` — the session in hand.
    pub session_name: String,
    pub session_turns: usize,
    pub session_branches: usize,
    /// `4b` — `cost so far $0.84`.
    pub session_cost: String,
    /// `4b` — `4 user turns, 4 agent turns, 11 tool results · nothing compacted yet`.
    pub tree_summary: String,
    /// `4b` — `branch 06 has its own 9 turns and will not merge back`.
    pub tree_branch_note: String,
    /// `6d` — commits the branch is behind its upstream.
    pub commits_behind: Option<u32>,
    /// `6b` — `24 tools │ 37 commands │ 21.4k of schema`.
    pub tool_count: usize,
    pub command_count: usize,
    pub tool_schema_tokens: u64,
}

/// One row of the full model catalog (`3a`). Rows with no credential stay
/// listed, dimmed, so the catalog reads the same every time.
#[derive(Debug, Clone, Default)]
pub struct CatalogRow {
    pub name: String,
    /// What follows the name in border ink: `router :8080` for a local model.
    pub detail: String,
    pub window: String,
    pub thinking: String,
    pub price: String,
    pub credential: Credential,
    pub note: String,
    /// Whether the row is one of the models `--models` ringed for this run.
    pub ring: bool,
    /// The provider and id behind the row, so choosing it can switch.
    pub provider: String,
    pub id: String,
}

/// One setting (`3b`), with the ramp of values it accepts and its scope.
#[derive(Debug, Clone, Default)]
pub struct SettingRow {
    pub label: String,
    pub value: String,
    /// `true` when the project set it, overriding the user's value.
    pub project: bool,
    pub values: Vec<String>,
    pub description: String,
    /// The stored key behind the row, so a change knows where to land.
    pub key: String,
    /// What follows the ramp in border ink: `cells`, `registers /skill:name`.
    pub note: String,
}

/// One thinking level (`3c`). `fraction` is of the 64k ceiling, not of the
/// window; `warn` marks a level that takes a third of the window.
#[derive(Debug, Clone, Default)]
pub struct ThinkingRow {
    pub level: String,
    pub budget: String,
    pub fraction: f64,
    pub maps_to: String,
    pub warn: bool,
}

/// One provider credential and where it came from (`3d`).
#[derive(Debug, Clone, Default)]
pub struct ProviderRow {
    pub name: String,
    pub method: String,
    pub source: String,
    pub state: Credential,
}

/// The device-code grant in flight (`3d`).
#[derive(Debug, Clone, Default)]
pub struct DeviceCode {
    pub code: String,
    pub url: String,
    pub expires: String,
    pub polls: u32,
}

/// One group of the keymap (`3e`).
#[derive(Debug, Clone, Default)]
pub struct KeymapGroup {
    pub title: String,
    pub note: String,
    pub rows: Vec<(String, String)>,
}

/// One session of the resume list (`4a`), with what resuming it would carry.
#[derive(Debug, Clone, Default)]
pub struct ResumeRow {
    pub name: String,
    pub branch: String,
    pub turns: String,
    pub tokens: String,
    pub model: String,
    pub touched: String,
    pub named: bool,
    pub warning: Option<String>,
    pub note: String,
    pub last: String,
    pub path: String,
    /// `1.8 MB` — the jsonl on disk.
    pub size: String,
    /// The commit the session was last at, if known.
    pub commit: String,
}

/// One row of the session tree (`4b`). Rows with no `id` are spacers that
/// carry only the trunk, so the verticals stay continuous.
#[derive(Debug, Clone, Default)]
pub struct TreeNode {
    pub trunk: String,
    pub state: Option<State>,
    pub id: Option<String>,
    pub label: Option<String>,
    pub meta: Option<String>,
    /// The session entry behind the row, so choosing it can navigate.
    pub entry_id: String,
    /// A second row under the label: `abandoned · 2 files reverted`.
    pub detail: Option<String>,
}

/// What a compaction would do, before it does it (`4c`).
#[derive(Debug, Clone, Default)]
pub struct Compaction {
    pub before_tokens: String,
    pub before_fraction: f64,
    pub before_note: String,
    pub after_tokens: String,
    pub after_fraction: f64,
    pub after_note: String,
    pub kept: Vec<String>,
    pub folded: Vec<String>,
    pub recovers: String,
    pub call_cost: String,
    pub cache_cost: String,
}

/// What an export carries out of the session (`4d`).
#[derive(Debug, Clone, Default)]
pub struct ExportLedger {
    pub included: Vec<String>,
    pub excluded: Vec<(State, String)>,
    pub size: String,
    pub elapsed: String,
    pub gist: String,
}

/// One worker of a graph run (`5a`).
#[derive(Debug, Clone, Default)]
pub struct GraphTask {
    pub id: String,
    pub policy: String,
    pub artifact: String,
    pub usage: String,
    pub state: State,
}

/// A task running as a graph of isolated workers (`5a`).
#[derive(Debug, Clone, Default)]
pub struct GraphRunSheet {
    pub goal: String,
    pub phases: Vec<(String, State)>,
    pub shape: Vec<String>,
    pub tasks: Vec<GraphTask>,
    pub cost: String,
    pub cost_cap: String,
    pub cost_fraction: f64,
    pub workers: String,
    pub parallel: String,
    pub cycles: String,
    pub replans: String,
    pub artifacts: String,
}

/// The vector index itself (`5b`).
#[derive(Debug, Clone, Default)]
pub struct VectorIndex {
    pub repo: String,
    pub repo_records: String,
    pub total_records: String,
    pub injection_cap: String,
    pub floor: String,
    /// `(kind, count, fraction, note)`.
    pub kinds: Vec<(String, String, f64, String)>,
    pub embeddings: String,
    pub embed_host: String,
    pub store: String,
    pub collection: String,
    pub extraction: String,
    pub config: String,
    pub health: Vec<(State, String)>,
}

/// One counter of the governor's ledger (`5c`).
#[derive(Debug, Clone, Default)]
pub struct GovernorCounter {
    pub number: String,
    pub of: String,
    pub verb: String,
    pub note: String,
    /// Which theme colour the number is drawn in.
    pub tone: Tone,
}

/// One stored output the governor holds on disk (`5c`).
#[derive(Debug, Clone, Default)]
pub struct GovernorStored {
    pub id: String,
    pub tool: String,
    pub call: String,
    pub size: String,
    pub stale: bool,
}

/// What the governor did to this session's tool output (`5c`).
#[derive(Debug, Clone, Default)]
pub struct GovernorSheet {
    pub counters: Vec<GovernorCounter>,
    pub stored: Vec<GovernorStored>,
    pub store_dir: String,
}

/// One row of `/mcp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerRow {
    pub name: String,
    pub transport: String,
    pub status: String,
    pub tools: usize,
    pub error: Option<String>,
}

/// `/mcp` — connected servers, one row each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpSheet {
    pub servers: Vec<McpServerRow>,
    pub config_path: String,
}

/// One row of `/permissions`: a mode or a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRow {
    pub label: String,
    pub detail: String,
    pub current: bool,
    /// `mode` or `rule`.
    pub kind: String,
    pub key: String,
    /// `session`, `user`, `project`, or empty for a mode.
    pub source: String,
}

/// Which of the theme's inks a figure is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Primary,
    Secondary,
    Warning,
    Success,
}

/// A finding's severity band (`5d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    Critical,
    High,
    #[default]
    Medium,
    Low,
    Dismissed,
}

/// One security finding (`5d`).
#[derive(Debug, Clone, Default)]
pub struct Finding {
    pub message: String,
    pub location: String,
    pub severity: Severity,
    pub rule: String,
    pub evidence: String,
    pub path: String,
}

/// A security scan mid-validation (`5d`).
#[derive(Debug, Clone, Default)]
pub struct SecurityScan {
    pub validated: u32,
    pub candidates: u32,
    pub fraction: f64,
    pub files: String,
    pub skipped: String,
    pub bytes: String,
    pub severities: Vec<(String, u32, Severity)>,
    pub dismissed: u32,
    pub findings: Vec<Finding>,
    pub seal: String,
    pub report: String,
}

/// One file a project would load, before it is trusted (`6a`).
#[derive(Debug, Clone, Default)]
pub struct TrustFile {
    pub state: State,
    pub path: String,
    pub detail: String,
    pub risk_label: String,
}

/// What a project would load, before it is trusted (`6a`).
#[derive(Debug, Clone, Default)]
pub struct ProjectTrustSheet {
    pub files: Vec<TrustFile>,
    pub path: String,
    pub trusted: String,
    pub ignored: String,
    pub store: String,
}

/// What `/reload` loaded, and what it cost (`6b`).
#[derive(Debug, Clone, Default)]
pub struct WorkshopSheet {
    /// `(state, what, elapsed, error detail)`.
    pub reload: Vec<(State, String, String, Option<String>)>,
    pub native: Vec<(State, String, String)>,
    pub javascript: Vec<(State, String, String)>,
    pub node: String,
    pub schema: String,
    /// `(kind, count, fraction, note)`.
    pub tools: Vec<(String, String, f64, String)>,
}

/// The turn that did not complete, and the interrupt after it (`6c`).
#[derive(Debug, Clone, Default)]
pub struct FailedRun {
    pub prompt: String,
    pub tools: Vec<(State, String, String)>,
    pub kept: String,
    pub billed: String,
    pub aftermath: Vec<(State, String)>,
}

/// One file of the Δ review (`6d`), carrying its own hunk.
#[derive(Debug, Clone, Default)]
pub struct ReviewFile {
    pub state: State,
    pub path: String,
    pub adds: Option<u32>,
    pub dels: Option<u32>,
    pub tests: String,
    pub test_state: State,
    pub hunk_note: String,
    pub hunk: Vec<Hunk>,
}

/// Every file the turn changed (`6d`).
#[derive(Debug, Clone, Default)]
pub struct ReviewSheet {
    pub files: Vec<ReviewFile>,
    pub adds: u32,
    pub dels: u32,
    pub branch: String,
    pub behind: String,
    pub warning: String,
    pub tests: String,
}

/// What the session found when it opened — the empty state, screen `1a`.
#[derive(Debug, Clone, Default)]
pub struct Startup {
    pub cwd: String,
    pub branch: String,
    pub language: String,
    pub crates: String,
    pub restored: bool,
    /// What the session found when it opened — `loaded 1 context file · 41
    /// skills`, `models scoped to …` — one row each under the mark, so a
    /// fresh session opens on the `1a` screen instead of a transcript that
    /// begins with bookkeeping.
    pub found: Vec<String>,
}

/// Where the session lives, and what it costs. Every field here is shown as a
/// meter or a labelled unit, never as a bare number (design.md §9).
#[derive(Debug, Clone)]
pub struct Model {
    pub width: u16,
    pub height: u16,
    /// One clock, 250ms per step, driving both animations (design.md §8).
    pub tick: u64,
    /// The tick the caret last moved on. The blink phase is measured from here
    /// rather than from zero, so a caret being typed at or arrowed across is
    /// solid and only resumes blinking once the composer has been left alone.
    pub caret_moved_at: u64,
    pub animate: bool,
    pub theme: Theme,

    pub screen: Screen,
    pub overlay: Option<Overlay>,
    pub codex: bool,

    pub composer: Composer,
    /// Configurable bindings shared with the regular TUI editor.
    pub keybindings: Keybindings,
    /// `(key, extension path)` for every `pi.registerShortcut` an extension
    /// made, resolved against the keybindings so a shortcut never shadows a
    /// reserved chord. The shell dispatches these before its own keys.
    pub extension_shortcuts: Vec<(String, String)>,
    /// An extension asked to see raw terminal input (`onTerminalInput`); the
    /// shell offers it each chord before its own keys, as the legacy loop
    /// does through `dispatch_terminal_input`.
    pub terminal_input_registered: bool,
    /// How many completion rows the composer offers — the stored
    /// `autocomplete_max_visible`, clamped as the legacy chrome clamps it.
    pub suggestion_rows: usize,
    /// Whether the stored `show_terminal_progress` asked for OSC 9;4
    /// progress reports while a turn runs.
    pub terminal_progress: bool,
    /// The stored double-escape action — `tree`, `fork` or `none` — kept on
    /// the model so a settings change takes effect at once.
    pub double_escape_action: String,
    /// Turns typed while one was already running, waiting their place. They
    /// sit in the composer box above the line being typed, so what is waiting
    /// is visible rather than remembered.
    pub queued: Vec<String>,
    pub query: String,
    pub transcript: Vec<Entry>,
    /// Rows the loaded extensions have asked for.
    pub extensions: Extensions,
    pub running: bool,
    /// The working line's numbers while a turn is under way. `None` between
    /// turns, which is what takes the row off the window.
    pub working: Option<Working>,

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
    /// Active thinking/reasoning level for the model in hand (`off`, `high`, ...).
    pub thinking_level: String,
    /// The permission mode in force (`ask`, `edits`, `auto`, `read-only`).
    /// The header names it only when it is `auto`: the one state that is
    /// worth a permanent reminder.
    pub permission_mode: String,
    /// `/plan` is on: mutations are frozen until `/act`.
    pub plan_mode: bool,
    /// Whether tool lines show what they came back with (`ctrl+t`, the
    /// `showToolOutput` setting). Off, a call is one line; on, up to twelve
    /// rows of its output follow it.
    pub show_tool_output: bool,
    /// Background jobs still running, for the status bar's `· 2 jobs`.
    pub jobs_running: usize,
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
    /// What `/` offers, and what its arguments offer. Supplied by the shell so
    /// davinci and the legacy chrome complete against the same corpus through
    /// the same engine (`crate::autocomplete`, which mirrors
    /// `vendor/pi/packages/tui/src/autocomplete.ts`).
    pub slash_commands: Vec<SlashCommandSpec>,
    pub model_names: Vec<String>,
    pub thinking_levels: Vec<String>,
    pub login_providers: Vec<String>,
    /// Autocomplete providers a loaded extension registered, keyed by their
    /// trigger characters (`#` and friends).
    pub extra_autocomplete: Vec<ExtraAutocompleteProvider>,
    /// What the composer is offering right now, and which row is marked. A
    /// list open here owns the arrows, tab and enter until it is taken or
    /// dismissed (design.md §6: the most specific layer wins).
    pub suggestions: Option<AutocompleteSuggestions>,
    pub suggestion_index: usize,
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

    // --- screens 3a–6d ------------------------------------------------------
    /// `3a` — the full model catalog, and its selection.
    pub catalog: Vec<CatalogRow>,
    pub catalog_index: usize,
    /// `3b` — the settings sheet.
    pub settings_rows: Vec<SettingRow>,
    pub settings_index: usize,
    /// `3c` — the thinking sheet.
    pub thinking_rows: Vec<ThinkingRow>,
    pub thinking_index: usize,
    /// `3d` — provider credentials, and the grant in flight if any.
    pub providers: Vec<ProviderRow>,
    pub login_index: usize,
    pub device_code: Option<DeviceCode>,
    /// `3e` — the keymap, and how far it is scrolled.
    pub keymap: Vec<KeymapGroup>,
    pub keys_offset: usize,
    /// `4a` — the resume list, and how many sessions exist in all.
    pub resume_sessions: Vec<ResumeRow>,
    pub resume_index: usize,
    pub session_count: usize,
    /// `4b` — the session tree.
    pub session_tree: Vec<TreeNode>,
    pub tree_index: usize,
    /// `4c` — the compaction preview.
    pub compaction: Option<Compaction>,
    /// `4d` — the export ledger.
    pub export_ledger: Option<ExportLedger>,
    /// `5a` — the graph run.
    pub graph_run: Option<GraphRunSheet>,
    /// `5b` — the vector index.
    pub vector_index: Option<VectorIndex>,
    /// `5c` — the governor's ledger.
    pub governor: Option<GovernorSheet>,
    /// `5d` — the security scan, and its selection.
    pub security: Option<SecurityScan>,
    pub security_index: usize,
    /// `6a` — project trust.
    pub project_trust: Option<ProjectTrustSheet>,
    /// `6b` — the workshop.
    pub workshop: Option<WorkshopSheet>,
    /// `6c` — the interrupt aftermath.
    pub failed_run: Option<FailedRun>,
    /// `6d` — the Δ review, and which file is in hand.
    pub review: Option<ReviewSheet>,
    pub diff_index: usize,
    /// `/mcp` — connected MCP servers.
    pub mcp: Option<McpSheet>,
    /// `/permissions` — mode and rules.
    pub permission_rows: Vec<PermissionRow>,
    pub permission_index: usize,
    /// What the command sheets state about the session (design.md §11).
    pub facts: Facts,
}

impl Model {
    pub fn new(theme: Theme, width: u16, height: u16, animate: bool) -> Self {
        Self {
            width,
            height,
            tick: 0,
            caret_moved_at: 0,
            animate,
            theme,
            screen: Screen::Agent,
            overlay: None,
            codex: false,
            composer: Composer::default(),
            keybindings: Keybindings::defaults(),
            extension_shortcuts: Vec::new(),
            terminal_input_registered: false,
            suggestion_rows: SUGGESTION_ROWS,
            terminal_progress: false,
            double_escape_action: "tree".into(),
            queued: Vec::new(),
            extensions: Extensions::default(),
            query: String::new(),
            transcript: Vec::new(),
            running: false,
            working: None,
            palette_index: 0,
            session_index: 0,
            model_index: 0,
            recall_index: 0,
            ask: Ask::default(),
            ask_index: 0,
            cwd: String::new(),
            branch: String::new(),
            model_name: String::new(),
            thinking_level: "off".into(),
            permission_mode: "ask".into(),
            plan_mode: false,
            show_tool_output: false,
            jobs_running: 0,
            changes: (0, 0, 0),
            context: (0, 200_000),
            startup: Startup::default(),
            corpus: Vec::new(),
            corpus_total: 0,
            paths: Vec::new(),
            slash_commands: Vec::new(),
            model_names: Vec::new(),
            thinking_levels: Vec::new(),
            login_providers: Vec::new(),
            extra_autocomplete: Vec::new(),
            suggestions: None,
            suggestion_index: 0,
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
            catalog: Vec::new(),
            catalog_index: 0,
            settings_rows: Vec::new(),
            settings_index: 0,
            thinking_rows: Vec::new(),
            thinking_index: 0,
            providers: Vec::new(),
            login_index: 0,
            device_code: None,
            keymap: Vec::new(),
            keys_offset: 0,
            resume_sessions: Vec::new(),
            resume_index: 0,
            session_count: 0,
            session_tree: Vec::new(),
            tree_index: 0,
            compaction: None,
            export_ledger: None,
            graph_run: None,
            vector_index: None,
            governor: None,
            security: None,
            security_index: 0,
            project_trust: None,
            workshop: None,
            failed_run: None,
            review: None,
            diff_index: 0,
            mcp: None,
            permission_rows: Vec::new(),
            permission_index: 0,
            facts: Facts::default(),
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
    /// The row a sheet's window is kept around: its selection, or the top.
    pub fn sheet_anchor(&self) -> usize {
        match self.screen {
            Screen::Models => self.catalog_index,
            Screen::Settings => self.settings_index,
            Screen::Thinking => self.thinking_index,
            Screen::Login => self.login_index,
            Screen::Keys => self.keys_offset,
            Screen::Resume => self.resume_index,
            Screen::Tree => self.tree_index,
            Screen::Securitas => self.security_index,
            Screen::Diff => self.diff_index,
            Screen::Permissions => self.permission_index,
            _ => 0,
        }
    }

    pub fn mode(&self) -> &'static str {
        match self.screen {
            Screen::Agent | Screen::Recovery | Screen::Diff if self.plan_mode => "plan",
            Screen::Agent | Screen::Recovery | Screen::Diff => "agent",
            Screen::Plan => "plan",
            Screen::Grafo | Screen::GraphRun => "grafo",
            Screen::Memoria | Screen::Resume | Screen::Tree | Screen::Export | Screen::Vectors => {
                "memoria"
            }
            Screen::Mensura | Screen::Compact | Screen::Governor => "mensura",
            Screen::Models | Screen::Thinking | Screen::Login => "cogitator",
            Screen::Settings => "settings",
            Screen::Keys => "keys",
            Screen::Securitas => "securitas",
            Screen::Trust => "fiducia",
            Screen::Officina => "officina",
            Screen::Mcp => "instrumenta",
            Screen::Permissions => "fiducia",
        }
    }

    /// The caret blinks at ~1s, step-end, off the same clock as the spinner.
    ///
    /// The phase is measured from the last caret move, not from zero, so the
    /// caret is solid while it is being typed at or arrowed across and only
    /// resumes blinking a full phase after the composer goes still. A caret
    /// that winks out mid-motion reads as a dropped keystroke.
    pub fn blink(&self) -> bool {
        if !self.animate {
            return true;
        }
        (self.tick.wrapping_sub(self.caret_moved_at) / 4) % 2 == 0
    }

    /// Restart the blink phase: the caret is somewhere new, so it is drawn
    /// solid from here. `tick` wraps, so the phase arithmetic wraps with it.
    pub fn mark_caret_moved(&mut self) {
        self.caret_moved_at = self.tick;
    }

    pub fn context_fraction(&self) -> f64 {
        let (used, cap) = self.context;
        if cap == 0 {
            return 0.0;
        }
        (used as f64 / cap as f64).clamp(0.0, 1.0)
    }

    // --- reducers ------------------------------------------------------------

    /// A bracketed paste: text, arriving whole. Newlines stay newlines in the
    /// composer — the block was pasted, not typed, so none of them is a
    /// submit — and are flattened in a query, which is one line by definition.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.overlay == Some(Overlay::Instrumenta) {
            self.type_char(&text.replace('\n', " "));
        } else {
            self.type_char(&text);
        }
    }

    pub fn type_char(&mut self, text: &str) {
        self.mark_caret_moved();
        if self.overlay == Some(Overlay::Instrumenta) {
            self.query.push_str(text);
            self.palette_index = 0;
        } else {
            self.composer.push_str(text);
            self.refresh_suggestions();
        }
    }

    /// Recompute what the composer offers, after every edit that could change
    /// it. Slash names and their arguments are matched in memory; only an `@`
    /// token reaches the disk, which is what keeps this cheap enough to run on
    /// each keystroke.
    pub fn refresh_suggestions(&mut self) {
        if self.overlay.is_some() || self.screen != Screen::Agent || self.codex_open() {
            self.suggestions = None;
            self.suggestion_index = 0;
            return;
        }
        let text = self.composer.to_string();
        let found = suggestions(SuggestionQuery {
            text: &text,
            commands: &self.slash_commands,
            models: &self.model_names,
            thinking_levels: &self.thinking_levels,
            login_providers: &self.login_providers,
            extra_providers: &self.extra_autocomplete,
            cwd: Path::new(&self.cwd),
            force_path: false,
        });
        // The whole list is kept: the composer window shows a slice of it
        // around the selection, so a provider past the fold is still
        // reachable with ↓ — truncating here silently lost everything past
        // the cap.
        self.suggestions = found;
        self.suggestion_index = 0;
    }

    /// The visible slice of the suggestion list: `suggestion_rows` around the
    /// selection. What is folded is counted by the view either side.
    pub fn suggestion_window(&self) -> (usize, usize) {
        let Some(found) = &self.suggestions else {
            return (0, 0);
        };
        let len = found.items.len();
        let window = self.suggestion_rows.max(1);
        if len <= window {
            return (0, len);
        }
        let start = self
            .suggestion_index
            .saturating_sub(window / 2)
            .min(len - window);
        (start, start + window)
    }

    /// Move down (`+1`) or up (`-1`) the offered list, wrapping at both ends.
    /// Returns whether there was a list to move within.
    pub fn suggestion_move(&mut self, delta: isize) -> bool {
        let Some(found) = &self.suggestions else {
            return false;
        };
        let len = found.items.len();
        if len == 0 {
            return false;
        }
        let index = self.suggestion_index.min(len - 1) as isize;
        self.suggestion_index = (index + delta).rem_euclid(len as isize) as usize;
        true
    }

    /// Take the marked row into the composer, replacing the token it completes.
    pub fn accept_suggestion(&mut self) -> bool {
        let Some(found) = self.suggestions.clone() else {
            return false;
        };
        let Some(item) = found.items.get(self.suggestion_index) else {
            return false;
        };
        let text = self.composer.to_string();
        let cursor = self.composer.editor().cursor.min(text.len());
        let completed = apply_completion(&text, cursor, &found.prefix, item);
        // Taking the row the composer already holds is not a completion. The
        // list steps aside so the next enter sends, rather than trapping the
        // user in a suggestion they have already accepted.
        if completed == text {
            self.dismiss_suggestions();
            return false;
        }
        self.mark_caret_moved();
        self.composer.set_text(completed);
        let end = self.composer.editor().buffer.len();
        self.composer.editor_mut().cursor = end;
        // What the composer now holds may itself be completable — `/model `
        // offers the models — so ask again rather than closing blind.
        self.refresh_suggestions();
        true
    }

    /// esc with a list open closes the list rather than the screen behind it.
    pub fn dismiss_suggestions(&mut self) -> bool {
        if self.suggestions.is_none() {
            return false;
        }
        self.suggestions = None;
        self.suggestion_index = 0;
        true
    }

    /// A newline inside the composer. The hint line promises `shift+enter
    /// newline`; this is what keeps that promise.
    pub fn newline(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        self.mark_caret_moved();
        self.composer.push('\n');
        self.refresh_suggestions();
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
        // Past the whitespace, not one byte past its start: a non-breaking
        // space is whitespace and is two bytes wide, so `index + 1` landed
        // mid-character and the slice below panicked.
        let start = self
            .composer
            .char_indices()
            .filter(|(_, ch)| ch.is_whitespace())
            .next_back()
            .map(|(index, ch)| index + ch.len_utf8())
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
        self.mark_caret_moved();
        if self.overlay == Some(Overlay::Instrumenta) {
            self.query.pop();
            self.palette_index = 0;
        } else {
            self.composer.editor_mut().backspace();
            self.refresh_suggestions();
        }
    }

    /// Enter sends. An empty composer sends nothing.
    ///
    /// The line goes through [`Editor::submit`], which is what records it in
    /// the composer history and expands any paste markers — taking the buffer
    /// directly shipped markers literally and remembered nothing.
    pub fn submit(&mut self) {
        if self.composer.trim().is_empty() {
            return;
        }
        self.mark_caret_moved();
        let text = self.composer.editor_mut().submit();
        self.dismiss_suggestions();
        if !self.transcript.is_empty() {
            self.transcript.push(Entry::Gap);
        }
        self.transcript.push(Entry::user(&text));
        self.transcript.push(Entry::Gap);
        self.transcript.push(Entry::agent("davinci"));
        self.running = true;
    }

    /// Set the composer aside to be sent when the running turn is over.
    /// Returns whether anything was queued. Queued lines join the history
    /// too: they were typed and sent, only later.
    pub fn queue(&mut self) -> bool {
        if self.composer.trim().is_empty() {
            return false;
        }
        self.mark_caret_moved();
        let text = self.composer.editor_mut().submit();
        self.queued.push(text);
        self.dismiss_suggestions();
        true
    }

    /// ctrl+c interrupts the run, never the app (design.md §6). What was
    /// waiting to be sent goes with it: the user stopped this train of
    /// thought, not just this turn.
    pub fn interrupt(&mut self) {
        self.running = false;
        self.queued.clear();
    }

    /// Keep the transcript bounded. A multi-hour session grows without limit
    /// otherwise, and every entry kept is memory held for the life of the
    /// process. The cut lands on a block boundary so no block is beheaded.
    pub fn trim_transcript(&mut self) {
        const TRANSCRIPT_CAP: usize = 4000;
        if self.transcript.len() <= TRANSCRIPT_CAP {
            return;
        }
        let mut cut = self.transcript.len() - TRANSCRIPT_CAP;
        while cut < self.transcript.len() && !matches!(self.transcript[cut], Entry::Gap) {
            cut += 1;
        }
        self.transcript.drain(..cut);
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

    fn with_commands() -> Model {
        let mut m = model(100);
        m.slash_commands = ["settings", "sessions", "model", "compact"]
            .into_iter()
            .map(|name| SlashCommandSpec {
                name: name.to_string(),
                description: format!("the {name} command"),
                argument_hint: None,
                argument_items: Vec::new(),
            })
            .collect();
        m
    }

    #[test]
    fn typing_a_slash_offers_the_commands_it_could_be() {
        let mut m = with_commands();
        m.type_char("/");
        let offered = m.suggestions.as_ref().expect("a bare slash offers all");
        assert_eq!(offered.items.len(), 4);

        m.type_char("se");
        let names: Vec<&str> = m
            .suggestions
            .as_ref()
            .expect("/se still matches")
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert!(names.contains(&"settings"), "got {names:?}");
        assert!(names.contains(&"sessions"), "got {names:?}");
        assert!(!names.contains(&"compact"), "got {names:?}");
    }

    #[test]
    fn nothing_is_offered_for_ordinary_prose() {
        let mut m = with_commands();
        m.type_char("explain the runtime");
        assert!(m.suggestions.is_none());
    }

    #[test]
    fn taking_a_row_puts_it_in_the_composer() {
        let mut m = with_commands();
        m.type_char("/se");
        m.suggestion_move(1);
        let taken = m.suggestions.as_ref().unwrap().items[m.suggestion_index]
            .label
            .clone();
        assert!(m.accept_suggestion());
        assert_eq!(m.composer, format!("/{taken} "));
    }

    #[test]
    fn a_row_already_in_the_composer_is_not_a_completion() {
        let mut m = with_commands();
        m.type_char("/compact");
        // `/compact ` is what taking the only row would produce, so the first
        // enter completes it and the second must be free to send.
        assert!(m.accept_suggestion());
        assert_eq!(m.composer, "/compact ");
        assert!(!m.accept_suggestion());
        assert!(m.suggestions.is_none());
    }

    #[test]
    fn escape_closes_the_list_before_anything_else() {
        let mut m = with_commands();
        m.type_char("/se");
        assert!(m.dismiss_suggestions());
        assert!(m.suggestions.is_none());
        assert!(!m.dismiss_suggestions());
        assert_eq!(m.composer, "/se", "the composer is not disturbed");
    }

    #[test]
    fn backspacing_narrows_what_is_offered() {
        let mut m = with_commands();
        m.type_char("/settingsx");
        assert!(m.suggestions.is_none(), "nothing matches that");
        m.backspace();
        assert!(m.suggestions.is_some(), "removing the typo offers again");
    }

    #[test]
    fn an_open_instrument_takes_the_offer_off_the_screen() {
        let mut m = with_commands();
        m.type_char("/se");
        assert!(m.suggestions.is_some());
        m.toggle_overlay(Overlay::Instrumenta);
        m.refresh_suggestions();
        assert!(m.suggestions.is_none());
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
    fn a_caret_being_moved_does_not_blink() {
        let mut m = model(120);
        m.type_char("hello");
        // Six ticks after the last keystroke would be the dark half of the
        // phase if the phase ran from zero.
        m.tick = 6;
        assert!(!m.blink(), "left alone, it blinks");

        // Typing at that same tick restarts the phase.
        m.type_char("!");
        assert!(m.blink(), "a caret being typed at is solid");

        // And so does moving it, without editing anything.
        m.tick = 10;
        assert!(!m.blink(), "still again, blinking again");
        m.composer.editor_mut().move_left();
        m.mark_caret_moved();
        assert!(m.blink(), "a caret being arrowed across is solid");

        // Solid for the whole phase after the last move, then blinking.
        m.tick = 13;
        assert!(m.blink(), "solid to the end of the phase");
        m.tick = 14;
        assert!(!m.blink(), "and dark on the next one");
    }

    #[test]
    fn the_mode_word_follows_the_screen() {
        let mut m = model(120);
        assert_eq!(m.mode(), "agent");
        m.toggle_screen(Screen::Plan);
        assert_eq!(m.mode(), "plan");
        m.toggle_screen(Screen::Mensura);
        assert_eq!(m.mode(), "mensura");
        m.screen = Screen::Agent;
        m.plan_mode = true;
        assert_eq!(m.mode(), "plan");
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

        m.composer = String::new().into();
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
    fn completing_after_a_non_breaking_space_does_not_split_a_character() {
        // U+00A0 is whitespace and two bytes wide. Stepping one byte past its
        // start landed mid-character, and the slice that follows panicked —
        // inside the alternate screen, on text pasted from a browser.
        let mut m = model(100);
        m.corpus = vec![CorpusItem::new("/compact", "", "command")];
        m.composer = "look\u{a0}/comp".into();
        assert!(m.complete());
        assert!(
            m.composer.to_string().starts_with("look\u{a0}/compact"),
            "{}",
            m.composer.to_string()
        );

        m.composer = "look\u{3000}at".into();
        m.complete();
    }

    #[test]
    fn a_paste_is_text_not_a_run_of_submits() {
        let mut m = model(100);
        m.paste("first line\r\nsecond line");
        assert_eq!(m.composer.to_string(), "first line\nsecond line");

        // A query is one line by definition, so a pasted block flattens.
        let mut m = model(100);
        m.toggle_overlay(Overlay::Instrumenta);
        m.paste("one\ntwo");
        assert_eq!(m.query, "one two");
    }

    #[test]
    fn the_transcript_is_bounded_and_cut_on_a_block_boundary() {
        let mut m = model(100);
        for turn in 0..3000 {
            m.transcript.push(Entry::Gap);
            m.transcript.push(Entry::user(&format!("turn {turn}")));
        }
        m.trim_transcript();
        assert!(m.transcript.len() <= 4000, "{}", m.transcript.len());
        // The cut lands on a Gap so no block is beheaded.
        assert!(matches!(m.transcript.first(), Some(Entry::Gap)));
        // The newest turns survive.
        assert!(m
            .transcript
            .iter()
            .any(|entry| matches!(entry, Entry::User(text) if text == "turn 2999")));
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        assert_eq!(wrap_index(0, -1, 5), 4);
        assert_eq!(wrap_index(4, 1, 5), 0);
        assert_eq!(wrap_index(2, 1, 5), 3);
        assert_eq!(wrap_index(0, 1, 0), 0);
    }
}
