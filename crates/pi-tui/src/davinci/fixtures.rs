//! Screen content, as plain data, taken from the mockups.
//!
//! These exist so each screen can be reproduced in a real terminal and matched
//! against `docs/ui/Pi TUI Mockups.dc.html` before it is wired to the real
//! session store, git status, token accountant and code graph. Everything here
//! is scheduled for replacement; nothing else in the tree may depend on it.

use super::model::{
    BudgetMeta, BudgetRow, CatalogRow, ChangeRow, Compaction, CorpusItem, Credential, DeviceCode,
    Entry, ExportLedger, FailedRun, Finding, GovernorCounter, GovernorSheet, GovernorStored,
    GraphInk, GraphMeta, GraphRow, GraphRunSheet, GraphTask, Hunk, HunkKind, ImpactRow,
    KeymapGroup, Model, ModelItem, PlanStep, ProjectTrustSheet, Proposal, ProviderRow, RecallHit,
    RecallMeta, ResumeRow, ReviewFile, ReviewSheet, SecurityScan, SessionItem, SettingRow,
    Severity, Startup, Step, ThinkingRow, Tone, TreeNode, TreeRow, TrustFile, VectorIndex,
    WorkshopSheet,
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
        found: Vec::new(),
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
        Entry::failure("error[E0308]", "mismatched types · store.rs:118"),
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
        Entry::failure("1 failed", "store::roundtrip_windows_paths"),
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
        SessionItem::new("review-agent-runtime", "3m").facts(
            "42",
            "128k",
            "forked from provider-parity",
        ),
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
        TreeRow::new(1, Some("▾"), "davinci-session", Some(State::Delta)).current(),
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

/// `2a` — the graph drawing, on a strict column grid. The connector columns
/// are literal so the grid can be verified by eye against the mockup.
pub fn graph() -> Vec<GraphRow> {
    use GraphInk::{Connector, Current, Name};
    vec![
        GraphRow(vec![
            ("davinci-cli ".into(), Name),
            ("──┬── ".into(), Connector),
            ("davinci-agent ".into(), Name),
            ("──┬── ".into(), Connector),
            ("davinci-ai ".into(), Name),
            ("─── ".into(), Connector),
            ("providers".into(), Name),
        ]),
        GraphRow(vec![
            ("            ".into(), Name),
            ("  │".into(), Connector),
            ("                   ".into(), Name),
            ("├── ".into(), Connector),
            ("davinci-tools".into(), Name),
        ]),
        GraphRow(vec![
            ("            ".into(), Name),
            ("  │".into(), Connector),
            ("                   ".into(), Name),
            ("└── ".into(), Connector),
            ("davinci-session ◉".into(), Current),
            (" ── ".into(), Connector),
            ("davinci-store".into(), Name),
        ]),
        GraphRow(vec![
            ("            ".into(), Name),
            ("  └── ".into(), Connector),
            ("davinci-tui".into(), Name),
        ]),
    ]
}

pub fn graph_meta() -> GraphMeta {
    GraphMeta {
        nodes: "412".into(),
        edges: "1207".into(),
        cycles: "0".into(),
        subject: "store.rs".into(),
        fan_in: "6".into(),
        fan_out: "2".into(),
        depth: "3".into(),
        tests: "14".into(),
        untested: "2".into(),
        freshness: "graph rebuilt 4s ago from rust-analyzer".into(),
    }
}

/// `2a` — the impact list.
pub fn impact() -> Vec<ImpactRow> {
    vec![
        ImpactRow::new(
            State::Active,
            "davinci-session::store",
            "direct",
            "6 call sites",
            false,
        ),
        ImpactRow::new(
            State::Read,
            "davinci-session::manager",
            "1 hop",
            "2 call sites",
            false,
        ),
        ImpactRow::new(
            State::Read,
            "davinci-agent::runtime",
            "2 hops",
            "1 call site",
            false,
        ),
        ImpactRow::new(
            State::Read,
            "davinci-tui::app",
            "3 hops",
            "render only",
            false,
        ),
        ImpactRow::new(
            State::Attention,
            "davinci-cli::main",
            "3 hops",
            "no test coverage",
            true,
        ),
    ]
}

/// `2b` — vector recall.
pub fn recall() -> Vec<RecallHit> {
    vec![
        RecallHit::new(
            0.91,
            "store::roundtrip: write after every tool call",
            "davinci-session\\src\\store.rs:118",
            "turn 12 · promoted",
            true,
        ),
        RecallHit::new(
            0.87,
            "manager::resume rebuilds the transcript",
            "davinci-session\\src\\manager.rs:44",
            "turn 12 · promoted",
            true,
        ),
        RecallHit::new(
            0.74,
            "notes: \"interrupts must not truncate memoria\"",
            "memoria\\decisions.md:9",
            "session · tui-redesign",
            true,
        ),
        RecallHit::new(
            0.61,
            "test roundtrip_windows_paths",
            "davinci-session\\tests\\store.rs:7",
            "below floor",
            false,
        ),
        RecallHit::new(
            0.58,
            "changelog: atomic write via tempfile + rename",
            "CHANGELOG.md:212",
            "below floor",
            false,
        ),
    ]
}

pub fn recall_meta() -> RecallMeta {
    RecallMeta {
        query: "how does session persistence survive an interrupt".into(),
        vectors: "18,402".into(),
        shards: "3".into(),
        embedding: "bge-small 384d".into(),
        metric: "cosine".into(),
        elapsed: "12ms".into(),
        k: "6".into(),
        floor: 0.70,
        promoted: "3 chunks · 1.2k tokens".into(),
        freshness: "reindexed on last commit".into(),
    }
}

/// `2c` — the budget by role.
pub fn budget() -> Vec<BudgetRow> {
    vec![
        BudgetRow::new("system", "4.2k", 4.2 / 40.0, "pinned", false),
        BudgetRow::new("codex map", "22.8k", 22.8 / 40.0, "cap 40k", false),
        BudgetRow::new("transcript", "71.6k", 1.0, "! soft cap 60k", true),
        BudgetRow::new("instrumenta", "21.4k", 21.4 / 40.0, "14 schemas", false),
        BudgetRow::new("memoria", "8.4k", 8.4 / 40.0, "3 chunks", false),
        BudgetRow::new("reserve", "10.0k", 10.0 / 40.0, "for the reply", false),
    ]
}

pub fn budget_meta() -> BudgetMeta {
    BudgetMeta {
        policy: "balanced".into(),
        in_use: "128.4k".into(),
        window: "200k".into(),
        headroom: "71.6k".into(),
        in_use_fraction: 128.4 / 200.0,
        rate: "1.9k tok/s".into(),
        session_spend: "412k".into(),
        daily_cap: "2m".into(),
        daily_fraction: 0.21,
        history: "governor acted 3× today · last: evicted 2 tool results".into(),
    }
}

/// `2c` — the standing proposal.
pub fn proposal() -> Proposal {
    Proposal {
        summary: "transcript is 19% over its soft cap. Proposed: summarise turns 1-18 into \
                  one note and evict their tool output."
            .into(),
        recovers: "18.2k".into(),
        keeps: "last 6 turns".into(),
        cost: "1 summarising call".into(),
        reversible: true,
        actions: vec![
            ("a".into(), "apply".into()),
            ("e".into(), "evict oldest 6".into()),
            ("p".into(), "policy".into()),
            ("h".into(), "hold, warn at 90%".into()),
            ("d".into(), "dismiss".into()),
        ],
    }
}

/// A model dressed with the mockups' session, for eyeballing a screen.
/// `3a` — the full model catalog.
pub fn catalog() -> Vec<CatalogRow> {
    let row = |name: &str,
               window: &str,
               thinking: &str,
               price: &str,
               credential: Credential,
               note: &str,
               ring: bool| CatalogRow {
        name: name.into(),
        detail: if name.starts_with("llama.cpp") {
            "router :8080".into()
        } else {
            String::new()
        },
        window: window.into(),
        thinking: thinking.into(),
        price: price.into(),
        credential,
        note: note.into(),
        ring,
        provider: name.split(" / ").next().unwrap_or("").into(),
        id: name.split(" / ").nth(1).unwrap_or("").into(),
    };
    vec![
        row(
            "anthropic / claude-sonnet",
            "200k",
            "budget",
            "3.00 · 15.00",
            Credential::Ready,
            "oauth",
            true,
        ),
        row(
            "anthropic / claude-opus",
            "200k",
            "budget",
            "15.00 · 75.00",
            Credential::Ready,
            "oauth",
            true,
        ),
        row(
            "anthropic / claude-haiku",
            "200k",
            "budget",
            "0.80 · 4.00",
            Credential::Ready,
            "oauth",
            false,
        ),
        row(
            "openai / gpt",
            "128k",
            "effort",
            "2.50 · 10.00",
            Credential::Ready,
            "api key",
            true,
        ),
        row(
            "openai-codex / gpt-codex",
            "272k",
            "effort",
            "plan",
            Credential::Ready,
            "oauth",
            false,
        ),
        row(
            "google / gemini",
            "1m",
            "budget",
            "1.25 · 10.00",
            Credential::Ready,
            "api key",
            false,
        ),
        row(
            "groq / llama",
            "131k",
            "none",
            "0.59 · 0.79",
            Credential::Ready,
            "api key",
            false,
        ),
        row(
            "github-copilot / gpt",
            "128k",
            "effort",
            "seat",
            Credential::Expired,
            "token expired",
            false,
        ),
        row(
            "xai / grok",
            "256k",
            "effort",
            "3.00 · 15.00",
            Credential::Absent,
            "no credential",
            false,
        ),
        row(
            "deepseek / chat",
            "64k",
            "budget",
            "0.28 · 0.42",
            Credential::Absent,
            "no credential",
            false,
        ),
        row(
            "zai / glm",
            "200k",
            "budget",
            "0.60 · 2.20",
            Credential::Absent,
            "no credential",
            false,
        ),
        row(
            "llama.cpp / qwen-coder",
            "32k",
            "none",
            "local",
            Credential::Ready,
            "running",
            false,
        ),
    ]
}

/// `3b` — the settings sheet.
pub fn settings_rows() -> Vec<SettingRow> {
    let row =
        |label: &str, value: &str, project: bool, values: &[&str], description: &str| SettingRow {
            label: label.into(),
            value: value.into(),
            project,
            values: values.iter().map(|value| value.to_string()).collect(),
            description: description.into(),
            key: label.to_lowercase().replace(' ', "-"),
        };
    vec![
        row("Auto-compact threshold", "default", false, &["default", "90%", "75%", "50%", "25%"],
            "When auto-compaction triggers: a context percentage or an absolute token count. default is 92% of the model window."),
        row("Auto-compact", "on", false, &["on", "off"],
            "Compact the context automatically before it overflows."),
        row("Steering mode", "one-at-a-time", false, &["one-at-a-time", "all"],
            "Enter while streaming queues a steering message. one-at-a-time delivers one and waits for the reply."),
        row("Follow-up mode", "one-at-a-time", false, &["one-at-a-time", "all"],
            "Queue follow-up messages until the agent stops."),
        row("Transport", "websocket-cached", true, &["sse", "websocket", "websocket-cached", "auto"],
            "Preferred transport for providers that support more than one. Set by this project, overriding your user setting."),
        row("HTTP idle timeout", "2 min", false, &["30 sec", "1 min", "2 min", "5 min", "disabled"],
            "Longest idle gap while waiting for headers or body chunks. Disable it for local models that pause longer than five minutes."),
        row("Mermaid diagrams", "final", false, &["off", "final", "streaming"],
            "Render mermaid code blocks as unicode diagrams."),
        row("Hide thinking", "off", false, &["on", "off"],
            "Hide thinking blocks in assistant replies."),
        row("Cache miss notices", "on", false, &["on", "off"],
            "Show a transcript notice for a significant prompt-cache miss and for what a compaction cost."),
        row("Autocomplete max items", "7", false, &["3", "5", "7", "10", "15", "20"],
            "How many rows the composer's completion list may show."),
        row("Skill commands", "on", false, &["on", "off"],
            "Register every discovered skill as a /skill:name command."),
        row("Quiet startup", "on", false, &["on", "off"],
            "Skip the verbose banner when a session opens."),
    ]
}

/// `3c` — the thinking sheet.
pub fn thinking_rows() -> Vec<ThinkingRow> {
    let row = |level: &str, budget: &str, fraction: f64, maps_to: &str, warn: bool| ThinkingRow {
        level: level.into(),
        budget: budget.into(),
        fraction,
        maps_to: maps_to.into(),
        warn,
    };
    vec![
        row("off", "0", 0.0, "disabled → none", false),
        row("minimal", "1.0k", 0.016, "1024 → minimal", false),
        row("low", "4.0k", 0.063, "4096 → low", false),
        row("medium", "8.0k", 0.125, "8192 → medium", false),
        row("high", "16.0k", 0.25, "16384 → high", false),
        row("xhigh", "32.0k", 0.5, "32768 → high", false),
        row("max", "64.0k", 1.0, "! 32% of the window", true),
    ]
}

/// `3d` — provider credentials, and the grant in flight.
pub fn providers() -> Vec<ProviderRow> {
    let row = |name: &str, method: &str, source: &str, state: Credential| ProviderRow {
        name: name.into(),
        method: method.into(),
        source: source.into(),
        state,
    };
    vec![
        row(
            "anthropic",
            "oauth",
            "device flow, in progress",
            Credential::Pending,
        ),
        row("openai", "api key", "env OPENAI_API_KEY", Credential::Ready),
        row(
            "openai-codex",
            "oauth",
            "auth.json · refreshes in 22h",
            Credential::Ready,
        ),
        row("google", "api key", "auth.json", Credential::Ready),
        row(
            "github-copilot",
            "oauth",
            "refresh rejected 401 · 2d ago",
            Credential::Expired,
        ),
        row(
            "groq",
            "api key",
            "env GROQ_API_KEY, unset",
            Credential::Absent,
        ),
        row(
            "xai · deepseek · zai",
            "api key",
            "never configured",
            Credential::Absent,
        ),
        row(
            "llama.cpp",
            "local",
            "router at 127.0.0.1:8080",
            Credential::Local,
        ),
    ]
}

pub fn device_code() -> DeviceCode {
    DeviceCode {
        code: "WQPT-FJ4M".into(),
        url: "https://claude.ai/oauth/device".into(),
        expires: "8m 41s".into(),
        polls: 6,
    }
}

/// `3e` — the keymap.
pub fn keymap() -> Vec<KeymapGroup> {
    let group = |title: &str, note: &str, rows: &[(&str, &str)]| KeymapGroup {
        title: title.into(),
        note: note.into(),
        rows: rows
            .iter()
            .map(|(key, what)| (key.to_string(), what.to_string()))
            .collect(),
    };
    vec![
        group(
            "INSTRUMENTS",
            "over the transcript",
            &[
                ("ctrl+p", "instrumenta · palette"),
                ("ctrl+s", "memoria · sessions"),
                ("ctrl+r", "memoria · vector recall"),
                ("ctrl+o", "cogitator · model"),
                ("ctrl+l", "disegno · plan"),
                ("ctrl+g", "grafo · graph"),
                ("ctrl+u", "mensura · governor"),
                ("ctrl+e", "codex · workspace"),
                ("esc", "close whichever is open"),
            ],
        ),
        group(
            "RUN",
            "while the agent works",
            &[
                ("ctrl+c", "interrupt the run · never the app"),
                ("ctrl+d", "quit"),
                ("ctrl+z", "suspend to the shell"),
                ("shift+tab", "cycle thinking level"),
                ("ctrl+t", "thinking on / off"),
            ],
        ),
        group(
            "COMPOSER",
            "",
            &[
                ("enter", "send"),
                ("shift+enter", "newline · also ctrl+j"),
                ("alt+enter", "queue as follow-up"),
                ("alt+up", "take the last queued back"),
                ("tab", "complete command, file, skill"),
                ("ctrl+v", "paste image from clipboard"),
                ("ctrl+g", "open $EDITOR on the draft"),
                ("ctrl+x", "copy last agent message"),
            ],
        ),
        group(
            "SESSION LIST",
            "inside memoria",
            &[
                ("ctrl+p", "show full paths"),
                ("ctrl+s", "sort recent / name"),
                ("ctrl+r", "rename"),
                ("ctrl+n", "named sessions only"),
                ("ctrl+d", "delete · confirms first"),
            ],
        ),
        group(
            "SESSION TREE",
            "",
            &[
                ("ctrl+← ctrl+→", "fold / unfold a branch"),
                ("shift+l", "label this turn"),
                (
                    "ctrl+d t u l a",
                    "filter: default, no tools, user, labeled, all",
                ),
            ],
        ),
    ]
}

/// `4a` — the resume list.
pub const SESSION_COUNT: usize = 34;

pub fn resume_rows() -> Vec<ResumeRow> {
    let row = |name: &str,
               branch: &str,
               turns: &str,
               tokens: &str,
               model: &str,
               touched: &str,
               named: bool,
               warning: Option<&str>,
               note: &str,
               last: &str,
               path: &str| ResumeRow {
        name: name.into(),
        branch: branch.into(),
        turns: turns.into(),
        tokens: tokens.into(),
        model: model.into(),
        touched: touched.into(),
        named,
        warning: warning.map(str::to_string),
        note: note.into(),
        last: last.into(),
        path: path.into(),
    };
    vec![
        row(
            "review-agent-runtime",
            "main",
            "42",
            "128k",
            "sonnet",
            "3m",
            true,
            None,
            "forked from provider-parity at turn 12 · Δ7 files · 3 branches",
            "now fix the store.rs type error",
            "~\\.pi\\agent\\sessions\\--dev--oss--davinci-rust\\01JB2K….jsonl",
        ),
        row(
            "implement-rpc-mode",
            "main",
            "61",
            "184k",
            "sonnet",
            "18m",
            true,
            None,
            "compacted twice · 2 forks",
            "add the rpc handshake test",
            "~\\.pi\\agent\\sessions\\--dev--oss--davinci-rust\\01JAX7….jsonl",
        ),
        row(
            "provider-parity",
            "main",
            "28",
            "96k",
            "opus",
            "1h",
            true,
            None,
            "parent of review-agent-runtime",
            "compare the streaming shapes",
            "~\\.pi\\agent\\sessions\\--dev--oss--davinci-rust\\01JAW1….jsonl",
        ),
        row(
            "tui-redesign",
            "davinci",
            "117",
            "412k",
            "sonnet",
            "yest.",
            true,
            None,
            "the longest session in this project",
            "draw 1h in NO_COLOR",
            "~\\.pi\\agent\\sessions\\--dev--oss--davinci-rust\\01J9Q4….jsonl",
        ),
        row(
            "01J8ZK…7QW4",
            "main",
            "4",
            "11k",
            "haiku",
            "2d",
            false,
            None,
            "never named · four turns",
            "what does pi-parity do",
            "~\\.pi\\agent\\sessions\\--dev--oss--davinci-rust\\01J8ZK….jsonl",
        ),
        row(
            "fix-git-hooks",
            "hooks",
            "33",
            "88k",
            "gpt",
            "2d",
            true,
            Some("! branch hooks no longer exists · resuming replays against main"),
            "! branch hooks no longer exists · resuming replays against main",
            "the pre-commit hook eats the exit code",
            "~\\.pi\\agent\\sessions\\--dev--oss--davinci-rust\\01J8PD….jsonl",
        ),
    ]
}

/// `4b` — the session tree.
pub fn session_tree() -> Vec<TreeNode> {
    let node = |trunk: &str,
                state: Option<State>,
                id: Option<&str>,
                label: Option<&str>,
                meta: Option<&str>| TreeNode {
        trunk: trunk.into(),
        state,
        id: id.map(str::to_string),
        label: label.map(str::to_string),
        meta: meta.map(str::to_string),
        entry_id: id.unwrap_or_default().into(),
    };
    vec![
        node(
            "",
            Some(State::Queued),
            Some("01"),
            Some("explain how the agent runtime works"),
            Some("12:04"),
        ),
        node("│", None, None, None, None),
        node(
            "├── ",
            Some(State::Done),
            Some("02"),
            Some("surveyed the workspace"),
            Some("12:05"),
        ),
        node("│   │", None, None, None, None),
        node(
            "│   └── ",
            Some(State::Failed),
            Some("03"),
            Some("store as a trait · abandoned"),
            Some("12:09"),
        ),
        node("│", None, None, None, None),
        node(
            "└── ",
            Some(State::Done),
            Some("04"),
            Some("traced the request pipeline"),
            Some("12:11"),
        ),
        node("    │", None, None, None, None),
        node(
            "    ├── ",
            Some(State::Active),
            Some("05"),
            Some("fix the store.rs type error"),
            Some("12:18"),
        ),
        node("    │", None, None, None, None),
        node(
            "    └── ",
            Some(State::Queued),
            Some("06"),
            Some("fork · streaming rewrite"),
            Some("12:22"),
        ),
    ]
}

/// `4c` — the compaction sheet.
pub fn compaction() -> Compaction {
    Compaction {
        before_tokens: "184.2k".into(),
        before_fraction: 0.92,
        before_note: "! 92% of 200k".into(),
        after_tokens: "61.8k".into(),
        after_fraction: 0.31,
        after_note: "31% of 200k".into(),
        kept: vec![
            "the last 6 turns, whole".into(),
            "every Δ and its hunks · 7 files".into(),
            "the disegno plan, steps I–V".into(),
            "your instruction: store.rs decisions".into(),
            "AGENTS.md and CLAUDE.md · re-read, not summarised".into(),
        ],
        folded: vec![
            "turns 1–18 · 96.4k".into(),
            "31 tool results · kept as ids, retrievable".into(),
            "9 superseded reads of the same file".into(),
            "2 memoria injections, now stale".into(),
        ],
        recovers: "122.4k".into(),
        call_cost: "$0.19".into(),
        cache_cost: "$0.23".into(),
    }
}

/// `4d` — the export ledger.
pub fn export_ledger() -> ExportLedger {
    ExportLedger {
        included: vec![
            "42 turns of prose and thinking".into(),
            "31 tool calls with their output".into(),
            "every Δ hunk, syntax coloured".into(),
            "4 pasted images, inlined as base64".into(),
        ],
        excluded: vec![
            (
                State::Failed,
                "api keys and bearer tokens · redacted".into(),
            ),
            (
                State::Failed,
                "the contents of .env · 2 reads masked".into(),
            ),
            (
                State::Attention,
                "absolute paths · kept, they name your machine".into(),
            ),
            (
                State::Attention,
                "branch names and commit subjects · kept".into(),
            ),
        ],
        size: "2.9 MB".into(),
        elapsed: "1.4s".into(),
        gist: "https://gist.github.com/9f21c4…a70".into(),
    }
}

/// `5a` — the graph run.
pub fn graph_run_sheet() -> GraphRunSheet {
    let task = |id: &str, policy: &str, artifact: &str, usage: &str, state: State| GraphTask {
        id: id.into(),
        policy: policy.into(),
        artifact: artifact.into(),
        usage: usage.into(),
        state,
    };
    GraphRunSheet {
        goal: "add prompt-cache parity to the openai adapter --complex".into(),
        phases: vec![
            ("classify".into(), State::Done),
            ("investigate".into(), State::Done),
            ("plan".into(), State::Done),
            ("implement".into(), State::Active),
            ("verify".into(), State::Queued),
            ("review".into(), State::Queued),
            ("done".into(), State::Queued),
        ],
        shape: vec![
            "t1 classifier ─┬─ t2 researcher    ─┐".into(),
            "               ├─ t3 test-analyzer ─┼─ t5 planner".into(),
            "               └─ t4 historian     ─┘      │".into(),
            "                                           └─ t6 writer ◉ ─ t7 reviewer".into(),
        ],
        tasks: vec![
            task(
                "t1 classifier",
                "read-only",
                "feature · complex",
                "2.1k↑ 0.4k↓ $0.01 4s",
                State::Done,
            ),
            task(
                "t2 researcher",
                "read-only",
                "evidence · 14 call sites",
                "31k↑ 3.2k↓ $0.14 48s",
                State::Done,
            ),
            task(
                "t3 test-analyzer",
                "read-and-test",
                "baseline · 212 pass",
                "18k↑ 2.0k↓ $0.09 1m52s",
                State::Done,
            ),
            task(
                "t4 historian",
                "read-only",
                "evidence · 3 attempts",
                "9.4k↑ 1.1k↓ $0.05 22s",
                State::Done,
            ),
            task(
                "t5 planner",
                "read-only",
                "plan · 4 milestones",
                "42k↑ 5.8k↓ $0.31 1m09s",
                State::Done,
            ),
            task(
                "t6 writer",
                "write-no-git-mutation",
                "davinci-ai\\src\\openai.rs",
                "64k↑ 9.7k↓ $0.71 2m14s",
                State::Active,
            ),
            task(
                "t7 reviewer",
                "read-and-test",
                "pending · waits on t6",
                "—",
                State::Queued,
            ),
        ],
        cost: "$1.31".into(),
        cost_cap: "$8.00".into(),
        cost_fraction: 0.16,
        workers: "6 of 12".into(),
        parallel: "3".into(),
        cycles: "0 of 2".into(),
        replans: "0 of 1".into(),
        artifacts: ".pi\\graph\\g-7f2a\\".into(),
    }
}

/// `5b` — the vector index.
pub fn vector_index() -> VectorIndex {
    VectorIndex {
        repo: "davinci-rust".into(),
        repo_records: "6,914".into(),
        total_records: "18,402".into(),
        injection_cap: "1.5k tokens".into(),
        floor: "0.70".into(),
        kinds: vec![
            (
                "decision".into(),
                "1,482".into(),
                0.48,
                "importance 0.9".into(),
            ),
            (
                "architecture".into(),
                "906".into(),
                0.30,
                "importance 0.9".into(),
            ),
            (
                "discovery".into(),
                "1,105".into(),
                0.36,
                "importance 0.7".into(),
            ),
            (
                "bug · fix".into(),
                "842".into(),
                0.27,
                "importance 0.8".into(),
            ),
            (
                "constraint".into(),
                "311".into(),
                0.10,
                "never evicted".into(),
            ),
            (
                "task result".into(),
                "1,674".into(),
                0.54,
                "importance 0.6".into(),
            ),
            (
                "compaction".into(),
                "64".into(),
                0.02,
                "one per compaction".into(),
            ),
            (
                "conversation".into(),
                "530".into(),
                0.17,
                "first to go".into(),
            ),
        ],
        embeddings: "ollama".into(),
        embed_host: "127.0.0.1:11434 · bge-small 384d".into(),
        store: "qdrant".into(),
        collection: "collection davinci-memoria · 3 shards".into(),
        extraction: "haiku".into(),
        config: "%USERPROFILE%\\.pi\\vector-memory.json".into(),
        health: vec![
            (State::Done, "reindexed on the last commit · 4m ago".into()),
            (State::Done, "embed 11ms p50 · 34ms p95".into()),
            (
                State::Attention,
                "7 records failed to embed · retried next reindex".into(),
            ),
            (State::Done, "no duplicate content hashes".into()),
        ],
    }
}

/// `5c` — the governor's ledger.
pub fn governor_sheet() -> GovernorSheet {
    let counter = |number: &str, of: &str, verb: &str, note: &str, tone: Tone| GovernorCounter {
        number: number.into(),
        of: of.into(),
        verb: verb.into(),
        note: note.into(),
        tone,
    };
    let stored = |id: &str, tool: &str, call: &str, size: &str, stale: bool| GovernorStored {
        id: id.into(),
        tool: tool.into(),
        call: call.into(),
        size: size.into(),
        stale,
    };
    GovernorSheet {
        counters: vec![
            counter(
                "31",
                "of 96 results",
                "compressed",
                "head 40 · tail 40 · rest on disk",
                Tone::Primary,
            ),
            counter(
                "9",
                "of 61 reads",
                "deduplicated",
                "same file, same state hash",
                Tone::Secondary,
            ),
            counter(
                "4",
                "of 96 calls",
                "blocked",
                "anti-loop · no new state",
                Tone::Warning,
            ),
            counter(
                "96.2k",
                "of 200k",
                "tokens never sent",
                "about $0.29 at sonnet input",
                Tone::Success,
            ),
        ],
        stored: vec![
            stored(
                "out-9f21c4",
                "bash",
                "cargo test --workspace",
                "1,184 ln · 84 KB",
                false,
            ),
            stored(
                "out-4c07ab",
                "grep",
                "\"SessionManager\" across crates",
                "612 ln · 31 KB",
                false,
            ),
            stored(
                "out-1e88d0",
                "read",
                "davinci-ai\\src\\openai.rs",
                "2,041 ln · 96 KB",
                false,
            ),
            stored(
                "out-77b3e5",
                "powershell",
                "git log --stat -n 40",
                "498 ln · 22 KB",
                true,
            ),
        ],
        store_dir: "%USERPROFILE%\\.pi\\outputs\\01JB2K\\ · dropped when the session ends".into(),
    }
}

/// `5d` — the security scan.
pub fn security_scan() -> SecurityScan {
    let finding = |message: &str,
                   location: &str,
                   severity: Severity,
                   rule: &str,
                   evidence: &str,
                   path: &str| Finding {
        message: message.into(),
        location: location.into(),
        severity,
        rule: rule.into(),
        evidence: evidence.into(),
        path: path.into(),
    };
    SecurityScan {
        validated: 31,
        candidates: 44,
        fraction: 0.7,
        files: "1,842".into(),
        skipped: "96".into(),
        bytes: "41.2 MB".into(),
        severities: vec![
            ("critical".into(), 1, Severity::Critical),
            ("high".into(), 3, Severity::High),
            ("medium".into(), 6, Severity::Medium),
            ("low".into(), 9, Severity::Low),
            ("informational".into(), 14, Severity::Dismissed),
        ],
        dismissed: 11,
        findings: vec![
            finding(
                "bearer token written to the transcript",
                "davinci-ai\\src\\auth.rs:214",
                Severity::Critical,
                "secret-in-log",
                "tracing::debug!(\"refresh {token}\")",
                "refresh_token() → subscriber → session jsonl → /export, /share",
            ),
            finding(
                "command built from an unquoted path",
                "davinci-agent\\src\\tools\\bash.rs:88",
                Severity::High,
                "shell-injection",
                "format!(\"cd {} && {}\", dir, cmd)",
                "bash tool → cmd.exe → any path with a space or an ampersand",
            ),
            finding(
                "extension host inherits your environment",
                "davinci-cli\\src\\js_host.rs:141",
                Severity::High,
                "env-leak",
                "Command::new(node).envs(env::vars())",
                "project extension → node subprocess → every API key you hold",
            ),
            finding(
                "session files written 0644 on unix",
                "davinci-session\\src\\store.rs:118",
                Severity::High,
                "file-mode",
                "OpenOptions::new().create(true)",
                "session jsonl → any local account on a shared machine",
            ),
            finding(
                "http client accepts any tls version",
                "davinci-ai\\src\\http.rs:57",
                Severity::Medium,
                "weak-tls",
                "danger_accept_invalid_certs(false) only",
                "provider request → downgraded transport on a hostile network",
            ),
            finding(
                "hard-coded test key in a fixture",
                "tests\\fixtures\\auth.json:3",
                Severity::Dismissed,
                "secret-literal",
                "\"api_key\": \"sk-test-0000\"",
                "not a real credential · never read outside the test",
            ),
        ],
        seal: "4b1f…c9e0".into(),
        report: ".pi\\security\\s-31c8\\report.json · 214 KB".into(),
    }
}

/// `6a` — project trust.
pub fn project_trust() -> ProjectTrustSheet {
    let file = |state: State, path: &str, detail: &str, risk_label: &str| TrustFile {
        state,
        path: path.into(),
        detail: detail.into(),
        risk_label: risk_label.into(),
    };
    ProjectTrustSheet {
        files: vec![
            file(
                State::Attention,
                ".pi\\extensions\\lint.js",
                "runs as node, no sandbox",
                "executes code",
            ),
            file(
                State::Attention,
                ".pi\\extensions\\deploy.js",
                "registers 3 tools, 1 pre-tool hook",
                "executes code",
            ),
            file(
                State::Attention,
                ".pi\\settings.json",
                "3 keys, incl. transport and tool allowlist",
                "changes limits",
            ),
            file(
                State::Read,
                ".pi\\skills\\ (6)",
                "instructions loaded on demand",
                "prompt text",
            ),
            file(
                State::Read,
                ".pi\\prompts\\ (3)",
                "slash commands that expand to prompts",
                "prompt text",
            ),
            file(
                State::Read,
                "AGENTS.md · CLAUDE.md",
                "1,208 lines, prepended to every turn",
                "prompt text",
            ),
            file(
                State::Queued,
                ".pi\\themes\\ (1)",
                "colours only",
                "harmless",
            ),
        ],
        path: "C:\\dev\\clones\\vendor-cli".into(),
        trusted: "14 projects".into(),
        ignored: "2".into(),
        store: "%USERPROFILE%\\.pi\\trust.json".into(),
    }
}

/// `6b` — the workshop.
pub fn workshop() -> WorkshopSheet {
    WorkshopSheet {
        reload: vec![
            (State::Done, "keybindings · 39 bindings, 2 yours".into(), "3ms".into(), None),
            (State::Done, "skills · 6 found, none loaded until named".into(), "11ms".into(), None),
            (State::Done, "context files · AGENTS.md, CLAUDE.md · 4.1k".into(), "6ms".into(), None),
            (State::Failed, "extensions · deploy.js failed to register".into(), "318ms".into(),
             Some("TypeError: hooks.preTool is not a function · deploy.js:41 · its 3 tools are missing".into())),
        ],
        native: vec![
            (State::Done, "vector-memory".into(), "4 tools · 4 commands".into()),
            (State::Done, "token-governor".into(), "2 tools · 2 commands".into()),
            (State::Done, "graph".into(), "1 tool · 5 commands".into()),
            (State::Done, "security-scan".into(), "7 tools · 3 commands".into()),
        ],
        javascript: vec![
            (State::Done, "plan-mode".into(), "1 tool · registers --plan".into()),
            (State::Done, "lint.js · project".into(), "1 post-tool hook".into()),
            (State::Failed, "deploy.js · project".into(), "failed to register".into()),
        ],
        node: "node v24.19.0".into(),
        schema: "21.4k · 11%".into(),
        tools: vec![
            ("built-in tools".into(), "8".into(), 0.33, "read write edit grep find ls bash pwsh".into()),
            ("native tools".into(), "14".into(), 0.58, "memory, governor, graph, sec".into()),
            ("extension tools".into(), "2".into(), 0.08, "3 more if deploy.js is fixed".into()),
        ],
    }
}

/// `6c` — the interrupt aftermath.
pub fn failed_run() -> FailedRun {
    FailedRun {
        prompt: "rewrite the provider adapter to stream".into(),
        tools: vec![
            (
                State::Read,
                "read davinci-ai\\src\\openai.rs".into(),
                "2,041 lines".into(),
            ),
            (
                State::Done,
                "cargo check -p davinci-ai".into(),
                "1.84s · manus".into(),
            ),
            (
                State::Failed,
                "stream · 429 rate limited after 1,204 tokens".into(),
                "0.9s".into(),
            ),
        ],
        kept: "1,204 tokens".into(),
        billed: "$0.04".into(),
        aftermath: vec![
            (
                State::Done,
                "transcript written to the session file · nothing to recover on restart".into(),
            ),
            (
                State::Done,
                "the running cargo check was killed with its process group".into(),
            ),
            (
                State::Attention,
                "edit to openai.rs had not started — the file on disk is untouched".into(),
            ),
            (
                State::Skipped,
                "a second ctrl+c within a second clears the composer; ctrl+d quits".into(),
            ),
        ],
    }
}

/// `6d` — the Δ review.
pub fn review() -> ReviewSheet {
    let hunk = |kind: HunkKind, text: &str| Hunk::new(kind, text);
    let file = |state: State,
                path: &str,
                adds: Option<u32>,
                dels: Option<u32>,
                tests: &str,
                test_state: State,
                hunk_note: &str,
                hunks: Vec<Hunk>| ReviewFile {
        state,
        path: path.into(),
        adds,
        dels,
        tests: tests.into(),
        test_state,
        hunk_note: hunk_note.into(),
        hunk: hunks,
    };
    ReviewSheet {
        files: vec![
            file(
                State::Delta,
                "crates\\davinci-ai\\src\\openai.rs",
                Some(64),
                Some(19),
                "✓ 14 tests pass",
                State::Done,
                "hunk 2 of 5",
                vec![
                    hunk(
                        HunkKind::Context,
                        "pub async fn complete(&self, req: Request) -> Result<Reply> {",
                    ),
                    hunk(HunkKind::Del, "    let body = self.post(req).await?;"),
                    hunk(HunkKind::Del, "    Ok(Reply::from(body))"),
                    hunk(
                        HunkKind::Add,
                        "    let mut stream = self.post_stream(req).await?;",
                    ),
                    hunk(HunkKind::Add, "    let mut reply = Reply::default();"),
                    hunk(
                        HunkKind::Add,
                        "    while let Some(chunk) = stream.next().await {",
                    ),
                    hunk(HunkKind::Add, "        reply.push(chunk?);"),
                    hunk(HunkKind::Add, "    }"),
                    hunk(HunkKind::Add, "    Ok(reply)"),
                    hunk(HunkKind::Context, "}"),
                ],
            ),
            file(
                State::Delta,
                "crates\\davinci-ai\\src\\stream.rs",
                Some(38),
                Some(11),
                "✓ 6 tests pass",
                State::Done,
                "hunk 1 of 3",
                vec![
                    hunk(
                        HunkKind::Context,
                        "fn parse_event(line: &str) -> Option<Chunk> {",
                    ),
                    hunk(HunkKind::Del, "    serde_json::from_str(line).ok()"),
                    hunk(
                        HunkKind::Add,
                        "    let payload = line.strip_prefix(\"data: \")?;",
                    ),
                    hunk(
                        HunkKind::Add,
                        "    if payload == \"[DONE]\" { return None; }",
                    ),
                    hunk(HunkKind::Add, "    serde_json::from_str(payload).ok()"),
                    hunk(HunkKind::Context, "}"),
                ],
            ),
            file(
                State::Delta,
                "crates\\davinci-agent\\src\\runtime.rs",
                Some(21),
                Some(6),
                "! untested path",
                State::Attention,
                "hunk 1 of 2",
                vec![
                    hunk(HunkKind::Context, "match provider.complete(req).await {"),
                    hunk(HunkKind::Del, "    Ok(reply) => self.record(reply),"),
                    hunk(HunkKind::Add, "    Ok(reply) => {"),
                    hunk(HunkKind::Add, "        self.record(reply.clone());"),
                    hunk(HunkKind::Add, "        self.session.write(&reply)?;"),
                    hunk(HunkKind::Add, "    }"),
                    hunk(HunkKind::Context, "    Err(err) => self.fail(err),"),
                ],
            ),
            file(
                State::Done,
                "crates\\davinci-ai\\tests\\stream.rs · new",
                Some(17),
                None,
                "✓ 4 tests pass",
                State::Done,
                "the whole file",
                vec![
                    hunk(HunkKind::Add, "#[test]"),
                    hunk(HunkKind::Add, "fn done_sentinel_ends_the_stream() {"),
                    hunk(
                        HunkKind::Add,
                        "    let chunks = collect(\"data: [DONE]\\n\");",
                    ),
                    hunk(HunkKind::Add, "    assert!(chunks.is_empty());"),
                    hunk(HunkKind::Add, "}"),
                ],
            ),
            file(
                State::Delta,
                "Cargo.toml",
                Some(2),
                Some(2),
                "pinned",
                State::Queued,
                "hunk 1 of 1",
                vec![
                    hunk(HunkKind::Del, "eventsource-stream = \"0.2\""),
                    hunk(HunkKind::Del, "futures = \"0.3\""),
                    hunk(HunkKind::Add, "eventsource-stream = \"=0.2.3\""),
                    hunk(HunkKind::Add, "futures = \"=0.3.31\""),
                ],
            ),
            file(
                State::Failed,
                "crates\\davinci-ai\\src\\legacy.rs · deleted",
                None,
                Some(88),
                "! 2 references left",
                State::Attention,
                "88 lines removed",
                vec![
                    hunk(HunkKind::Del, "pub struct LegacyProvider {"),
                    hunk(HunkKind::Del, "    client: Client,"),
                    hunk(HunkKind::Del, "}"),
                    hunk(HunkKind::Context, "… 85 more removed lines"),
                ],
            ),
            file(
                State::Delta,
                "CHANGELOG.md",
                Some(3),
                Some(1),
                "one entry",
                State::Queued,
                "hunk 1 of 1",
                vec![
                    hunk(HunkKind::Context, "## Unreleased"),
                    hunk(HunkKind::Del, "- nothing yet"),
                    hunk(HunkKind::Add, "### Added"),
                    hunk(HunkKind::Add, "- streaming for the openai adapter"),
                    hunk(HunkKind::Add, "- prompt-cache parity with the TS client"),
                ],
            ),
        ],
        adds: 145,
        dels: 127,
        branch: "main".into(),
        behind: "3 commits behind".into(),
        warning: "legacy.rs is gone but 2 files still name it · the build will fail".into(),
        tests: "212 of 212 tests pass on the changed crates · 41.2s".into(),
    }
}

/// A turn twelve seconds in, for the screens that show one running.
fn working() -> crate::davinci::model::Working {
    crate::davinci::model::Working {
        seconds: 12,
        tokens: 423,
        thinking: Some("high".into()),
    }
}

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
    model.graph = graph();
    model.graph_meta = graph_meta();
    model.impact = impact();
    model.recall = recall();
    model.recall_meta = recall_meta();
    model.budget = budget();
    model.budget_meta = budget_meta();
    model.proposal = Some(proposal());
    model.catalog = catalog();
    model.settings_rows = settings_rows();
    model.thinking_rows = thinking_rows();
    model.thinking_index = 4;
    model.providers = providers();
    model.keymap = keymap();
    model.resume_sessions = resume_rows();
    model.session_count = SESSION_COUNT;
    model.session_tree = session_tree();
    model.tree_index = 8;
    model.compaction = Some(compaction());
    model.export_ledger = Some(export_ledger());
    model.graph_run = Some(graph_run_sheet());
    model.vector_index = Some(vector_index());
    model.governor = Some(governor_sheet());
    model.security = Some(security_scan());
    model.project_trust = Some(project_trust());
    model.workshop = Some(workshop());
    model.failed_run = Some(failed_run());
    model.review = Some(review());
    model.transcript = if model.narrow() {
        narrow_transcript()
    } else {
        transcript()
    };
}

/// Dress the model as one named mockup screen, so it can be matched against
/// `docs/ui/Pi TUI Mockups.dc.html` in a real terminal. Each arm reproduces
/// that screen's own session facts — its context, its Δ tally, what sits in
/// the composer — not just its panel.
pub fn dress_screen(model: &mut Model, id: &str) {
    use crate::davinci::model::{Overlay, Screen};

    dress(model);
    match id {
        "1a" => {
            model.transcript.clear();
            model.changes = (0, 0, 0);
            model.context = (0, 200_000);
        }
        "1b" => {
            model.composer = "now fix the store.rs type error".into();
            model.working = Some(working());
        }
        "1d" => {
            model.toggle_overlay(Overlay::Instrumenta);
            model.query = "git".into();
        }
        "1f" => model.toggle_overlay(Overlay::Sessions),
        "1f-cogitator" => model.toggle_overlay(Overlay::Cogitator),
        "1c" => {
            model.toggle_screen(Screen::Plan);
            model.transcript = vec![
                Entry::user("add streaming to the anthropic adapter, keep TS parity"),
                Entry::Gap,
                Entry::agent("davinci"),
            ];
            model.composer = "keep step IV, but generate the fixtures from the\nexisting TS \
                              golden files under tests\\golden\\"
                .into();
            model.context = (62_000, 200_000);
        }
        "1e" => {
            model.toggle_codex();
            model.transcript = codex_transcript();
            model.changes = (3, 91, 11);
            model.context = (78_000, 200_000);
        }
        "1g" | "1h" => {
            model.transcript = narrow_transcript();
            if id == "1g" {
                model.working = Some(working());
            }
            if id == "1h" {
                model.transcript.push(Entry::Gap);
                model.transcript.push(Entry::Delta {
                    path: "davinci-session\\src\\store.rs".into(),
                    adds: 6,
                    dels: 3,
                    hunks: Vec::new(),
                });
            }
        }
        "2a" => {
            model.toggle_screen(Screen::Grafo);
            model.transcript = vec![
                Entry::user("/graph impact crates\\davinci-session\\src\\store.rs"),
                Entry::Gap,
            ];
            model.composer = "/graph path davinci-cli::main → store::write".into();
        }
        "2b" => {
            model.toggle_screen(Screen::Memoria);
            model.transcript.clear();
            model.context = (48_000, 200_000);
        }
        "2c" => {
            model.toggle_screen(Screen::Mensura);
            model.transcript.clear();
            model.context = (128_400, 200_000);
            model.composer = "/mensura policy frugal".into();
        }
        // `3a`–`6d` — the command sheets; each opens over a cleared
        // transcript with the session facts its mockup sets.
        "3a" => {
            sheet(model, Screen::Models);
            model.facts.catalog_shown = 12;
            model.facts.catalog_total = 63;
            model.facts.providers_ready = 6;
            model.facts.providers_total = 10;
            model.facts.catalog_refreshed = "2h ago".into();
            model.facts.catalog_path = "%USERPROFILE%\\.pi\\agent\\models.json".into();
        }
        "3b" => sheet(model, Screen::Settings),
        "3c" => sheet(model, Screen::Thinking),
        "3d" => {
            model.device_code = Some(device_code());
            sheet(model, Screen::Login);
        }
        "3e" => sheet(model, Screen::Keys),
        "4a" => sheet(model, Screen::Resume),
        "4b" => sheet(model, Screen::Tree),
        "4c" => {
            sheet(model, Screen::Compact);
            model.context = (184_200, 200_000);
        }
        "4d" => sheet(model, Screen::Export),
        "5a" => sheet(model, Screen::GraphRun),
        "5b" => sheet(model, Screen::Vectors),
        "5c" => sheet(model, Screen::Governor),
        "5d" => sheet(model, Screen::Securitas),
        "6a" => sheet(model, Screen::Trust),
        "6b" => sheet(model, Screen::Officina),
        "6c" => sheet(model, Screen::Recovery),
        "6d" => sheet(model, Screen::Diff),
        _ => {}
    }
}

/// Open one command sheet over a quiet transcript.
fn sheet(model: &mut Model, screen: crate::davinci::model::Screen) {
    model.transcript.clear();
    model.running = false;
    model.overlay = None;
    model.screen = screen;
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
        assert!(entries.iter().any(|e| matches!(e, Entry::Failure { .. })));
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
