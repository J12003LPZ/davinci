# Orchestration coordination implementation plan

**Goal:** Address the six orchestration findings reviewed at PR #239 head `7e53c839`.

**Architecture:** Serialize actual shared-workspace mutations, persist messages and worker results on the coordinator journal, and hand Git snapshots between dependent isolated workers. Keep existing lifecycle, permission, artifact and worktree owners.

**Tech stack:** Rust, offline unit/integration tests, Git worktrees, runtime JSONL journal.

**Spec:** The six findings supplied in the task: shared writers, workflow code lineage, peer-message durability, delivery retry timeout, result delivery, partial batch launch.

## Constraints and review focus

- Preserve unrelated working changes and the lead checkout.
- No live provider requests are required for regression coverage.
- Cancellation must release writer ownership; background processes must retain it until exit.
- Missing handoffs, Git conflicts and persistence failures must stop unsafe completion.
- Stable message IDs and structured partial receipts must permit reconciliation without duplicate work.

## Tasks

- [x] Reproduce peer acknowledgement with injected journal rejection; require durable queue acceptance and verify replay/retry.
- [x] Reproduce a blocked teammate delivery exceeding its idle timeout; bound retries with one deadline and retain queued messages.
- [x] Fill the lead mailbox and reproduce unavailable results; journal results independently, expose status and retry notifications with stable IDs.
- [x] Reproduce mutation dispatch while a writer lease is held; enforce cancellation-aware ownership for lead and worker mutations.
- [x] Reproduce a delayed background write escaping dispatch ownership; retain the lease through process exit.
- [x] Reproduce missing and conflicting workflow code; snapshot code with a private index, persist handoffs and verify merges before runner execution.
- [x] Inject a second-thread launch failure; return started and unstarted IDs after unstarted resource cleanup.
- [x] Verify offline regressions, affected-library Clippy and formatting; address independent review findings. The agent unit suite passed 1,809 tests, and the final result-journal regression passed separately. Scope further testing to necessary checks per the user request.
- [x] Prepare the isolated fix branch and PR description for publication against the reviewed PR branch.
