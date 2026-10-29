# Parity contracts (Task M2)

The interfaces and invariants this branch relies on, stated once so later work can test against them. Each line names the owner and the test that pins it. Invariants I-01..I-10 are defined in `docs/superpowers/specs/2026-10-28-claude-runtime-capability-convergence-design.md`.

## Tool call identity and outcome (I-01, I-02)

- **Logical call id.** The provider tool-call id is the identity of one logical call across ledger, operation journal, MCP and session. MCP `tools/call` carries it as `params._meta["davinci/invocationId"]` (`davinci-mcp` `INVOCATION_ID_META_KEY`); agent tool paths must supply it. Test: `g12_the_tool_call_id_reaches_the_server_as_meta`.
- **Outcome classes.** `NotDelivered` only when zero request bytes reached the transport; any other transport loss after sending is `Ambiguous` → `ToolError::MayHaveRun` → ledger `StartedUnknown`, shown to the model as "Outcome unknown". Never auto-retried. Tests: `a_line_with_no_byte_written_is_provably_not_delivered`, `a_partly_written_line_is_an_unknown_outcome`, `g14_*`.
- **Retired MCP connections.** A slot is flagged evicted under its own client mutex before the registry changes; check and send happen in one mutex hold. Tests: `g01_a_stale_caller_cannot_send_while_*`.
- **Ledger memory.** At most 256 terminal records. Pruning a non-`SafeToReplay` record leaves a 64-bit tombstone (journaled before the removal). A tombstoned id is refused whatever policy a later call claims. Admission keeps `tombstones + live non-safe records < cap`; at the cap new non-safe calls are refused ("start a new session"). Ledgers from before tombstones that pruned load with `legacy_pruned` (all non-live ids refused); a load over budget is refused with files untouched. Forgotten ids are never routed to the operation journal, which only protects ids it admitted. Tests: `g16_*`, `a_forgotten_id_never_reaches_the_operation_journal`.

## Dispatch authority (I-03)

- **Revision binding.** Authorization returns the permission revision it evaluated, captured in the same lock hold as the decision (including a session grant). The compatibility route stores it per call id (single use; dropped on cancel, skip or re-prepare); a call with no record is refused.
- **Last-moment gate.** Both routes take the permission lock, then check cancellation, then compare the revision, immediately before the adapter (journal: after contract/capability/digest validation). A policy change after the adapter starts applies from the next call. Tests: `operation_dispatch_tests`, `journal_route_policy_change_during_validation_is_denied_before_the_adapter`, `script_child_is_refused_when_policy_moves_after_its_decision`.
- **Authority inputs.** `execution_authority(id, journal_eligible, journal_configured)`: eligibility from the tool's own capability source, configuration from the runtime. Test: `execution_authority_follows_each_tools_own_capability_and_the_journal`.

## Context (I-04)

- **Compaction summaries** are identified only by `ChatMessage.extra["davinciCompaction"]` (persisted as `carriedUserRequirements` in the compaction session entry), never by text. The carried list holds the verbatim user-authored text of every compacted message, in order, exact-deduped; markers in model-written text are escaped; the rendered block is bounded and an over-bound or failed verification keeps the original history. Tests: `compaction.rs` `*user_requirement*` tests.
- **Skills.** Discovery reads only frontmatter (stops at the closing `---`, capped); bodies are read fresh on invocation only; accounting and retrieval use file size. Tests: `discovery_reads_only_bounded_frontmatter`, `invocation_reloads_body_after_file_change`.

## Background work (I-05, I-06)

- **Workflow artifacts** record a source fingerprint (HEAD + dirty digest). On resume a missing, different or unavailable fingerprint in a git workspace invalidates the artifact (the worker reruns). Tests: `resume_reruns_artifacts_when_source_fingerprint_changes`, `legacy_artifact_without_fingerprint_is_rerun`.
- **Worker admission**: bounded waiter queue (default 32) and wait timeout (default 15 min); cancellation, then deadline, then capacity; slot freed on every exit including panics.
- **Worktrees**: release keeps the lease and returns `CleanupIncomplete` while the directory, registration or branch survives or any inspection fails; removal is non-forced `git worktree remove` (git decides cleanliness); only this lease's registration is ever touched; dirty worktrees are never deleted; orphan scan reports and never deletes.
- **Shutdown**: one cleanup coordinator, steps isolated with `catch_unwind`, owned processes terminated first, one absolute deadline (5 s) shared by all triggers, plus an exit watchdog at deadline + 1 s.

## Local IPC (I-08)

- No listener binds and no connection is authorized without a nonempty token; constant-time compare; unauthenticated helpers are crate-private. Unix peer-uid check: not implemented (needs a new dependency).

## Rollback

Every change is additive behind existing types; persisted additions are serde-defaulted (`tombstone_format`, `legacy_pruned`, `source_fingerprint`, `davinciCompaction`, `carriedUserRequirements`), so older builds ignore them. Reverting a commit restores prior behavior. Sessions written by this branch stay readable by older builds; ledgers written by this branch upgrade older ones and are not downgraded.

## Platform matrix

Windows: all targeted suites run on the host. Linux (Docker, `rust:1.88`): `davinci-server`, `davinci-mcp`, `tool_ledger`, worktree and `experimental-ipc` suites. Interactive TTY, IDE peer and live provider paths were not exercised.
