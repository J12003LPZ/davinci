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
- Background turn reviews execute on dedicated worker threads after vector memory indexing finishes.
- Any background failure, parse error, or timeout is cleanly swallowed and recorded in local diagnostics; foreground agent execution is never blocked or failed by learning tasks.
- If a new foreground turn begins while a background review is running, the active review is immediately cancelled via cooperative cancellation (`AtomicBool`).
- Review can be completely disabled by setting `PI_LEARNING_DISABLE_BACKGROUND=1`.

### Safe Defaults
- **Shadow Mode Enabled by Default**: The system records candidate artifacts and verifies evidence, but does not automatically write or modify skills until evaluated or approved.
- **Auto-Apply Guardrails**:
  - `auto_apply_project`: Disabled by default (`false`). Only applies when explicitly enabled *and* the target project directory is trusted in project trust settings.
  - `auto_apply_global`: Disabled by default (`false`). Modifying global skills requires explicit configuration or manual approval.
- **Deterministic Verification Required**:
  - Auto-promotion from candidate to active skill requires verified command execution (e.g. tests or build verification commands that exited with code 0).
  - Turns with zero command executions, failed verification, or user corrections cannot auto-promote.
- **Read-Before-Write Hash Verification & Path Traversal Prevention**:
  - Patching existing skills checks that the current file content matches the expected hash before applying changes.
  - Rejects attempts to escape skill directories or overwrite user-authored / imported skills.
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
  │     ├── Untrusted / Unverified? ──► Stage For Approval (PendingApproval)
  │     └── Trusted & Verified & Auto-Apply? ──► Apply to SKILL.md & Ledger & Vector Memory
  └── Queue Progressive Disclosure Notice
```

---

## 4. Storage Layout & Ownership

The learning subsystem maintains stores at two scopes:
- **Project Scope**: `<repo>/.pi/learning/` in the repository root.
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
- Project skills: `<repo>/.pi/skills/<skill_name>/SKILL.md`
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
| `/learning-status` | Displays current learning configuration, shadow mode status, project trust, candidate stats, and active skills count. |
| `/learning-pending` | Lists staged learning candidates awaiting human approval. |
| `/learning-approve <id\|all>` | Approves a staged candidate, activating the skill/memory and updating the ledger. |
| `/learning-reject <id\|all>` | Rejects a staged candidate, marking it as dismissed. |
| `/skill-list [query]` | Lists compact descriptors of known skills across project and global scopes with versions and usage counts. |
| `/skill-view <name> [file]` | Displays the contents and ledger metadata of a specific skill or support file. |

### Progressive Disclosure
When background reviews stage candidates or activate skills, notifications are buffered and drained into the interactive transcript without corrupting the TUI screen:
- `"learning · 1 pending candidate(s) awaiting approval (/learning-pending)"`
- `"learning · skill activated: debug-sqlx"`

---

## 6. Configuration & Defaults

Learning configuration is specified under the `"learning"` key in settings:

```json
{
  "learning": {
    "enabled": true,
    "backgroundReview": true,
    "shadowMode": true,
    "autoApplyProject": false,
    "autoApplyGlobal": false,
    "maxCandidatesPerReview": 3,
    "maxReviewInputTokens": 12000,
    "maxReviewIterations": 6,
    "autoPromoteVerifiedUses": 2,
    "reviewTimeoutMs": 30000
  }
}
```

### Failure Behavior
- **Fail-open**: Learning failures never fail normal turns.
- **Review disablement**: Setting `PI_LEARNING_DISABLE_BACKGROUND=1` immediately short-circuits background review execution.
- **Fallback to lexical**: If Ollama or Qdrant are unavailable, skill retrieval cleanly falls back to lexical matching.
- **Untrusted projects**: Projects that are untrusted receive no autonomous project writes.
- **Global protection**: Global autonomous writes are disabled by default.
