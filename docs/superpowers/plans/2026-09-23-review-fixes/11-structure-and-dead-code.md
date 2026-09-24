# Phase 11: Structure, Dead Code and Open Decisions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the structural causes behind many of the review's bugs (modules compiled twice, 12k-line files, several tables that disagree about the same tool, subsystems wired only in tests), and resolve the partly-wired features by wiring or deleting them.

**Architecture:** Refactors in this phase change no behavior unless the task says so. Each refactor task's verification is "the full test suite passes unchanged, and the diff moves code rather than rewriting it". Behavior tasks follow the usual test-first steps.

**Tech Stack:** Rust 1.83.

Run this phase **last**: earlier phases edit the same large files, and moving code first would turn every earlier task into a merge conflict.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-coding-agent/src/main.rs:1-113`, `lib.rs` | 11.1 |
| `crates/davinci-coding-agent/src/main.rs` (13.5k lines), `davinci_interactive.rs` (12.7k), `native_extensions/graph/controller.rs` (6.3k), `crates/davinci-agent/src/{lib.rs, turn.rs, tools.rs, permission.rs, runtime/tasks.rs}` | 11.2 |
| `crates/davinci-tui/src/*` (legacy stack), `crates/davinci-coding-agent/src/main.rs:261-273, 4571` | 11.3 |
| dead-code sites listed in 11.4 | 11.4 |
| `crates/davinci-agent/src/runtime/budget.rs`, `turn.rs:734, 783, 2598-2600`, `davinci_interactive.rs:3090-3180` | 11.5 |
| `crates/davinci-coding-agent/src/davinci_interactive.rs` event loops | 11.6 |
| `crates/davinci-agent/src/{permission.rs, runtime/capabilities.rs, scheduler.rs, subagent.rs, tool_ledger.rs, runtime/contracts.rs, permission_risk.rs, turn.rs}` | 11.7 |
| `crates/davinci-coding-agent/src/native_extensions/mod.rs:340-484` | 11.8 |
| `crates/davinci-coding-agent/src/packages.rs:275-396`, `self_update.rs:534-572` | 11.9 |
| `crates/davinci-coding-agent/src/js_host.rs:329, 387-469` | 11.10 |

---

### Task 11.1: `main.rs` stops compiling its own copy of library modules

**Finding:** coding-agent structural 1. `main.rs:1-113` declares `native_extensions`, `settings`, `hooks`, `trust`, `runtime_host`, `agent_profiles`, `completion_delivery`, `native_tools`, `args` (and more), which `lib.rs` also compiles. About 100k lines are built twice; every static exists twice (for example `hooks::GLOBAL_HOOK_TELEMETRY`); types from the two copies are different types.

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs:1-113`, `crates/davinci-coding-agent/src/lib.rs`

- [ ] **Step 1: Inventory**

Run:

```bash
grep -E "^\s*(pub )?mod [a-z_]+;" crates/davinci-coding-agent/src/main.rs | sed -E 's/.*mod ([a-z_]+);/\1/' | sort > /tmp/main_mods
grep -E "^\s*(pub )?mod [a-z_]+;" crates/davinci-coding-agent/src/lib.rs | sed -E 's/.*mod ([a-z_]+);/\1/' | sort > /tmp/lib_mods
comm -12 /tmp/main_mods /tmp/lib_mods   # compiled twice
comm -23 /tmp/main_mods /tmp/lib_mods   # binary-only
```

Write both lists into the PR description.

- [ ] **Step 2: Add a guard test that fails today**

```rust
// crates/davinci-coding-agent/tests/single_compilation.rs
#[test]
fn no_module_is_compiled_by_both_crate_roots() {
    let main = include_str!("../src/main.rs");
    let lib = include_str!("../src/lib.rs");
    let mods = |src: &str| -> std::collections::BTreeSet<String> {
        src.lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("pub mod ").or_else(|| line.strip_prefix("mod "))?;
                rest.strip_suffix(';').map(str::to_string)
            })
            .collect()
    };
    let both: Vec<_> = mods(main).intersection(&mods(lib)).cloned().collect();
    assert!(both.is_empty(), "compiled twice: {both:?}");
}
```

Run: `cargo test -p davinci-coding-agent --test single_compilation` → FAIL listing the duplicates.

- [ ] **Step 3: Move one module at a time**

For each duplicated module, in this order (fewest `crate::` references into binary-only modules first): `args`, `agent_profiles`, `completion_delivery`, `trust`, `settings`, `hooks`, `native_tools`, `runtime_host`, `native_extensions`:
1. In `main.rs`, replace `mod <name>;` with `use davinci_coding_agent::<name>;`.
2. Make the library module `pub` if it is `mod` in `lib.rs`.
3. `cargo build -p davinci-coding-agent --bin davinci`. Fix errors, which fall into two kinds: items that were `pub(crate)` and are used by the binary (make them `pub`), and library code that reaches `crate::<binary-only module>` (move that function into the binary, or move the binary-only module into the library first).
4. `cargo test -p davinci-coding-agent` must pass before the next module.
5. Commit per module: `git commit -m "refactor: compile <name> once, in the library"`.

Then move binary-only modules into the library the same way, until `main.rs` holds only `fn main` and argument dispatch. Stop and report if a module cannot move without a behavior change.

- [ ] **Step 4: Verify**

Run: `cargo test -p davinci-coding-agent --test single_compilation` → PASS; `cargo test --workspace` → PASS; `cargo build --release -p davinci-coding-agent` and compare binary size before/after in the PR (expected: smaller).

---

### Task 11.2: Split the largest files, tests first

**Finding:** structural items in every review. Files over 3,000 lines: `main.rs` 13,459 (≈3k tests from line 10434), `davinci_interactive.rs` 12,711 (≈3.3k tests from 9358), `graph/controller.rs` 6,276, `davinci-agent/src/lib.rs` 6,251 (≈3k tests from 3203), `turn.rs` 5,992 (tests from 3842), `tools.rs` 4,669, `runtime/tasks.rs` 4,016, `permission.rs` 3,436 (≈1.7k tests), `davinci-tui/src/davinci/app.rs` 3,452, `model.rs` 3,285.

**Rule:** move, do not rewrite. Each commit moves one cohesive block and must leave `cargo test --workspace` green with the same number of tests.

- [ ] **Step 1: Move inline test modules to sibling files**

For each file above with an inline `#[cfg(test)] mod tests { ... }` block, move the block to `<file>_tests.rs` (or `<dir>/tests.rs`) and replace it with

```rust
#[cfg(test)]
#[path = "<file>_tests.rs"]
mod tests;
```

(`turn.rs:3833-3835` already does this for `session_persistence_tests.rs`; follow that pattern.) Record the test count before and after: `cargo test -p <crate> -- --list 2>/dev/null | grep -c ': test$'`; the numbers must match.

- [ ] **Step 2: Split production code along existing seams**

- `main.rs`: the RPC host loop (`main.rs:3083-3900` today) → `rpc_host.rs`; extension session-call application (`apply_session_calls` and helpers, around `main.rs:8646-9000`) → `session_calls.rs`; print mode → `print_mode.rs`; login/auth commands → `login.rs`.
- `turn.rs`: loop body / request preparation / tool dispatch / finalize / contract gate → `turn/{loop.rs, prepare.rs, dispatch.rs, finalize.rs, contract_gate.rs}`.
- `tools.rs`: one module per tool family (`tools/{fs.rs, search.rs, shell.rs, web.rs, glob.rs}`), keeping `execute_tool_with` as the dispatcher.
- `graph/controller.rs`: checkpoint I/O → `graph/checkpoint.rs`; delivery stages → `graph/delivery.rs`; diff capture → `graph/diff.rs`.

One commit per extracted module: `git commit -m "refactor(<crate>): move <area> out of <file>"`.

- [ ] **Step 3: Verify**

After each commit: `cargo test --workspace` green, same test count. At the end, list line counts (`wc -l` on the files above) in the PR.

---

### Task 11.3 (DECISION D9, recommended: feature-gate): the legacy TUI

**Finding:** TUI structural 4. About 34k lines of `davinci-tui` (`tui_alt_screen.rs` 2,451, `session.rs` 2,565, `tui_runtime.rs` 1,308, `terminal.rs`, `stdin_buffer.rs`, `overlay.rs`, the selectors) run only under `--legacy-tui` or `PI_DAVINCI=0`. `davinci/fixtures.rs` (2,178 lines, `pub`) ships in release builds only to support `--davinci --screen` (`main.rs:261-273`).

**Recommended:** put the legacy stack behind a `legacy-tui` cargo feature (off by default) and fixtures behind `mockups`:
- [ ] Move the modules davinci actually uses (`editor`, `keybindings`, `autocomplete`, `ansi`, `word_wrap`, `word_nav`, `interaction`, `osc`, `open_browser`) out of the legacy set; confirm with `rg -n "davinci_tui::(editor|keybindings|...)" crates/davinci-coding-agent/src` and a build with the feature off.
- [ ] `#[cfg(feature = "legacy-tui")]` on the remaining legacy modules and on `--legacy-tui` / `PI_DAVINCI=0` handling in `main.rs`; without the feature those print "the legacy UI is not included in this build".
- [ ] `#[cfg(feature = "mockups")]` on `davinci/fixtures.rs` and `--davinci --screen`.
- [ ] CI builds and tests once with `--all-features` so the gated code keeps compiling.
- [ ] Verify: `cargo build --release -p davinci-coding-agent` succeeds and the binary is smaller (record sizes); `cargo test --workspace --all-features` passes.
- [ ] Commit: `git commit -m "chore(tui): gate the legacy TUI and screen mockups behind features"`.

**Alternative:** delete the legacy stack outright. Choose this only if nobody uses `--legacy-tui`; it removes Task 7.12.

---

### Task 11.4: Remove dead code, and fix the one dead-looking path that is a bug

**Findings:** dead code listed by every review. Before removing each item, confirm it is unused with `rg -n "<name>"` across `crates/` (excluding its own definition and tests). Remove it and its tests together. One commit per bullet.

- [ ] `davinci-ai/src/oauth.rs:57-88` `poll_oauth_device_code_flow`: exported, unused, and it never sleeps (would hammer the endpoint and "expire" in milliseconds). Delete. The device-code providers (xai, kimi, github-copilot) then have no login flow; make `/login <provider>` for them say "not supported yet" instead of printing nothing.
- [ ] `davinci-ai/src/codex_ws.rs:70-73` `continuation_hit`, `used_delta`: computed and discarded. Delete.
- [ ] `davinci-agent/src/runtime/team.rs` (`TeamManager`, 488 lines; only re-exported at `runtime/mod.rs:121`): delete with its re-export, unless decision D5's owner wants it for the teammate feature; if kept, add a test that exercises it from a real code path.
- [ ] `davinci-coding-agent/src/output.rs:103-384` completion and graph-status printers used only by tests; `main.rs:10415` `store_api_key`; `completion_delivery` module's `#![allow(dead_code)]`: delete what is unused, then remove the `allow` so new dead code shows up in clippy.
- [ ] `davinci-coding-agent/src/migrations.rs:194-210` keybindings migration is a no-op: either implement the migration it names (read the TS pi migration it mirrors) or delete it and its call.
- [ ] `davinci-agent/src/subagent.rs:355-375` read-only-parent branch is unreachable because Plan mode denies `agent` (`permission.rs:1112`). **(DECISION D11)** Recommended: make it reachable, since research subagents in Plan mode are useful and the branch already enforces read-only tools: classify an `agent` call whose tasks request no mutation tools and no worktree as `ToolClass::Read` in Plan mode, with a test that a Plan-mode `agent` call runs and a Plan-mode `agent` call with `tools: ["write"]` is denied. Alternative: delete the branch.
- [ ] `native_extensions/graph/worker.rs:883-938`: `pub fn run_fixture_worker_with_deadline` ships in release builds and contains `assert!`. Put it behind `#[cfg(any(test, feature = "test-fixtures"))]` (the feature from Task 2.15).
- [ ] **Bug, not dead code:** `hooks::status_report` (`hooks.rs:829-831`) calls `load(.., cwd, true)`, so `/hooks` reports untrusted project hooks as trusted. Fix: compute trust with `crate::settings::is_trusted(&crate::settings::load_merged_settings(&agent_dir, cwd), cwd, None)` and pass it. Test: an untrusted project with `.pi/hooks.json` reports `"trusted": false`.

---

### Task 11.5 (DECISION D5): budget ledger and progress watchdog: wire them or delete them

**Findings:** davinci-agent 11 and 12. `runtime/budget.rs` (1,359 lines): `with_budget_ledger` is called only in a test (`davinci_surfaces.rs:1113`); `ResourceLedger::reserve` and `acquire_worker_slot` are never called outside tests; `turn.rs:734, 783` discard `record_retry` / `settle_receipt` results with `let _ =`. The watchdog: `turn.rs:2598-2600` does `let _ = wd.observe(observation)`; `apply_watchdog_choice` is `#[allow(dead_code)]` (`davinci_interactive.rs:3169`); `watchdog_ask` is used only in a test. Task 5.9 (`maxModelTurns`) already bounds runaway loops, which lowers the urgency of both.

**Recommended: wire the watchdog, delete the budget ledger.**

Watchdog:
- [ ] When `wd.observe(observation)` returns `Some(signal)`, call `wd.pause(signal.clone())` and ask the host through `watchdog_ask` (the davinci UI shows the existing modal at `davinci_interactive.rs:3090-3180`; RPC emits an extension-UI request; print mode fails the run with the signal text). Remove `#[allow(dead_code)]` from `apply_watchdog_choice`.
- [ ] Test (agent crate): a fixture model that edits the same file back and forth 6 times triggers exactly one watchdog ask, and choosing "stop" ends the run.
- [ ] Commit: `git commit -m "feat(agent): progress watchdog pauses looping runs and asks the user"`.

Budget ledger:
- [ ] Delete `runtime/budget.rs`, `with_budget_ledger`, the `record_retry` / `settle_receipt` calls and `stats.apply_budget_snapshot`; keep `RunStats` token and cost counters, which are displayed today. Cost and token ceilings, if wanted later, get their own plan based on those counters.
- [ ] Commit: `git commit -m "chore(agent): remove the unwired budget ledger"`.

**Alternative:** wire the ledger (install from settings, `reserve` before each provider call, stop the loop on `BudgetError`). Scope: its own plan; it needs settings, UI and RPC surfaces.

---

### Task 11.6: One event pump for the davinci UI

**Finding:** TUI structural 1. `davinci_interactive.rs` has three event loops (idle ≈4306, turn ≈1461, scope modal ≈3055) plus a fourth in `runtime::run`; each re-implements resize, paste, mouse, voice and tick handling, and they drifted apart (the bugs fixed in 7.2 and 7.9). About 15 identical `Shell { voice, parsed, agent, model, terminal, host, pending, cwd, dresser, images }` literals exist.

**Files:**
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`

- [ ] **Step 1: One constructor**

Replace the repeated `Shell { .. }` literals with `Shell::new(...)` (or `Shell::from_parts`), one commit, no behavior change.

- [ ] **Step 2: One pump**

```rust
/// What the loop is doing; it decides how keys are handled, nothing else.
enum Mode<'a> {
    Idle,
    Turn(&'a mut TurnState),
    ScopeModal(&'a mut ScopeModalState),
}

/// Every iteration of every loop: reacquire the terminal, housekeeping
/// (tick, voice, jobs), then one event dispatched by mode, then draw if
/// dirty.
fn pump(shell: &mut Shell, mode: &mut Mode, timeout: Duration) -> io::Result<Option<ModeOutcome>> {
    let _ = shell.terminal.reacquire();
    housekeeping(shell);
    if let Some(event) = shell.terminal.poll_event(timeout)? {
        match event {
            Event::Resize(..) | Event::Paste(..) | Event::Mouse(..) => handle_common(shell, event),
            Event::Key(key) => return Ok(handle_key(shell, mode, key)),
            _ => {}
        }
    }
    if shell.model.dirty {
        shell.terminal.draw(&shell.model)?;
        shell.model.dirty = false;
    }
    Ok(None)
}
```

Move each loop's key handling into `handle_key`'s arm for its mode, one loop per commit, starting with the scope modal (smallest). Existing interaction tests (`interaction_testing` feature) must pass after each commit.

- [ ] **Step 3: Verify**

`cargo test -p davinci-coding-agent --features interaction-testing` green after every commit; manual smoke test in the PR: idle typing, a turn with a permission prompt, a scope-expansion prompt, voice push-to-talk, resize during each.

---

### Task 11.7: One table decides what a tool is

**Finding:** davinci-agent structural 2. Tool classification lives in 7+ places that disagree: `permission::tool_class`, `capabilities::default_declared_effects` and its concurrency policy, `scheduler::lane_for`, `subagent::MUTATION_TOOLS`, `tool_ledger::classify_side_effect`, and three target extractors (`contracts::extract_tool_targets`, `permission_risk::file_targets`, `turn::mutation_paths_from_tool`). The `agent` lane bug (Task 5.6) and the unreachable Plan-mode branch (Task 11.4) came from this.

**Rule:** `RuntimeCapabilityRegistry` (`runtime/capabilities.rs`) becomes the only source; the other functions become thin reads of it.

- [ ] **Step 1: Pin today's behavior**

Write a table test that, for every built-in tool name (`crate::tools::BUILTIN_TOOLS`), records `tool_class`, `lane_for`, whether it is in `MUTATION_TOOLS`, `classify_side_effect`, and the declared effects, and asserts they are mutually consistent (a `ParallelSafe` tool has no `FileSystemWrite`; every `MUTATION_TOOLS` entry declares `FileSystemWrite`; `ToolClass::Edit` ⇔ `FileSystemWrite`). Run it; every inconsistency it reports is a bug to fix in its own commit with the table row as the test.

- [ ] **Step 2: Derive, one function at a time**

Rewrite `lane_for`, `MUTATION_TOOLS` (becomes a function), `classify_side_effect` and `tool_class` for built-ins as lookups in the registry. Keep the three target extractors but make two of them call the third (`contracts::extract_tool_targets`), with a test that all three returned the same targets for a fixed set of tool calls before the change.

- [ ] **Step 3: Verify and commit**

`cargo test -p davinci-agent` green after each function. One commit per function: `git commit -m "refactor(agent): derive <function> from the capability registry"`.

---

### Task 11.8: Native extension host loads settings once

**Finding:** native-extensions structural 5. `NativeExtensionHost::new_with_agent_dir` (`native_extensions/mod.rs:340-484`) calls `load_merged_settings` about 9 times, and the `.davinci` vs `.pi` fallback logic is repeated in `learning/mod.rs:53-120`, `vector_memory.rs:~850`, `token_governor.rs:476-501`, `graph/definitions.rs:1199-1230` and `graph/store.rs`.

- [ ] Load `Settings` once at the top of `new_with_agent_dir` and pass `&Settings` to each sub-host constructor (signature change only; no behavior change). Test: a counter in `load_merged_settings` (behind `#[cfg(test)]`) reads 1 after constructing a host.
- [ ] Replace the repeated directory fallbacks with `crate::project_config::resolve` / `all` (Task 1.4) plus one helper `project_config::data_dir(cwd, name) -> PathBuf` (returns the `.davinci/<name>` path unless only `.pi/<name>` exists). One commit per module; each module's tests must pass unchanged.

---

### Task 11.9 (DECISION D10): `davinci update` does only what it says

**Findings:** coding-agent 25 (self-update). A plain `davinci update` runs self-update; for an unknown install method it copies the running binary to `<agent>/../bin/davinci` (no `.exe` on Windows) and reports "Updated from X to X"; the npm path installs upstream `@earendil-works/pi-coding-agent`; the managed path refuses live HTTP (`packages.rs:275-396`, `self_update.rs:534-572`).

**Recommended behavior:**
- `davinci update` updates extensions and packages only.
- `davinci update --self` updates the binary only for a detected install method with a correct package name; `InstallMethod::Unknown` prints how davinci was installed and the command to update it manually (for this repo's documented install, the memory note says `cargo install` is broken and the binary is copied from `target/release`; print that), and exits non-zero.
- The npm package name comes from one constant that names davinci's package, not upstream pi's; if davinci has no published npm package, the npm path is removed.

- [ ] Tests: `update` with no flags never calls the self-update path (use the existing `PI_*_REPLY` stubs to observe calls); `update --self` with `Unknown` returns an error containing "manually"; no code path reports "Updated from X to X" with equal versions.
- [ ] Commit: `git commit -m "fix(update): update runs extensions only; --self refuses unknown install methods"`.

---

### Task 11.10: One JS runner pool

**Finding:** coding-agent 22 (pool half). `PERSISTENT_JS` (`js_host.rs:329`) has one slot, used for autocomplete, `streamSimple`, `refreshModels` and OAuth; alternating between modules respawns Node on every call, and the same extension can run in two Node processes that do not share state (`js_host.rs:387-413` vs `:426-469`).

- [ ] Replace both pools with one `HashMap<PathBuf, PersistentJsSession>` behind a mutex, capped at 8 live sessions (least recently used evicted), keyed by module path. Test: calling two modules alternately 10 times spawns exactly 2 Node processes (count spawns through a `#[cfg(test)]` counter).
- [ ] Commit: `git commit -m "perf(extensions): one keyed pool of persistent JS runners"`.

---

## Phase 11 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
```

Test counts per crate equal or greater than before the phase (list them in the PR).
