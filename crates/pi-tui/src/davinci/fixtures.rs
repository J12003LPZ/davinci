//! Screen content, as plain data, taken from the mockups.
//!
//! These exist so each screen can be reproduced in a real terminal and matched
//! against `docs/ui/Pi TUI Mockups.dc.html` before it is wired to the real
//! session store, git status, token accountant and code graph. Everything here
//! is scheduled for replacement; nothing else in the tree may depend on it.

use super::model::{
    ChangeRow, CorpusItem, Entry, Hunk, HunkKind, Model, ModelItem, PlanStep, SessionItem, Startup,
    Step, TreeRow,
};
use super::theme::State;

/// `1a` — what the session found when it opened.
pub fn startup() -> Startup {
    Startup {
        cwd: "C:\\dev\\oss\\davinci-rust".into(),
        branch: "main".into(),
        language: "rust".into(),
        crates: "11 crates".into(),
        restored: true,
    }
}

/// `1b` — transcript with tools, Studio and a Δ block.
pub fn transcript() -> Vec<Entry> {
    vec![
        Entry::user("explain how the agent runtime works"),
        Entry::Gap,
        Entry::agent("davinci"),
        Entry::Gap,
        Entry::tool(
            State::Read,
            "instrumenta",
            "read crates\\davinci-agent\\src\\lib.rs",
            None,
        ),
        Entry::tool(
            State::Search,
            "instrumenta",
            "search \"SessionManager\" · 8 matches",
            None,
        ),
        Entry::tool(
            State::Done,
            "manus",
            "cargo check -p davinci-agent",
            Some("1.84s"),
        ),
        Entry::tool(
            State::Failed,
            "manus",
            "cargo test -p davinci-session",
            Some("0.42s"),
        ),
        Entry::detail("error[E0308] mismatched types · store.rs:118"),
        Entry::Gap,
        Entry::Studio(vec![
            Step::new(State::Done, "surveyed workspace", None),
            Step::new(State::Done, "traced request pipeline", None),
            Step::new(
                State::Active,
                "examining session persistence",
                Some("davinci-session\\src\\store.rs"),
            ),
            Step::new(State::Queued, "verify provider abstraction", None),
        ]),
        Entry::Gap,
        Entry::prose(
            "A request enters davinci-agent as a Turn, is planned, then dispatched to the \
             provider adapter. Session state is written after every tool call, so an \
             interrupt never loses the transcript.",
        ),
        Entry::Gap,
        Entry::Delta {
            path: "crates\\davinci-agent\\src\\runtime.rs".into(),
            adds: 31,
            dels: 8,
            hunks: vec![
                Hunk::new(HunkKind::Add, "pub async fn execute_stream("),
                Hunk::new(HunkKind::Add, "    &self, req: Request, tx: Sender<Chunk>,"),
                Hunk::new(HunkKind::Add, ") -> Result<Usage> {"),
                Hunk::new(HunkKind::Del, "    self.execute(req).await"),
            ],
        },
    ]
}

/// `1g` / `1h` — the narrow transcript.
pub fn narrow_transcript() -> Vec<Entry> {
    vec![
        Entry::user("run the tests"),
        Entry::Gap,
        Entry::agent("davinci"),
        Entry::Gap,
        Entry::tool(State::Done, "manus", "cargo fmt", Some("0.31s")),
        Entry::tool(State::Done, "manus", "cargo clippy", Some("4.10s")),
        Entry::tool(State::Failed, "manus", "cargo test", Some("6.72s")),
        Entry::tool(
            State::Attention,
            "manus",
            "1 failed store::roundtrip_windows_paths",
            None,
        ),
        Entry::Gap,
        Entry::Studio(vec![
            Step::new(State::Done, "surveyed workspace", None),
            Step::new(
                State::Active,
                "examining session persistence",
                Some("davinci-session\\src\\store.rs"),
            ),
            Step::new(State::Queued, "verify provider abstraction", None),
        ]),
        Entry::Gap,
        Entry::prose(
            "The failing case builds a path with a forward slash. I will switch it to \
             Path::join and re-run.",
        ),
    ]
}

/// `1d` — the Instrumenta corpus: tools, sessions, files, modes.
pub fn corpus() -> Vec<CorpusItem> {
    vec![
        CorpusItem::new("/git status", "working tree · 3 modified", "tool"),
        CorpusItem::new("/git diff", "unstaged hunks", "tool"),
        CorpusItem::new("/git commit", "stage all · write message", "tool"),
        CorpusItem::new("memoria: fix-git-hooks", "session · 2 days ago", "session"),
        CorpusItem::new(".gitignore", "C:\\dev\\oss\\davinci-rust", "file"),
        CorpusItem::new("crates\\davinci-git\\src\\lib.rs", "414 lines", "file"),
        CorpusItem::new("mode: git-worktree", "isolate edits per turn", "mode"),
    ]
}

/// How much of the corpus there is, so the palette can say `7 of 214`.
pub const CORPUS_TOTAL: usize = 214;

/// `1f` — Memoria sessions.
pub fn sessions() -> Vec<SessionItem> {
    vec![
        SessionItem::new("review-agent-runtime", "3m"),
        SessionItem::new("implement-rpc-mode", "18m"),
        SessionItem::new("provider-parity", "1h"),
        SessionItem::new("tui-redesign", "yesterday"),
        SessionItem::new("fix-git-hooks", "2d"),
    ]
}

/// `1f` — Cogitator models.
pub fn models() -> Vec<ModelItem> {
    vec![
        ModelItem::new("anthropic / sonnet", "200k"),
        ModelItem::new("anthropic / opus", "200k"),
        ModelItem::new("openai / gpt", "128k"),
        ModelItem::new("google / gemini", "1m"),
        ModelItem::new("local / ollama", "32k"),
    ]
}

/// `1e` — the Codex transcript, a windows-path bug.
pub fn codex_transcript() -> Vec<Entry> {
    vec![
        Entry::user("why does the session store fail on windows paths?"),
        Entry::Gap,
        Entry::agent("davinci"),
        Entry::Gap,
        Entry::tool(
            State::Search,
            "instrumenta",
            "search \"PathBuf::from\" · 12 matches",
            None,
        ),
        Entry::tool(
            State::Read,
            "instrumenta",
            "read crates\\davinci-session\\src\\store.rs",
            None,
        ),
        Entry::tool(
            State::Failed,
            "manus",
            "cargo test -p davinci-session",
            Some("0.42s"),
        ),
        Entry::Gap,
        Entry::prose(
            "The store joins session ids with a literal / instead of Path::join, so a \
             restored path becomes C:\\Users\\ines\\.davinci/sessions/…. Canonicalising \
             it fixes both platforms.",
        ),
        Entry::Gap,
        Entry::Delta {
            path: "crates\\davinci-session\\src\\store.rs".into(),
            adds: 6,
            dels: 3,
            hunks: vec![
                Hunk::new(
                    HunkKind::Del,
                    "let p = format!(\"{}/sessions/{}\", root, id);",
                ),
                Hunk::new(HunkKind::Add, "let p = root.join(\"sessions\").join(id);"),
            ],
        },
    ]
}

/// `1c` — the Disegno plan.
pub fn plan() -> Vec<PlanStep> {
    vec![
        PlanStep::new(
            "I",
            State::Done,
            "Map provider abstraction",
            Some("davinci-ai\\src\\provider.rs"),
        ),
        PlanStep::new("II", State::Done, "Trace session lifecycle", None),
        PlanStep::new("III", State::Active, "Implement streaming adapter", None),
        PlanStep::new(
            "IV",
            State::Queued,
            "Add parity fixtures against TS davinci",
            None,
        ),
        PlanStep::new(
            "V",
            State::Queued,
            "Verify workspace: fmt, clippy, test",
            None,
        ),
    ]
}

/// `1e` — the Codex file tree, already flattened to (depth, twisty, name).
pub fn tree() -> Vec<TreeRow> {
    vec![
        TreeRow::new(0, Some("▾"), "crates", None),
        TreeRow::new(1, Some("▸"), "davinci-agent", Some(State::Delta)),
        TreeRow::new(1, Some("▸"), "davinci-ai", None),
        TreeRow::new(1, Some("▸"), "davinci-client", None),
        TreeRow::new(1, Some("▾"), "davinci-session", Some(State::Delta)),
        TreeRow::new(2, None, "store.rs", Some(State::Failed)),
        TreeRow::new(2, None, "manager.rs", None),
        TreeRow::new(1, Some("▸"), "davinci-tui", Some(State::Delta)),
        TreeRow::new(1, Some("▸"), "davinci-git", None),
        TreeRow::new(0, None, "Cargo.toml", None),
        TreeRow::new(0, None, "Cargo.lock", None),
        TreeRow::new(0, None, "README.md", None),
    ]
}

/// `1e` — the git changes popover.
pub fn changes_list() -> Vec<ChangeRow> {
    vec![
        ChangeRow::new("M", "davinci-session\\src\\store.rs", "+6"),
        ChangeRow::new("M", "davinci-agent\\src\\runtime.rs", "+31"),
        ChangeRow::new("A", "davinci-tui\\src\\theme.rs", "+54"),
    ]
}

/// A model dressed with the mockups' session, for eyeballing a screen.
pub fn dress(model: &mut Model) {
    model.cwd = "C:\\dev\\oss\\davinci-rust".into();
    model.branch = "main".into();
    model.model_name = "sonnet".into();
    model.changes = (3, 42, 11);
    model.context = (47_000, 200_000);
    model.startup = startup();
    model.corpus = corpus();
    model.corpus_total = CORPUS_TOTAL;
    model.sessions = sessions();
    model.models = models();
    model.plan = plan();
    model.tree = tree();
    model.changes_list = changes_list();
    model.transcript = if model.narrow() {
        narrow_transcript()
    } else {
        transcript()
    };
}

/// Dress the model as one named mockup screen, so it can be matched against
/// `docs/ui/Pi TUI Mockups.dc.html` in a real terminal.
pub fn dress_screen(model: &mut Model, id: &str) {
    dress(model);
    match id {
        "1a" => model.transcript.clear(),
        "1d" => model.toggle_overlay(crate::davinci::model::Overlay::Instrumenta),
        "1f" => model.toggle_overlay(crate::davinci::model::Overlay::Sessions),
        "1f-cogitator" => model.toggle_overlay(crate::davinci::model::Overlay::Cogitator),
        "1c" => model.toggle_screen(crate::davinci::model::Screen::Plan),
        "1e" => {
            model.toggle_codex();
            model.transcript = codex_transcript();
        }
        "1g" | "1h" => model.transcript = narrow_transcript(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    #[test]
    fn the_mockup_transcripts_carry_every_block_kind() {
        let entries = transcript();
        assert!(entries.iter().any(|e| matches!(e, Entry::User(_))));
        assert!(entries.iter().any(|e| matches!(e, Entry::Agent(_))));
        assert!(entries.iter().any(|e| matches!(e, Entry::Tool { .. })));
        assert!(entries.iter().any(|e| matches!(e, Entry::Detail(_))));
        assert!(entries.iter().any(|e| matches!(e, Entry::Studio(_))));
        assert!(entries.iter().any(|e| matches!(e, Entry::Prose(_))));
        assert!(entries.iter().any(|e| matches!(e, Entry::Delta { .. })));
    }

    #[test]
    fn dressing_a_narrow_window_drops_to_the_narrow_transcript() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let mut wide = Model::new(theme, 100, 44, true);
        dress(&mut wide);
        assert_eq!(wide.transcript.len(), transcript().len());

        let mut narrow = Model::new(theme, 80, 24, true);
        dress(&mut narrow);
        assert_eq!(narrow.transcript.len(), narrow_transcript().len());
    }
}
