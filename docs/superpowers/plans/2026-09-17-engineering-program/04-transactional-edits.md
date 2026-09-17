# P4: Transactional Edit Engine

Status: design approved by the user on 2026-09-17; implementation in progress.
Execution sequence: 3 of 12.
Dependencies: P2 green; existing apply_patch journal, mutation confinement, effects/checkpoints/rewind.
Requirements authority: project section 9 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Multi-file edits have recoverable provenance; stale application and unsafe rollback return conflict without overwriting another actor.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-agent/src/apply_patch.rs and tools.rs
- crates/davinci-agent/src/runtime/{effects,checkpoints,rewind,source_manifest}.rs; new transaction lifecycle module only as needed
- crates/davinci-coding-agent/src/semantic/rename.rs (inspect preview integration; production apply consumer not yet established)
- crates/davinci-coding-agent/src/{extension_host,main,settings}.rs and native adapters
- New transaction integration, crash-recovery and normal-session tests

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Run current patch/rewind fixtures, record preimage/postimage/journal behavior, demonstrate missing explicit lifecycle across ordinary write/edit calls. Use temporary files and byte hashes, not user files.

## Ordered implementation and RED/GREEN work

1. RED: preview -> externally change one file -> apply refuses every write; apply -> external edit -> rollback refuses to overwrite it. Include multi-file failure/crash and semantic replacement paths.

2. Define transaction identity/owner/workspace/base revision, affected files, before/proposed/applied hashes, sequence and verification state using existing effects/checkpoint contracts. Lifecycle is draft, previewed, applied, verified, committed, rolled_back or conflicted with checked transitions.

3. Wrap existing write/edit/apply_patch and semantic replacements through one transaction coordinator. Expose patch_preview/apply/status/rollback only where needed; keep existing tool compatibility and automatic provenance for single-file calls.

4. Resolve paths with existing confined primitives; reject linked/reparse/sensitive/outside targets according to current policy. Under the existing workspace mutation lane, reauthorize and verify every before-hash before any file write.

5. Persist journal/preimages durably before mutation, bound transaction file/byte counts, and publish applied hashes/effects after successful writes. Preserve create/delete/rename semantics and permissions; fail closed on journal/storage error.

6. Rollback/recover only files whose current bytes and identity match the transaction-owned postimage. Preserve conflict journal for inspection; never use blanket Git reset/checkout to undo. Pin durable recovery blobs independent of evictable computation cache.

7. Connect writes to repo/LSP invalidation and source-bound verification. Only actual current evidence can mark verified; only observed Git commit state can mark committed. Record session/agent/Graph node and sequence in provenance.

8. Wire native status/preview/rollback with current permissions and normal session attachment, then Graph reuse; cancellation and shutdown leave recoverable journals.

9. GREEN: fail-injection, concurrent actor, normal edit/verify and cross-worktree tests; rollback eval; affected package/fmt/lint/CI; document filesystem race and recovery limits.

## Required targeted validation

- New/edit/delete/rename and multi-file patch; stale hashes with same timestamp/length; case/path normalization; symlink/reparse escape; disk-full/write failure.
- Cancellation after journal and between writes; crash recovery with correct owned bytes; unrelated new user content preserved.
- Real normal dispatch wraps edits without Graph, reports transaction ID/effects and invalidates previous successful evidence.
- Concurrent transactions serialize only mutation, not analysis/process I/O; retry cannot apply twice or silently adopt another owner.

## Settings and fallback

editingTransactions.enabled controls explicit coordinated transactions. Existing stale-state/journal safety is never disabled as an authorization bypass. Default enable only after compatibility and recovery gates pass.

## Specific acceptance guard

Do not claim atomic multi-file OS writes. Prove journaled recovery and safe conflict handling through actual file mutations and fault injection.

## Completion gate and handoff

The approved [program design](../../specs/2026-09-17-engineering-program.md)
and original [requirements](../../specs/2026-09-17-engineering-program-requirements.md)
are the reference. Shared authorization, bounded-output, cache, lifecycle, telemetry,
settings and security invariants apply in addition to the project-specific steps.

Record all fourteen section-34 gates in this project's ledger row: design approval,
plan, observed RED/GREEN, affected package tests, fmt, Clippy, integration, security,
normal path, applicable Graph path, benchmark/eval, docs, diff review, and exact-head
CI. Planned fixtures/test names above are not claims of existing or executed tests.

Validation sequence: focused failing acceptance test; focused passing checks after
implementation; affected crate tests with `--offline --locked`; `cargo fmt --check`;
affected Clippy during development and required workspace Clippy at the milestone;
matching integration/security/eval cases; feature-branch push and actual CI monitoring.
Run broader suites only for affected shared contracts or final acceptance. Record
exact executed selectors, exits and test counts; zero-case runs do not count.

Use a separate coherent commit/PR for the subsystem. With earlier PRs unmerged,
stack on the preceding verified branch and target that branch for review. Preserve
dependency commits without force pushes. Do not begin the next subsystem with
known failures or incomplete required gates. Approval to implement this program
does not authorize merging new PRs into main.

The handoff in README records files/APIs changed, validated commands and results,
metric/artifact paths, head SHA/CI URLs, limitations, and the next dependency input.
No production capability is called done from this plan alone.

## Implementation checkpoint: session recovery

The transaction coordinator now distinguishes the original owner from the actor
performing recovery. A trusted root-session attachment may recover records from
the same nonempty session and task after its agent ID changes. Both original and
current owners must be outside worker/Graph ownership. Direct library callers
retain exact-owner matching. Every recovery still checks current dispatch
authority, workspace identity and transaction-owned postimages.

The resumed-session regression initially failed with `transaction workspace or
owner mismatch`. After the fix, `cargo test -p davinci-agent --test transactions
--offline --locked -- --quiet` passed all 16 cases, including denial for another
session, a worker and a Graph node. The subsequent `parent_session` selector
passed two cases, including the additional inverse-ownership regression (root
session cannot adopt worker/Graph records). The coding-agent selector
`editing_transaction_settings_fail_closed` passed once in each library and
binary target; filtered integration targets ran zero cases and supply no
additional validation. Formatting was applied and `git diff --check` passed;
this is not a completed package, lint, eval or CI gate.

Outstanding P4 work includes actual verification/commit lifecycle evidence,
Graph end-to-end transaction validation, repo/LSP refresh validation, metadata and
failure-path hardening, native Unix validation, the after-baseline evaluation,
affected package/lint checks, final review and exact-head CI. P3 has not started.

### Graph adapter checkpoint

Writers now receive the same four transaction tools; other roles do not. Worker
contract checks cover their complete path lists. The parent exports its graph
task ID as `PI_GRAPH_NODE_ID`, and CLI initialization attaches that identity to
transaction ownership. Legacy workers use their artifact path as an ownership
boundary. Invalid worker context now stops initialization instead of falling
back to ordinary-session authority. Disabled transaction settings remain binding
when the worker authorization list is reconstructed.

The writer-role regression failed before implementation. Afterwards,
`cargo test -p davinci-coding-agent --lib native_extensions::graph:: --offline
--locked -- --quiet` passed 338 cases. Following node-identity validation and the
CLI startup guard, `cargo test -p davinci-coding-agent --bin davinci
native_extensions::graph::worker_hooks::tests:: --offline --locked -- --quiet`
passed 16 cases. The latter compiled the changed CLI; it does not prove a real
worker launch, settings propagation or an end-to-end transaction. Those remain
required integration checks. Formatting and whitespace checks passed.

### Packaged CLI checkpoint

`cargo test -p davinci-coding-agent --test transaction_cli --offline --locked
-- --quiet` now passes three offline subprocess cases through the freshly built
CLI. A Graph-context worker writes one durable transaction bearing `writer-17`
and one effect handoff record. Disabled explicit tools return `Unknown tool`
without creating a journal; invalid node context fails startup without writing.
An ordinary non-Graph write still creates a transaction when explicit tools are
disabled. Tests clear inherited environment, use temporary config/workspaces and
bound child lifetime and failure output. This validates child CLI behavior; the
parent `run_worker` launch and complete preview/apply/rollback flow are not yet
covered by this fixture.

The first run exposed an offline-driver replay defect: synthetic verification
reminders repeated the scripted write until the 128-record journal limit. A
focused regression also failed before the fix. The driver now excludes those
reminders from scripted calls, leaving completion verification enforcement
unchanged. The initial disabled-tool assertion was corrected to the observed
CLI `Unknown tool` response; mutation was already denied.

### Verification lifecycle checkpoint

Host-only observations now bind successful execution receipts to the transaction
sequence, owner, workspace and unchanged affected-file images. Beginning a
recheck invalidates the current verified state while retaining historical
evidence. Superseded observations and failed, simulated or unauthorized results
cannot restore that state. Status detects changed affected files.

Persisted receipt validation also checks the canonical workspace binding without
following a journal-supplied path. The tampered-workspace regression failed
before this check and passed afterwards. `cargo test -p davinci-agent --test
transaction_verification --offline --locked -- --quiet` passed four cases.
The preceding combined transaction/verification run passed 17 transaction cases
and the then-current three verification cases. Formatting was applied.

These tests launch a real fixture subprocess and validate receipt lifecycle;
they do not prove coverage of edited application behavior. Normal command
dispatch still needs trusted receipt capture and observation integration after
post-hook processing. Git commit observation and the remaining P4 acceptance
gates above remain unfinished. No new subsystem has started.

### Actual command receipt checkpoint

Built-in foreground Bash, PowerShell and exec_command calls now support an
opaque per-dispatch capture of actual exit status, timestamps and stdout/stderr
hashes. Normal dispatch installs the capture and retains at most 128 receipts;
post-hook vetoes are recorded without replacing the observed exit status.
Tool-result JSON cannot create these captures. Background jobs, interrupted
commands and the PowerShell reply stub do not produce successful receipts.

The actual-command regression failed before capture was connected. The
`actual_command` library selector then passed two tests covering real exit 0/7,
single consumption, normal dispatch, hook veto and fabricated output. The
`powershell_and_image_read` selector passed its simulated-output rejection
case; its environment override now runs in an isolated child test process.
Transaction observations are not yet connected to these retained receipts, so
this remains an intermediate implementation checkpoint, not the normal-session
edit/verify acceptance gate.

### Normal-dispatch source observation checkpoint

Normal dispatch now observes current-agent transactions from the runtime effect
ledger before eligible foreground commands. It checks current read permissions
and task contracts before capture and again after execution. After post hooks,
the matching host receipt can verify an unchanged transaction; command failures,
vetoes and changed sources leave it unverified. Results include bounded
transaction verification outcomes. No journal or shared-agent mutex is held
across command execution. Receipt IDs are cleared before a new execution to
avoid reusing older evidence.

The normal-session integration regression initially failed because a successful
actual Cargo check left the transaction applied. After wiring observations and
receipts, `cargo test -p davinci-agent --lib normal_command_verifies_transaction
--offline --locked -- --quiet` passed one test with three real subprocess cases:
unchanged success, changed source, and post-hook veto. The separate
`transaction_verification` library selector passed the command-recognition test.
The affected library Clippy command, `cargo clippy -p davinci-agent --lib
--offline --locked -- -D warnings`, initially found six warnings in earlier P4
edits (five redundant borrows and one boolean expression); after correction it
passed. Formatting was applied and whitespace validation passed.

Coverage selection is still a bootstrap: explicit Cargo workspace check/test/
clippy commands with a small allowed flag set, no filters or shell composition.
It is not proof that arbitrary affected files belong to the selected Cargo
targets. Broader per-file/target coverage, TS/JS commands, resumed-session
transaction discovery and the full normal-mode acceptance gate remain open.
Do not promote this fixture to program-wide verification completion.

### Observed target coverage checkpoint

The bootstrap coverage gap above is now closed for automatic state promotion:
every affected file must match an actual compiler target root reported by Cargo
JSON output. Plain successful workspace commands no longer supply enough
evidence. Receipts carry bounded workspace-relative `compiler_source_roots`,
defaulting to empty for older serialized records. These are target roots, not
transitive module coverage. The parser follows Cargo's documented
[compiler-artifact and build-finished messages](https://doc.rust-lang.org/cargo/reference/external-tools.html).
It stops at build-finished, requires success, rejects escaping paths and bounds
output, message and result sizes. Commands are not silently rewritten to add
JSON flags.

The expanded normal-session regression demonstrated the defect: an unreferenced
Rust file was marked verified when only the library target compiled. After the
coverage check, its four cases passed (target-root success, stale source, hook
veto and uncompiled source). The command receipt selector passed two tests,
including missing/failed completion, traversal, excessive output and rejection
of artifact-shaped messages after build completion. Dependencies, non-root
modules, deletions and TS/JS verification remain unsupported by this automatic
coverage adapter until stronger evidence is integrated; they stay unverified.
This is a deliberate current limitation, not a reduction of program scope.

After this change, the four transaction-verification integration tests passed.
`cargo check -p davinci-coding-agent --lib --bin davinci --offline --locked`,
`cargo clippy -p davinci-agent --lib --offline --locked -- -D warnings`, and
`cargo fmt --all --check` passed. Whitespace validation also passed. These are
targeted checks; affected full-package tests and exact-head CI remain open.

### Commit observation and compound-read approval checkpoint

The trusted coordinator now supports read-only commit observation. It requires
current source authority, matching live postimages, and exact blob bytes and
executable modes in an immutable observed HEAD commit. Deletions must be absent
from that tree. A final sequence and source check precedes the durable Committed
transition. The recorded commit ID is historical evidence, not a promise that
HEAD will remain there. This API creates no commit and does not make failed tests
pass. Rollback continues to reject committed transactions.

Git inspection uses bounded output, a timeout, isolated environment, disabled
replacement objects and filesystem monitoring, and disabled lazy fetching and
transport protocols. Missing promisor objects fail locally. The three
`transaction_commit` integration tests passed: exact/mismatched/uncommitted/
revoked cases; mixed create/delete with unrelated-file preservation; and a
missing local blob available from a separate local promisor remote, with no
fetch. The Windows fixture explicitly clears the disposable loose object's
read-only attribute before removing it. Normal-session commit observation and
automatic base-revision capture are still pending.

Permission checks now include transaction source reads before outer-tool allow
rules. A source read requiring approval produces a one-call approval bound to
the full original tool and arguments, with no persistent grant offered. Explicit
read denials remain binding. The normal dispatcher now retains that permit for
coordinated mutations and explicit transaction tools as well as managed process
tools. This fixes both a read-approval bypass and rejection of legitimate
one-time transaction approvals.

The new permission regression failed on the edit-allow bypass before the fix.
The normal-dispatch regression then exposed the missing permit handoff before
that fix. Both passed afterward, including denied consent, one-time consent,
and policy revocation during approval. All 56 permission tests and 23
approval-focused library tests passed. Agent library Clippy with warnings denied
and whitespace validation passed; formatting was applied. The coding-agent
library/binary check passed after the commit-evidence change, before the later
compound-read permission fix. Full P4 validation and platform CI remain open.

### Normal-session commit observation checkpoint

`patch_status` now accepts optional `observe_commit: true`. It retains the exact
owned path-set check and delegates to the same coordinator Git observation API.
The ordinary status call performs no Git inspection. Observation never creates a
commit or fetches missing objects, and does not equate committing with passing
verification. Other transaction tools reject the new status-only argument.

Permission evaluation includes Git metadata reads in the complete original call;
the live dispatch authority rechecks this before and after observation. Because
Git may follow packed objects, linked-worktree metadata and configuration
includes, any applicable read-deny rule currently blocks commit observation
conservatively. A directory read grant alone cannot prove authorization for all
of those internal reads. Ordinary status remains available under such policies.
Per-object metadata authorization remains a limitation to address with the Git
inspection integration, not an excuse to bypass current read restrictions.

The new normal-dispatch fixture passed four real-Git cases: committed content,
uncommitted content, denied metadata access, and access revoked after preparation.
Only exact committed content reached Committed. A permission fixture passed
nested metadata and unrelated scoped read-deny cases while preserving plain
status. All 15 transaction-selected library tests and agent-library Clippy with
warnings denied passed. Formatting was applied. Automatic observed base-revision
capture and the other outstanding P4 gates remain unfinished.

### Observed base-revision checkpoint

Normal transaction previews now capture the starting Git HEAD through the shared
bounded, read-only revision observer when current metadata policy allows it.
The host-installed authority carries a separate metadata check; generic trusted
callbacks do not implicitly grant Git access. Metadata authority is checked
before and after the probe, and source authority is rechecked before preimage
capture. No transaction lock spans Git execution. A missing/unborn repository,
failed observation or unavailable metadata authority leaves base_revision
unknown; it does not invent a revision or disable source mutation safety.

The normal-dispatch fixture now asserts the exact baseline SHA and unknown
revision under denied metadata access. It caught an invalid empty-path internal
permission probe, which was fixed before the fixture passed. The shared revision
reader also produced one Clippy conversion warning, corrected before Clippy
passed. The three Git commit integration cases and all 17 transaction integration
cases passed after this change. Formatting check detected one wrapped line;
formatting was then applied. Remaining P4 gates are unchanged apart from the
normal commit attachment and observed base capture now implemented here.

### Journal failure and cancellation checkpoint

Active transaction markers now use a synchronized temporary file and an atomic
no-clobber publication. A failed partial write cannot expose a truncated marker,
and a second begin cannot replace an existing active transaction. Record updates
use the same publication helper. Test-only thread-local failure injection writes
a real partial temporary file and returns an I/O error; it has no production
configuration or cross-test global state.

Four storage tests passed: partial marker publication; preservation of the prior
journal on a failed replacement; recovery after each of three apply journal-write
failures; and recovery after each of three rollback journal-write failures. The
rollback regression first failed because failure before marker publication left
an orphan source stage. Rollback now cleans its owned temporary files on staging,
marker and pre-mutation journal failures. A new coordinator successfully recovers
both unchanged and partially restored sources without those orphan stages.

Cancellation is now checked again after current authority returns, immediately
before apply or rollback replaces a file. The apply regression first failed
because cancellation during the last authority callback still allowed a success
result. After the fix, the 18-case transaction integration suite passed, including
the existing real-process-exit-between-writes fixture. An additional rollback
cancellation fixture passed separately: one path remained applied, another was
restored, and a fresh coordinator completed recovery from the retained journal.
Agent library Clippy with warnings denied passed and formatting was applied.

These are injected journal I/O failures and process-crash recovery results, not
power-loss or universal disk-full guarantees. Source-stage I/O failure coverage,
metadata preservation, native Unix checks, Graph end-to-end acceptance, repository
and LSP refresh, evaluation and final package/CI gates remain open.

### Windows named-stream recovery checkpoint

Two new regressions demonstrated lost named-stream data after delete/rollback
and missing conflicts after a stream-only change. Both now pass. Transaction
images persist named data streams, include them in stale-image comparisons and
restore them into owned stages before publication. Recovery through a fresh
coordinator preserves ordinary, empty and Unicode-named streams. Existing edit
replacement preservation continues to pass.

Enumeration uses the pinned file handle and a fixed 64 KiB buffer, following
[Microsoft's FILE_STREAM_INFO layout](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_stream_info).
Capture keeps opened stream handles pinned against writes/deletion while reading,
checks their file identities and verifies the stream list again. Limits are 64
named streams and 1 MiB total stream content per file, also charged to the existing
transaction byte budget. Unsupported, malformed or oversized metadata fails
closed before source mutation. Journal validation rejects unsafe stream names
and stream-bearing records on non-Windows hosts. The existing final observation
to rename race limitation still applies; this is not filesystem-wide atomicity.

Validation: 29 integration tests across `transactions`, `transaction_commit` and
`transaction_verification` passed; the bounded metadata parser unit test passed;
the expanded empty/Unicode deletion recovery test passed separately. Agent library
Clippy with warnings denied passed. No full package or platform-CI result is
claimed by these targeted checks. Unix extended attributes/ownership, remaining
source-stage failure coverage and the outstanding P4 integration/evaluation gates
still require completion.

### Source-stage failures, intelligence refresh and after evaluation

The new `transaction_source_stage_failures` library test passed eight injected
failure combinations: partial writes or sync failures at either of two source
stages, during apply or rollback. No source mutation occurred before all stages
were ready, owned temporary stages were removed, and a fresh coordinator could
apply/recover the original record. These test-only thread-local injections cover
the staging error branches; they do not emulate a physically full filesystem or
power interruption.

`transaction_intelligence` passed through the ordinary `apply_patch` and explicit
rollback tool implementations, without Graph. Existing repo intelligence observed
the changed symbol and LSP diagnostics reflected the changed document; rollback
restored both observations while retaining one LSP process. No new invalidation
system was needed: the existing managers refresh from current source content.
The offline LSP fixture initially missed its error token with a trailing newline;
its diagnostic predicate now trims whitespace. All four existing LSP session
tests passed afterward. This is protocol-fixture integration, not a fresh real
TypeScript compiler validation claim.

The after evaluation passed and wrote `evidence/p4-transaction-after.json`.
Three public mutations produced three transaction IDs; owned rollback restored
two files; stale rollback returned Conflicted and preserved later user bytes.
The debug Windows sample measured 74.1761 ms for the mutations and 20.3552 ms for
rollback. The frozen pre-P4 mutation sample was 19.9454 ms. These individual
samples show added journal/metadata work, not a statistical performance estimate.
The before recovery scenario used a constructed legacy journal; after uses real
transaction records, so the recovery timing/implementation are not equivalent.
The first artifact-writing run failed because its output path was relative to
Cargo's crate working directory; rerunning with an absolute output path passed.

Graph parent/worker end-to-end validation, Unix metadata behavior, final package
checks and exact-head CI remain open. The program has not advanced to P3.
Agent library/test Clippy and the coding-agent `transaction_intelligence` target
Clippy passed with warnings denied. The wider agent check exposed two test-only
lint issues: the storage test module preceded production helpers, and a Windows-
only disposable Git-object permission reset needed a narrowly explained allowance.
The module was moved after the helpers and the Windows-only statement annotated.

### Graph transaction identity checkpoint

The real parent launcher regression exposed a provenance defect: the launcher
exported `DAVINCI_AGENT_ID`, but the child generated a different identity. The
validated Graph context now accepts the parent ID, rejects malformed IDs before
mutation, and retains that identity across prompt turns. Legacy launchers without
an ID retain the process-local identity. Ordinary sessions keep their existing
identity path.

The explicit offline launcher test passed after the fix: an actual child binary
wrote a transaction with the parent-assigned agent ID and Graph node, emitted an
effect record, and allowed exact-owner recovery while refusing another node or
revoked authority. Its one-call fixture submits no Graph artifact, so the parent
correctly reports missing-artifact failure. This is launcher/provenance/recovery
coverage, not a successful full Graph lifecycle.

`graph_transaction_owner_survives_multiple_prompt_turns` passed through the prompt
host: write on turn one and rollback on turn two retain the same runtime identity.
All four `transaction_cli` tests passed, including malformed parent ID rejection
before source or journal creation. The new prompt test initially failed to compile
because its file read lacked the `std::` qualifier; that test-only error is fixed.
Full package validation and exact-head CI remain open.

### Successful Graph worker transaction and submission

The offline provider fixture now accepts at most 32 sequential tool calls. It
advances after successful results, stops after a failed tool, ignores capability
reminders as turn boundaries, and resets only for an actual user prompt. The
sequencing regression failed before implementation (the old fixture interpreted
an array as a default bash call) and passed afterward.

With a freshly built `davinci` binary, the explicitly enabled
`transaction_parent_launch_accepts_submitted_edit_and_preserves_recovery` test
passed. The real parent launched a child that used ordinary `write`, then
`graph_submit`. The parent accepted the exact PatchReport, with no timeout or
failure reason. The journal retained the assigned agent/node identities and an
effect record; recovery rejected another node and revoked authority, then
restored the original bytes for the authorized owner. This proves the worker
launch/edit/submission/recovery lifecycle, not a complete multi-node Graph run.

Commands: `cargo build -p davinci-coding-agent --bin davinci --offline --locked`;
`cargo test -p davinci-coding-agent --bin davinci --offline --locked
offline_tool_sequence_waits -- --quiet`; and `cargo test -p davinci-coding-agent
--lib --offline --locked transaction_parent_launch_accepts_submitted_edit --
--ignored --nocapture`. The launcher test requires `PI_GRAPH_WORKER_EXECUTABLE`,
isolated PI/DAVINCI config directories, offline/network-disabled environment,
and a `PI_OFFLINE_TOOL_CALL` array containing the exact write and PatchReport
submission asserted in its source. It ran one test successfully in 3.29 seconds.
Metadata hardening, final affected-package checks, review and CI remain open.

### Windows owner and primary-group preservation

The security-descriptor regression failed because capture contained only the
DACL. Capture now includes owner and primary group; staging restores these with
the DACL using the pinned file handle. The exclusively created stage requests
WRITE_OWNER as well as its existing permissions. No token privilege is enabled.
If Windows refuses restoration, staging fails before source replacement.
The API contract is documented by
[Microsoft SetSecurityInfo](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setsecurityinfo).

Both `windows_acl::tests` passed: protected DACL and a non-default primary group
survive edit/delete/recovery, and a group-only external change causes both apply
and rollback to refuse mutation while preserving current bytes/access. The latter
test initially expected a returned Conflicted summary; the API correctly returned
a conflict error, and the assertion now checks that contract. All 29 cases across
`transactions`, `transaction_commit` and `transaction_verification` passed after
the metadata change. Owner identity is captured and compared, but changing a file
to a different owner was not exercised under this local token.

Unix ownership/extended attributes remain open. Local environment inspection
found only the Docker Desktop WSL distribution and an unavailable Docker Linux
engine; no native Unix result is claimed. The program remains on P4.

### Unix ownership and mode implementation checkpoint

Transaction images now include optional numeric Unix owner/group IDs. Capture
checks ownership and mode before/after reading; staged replacement restores
ownership with descriptor-bound `fchown`, then the full `07777` mode, and verifies
both. It does not change process credentials or enable privileges. An OS refusal
fails staging before source replacement. Journal validation rejects sentinel IDs,
ownership on missing images, and Unix ownership records on non-Unix hosts.
Restored images must match the recorded ownership as well as bytes and mode.
Ownership is restored before mode because ownership changes may clear special
bits; see the [POSIX ownership contract](https://pubs.opengroup.org/onlinepubs/9699919799.2013edition/functions/chown.html).

The platform-independent ownership serialization test failed on the old image
schema, then passed. The journal ownership validation test also passed locally.
Two Unix-only integration tests cover edit/delete/fresh-coordinator recovery of
ownership/special mode and refusal after a special-mode-only external change.
They have been added but NOT executed: only the Windows Rust target is installed.
These local results do not prove `fchown` behavior or Unix compilation. Linux and
macOS CI remain required, as does extended-attribute/ACL preservation work.

### Extended-attribute implementation checkpoint

`unix_xattrs.rs` uses the existing libc descriptor APIs for Linux/macOS. Images
now persist extended attributes and compare them during apply/recovery. Staging
restores the saved attributes, removes extra inherited attributes for existing
files, and recaptures the result before publication. New files retain their
creation-time inherited attributes. Capture compares metadata again after source
reading. Limits are 64 names, 255 bytes/name, a 64 KiB name buffer, and 1 MiB
aggregate names/values per file, also charged to the transaction byte budget.
Unsupported name encoding, oversized values, races, and read/restore failures
refuse mutation. Names must currently be UTF-8. Filesystems explicitly reporting
no extended-attribute support are treated as having an empty attribute set.

The name parser and aggregate bounds tests passed locally. Linux/macOS filesystem
tests were added for binary/empty attributes across edit/delete/fresh recovery and
attribute-only changes before apply/rollback. Those native cases have NOT run on
this Windows host. The Windows agent library/test Clippy check passed. This does
not establish native compilation or complete Unix ACL preservation: macOS ACLs
need a separate adapter, and Linux ACL behavior still requires native evidence.

### Foreground command-output evidence checkpoint

`wait_shell_output` no longer converts reader I/O errors or thread panics into
successful partial/empty output. Both reader threads are joined on normal exit;
capture failure returns before the shell tool can record a completion receipt.
Each stream retains at most 16 MiB. Excess output is drained without retention
to avoid blocking the child on a full pipe, then reported as a capture failure.
Commands above this cap therefore return an error, not a truncated success or
verification receipt. This does not change background-job output handling.

The regression first failed when a reader error following a valid prefix was
silently accepted. After the fix, four `shell_capture_` tests passed, including
real child-process overflow on each stream, exact-boundary/empty input, reader
errors and reader panic. Two command-receipt tests and all four
`transaction_verification` integration tests passed. Agent library/test Clippy
with warnings denied passed. These are Windows results, not native Unix proof.

Outstanding P4 work still includes macOS ACL preservation, native Unix execution,
resumed-session verification discovery, final affected-package verification,
review and CI. The existing foreground wait can still wait on inherited pipe
handles held by descendants after the direct child exits; bounding retained bytes
does not solve that lifecycle issue and no bounded-drain-time claim is made here.

### macOS ACL implementation checkpoint

The descriptor-based adapter now persists optional portable binary ACLs in each
transaction image. It validates the magic, reserved owner/group fields, exact
length and maximum 128 entries before calling `acl_copy_int` (which has no length
argument). Aligned storage is used at the FFI boundary. Capture/restore uses
`acl_get_fd_np`/`acl_set_fd_np`; allocated ACLs are freed. Absent ACLs remain
distinct from empty ACLs, and an existing file's absent ACL removes inherited
stage ACLs. New files keep creation-time inheritance. Restore is recaptured and
compared before publication. ACL bytes are included in stale-state comparisons,
rollback validation and transaction size accounting. Non-macOS journals reject
macOS ACL data.

API/layout evidence: Apple's [ACL header](https://github.com/apple-oss-distributions/Libc/blob/main/include/sys/acl.h),
[portable serializer](https://github.com/apple-oss-distributions/Libc/blob/main/posix1e/acl_translate.c),
[descriptor adapter](https://github.com/apple-oss-distributions/Libc/blob/main/posix1e/acl_file.c),
and [kernel security layout](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/kauth.h).

Local Windows results: the format-boundary test passed; all 29 tests across
`transactions`, `transaction_commit` and `transaction_verification` passed;
agent library/test Clippy with warnings denied passed. Native macOS tests now
cover edit/delete/fresh recovery, ACL removal and ACL-only conflicts before
apply/rollback, but have NOT run. No native red/green result or macOS compilation
is claimed. Linux ACL behavior, macOS execution, foreground descendant pipe
lifecycle, resumed verification and final package/review/CI gates remain open.

### Resumed verification discovery checkpoint

The affected-package gate completed on Windows before this follow-up:
`cargo test -p davinci-agent -p davinci-coding-agent --offline --locked`
passed 3,265 tests with 21 ignored across 34 suites. Native-only tests remain
unexecuted locally.

Foreground verification now discovers eligible durable transactions instead of
depending on the current process's effect ledger. Discovery does not create a
store in an untouched workspace. It bounds record count to 128 and cumulative
serialized reads to the existing 256 MiB store budget (plus one overflow-detection
byte), validates records, and selects only applied/verified transactions owned
by the current agent or an explicitly permitted resumed root session. Foreign
sessions, tasks, parents and Graph nodes remain isolated. Corrupt or inaccessible
records cannot authorize verification. Every selected transaction still undergoes
current read authorization, source checks and receipt/coverage validation.

The actual-Cargo normal-dispatch regression first failed for a resumed session
(`applied` instead of `verified`), then passed after the fix. Its eight cases cover
ordinary success, fresh-agent session resume, no-runtime operation, changed
source, hook veto, uncovered source, denied reads and permission revocation.
Two discovery tests passed for ownership/lifecycle boundaries, no-store behavior
and shared read-budget exhaustion. Agent library/test Clippy passed after the
production change. No performance improvement is claimed: discovery reads durable
records before qualifying workspace commands, with a bounded worst-case cost.

P4 remains incomplete pending native Linux/macOS execution, foreground descendant
pipe lifecycle, final assembled checks, review and CI. No P4 commit or push has
been made at this checkpoint.

### First native CI follow-up

Implementation commit `948130fd4c089d8db830ed74dbfd07192ae9f06a` was pushed
as draft PR #11. CI run `35273255591` passed the Linux native matrix, including
transaction metadata, recovery, CLI/intelligence integration, actual Graph worker
submit/recovery and the transaction evaluation. Quality checks and the coding-agent
package passed. The agent package failed with a transient Unix transaction lease
conflict; Windows and macOS native lanes failed earlier in normal edit acceptance
with `transaction target is outside the workspace`.

The directory lease now explicitly releases Unix flock on scope exit so an
inherited open file description cannot prolong the parent's lock. Its regression
duplicates that description and checks both release and subsequent exclusion.
Windows still uses exclusive handle lifetime. The native matrix now runs these
lease tests on all three operating systems. Unix execution of this follow-up is
pending CI; the local Windows machine has no running Linux test environment.

Transaction snapshots now retain the host's workspace spelling when deriving
relative paths, while validating the alias still resolves to the pinned canonical
root. Targets are not canonicalized to bypass symlink rejection. A case-alias
regression failed locally before the change and passed afterward; the Unix variant
also covers a workspace alias and rejection of a target symlink. The 30 transaction
library tests, six cache library tests and agent all-target Clippy passed locally.
P4 remains incomplete: follow-up native CI and foreground descendant lifecycle
are still required before the final completion audit.

### Supervisor capture and dispatch alias follow-up

CI run `35274279940` for `5d9ac196c3273cfa7640c4de8e6f84066beb8ea4`
passed all workspace package tests, quality checks and the Linux native matrix,
including the new inherited-descriptor lease regression. Windows and macOS
normal-agent acceptance exposed the next alias mismatch at dispatch authorization.
The alias regression now goes through authorized apply: it failed with
`transaction target was not authorized by this dispatch`, then passed after
authorization learned the equivalent canonical workspace prefix. This only
substitutes the workspace prefix and revalidates the alias; target symlinks do not
grant authority to additional files. Native confirmation remains pending.

The existing supervisor now distinguishes pipe EOF from read failure, retries
interrupted reads, and requires successful reader completion and delivery before
reporting complete output. Reader panic and a descendant retaining a pipe remain
incomplete output. Two failure regressions were red before the fix; all three
capture tests pass. A separate stderr callback preserves foreground stream
identity while the existing spawn API retains merged output. The real subprocess
stream regression was red before tagging streams and green afterward. An
acknowledged stdin-close operation supports EOF after acknowledged writes and
rejects later writes. All 17 supervisor integration tests and 30 transaction
library tests pass locally, as does agent all-target Clippy with warnings denied.

These are supervisor prerequisites, not completed foreground shell integration.
The shell tools still need to consume the owned supervisor and prove bounded
timeout/cancellation/descendant cleanup before P4 can be marked complete.

### Foreground host integration and macOS fixture correction

The CLI now installs a foreground supervisor independently of the optional
process-manager setting. Foreground Bash and PowerShell use that existing owned
lifetime, retain separate bounded streams, acknowledge stdin EOF, and reject
incomplete output before creating a command receipt. A focused regression was
red before the adapter existed and now passes for literal stdin, nonzero exit,
stream hashes, real `exec_command`, timeout, cancellation, and a descendant
holding explicitly inherited pipes. The descendant listener is closed before
return. An initial Node-only inheritance fixture did not reproduce held handles
on Windows and was replaced with explicit Rust handle inheritance.

Local validation: the foreground regression, 30 transaction library tests,
normal-agent test-impact acceptance, four existing packaged transaction CLI
tests, and the new packaged foreground test passed. The latter disables managed
process tools while running a real foreground command. Agent and coding-agent
all-target Clippy, formatting and diff checks passed. All five packaged CLI
tests passed together after the fixture update. CI remains required for this head.

CI `35275466425` for `e3c43df197be2052ae6f69a1d02e7c28a59f1da3`
passed Linux native, all workspace packages and quality. macOS passed the normal
edit path, then failed five internal store fixtures with `Not a directory`.
Those fixtures passed the noncanonical temporary-directory alias directly to
the no-follow store. They now canonicalize the fixture root as the production
coordinator already does. The production no-follow protection is unchanged.
Native macOS confirmation is pending the next CI run.

The same prior-head run also finished Windows with four failures in recovery and
explicit rollback tests: `transaction restore image mismatch`. These passed on
the local Windows machine, so runner metadata differences require diagnosis.
The failing job is `105384905165`; no success is claimed for that platform gate.

P4 is still incomplete. Standalone library contexts without an installed host
retain the legacy foreground path; its lifecycle and verification contract still
need resolution. Final assembled package checks, platform CI and the P4 completion
audit remain open. P3 has not started.

### Runner diagnosis checkpoint

CI `35276628781` for `0284d19ddebe338fc4700c2731404b584aa7816c`
reported an agent-package failure in the new foreground regression: the Linux
descendant listener accepted a connection after the foreground call returned.
The earlier listener-closure result above applies only to local Windows testing;
it is not a cross-platform guarantee. Native matrix jobs were still running when
this failure was observed. Cleanup completion remains an open gate.

Restore-image validation now identifies the first mismatching field without
printing contents, descriptors, attributes, or stream values. All prior equality
checks remain enforced. The focused diagnostic regression and formatting check
passed locally. This is diagnostic instrumentation, not a Windows rollback fix;
the next runner failure must identify the field before changing restoration.

The Linux and macOS native failures in that run both resolve to the same new
instantaneous listener assertion, after native transaction metadata tests passed.
The foreground fixture now observes closure within two seconds, following the
existing supervisor tests' bounded cleanup contract. Its ten-second self-exit
cannot satisfy this deadline after the foreground return bound of five seconds.
This corrects a scheduling-sensitive assertion; it does not change termination
code or claim synchronous socket closure when the helper is reaped. The focused
foreground regression passed locally; native confirmation remains required.

### Windows legacy DACL reproduction (working change, not ready to ship)

Diagnostic CI `35277212922`, Windows job `105390598493`, confirmed `access`
as the mismatching field in all four rollback/recovery failures. A local legacy
inheritance fixture reproduced descriptor drift: SetSecurityInfo introduces the
auto-inherited flag, and ReplaceFileW can rewrite a staged descriptor during
replacement. The regression failed before the working change.

The working tree experiments with NtSetSecurityObject on the owned staging
handle and intact source-image rename. It retains the old journal replacement
path separately. Legacy edit/delete rollback and durable status reload now pass;
all 32 transaction-filtered agent library tests and agent all-target Clippy pass
locally. This is not yet a completed fix: ReplaceFileW also preserves creation
time, short names, object identifiers, security resource attributes, encryption,
and compression. The new source rename must account for this metadata before
the change is committed. Do not treat the green ACL tests as proof of full file
metadata preservation. No native CI has run these uncommitted changes.

API references: [native descriptor setter](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetsecurityobject),
[inheritance conversion](https://learn.microsoft.com/en-us/windows/win32/secauthz/automatic-propagation-of-inheritable-aces),
and [ReplaceFileW metadata contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew).

CI `35277525581` for pushed `8cf4a9667bc373fd7cdc75528fc438de934692e3`
completed: Linux and macOS native checks, all workspace package jobs, quality,
and the workspace aggregate passed. Windows native job `105391600967` still
failed the transaction metadata step. Workflow lint `35277525613` and Security
SARIF `35277525689` passed. This confirms the bounded foreground cleanup fixture
on both Unix platforms; it does not cover the pending Windows working-tree fix.

### Windows metadata checkpoint (uncommitted)

The creation-time/hidden/system/not-content-indexed regression failed on the
working rename implementation: the replacement acquired its staging creation
time. Images now capture Windows creation time and attributes, include them in
capture stability and stale-state comparison, and restore them through the owned
staging handle. Durable restore validation compares the new metadata field.
Missing-file images cannot carry it; non-Windows record validation rejects it.

A compressed-file edit/delete regression then failed because setting basic
attributes does not enable NTFS compression. Staging now restores compression
with FSCTL_SET_COMPRESSION before setting basic information and verifies the
result. The extra read access is restricted to the exclusively created stage;
source access is unchanged. Attribute-only changes after preview/apply conflict
without overwriting the changed file. Creation-time, attribute, and compression
restoration regressions pass for edit and delete rollback.

Validation actually run at this checkpoint:

- `rtk cargo test -p davinci-agent --offline --locked --lib transaction_preserves_windows_creation_time_and_attributes`: RED, then GREEN.
- `rtk cargo test -p davinci-agent --offline --locked --lib transaction_preserves_compression_after_edit_and_delete`: RED before compression restoration.
- `rtk cargo test -p davinci-agent --offline --locked --lib transaction`: 35 passed, 889 filtered; includes both regressions and attribute conflicts.
- `rtk cargo fmt --all`: passed.
- `rtk cargo clippy -p davinci-agent --offline --locked --all-targets -- -D warnings`: passed.
- `rtk git diff --check`: passed before this documentation addition.

Remaining before shipping: encryption and security resource attributes, short
names and object identifiers, compatibility of prior persisted images without
the new metadata field, and native CI for the combined change. Exact creation
times represented by Windows API sentinel values currently fail staging rather
than silently changing them. Full Windows metadata preservation is not proven.
The separate no-host foreground execution/evidence contract also remains open.

API references: [FILE_BASIC_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_basic_info)
and [FSCTL_SET_COMPRESSION](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_set_compression).

### Security resource attribute checkpoint (uncommitted)

The next RED regression created a real Windows resource ACE (`Department`),
then demonstrated that source replacement lost it. ACL snapshots now query
ATTRIBUTE_SECURITY_INFORMATION in addition to owner/group/DACL; the native
staging setter restores those properties with existing WRITE_DAC rights.
There is no privilege adjustment or request for audit-SACL access.

The wider transaction filter exposed four recovery failures: restoring absent
properties causes Windows to materialize a null filtered resource SACL. The
snapshot encoder now represents absent/null/empty resource-only SACLs as no
resource properties, while retaining every nonempty resource ACE. This rule is
not applied to the DACL. The query does not capture audit ACEs, so this is not a
claim that audit SACL backup/restore is implemented.

Validation: resource-preservation regression RED then GREEN for edit and delete
rollback; all 37 transaction-filtered agent library tests passed (889 filtered).
The strengthened resource-conflict regression also passed separately: changing
an existing property value after preview or apply conflicts and preserves both
the other actor's value and current bytes. Formatting, final agent all-target
Clippy with warnings denied, and diff whitespace checks passed. No native CI
has run these uncommitted changes.

Still open: encryption, short-name/object-ID handling, old images without the
Windows metadata field, combined native CI, and the no-host foreground contract.
Reference: [Windows security information access rights](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-information).

### Foreground evidence boundary checkpoint

The no-host evidence defect was reproduced: an `exec_command` without a trusted
foreground supervisor returned a successful verification receipt. Shell and
PowerShell receipt capture now occurs only after the supervised adapter returns
complete, bounded output. Legacy execution remains available without a receipt;
its successful exit alone cannot mark a transaction verified. This does not yet
repair the legacy runner's descendant-held pipe or cancellation lifecycle.

The real-command receipt and hook-veto tests now launch the existing supervisor
fixture, rather than relying on the legacy fallback. The normal transaction
verification test retains all previous success/stale/permission/hook cases and
adds no-supervisor execution, which must leave the transaction applied. CLI host
configuration is unchanged.

Validation actually run:

- `rtk cargo test -p davinci-agent --offline --locked --lib unsupervised_commands_do_not_produce_verification_receipts`: RED before the fix.
- `rtk cargo test -p davinci-agent --offline --locked --lib command_receipt`: four passed, including successful legacy shell/PowerShell execution without receipts, supervised receipts, and hook veto.
- `rtk cargo test -p davinci-agent --offline --locked --lib normal_command_verifies_transaction_only_after_unchanged_source_and_hooks`: one passed; covers nine normal-session variants using actual Cargo execution.
- `rtk cargo fmt --all`: passed.
- `rtk cargo clippy -p davinci-agent --offline --locked --all-targets -- -D warnings`: passed.

The evidence boundary is locally fixed. Legacy fallback lifecycle, remaining
Windows metadata/recovery work, and combined native CI remain required. No
subproject completion or exact-head CI claim follows from this checkpoint.

### Legacy PowerShell timeout correction

The fallback PowerShell runner passed no timeout to its wait loop. A real
`Start-Sleep -Seconds 2` command with a 0.2-second timeout incorrectly succeeded
after 2.43 seconds (RED). Timeout parsing now happens before any PowerShell spawn
and feeds both supervised and fallback execution. The regression passed in
0.37 seconds (GREEN), and verifies that a zero timeout rejects a command before
it creates its marker file. No receipt is produced on timeout.

Final focused validation: `rtk cargo test -p davinci-agent --offline --locked
--lib powershell` passed all three matching tests; formatting and agent all-target
Clippy with warnings denied passed. The execution/evidence correction is a
separate commit from the unfinished Windows metadata working changes. Legacy
descendant-held pipe cleanup remains open; honoring the timer for a direct
PowerShell process does not prove descendant cleanup. Native CI is still required.

### Windows journal format boundary

Windows records now use schema 2 because captured files require creation time
and attributes. Other platforms retain schema 1. Loading a Windows schema-1
record returns an explicit unsupported-schema error and leaves the record and
source intact; there is no automatic migration that guesses lost metadata from
the current source. Recover legacy records with the original implementation
before upgrading. This is an intentional compatibility boundary, not a claim
that old records have been migrated or their recovery defects repaired.

Schema-2 captured images with omitted Windows metadata are also rejected before
mutation. New-file proposals may lack metadata until a real staged file exists.
The regression first demonstrated that a legacy record was incorrectly applied
(RED), then passed after the format gate (GREEN), checking both unchanged source
bytes and unchanged journal bytes. The targeted transaction tests passed:
`rtk cargo test -p davinci-agent --offline --locked --lib transaction` — 38 passed.
These changes remain part of the uncommitted Windows metadata work; remaining
replacement metadata and native CI gates still prevent P4 completion.

### Inherited compression validation and current CI

Added a real NTFS fixture that marks its temporary parent directory compressed,
confirms a newly created file inherits compression, then explicitly uncompresses
that file. Transaction edit and delete/rollback preserve its uncompressed state,
creation time, attributes, and bytes even though each private stage inherits
compression from the parent. This exercises the clear-compression branch rather
than only compression enablement. The three `windows_metadata` tests passed;
formatting passed. Directory inheritance semantics were checked against
[FSCTL_SET_COMPRESSION](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_set_compression).

On pushed commit `92bc071`, CI run `35280267775` passed workspace tests and
quality; Windows native job `105400313288` failed with the same four
`transaction restore image mismatch: access` errors in source-stage failure,
journal failure, rollback-journal failure, and explicit-dispatch recovery tests.
The working Windows changes were not present in that run. Linux native passed;
macOS native was still building its transaction worker at the last observation.

### Object-ID safety boundary

The Windows staging format cannot transfer an NTFS object identifier safely.
Source capture now queries the pinned handle before reading contents and refuses
files that carry one. It never creates, deletes, or transfers production object
IDs. This is an explicit unsupported-file case, not object-ID-preserving edit
support. It prevents silently losing identifiers through replacement or delete.
Microsoft documents that applications should not modify these identifiers:
[FSCTL_GET_OBJECT_ID](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_get_object_id).

A temporary NTFS fixture creates its own object ID, then proves edit and delete
previews refuse while the source bytes and complete identifier buffer remain
unchanged. After correcting the fixture to close its write handle before source
capture, the regression failed because preview incorrectly succeeded (RED), then
passed after the query guard (GREEN). Local NTFS reports error 2 for a missing
object ID on an existing pinned file; the initial guard rejected that case and
the transaction suite caught it. The corrected guard accepts that absence,
explicit no-object errors, and unsupported control codes, while propagating
other query failures.

Validation: all 40 `--lib transaction` tests passed; after consolidating the
Windows API declaration, all four `--lib windows_metadata` tests and agent
all-target Clippy with warnings denied passed. Formatting passed. No native CI
has tested these working changes. CI run `35280267775` is now terminal failure
on the pushed commit. Encryption and short-name preservation remain unresolved;
the object-ID refusal does not establish complete replacement metadata safety.

### Encrypted-source journal boundary

Source capture now rejects FILE_ATTRIBUTE_ENCRYPTED on the pinned Windows file
before reading default or named streams. EFS read handles expose decrypted data;
the transaction JSON journal has no encrypted recovery-blob format. This prevents
silently recording decrypted preimages. Loading a journal image marked encrypted
also fails before apply changes the source. Encrypted-file editing remains
unsupported; this is not an implementation of encrypted recovery storage.

The attribute-level regression failed before enforcement and passed afterwards.
The real journal/apply rejection test additionally injects the encrypted bit into
a stored image and verifies both source and record bytes remain unchanged.
`rtk cargo test -p davinci-agent --offline --locked --lib transaction` passed
41 tests. Formatting and diff checks passed. No live EFS fixture was created:
validation covers the metadata policy and journal/apply boundary, not an actual
encrypted-file lifecycle. Short-name handling and native CI for the working
metadata changes remain outstanding.

### Windows short-name boundary and private stages

Source capture now queries the pinned handle for an alternate short name and
refuses replacement when one exists. The NTFS fixture assigns `ALIAS.TXT` and
verifies both edit and delete preview preserve the source and alias. This is an
unsupported-source boundary, not short-name-preserving editing; it also affects
ordinary files on volumes that automatically assign 8.3 names.

Exclusively created private stages request DELETE access and backup semantics so
their generated alias can be removed before content is written. Stage preparation
then checks the same encryption/object-ID/short-name constraints as source
capture. A real fixture assigns `STAGE.TMP` to a private stage, prepares and
publishes it, and verifies the content exists only under the published long name.
No source alias is removed by this operation.

Validation: all 43 focused `davinci-agent --lib transaction` tests passed on
Windows, including private-stage publication. Formatting and agent all-target
Clippy with warnings denied passed. These changes remain unpublished and native
CI has not validated them. Full security metadata coverage and source short-name
support remain limitations; this checkpoint does not complete Project 4.

### Mandatory integrity label preservation

A real NTFS fixture with an explicit low-integrity, no-write-up label reproduced
another replacement defect: apply dropped the label (RED). The bounded security
snapshot now includes LABEL_SECURITY_INFORMATION alongside resource attributes;
stage restoration sets both using the existing handle rights. Empty filtered
SACLs are canonicalized without changing DACL semantics. This requires no token
privilege changes: Microsoft documents READ_CONTROL for label queries and
WRITE_OWNER for setting labels in
[SECURITY_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-information).

The fixture now verifies exact descriptor preservation after edit, delete and
rollback (GREEN). All 44 focused transaction tests passed; formatting and agent
all-target Clippy with warnings denied passed. Full audit SACL preservation is
not established by this test, and native CI for these changes remains pending.

### Integrity-label conflict validation and publication

The metadata work above is published as `b4781e4` on draft PR #11. Native CI run
`35282774087` is live at this checkpoint. A follow-up real-file test changes only
the integrity label after preview or apply, then proves apply/rollback returns
conflict and preserves both the external label and expected source bytes. Both
integrity-label tests passed; formatting and all-target agent Clippy passed.

The foreground lifecycle review confirms packaged sessions install the existing
supervisor in `main.rs`. The library fallback without a supplied supervisor still
joins descendant-held pipes without a deadline and cannot issue verification
receipts. That limitation remains explicit; the label tests do not validate or
repair that separate lifecycle path.

### Named-stream restoration sharing regression

The Windows integration lane exposed three failures after `b4781e4`: applying
or recovering files with named streams returned sharing violation 32. The new
private base handle's DELETE access (needed for staging short-name cleanup)
was incompatible with the named-stream restoration handle's share flags.
Only private restoration now includes FILE_SHARE_DELETE. The existing base
handle continues to deny deletion; source capture continues denying writes and
deletion. No source sharing policy is relaxed.

The existing edit, deleted-stream recovery and stream-conflict integration tests
provided RED. After the fix all four Windows-specific cases and all 22 transaction
integration tests passed. The preceding run separately passed three commit tests,
four verification tests and the offline rollback evaluation. Formatting and
agent all-target Clippy passed. The integrity-label conflict follow-up also
passed both focused cases. Native CI must still validate the corrected head.

### Native Windows short-name compatibility regression

Implementation head `98e363d` failed CI run `35283245859`, Windows job
`105409749119`, at `normal_agent_discovers_edits_replans_and_runs_tests_without_graph`:
`transaction cannot preserve an existing Windows short name`. The runner assigns
8.3 names to ordinary files. Refusing those files is therefore a demonstrated
normal-edit compatibility defect, not an acceptable completion boundary.

Documentation head `3106598` adds the capability guide; CI run `35283684663` is
active. That documentation-only commit does not repair this failure.

The local short-name fixture now requires successful edit, delete and recovery
while preserving the explicit alias. Its focused run failed at preview with the
same error (0 passed, 1 failed), establishing RED. The regression is intentionally
uncommitted pending implementation. A separate temporary NTFS probe confirmed
that MoveFileExW replacement removes an existing ALIAS.TXT alias, so removing the
guard alone is incorrect. The probe changed only its temporary fixture.

Next implementation must persist the captured alias, detect alias-only conflicts,
and restore it with recovery coverage for interruption between content publication
and alias restoration. It must preserve existing descriptor/stream checks and
avoid adopting or deleting an alias claimed by another file. ReplaceFileW is not
a drop-in fix because its descriptor merging caused the earlier inheritance
regression. Add failure-window cases before claiming short-name recovery complete.
P4 remains open; do not start P3 while this native failure is unresolved.

Short-name capture preparation now decodes the handle-bound alternate-name
response into bounded UTF-16 units, rejecting invalid byte lengths and path
separators. The private-stage fixture checks the exact alias before clearing it
and an empty alias afterward. Its missing-helper RED was followed by one passing
focused test. `cargo fmt --all` and `git diff --check` passed. The end-to-end
edit/delete/recovery regression still fails at the existing preview guard; alias
journaling and interruption-safe publication remain unimplemented. These changes
are uncommitted and do not close the Windows compatibility gate.

## Windows short-name preservation implementation

The formerly failing alias edit/delete/fresh-host recovery regression now passes,
including named streams. Windows journal schema 3 records bounded aliases and
pending publication separately from the final file image. Private stages keep no
alias; publication restores the captured alias using a pinned DELETE handle after
verifying the full staged image. Completed publications clear their pending flag
durably, so later alias-only changes remain conflicts.

Interrupted apply and interrupted rollback fixtures exercise the real durable
journal and rename boundary. Recovery preserves a foreign alias claimant, and a
fresh host can retry after that fixture releases its alias. Interrupted rollback
finishes the alias on the already-restored file without replacing its durable
identity. Alias-addressed source edits are refused to preserve primary filenames.
Schema 1/2 records remain untouched and require their original recovery version.

Executed validation for this follow-up:

- `cargo test -p davinci-agent --offline --locked --lib transaction`: 49 passed.
- Agent transaction, commit, verification and evaluation integration targets:
  30 passed across four suites.
- `cargo test -p davinci-coding-agent --offline --locked --bin davinci test_impact_integration_tests`:
  one passed, matching the previously failing native CI step. An earlier `--lib`
  selector ran zero tests and is not counted as validation.
- Agent all-target Clippy with warnings denied passed; formatting and diff checks
  passed. Final review was solo as explicitly requested.

Previous-head CI `35283684663` is terminal: Linux/macOS native paths, quality and
workspace shards passed; Windows failed the known ordinary-edit short-name gate.
This implementation still requires its own exact-head CI before closing P4.
