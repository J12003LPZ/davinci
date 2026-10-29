# 05 - G-02 trusted subagent verification receipts

Status: implemented and verified offline.
Files: `crates/davinci-agent/src/subagent_receipt.rs`,
`crates/davinci-agent/src/lib.rs`.

## 1. The problem

A delegated worker returns text. The parent previously had no way to distinguish
"the worker ran the tests and they passed" from "the worker wrote that the tests
passed". Treating the text, an exit code, or the worker's own summary as proof is
exactly the failure mode G-02 is meant to remove.

A receipt is now a host-derived statement with two independent inputs:

1. the worker agent's runtime state: `CompletionStatus`, `mutation_generation`,
   `verified_generation`, and the paths the runtime recorded;
2. a before/after `WorkspaceSnapshot` taken by the host around the worker turn,
   so a change the worker's own accounting missed is still visible.

No model-authored text is parsed to decide the status.

## 2. Types

```rust
pub const RECEIPT_SCHEMA_VERSION: u32 = 1;

pub enum ReceiptStatus { Verified, Unverified, Failed, Unavailable }

pub struct VerificationReceipt {
    pub schema_version: u32,
    pub status: ReceiptStatus,
    pub detail: String,                 // host reason, never model text
    pub workspace: Option<String>,
    pub workspace_digest: Option<String>, // after-snapshot digest
    pub changed_files: Vec<String>,
    pub changed_files_known: bool,
    pub mutation_generation: u64,
    pub verified_generation: Option<u64>,
    pub completion_status: Option<CompletionStatus>,
    pub observed_at_ms: u64,
}
```

`Unavailable` is the honest default. `VerificationReceipt::take(slot)` returns it
when the slot is missing or empty, with the detail "the host did not observe the
worker's completion state".

## 3. Derivation

`VerificationReceipt::from_worker(completion, state, before, after, workspace)`
applies these rules in order:

| condition | status | detail |
| --- | --- | --- |
| `after` snapshot missing | `Unavailable` | host could not snapshot the workspace |
| `Verified`, no changed files, `mutation_generation > 0` | `Unavailable` | reported verified but workspace did not change |
| `Verified`, files changed, `mutation_generation == 0` | `Unverified` | host saw changes the runtime did not record |
| `Verified`, generation > 0, `verified_generation == generation` | `Verified` | a passing check covers the latest generation |
| `Verified` otherwise | `Unverified` | pass does not cover the latest generation |
| `Partial` | `Unverified` | checks cover only part of the changes |
| `VerificationFailed` | `Failed` | latest check failed |
| `Unverified` | `Unverified` | stale or incomplete |
| `VerificationUnavailable` | `Unverified` | changes without an observed command |
| `NoChanges`, workspace changed | `Unverified` | host saw changes the worker did not account for |
| `NoChanges` | `Unavailable` | nothing to verify |

The two cross-checks matter. A runtime that claims verification while the host
sees no workspace change is downgraded, and a runtime that reports no mutation
while the host sees changed files is downgraded. Neither can become `Verified`
from text.

## 4. Workspace snapshots

`WorkspaceSnapshot::capture(root)` canonicalizes the root, walks it, hashes each
file's content (or a symlink target), and stores a `BTreeMap<relative path,
digest>` plus an aggregate digest. `changed_files` reports added, removed and
modified files in sorted order.

Excluded directories:

```text
.git .davinci target node_modules dist build .next .nuxt .venv venv
__pycache__ .pytest_cache .mypy_cache .ruff_cache .cargo .gradle .idea
```

Excluded files: `.DS_Store`, `Thumbs.db`.

The snapshot is bounded at 50,000 files and 512 MiB. Exceeding a limit is an
error, not a partial snapshot, so a huge workspace cannot silently produce a
receipt over an unobserved subset.

## 5. Host API

```rust
pub type SubagentReceiptSlot = Arc<Mutex<Option<VerificationReceipt>>>;
pub fn receipt_slot() -> SubagentReceiptSlot;
pub fn publish(slot: &SubagentReceiptSlot, receipt: VerificationReceipt);
pub fn receipt_for_agent(agent, before, after, workspace) -> VerificationReceipt;
```

`publish` writes even if the mutex is poisoned, so a panic in another thread
cannot hide the receipt. `take` removes the value so one receipt is consumed
once by the parent.

`VerificationReceipt::render()` produces the host text block:

```text
[verification receipt] status: unverified
detail: the worker's verification is stale or incomplete
changed files (1): src/lib.rs
mutation generation: 1 (unverified)
```

`to_json()` is the same data for `details.verificationReceipt` /
`details.verificationReceipts`.

## 6. Tests

```text
cargo test -p davinci-agent --lib --offline subagent_receipt
EXIT=0
test result: ok. 14 passed; 0 failed; 0 ignored
```

The 14 tests:

| test | property |
| --- | --- |
| `capture_and_diff_detects_added_removed_and_modified_files` | snapshot diff covers all three change kinds |
| `capture_ignores_build_and_vcs_artifacts` | excluded directories do not create noise |
| `verified_runtime_state_produces_verified_receipt` | the one positive path |
| `stale_generation_is_unverified` | old pass does not cover a new edit |
| `failed_verification_is_failed` | failure maps to `Failed` |
| `changed_files_without_verification_are_unverified` | changes without a check are not verified |
| `no_changes_is_unavailable_not_verified` | idle worker is unavailable |
| `reported_verified_without_a_workspace_change_is_unavailable` | runtime claim vs. empty diff |
| `missing_snapshot_is_unavailable` | snapshot failure is not success |
| `host_snapshot_catches_changes_the_runtime_missed` | host diff vs. runtime generation |
| `claimed_text_alone_can_never_verify` | no text input exists in the derivation |
| `receipt_slot_is_shared_by_clone` | the slot is shared, not copied |
| `receipt_serializes_with_schema_version` | schema version is present |
| `render_lists_status_files_and_generation` | host block is stable |

## 7. Limits

The receipt proves what the host observed in one workspace during one worker
turn. It does not prove the change is correct; that is the job of the scenario
oracle (G-06). Snapshot exclusions mean generated output is intentionally not
part of the identity. A workspace larger than the limits yields `Unavailable`
rather than a partial claim.
