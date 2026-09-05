# AGENTS.md

This file provides guidance and operating standards for AI coding agents working with code in this repository.

---

## 1. What this repository is

A production-grade Rust reimplementation of the TypeScript agent CLI [`pi`](https://github.com/earendil-works/pi), pinned to vendor commit `853a80d26c90a14c1886f0ebb8ffaae133ca2185`. The goal is **product equivalence**: a user running this binary keeps the same CLI flags, `~/.davinci` (and legacy `~/.pi`) sessions, provider credentials, interactive TUI, `--print` pipelines, and `--mode rpc`.

- `vendor/davinci`: The authoritative upstream TypeScript source (~1,169 `.ts` files). **It is reference-only** — read it frequently to verify intended behavior; never edit or delete it.
- `packages/*`: Stale TypeScript stubs from early porting phases; do not use them as reference.
- `crates/*`: The active Rust implementation (13 workspace crates).
- `docs/*`: Architecture specs, phased rewrite plans, UI design mockups, and security evaluations.

Key documentation:
- Architecture & crate design: [`crates/README.md`](crates/README.md)
- Phased roadmap & design index: [`docs/README.md`](docs/README.md)
- Ecosystem integration: [`docs/ecosystem.md`](docs/ecosystem.md), [`docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`](docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md)
- Harness reliability: [`docs/superpowers/specs/2026-09-04-harness-reliability-design.md`](docs/superpowers/specs/2026-09-04-harness-reliability-design.md), [`docs/superpowers/plans/2026-09-04-harness-reliability.md`](docs/superpowers/plans/2026-09-04-harness-reliability.md)
- Self-improving learning: [`docs/learning.md`](docs/learning.md), [`docs/superpowers/specs/2026-09-03-davinci-self-improving-learning-design.md`](docs/superpowers/specs/2026-09-03-davinci-self-improving-learning-design.md)

---

## 2. Developer & Verification Commands

The toolchain is pinned to **Rust 1.83.0** (`rust-toolchain.toml`). All workspace dependencies use exact pinning (`=x.y.z`); maintain this convention when modifying dependencies.

### Common Build & Test Commands

```bash
# Build the product binary (`davinci`)
cargo build -p davinci-coding-agent

# Run workspace test suite
cargo test --workspace

# Targeted testing (fastest, saves tokens & execution time)
cargo test -p davinci-agent                       # single crate
cargo test -p davinci-agent apply_patch::tests   # single test module
cargo test -p davinci-coding-agent -- --nocapture # show stdout

# Formatting and lints
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Running the Binary

```bash
./target/debug/davinci --help
./target/debug/davinci -p "Explain src/main.rs"         # print mode: run, print, exit
./target/debug/davinci --mode json -p "run checks"      # JSON event stream
./target/debug/davinci --mode rpc                       # JSON-RPC over stdio
cargo run -p davinci-parity                             # golden-fixture parity corpora
```

---

## 3. Architecture & Crate Dependency Structure

Dependencies flow strictly bottom-up; `davinci-coding-agent` is the top-level product executable (`davinci`).

```
davinci-coding-agent (bin `davinci`) — CLI, TUI wiring, extensions, slash commands, auth UI
  ├── davinci-tui        — Terminal UI: `davinci/` (Ratatui default) & legacy chrome
  ├── davinci-agent      — Agent loop, tools, compaction, scheduler, permissions, skills
  │     ├── davinci-ai   — Providers, OAuth/auth, streaming, model catalog, cost tracking
  │     └── davinci-mcp  — Native Model Context Protocol client (stdio + HTTP SSE)
  ├── davinci-session    — JSONL session storage, cwd encoding, turn history
  ├── davinci-session-sqlite — SQLite branch cache and session indexing
  ├── davinci-protocol   — Length-prefixed CBOR wire framing & RPC types
  ├── davinci-client / davinci-server — Protocol client/server over socket or TCP
  ├── davinci-evals, davinci-telemetry
  └── davinci-parity     — Golden fixtures and differential parity testing
```

---

## 4. Key Subsystems & Core Invariants

### Agent Runtime & Tooling (`davinci-agent`)
- Built-in tools: `read, write, edit, bash, powershell, grep, find, ls, web_fetch, web_search, todo, job_output, job_kill, notebook_edit, mcp_read, agent, batch, apply_patch`.
- Read lanes vs Mutation barriers: Tool calls within a single assistant message execute via `scheduler.rs`. Read-class tools, read-only MCP tools, and `agent` overlap concurrently on up to 8 threads. `write`, `edit`, shell, and extension mutations act as barriers preserving source order.
- Exact-text edits: `edit` requires unique, non-overlapping matches against the existing file.
- Background jobs: `job_output`, `job_kill`, and `write_stdin` handle persistent child processes. Process groups are signaled with `kill -TERM -- -{pid}`.

### Harness Reliability & Guard Invariants
- **Patch Authority & Recovery** (`apply_patch.rs`): `.davinci_patch_journal.json` (and legacy `.pi_patch_journal.json`) is a reserved path. Pre-existing repository journals are rejected during standard patch application. Recovery is explicit via `recover_incomplete_journal_if_any`, validating all targets beforehand and retaining the journal on failure. Journal creation uses exclusive flags (`create_new(true)`) and flushes (`sync_all()`).
- **Graph & Extension Guard Boundaries** (`controller.rs`, `worker_hooks.rs`, `extension_host.rs`): Global extra tools are advertised and allowed solely to `Writer` roles; non-writers enforce baseline tools even if configured. Shell aliases (`exec_command`) enforce command execution policies. Poisoned native mutexes recover to execute pre-tool guards.
- **Provider Context Budget** (`Agent::estimated_context_tokens`, `main.rs`): Context token estimates account for `system_prompt` length and tool schema overhead; host overhead is cached once per prompt configuration, driving earlier pruning/compaction before model calls.
- **Token Governor Visibility & Freshness** (`token_governor.rs`, `extension_host.rs`, `vector_memory.rs`): Agent pruning/compaction calls `native_context_pruned()` to reset read and search ledgers so pruned content can be retrieved again. Weak Git HEAD/status hashes are omitted; searches execute fresh on unknown freshness.
- **Interruptible Retries & Telemetry** (`turn.rs`, `stats.rs`): Exponential backoff sleeps in 25 ms slices checking abort signals; `RunStats::provider_retries` counts actual additional attempts; model wall time includes failed attempts; older stats JSON compatibility is preserved.
- **Evaluation Gate Integrity** (`codex_eval.rs`): Release gates enforce `median_tools <= 10.0` no-worsening to detect tool-call explosions alongside wall time, responses, and token usage.

### Ecosystem Integration & Graph Execution (`native_extensions/graph/`, `ecosystem/`)
- Deterministic orchestration: Pipelines follow `classify → investigate → plan → implement → verify → review`. Models run only inside isolated `--print` worker child processes; only schema-validated JSON artifacts cross node boundaries.
- DAG Invariants: Strictly forbids cycles, review bypass, missing verification, and concurrent mutation-capable writers. Enforces `run_deadline_ms`, replay fingerprints, graph-owned mutation deltas, and line-bounded review chunk coverage.
- Governor recovery: Graph workers retain `retrieve_output` in their allowlist whenever compressible tools exist.
- Cache affinity: Workers derive deterministic cache keys (`derive_worker_cache_key`) passed via `PI_GRAPH_CACHE_KEY`.
- Bounded context packets: Context packets injected into workers are capped at 2,500 aggregate tokens (<= 1,200 memory tokens, <= 1,000 skill tokens). Duplicate child memory injection is suppressed via `PI_GRAPH_SUPPRESS_MEMORY_INJECT=1`.

### Self-Improving Learning System (`native_extensions/learning/`)
- Asynchronous background reviewer: Settled turns trigger the background reviewer thread (`reviewer.rs`) without blocking foreground execution.
- Review gating: `should_review_evidence` filters low-signal read-only turns while preserving full vector memory indexing.
- Exact version provenance: Injected skills carry immutable `SkillVersionRef (name, version, content_hash)`. Graph execution outcomes (`VerifiedSuccess`, `VerifiedFailure`, `Neutral`) update only the specific targeted version ledger record.
- Procedural skills: Saved as `SKILL.md` under `.davinci/skills/<name>/` (or legacy `.pi/skills/<name>/`) or `~/.davinci/agent/skills/<name>/` (global) with structured YAML frontmatter.

### Permissions & Security (`davinci-agent/src/permission.rs`)
- Modes: `read-only | ask | edits | auto` (`--permission-mode`, `/permissions <mode>`).
- Rules: Defined under `permissions.allow` and `permissions.deny` in `~/.davinci/agent/settings.json` (or `~/.pi/...`) or `.davinci/settings.json` (or `.pi/...`, trusted projects only). Deny rules always win.
- Least-privilege child workers: `--print` worker subprocesses run with `--permission-mode auto` gated by strict tool allowlists and command inspection.

---

## 5. Coding & Development Conventions

1. **Reference the TypeScript Source Constantly**:
   When implementing or modifying behavior, check the corresponding file in `vendor/davinci/` first and preserve parity. Open new modules with a doc comment referencing the upstream TypeScript file (e.g. `//! Project trust store matching vendor/davinci/...`).

2. **Offline & Fixture-Driven Testing**:
   **Never make live network or provider calls in tests.** All external operations (model completion, updates, OAuth, browser, MCP, subprocesses) must be controlled via `DAVINCI_*` or `PI_*` fixture environment variables (`DAVINCI_OFFLINE`, `PI_OFFLINE`, `PI_DISABLE_NETWORK`, `PI_SUBAGENT_FIXTURE`, `PI_HOOKS_DRY_RUN`, `PI_LEARNING_REVIEW_FIXTURE`, etc.).

3. **Inline Test Modules**:
   Place tests in inline `#[cfg(test)] mod tests` blocks within each source file. Do not create external `tests/` directories (the only exception is `crates/davinci-parity/fixtures`).

4. **Preserve Upstream Prompt Constants**:
   Compaction (`compaction.rs`), branch summarization (`branch.rs`), and system prompts reproduce upstream TypeScript strings verbatim. These are part of the parity contract; do not alter or reword them.

5. **Windows & Shell Gotchas**:
   - `core.hooksPath`: If global Git hooks (`.codex/git-hooks`) fail on Windows with `execvpe(/bin/bash) failed`, use `git commit --no-verify` and `git push --no-verify`.
   - Line Endings: Normalize line endings with CRLF/LF awareness. `apply_patch.rs` explicitly preserves CRLF line endings on files that use them.
   - Stray Test Directories: Sessions create cwd-encoded directories (`--C--Users-...--/`). Never commit stray session test directories to git.
   - Untracked `plugins/`: The `plugins/` folder in the repository root is pre-existing and intentionally untracked; do not stage or commit it.
