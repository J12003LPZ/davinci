# Davinci Agent Harness

A terminal coding agent: a product-equivalent Rust rewrite of the TypeScript CLI [`pi`](https://github.com/earendil-works/pi) (vendor pin `853a80d26c90a14c1886f0ebb8ffaae133ca2185`).

Same flags, same `~/.davinci` (and legacy `~/.pi`) sessions, same provider credentials, same `--print` and `--mode rpc` — one static binary, no Node runtime required (Node is optional, and only for JavaScript extensions).

The interactive terminal UI opens the davinci shell described below. `--legacy-tui` keeps the original.

The TypeScript sources under `vendor/davinci` are reference-only. Do not delete them.

---

## Install

Requires Rust 1.83.0 (pinned in `rust-toolchain.toml`).

```bash
make install          # cargo install --path crates/davinci-coding-agent --force
# or
make build            # cargo build -p davinci-coding-agent  ->  ./target/debug/davinci
```

## Quick start

```bash
davinci                                  # interactive TUI
davinci "List all .ts files in src/"     # interactive, with an opening prompt
davinci -p "explain src/main.rs"         # print mode: run, print, exit
davinci --mode json -p "fix the build"   # newline-delimited JSON event stream
davinci --mode rpc                       # JSON-RPC over stdio, for embedding
davinci @notes.md @screenshot.png "what changed?"
```

Print mode is also selected automatically when stdin or stdout is not a TTY, so `davinci` composes in pipelines.

---

## Features

### Agent core

Eight built-in tools: `read`, `write`, `edit`, `bash`, `powershell`, `grep`, `find`, `ls`.

- `read` returns text or attaches images (jpg/png/gif/webp/bmp), truncating at 2000 lines / 50 KB with `offset` and `limit` for the rest.
- `edit` is exact-text replacement with a batch form; every edit is matched against the original file and must be unique and non-overlapping.
- `grep` and `find` respect `.gitignore` and prefer fast managed binaries (`rg`, `fd`) when present, with pure-Rust fallbacks when they are not.
- `powershell` is a Windows-native addition: it prefers `pwsh`, falls back to `powershell`, and forces UTF-8 console encoding.

Tool exposure is controlled per run with `--tools`, `--exclude-tools`, `--no-tools`, and `--no-builtin-tools`.

Project-local configuration is trusted explicitly. If a repository contains `.davinci/settings.json`, `.davinci/extensions`, `.davinci/skills` (or legacy `.pi/` equivalents), `SYSTEM.md`, or `APPEND_SYSTEM.md`, davinci asks before honoring it and remembers the answer in `~/.davinci/agent/trust.json` (or `~/.pi/agent/trust.json`). `--approve` / `--no-approve` override for one run; `/trust` changes the stored decision. Repositories with no such files need no decision at all.

`AGENTS.md` and `CLAUDE.md` are discovered and loaded as context (disable with `--no-context-files`).

### Sessions

Sessions are JSONL files under `~/.davinci/agent/sessions/` (or legacy `~/.pi/agent/sessions/`), grouped by a cwd-encoded directory (`--C--Users-me-project--/`) byte-compatible with TypeScript pi. Override with `--session-dir`, `DAVINCI_CODING_AGENT_SESSION_DIR`, `PI_CODING_AGENT_SESSION_DIR`, or the `sessionDir` setting.

- `--continue` resumes the latest session for this directory; `--resume` opens a searchable picker.
- `--session <path|id>` accepts a path or a partial UUID; `--session-id` creates a named id if missing.
- `--fork` branches an existing session, `/clone` copies one, and `/tree` navigates the branch tree — conversations are a tree of lanes, not a single line.
- `--no-session` runs ephemerally; `--export file.jsonl` renders a session to standalone HTML.
- An optional SQLite backend keeps a derived branch cache (root-to-leaf paths per branch) for large session stores.

### Terminal UI

`davinci` opens the **davinci shell** (`crates/davinci-tui/src/davinci/`, built on
ratatui). Its visual language is specified in `docs/ui/design.md` and drawn in
`docs/ui/Pi TUI Mockups.dc.html` across eleven screens: a truecolor
copper/verdigris palette where copper carries state and verdigris carries
location, a fixed glyph vocabulary (`✓ ◉ ○ ◌ × ! Δ ↳ ⌕ ◆`) so every state
reads under `NO_COLOR`, proportion meters instead of bare numbers, prose
wrapped at 74 columns however wide the terminal, and exactly two things that
move — a caret blink and one 4-frame spinner, both off a single 250ms clock
and both static under `--no-animation`. One instrument at a time: each is
summoned with a chord, used, and dismissed with `esc`, and states its own
exits in its footer. When the binary is installed under another name (for
example `davinci`), the startup mark and agent label follow it.

Instruments: `ctrl+p` Instrumenta (commands), `ctrl+s` Memoria (sessions),
`ctrl+r` Memoria (vector recall), `ctrl+o` Cogitator (models), `ctrl+e` Codex
(workspace, ≥120 columns), `ctrl+l` Disegno (plan), `ctrl+g` Grafo (code
graph), `ctrl+u` Mensura (token governor). `ctrl+c` interrupts the run and
never the app.

- Startup identity mark, then a transcript where user turns are `> text`,
  agent turns open with `◆ <name>`, and each tool call is one line
  (`✓ manus · cargo test  0.42s`) with failures expanding to a few lines.
- Live turns run on a worker thread: tool lines appear as they happen, a
  4-frame spinner shows the current work verb, `Esc` interrupts the run, and
  `Enter` queues follow-up prompts while the agent is busy.
- Status bar: `dir · branch · Δfiles +added -removed` on the left, model and
  a context meter (`━━━╸──── 47k/200k`) on the right.
- `/model` lists every catalog model; providers without credentials show
  dimmed with their `/login` hint instead of being hidden.
- `tab` completes slash commands and workspace paths, as far as every
  candidate agrees and no further. `shift+enter` (or `alt+enter`, or `ctrl+j`)
  adds a line.

#### `--legacy-tui`

`--legacy-tui`, or `PI_DAVINCI=0`, opens the previous custom renderer. It is
still the only mode with these:

- Markdown with syntax-aware code blocks, OSC-8 hyperlinks, LaTeX, and diff rendering.
- Mermaid diagrams rendered as terminal graphics — `off`, `final`, or `streaming` (default: draws while the answer streams).
- Inline images via the Kitty graphics protocol or iTerm inline images, tracked across scrolling.
- Themes (`dark`, `light`, `pi`, plus your own via `--theme`), fuzzy slash-command autocomplete, mouse and scrollback support, and a fullscreen alt-screen mode (`--tui-mode fullscreen`) with in-scrollback search.
- Emacs-style line editing, kill ring, undo, and `Ctrl+G` to drop into `$EDITOR`.
- Extension select / input / editor dialogs.
- Keybindings are data: defaults live in code and are overridable in `~/.pi/agent/keybindings.json`. Highlights — `Shift+Tab` cycle thinking level, `Ctrl+P` cycle model, `Ctrl+L` model picker, `Ctrl+O` expand tool output, `Ctrl+T` toggle thinking, `Ctrl+V` paste image, `Esc` interrupt.

Built-in slash commands: `/settings /model /tree /thinking /scoped-models /export /import /share /copy /name /session /changelog /hotkeys /fork /clone /trust /login /logout /new /compact /resume /reload /llama /quit`.

### Models and providers

39 providers and roughly 1,290 model entries ship compiled into the binary; `pi update` refreshes the catalogs into `~/.pi/agent`.

Wire protocols implemented natively: `anthropic-messages`, `openai-responses`, `openai-completions`, `openai-codex-responses`, `azure-openai-responses`, `google-generative-ai`, `google-vertex`, `bedrock-converse-stream` (with SigV4 signing), `mistral-conversations`.

- `--model` accepts `provider/id` patterns with an optional `:thinking` suffix; `--models` sets the `Ctrl+P` cycle ring; `--list-models` fuzzy-searches the catalog.
- `--thinking off|minimal|low|medium|high|xhigh|max` maps to each provider's own reasoning-budget field.
- `/login` supports API keys, browser OAuth with PKCE (Anthropic, OpenAI Codex, OpenRouter, xAI, GitHub Copilot, Kimi, Radius), and device-code flow. Credentials live in `~/.pi/agent/auth.json`; Vertex also reads gcloud ADC. Provider API keys can come from the usual environment variables instead.
- `pi auth print-api-key` / `print-bearer-token` expose a credential (refreshing OAuth if expired) for external clients.

### Extensions

Two tiers, both indistinguishable from built-ins at the prompt.

**JavaScript extensions** run in a Node subprocess driven by an embedded runner. They can register tools, slash commands, CLI flags, autocomplete providers, model providers, OAuth providers, custom tool renderers, and terminal-input handlers. Discovered from `~/.pi/agent/extensions/*/pi.extension.json`, loaded explicitly with `-e`, or installed with `pi install <npm-spec|git-url|path>` (`-l` for project scope). `pi list`, `pi remove`, and `pi config` (a TUI for enabling/disabling package resources) manage them.

**Native Rust extensions** are compiled in and always available — no Node, no subprocess:

| | commands | tools |
| --- | --- | --- |
| Graph engineer | `/graph`, `/graph-resume`, `/graph-status`, `/graph-view`, `/graph-abort` | `graph_run`, `graph_status` |
| Token governor | `/governor-status`, `/governor-reset` | `retrieve_output` |
| Vector memory | `/memory-status`, `/memory-search`, `/memory-reindex`, `/memory-clear` | `memory_search` |
| Security scan | `/sec-status`, `/sec-report`, `/sec-abort` | 14 `sec_*` tools |

---

## Graph engineer

```
/graph <goal> [--simple|--complex] [--dry-run]
```

Runs one coding task as an explicit execution graph of isolated `pi` child processes rather than as one long conversation. The controller is deterministic Rust; models only ever run inside a worker child, and the only thing that crosses a node boundary is a schema-validated JSON artifact.

**Pipeline:** `classify → investigate → plan → implement → verify → review`.

| Role | Artifact it owes | Tools it gets | Shell policy |
| --- | --- | --- | --- |
| classifier | `classification` | `graph_submit` only | none |
| researcher / test-analyzer / reviewer | `evidence` / `review` | read, grep, find, ls, bash | read-only / read-and-test |
| historian | `evidence` | read, grep, bash | read-only |
| planner | `plan` | read, grep, find, ls | none |
| writer | `patch-report` | read, grep, find, ls, bash, edit, write | write, but no git state changes |

Least privilege is enforced twice — as the child process's `--tools` allowlist, and again inside the child, because the danger of `bash` lives in its command text rather than its name. The writer is the only node that may mutate files, and no node may run `git commit`, `push`, `reset`, `checkout`, or any other git state change: committing is reserved for the human operator. A destructive-pattern list (36 rules covering `rm`, redirects, package installs, `sudo`, PowerShell equivalents) blocks the obvious ways around a read-only policy. Once a worker calls `graph_submit`, every further tool call in that process is refused.

**Verification has no model in it at all.** "Did the tests pass?" is an exit code. Verify commands come from `.pi/graph.json`, or are auto-detected (Cargo: `cargo fmt --check`, `cargo clippy … -D warnings`, `cargo test --workspace`; npm: `npm run check` / `typecheck` / `lint` / `npm test`). Test commands proposed by the planner are filtered through the same read-and-test shell policy before they run, and a command the planner invented that does not exist is marked skipped rather than failing the run and burning a revision cycle.

Failures loop rather than abort: a failed verification or a `changes_required` review sends the work back to the writer, bounded by `maxRevisionCycles`; a `planInvalidated` patch report triggers a replan, bounded by `maxReplans`. A run with no review artifact is blocked — nothing is approved by default.

`--simple` collapses the graph to `classify → implement → verify`. `--complex` forces the full path and enables milestone decomposition (up to 8 milestones, each with its own plan/implement/verify/review). `--dry-run` exercises the whole graph with canned artifacts and skipped shell commands, spending no tokens.

Runs are persisted under `.pi/graph/runs/<runId>/` (`state.json`, `artifacts/`, `logs/`), so `/graph-resume` can replay completed nodes without respawning their workers — with the guard that if the previous run had already entered a revision loop, only the investigation nodes are reused, because a superseded plan or patch should not be replayed. `/graph-view` tails a live worker transcript. The newest 20 finished runs are retained.

Everything is bounded, and every budget is off by default rather than silently guessing: `maxResearchers` (3), `maxParallelWorkers` (3), `maxWorkers`, `maxRevisionCycles` (3), `maxReplans` (2), `maxCostUsd`, `runDeadlineMs`, and per-role worker timeouts. Per-role model pins let a cheap model classify and an expensive one write. A background run never outlives its session: session shutdown aborts every run and kills the worker process tree.

**Hardened Invariants:**
- **Explicit execution topology**: Runs execute against a validated, persisted DAG definition (`GraphDefinition`). DAG validation strictly forbids cycles, review bypass, missing verification, and concurrent mutation-capable writers.
- **Active run deadlines**: Enforces `run_deadline_ms` across the run and per worker, actively terminating long-running child processes when wall-clock limits expire.
- **Deterministic replay fingerprints**: Completed tasks persist a `ReplayFingerprint` (graph version, config hash, repo state, briefing hash, contract hash). Incompatible cached nodes are rejected and re-executed with explicit diagnostics; tasks superseded by a revision cycle are never replayed.
- **Graph-owned mutation provenance**: Changes made by writer nodes are captured deterministically against a pre-mutation baseline, excluding pre-existing uncommitted user edits and preserving Git index integrity.
- **Complete review coverage**: Large mutations exceeding review context thresholds are deterministically split into line-bounded `ReviewChunk`s with stable IDs. `ReviewCoverage` ensures every chunk is reviewed before final approval is possible.

Configure in `.pi/graph.json` (no file means all defaults and no errors; a malformed file reports the problem and proceeds on defaults):

```json
{
  "budgets": { "maxResearchers": 3, "maxRevisionCycles": 3, "maxCostUsd": 5.0 },
  "models": { "classifier": "google/gemini-2.5-flash", "writer": "anthropic/claude-opus-4-6" },
  "verifyCommands": [{ "name": "test", "command": "cargo test --workspace" }],
  "workerExtraTools": []
}
```

## Token governor

Large tool outputs are the fastest way to burn a context window, and most of a 4,000-line build log is not the part that matters.

When a tool result crosses 8 KB or 200 lines, the governor replaces it with a digest — first 15 lines, up to 60 "notable" lines (matching `error`, `warn`, `fail`, `panic`, `exception`, `todo`, `fixme`), last 30 lines, runs of identical lines collapsed — and writes the full text to disk under a content-addressed id. The model gets a footer telling it how many lines were omitted, and can call `retrieve_output` to read the original back losslessly, by line range or filtered by substring. Nothing is lost; it just stops being resident.

Two other savings, both on by default:

- **Read dedup.** Re-reading a file whose content hash has not changed returns `[unchanged read: …; the previous output is still valid]` instead of the file again.
- **Anti-loop.** A `grep`, `find`, or `ls` call identical to an earlier one — same normalized arguments, same repository state (git HEAD plus `git status --porcelain`) — is blocked with a note to change the query or re-read the previous result. Only calls that actually succeeded enter the ledger, so a failed search stays retryable, and the ledger is bounded at 200 entries.

Error results, `memory_search`, and `retrieve_output` itself are never compressed. Every path fails open: if the store cannot be written, the output simply passes through uncompressed. `/governor-status` shows the counters; `/governor-reset` clears the ledgers.

Configure in `~/.pi/agent/token-governor.json` (`compressThresholdBytes`, `compressThresholdLines`, `keepHeadLines`, `keepTailLines`, `maxImportantLines`, `dedupeReads`, `antiLoop`, `storeDir`) or with the matching `PI_TOKEN_GOVERNOR_*` environment variables.

## Vector memory

Durable, per-repository memory that survives sessions and compaction.

Conversation turns are chunked (4,000 chars), classified (`Task`, `Decision`, `Fact`, `Conversation`), redacted of secrets, and stored locally at `.pi/vector-memory/records.jsonl`, scoped to the repository's git origin so another checkout's notes never leak in. Retrieval is hybrid: `0.6 × dense + 0.3 × lexical + 0.1 × importance`, with hits below `minimumScore` dropped.

It runs fully offline by default. Qdrant and Ollama are optional accelerators — if no embeddings exist locally, no embedding request is made at all, and lexical scoring answers on its own, so a missing daemon costs a fallback rather than a timeout. When automatic retrieval is on, matching memories are injected as ephemeral context ahead of the turn, wrapped in a `<pi-memory>` block that explicitly marks the content as data and not instructions.

`/memory-search <query>` searches it directly, `memory_search` exposes the same thing to the model, `/memory-status` reports record counts and daemon health, `/memory-reindex` reloads, `/memory-clear` wipes the store.

Configure in `~/.pi/agent/vector-memory.json` (`enabled`, `ollamaUrl`, `embeddingModel`, `qdrantUrl`, `collection`, `automaticRetrieval`, `resultLimit`, `maxInjectedTokens`, `minimumScore`, …) or with `PI_MEMORY_*` environment variables.

## Security scan

A deterministic local scan with hash-sealed, immutable artifacts — built so a result can be audited later rather than merely believed.

The scanner enumerates in-scope files (no symlinks; `.git`, `node_modules`, and `target` excluded; over 2 MiB or binary is skipped and counted), applies fixed rules, and records candidates and findings. Evidence is redacted before it is written anywhere: private key material, `sk-`/`ghp_`/`Bearer` tokens, and `password=` values never reach an artifact.

Artifacts are written outside the repository, under the system temp directory, as `findings.json`, `candidates.json`, `coverage.json`, `report.md`, `report.sarif`, and a `scan-manifest.json` with a SHA-256 seal. Completing a scan makes it immutable — a later mutation attempt is refused, and `sec_tracking_validate` re-verifies every sealed artifact's hash and length and reports any drift. Network access is off by default and tracked in the coverage report.

Fourteen `sec_*` tools drive the lifecycle (start, scope, progress, candidate record/list/validate, attack-path analysis, deep scan, complete, cancel); `/sec-status`, `/sec-report`, and `/sec-abort` drive it from the prompt.

## Self-improving learning

Turn settled agent turns into durable memory and reusable procedural skills (`SKILL.md`) with a fail-open background review loop.

- **Fail-open & Non-blocking:** Background reviews run on asynchronous worker threads and are cancelled cooperatively when a new turn begins. Normal agent execution never fails due to learning operations.
- **Review Gating:** Evaluates turn evidence with `should_review_evidence`, skipping low-signal read-only turns to cut reviewer input tokens by >= 40% while preserving 100% of accepted high-confidence artifacts. Memory indexing remains active.
- **Exact Version Attribution:** Skills carry explicit `SkillVersionRef (name, version, content_hash)`. Graph execution outcomes (`VerifiedSuccess`, `VerifiedFailure`, `Neutral`) update only the specific targeted version ledger record.
- **Closed-Loop Graph Learning:** Verified graph completions persist high-confidence memories and project skills. Later runs with related goals automatically retrieve these exact skill versions and memories into worker context, and successful verification increments the skill version's success counter.
- **Conditional Security Gate:** Graph changes undergo deterministic change-risk classification (`assess_change_risk`); high-risk mutations or `always` policy trigger non-interactive security verification (`verify_changed_surface`) before review, blocking approval on unmitigated blockers.
- **Commands:** `/learn` to distill procedures in the foreground; `/learning-status`, `/learning-pending`, `/learning-approve <id>`, `/learning-reject <id>`, `/skill-list`, and `/skill-view <name>`.
- Full documentation in [`docs/learning.md`](docs/learning.md).

---

## Configuration

| Path | What |
| --- | --- |
| `~/.pi/agent/settings.json` | user settings |
| `~/.pi/agent/auth.json` | provider credentials |
| `~/.pi/agent/keybindings.json` | key overrides |
| `~/.pi/agent/trust.json` | per-project trust decisions |
| `~/.pi/agent/sessions/` | session JSONL, grouped by encoded cwd |
| `~/.pi/agent/{extensions,skills,prompts,themes}/` | user resources |
| `~/.pi/agent/token-governor.json` | token governor |
| `~/.pi/agent/vector-memory.json` | vector memory |
| `<project>/.pi/settings.json` | project settings (requires trust) |
| `<project>/.pi/graph.json` | graph budgets, per-role models, verify commands |
| `<project>/.pi/graph/runs/` | graph run state and artifacts |
| `<project>/.pi/vector-memory/records.jsonl` | repository memory |

`PI_CODING_AGENT_DIR` relocates the agent directory, `PI_CODING_AGENT_SESSION_DIR` the session store, `PI_OFFLINE=1` (or `--offline`) disables all startup network work, and `PI_NODE` points at a specific Node binary. Provider keys are read from the conventional variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `AWS_*`, and so on) — `pi --help` lists them all.

---

## Repository Structure & Navigation

The repository is organized into distinct functional domains:

```
pi-rust/
├── crates/             # 13 production Rust crates (the active implementation)
│   └── README.md       # Crate architecture & dependency guide
├── docs/               # Architecture specs, plans, UI mockups & security reviews
│   └── README.md       # Full documentation index & navigation hub
├── scripts/            # Build & installation scripts (pwsh, bash)
├── packages/           # Legacy TypeScript monorepo stubs from initial porting
│   └── README.md       # Legacy package context
├── vendor/             # Upstream behavioral reference source (vendor/pi)
├── Cargo.toml          # Cargo workspace root configuration
├── Makefile            # Standard developer commands (build, test, fmt, clippy)
├── CLAUDE.md           # Instructions for AI coding assistants
└── README.md           # Product overview and user documentation
```

### Workspace Crates

Dependencies flow strictly bottom-up; `davinci-coding-agent` is the primary executable binary. See [`crates/README.md`](crates/README.md) for the detailed architecture.

| Crate | Role | Documentation |
| :--- | :--- | :--- |
| `davinci-coding-agent` | CLI entry point (`davinci`), interactive shell, extensions | [`crates/davinci-coding-agent/README.md`](crates/davinci-coding-agent/README.md) |
| `davinci-agent` | Agent loop, tool execution engine, permissions, scheduler | [`crates/davinci-agent/README.md`](crates/davinci-agent/README.md) |
| `davinci-ai` | Providers, auth/OAuth, streaming, model catalog, cost | [`crates/davinci-ai/README.md`](crates/davinci-ai/README.md) |
| `davinci-tui` | Terminal UI (Ratatui), instruments, sheets, themes | [`crates/davinci-tui/README.md`](crates/davinci-tui/README.md) |
| `davinci-session` | JSONL session store, discovery, turn history | [`crates/davinci-session/README.md`](crates/davinci-session/README.md) |
| `davinci-session-sqlite` | SQLite branch cache and session indexing | [`crates/davinci-session-sqlite/README.md`](crates/davinci-session-sqlite/README.md) |
| `davinci-mcp` | Native Model Context Protocol (MCP) client & transports | [`crates/davinci-mcp/README.md`](crates/davinci-mcp/README.md) |
| `davinci-protocol` | Length-prefixed CBOR wire framing and RPC types | [`crates/davinci-protocol/README.md`](crates/davinci-protocol/README.md) |
| `davinci-client` | Client SDK for communicating with the agent daemon | [`crates/davinci-client/README.md`](crates/davinci-client/README.md) |
| `davinci-server` | Standalone background RPC daemon server | [`crates/davinci-server/README.md`](crates/davinci-server/README.md) |
| `davinci-telemetry` | Telemetry events, OpenTelemetry, metrics | [`crates/davinci-telemetry/README.md`](crates/davinci-telemetry/README.md) |
| `davinci-evals` | Automated evaluation harness and benchmark runners | [`crates/davinci-evals/README.md`](crates/davinci-evals/README.md) |
| `davinci-parity` | Golden fixtures and differential parity testing | [`crates/davinci-parity/README.md`](crates/davinci-parity/README.md) |


## Development

```bash
make build     # cargo build -p davinci-coding-agent
make test      # cargo test --workspace
make fmt       # cargo fmt --check
make clippy    # cargo clippy --workspace --all-targets -- -D warnings
cargo run -p davinci-parity                            # golden-fixture parity corpora
```

Tests are fixture-only and never touch the network: anything that would call a provider, an installer, a browser, or an update server is driven by a `PI_*` fixture environment variable read at the call site.

## License

MIT, matching upstream pi.
