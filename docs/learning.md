# Davinci Self-Improving Learning System

The Davinci Self-Improving Learning System turns settled Davinci agent turns into durable vector memory and reusable procedural skills (`SKILL.md`). It operates with a fail-open loop designed so that learning operations never block, crash, or interrupt normal foreground agent turns.

---

## 1. Conceptual Split

Davinci maintains a strict distinction between declarative facts and procedural workflows:

- **Memory**: Durable declarative facts (repository architecture, conventions, constraints, bug causes). Stored and retrieved via the repo-aware `VectorMemory` system with dense embedding and lexical fallbacks.
- **Skills**: Reusable procedural workflows (`SKILL.md` files). Stored in standard skill directories and invocable via `/skill:name` or model-selected via progressive disclosure tools.
- **Candidates**: Proposed learning items extracted from completed turns that have not yet met the policy threshold for autonomous activation.
- **Verification**: Deterministic ground truth from real command executions (`bash`, `powershell`, or graph worker verification) required to auto-promote candidates.

---

## 2. Core Principles & Safety Model

### Fail-Open Asynchronous Loop
- Background review is off by default (`learning.backgroundReview: false`). When explicitly enabled, after a turn that passes `should_review_evidence`, the interactive shell and RPC sessions start a model review on a background thread. The review runs this `davinci` as a print-mode child (`-p --mode json --no-session --no-extensions --no-skills --no-mcp --no-tools`) on the session's model, with `REVIEWER_SYSTEM_PROMPT` and the turn evidence (`reviewer.rs::execute_live_review`).
- One review runs at a time, and reviews start at least `minReviewIntervalMs` apart (default 3 minutes). The evidence covers the last ten messages, so a turn skipped for spacing is mostly seen by the next review. A new turn does not cancel a running review.
- The child runs with `PI_LEARNING_DISABLE_BACKGROUND=1`, `DAVINCI_SECURITY_WATCH=0`, and `PI_MEMORY_ENABLED=0`, so it neither dispatches further background reviews nor indexes the review into vector memory.
- Finished reviews are applied at the start of the next turn and by any `/learning-*` or `/skill-*` command. A review still running when the process exits is lost.
- Print mode (`davinci -p`, graph workers) does not start model reviews. When review is opted in, `PI_LEARNING_REVIEW_FIXTURE` answers synchronously in every mode for tests.
- Any background failure, parse error, or timeout (`reviewTimeoutMs`, default 2 minutes) is swallowed and recorded in diagnostics; the learning section of `/status` shows the last one. Foreground turns are never blocked or failed by learning.
- Review can be completely disabled by setting `PI_LEARNING_DISABLE_BACKGROUND=1`.

### Approval by default
- `autoApplyProject` and `autoApplyGlobal` default to `false`. Valid candidates are staged for `/learning-approve <id|all>` before activation. Project auto-apply remains opt-in until private evaluation shows a benefit; global promotion always requires approval unless the owner explicitly changes that setting.
- Enabling background review does not enable automatic promotion. Opt-in belongs in the owner's agent-directory settings: trusted project settings can narrow `enabled`, `backgroundReview`, `autoApplyProject`, and `autoApplyGlobal`, but cannot grant permission that the owner did not enable. Existing owner opt-ins retain the verification, ownership, hash, and path checks.
- Project configuration cannot disable owner shadow mode, increase review input, iteration, candidate, or timeout limits, shorten the review interval, or lower the verified-use promotion threshold. Smaller review budgets and longer intervals remain available per project.
- Declarative memory facts and failure lessons need confidence of at least 0.80; procedural workflows also need successful command verification. Both obey the scope's approval setting.
- User-authored and imported skills remain protected. Executable `scripts/` support files are never written unattended, missing patch targets are rejected, and duplicate active skills are kept as candidates.
- **Use in later turns**: before each turn, learned skills (ledger status `active`, origin learned) that match the prompt are injected beside precision-gated vector memory, up to 2 skills and 1,200 tokens (`LearningController::learned_skill_block`). Vector memory no longer persists raw settled-turn chat: learning promotes compact typed claims, pinned constraints are bounded, ordinary automatic recall requires a shared code anchor, a clear lead over the runner-up and an unchanged source file, and broader recall remains available through `memory_search` (details in `docs/vector-memory.md`).
  - Maintains versioned history backups (up to 5 versions) under `<store>/history/<skill>/<version>.md`.

---

## 3. Architecture & Turn Lifecycle

```text
[Agent Turn Settled]
         │
         ▼
[Vector Memory Indexing]
         │
         ▼
[Deterministic Verification Evidence & Skill Outcomes]
         │
         ▼
[Background Review Dispatch (Worker Thread)]
  ├── Extract tool calls, diffs, user prompts, verification results
  ├── Evaluate Candidate Artifact (SkillCreate / SkillPatch / Memory / FailureLesson)
  ├── Policy Evaluation:
  │     ├── Shadow Mode? ──► Persist as Candidate (candidates.jsonl)
  │     ├── Unverified / user-owned / corrected procedure? ──► Keep as Candidate
  │     └── Trusted & Verified & Auto-Apply? ──► Apply to SKILL.md & Ledger & Vector Memory
  └── Queue Progressive Disclosure Notice
```

---

## 4. Storage Layout & Ownership

The learning subsystem maintains stores at two scopes:
- **Project Scope**: `<agent_dir>/learning/projects/<repo-key>/`, where `<repo-key>` is the first 16 hex digits of the hash of the repository id. The repository cannot write there, so learned project skills need no project trust, and learning never adds trust-requiring files to the repository. A pre-existing in-repository store (`<repo>/.davinci/learning/` or `<repo>/.pi/learning/` whose `skills.jsonl` is not empty) keeps being used, together with its `<repo>/.davinci/skills/` directory.
- **Global Scope**: `<agent_dir>/learning/` (e.g. `~/.pi/agent/learning/`).

### Directory Structure
```text
<store_root>/
├── candidates.jsonl          # Log of proposed learning candidates
├── skills.jsonl              # Skill ledger tracking versions, counts, and verification
├── state.json                # Atomic snapshot of store state and compaction metadata
└── history/                  # Bounded numerical rollback backups (latest 5 versions)
    └── <skill_name>/
        ├── 1.md
        └── 2.md
```

### Skills Directory
Active procedural skills are persisted as standard skill directories containing `SKILL.md`:
- Project skills: `<agent_dir>/learning/projects/<repo-key>/skills/<skill_name>/SKILL.md`
- Global skills: `<agent_dir>/skills/<skill_name>/SKILL.md`

### Autonomous Modification Boundaries
- Autonomous background review may only mutate Davinci-owned learned artifacts (`LearnedReview`).
- Autonomous background review cannot mutate user-authored (`SkillOrigin::User`) or imported (`SkillOrigin::Imported`) skills.
- Autonomous background review cannot write executable `scripts/` inside skills.

---

## 5. Slash Commands & Progressive Disclosure

### Foreground Interactive Commands

| Command | Description |
| :--- | :--- |
| `/learn [--global] <instruction>` | Distills a reusable procedure into a project or global skill in the foreground. Searches existing skills first to prefer patching over duplicates. |
| `/status` (learning section; `learning-status` remains an internal view) | Displays current learning configuration, shadow mode status, project trust, candidate stats, and active skills count. |
| `/learning-pending` | Lists candidates staged for approval after an opted-in review. |
| `/learning-approve <id\|all>` | Approves a staged candidate, activating the skill/memory and updating the ledger. |
| `/learning-reject <id\|all>` | Rejects a staged candidate, marking it as dismissed. |
| `/skill-list [query]` | Lists compact descriptors of known skills across project and global scopes with versions and usage counts. |
| `/skill-view <name> [file]` | Displays the contents and ledger metadata of a specific skill or support file. |

### Progressive Disclosure
When background reviews stage candidates or activate skills, notifications are buffered and drained into the interactive transcript without corrupting the TUI screen:
- `"learning · skill activated: debug-sqlx"`

---

## 6. Configuration & Defaults

Learning configuration is specified under the `"learning"` key in settings:

```json
{
  "learning": {
    "enabled": true,
    "backgroundReview": false,
    "shadowMode": false,
    "autoApplyProject": false,
    "autoApplyGlobal": false,
    "maxCandidatesPerReview": 3,
    "maxReviewInputTokens": 12000,
    "maxReviewIterations": 6,
    "autoPromoteVerifiedUses": 2,
    "reviewTimeoutMs": 120000,
    "minReviewIntervalMs": 180000
  }
}
```

### Failure Behavior
- **Fail-open**: Learning failures never fail normal turns.
- **Review disablement**: Setting `PI_LEARNING_DISABLE_BACKGROUND=1` immediately short-circuits background review execution.
- **Fallback to lexical**: If Ollama or Qdrant are unavailable, skill retrieval cleanly falls back to lexical matching.
- **Untrusted projects**: Learned project skills live in the Davinci-owned store, so trust does not gate them. A legacy in-repository store in an untrusted project receives no autonomous project writes.
- **Cost**: an opted-in review adds provider requests on the session's model, with input bounded by `maxReviewInputTokens`, only for turns that pass review gating and the interval.
- **Activation**: high-confidence memories and verified skills require approval by default. Autonomous activation additionally requires an explicit scope auto-apply setting.

---

## 7. Graph Learning Integration & Ecosystem Gates

The Davinci learning system deeply integrates with the Graph pipeline to create a closed, verifiable self-improvement loop:

### Review Gating (`should_review_evidence`)
To eliminate wasteful background reviewer model calls on low-signal turns, `should_review_evidence` evaluates incoming evidence before dispatching reviewer execution. A turn is reviewed only when at least one of five high-signal conditions is met:
1. **Explicit Request**: Turn contains `/learn` or `learn:`.
2. **User Correction / Rejection**: User expresses correction (e.g. "that's wrong", "undo", "revert") or permission was denied.
3. **Graph Run Execution**: A graph run completed (`graph_run_id` present) or `graph_run` tool was called.
4. **Deterministic Commands Ran**: Verification commands executed (`commands_ran > 0`).
5. **Skill Verification Outcome**: An injected skill received verified outcome or user acceptance.

This deterministic gate limits review eligibility. Its effect on task success,
tokens, and learning quality still needs live evaluation.

### Exact Skill Version Provenance & Attribution
Skills injected into graph worker context carry immutable version references:
```rust
pub struct SkillVersionRef {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
}
```
At graph completion, `VerificationBundle` derives a deterministic outcome (`VerifiedSuccess`, `VerifiedFailure`, or `Neutral`). The outcome is recorded against the exact injected version (`record_skill_version_outcome`), guaranteeing that successes or regressions are never misattributed across edits.

### Capability Toolbox Closed Loop
Verified skill outcomes update the existing `SkillLedgerRecord` counters. Future Capability Toolbox selection consumes the same ranking path, so successful skill versions gain selection weight and failed versions are penalized. There is no separate toolbox learner or duplicate skill database.

Capability Toolbox never rewrites user-authored or imported skills; ownership rules remain in the Learning subsystem. Tool authority remains with the existing graph role policy, while toolbox assembly adds no model calls or storage.

### Conditional Security Gate (`securityVerification`)
Graph configurations support configurable security verification modes (`off | risk | always`, default `risk`):
- `off`: Security scanning bypassed (`SecurityVerification::NotRequired`).
- `risk`: Graph mutations are classified deterministically by `assess_change_risk`. High-risk mutations (authentication, cryptography, credentials, permission policies, process execution, manifests) trigger non-interactive changed-surface security verification (`verify_changed_surface`). If scanning is unavailable, fails open with warning.
- `always`: Every mutated run triggers security verification. Unavailable scanner fails closed.

When security verification fails, review approval is blocked (`bundle.approval_eligible == false`), security diagnostics are injected into revision notes, and additional revision cycles are required.

### Closed-Loop Learning Proof
The fixture explicitly enables background review and project auto-apply:

1. **Run #1**: Performs verified workflow with no prior learned skills; verification bundle triggers settled turn review, persisting a high-confidence memory and a new versioned skill (`SKILL.md` + ledger record).
2. **Run #2**: A related goal is launched; `build_context_packet` retrieves the persisted memory and exact skill version into worker context.
3. **Feedback Proof**: Run #2 completes with verified success; graph outcome attribution increments the exact skill version's success counter (`success_count: 1`) in the durable ledger with zero extra coordinator model calls.


## Background cost visibility

`/cost`, `/status`, and RPC `get_session_stats.background` report learning review and security watch separately from foreground session tokens. Input, output, cache read, and cache write are provider measurements; reasoning tokens are an output subset and are not added twice. Costs are estimates from provider catalog pricing. Missing usage and zero/unknown prices appear as `null` / `unknown`, with partial measured totals retained. Budget reservations are never presented as measured tokens.

Counters belong to the session that dispatched the call, including calls still running during a session switch. They cover only the current process and are not restored from session history. Reloading the host within that process preserves existing receipts. No-request totals are zero for a bound host; an unbound host reports unknown. Failed and interrupted requests remain visible.

Existing provider attempt telemetry includes actual sends across harness
retries, provider retries, and transport fallback. Its receipts take precedence
over the duplicated terminal assistant usage. Attempts without a receipt remain
unknown, even when a later attempt succeeds; known measurements remain available
as a partial total. Legacy retry events without attempt telemetry likewise keep
discarded failed attempts unknown.

To enable learning review while keeping approval, set `learning.backgroundReview` to `true` in the owner's agent-directory settings. `/learning-approve` and `/learning-reject` control activation. This change does not establish measured quality improvements; live private dev/holdout evaluations remain pending.
