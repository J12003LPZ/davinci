# Runtime recovery playbook

This playbook is for an interrupted execution. Commands are read-only unless
a domain owner supplies a verified receipt. Do not delete a journal, rewrite a
graph checkpoint, retry an external effect, or kill a process merely because
startup found an incomplete record.

## First response

1. Stop new dispatch for the affected workspace or caller scope.
2. Preserve the journal, SQLite sidecars, graph root, transaction directory,
   result artifacts, and process/supervisor receipts byte-for-byte.
3. Run:

       davinci doctor runtime --json
       davinci inspect operation <operation-uuid> --json
       davinci inspect run <run-uuid> --json
       davinci inspect session <session-id> --json

4. Record the JSON, journal application ID/schema, source revisions, owner
   generation, latest event sequence, and command exit code.
5. Continue only when the owning adapter proves a postcondition, durable
   receipt, compensation, or safe no-effect decision.

Exit code 0 is a healthy report. Exit code 3 means the inspector could not
establish a safe complete view. Exit code 3 is a diagnosis, not repair
permission.

## Result persistence failed after an effect

Keep EffectPossible, Interrupted, or RecoveryRequired. Do not mark success from
caller text. Reconcile through the owning adapter's idempotency key or
postcondition probe: process birth/listener identity, transaction receipt and
pre/postimage, or a verified remote receipt. Record the evidence and let the
coordinator finalize recovered success, recovered failure, or block recovery.
If evidence cannot distinguish effect from no effect, retain the resource claim
and require domain-owner resolution. A fresh key cannot bypass it.

## Unknown process ownership or termination

Ask ProcessManager/supervisor for managed lifetime, birth identity, executable
identity, and listener ownership. If ownership is unproven, report stopping or
unknown; never infer termination from a missing PID or timeout. Do not launch a
replacement while the claim remains. A verified exit receipt can close it;
otherwise escalate.

## Stale worker binding

Preserve worker output and binding receipt. Reject output whose parent,
workspace, session, generation, or lease is stale. Check parent revision and
owner generation, reconcile through the graph/session owner, and retry only with
a new owner generation on the same logical operation lineage. Never copy an
artifact into a parent run only because its filename matches.

## Incomplete transaction

Stop competing workspace writers. Inspect transaction owner, workspace/source
revision, preimages, postimages, phase, and verification evidence. Let the
transaction coordinator complete or roll back under its owner checks; the
operation journal records phase and recovery evidence but does not invent a
commit. A changed workspace or source revision blocks blind rollback.

## Missing evidence or result artifact

Preserve the reference and do not replace it with caller text. Resolve it
read-only and compare digest, workspace, and root. Regenerate only an exact
artifact without repeating the effect; otherwise leave the attempt non-success
and retain graph/task/session ancestors. Graph pruning retains unreadable or
over-limit operation-binding JSON under uncertainty.

## Corrupt or unknown journal

Copy the database and sidecars and record the copy digest. Run the inspector.
Do not open a corrupt file with the normal writer, delete it, vacuum it, or
perform automatic repair. Restore only from a verified separate backup and
reopen it under the same journal/workspace/root identity. Unknown schemas block;
transactional migration never guesses owner, effect, or success facts.

## Duplicate command collision

Keep the first immutable specification. Return an idempotency collision, stale
revision, owner-fenced, or invalid-permit error as appropriate. Do not overwrite
the payload or create an unrelated replacement key. For an unresolved original,
resolve the effect before allowing a new logical operation on the resource.

## Blocked downgrade or legacy-root conflict

Keep both .davinci and .pi roots and record their digests. Require explicit
migration/reconciliation. Do not remove modern markers for an older binary and
do not choose a root by directory order. Report downgrade support as unavailable
until a tested compatibility contract exists.

## Storage pressure and escalation

For WriterBusy, Capacity, SnapshotTooLarge, or OversizedRecord, use bounded
caller backoff or drain acknowledged outbox rows through their consumer
contract. Never evict unresolved operations, attempt history, active claims,
or referenced artifacts. An incident record should include operation/attempt,
owner and generation, root/workspace/session, schema/application ID, event
sequence, adapter receipt, effect classification, inspector JSON, exact next
safe action, and the authorized owner.

A recovery decision is complete only when the journal records evidence for the
decision and the owning domain agrees that result, verification, and delivery
facts are separately closed.
