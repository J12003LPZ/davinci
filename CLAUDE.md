# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

A Rust reimplementation of the TypeScript agent CLI [`pi`](https://github.com/earendil-works/pi), pinned to vendor commit `853a80d26c90a14c1886f0ebb8ffaae133ca2185`. The goal is *product equivalence*: a user who runs TypeScript `pi` installs this binary and keeps the same flags, `~/.davinci` (and legacy `~/.pi`) sessions, provider credentials, interactive TUI, `--print`, and `--mode rpc`.

`vendor/davinci` holds the authoritative TypeScript source (~1,169 `.ts` files). It is reference-only — read it constantly, never delete or edit it. `packages/*` holds a handful of stale TypeScript stubs from the early phases; they are not the reference.

Phase plans describing the rewrite are in `docs/superpowers/plans/`. The ecosystem integration roadmap and its design plus execution plans are in `docs/superpowers/plans/2026-09-04-davinci-ecosystem-integration-roadmap.md`, `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`, and the dated ecosystem A–D plan files. The harness reliability design and implementation plan are in `docs/superpowers/specs/2026-09-04-harness-reliability-design.md` and `docs/superpowers/plans/2026-09-04-harness-reliability.md`. Comprehensive documentation index is in `docs/README.md` and crate architecture is in `crates/README.md`.

## Commands

The current Davinci product version is **1.0.0**, defined by `[workspace.package].version` in `Cargo.toml` and inherited by the workspace crates. Keep `Cargo.lock` synchronized when changing it. The CLI `--version` and native welcome banner use `CARGO_PKG_VERSION`. Provider and JavaScript-extension compatibility identities remain pinned to upstream `0.84.4`; do not change those wire contracts when bumping the product version.

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

On Windows, build and launch the local executable with `rtk cargo build -p davinci-coding-agent --offline` and `.\target\debug\davinci.exe`. A build does not replace the installed `%USERPROFILE%\.cargo\bin\davinci.exe` resolved by bare `davinci`. Use RTK for shell commands and Headroom for large outputs; honor explicit user restrictions on subagents and test execution.

### Context VM / state folding

The Context VM is derived state, not a replacement for the session history:

```text
session JSONL/events = authoritative WAL
typed pages           = immutable derived state
artifacts             = exact evidence
ContextImage          = bounded provider working set
provider cache        = backend optimization only
```

Set `DAVINCI_CONTEXT_VM=off|shadow|active` (default `off`). `off` preserves
legacy pruning and compaction. `shadow` compiles and measures the VM while
returning the exact legacy provider projection. `active` uses the VM image and
derived folds while retaining every authoritative `Agent::messages` entry and
session event. Hidden thinking is never projected into pages or retrieval.

Active-mode failures rebuild from the authoritative session branch; a missing
mandatory policy blocks the request. `retrieve_context` is a bounded,
read-only exact-recovery path for Context VM pages and sources. The existing
`retrieve_output` contract and Token Governor output store remain unchanged.
Context VM cache affinity is only an optimization: disabling provider caching
must not change the logical ContextImage.

For debugging, use `shadow`, inspect the prepared context manifest and Context
VM status (`mode`, `epoch`, checkpoint, delta/episode/hot counts, fold reason,
page-fault hits/misses, and prefix digest), compare missing user/tool refs, use
`retrieve_context`, then replay the session branch and compare legacy versus VM
provider messages. Normal non-fold turns perform no extra model call for
context maintenance.

Focused offline release-gate commands are:

```powershell
rtk cargo test -p davinci-agent --test context_vm_types --offline --locked
rtk cargo test -p davinci-agent --test context_vm_cache --offline --locked
rtk cargo test -p davinci-agent --test context_vm_replay --offline --locked
rtk cargo test -p davinci-agent --test context_vm_shadow --offline --locked
rtk cargo test -p davinci-agent --test context_vm_retrieval --offline --locked
rtk cargo test -p davinci-agent --test context_vm_active --offline --locked
rtk cargo test -p davinci-evals context_vm_ --offline --locked
```

### Deliver changes to the executable the user actually runs

For user-facing executable changes, complete the local build and installed update as part of delivery unless the user explicitly requests source-only work. Do not stop at changing source or rebuilding `target/debug`.

1. Trace the user's normal launch command to its actual executable, following aliases, wrappers, and PATH resolution. On PowerShell, start with `Get-Command davinci -All`; do not assume the installed path.
2. Build the updated app and run the minimum relevant validation. For the installed Rust CLI, use `rtk cargo build -p davinci-coding-agent --release --offline`.
3. Preserve a backup of the resolved installed executable, then replace it with the successful build. Preserve user settings, credentials, sessions, and unrelated changes. Do not terminate an active session to unlock its executable; report the blocker if replacement cannot proceed.
4. Resolve the normal command again, verify the installed binary matches the build using a cryptographic hash, and smoke-check that installed executable. A matching version string alone does not prove that it contains the changes.
5. Report the updated executable path and validation performed. Tell the user to restart existing sessions, which continue running their original executable. For UI changes, distinguish build and launch checks from live visual verification.

Do not claim the user's app is updated while their normal command still launches an older copy. This delivery rule does not apply to documentation-only or report-only tasks and does not authorize publishing, deployment, or changes to third-party installations.

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

**Ecosystem integration plan**: The roadmap coordinates four bounded workstreams: graph execution hardening, runtime integration with the token governor and vector memory, learning/security feedback, and proof plus CI hygiene. Treat the design document as the contract and the A–D plans as the execution order. Preserve worker isolation, deterministic/offline verification, explicit provenance, bounded context and resource budgets, and fail-closed security approval as cross-cutting acceptance criteria.

**Entrypoint dispatch** (`crates/davinci-coding-agent/src/main.rs`, ~7k lines): `run()` picks a mode — `run_rpc` (`--mode rpc`), `run_print` (`--print` / `--mode json` / non-TTY stdin or stdout), otherwise `run_interactive`, which opens the davinci shell (`davinci_interactive::run`) unless `--legacy-tui` or `PI_DAVINCI=0` asks for the old chrome. Unix-only `experimental` subcommands (`server`, `client`) are stubbed out on Windows by an inline `mod experimental` in `main.rs`.

**Provider streaming** (`davinci-ai`): every request goes through `live_complete_streaming_with_sink` (`stream.rs`), which reads the SSE body on a reader thread and hands each decoded event to a sink as it arrives; `StreamOptions::abort_signal` is polled between frames. Wire formats are decoded by `stream_decoder.rs` (Responses/Codex), `stream_decoder_completions.rs` and `stream_decoder_anthropic.rs`; APIs without a decoder are requested without `stream: true` and their events synthesised. Responses tool-call ids are stored as `call_id|item_id` and only the `call_id` half is replayed. `PI_AI_TRACE=1` (or a file path) logs every request, frame and failure — reach for it before reading code when a turn misbehaves.

**Agent runtime** (`davinci-agent`): built-in tools are `read, write, edit, bash, powershell, grep, find, ls, web_fetch, web_search, todo, propose_plan, job_output, job_kill, notebook_edit, mcp_read, agent, batch, apply_patch`. Tools from phase 3 onward have no TypeScript counterpart (phase 3 spec `docs/superpowers/specs/2026-09-01-tools-that-compete-design.md`; phase 4 MCP spec `docs/superpowers/specs/2026-09-01-native-mcp-design.md`; phase 5 spec `docs/superpowers/specs/2026-09-01-plan-and-subagents-design.md`). MCP servers from `~/.davinci/agent/mcp.json` (with legacy `~/.pi/agent/mcp.json` support, plus trusted project configuration) become agent tools named `mcp__<server>__<tool>`; `mcp_read` reads a listed resource. Plan Mode is the authoritative `read-only` permission state. `/plan` enters it and manages a session-owned, revision-checked `LivingPlan`; `/act` leaves it without implicitly approving that plan. `propose_plan` is read-only and cannot approve its own output. Evidence fingerprints, unresolved questions, decision state, and explicit user approval gate the handoff to execution. See `docs/permission-modes.md`. `agent` starts a scoped worker whose permissions cannot exceed its parent's (depth 1; `PI_SUBAGENT_FIXTURE` in tests). Extensions inject behavior through `PreToolHook` / `PostToolHook` / custom tool functions. Compaction (`compaction.rs`) and branch summarization (`branch.rs`) reproduce the TS prompts verbatim — the prompt string constants are part of the parity contract, do not reword them.

**Tool scheduling and context budget** (spec `docs/superpowers/specs/2026-09-02-harness-throughput-design.md`): the calls of one assistant message run through `scheduler.rs` in lanes — read-class tools, read-only MCP tools and `agent` overlap on up to 8 threads, while `write`/`edit`/shell/extension tools are barriers that keep source order — and `Agent::new` defaults to `ToolExecutionMode::Parallel` like TS. `turn.rs` is three stages (prepare on the loop thread with the permission gate, run on `&self`, finalize in source order). `batch` runs up to 16 operations behind one tool result (each gated like a direct call; 12 KB/64 KB visible caps, overflow to the evidence store `~/.pi/agent/evidence/`, no nesting); `agent { tasks: [...] }` fans out up to 8 workers. `pruning.rs` replaces old large tool results with a placeholder in the provider view once the estimate passes 50% of the window (history and the session file keep every byte; compaction is unchanged). `RunStats` (`stats.rs`, `Agent::run_stats()`) counts turns, batch widths, wall time, peak context and prunings; it is `runtime` in `get_session_stats` and a block in `/status`. `TOOL_USE_STRATEGY` in `lib.rs` is the prompt's half of the bargain and rides on the default and worker prompts.

**Tool permissions** (`davinci-agent/src/permission.rs`, `davinci-agent/src/permission_risk.rs`, `davinci-coding-agent/src/permissions.rs`; no TS counterpart, full operator guide in `docs/permission-modes.md`): every tool call passes a gate in `Agent::execute_one` after the extension `tool_call` hook. The five user-facing modes are **Manual** (`ask`, default), **Accept Edits** (`edits`), **Plan Mode** (`read-only`), **Auto Mode** (`auto`), and **Always Approve** (`always-approve`). At an idle composer, Shift+Tab cycles them in that order; `/permissions [mode]` inspects or selects one. Always Approve bypasses harness prompts, not explicit denies, filesystem boundaries, OS restrictions, or missing credentials. Auto approves ordinary workspace edits and a conservative set of literal local checks; ambiguous expansion, network/destructive behavior, sensitive paths, and wider scope still ask. Plan Mode denies implementation mutations before saved/session allow rules. Rules are `tool` or `tool(glob)` under `permissions.allow` / `permissions.deny` in user settings and trusted project settings; deny rules always win. Every compound patch target is classified independently, including protected paths, credentials, deletes, outside-workspace paths, Windows aliases, and symlink escapes. MCP `readOnlyHint` remains a server declaration, not proof of remote behavior. An `Ask` goes to `Agent.approver`: davinci opens the `LICENTIA · PERMISSION` panel mid-turn, RPC emits a `select` UI request, the legacy chrome uses its confirm dialog, and `--print` fails closed with a message naming the flag and rule.

**Extensions** are two-tier (`extension_host.rs`): JavaScript extensions run in a Node subprocess driven by the embedded `extension_runner.js` (`js_host.rs`, only when Node is present), while the bundled pi extensions have been ported to native Rust under `src/native_extensions/` (`vector_memory`, `token_governor`, `security_scan`, `graph`, `learning`), exposed via `NATIVE_TOOLS` / `NATIVE_COMMANDS`. The token governor digests large outputs of shell/search tools only (`LOSSLESS_TOOLS` — `read`, `edit`, `write`, `batch`, `agent`… stay verbatim), names the `retrieve_output` id inside the digest because tool `details` never reach the model, and replaces a byte-identical repeat `read` with a marker only within `dedupe_window` (6) tool calls — below pruning's `keep_recent` (8), so a pruned twin is served again; stored outputs live under `~/.pi/agent/token-governor/outputs/<session>/` and other sessions' are swept after 14 days. Vector memory indexes only `user`/`assistant` messages (tool output is transient), dedupes by content hash, honours `automaticRetrieval`, and turns dense retrieval off for two minutes after an Ollama failure so a dead host costs one timeout per window. Graph workers are `--print` children spawned with `--permission-mode auto` (nobody can answer a prompt in a child; the per-role `--tools` allowlist and `worker_hooks` bash policy are their gate), their `graph_submit` tool carries the artifact's JSON schema and the same contract goes into the worker's system prompt (`validate.rs::artifact_schema` / `artifact_contract` — keep them in step with the validators); the shell policy checks every `&&`/`;`/`|` segment, verification with no command that ran is never a pass, and `graph_run` executes without holding the native-host mutex. Hardened execution invariants guarantee: (1) explicit persisted DAG topology with ready-frontier progression; (2) active run deadlines terminating runaway workers; (3) deterministic replay fingerprints rejecting incompatible node reuse; (4) graph-owned mutation deltas isolating graph changes from pre-existing uncommitted user edits; and (5) complete review coverage requiring every diff chunk to be reviewed before approval. `/graph <goal>` starts the graph, while bare `/graph` opens the active/latest run and automatically continues a stopped run with the same run id, cumulative counters, and persisted activity. `graph-status`, `graph-resume`, `graph-view`, and `graph-abort` remain internal lifecycle operations for compatibility and live sheet refresh; do not expose them as separate slash commands. This interaction resembles goal continuation only at the command surface—the graph remains the existing DAG controller and must not inherit `/goal` semantics.

**Ecosystem Runtime Integration** (`crates/davinci-coding-agent/src/native_extensions/ecosystem/`, docs in `docs/ecosystem.md`, specs in `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`, plans in `docs/superpowers/plans/2026-09-04-ecosystem-b-runtime-integration.md`, `docs/superpowers/plans/2026-09-04-ecosystem-c-learning-security.md`, `docs/superpowers/plans/2026-09-04-ecosystem-d-proof-ci-hygiene.md`): Bounded runtime integration connects Graph with Token Governor, Vector Memory, Learning, and Security while maintaining strict worker isolation:
- **Governor Recovery Loop**: Workers preserve `retrieve_output` in their allowlist whenever compressible tools exist (`ensure_governor_recovery_tool`), guaranteeing compressed tool outputs remain fully retrievable in graph sub-processes.
- **Stable Cache Affinity**: Provider cache keys are decoupled from session IDs (`StreamOptions::cache_key`). Graph workers derive deterministic cache keys (`derive_worker_cache_key`, passed via `PI_GRAPH_CACHE_KEY`) based on `(cwd, graph_version, role, model, toolset, system_prompt, contract)`, keeping prompt caching warm across retries while retaining `--no-session --no-extensions --no-skills`.
- **Bounded Context Packets**: Graph workers receive a compact context packet (`<context source="davinci" untrusted="true">`) strictly capped at 2,500 aggregate tokens (up to 1,200 memory tokens / 4 hits; up to 1,000 skill tokens / 2 skills). Automatic child memory injection is suppressed (`PI_GRAPH_SUPPRESS_MEMORY_INJECT=1`) to prevent duplicate prompts. Context provenance (`context_fingerprint`, `memory_refs`, `skill_refs`) is recorded prior to worker execution.
- **Deterministic Resource Snapshots & Telemetry**: Cumulative token usage, cache read/write stats, and token governor savings/retrievals are aggregated deterministically into `GraphRun::resource_snapshot` and `GraphRun::ecosystem_stats` with zero coordinator model calls. Compact ecosystem participation renders on `/status` and `/graph-status`.
- **Conditional Security Gate**: Graph runs enforce configurable security policy (`securityVerification` in `GraphConfig`: `off | risk | always`, default `risk`). File mutations are classified deterministically by `assess_change_risk`; high-risk surfaces (auth, crypto, manifests, permissions, process execution) or `always` mode invoke non-interactive changed-surface security verification (`verify_changed_surface`) before review. Blockers block review approval and require revisions.
- **Closed-Loop Graph Learning**: Verified graph work improves later graph work. Successful verified runs persist high-confidence memories and project skills; subsequent related runs retrieve these exact skill versions and memories into worker context, and successful verification promotes the exact version's success counter.
- **Named Offline Test Suite**: Closed-loop suites `ecosystem_loop_*` and `ecosystem_invariants_*` run strictly offline against canned fixtures with zero model calls or external network dependencies.

**Self-improving learning system** (`crates/davinci-coding-agent/src/native_extensions/learning/`, design in `docs/superpowers/specs/2026-09-03-davinci-self-improving-learning-design.md`, plan in `docs/superpowers/plans/2026-09-03-davinci-self-improving-learning-plan.md`, docs in `docs/learning.md`): Settled turns hook (`complete_prompt_with_host`) invokes the asynchronous background reviewer thread (`reviewer.rs`). The reviewer extracts deterministic verification evidence (`evidence.rs`) by analyzing shell commands (`bash`, `powershell`, `execute_command`), `graph_run` outcomes, tool exit codes, tool errors, permission denials, and user acceptance/correction signals:
- **Review Gating & Signal Policy (`should_review_evidence`)**: Gating filters out low-signal, read-only turns while preserving full vector memory indexing. Reduces median reviewer input tokens by >= 40% with zero loss of accepted high-confidence artifacts.
- **Exact Skill Version Provenance & Attribution**: Skills injected into worker context carry immutable version references `(name, version, content_hash)`. Graph execution outcomes (`VerifiedSuccess`, `VerifiedFailure`, `Neutral`) are derived deterministically from `VerificationBundle` and attributed strictly to the targeted version record in the ledger (`record_skill_version_outcome`).
- **Declarative memory facts** (`LearningArtifact::Memory`): high-confidence facts (≥ 0.80) are persisted directly into vector memory without requiring manual confirmation.
- **Procedural skills** (`SkillCreate`, `SkillPatch`, `SkillSupportFile`): saved as `SKILL.md` under `.pi/skills/<name>/` (project) or `~/.pi/agent/skills/<name>/` (global) with structured YAML frontmatter. Verified workflows auto-promote to active after deterministic verification (`commands_ran > 0` and zero failures) or 2 verified usages without failures.
- **Progressive disclosure & discovery**: Native agent tools `skill_list`, `skill_view`, and `skill_manage` allow discovery, on-demand inspection, and guarded modification without bloating system prompt context. Full skill bodies load only on demand.
- **Append-only ledger & rollback**: `<learning_root>/candidates.jsonl`, `<learning_root>/skills.jsonl`, and `<learning_root>/state.json` track candidates, versions, and verification metrics (`store.rs`), keeping up to 5 versioned backup snapshots under `<learning_root>/history/<skill_name>/<version>.md` for rollbacks (`skill_manager.rs`).
- **Interactive commands & control**: `/learn [instruction]` synthesizes learning turns; `/learning-status`, `/learning-pending`, `/learning-approve <id>`, `/learning-reject <id>`, `/skill-list`, and `/skill-view <name>` provide review and observability.
- **Fail-open lifecycle**: Learning operations run asynchronously in the background and will never block or fail foreground turn executions. Can be disabled via `PI_LEARNING_DISABLE_BACKGROUND=1`.

**Harness Reliability and Guard Invariants** (design in `docs/superpowers/specs/2026-09-04-harness-reliability-design.md`, plan in `docs/superpowers/plans/2026-09-04-harness-reliability.md`):
- **Patch Authority and Recovery** (`apply_patch.rs`): `.pi_patch_journal.json` is reserved and pre-existing journals are refused during standard patch application; explicit recovery (`recover_incomplete_journal_if_any`) validates all targets before restoring and preserves journals on failure; exclusive creation (`create_new`) and flushing guarantee new journal durability.
- **Graph and Native Guard Boundaries** (`controller.rs`, `worker_hooks.rs`, `extension_host.rs`): Global extra tools are advertised and allowed solely to `Writer` roles; non-writers enforce baseline tools even if configured; shell aliases like `exec_command` enforce command policies; poisoned native locks recover to continue running pre-tool guards.
- **Context Budget Accounting** (`Agent::estimated_context_tokens`, `main.rs`): Estimates include `system_prompt` length and tool schema overhead; host overhead tokens are cached per prompt configuration, driving earlier pruning/compaction before model loops.
- **Governor Visibility and Search Freshness** (`token_governor.rs`, `extension_host.rs`, `vector_memory.rs`): Context pruning/compaction notifies the governor (`native_context_pruned`) to clear read/search dedupe ledgers so pruned output is re-fetched; unproven Git HEAD/status hashes were removed so searches run fresh when content state is unknown.
- **Interruptible Retries and Telemetry** (`turn.rs`, `stats.rs`): Exponential backoff sleeps in 25 ms slices checking abort signals; `RunStats::provider_retries` counts actual additional attempts; model wall time includes failed attempts; backward-compatible with older serialized stats JSON.
- **Evaluation Release Gate** (`codex_eval.rs`): Release criteria enforces `median_tools <= 10.0` no-worsening to detect tool-call explosions.

**Sessions** are JSONL files under `~/.davinci/agent/sessions` by default, with existing `~/.pi/agent/sessions` stores discovered for compatibility. The environment-variable order is `DAVINCI_*` first and legacy `PI_*` second. Cwd-encoded directories remain byte-compatible with TypeScript `pi`. Path resolution lives in `davinci-session/src/discovery.rs` (`default_agent_dir`, `default_session_dir`). Project configuration follows the same Davinci-first, Pi-compatible policy.

**Protocol and MCP transport hardening**: `davinci-protocol` is length-prefixed CBOR with explicit depth/length limits; it rejects duplicate map keys, out-of-range integers/floats, and avoids preallocating from untrusted advertised container lengths. `davinci-server` serves an `Agent` over it and `davinci-client` consumes it; `PROTOCOL_VERSION` compatibility is checked on hello. MCP HTTP response bodies and stdio lines are capped at 16 MiB, stdio buffering is bounded, and JSON-RPC responses must match the request ID and envelope shape exactly. Shell auto-approval parsing is byte-safe for Unicode and rejects ambiguous quoting, expansion, redirection, and disguised mutation. See `docs/security-audit-2026-09-07.md` for the bounded audit record and its validation limits.

**The davinci TUI** (`crates/davinci-tui/src/davinci/`, driven by `crates/davinci-coding-agent/src/davinci_interactive.rs`) is the native Rust interactive default. Its current visual contract is `docs/ui/design.md`: a Claude Code-style conversation with warm coral accents, a pixel welcome banner showing the actual version/model/cwd, shaded user messages, assistant bullets, compact tool calls with result rows, and a ruled composer that scrolls around the caret. The conversation hides the dashboard header and places the composer after short content. The full `/model` picker is content-sized and bottom-anchored so the recent conversation remains visible above it; Left/Right changes the selected model's supported reasoning level and Enter saves both choices. Selecting terminal text with a left-button drag copies it on release. Local voice input inserts editable composer text only and must never submit it. Preserve narrow-terminal behavior, Unicode caret handling, monochrome state glyphs, and existing shortcuts. Theme colors belong in `theme.rs`; views return `Vec<Line<'static>>`.

The historical HTML mockups and Elixir reference under `docs/ui/` document earlier layouts; they do not override the current conversation design. Current built-in command discovery is defined only by `builtin_slash_commands()` in `slash.rs`; native-extension discovery is defined by `command_specs()` in `native_extensions/mod.rs`. Notable interactive surfaces include `/model`, `/settings`, `/thinking <off|minimal|low|medium|high|xhigh|max>`, `/login`, `/hotkeys`, `/resume`, `/tree`, `/compact`, `/export`, `/graph`, `/memory-status`, `/governor-status`, `/security-scan`, `/reload`, `/mcp`, `/permissions`, `/plan`, and `/act`. `/llama`, `/trust`, `/changelog`, `/scoped-models`, and the separate `/sec-status`, `/sec-report`, `/sec-abort`, and `/sec-resume` lifecycle commands are not public slash commands. Security lifecycle operations remain internal to `/security-scan`. Shared sheet framing lives in `views/sheet.rs`. Keep graph, governor, vector-memory, security, and permission surfaces wired to real runtime data. `davinci --davinci --screen <id>` opens fixture screens for visual inspection; it is interactive and blocks.

**Startup responsiveness** (`davinci_interactive.rs`, `davinci_sources.rs`, `startup.rs`): the initial workspace scan uses the existing `WorkspaceDresser` worker. Apply completed results between turns and rebuild the command palette corpus when session data arrives. Package update and tmux checks run in the background; configuration, migration, and trust warnings remain in the opening block. Preserve typed input and transcript indices when background results arrive. Davinci does not query the upstream Pi release endpoint or show Pi version/changelog update notices. Do not reintroduce those notices when updating parity code. Measure time to the first usable composer, accounting for terminal-harness startup overhead; offline timings do not establish startup speed with every user extension or MCP configuration.

## Conventions

- **Cite the TypeScript source.** Most modules open with a doc comment naming the file they mirror, e.g. `//! Project trust store matching vendor/pi/packages/coding-agent/src/core/trust-manager.ts`. Follow this for new modules; when changing behavior, read the cited TS file first and match it rather than improving on it.
- **Tests are fixture-only and never touch the network.** Prefer the `DAVINCI_*` fixture/environment spelling where supported and retain `PI_*` aliases for compatibility. Existing fixtures include `DAVINCI_OFFLINE` / `PI_OFFLINE`, `PI_DISABLE_NETWORK`, provider/update/package fixtures, `PI_OPEN_BROWSER_DRY_RUN`, `PI_MCP_CONFIG`, `PI_MCP_FIXTURE`, `PI_SUBAGENT_FIXTURE`, hook fixtures, and learning fixtures. MCP HTTP tests may also use a `fixture:<path>` URL. Add a fixture hook rather than a live call when a new code path needs one.
- Tests are inline `#[cfg(test)] mod tests` blocks (~188 of them); there are no `tests/` directories except `davinci-parity/fixtures`.
- Preserve TS behavioral contracts unless the user authorizes a divergence. Document intentional differences, including the native Davinci UI, independent release identity, omitted Pi update notices, and platform-specific tools.

## Gotchas

- **Legacy crates and dead code cleanup**: Uncompiled legacy source islands in `davinci-session`, `davinci-agent`, and `davinci-session-sqlite` were audited and removed in release Gate D (see `docs/archive/dead-code-audit-2026-09-04.md`). The standalone crate `crates/davinci-core` remains as an explicit historical archive crate that is not a workspace member and has no dependents. Before editing something a grep turned up, check that its module is actually declared in the active workspace.
- **`home_dir()` mirrors Node `os.homedir()`** (`davinci-session/src/discovery.rs`): `USERPROFILE` first on Windows, `HOME` otherwise. Session dirs use the TS `--…--` cwd encoding (every `/`, `\`, `:` becomes `-`); the older Rust `--a--b` encoding is still scanned read-only for pre-existing stores. Tests that create sessions should still set `PI_CODING_AGENT_DIR` or `PI_CODING_AGENT_SESSION_DIR` so they never touch the real `~/.pi`; if `--Users--…/` directories ever reappear in the repo, they are test/session strays — delete, never commit.
- **Git pre-commit hook on Windows**: Environments where `core.hooksPath` points to Unix shell scripts (e.g. `~/.codex/git-hooks`) may fail on Windows with `execvpe(/bin/bash) failed: No such file or directory`. Use `git commit --no-verify` to bypass this hook when committing on Windows.
- `davinci-coding-agent` (~47k lines) and `davinci-tui` (~39k lines) are large; prefer targeted `grep` over reading whole files.
