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
    pub query: String,
    pub transcript: Vec<Entry>,
    pub running: bool,

    pub palette_index: usize,
    pub session_index: usize,
    pub model_index: usize,
    pub recall_index: usize,

    pub cwd: String,
    pub branch: String,
    pub model_name: String,
    /// `Δn +a -d` for the status bar.
    pub changes: (u32, u32, u32),
    /// `(used, cap)` in tokens.
    pub context: (u64, u64),
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
            query: String::new(),
            transcript: Vec::new(),
            running: false,
            palette_index: 0,
            session_index: 0,
            model_index: 0,
            recall_index: 0,
            cwd: String::new(),
            branch: String::new(),
            model_name: String::new(),
            changes: (0, 0, 0),
            context: (0, 200_000),
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

    /// ctrl+c interrupts the run, never the app (design.md §6).
    pub fn interrupt(&mut self) {
        self.running = false;
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

/// Move a selection index by `delta`, wrapping at both ends.
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
    fn selection_wraps_at_both_ends() {
        assert_eq!(wrap_index(0, -1, 5), 4);
        assert_eq!(wrap_index(4, 1, 5), 0);
        assert_eq!(wrap_index(2, 1, 5), 3);
        assert_eq!(wrap_index(0, 1, 0), 0);
    }
}
