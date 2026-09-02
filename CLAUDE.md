# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

A Rust reimplementation of the TypeScript agent CLI [`pi`](https://github.com/earendil-works/pi), pinned to vendor commit `853a80d26c90a14c1886f0ebb8ffaae133ca2185`. The goal is *product equivalence*: a user who runs TypeScript `pi` installs this binary and keeps the same flags, `~/.pi` sessions, provider credentials, interactive TUI, `--print`, and `--mode rpc`.

`vendor/pi` holds the authoritative TypeScript source (~1,169 `.ts` files). It is reference-only — read it constantly, never delete or edit it. `packages/*` holds a handful of stale TypeScript stubs from the early phases; they are not the reference.

Phase plans describing the rewrite are in `docs/superpowers/plans/`.

## Commands

```bash
make build          # cargo build -p pi-coding-agent   (produces the `pi` binary)
make test           # cargo test --workspace
make fmt            # cargo fmt --check
make clippy         # cargo clippy --workspace --all-targets -- -D warnings
make install        # cargo install --path crates/pi-coding-agent --force
```

Single crate / single test:

```bash
cargo test -p pi-agent                       # one crate
cargo test -p pi-agent compaction            # tests whose name contains "compaction"
cargo test -p pi-coding-agent -- --nocapture # show stdout
```

Running the binary:

```bash
./target/debug/pi --help
./target/debug/pi -p "List files in src/"   # print mode
./target/debug/pi --mode rpc                # JSON-RPC over stdio
cargo run -p pi-parity                      # golden-fixture parity corpora
```

Toolchain is pinned to Rust 1.83.0 (`rust-toolchain.toml`). Every workspace dependency is pinned with `=` exact versions; keep that convention when adding one.

## Architecture

Dependency direction is strictly bottom-up; `pi-coding-agent` is the only binary.

```
pi-coding-agent (bin `pi`)  — CLI, TUI wiring, extensions, slash commands, auth UI
  ├── pi-tui        — terminal rendering: `davinci/` (ratatui, the interactive
  │                   default) and the legacy chrome (Component trait -> Vec<String>)
  ├── pi-agent      — agent loop, tools, compaction, skills, prompt templates
  │     └── pi-ai   — providers, auth/OAuth, streaming, model catalog, cost
  ├── pi-session(-sqlite) — JSONL session store + sqlite branch cache
  ├── pi-protocol   — length-prefixed CBOR wire format
  ├── pi-client / pi-server — protocol client/server over Unix socket or TCP
  ├── pi-evals, pi-telemetry
  └── pi-parity     — golden fixtures, optional diff against the TS binary
```

**Entrypoint dispatch** (`crates/pi-coding-agent/src/main.rs`, ~7k lines): `run()` picks a mode — `run_rpc` (`--mode rpc`), `run_print` (`--print` / `--mode json` / non-TTY stdin or stdout), otherwise `run_interactive`, which opens the davinci shell (`davinci_session::run`) unless `--legacy-tui` or `PI_DAVINCI=0` asks for the old chrome. Unix-only `experimental` subcommands (`server`, `client`) are stubbed out on Windows by an inline `mod experimental` in `main.rs`.

**Provider streaming** (`pi-ai`): every request goes through `live_complete_streaming_with_sink` (`stream.rs`), which reads the SSE body on a reader thread and hands each decoded event to a sink as it arrives; `StreamOptions::abort_signal` is polled between frames. Wire formats are decoded by `stream_decoder.rs` (Responses/Codex), `stream_decoder_completions.rs` and `stream_decoder_anthropic.rs`; APIs without a decoder are requested without `stream: true` and their events synthesised. Responses tool-call ids are stored as `call_id|item_id` and only the `call_id` half is replayed. `PI_AI_TRACE=1` (or a file path) logs every request, frame and failure — reach for it before reading code when a turn misbehaves.

**Agent runtime** (`pi-agent`): built-in tools are `read, write, edit, bash, powershell, grep, find, ls`. Extensions inject behavior through `PreToolHook` / `PostToolHook` / custom tool functions. Compaction (`compaction.rs`) and branch summarization (`branch.rs`) reproduce the TS prompts verbatim — the prompt string constants are part of the parity contract, do not reword them.

**Tool permissions** (`pi-agent/src/permission.rs`, `pi-coding-agent/src/permissions.rs`; no TS counterpart, spec in `docs/superpowers/specs/2026-09-01-trust-and-control-design.md`): every tool call passes a gate in `Agent::execute_one` after the extension `tool_call` hook. Modes are `read-only | ask | edits | auto` (`--permission-mode`, `--sandbox` with Codex names, `permissions.mode` in settings, `/permissions <mode>` for the session); rules are `tool` or `tool(glob)` under `permissions.allow` / `permissions.deny` in the user file and, only when the project is trusted, `.pi/settings.json`. Deny rules always win. An `Ask` goes to `Agent.approver`: davinci opens the `LICENTIA · PERMISSION` panel mid-turn, RPC emits a `select` UI request, the legacy chrome uses its confirm dialog, and `--print` fails closed with a message naming the flag and rule. The library default (`Agent::new`) is `auto`, vendor behaviour; `build_agent` installs the configured policy, default `ask`.

**Extensions** are two-tier (`extension_host.rs`): JavaScript extensions run in a Node subprocess driven by the embedded `extension_runner.js` (`js_host.rs`, only when Node is present), while the bundled pi extensions have been ported to native Rust under `src/native_extensions/` (`vector_memory`, `token_governor`, `security_scan`, `graph`), exposed via `NATIVE_TOOLS` / `NATIVE_COMMANDS`.

**Sessions** are JSONL files under `~/.pi/agent/sessions`, in cwd-encoded directories byte-compatible with TypeScript `pi`. Path resolution lives in `pi-session/src/discovery.rs` (`default_agent_dir`, `default_session_dir`). Project config and trust decisions live under `.pi/` in the project (`settings.rs`, `trust.rs`).

**Protocol**: `pi-protocol` is length-prefixed CBOR with explicit depth/length limits; `pi-server` serves an `Agent` over it and `pi-client` consumes it. `PROTOCOL_VERSION` compatibility is checked on hello.

**The davinci TUI** (`pi-tui/src/davinci/`, driven by `pi-coding-agent/src/davinci_{session,sources}.rs`) implements `docs/ui/design.md` against the mockups in `docs/ui/Pi TUI Mockups.dc.html` (screens `1a`–`1h`, `2a`–`2c`). Its rules are a contract, not preferences: one panel at a time, every state carries a glyph so `NO_COLOR` still reads, `theme.rs` is the only file holding a colour literal, exactly two things animate off one 250ms clock, prose wraps at 74 columns, and numbers are meters with their unit and cap. Views return `Vec<Line<'static>>` so heights are known before rendering. `pi --davinci --screen <id>` renders one mockup screen against fixtures for comparison — it is interactive and blocks; for headless frame dumps run `cargo test -p pi-tui dump_every_screen_for_the_mockup_audit -- --ignored --nocapture`. The mockups' source of truth is the claude.ai/design project `0b6a4165-8090-4dd0-b21c-80ffd802052d` (DesignSync MCP, auth via `/design-login`); the `docs/ui/` copies are byte-identical exports, and the `Mirrors docs/ui/davinci_tui/*.ex` doc comments cite the Elixir/Ratatouille reference tree, which now lives at `docs/ui/davinci_tui/` (`mix deps.get && mix run run.exs`; needs Elixir, which the Rust build does not). New screens land there first, then in `pi-tui`. Screens `3a`–`6d` — `/model`, `/settings`, `/thinking`, `/login`, `/hotkeys`, `/resume`, `/tree`, `/compact`, `/export`, `/graph`, `memory-status`, `governor-status`, `sec-report`, `/trust`, `/reload`, interrupt, and the `Δ` review (`/diff`, a davinci-only command) — are implemented in the Rust TUI as command sheets: each view in `pi-tui/src/davinci/views/` mirrors its `.ex` reference, `fixtures.rs` dresses every one for the mockup audit, and `davinci_session.rs` opens them with live data (settings store, model runtime, session store, git, the native extensions). Where a mockup and design.md disagree on a visual detail, the mockup wins.

## Conventions

- **Cite the TypeScript source.** Most modules open with a doc comment naming the file they mirror, e.g. `//! Project trust store matching vendor/pi/packages/coding-agent/src/core/trust-manager.ts`. Follow this for new modules; when changing behavior, read the cited TS file first and match it rather than improving on it.
- **Tests are fixture-only and never touch the network.** Behavior that would call a provider, an installer, a browser, or an update server is instead driven by `PI_*` fixture environment variables read at the call site — `PI_OFFLINE`, `PI_DISABLE_NETWORK`, `PI_SELF_UPDATE_FIXTURE`, `PI_OAUTH_TOKEN_FIXTURE`, `PI_PACKAGE_FIXTURE`, `PI_LLAMA_*`, `PI_MANAGED_INSTALLER_REPLY`, `PI_OPEN_BROWSER_DRY_RUN`, and others. Add a fixture hook rather than a live call when a new code path needs one.
- Tests are inline `#[cfg(test)] mod tests` blocks (~188 of them); there are no `tests/` directories except `pi-parity/fixtures`.
- Divergence from TS is acceptable only where the platform forces it (the `powershell` tool, the Windows `experimental` stub); document it in a comment.

## Gotchas

- **Not every file in `src/` is compiled.** Several crates have leftover source files that no `mod` declaration references: `pi-session/src/{jsonl,memory,backend,conformance}.rs`, `pi-agent/src/{loop_,permission}.rs`, most of `pi-session-sqlite/src/` (only `branch_cache` is declared), and the whole `crates/pi-core` crate, which is not a workspace member and has no dependents. Before editing something a grep turned up, check that its module is actually declared — otherwise you will be editing dead code.
- **`home_dir()` mirrors Node `os.homedir()`** (`pi-session/src/discovery.rs`): `USERPROFILE` first on Windows, `HOME` otherwise. Session dirs use the TS `--…--` cwd encoding (every `/`, `\`, `:` becomes `-`); the older Rust `--a--b` encoding is still scanned read-only for pre-existing stores. Tests that create sessions should still set `PI_CODING_AGENT_DIR` or `PI_CODING_AGENT_SESSION_DIR` so they never touch the real `~/.pi`; if `--Users--…/` directories ever reappear in the repo, they are test/session strays — delete, never commit.
- `pi-coding-agent` (~47k lines) and `pi-tui` (~39k lines) are large; prefer targeted `grep` over reading whole files.
