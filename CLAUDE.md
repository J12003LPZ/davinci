# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

A Rust reimplementation of the TypeScript agent CLI [`pi`](https://github.com/earendil-works/pi), pinned to vendor commit `853a80d26c90a14c1886f0ebb8ffaae133ca2185`. The goal is *product equivalence*: a user who runs TypeScript `pi` installs this binary and keeps the same flags, `~/.pi` sessions, provider credentials, interactive TUI, `--print`, and `--mode rpc`.

`vendor/pi` holds the authoritative TypeScript source (~1,169 `.ts` files). It is reference-only — read it constantly, never delete or edit it. `packages/*` holds a handful of stale TypeScript stubs from the early phases; they are not the reference.

Phase plans describing the rewrite are in `docs/superpowers/plans/`. Comprehensive documentation index is in `docs/README.md` and crate architecture is in `crates/README.md`.

## Commands

```bash
make build          # cargo build -p davinci-coding-agent   (produces the `davinci` binary)
make test           # cargo test --workspace
make fmt            # cargo fmt --check
make clippy         # cargo clippy --workspace --all-targets -- -D warnings
make install        # cargo install --path crates/davinci-coding-agent --force
```

Single crate / single test:

```bash
cargo test -p davinci-agent                       # one crate
cargo test -p davinci-agent compaction            # tests whose name contains "compaction"
cargo test -p davinci-coding-agent -- --nocapture # show stdout
```

Running the binary:

```bash
./target/debug/davinci --help
./target/debug/davinci -p "List files in src/"   # print mode
./target/debug/davinci --mode rpc                # JSON-RPC over stdio
cargo run -p davinci-parity                      # golden-fixture parity corpora
```

Toolchain is pinned to Rust 1.83.0 (`rust-toolchain.toml`). Every workspace dependency is pinned with `=` exact versions; keep that convention when adding one.

## Architecture

Dependency direction is strictly bottom-up; `davinci-coding-agent` is the product binary (`davinci`). `davinci-mcp` also ships `mcp-fixture`, an in-tree stdio MCP server for tests.

```
davinci-coding-agent (bin `davinci`) — CLI, TUI wiring, extensions, slash commands, auth UI
  ├── davinci-tui        — terminal rendering: `davinci/` (ratatui, the interactive
  │                        default) and the legacy chrome (Component trait -> Vec<String>)
  ├── davinci-agent      — agent loop, tools, compaction, skills, prompt templates
  │     ├── davinci-ai   — providers, auth/OAuth, streaming, model catalog, cost
  │     └── davinci-mcp  — native MCP client (stdio + streamable HTTP); no TS counterpart
  ├── davinci-session(-sqlite) — JSONL session store + sqlite branch cache
  ├── davinci-protocol   — length-prefixed CBOR wire format
  ├── davinci-client / davinci-server — protocol client/server over Unix socket or TCP
  ├── davinci-evals, davinci-telemetry
  └── davinci-parity     — golden fixtures, optional diff against the TS binary
```

**Entrypoint dispatch** (`crates/davinci-coding-agent/src/main.rs`, ~7k lines): `run()` picks a mode — `run_rpc` (`--mode rpc`), `run_print` (`--print` / `--mode json` / non-TTY stdin or stdout), otherwise `run_interactive`, which opens the davinci shell (`davinci_interactive::run`) unless `--legacy-tui` or `PI_DAVINCI=0` asks for the old chrome. Unix-only `experimental` subcommands (`server`, `client`) are stubbed out on Windows by an inline `mod experimental` in `main.rs`.

**Provider streaming** (`davinci-ai`): every request goes through `live_complete_streaming_with_sink` (`stream.rs`), which reads the SSE body on a reader thread and hands each decoded event to a sink as it arrives; `StreamOptions::abort_signal` is polled between frames. Wire formats are decoded by `stream_decoder.rs` (Responses/Codex), `stream_decoder_completions.rs` and `stream_decoder_anthropic.rs`; APIs without a decoder are requested without `stream: true` and their events synthesised. Responses tool-call ids are stored as `call_id|item_id` and only the `call_id` half is replayed. `PI_AI_TRACE=1` (or a file path) logs every request, frame and failure — reach for it before reading code when a turn misbehaves.

**Agent runtime** (`davinci-agent`): built-in tools are `read, write, edit, bash, powershell, grep, find, ls, web_fetch, web_search, todo, job_output, job_kill, notebook_edit, mcp_read, agent, batch, apply_patch`. Tools from phase 3 onward have no TypeScript counterpart (phase 3 spec `docs/superpowers/specs/2026-09-01-tools-that-compete-design.md`; phase 4 MCP spec `docs/superpowers/specs/2026-09-01-native-mcp-design.md`; phase 5 spec `docs/superpowers/specs/2026-09-01-plan-and-subagents-design.md`). MCP servers from `~/.pi/agent/mcp.json` (and trusted `.pi/mcp.json`) become agent tools named `mcp__<server>__<tool>`; `mcp_read` reads a listed resource. `/plan` freezes mutations until `/act`. `agent` starts a scoped read-only worker (depth 1; `PI_SUBAGENT_FIXTURE` in tests). Extensions inject behavior through `PreToolHook` / `PostToolHook` / custom tool functions. Compaction (`compaction.rs`) and branch summarization (`branch.rs`) reproduce the TS prompts verbatim — the prompt string constants are part of the parity contract, do not reword them.

**Tool scheduling and context budget** (spec `docs/superpowers/specs/2026-09-02-harness-throughput-design.md`): the calls of one assistant message run through `scheduler.rs` in lanes — read-class tools, read-only MCP tools and `agent` overlap on up to 8 threads, while `write`/`edit`/shell/extension tools are barriers that keep source order — and `Agent::new` defaults to `ToolExecutionMode::Parallel` like TS. `turn.rs` is three stages (prepare on the loop thread with the permission gate, run on `&self`, finalize in source order). `batch` runs up to 16 operations behind one tool result (each gated like a direct call; 12 KB/64 KB visible caps, overflow to the evidence store `~/.pi/agent/evidence/`, no nesting); `agent { tasks: [...] }` fans out up to 8 workers. `pruning.rs` replaces old large tool results with a placeholder in the provider view once the estimate passes 50% of the window (history and the session file keep every byte; compaction is unchanged). `RunStats` (`stats.rs`, `Agent::run_stats()`) counts turns, batch widths, wall time, peak context and prunings; it is `runtime` in `get_session_stats` and a block in `/status`. `TOOL_USE_STRATEGY` in `lib.rs` is the prompt's half of the bargain and rides on the default and worker prompts.

**Tool permissions** (`davinci-agent/src/permission.rs`, `davinci-coding-agent/src/permissions.rs`; no TS counterpart, spec in `docs/superpowers/specs/2026-09-01-trust-and-control-design.md`): every tool call passes a gate in `Agent::execute_one` after the extension `tool_call` hook. Modes are `read-only | ask | edits | auto` (`--permission-mode`, `--sandbox` with Codex names, `permissions.mode` in settings, `/permissions <mode>` for the session); rules are `tool` or `tool(glob)` under `permissions.allow` / `permissions.deny` in the user file and, only when the project is trusted, `.pi/settings.json`. Deny rules always win (tool-name globs such as `mcp__memory__*` match). MCP tools with `annotations.readOnlyHint` are `Read`; other `mcp__*` tools are `Other`; `mcp_read` is `Read`. An `Ask` goes to `Agent.approver`: davinci opens the `LICENTIA · PERMISSION` panel mid-turn, RPC emits a `select` UI request, the legacy chrome uses its confirm dialog, and `--print` fails closed with a message naming the flag and rule. The library default (`Agent::new`) is `auto`, vendor behaviour; `build_agent` installs the configured policy, default `ask`.

**Extensions** are two-tier (`extension_host.rs`): JavaScript extensions run in a Node subprocess driven by the embedded `extension_runner.js` (`js_host.rs`, only when Node is present), while the bundled pi extensions have been ported to native Rust under `src/native_extensions/` (`vector_memory`, `token_governor`, `security_scan`, `graph`, `learning`), exposed via `NATIVE_TOOLS` / `NATIVE_COMMANDS`. The token governor digests large outputs of shell/search tools only (`LOSSLESS_TOOLS` — `read`, `edit`, `write`, `batch`, `agent`… stay verbatim), names the `retrieve_output` id inside the digest because tool `details` never reach the model, and replaces a byte-identical repeat `read` with a marker only within `dedupe_window` (6) tool calls — below pruning's `keep_recent` (8), so a pruned twin is served again; stored outputs live under `~/.pi/agent/token-governor/outputs/<session>/` and other sessions' are swept after 14 days. Vector memory indexes only `user`/`assistant` messages (tool output is transient), dedupes by content hash, honours `automaticRetrieval`, and turns dense retrieval off for two minutes after an Ollama failure so a dead host costs one timeout per window. Graph workers are `--print` children spawned with `--permission-mode auto` (nobody can answer a prompt in a child; the per-role `--tools` allowlist and `worker_hooks` bash policy are their gate), their `graph_submit` tool carries the artifact's JSON schema and the same contract goes into the worker's system prompt (`validate.rs::artifact_schema` / `artifact_contract` — keep them in step with the validators); the shell policy checks every `&&`/`;`/`|` segment, verification with no command that ran is never a pass, and `graph_run` executes without holding the native-host mutex. The `5a` sheet re-reads `graph-status` once a second while open; `/graph` opens it directly.

**Self-improving learning system** (`crates/davinci-coding-agent/src/native_extensions/learning/`, design in `docs/superpowers/specs/2026-09-03-davinci-self-improving-learning-design.md`, docs in `docs/learning.md`): Settled turns hook (`complete_prompt_with_host`) invokes the asynchronous background reviewer thread (`reviewer.rs`). The reviewer extracts deterministic verification evidence (`evidence.rs`) by analyzing shell commands (`bash`, `powershell`, `execute_command`), `graph_run` outcomes, tool exit codes, tool errors, permission denials, and user acceptance/correction signals. By default, learning operates fully autonomously with zero user interaction (`shadow_mode = false`, `auto_apply_project = true`, `auto_apply_global = true` in `config.rs`):
- **Declarative memory facts** (`LearningArtifact::Memory`): high-confidence facts (≥ 0.80) are persisted directly into vector memory without requiring user confirmation.
- **Procedural skills** (`SkillCreate`, `SkillPatch`): saved as `SKILL.md` under `.pi/skills/<name>/` (project) or `~/.pi/agent/skills/<name>/` (global) with structured YAML frontmatter. Candidate skills auto-promote to `Active` after reaching 2 verified usages without failures (`mod.rs::auto_promote_if_threshold_met`).
- **Progressive disclosure & discovery**: Native agent tools `skill_list` and `skill_view` allow discovery and on-demand inspection of skill content without bloating system prompt context. Skills are hot-reloaded dynamically into `agent.skills` via `Agent::reload_skills`.
- **Append-only ledger & rollback**: `~/.pi/agent/learning/ledger.jsonl` tracks proposals, verifications, activations, deprecations, and patches (`store.rs`), keeping backup snapshots for rollbacks (`skill_manager.rs`).
- **Interactive commands & control**: `/learn [instruction]` synthesizes learning turns; `/learning-status`, `/learning-pending`, `/learning-approve <id>`, `/learning-reject <id>`, `/skill-list`, and `/skill-view <name>` provide manual review and observability when needed.
- **Fail-open lifecycle**: Learning operations run asynchronously in the background and will never block or fail foreground turn executions.

**Sessions** are JSONL files under `~/.pi/agent/sessions`, in cwd-encoded directories byte-compatible with TypeScript `pi`. Path resolution lives in `davinci-session/src/discovery.rs` (`default_agent_dir`, `default_session_dir`). Project config and trust decisions live under `.pi/` in the project (`settings.rs`, `trust.rs`).

**Protocol**: `davinci-protocol` is length-prefixed CBOR with explicit depth/length limits; `davinci-server` serves an `Agent` over it and `davinci-client` consumes it. `PROTOCOL_VERSION` compatibility is checked on hello.

**The davinci TUI** (`davinci-tui/src/davinci/`, driven by `davinci-coding-agent/src/davinci_{interactive,sources}.rs`) implements `docs/ui/design.md` against two canvases: `docs/ui/Pi TUI Mockups.dc.html` for the transcript screens (`1a`–`1h`, `2a`–`2c`) and `docs/ui/Pi TUI Instruments.dc.html` for the command sheets (`3a`–`6d`, one artboard each; spec `docs/superpowers/specs/2026-09-02-davinci-instruments-fidelity-design.md`, rules in design.md §11). Its rules are a contract, not preferences: one panel at a time, every state carries a glyph so `NO_COLOR` still reads, `theme.rs` is the only file holding a colour literal, exactly two things animate off one 250ms clock, prose wraps at 74 columns, and numbers are meters with their unit and cap. Views return `Vec<Line<'static>>` so heights are known before rendering. `davinci --davinci --screen <id>` renders one mockup screen against fixtures for comparison — it is interactive and blocks; for headless frame dumps run `cargo test -p davinci-tui dump_every_screen_for_the_mockup_audit -- --ignored --nocapture`. The mockups' source of truth is the claude.ai/design project `0b6a4165-8090-4dd0-b21c-80ffd802052d` (DesignSync MCP, auth via `/design-login`); the `docs/ui/` copies are byte-identical exports. The `Mirrors docs/ui/davinci_tui/*.ex` doc comments on the `1a`–`2c` views cite the Elixir/Ratatouille reference tree at `docs/ui/davinci_tui/` (`mix deps.get && mix run run.exs`; needs Elixir, which the Rust build does not); for the command sheets that tree is superseded by the Instruments canvas and their views carry `Mirrors artboard <id> of docs/ui/Pi TUI Instruments.dc.html` instead. Screens `3a`–`6d` — `/model`, `/settings`, `/thinking`, `/login`, `/hotkeys`, `/resume`, `/tree`, `/compact`, `/export`, `/graph`, `memory-status`, `governor-status`, `sec-report`, `/trust`, `/reload`, interrupt, and the `Δ` review (`/diff`, a davinci-only command) — share one frame, the `SheetChrome` descriptor in `views/sheet.rs` (header facts, status third, hint row, composer, echo); `fixtures.rs` dresses every sheet with its artboard's facts for the audit, and `davinci_interactive.rs` opens them with live data (settings store, model runtime, session store, git, the native extensions), omitting any fact it cannot compute rather than inventing one. `/mcp` and `/permissions` have no artboard but wear the same frame. Where a mockup and design.md disagree on a visual detail, the mockup wins.

## Conventions

- **Cite the TypeScript source.** Most modules open with a doc comment naming the file they mirror, e.g. `//! Project trust store matching vendor/pi/packages/coding-agent/src/core/trust-manager.ts`. Follow this for new modules; when changing behavior, read the cited TS file first and match it rather than improving on it.
- **Tests are fixture-only and never touch the network.** Behavior that would call a provider, an installer, a browser, or an update server is instead driven by `PI_*` fixture environment variables read at the call site — `PI_OFFLINE`, `PI_DISABLE_NETWORK`, `PI_SELF_UPDATE_FIXTURE`, `PI_OAUTH_TOKEN_FIXTURE`, `PI_PACKAGE_FIXTURE`, `PI_LLAMA_*`, `PI_MANAGED_INSTALLER_REPLY`, `PI_OPEN_BROWSER_DRY_RUN`, `PI_MCP_CONFIG`, `PI_MCP_FIXTURE`, `PI_SUBAGENT_FIXTURE`, `PI_HOOKS_CONFIG`, `PI_HOOKS_DRY_RUN`, and others. MCP HTTP tests may also use a `fixture:<path>` URL. Add a fixture hook rather than a live call when a new code path needs one.
- Tests are inline `#[cfg(test)] mod tests` blocks (~188 of them); there are no `tests/` directories except `davinci-parity/fixtures`.
- Divergence from TS is acceptable only where the platform forces it (the `powershell` tool, the Windows `experimental` stub); document it in a comment.

## Gotchas

- **Not every file in `src/` is compiled.** Several crates have leftover source files that no `mod` declaration references: `davinci-session/src/{jsonl,memory,backend,conformance}.rs`, `davinci-agent/src/{loop_,permission}.rs`, most of `davinci-session-sqlite/src/` (only `branch_cache` is declared), and the whole `crates/davinci-core` crate, which is not a workspace member and has no dependents. Before editing something a grep turned up, check that its module is actually declared — otherwise you will be editing dead code.
- **`home_dir()` mirrors Node `os.homedir()`** (`davinci-session/src/discovery.rs`): `USERPROFILE` first on Windows, `HOME` otherwise. Session dirs use the TS `--…--` cwd encoding (every `/`, `\`, `:` becomes `-`); the older Rust `--a--b` encoding is still scanned read-only for pre-existing stores. Tests that create sessions should still set `PI_CODING_AGENT_DIR` or `PI_CODING_AGENT_SESSION_DIR` so they never touch the real `~/.pi`; if `--Users--…/` directories ever reappear in the repo, they are test/session strays — delete, never commit.
- **Git pre-commit hook on Windows**: Environments where `core.hooksPath` points to Unix shell scripts (e.g. `~/.codex/git-hooks`) may fail on Windows with `execvpe(/bin/bash) failed: No such file or directory`. Use `git commit --no-verify` to bypass this hook when committing on Windows.
- `davinci-coding-agent` (~47k lines) and `davinci-tui` (~39k lines) are large; prefer targeted `grep` over reading whole files.
