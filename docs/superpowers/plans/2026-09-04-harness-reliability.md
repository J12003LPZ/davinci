# Davinci Harness Reliability Implementation Plan

Date: 2026-09-04. Workflow: Superpowers design, writing-plans, executing-plans,
TDD and verification. Single writer; after the user's continuation instruction,
all further implementation, verification and review are solo.

## Objective and completion contract

Repair high-impact authority, context and recovery defects in the existing Rust
harness. Preserve entry points, supported workflows and unrelated `plugins/`.
No dependencies, installs, commits, external writes or paid provider runs.

Completion requires source-backed priorities, implemented bounded fixes, targeted
checks, diff review and documented limits. It does not mean competitor parity.
The full assessment, tradeoffs and remaining priorities are in
`docs/superpowers/specs/2026-09-04-harness-reliability-design.md`.

## Dependency-aware execution

Context, tool-authority and provider/eval investigations were initially parallel:
none depended on another's output. Findings were reconciled before implementation.
Patch safety came first; context and retry changes were logically independent but
all writes remained in the main session. Each changed boundary received targeted
validation. No second orchestration framework was added.

## 1. Patch authority and recovery — implemented and verified

**Inputs:** parsed patch, permission subject, journal, path sanitizer.
**Files:** `crates/davinci-agent/src/apply_patch.rs`.
**Scope:** preserve execute/recovery signatures; remove implicit journal authority.

1. Add temporary-directory tests for an unrelated victim journal, reserved journal
   target, invalid recovery path and failed restore. Observe four failures on old code.
2. Refuse existing journals without touching targets. Exclusively create and flush
   new journals. Validate every recovery path before restoring anything.
3. Preserve unresolved journals and distinguish mutation/rollback/cleanup failures.
4. Verify normal multi-file/hunk/CRLF behavior, explicit recovery, and rollback after
   a real second-mutation failure.

**Acceptance:** patches cannot replay unrequested journal actions; failed recovery
retains evidence. **Check:** 14 patch tests passed. **Handoff:** interrupted patches
now require explicitly authorized recovery; not a race-proof transaction manager.

## 2. Graph and native guard boundaries — implemented and verified

**Inputs:** role tool baselines, extras, shell policies, native host lock.
**Files:** graph `controller.rs`, `worker_hooks.rs`; `extension_host.rs`.

1. Reproduce nonwriter mutation extras and shell-alias policy bypass.
2. Advertise extras only to writers. Enforce nonwriter baselines in the worker
   even if its tool set is widened; check all shell aliases and missing commands.
3. Preserve output retrieval, submission and intended writer edits.
4. Reproduce native-lock poisoning becoming an allow decision; continue calling
   guards through recovered locks, consistent with the outer host's policy.

**Acceptance:** extras cannot widen read-oriented roles; poisoned locks do not
silently disable pre-tool guards. **Checks:** worker regressions, controller
advertisement fixture, poisoned-lock regression. No real subprocess agents/models.
**Tradeoff:** unknown extras for nonwriters are denied; regex guards are not OS isolation.

## 3. Provider context budget — implemented and verified

**Inputs:** system prompt, actual builtin/MCP/native schemas, identity suffix,
messages and ephemeral context. **Files:** agent `lib.rs`, product `main.rs`.

1. Reproduce omitted 4,000-byte system prompt contribution.
2. Add system estimate and optional host overhead; retain active-tool fallback.
3. Cache actual product catalog/identity estimate once per prompt configuration,
   outside the internal model loop.
4. Verify active-tool changes, override/reset, overhead-triggered pruning,
   ephemeral context and original-history preservation.

**Acceptance:** pruning/compaction pressure includes fixed request overhead.
**Checks:** context/pruning selectors and compiled consumer. **Risk:** byte/4 is
approximate, not billing tokens or a guaranteed upper bound.

## 4. Governor visibility and search freshness — implemented and verified

**Inputs:** pruning/compaction counters, read/search ledgers and search state.
**Files:** product `main.rs`, `extension_host.rs`, native `token_governor.rs`;
`vector_memory.rs` only to remove the obsolete state-key helper.

1. Reproduce suppression after pruning and suppression when freshness is unknown.
2. Notify the governor at provider boundary when visibility counters change.
   Clear ledgers without losing stored output or accumulated counters.
3. Stop treating Git HEAD/status as proof of unchanged content. Unknown freshness
   executes searches; complete authoritative-key dedupe remains available.
4. Verify host hook, read dedupe, compression, retrieval and failed-search behavior.

**Acceptance:** needed content can be retrieved after pruning; dirty/ignored/outside
changes cannot be hidden by a weak state key. **Tradeoff:** repeated searches may
cost output tokens; two Git subprocesses per formerly fingerprinted search are
removed. No net end-to-end token-saving claim is made.

## 5. Cancellable and observable retries — implemented and verified

**Inputs:** existing classifier, attempt limits and cross-thread abort flag.
**Files:** agent `lib.rs`, `turn.rs`, `stats.rs`.

1. Reproduce cancellation still issuing a second request under the old test bypass.
2. Use real backoff in tests/runtime, checking abort in 25 ms sleep slices.
3. Count only actual extra provider attempts; aborted recovery is not success.
   Record failed-call wall time and default older JSON retry counts to zero.
4. Verify transient success, permanent failure, cancellation, aborted response,
   existing retry bounds and stats compatibility.

**Acceptance:** abort prevents the next request; permanent failures are not retried.
**Checks:** retry and stats selectors. **Risk:** this interrupts backoff, not every
in-flight provider transport. Tool-mutation retries are unchanged.

## 6. Evaluation integrity — implemented and verified, bounded

**File:** `crates/davinci-evals/src/codex_eval.rs`.

1. Reproduce release acceptance despite tool calls increasing tenfold.
2. Include median tool-call regression in the existing 10% no-worsening guard.
3. Refresh JSONL path/package against actual workspace files.
4. Verify acceptance, duplicate-side-effect rejection and statistical helpers.

**Acceptance:** reproduced tool explosion fails; valid existing improvements pass.
**Check:** five eval-module tests. These fixtures do not measure real task performance.

## 7. Delivery gate

- Source-backed priority/benefit/risk assessment and solo final diff review.
- Targeted checks below; no whole-workspace or coverage campaign.
- Passed formatting on all 11 changed Rust files, non-test consumer compilation,
  and `git diff --check`; final status contains only intended source/docs plus
  pre-existing `plugins/`. No independent final reviewer is claimed.
- Defer unmeasured prompt rewrites, projected compaction, incremental memory,
  transport identity changes and live eval integration to the next justified scope.

## Verification ledger

All commands below are prefixed with `rtk proxy`. RED runs reproduced defects
before fixes. Counts are per selector and overlap; do not sum as unique coverage.

| Command | Final observed result |
| --- | --- |
| `cargo test -p davinci-agent apply_patch::tests --lib` | 14 passed |
| `cargo test -p davinci-agent context --lib` | 5 passed |
| `cargo test -p davinci-agent prun --lib` | 6 passed |
| `cargo test -p davinci-agent retry --lib` | 7 passed |
| `cargo test -p davinci-agent stats::tests --lib` | 3 passed |
| `cargo test -p davinci-coding-agent token_governor::tests --bin davinci` | 20 passed |
| `cargo test -p davinci-coding-agent context_pruned_hook --bin davinci` | 1 passed |
| `cargo test -p davinci-coding-agent worker_ --bin davinci` | 22 passed, including worker hooks, controller and native-host boundaries |
| `cargo test -p davinci-coding-agent poisoned_native_lock --bin davinci` | 1 passed |
| `cargo test -p davinci-coding-agent graph_worker_spec --bin davinci` | 1 passed after strengthening the final assertion |
| `cargo test -p davinci-evals codex_eval::tests --lib` | 5 passed |
| `cargo check -p davinci-coding-agent --bin davinci` | Passed, no warnings |
| `rustfmt --check --edition 2021 --config skip_children=true` with the 11 changed Rust paths | Passed |
| `git -c core.safecrlf=false diff --check` | Passed |

Final source review found no known critical regression in the changed paths.
Tests exercise deterministic behavior; the assessment lists remaining architectural
risks and limits. Nothing was committed, installed or deployed.

### Deterministic before/after evidence

| Scenario | Before | After |
| --- | --- | --- |
| Existing journal names unrelated victim | Patch can delete victim | Refused; victim/target preserved, journal retained |
| Invalid/failed recovery | Ignores errors and removes journal | Error with recovery state retained |
| Nonwriter receives write extras | Mutation permitted | Mutation refused |
| 4,000-byte ASCII system prompt | Zero estimate contribution | Adds 1,000 estimated tokens |
| Unknown search freshness | Repeat blocked | Fresh result returned |
| Read after pruning | May be marker-only | Source body available again |
| Cancel during 5-second backoff | Test skips wait; runtime sleep uninterruptible | One request, real test below its 2-second assertion bound |
| Tool calls 10 to 100, other metrics halve | Release accepted | Release rejected |

No billed-token, task-success, cache-hit or competitor timing measurements were made.

## Next milestone: real coding-task evaluation

This is a plan, not an implemented/executed live benchmark.

**Objective:** establish task success at comparable model capability and permissions
before adding mechanisms. **Inputs:** immutable task snapshots and acceptance
checks; approved model/cost boundaries; baseline/candidate binaries; isolated
temporary working copies. Never benchmark by mutating the user's checkout.

**Categories:** exploration, local bug fixing, multi-file features, test-failure
diagnosis, pinned dependency/API investigation, behavior-preserving refactor,
tool/provider recovery, long-context constraint retention and reversible ambiguity.

1. Add a thin real-runtime runner. Each fixture supplies starting snapshot,
   allowed paths/actions, expected result, forbidden changes and smallest relevant check.
2. Capture provenance-tagged events: live/synthetic, task/model/version, provider
   input/output/cache usage, context estimate trajectory, tool identity/result/time,
   retries, edits, tests and interventions. Missing metrics are unknown, not zero;
   do not record raw prompts or secrets by default.
3. Run one baseline/candidate smoke pair first. Match model, starting state and
   permissions; separate cold/warm cache conditions. Include failed tasks and do
   not hide retries as additional independent attempts.
4. Judge real outcomes/tests and allowed file changes, not assistant completion
   statements. Inspect per-task failures before aggregate metrics. Reject safety
   regressions and duplicate side effects regardless of token savings.
5. Add only enough repeated trials to resolve observed variability. Expand when
   evidence warrants it, not to optimize a leaderboard or inflate test counts.

**Acceptance:** replayable artifacts with metric provenance; no correctness/safety
regression; efficiency claims backed by comparable real runs.
**Stop:** the decision is clear or further runs require authorization.
**Handoff:** task/commit IDs, model/settings, changed paths, checks, metric source,
failure classes and next smallest hypothesis to test.
