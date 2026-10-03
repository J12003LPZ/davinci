# Review Fixes: Index and Execution Guide

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement these plans task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Read this index first; every phase file assumes it.

**Goal:** Fix every problem found by the six-part code review of `main` at `4d55d60a` (2026-09-23): 6 critical/high security issues, 7 data-loss issues, about 40 correctness bugs, the Windows-specific failures, and the structural causes behind them.

**Architecture:** One new leaf crate (`crates/davinci-sys`) provides crash-safe writes, two lock types, torn-tail repair and a supervised child-process runner; one new module (`project_config`) decides where project config lives. Every later phase builds on those instead of re-implementing them per call site, which is how most of the reviewed bugs arose.

**Tech Stack:** Rust 1.83 (edition 2021), existing pinned workspace dependencies, GitHub Actions.

---

## Global constraints

Every task in every phase implicitly includes these.

- **Toolchain:** `rust-version = "1.83"`, edition 2021 (`Cargo.toml` `[workspace.package]`). No API newer than 1.83 (for example `std::fs::File::lock` is 1.89: do not use it).
- **Dependencies:** workspace dependencies are pinned with `=` versions. No new third-party crate without writing the reason in the commit message; this plan needs none. `davinci-sys` is a new workspace member, not a third-party crate.
- **Platforms:** every change must build and pass tests on Windows and Linux. Tests that are platform-specific carry `#[cfg(unix)]` / `#[cfg(windows)]`; a Unix-only fix still needs its Windows counterpart considered in the task.
- **Tests:** test first (write it, watch it fail for the stated reason, then implement). Every behavior change ships its test in the same commit. Performance tasks state a numeric budget before any code changes and record before/after numbers.
- **Checks per task:** `cargo fmt --all -- --check` and `cargo clippy -p <touched crates> --all-targets -- -D warnings` pass before each commit (CI enforces strict warnings; see the upstream `fix(ci)` commits).
- **Branching (Julien's CLAUDE.md):** each executing session works in its own worktree and branch, and ships through a PR that a human merges. Never commit on `main`. Parallel builders that touch overlapping files use `isolation: "worktree"`.
- **Writing style for code comments, docs and commit messages:** direct, no em dashes, no filler words (see Julien's CLAUDE.md "How Julien wants to be talked to").
- **Scope discipline:** a task changes only what it names. Refactors are Phase 11 and run last.

## State before starting (Phase 0)

These were verified on 2026-09-23 and must be redone right before execution:

| Check | Result on 2026-09-23 |
|---|---|
| Free space on C: | 205 GB after cleanup (was 0 bytes; builds failed with `os error 112`) |
| `cargo test --workspace --no-fail-fast` on local `main` (`4d55d60a`) | **5211 passed, 0 failed** |
| `cargo clippy --workspace --all-targets` on local `main` | exit 0, 31 warnings |
| `cargo fmt --all -- --check` on local `main` | failed on 8 files |
| `origin/main` (`a9887dbe`, 24 commits ahead of local `main`) | fmt passes; the clippy warnings and fmt diffs are fixed there |

- [ ] **0.1 Sync.** Update local `main` to `origin/main` (`git fetch origin && git switch main && git merge --ff-only origin/main` in the shared checkout, done by Julien or with his go-ahead). Every phase branches from `origin/main`.
- [ ] **0.2 Re-run the baseline** on `origin/main` and record it in the first PR: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --locked --no-fail-fast 2>&1 | tee baseline.log`, and the passed/failed totals (`grep -oE "test result: [a-z]+\. [0-9]+ passed; [0-9]+ failed" baseline.log | awk '{p+=$4; f+=$6} END {print p, f}'`).
- [ ] **0.3 Keep disk space.** The C: drive filled up with Rust build output in 20+ worktrees (about 245 GB of `target/` directories on 2026-09-23). Before long builds, check free space; `cargo clean` in worktrees whose branches have merged, and `git worktree prune`. These are destructive: confirm with Julien first.
- [ ] **0.4 Uncommitted deletion of `CLAUDE.md`** in the shared checkout (`git status` showed `D CLAUDE.md`; the file is still in HEAD). Ask Julien whether that deletion is intended before any session commits from that checkout.

## Phases

| Phase | File | Tasks | What it fixes |
|---|---|---|---|
| 1 | `01-foundation.md` | 1.1-1.6 | `davinci-sys` (atomic write, `LockFile`, `ExclusiveFileLock`, torn-tail repair, `run_bounded`, `resolve_program`, `kill_tree`) and `project_config` |
| 2 | `02-trust-and-security.md` | 2.1-2.15 | untrusted repos running code, trust fail-open, secrets in URLs/logs/errors, MCP env leak, graph results leaking files, OAuth callback, test backdoors |
| 3 | `03-data-durability.md` | 3.1-3.10 | unopenable sessions, v3 session corruption, two writers, settings/auth/memory wipe, git update deleting installs, SQLite |
| 4 | `04-provider-correctness.md` | 4.1-4.11 | Anthropic auth/thinking, truncated streams, timeouts, retry classification, tool-call parsing, Codex WS, stub providers, decoding cost |
| 5 | `05-agent-runtime.md` | 5.1-5.17 | panics, accepted plan blocking writes, stuck streaming state, exponential glob, subagent races/leaks, retries, run cap, verification detection, bounded jobs/ledger |
| 6 | `06-coding-agent-host.md` | 6.1-6.14 | RPC robustness and abort, hooks, JS extensions, installed packages, Windows paths and programs, editor, startup ordering |
| 7 | `07-tui-and-voice.md` | 7.1-7.15 | terminal ownership, escape injection, transcript wipe, editor width/graphemes, redraw cost, voice accuracy and idle behavior |
| 8 | `08-native-extensions.md` | 8.1-8.16 | graph checkpoint size, pruning, cancellation, process groups, LSP restarts, security scan, learning ledgers |
| 9 | `09-mcp.md` | 9.1-9.5 | stdout JSON logs, ping, HTTP SSE, chatty servers, tool name routing |
| 10 | `10-sessions-ipc-evals-ci.md` | 10.1-10.10 | eval gates that cannot fail, stub subsystems, discovery cost, Windows CI, repo hygiene |
| 11 | `11-structure-and-dead-code.md` | 11.1-11.10 | double compilation, giant files, legacy TUI, dead code, budget/watchdog, event pump, tool classification |

## Execution order

1. **Phase 0**, then **Phase 1** (everything depends on it).
2. **Phase 2** next: it closes the paths by which opening a repository can run code.
3. Then Phases 3-10. File overlaps decide what can run in parallel:
   - `crates/davinci-coding-agent/src/main.rs`: 2.1, 2.4, 2.14, 6.1, 6.2, 6.14, 7.1, 7.9, 7.12. Run these sequentially.
   - `crates/davinci-coding-agent/src/packages.rs`: 2.3, 3.4, 3.7, 6.5, 6.6, 6.7. Sequential, in that order.
   - `crates/davinci-coding-agent/src/settings.rs`: 2.1, 3.4, 6.5, 6.14. Sequential.
   - `crates/davinci-agent/src/turn.rs`: 4.7, 5.2, 5.4, 5.6, 5.8, 5.9, 5.10. Sequential.
   - `crates/davinci-ai/src/stream.rs`: 2.10, 4.1, 4.2, 4.4, 4.9, 4.10, 4.11. Sequential.
   - Independent lanes that can run beside the above in their own worktrees: **Phase 7** (TUI/voice), **Phase 8** (native extensions), **Phase 9** (MCP), **Phase 10** except 10.9.
4. **Phase 11** last (it moves code the other phases edit).

Suggested PR granularity: one PR per phase for Phases 1, 9 and 10; one PR per 3-5 related tasks for the larger phases (for example "2.1-2.5 trust", "2.6-2.9 permissions", "2.10-2.15 secrets"). Each PR lists the tasks it closes and the verification it ran.

## Decisions for Julien

Each of these changes documented behavior. The phase file implements the **recommended** option; confirm or pick the alternative before that task starts.

| ID | Task | Question | Recommended |
|---|---|---|---|
| D1 | 4.1 | Anthropic OAuth only works in the TS code by impersonating Claude Code (user agent, `x-app: cli`, system line, tool renaming, Claude Code's client id). This plan does not port that. | Require API keys for Anthropic; say so in `/login anthropic`; fix the `x-api-key` conflict |
| D2 | 4.9 | Google, Vertex and Bedrock request builders flatten tool calls into text. | Refuse tool use on those providers until native clients exist (X1) |
| D3 | 5.2 | Shell commands under an accepted plan's hard contract: the host has no sandbox. | Send them to the normal permission policy; keep network/redirection/substitution rules |
| D4 | 10.4 | SQLite session backend writes empty sessions. | Remove the setting, keep the crate |
| D5 | 11.5 | Budget ledger and progress watchdog are wired only in tests. | Wire the watchdog, delete the ledger |
| D6 | 10.3 | `davinci-parity` cannot fail. | Delete it |
| D7 | 10.5 | Experimental server/client are stubs, Unix-only. | Gate behind a cargo feature; fix socket dir permissions |
| D8 | 10.10 | `crates/davinci-core` and `packages/` are referenced by nothing. | Delete |
| D9 | 11.3 | Legacy TUI (~34k lines) runs only with `--legacy-tui`. | Gate behind a cargo feature |
| D10 | 11.9 | `davinci update` runs self-update and can report fake updates. | `update` = extensions only; `--self` refuses unknown install methods |
| D11 | 11.4 | Plan-mode research subagents are unreachable. | Make read-only `agent` calls allowed in Plan mode |
| D12 | 2.6 | Plan mode fetches URLs without asking. | Ask for `web_fetch`/`visual_snapshot`; keep `web_search` allowed |
| D13 | 2.7 | Auto mode edits `.cargo/config*` and `rust-toolchain*` without asking. | Protect them; document that Auto mode runs model-written tests |
| D14 | 2.12 | MCP stdio servers stop inheriting the full environment (breaking for configs that relied on it). | Do it, with `${VAR}` expansion and a CHANGELOG entry |

## Deferred, with reasons

Not in this plan; each needs its own plan or depends on a decision above.

| ID | Item | Why deferred |
|---|---|---|
| X1 | Native Gemini, Vertex and Bedrock (SigV4) clients; images in requests | A feature project, not a fix; D2 makes the current stubs fail safely meanwhile |
| X2 | The ~80 remaining `PI_*_REPLY` / `PI_*_CMD` env stubs in production code | They require control of the process environment, which already means control of the process; Task 2.15 covers the credential-affecting ones |
| X3 | Codex WebSocket abort latency (abort checked only between frames) | Needs a read-slice loop in the TLS stream wrapper; low impact after Task 4.8 |
| X4 | Sweeper for worktrees of subagents that ran and failed | Retention is intentional (partial work); needs a retention policy decision |
| X6 | Slow synchronous work on the UI thread (graph sheet refresh, `sec-report`, Memoria recall, clipboard image) | Needs moving to background workers with result channels; after Task 11.6's single pump |
| X7 | Typed `ProviderError` end to end (replace `Result<_, String>` across `davinci-ai` and the agent retry path) | Task 4.5 adds the phase that matters; the full type change is a refactor across two crates |
| X9 | One shared decoder core for the three stream decoders | Refactor; Tasks 4.3, 4.6, 4.7, 4.11 fix the behaviors individually |

## Finding-to-task map

Every numbered finding from the six review reports, and where it is fixed. "S" = the report's structural suggestions.

**davinci-ai and MCP:** 1→4.1 · 2→4.2 · 3→4.3 · 4→4.4 · 5→2.10 · 6→3.5 · 7→4.5 · 8→4.5 · 9→4.6 · 10→4.7 · 11→4.11 · 12→4.8, X3 · 13→4.8 · 14→9.1 · 15→2.12 · 16→9.2 · 17→9.3 · 18→9.4 · 19→9.5 · 20→2.14 · 21→2.14 · 22→2.15, X2 · 23→2.11 · 24→4.9, X1 · 25→4.10 · dead code→11.4 · S1→4.5, X7 · S2→4.4, 2.10 · S3→X9 · S4→4.2, X1 · S5→9.4, 3.5
**Found while planning:** pasted OAuth code exchanged with a fresh PKCE verifier (`main.rs:6686-6691`) → 2.14

**Sessions, IPC, evals:** 1→3.1 · 2→3.2 · 3→3.3 · 4→10.1 · 5→10.2 · 6→10.3 · 7→3.10 · 8→3.10 · 9→3.10 · 10→10.4 · 11→10.5 · 12→10.5 · 13→10.5 · 14→10.5 · 15→10.5 · 16→10.8 · 17→10.5 · 18→10.8 · 19→3.8 · 20→10.6 · 21→10.7 · 22→10.10 · 23→10.9 · 24→10.10 · S1→3.1-3.3 · S2→10.4 · S3→10.5 · S4→10.1-10.3 · S5→10.8, 10.10, 11.4 · S6→10.9

**Native extensions:** 1→8.1 · 2→2.13 · 3→8.2, 3.9 · 4→8.3 · 5→8.4 · 6→8.5 · 7→3.6 · 8→8.7 · 9→8.8 · 10→8.6 · 11→8.1, 8.6 · 12→8.15 · 13→8.12 · 14→8.11 · 15→8.9 · 16→8.10 · 17→3.9 · 18→3.9 · 19→8.13 · 20→8.14 · 21→8.6 · 22→8.5 · 23→8.12 · 24→2.15, 11.4 · 25→8.16 · S (split files)→11.2 · S (subprocess helper)→1.3 · S (atomic write)→1.1, 3.9 · S (shared handles)→8.7 · S (settings once)→11.8 · S (path resolver)→1.4, 11.8

**Coding-agent core:** 1→2.1 · 2→2.1 · 3→2.3 · 4→2.4 · 5→2.5 · 6→3.4 · 7→3.4, 2.2 · 8→6.5 · 9→6.6, 3.7 · 10→6.1 · 11→6.2 · 12→6.4 · 13→6.4 · 14→3.7 · 15→6.3 · 16→6.7 · 17→6.6 · 18→6.8 · 19→6.9 · 20→6.10 · 21→6.11 · 22→6.4, 11.10 · 23→6.12 · 24→6.13 · 25→11.4, 11.9, 10.5, 11.5 · also-noted→6.14 · S (compile once)→11.1 · S (split)→11.2 · S (config resolver)→1.4 · S (subprocess)→1.3 · S (settings persistence)→3.4 · S (JS pool)→11.10

**davinci-agent:** 1→5.1 · 2→5.2 · 3→5.3 · 4→2.8 · 5→5.4 · 6→5.5 · 7→5.6 · 8→5.7 · 9→5.7 · 10→5.8 · 11→11.5 · 12→11.5 · 13→5.9 · 14→2.6 · 15→2.7 · 16→5.10 · 17→5.11 · 18→5.12 · 19→5.13 · 20→5.14 · 21→5.15 · 22→5.16 · 23→2.9 · 24→5.17 · 25→11.4 · S (split)→11.2 · S (classification)→11.7 · S (matchers)→5.5, 5.16 · S (test-only subsystems)→11.5, 5.2 · S (error hygiene)→5.4 · S (run_tool)→5.7

**TUI and voice:** 1→7.1 · 2→7.2 · 3→7.3 · 4→7.4, 7.11 · 5→7.6 · 6→7.5 · 7→7.1 · 8→7.7 · 9→7.8 · 10→7.10 · 11→7.9 · 12→7.9 · 13→X6 · 14→7.9 · 15→7.12 · 16→7.14 · 17→7.13 · 18→7.15 · 19→7.14 · 20 (no issue) · 21→11.4, 11.3 · S1→11.6 · S2→7.11 · S3→7.1, 7.3 · S4→11.3 · S5→11.2

**Environment:** full disk→0.3 · fmt failure on local `main`→fixed on `origin/main`, 0.1 · uncommitted `CLAUDE.md` deletion→0.4 · `.gitconfig-gh`→10.10

## Manual verification checklists

Automated tests cover each task; these confirm the user-visible result once per phase. Record results (and the OS used) in the phase's PR.

**Trust (after Phase 2):**
- [ ] Clone a scratch repo containing only `.pi/mcp.json` with a server whose command writes a marker file. Open davinci there with default settings: you are asked about trust and no marker file appears.
- [ ] Same with an empty `.davinci/` and `.pi/hooks.json` (`sessionStart` hook writing a marker).
- [ ] With `PI_AI_TRACE=stderr`, send one Gemini request: the trace shows `?<redacted>` and no key.
- [ ] `cargo build --release` then `strings` on the binary: no credential fixture variable names (Task 2.15 Step 5).

**Durability (after Phase 3):**
- [ ] Append a half line (`{"kind":"ent`) to a real session file; `davinci -c` opens it and the next message saves.
- [ ] Open the same session in two terminals; the second one's first message reports "open in another davinci process".
- [ ] Put a comment in `~/.pi/agent/settings.json`, run `davinci install <local dir>`: the command refuses and the file is unchanged.

**Providers (after Phase 4):**
- [ ] Anthropic model with thinking on and a tool call: the tool loop completes (no 400).
- [ ] Kill the network mid-answer (disable the adapter): the turn ends with "stream ended before a terminal response event" within the idle timeout, not a hang, and the partial text is visible.

**Agent loop (after Phase 5):**
- [ ] `/plan`, write a small plan, `/plan accept auto`: the agent can edit files in scope and run `cargo test`.
- [ ] Type `/review` followed by a full-width space (U+3000) and text: no crash.

**Host (after Phase 6):**
- [ ] Windows: `davinci install npm:<small package>` succeeds without `npmCommand`.
- [ ] RPC: send `prompt`, then `abort` 300 ms later: the turn ends within a few seconds.
- [ ] An extension that calls `console.log` in a command handler still works.

**Terminal (after Phase 7):**
- [ ] `/login openai-codex`, cancel, return: input does not echo to the shell; Ctrl+C does not exit.
- [ ] `!printf 'a\033]52;c;SGVsbG8=\007b\n'` (Unix) shows `ab` and leaves the clipboard unchanged.
- [ ] Move the mouse continuously over an idle davinci window for 10 s: CPU stays under 5% of one core.

**Graph (after Phase 8):**
- [ ] Run a graph on a repo of about 20 MB: `state.json` stays under 1 MB; `.davinci/graph/blobs` holds the file snapshots once.
- [ ] Start a graph through the model and press Esc: the run stops and its test processes are gone (`ps`/Task Manager).

**MCP (after Phase 9):**
- [ ] A stdio server that logs with pino to stdout stays connected and its tools work.
- [ ] A server with a 70-character tool name: every turn still succeeds; the tool is callable.

## Plan self-review (done while writing)

- **Coverage:** every numbered finding in the six reports maps to a task or a deferred item above.
- **Placeholders:** code steps contain code. Where a step names a helper that must match existing test scaffolding (for example a fixture builder), it says which existing test to copy from, because those helpers' exact names were not all read during planning; the implementer confirms the name before writing the test.
- **Consistency:** shared names used across phases are defined once: `davinci_sys::fs::{atomic_write, atomic_write_private, sync_parent, truncate_torn_tail}`, `davinci_sys::lock::{LockFile, ExclusiveFileLock, DEFAULT_STALE_AFTER, lock_path_for}`, `davinci_sys::process::{run_bounded, RunLimits, BoundedOutput, resolve_program, resolve_program_in, set_own_process_group, kill_tree}`, `project_config::{candidates, resolve, all, any_exists}` (Phase 1); `davinci_ai::INVALID_ARGUMENTS_KEY` (4.7); `RemoteQueue` (6.2); `graph::blobs::{dir, put, get}` (8.1).
- **Line numbers** refer to `origin/main` at `a9887dbe` for the files it changed since the review (turn.rs, graph worker/bridge/controller/mod, runtime_host, several tests) and to `4d55d60a` otherwise; they drift as tasks land. Search for the quoted code when a line number no longer matches.
