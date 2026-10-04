# Production Harness Readiness Plan

> **For agentic workers:** Tasks use checkbox (`- [ ]`) syntax for tracking. This document is a plan only. It contains no code. Every task names the existing code it builds on, the behavior to add or change, the tests to add, and the evidence that proves the change helped.

**Goal:** Make DaVinci beat Codex on task success, not only on efficiency, and reach Claude Code-level production behavior (control, recoverability, safe autonomy, cost honesty) without adding new subsystems.

**Principle:** Finish, connect, measure, subtract. Most of the missing behavior already exists in the tree but is switched off, not wired to the main conversation, or unmeasured. New architecture is out of scope unless an evaluation shows a gap that nothing existing can close.

**Baseline:** `main@2e52d0f68f9cc989fc69a6114409e237e1990158` (installed). Benchmark evidence was measured on `e8a276ec2f0f2e1dd6408c4fc6d03199a9401586`.

---

## 0. Where we are (evidence)

### 0.1 Benchmark result (GPT-5.6 Luna, high effort, 12 public tasks × 3 reps)

| Metric | DaVinci | Codex |
|---|---:|---:|
| Composite success | 22/36 | 22/36 |
| Legacy tasks | 22/24 | 22/24 |
| Large tasks | 0/12 | 0/12 |
| Median task time | 61.25 s | 165.75 s |
| Uncached input tokens | 674,536 | 1,472,110 |
| Output tokens | 130,169 | 292,033 |
| Runs with unrelated edits | 1 | 3 |

Efficiency lead: real. Accuracy: tied. Superiority claim: not supported. The repository's own competitor gate (`docs/competitor-benchmarks.md`) requires at least 150 shared scenarios.

### 0.2 Why both harnesses fail the large tasks

The hidden graders in `scripts/bench/make_large_tasks.py` test clauses that the prompt states and the public tests do not cover:

- **m1:** BOM handling, rejecting `"  = value"`, keeping `=` inside values, not mutating `defaults`, correct override order.
- **m2:** the public test itself must change, because it imports the name the task says to remove from `userlib`. "Keep the existing tests green" is the wrong stop condition here.
- **m3:** the old file stays intact when `os.replace` fails, per-scope idempotency, rejection of invalid inputs.
- **m4:** exit status 2 on every invalid input, `INVENTORY_TAX`, descending total sort with name tie-break.

DaVinci's completion gate (`crates/davinci-agent/src/turn.rs:730-810`) accepts completion once any verification passes after the last mutation. After a partial fix the public test passes, so the gate is satisfied and the model stops.

The prompt guidance aimed at this ("compare every explicit requirement with a concrete input…", normal/boundary/invalid inputs) exists only in the Preview profile (`crates/davinci-agent/src/prompt/bundle.rs:112-125`, `verification.rs` `verification_requirement_checks_module`). Stable is the default (`crates/davinci-coding-agent/src/prompt_host.rs:47-50`), and the benchmark ran Stable.

Caution: a few hidden checks encode implicit behavior (for example m2 trims `" Ada "`). Do not tune toward individual hidden assertions. Tune toward the general behavior: requirements mapped to evidence.

### 0.3 Verified gaps in the current tree

| Area | Current state | Reference |
|---|---|---|
| Completion gate | Only checks "some check passed after the last edit" plus debugging/frontend capability evidence | `turn.rs:730-810`, `prompt/capabilities/gate.rs` |
| Requirement prompt | Only in Preview profile, not default | `prompt/bundle.rs:112` |
| User `stop` hooks | Observe-only; run once at process/run end | `hooks.rs:12`, `hooks.rs:550`, `main.rs:5085-5098` |
| Plugin `Stop` hooks | Run per prompt, but as observers after the loop has ended, so a Claude Code `decision: block` cannot make the agent continue | `main.rs:3170-3174`, `plugins/hooks.rs` |
| PostToolUse hooks | Observe-only, fail open | `hooks.rs` runtime subscriber |
| Rewind | Content-addressed preimages are captured for write/edit (`turn.rs:2261`); three-way rewind and modal exist; reachable only from `/graph` and tests | `runtime/rewind.rs`, `davinci_interactive.rs:5758` |
| Sandbox | Off by default; only Linux bubblewrap enforces; Windows and macOS fail closed | `docs/sandbox.md` |
| Background model spend | Learning reviewer on by default (≥180 s apart, up to 6 iterations, auto-applies project and global skills); security watch on by default (≥600 s apart); neither runs in `-p`, so benchmarks never see their cost | `learning/config.rs:20-36`, `security_scan/config.rs:34-47` |
| Tool surface | `Full` is default; `Lean` + `tool_search` exists but is opt-in | `settings.rs` `toolSurface`, `DAVINCI_TOOL_SURFACE`, `docs/tool-discovery.md` |
| Provider tuning | Only OpenAI reasoning models have a prompt adapter | `prompt/provider.rs:62-70` |
| CI | `main` red at `2e52d0f`: Windows `davinci-coding-agent` shard, test `graph_worker_node_resolves_relative_script_from_canonical_workspace`; the child stayed `running` with no output for the full 30 s window | `native_extensions/graph/coordinator_handler.rs:320-352`, Actions run 36651481391 |
| Release | No tags, no release workflow; self-update still names upstream pi's npm package and cannot update DaVinci | `self_update.rs:8-10`, `packages.rs:286-293` |
| Surface sprawl | 14 native `*-status` commands plus `/status` and `/sandbox-status` | `native_extensions/mod.rs` `command_specs` |
| Size | ~476k lines of Rust; CLAUDE.md says the coding-agent crate is ~47k lines, it is ~197k | `crates/*` |

---

## Global constraints

- Tests stay fixture-only and offline (`DAVINCI_OFFLINE` / `PI_OFFLINE`, existing fixture hooks). Add a fixture hook rather than a live call.
- Never skip, disable, or quarantine a test to get green.
- Keep TypeScript parity contracts: compaction and branch-summary prompt strings are not reworded; cite the TS source when touching mirrored modules.
- Keep Rust 1.83 and exact `=` dependency pins. This plan needs no new dependencies.
- Stable prompt text changes go through the existing prompt-promotion process (`.github/workflows/prompt-promotion.yml`).
- Harness reminders are appended after the cached prefix. They must not change the Stable prompt hash or the frozen tool schema.
- Benchmark numbers are measured, never estimated. Every claim names its campaign directory, binary hash and commit.
- Preserve worker isolation, fail-closed security, and "project settings may narrow, never widen" everywhere.

---

## Phase 0 — Green main and release discipline

### Task 0.1: Diagnose and fix the Windows graph-worker test failure

- [ ] Reproduce on a Windows runner (or locally on Windows) with the full `davinci-coding-agent` library suite running in parallel, not the test alone.
- [ ] Decide which of these it is, from evidence:
  - the Node child never started running the script (spawn/cwd/`\\?\` verbatim-path issue with `node relative.cjs`);
  - the child is blocked (inherited handle, stdin wait, job-object interaction);
  - runner starvation under parallel process-heavy tests.
- [ ] Fix the cause. Raising the timeout is acceptable only if starvation is proven, and then paired with limiting that test group's parallelism.
- [ ] Rerun the Windows shard until it is green on the fix commit.

**Done when:** CI is green on `main`, and the root cause is written in the commit message.

### Task 0.2: Ship only from green commits

- [ ] Rule: the installed or benchmarked binary must come from a commit whose full CI (all shards, clippy, fmt) is green.
- [ ] Add version tags for installed releases, starting from the product version in `Cargo.toml` (1.0.71).
- [ ] Record the tag, commit, CI run and binary hash in the installation identity file that the install script already writes.

**Done when:** every installed binary maps to a tag with a green CI run.

### Task 0.3: Make self-update honest

- [ ] Either point `davinci update` for the CLI itself at DaVinci's own release source, or make it state plainly that DaVinci updates by reinstall from source and name the install script.
- [ ] Remove the upstream pi package name and pi-mono release link from user-facing output.
- [ ] Keep the TS-parity paths that handle pi packages (not the CLI itself) unchanged.

### Task 0.4: Subsystem freeze

- [ ] No new subsystems, tools, or slash commands until Phase 1 evaluation exists and Phase 2 is measured. Bug fixes and wiring of existing parts are allowed.

---

## Phase 1 — An evaluation that can support the claim

Everything later is accepted or rejected against this. Build it first.

### Task 1.1: Private task suite mined from real repositories

- [ ] Choose 3–5 repositories you own and actively work on (private preferred, to avoid training-data contamination). Mix languages you actually ship (for example Rust, TypeScript, Python).
- [ ] Select merged commits or PRs that change source code and add or change tests.
- [ ] For each one:
  - the starter is the parent commit;
  - the hidden grader is the tests added or changed by that commit;
  - the prompt is written by a human from the PR description, never generated from the diff;
  - the allowed files are the files the commit touched plus their test files.
- [ ] Validate each task the way `scripts/bench/bench.py validate` does today: the starter fails the hidden grader, and the reference commit passes it.
- [ ] Keep the existing 12 public tasks as a smoke set, not the decision set.
- [ ] Target sizes:
  - **dev split:** 40–60 tasks, used for iteration;
  - **holdout split:** at least 150 tasks, never used for tuning, run only to confirm a change and to support a claim.
- [ ] Label each task by size class (small/large), by whether it has visible tests, and by whether it requires changing existing tests.

### Task 1.2: One metric set, reported the same way every time

Reuse `scripts/bench` (campaign, report, gate, compare) and report per arm and per size class:

- [ ] Composite success (existing `task_success`: exit 0, grader pass, no unrelated edits, cleanup complete, no transaction/artifact leaks).
- [ ] Regression-free success, where the repository's pre-existing tests still pass.
- [ ] Unrelated-edit rate.
- [ ] Uncached input, cached input and output tokens per verified success, reported separately.
- [ ] Estimated cost per verified success from a pinned price table, labeled as an estimate.
- [ ] Median and P90 wall time.
- [ ] Model requests and tool calls per task.
- [ ] For interactive/RPC measurements only: background model tokens (learning reviewer, security watch), reported separately.

### Task 1.3: Baselines

- [ ] DaVinci vs Codex on the same OpenAI model and effort (existing runner).
- [ ] DaVinci vs Claude Code on the same Claude model, using the existing `competitor-differential.yml` path. This is the first time the Claude model path gets measured.
- [ ] Freeze each baseline campaign with its binary hashes before tuning starts.

### Task 1.4: Iteration protocol

- [ ] Every change in Phases 2–5 is an A/B on the dev split, with paired tasks, 3 reps and counterbalanced order (existing).
- [ ] A change is promoted only if:
  - dev-split success does not drop in any size class;
  - it improves its target metric;
  - a holdout confirmation run agrees.
- [ ] Report the bootstrap intervals the bench already computes. With small samples, say "no measurable difference" rather than claiming a win.

---

## Phase 2 — Completion correctness (the accuracy work)

This is the phase expected to move large-task success. It spends part of DaVinci's token lead on one extra, targeted model turn.

### Task 2.1: Requirement-coverage completion step

**Builds on:** the existing completion path in `turn.rs:698-810` and the reminder channel `queue_capability_reminder` (`turn.rs:884`), which is already bounded and kept out of the saved final answer.

**Behavior:**

- [ ] Trigger it the first time the model tries to finish (an assistant message with no tool calls) when all of these hold:
  - the run changed files;
  - the permission mode is not Plan Mode;
  - the task is multi-part: at least two changed files, or a request with several separate requirements. Start with a simple deterministic rule on the request text (count of sentences or clauses joined by commas or semicolons) and tune the threshold on the dev split. A false trigger costs one turn.
- [ ] Send one harness reminder asking the model to:
  1. list each explicit requirement in the user's request;
  2. name the test or command that demonstrates each one;
  3. write tests for any requirement with no evidence, placing them in existing test files where the project has them;
  4. update existing tests that the request makes obsolete, instead of preserving them;
  5. rerun the checks, then finish.
- [ ] Attach the harness-computed change set to the reminder: files changed since the run started, with new files marked (see Task 2.2).
- [ ] Limit it to once per user prompt. The existing verification gate still runs afterwards, because new tests change the mutation generation.
- [ ] Respect abort, steering and follow-up queues exactly as the existing reminders do.
- [ ] Record a reason code (for example `completion.requirements`) in `RunStats` and the event stream so `/status`, `get_session_stats` and the bench can count triggers.

**Tests (fixture-only, alongside the existing `turn.rs` tests):**

- [ ] triggers once for a multi-file mutation run and never twice in one prompt;
- [ ] does not trigger for read-only runs, Plan Mode, or single-file trivial edits below the threshold;
- [ ] does not change the Stable prompt hash or the provider tool schema;
- [ ] the final saved answer and session file do not contain the reminder text (mirror the existing `verification_notice.rs` test);
- [ ] works identically in print, RPC and interactive modes.

**Acceptance:**

- [ ] On the dev split, large-class composite success improves versus the frozen baseline, legacy/small success does not drop, and median uncached tokens per task rise by no more than an agreed ceiling (proposed 25%).
- [ ] Holdout confirms.

### Task 2.2: Change-set review in the same step

**Builds on:** mutation path tracking (`record_successful_mutation_paths`) and git status. Shell-made changes are not tracked by tool mutation records, so take a git status snapshot at run start and diff against it.

- [ ] First inspect the one DaVinci run with an unrelated edit (`unrelated` field in the campaign rows) and the three Codex ones. Design from the actual cause: a scratch script, a new test file outside the allowed list, or a formatter touching extra files.
- [ ] Include the change-set list, with new untracked files flagged, in the Task 2.1 reminder so the model removes temporary files it created.
- [ ] Do not delete anything automatically. The model decides; the harness only shows the facts.

**Acceptance:** the unrelated-edit rate does not rise, and ideally falls, on the dev split.

### Task 2.3: Preview prompt: promote or drop

- [ ] Run three arms on the dev split: Stable, Stable + Task 2.1, Preview + Task 2.1 (`--prompt-profile preview` / `DAVINCI_PROMPT_PROFILE`).
- [ ] Choose the simplest arm that wins.
- [ ] If Preview's modules add nothing beyond Task 2.1, remove the candidate modules rather than keep a dormant profile.
- [ ] If they help, promote them through the prompt-promotion workflow with its evidence record.

### Task 2.4 (conditional): Bind cited evidence to receipts

Do this only if Task 2.1 traces show the model citing checks it never ran.

- [ ] Have the model answer the reminder in a small structured form (requirement → command).
- [ ] The runtime checks each cited command against host-owned command receipts (`command_receipt.rs`) after the last mutation generation.
- [ ] An unbacked item gets one more reminder, then completion is allowed with an explicit "unverified" notice (existing `VerificationNotice` path).

---

## Phase 3 — User control

### Task 3.1: Hooks that can refuse completion and give feedback

**Builds on:** `hooks.rs` (user hooks), `plugins/hooks.rs` (Claude Code-format hooks, which already parse exit 2 and `decision: block`), and the Task 2.1 reminder channel.

- [ ] Add a per-prompt completion hook point inside the agent loop, at the moment the model tries to finish and before the loop ends. Today's user `stop` hooks keep their run-end meaning for compatibility. The new point gets a distinct, documented event name for user hooks.
- [ ] Run plugin `Stop` hooks at this new in-loop point instead of after the loop, so a Claude Code plugin's block actually takes effect.
- [ ] Block semantics match Claude Code: exit 2 (stderr is the reason) or exit 0 with `decision: block` and a `reason`. The reason is delivered to the model through the reminder channel, and the loop continues.
- [ ] Loop guard:
  - tell the hook when it is already running in a continuation caused by a previous block (Claude Code's `stop_hook_active` field);
  - cap consecutive blocks per prompt (proposed 3);
  - after the cap, finish with a visible notice.
- [ ] PostToolUse feedback: exit 2 appends the hook's stderr to that tool result as feedback. Other failures remain non-blocking warnings, as they are today.
- [ ] Trust rules stay unchanged: project hooks only in trusted projects, plugin hooks only while `hooksApproved` matches, and never in graph workers.
- [ ] Tests: block then continue, the block cap, the `stop_hook_active` flag, untrusted project ignored, plugin hook digest unapproved ignored, print/RPC/interactive parity.
- [ ] Document one recipe in `docs/`: "make the repo's test/lint command the finish line".

**Acceptance:** a Claude Code plugin that uses a blocking Stop hook behaves the same in DaVinci, and all existing hook tests still pass.

### Task 3.2: `/rewind` for normal conversations

**Builds on:** per-call preimage capture in `turn.rs:2255-2280` (blob store), `runtime/rewind.rs` (three-way plan, conflict detection, `can_apply_rewind`), the existing TUI rewind modal (`davinci-tui/src/davinci/views/rewind.rs`, `open_rewind_modal`), and session tree/fork.

- [ ] Group captured preimages by user prompt, so each prompt is one checkpoint.
- [ ] Add `/rewind` to `builtin_slash_commands()`. It lists recent prompts and offers to restore code, conversation, or both.
- [ ] Offer rewind from double-Escape. Today `double_escape_action` defaults to `tree`; keep `tree` as a configurable option.
- [ ] Use the existing conflict rules: files edited by the user since the checkpoint show as conflicts and are never silently overwritten.
- [ ] State the limit plainly in the UI and docs, as Claude Code does: changes made through shell commands are not tracked.
- [ ] RPC: expose rewind preview and apply.
- [ ] Tests: restore after write/edit/apply_patch, conflict on user edit, conversation-only restore, code-only restore, and the shell-change limitation message.

**Acceptance:** a user can undo the last N prompts' file edits from the main conversation without touching git.

---

## Phase 4 — Safe autonomy (sandbox)

Goal: Auto mode approves more because commands are contained, not because the checks are looser.

### Task 4.1: macOS Seatbelt backend

**Builds on:** the sandbox broker, capability negotiation and receipts from the 2026-09-29 sandboxed execution plane (`docs/superpowers/specs/2026-09-29-sandboxed-execution-plane-design.md`, "macOS" section).

- [ ] Add a backend that generates a Seatbelt profile from the existing sandbox spec: workspace read or read/write, read-only runtime mounts, `.git`/`.davinci`/`.pi` read-only, private temp, network denied unless allowed.
- [ ] It advertises only the capabilities it actually enforces. Memory/PID limits remain capability-gated.
- [ ] Mirror the Linux bubblewrap enforcement tests: write outside the workspace denied, network denied, secret environment variables absent, `.git/config` not writable, teardown complete.
- [ ] Run these on a macOS CI runner.

### Task 4.2: Sandbox on by default where a backend exists

- [ ] When a capable backend is available (bubblewrap on Linux, Seatbelt on macOS), Auto mode defaults to `workspace_write` with network denied.
- [ ] Commands that need network or paths outside the workspace go through the existing approval path.
- [ ] Where no backend exists, behavior is unchanged and `/status` says so.
- [ ] Measure the prompt count per task in Auto mode before and after on the dev split, plus success and wall time. Success must not drop.

**Decision needed (Windows):** choose one.

- **(a) Document WSL2 as the sandboxed path now.** Native Windows keeps Job Object ownership without isolation claims.
- **(b) Build a restricted-token/ACL backend.** This is larger, and it must follow the same "advertise only what is enforced" rule.

Recommendation: (a) now, and (b) only if Windows-native autonomy is a product requirement after Phase 2 results.

---

## Phase 5 — Cost honesty and simplification

### Task 5.1: Background model calls

- [ ] Make learning background review and the security watch visible in `/cost`, `/status` and `get_session_stats` as separate token lines.
- [ ] Choose a default and record the choice here:
  - **(a) Off by default, opt-in** (recommended until Phase 1 shows value);
  - **(b) On, but routed to a configured small model**, never the session's model by default.
- [ ] Measure on interactive/RPC dev-split runs whether learning or the security watch changes success, unrelated edits or security findings. Keep a feature on by default only if it measurably helps.

### Task 5.2: Learning scope

- [ ] Stop auto-applying learned skills globally without approval (`auto_apply_global`). Default global promotion to require approval; keep project-scoped auto-apply only if the evaluation supports it.
- [ ] Reason: a wrong lesson learned in one repository should not silently change behavior in every other one.

### Task 5.3: Lean tool surface A/B

- [ ] Run Full (current default) vs Lean + `tool_search` on the dev split for both the OpenAI and Claude paths.
- [ ] If Lean is non-inferior on success and lower on tokens, make it the default. Keep Full selectable (`toolSurface`, `DAVINCI_TOOL_SURFACE`).

### Task 5.4: Measure-or-cut audit

For each subsystem that is on by default or advertised to the model (vector memory injection, learned-skill injection, token governor digests, security watch, repo/LSP/package/build/git specialist tools, graph), run an on/off arm on the dev split.

- [ ] Keep it on by default only if it improves success, tokens or latency without hurting the others.
- [ ] Otherwise move it off the default path and document it as opt-in.
- [ ] Record each result in a table in this plan.
- [ ] Context VM stays off. Evaluate Context VM only on a dedicated long-session set (sessions long enough to compact at least twice) against current compaction; change the default only on a measured win.

### Task 5.5: Collapse the status surface

- [ ] Fold the 14 native `*-status` commands into sections of `/status`, plus a `/doctor` command that checks configuration, credentials, sandbox backend, MCP/LSP health and install identity.
- [ ] Keep the internal views for RPC and sheets. Remove them from public slash discovery, following the `/security-scan` precedent in CLAUDE.md.

### Task 5.6: Keep CLAUDE.md factual

- [ ] Correct the crate line counts and add the new behaviors (completion step, blocking hooks, `/rewind`, sandbox defaults) once they land.

---

## Phase 6 (conditional) — Claude model path

Start this only if Task 1.3 shows DaVinci behind Claude Code on the same Claude model.

- [ ] Compare traces for tool choice, edit format errors, over- or under-exploration, and stopping behavior.
- [ ] Add a small Anthropic prompt adapter only for behaviors the traces show, sized like the OpenAI adapter (`PROVIDER_ADAPTER_MAX_TOKENS`), and promote it through the same A/B protocol.

---

## Explicitly not in this plan

- New subsystems (new memory layers, new orchestration modes, more graph features).
- A separate reviewer subagent on every task. Task 2.1 gets the benefit from one extra turn of the same agent.
- A trained or LLM-based requirement extractor. The main model writes the checklist; the runtime only enforces that the step happens.
- IDE extensions, cloud execution, and a GitHub Action wrapper. Revisit after Phases 0–3 if users ask.
- Domain-allowlisted networking in the sandbox (the sandbox docs deliberately leave it unsupported).

---

## Order, dependencies and size

Sizes: S = wiring existing parts, M = new behavior on existing infrastructure, L = new backend or large data work.

| Order | Task | Depends on | Size |
|---:|---|---|:---:|
| 1 | 0.1 Fix Windows CI failure | — | S |
| 2 | 0.2 Release from green, tags | 0.1 | S |
| 3 | 0.3 Honest self-update | — | S |
| 4 | 1.1–1.4 Private eval suite + baselines | — | L |
| 5 | 2.1 Requirement-coverage step | 1.x for acceptance | M |
| 6 | 2.2 Change-set review | 2.1 | S |
| 7 | 2.3 Preview promote-or-drop | 2.1 | S |
| 8 | 3.1 Blocking Stop / PostToolUse hooks | 2.1 (reminder channel) | M |
| 9 | 3.2 `/rewind` in normal sessions | — | M |
| 10 | 5.1–5.2 Background spend + learning scope | 1.x | S |
| 11 | 5.3 Lean tool surface A/B | 1.x | S |
| 12 | 4.1 macOS Seatbelt backend | — | L |
| 13 | 4.2 Sandbox default in Auto | 4.1 | S |
| 14 | 5.4 Measure-or-cut audit | 1.x | M |
| 15 | 5.5–5.6 Status consolidation, docs | — | S |
| 16 | 2.4 / 6 Conditional tasks | evidence | M |

Phase 1 can run in parallel with Phase 0 and with Tasks 3.2 and 4.1, since none of these need its results to be built. They need it only to be accepted.

---

## Definition of done

DaVinci may be called "better than Codex" or "Claude Code level" for a model only when all of these hold:

- [ ] CI is green on the released tag.
- [ ] On the ≥150-task private holdout, same model and effort, 3 reps:
  - composite success is higher than the competitor, with the bootstrap interval's lower bound above zero;
  - unrelated-edit rate is no higher;
  - median wall time is no more than 15% above the competitor (the existing competitor gate);
  - tokens and estimated cost per verified success are reported.
- [ ] Large-class success is reported on its own, not hidden inside the average.
- [ ] Interactive background model spend is reported next to print-mode numbers.
- [ ] `/rewind`, blocking hooks, and the sandbox default (where a backend exists) are shipped and documented.

## Risks

- **Overfitting to public tasks.** Mitigation: dev/holdout split; the public 12 are a smoke set only.
- **Extra turn raises cost without helping.** Mitigation: trigger threshold tuned on the dev split, token ceiling in acceptance, promote-or-drop.
- **Blocking hooks cause loops.** Mitigation: `stop_hook_active` flag and a consecutive-block cap.
- **Sandbox default breaks builds that need the network.** Mitigation: the existing approval path for network, measured prompt counts, and full-access remains an explicit escape hatch.
- **Small samples mislead.** Mitigation: report intervals and say "no measurable difference" when that is the result.

## Decisions needed from the owner

1. Which repositories feed the private evaluation suite (Task 1.1).
2. Background learning and security watch default: off/opt-in or cheap model (Task 5.1).
3. Windows sandbox path: WSL2 documentation now, or a native backend (Phase 4).
4. Token ceiling for the requirement step's acceptance (proposed 25%, Task 2.1).
5. Live campaign budget per phase (model usage for dev-split A/Bs and holdout confirmations).

---

## Execution record (added September 30, 2026)

The original attachment was recorded as 27,547 bytes (406 lines), SHA-256 `003f29097f92d2ac99b80ebce0f266f35f685e06252e577327e2847bbf9d5380`. That hash identifies the original source before subsequent documentation cleanup, not the current edited text. This appendix is implementation evidence, not a replacement specification. Full coverage and checks are recorded in [readiness evidence](README.md) and the [99-requirement ledger](requirements-evidence.json).

Task 5.1 default: **off by default, opt-in** for learning background review and security watch. Task 5.2: both global and project skill activation require approval absent evaluation evidence. Windows: document WSL2 now; native Windows has no isolation claim. The proposed 25% token ceiling has not been approved by the owner. Repository selection, human prompts and campaign budget remain outstanding. No paid campaigns were run.

Task 5.4 audit results:

| Subsystem | On/off evaluation result | Default decision |
| --- | --- | --- |
| Vector memory injection | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Learned-skill injection | Not run; private inputs/budget pending | Automatic project/global activation requires approval |
| Token governor digests | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Security watch | Not run; interactive/RPC inputs/budget pending | Off, opt-in |
| Background learning reviewer | Not run; interactive/RPC inputs/budget pending | Off, opt-in |
| Repo specialist | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| LSP specialist | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Package specialist | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Build specialist | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Git specialist | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Graph | Not run; private inputs/budget pending | Existing control retained; no measured keep/cut claim |
| Context VM | Not run; dedicated sessions with at least two compactions required | Off |

No results justify selecting Preview, promoting Lean, adding a Claude adapter, or claiming competitor superiority. Native Seatbelt filesystem/network restrictions cannot establish whole-process-tree ownership, so macOS Auto acceptance is blocked rather than advertised.
