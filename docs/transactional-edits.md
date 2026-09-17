# Transactional edits

Ordinary DaVinci writes, edits, notebook edits and patches use a shared transaction
coordinator. Normal sessions and authorized Graph writers use the same ownership,
permission and recovery checks. A transaction records before/proposed/applied
hashes, workspace identity, owner, sequence and verification evidence.

## Explicit tools

| Tool | Input and behavior |
| --- | --- |
| `patch_preview` | `input`: the existing Codex patch format. Saves bounded preimages and returns an ID and `affected_files` without modifying source files. Preview does write recovery data. |
| `patch_apply` | `id` and `paths`: the complete `affected_files` returned by preview. Rechecks current authorization and source state before applying. A transaction cannot apply twice. |
| `patch_status` | `id` and complete `paths`. Inspects the owned record and refreshes source-bound verification. Optional `observe_commit: true` checks local Git objects for an exact matching HEAD; it does not commit or fetch. |
| `patch_rollback` | `id` and complete `paths`. Restores only transaction-owned file images. Conflicts preserve newer external changes and retain the journal. |

Apply and rollback require current write authority; inspection requires current
read authority for every affected path. Knowing an ID does not grant access.
Use the returned path list without dropping entries. Do not retry a conflicted
preview blindly: inspect the current files and create a new proposal.

The CLI enables explicit tools by default. Set `editingTransactions.enabled` to
`false` in the existing settings file to hide them. Invalid fields in that block
also disable explicit tools. Ordinary edit safety remains enabled; disabling
discovery does not permit stale or unauthorized writes.

## Recovery and verification

Recovery records live in `.davinci-transactions` under the workspace. They contain
file preimages and must be treated as private repository data, not disposable
computation cache. Preserve them when recovery fails. Do not edit journal JSON or
delete it merely to bypass a conflict or capacity error.

An ordinary resumed session may recover its own records when the session and task
identity still match, even if its agent ID changed. It cannot adopt another
session's or a Graph worker's records. Direct library callers retain exact-owner
matching. Graph identity and current role restrictions remain binding.

Applying an edit does not verify it. Verification requires supported, successful
execution evidence bound to the current files and authority. A changed source,
failed recheck or superseded observation invalidates current verification.
Diagnostics alone are insufficient. Commit observation is separate: a matching
Git commit does not prove tests passed. Library hosts without the foreground
supervisor cannot issue successful foreground verification receipts.

## Bounds and failure behavior

Transactions are limited to 64 paths, 16 MiB per file and 32 MiB of transaction
content. Journal records are bounded to 192 MiB each, with at most 128 records and
a 256 MiB store budget. Capacity and persistence errors fail closed. There is no
automatic journal archival or pruning facility.

Multi-file changes are journaled, not an atomic filesystem operation. Interrupted
application or rollback can require recovery. Hash and identity checks refuse
unrelated edits; they cannot remove the final check-to-replacement race against
external filesystem actors. Windows directory synchronization does not provide
a power-loss durability guarantee. A crash before journal publication can leave
private staging files.

## Windows metadata boundaries

Supported replacement and recovery preserve named streams, creation time,
supported attributes/compression, alternate 8.3 names, owner/group/DACL, resource attributes and
mandatory integrity labels. Metadata changes participate in conflict checks.
This is not a guarantee to preserve every NTFS security or filesystem feature;
full audit SACL and process-trust/capability label preservation is not established.

Encrypted files and files with object IDs are rejected before source content is
journaled. Private staging aliases are cleared before content is written. Source
aliases are captured and restored after publication, with durable intent allowing
recovery if the process exits between those steps. Recovery refuses an alias
claimed by another file. Use the primary filename when requesting an edit;
addressing the file through its short alias is rejected to preserve the primary
directory entry.

Windows journals now require schema version 3, including captured creation time,
attributes, aliases and pending alias publication. Older records are rejected rather than guessed or migrated.
Preserve an older record for recovery with its original version; do not change
its schema number manually.

## Validation status

The [P4 implementation ledger](superpowers/plans/2026-09-17-engineering-program/04-transactional-edits.md)
records executed tests, evaluations and unresolved acceptance gates. This guide
describes the implementation contract; it does not certify completion of the
twelve-project engineering program.
