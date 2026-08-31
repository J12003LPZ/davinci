//! Screen content, as plain data, taken from the mockups.
//!
//! These exist so each screen can be reproduced in a real terminal and matched
//! against `docs/ui/Pi TUI Mockups.dc.html` before it is wired to the real
//! session store, git status, token accountant and code graph. Everything here
//! is scheduled for replacement; nothing else in the tree may depend on it.

use super::model::{Entry, Hunk, HunkKind, Model, Startup, Step};
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

/// A model dressed with the mockups' session, for eyeballing a screen.
pub fn dress(model: &mut Model) {
    model.cwd = "C:\\dev\\oss\\davinci-rust".into();
    model.branch = "main".into();
    model.model_name = "sonnet".into();
    model.changes = (3, 42, 11);
    model.context = (47_000, 200_000);
    model.startup = startup();
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
