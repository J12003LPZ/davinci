# Native security scan implementation checkpoint

Date: 2026-09-07. Status: **Runnable experimental implementation; full-plan quality
and platform acceptance remain incomplete.** This supersedes the earlier
foundation-only checkpoint. The user explicitly authorized implementation.
No subagents, commits, or pushes were used. Unrelated dirty work was preserved.

## Implemented

- Dedicated selected-provider composition at the executable boundary. Existing
  authorization and effective reasoning provenance; no generic agent, MCP, hooks,
  auth helpers, or legacy substring findings in the new review.
- Strict command/config parsing; immutable worktree and Git changed/diff snapshots;
  baseline/head source identities; ignored-file handling; bounded, confined reads
  and credential-path exclusions.
- Five versioned native methodology documents. Workers can only list, search and
  read snapshot source. Claimed coverage and citations require actual retrieval.
- Standard audit plus independent counter-review; two fresh audits in Deep mode.
  Bounded requests, output, token reservations, deadlines and cancellation.
  Five assessment gates, exact-claim deduplication, and all-candidate dispositions.
  Deferred claims now keep coverage incomplete.
- Shared asynchronous TUI/RPC controller, legacy session continuity, print/JSON
  completion, status/abort/resume, and default TUI/RPC shutdown cancellation.
- Private durable snapshots outside the repository, exclusive leases, immutable
  report generations, projection hashes and seal verification. Resume preserves
  the original snapshot. Fully covered completed reports cannot be resumed.
- JSON, terminal and SARIF output; configured blocker/incomplete/cancel exit codes;
  report redaction; disabled general-memory indexing and provider wire tracing.
- Explicit offline fixture provider marked synthetic-only, plus an evaluation
  scorer. Neither constitutes measured real-model detection quality.

Important files: native_extensions/security_scan.rs and its new module directory,
main.rs, davinci_interactive.rs, extension_host.rs, settings.rs, native command
registration, and davinci-evals/src/security_eval.rs.
Usage: [docs/security-scan.md](../../security-scan.md).

## Verification and delivery

- Workspace tests: **2,199 passed, one ignored**, 31 suites, before the final small
  UI and deferred-coverage corrections.
- Affected-crate Clippy with all targets and warnings denied passed before those
  corrections. Final targeted rechecks are recorded below.
- Release build passed. The active path is
  C:\Users\sergi\.cargo\bin\davinci.exe. Original executable backup:
  davinci.exe.security-scan-backup-20260907-003413.
- Actual release and installed executables passed offline terminal/JSON/SARIF
  scans, invalid-argument exit 2, JSON event routing, persistent RPC start/status/
  report, and saved projection SHA-256 checks.
- Windows PTY inspection reached SECURITY REPORT, Files: 1/1, and completed
  coverage. It exposed a UUID mislabeled as SHA-256; the UI now leaves the hash
  field empty and labels the scan identifier correctly.
- Synthetic reportable-candidate exit 1 passed. Deferred-candidate testing
  reproduced incorrect exit 0 in the earlier binary; the correction requires
  incomplete coverage/exit 2 and has a dedicated inline regression test.
- Rustfmt ran on affected implementation files; git diff --check passed.
  Test counts include existing and duplicated library/binary modules.

No live provider calls were made. RTK was used throughout. Headroom was attempted,
but its local proxy was unreachable; no Headroom savings are claimed.

### Final revalidation

- Focused security tests after all source corrections: **102 passed**, 925 filtered
  out across two suites. Affected-crate all-target Clippy passed with warnings denied.
- Final release build passed with offline/locked dependencies. Installed and built
  executable SHA-256 matched:
  `940FE7701AB2286EB8B61E038A6D010C65F1B20592A46CC058A67350A5C8D323`.
  The resolved installed executable returned version 1.0.0.
- Repeated all CLI/RPC/projection checks against that installed path: passed.
  Synthetic confirmed-high exit 1 and deferred/incomplete exit 2: passed.
- Windows PTY on the installed executable showed `SECURITY REPORT`, `Files: 1/1`,
  and `Experimental · completed · coverage complete`; the false hash label was
  absent. The assertion requires the full phrase, not a substring of incomplete.
- Local smoke evidence is in
  `C:\Users\sergi\AppData\Local\Temp\davinci-security-smoke-gizc15d3`;
  the temporary harness is `C:\Users\sergi\AppData\Local\Temp\security-scan-smoke.py`.
  These temporary artifacts are not committed regression infrastructure.

## Remaining plan work

The nine-phase plan is **not fully complete**:

1. Held-out real-model evaluation, measured precision/recall and usage, and measured
   80% test coverage remain outstanding. The scorer alone is not quality evidence.
2. Optional offline cargo-audit is not implemented. External analyzers, dynamic
   execution, network tools and memory indexing remain disabled/rejected.
3. Complete fourteen-source-skill migration mapping and deterministic root-to-leaf
   policy/reconnaissance artifacts remain outstanding.
4. Git retains both rename sides without explicit rename-pair metadata. Bare diff
   needs a parent commit. Broader Git error differentiation and cancellation inside
   revision capture need further work.
5. Full planned typed report envelope, terminal drill-down, feedback expiration,
   actual-usage ledger and semantic root-cause deduplication remain outstanding.
6. SARIF JSON/projection checks passed, but official-schema validation has not run;
   local Python has no jsonschema package.
7. Linux/macOS release acceptance, broader adversarial filesystem/ACL and
   process-descendant tests, torn-checkpoint recovery and actual signal-delivery
   acceptance remain unverified. Local hashes are not signatures against an actor
   who can rewrite all artifacts and seals.
8. Live OAuth refresh/custom-auth compatibility has not been exercised. Admission
   requires existing usable selected-provider authorization.

Existing deterministic graph security behavior remains separate. Restart running
Davinci sessions to load the installed implementation.

## Production-readiness goal — active

The subsequent user request requires production readiness, not merely a runnable
experimental release. The entire original plan remains the acceptance contract;
the limitations above are unfinished requirements, not waivers. No completion or
external-blocker determination has been made.

First hardening pass:

- Added a regression reproducing corrupt HEAD being silently treated as unborn.
  Git now distinguishes a missing symbolic branch from damaged refs/objects and
  command failures. Git replacement objects are disabled for source identity.
- Bare diff on an initial commit now captures the committed tree with an explicit
  empty baseline. First-parent inspection uses commit headers and fails if a
  required parent is unavailable rather than treating shallow history as empty.
- Cancellation propagates into inventory, ref resolution, Git process wait loops,
  revision-object traversal and nested worktree capture. A cancellation-before-
  spawn test proves that a requested Git mutation in the fixture never starts.
- Focused tests: 108 passed across library/binary targets. These additions are
  source changes after the last installed release; no new production-ready binary
  has been declared or installed during this pass.

Next acceptance work remains all nine phases, especially policy/reconnaissance,
schema/report lifecycle and recovery, capability/adversarial tests, analyzer
containment, evaluator-only held-out labels and real-model quality measurements.
Each item must be verified against its original numbered requirements before
marking this goal complete. Platform and provider availability have not yet been
established as blockers.

Policy-context pass:

- Added typed root-to-leaf policy anchors with source hash, revision side and line
  count, supplied in initial and paginated worker inventories. Policy text is not
  interpreted as permissions, tools, exclusions or disclosure authorization.
- Missing ancestor policy bytes are explicitly unresolved, including when a
  narrower scope captures a leaf policy but not the root policy. The resolver
  does not claim that an uncaptured policy does not exist.
- Tests cover source identity, root/leaf ordering, malicious policy requesting
  shell access, wrong-side/out-of-snapshot reads and narrowed-scope gaps.
- The integrated security suite passed 112 tests before the final narrowed-scope
  assertion; focused policy revalidation and Clippy are recorded by the active
  run. This is policy indexing, not completion of the full reconnaissance phase:
  authorized supporting-source capture and architecture-aware review units remain.

Unresolved-merge pass:

- Snapshots now retain validated index-stage path, stage number, object ID and
  mode. Conflict identities participate in snapshot identity and appear in the
  report source record. Conflicted paths explicitly defer coverage.
- Disposable Git fixtures create a real merge conflict and check both worktree
  and changed selection, including deletion of the conflicted working file.
- Integrated security tests passed 114 tests before the final deleted-file case;
  the final merge regression and Clippy also passed. Stage identities are retained,
  but dedicated retrieval of stage bytes remains part of the full source-evidence
  work; this pass does not claim complete conflict analysis.

Reporting-policy pass:

- Implemented the plan's explicit trusted-policy option to block likely findings.
  Project narrowing unions blocking classifications and preserves the stricter
  severity threshold independently; it cannot discard parent blockers.
- Malformed failure policies and non-completed run states return exit 2; explicit
  cancellation returns 130. Defaults remain confirmed High/Critical.
- Terminal reports now expose the causal path, role/side-aware source locations,
  remediation and proof gaps, plus a separate unresolved-lead section.
- Added regressions that first failed for likely blocking and omitted causal
  output. Final focused security suite: 120 passed; all-target Clippy passed.
  These checks establish report/control behavior, not real-model quality.

Persistence and independent-coverage pass:

- Seals and reports now validate schema, scan identity and generation together;
  report publication requires a reserved generation. Resume validates normalized
  source paths and recounts captured bytes instead of trusting stored totals.
- New checkpoints atomically publish one identity-bound bundle containing source
  data and checksum. Legacy pairs remain readable when intact. Corrupt bundles
  cannot downgrade to a legacy pair. Failed writes clean their own temporary file;
  Unix publication and generation reservation also sync the containing directory.
- A failing regression showed Deep mode could combine two disjoint partial audits
  into complete coverage. Each audit now retains current/baseline reviewed and
  deferred paths, limitations and completeness; every required audit must complete.
  Terminal output exposes deferred rows with explicit bounded-display counts.
- Final focused security suite: 138 passed across library/binary targets;
  coding-agent all-target Clippy with warnings denied passed. Changed Rust files
  were formatted; Git diff whitespace validation passed. Windows fixture tests
  cover bundle publication, legacy reads, corruption and failed-write cleanup.
- These are source changes after the last installed release. Unix directory sync
  is compiled conditionally and has not been exercised on Unix in this pass.
  Architecture-aware mapping/supporting source, complete report lifecycle,
  analyzers, adversarial acceptance, real-model quality gates and final installed
  release/platform acceptance remain required; production readiness is unproven.

Aggregate budget and usage pass:

- Regression demonstrated that minimum-one-turn validation allocations could
  exceed the mode's total model-turn reserve. Allocation now partitions the exact
  reserve across candidates; zero-allowance candidates defer without provider calls.
  Tests cover every candidate count through the two-audit maximum of 128.
- Provider usage with zero or inconsistent totals no longer erases pre-request
  reservations. Accounting includes normalized input, output and cache tokens;
  reasoning tokens are not double-counted. A repeated zero-usage provider fixture
  now stops at token exhaustion before reaching its turn limit.
- Status and sealed reports retain per-request measured tokens (or null), budget
  charges, reservations, duration and provider failure state. Positive catalog
  price estimates are labeled estimates; unknown/default-zero prices stay null.
  Terminal summaries distinguish measured totals from conservative accounting;
  SARIF retains the same request records in run properties.
- Final focused security suite: 152 passed across library/binary targets.
  Coding-agent all-target Clippy with warnings denied passed. Rust changes were
  formatted. These remain offline engineering checks, not quality evaluation.
- Original acceptance scope remains active. Supporting-source capture and
  architecture-aware mapping, complete report/lifecycle contracts, analyzer
  containment, adversarial corpus, held-out real-model results, platform checks
  and the final installed release remain outstanding. No new external blocker
  or production-readiness claim was established by this pass.

Finding report retrieval pass:

- Added strict `/sec-report [scan-id] --finding <finding-id>` parsing with
  canonical identities and rejection of duplicate, missing and unknown arguments.
  Active and stored native reports resolve exactly one matching finding.
- Selection is a read-only projection: canonical findings, source identity,
  generation, coverage and policy verdict remain intact. JSON adds the selected
  record and identity; terminal output renders its details. A stored-report
  regression rereads the sealed report to prove selection did not mutate it.
- Native TUI dispatch selects the matching finding row. Print-mode native report
  retrieval now uses the report renderer and policy exit status; JSON mode emits
  a security_scan_report event without the legacy slash-command prefix. Legacy
  report rendering remains separate.
- Final focused security suite: 158 passed across library/binary targets;
  all-target coding-agent Clippy with warnings denied passed. Changed Rust files
  were formatted and scoped Git diff whitespace validation passed.
- Actual installed-binary and visible TUI acceptance of this selector have not
  been performed yet. Full production acceptance and all previously recorded
  unfinished phases remain required; this pass does not complete the goal.

Installed release integration pass:

- Built the current release with `cargo build -p davinci-coding-agent --release
  --offline --locked`. Release and installed executable smoke runs passed terminal,
  JSON, SARIF, JSON-event, RPC start/status/report, invalid-argument exit 2,
  synthetic confirmed exit 1 and deferred exit 2 contracts.
- Extended the disposable offline harness to retrieve a stored finding through
  JSON mode, preserve the full scan verdict, inspect the atomic checkpoint bundle,
  and compare all stored file hashes before/after retrieval. JSON mode emits a
  session header plus the report event; the harness validates each JSONL record.
- Resolved normal launch to `C:\Users\sergi\.cargo\bin\davinci.exe`, backed it up
  as `davinci.exe.security-scan-backup-20260907-013659`, and installed the successful
  build without terminating existing sessions. Matching build/installed SHA-256:
  `1C01C1789F09EC630BF265921741B99B7FD698A1164E5C2CFC2674674A1CBAB7`.
- Installed Windows PTY checks passed both the completed no-finding report and
  `/sec-report <scan-id> --finding <finding-id>` displaying the selected synthetic
  finding, classification, severity, location and evidence. Evidence directories:
  `C:\Users\sergi\AppData\Local\Temp\davinci-security-smoke-e5lsl2ka` and
  `C:\Users\sergi\AppData\Local\Temp\davinci-security-smoke-kr85kbmy`.
- Full workspace tests: 2,257 passed, 1 ignored across 31 suites. Workspace
  formatting check and all-target Clippy with warnings denied passed. These
  supersede the earlier uninstalled-source caveat for changes through this pass.
- Production readiness remains unproven: original reconnaissance/supporting-read,
  architecture-aware audit, lifecycle/schema, analyzer, adversarial/evaluation
  and supported-platform requirements still need completion. No live model calls
  or quality measurements were made; this installed release remains experimental.

Resume conflict-identity pass:

- A failing regression demonstrated that checkpoint validation accepted invalid
  conflict stage metadata. Source capture and resume now share stage identity
  validation for normalized paths, stages 1-3, supported Git modes and nonzero
  canonical SHA-1/SHA-256 object IDs. Resume rejects duplicate path/stage records.
- Every retained unresolved conflict must have a nonempty deferred-coverage
  disposition. Removing that row from a checkpoint cannot silently clear coverage.
- Focused security suite: 160 passed across library/binary targets. Coding-agent
  all-target Clippy with warnings denied passed; touched source files formatted.
- These additional source changes follow the installed release recorded above.
  Conflict object bytes still require dedicated immutable retrieval before full
  merge-conflict analysis can be claimed. All other pending acceptance phases
  remain active; this validation pass is not production-readiness evidence.

## Current acceptance audit against original phases

This matrix records outstanding requirements, not reduced acceptance criteria.
None of the nine phases is declared fully complete by this audit. File names are
implementation pointers; passing tests prove only the behavior they exercise.

| Original phase | Current evidence | Missing or insufficient evidence | Next required artifact/check |
| --- | --- | --- | --- |
| 1. Harness integration | Native controller, command parser, settings, print/JSON/RPC and installed Windows TUI smoke | Exhaustive admission/permission and shutdown behavior across supported surfaces | Authority and lifecycle acceptance cases from Phase 1 and Phase 9 |
| 2. Methodology | Five embedded native documents and versioned hash manifest in `skills.rs`; typed repository map in audit results | Complete fourteen-skill migration review remains unproven; remaining phase contracts require audit | Typed phase contracts that match every active methodology output; migration audit |
| 3. Source/reconnaissance | Immutable current/base target and supporting snapshots, Git change capture, policy anchors, conflict identities, resume validation and separate source-bound coordinator mapping | Conflict bytes unavailable; complete exclusion/rename and platform coverage remain unproven; supporting capture uses a bounded second pass | Complete exclusion/rename/conflict evidence and cross-platform acceptance; verify supporting limits and failure recovery at scale |
| 4. Analysis workflow | Isolated provider loop, three snapshot tools, separate coordinator mapping and fresh audits/counterreview, per-audit file reads, assigned-unit accounting, bounded turn allocations and usage records | End-to-end cross-file analysis quality remains unproven; runtime capacity sharing remains separate; candidate reconciliation remains incomplete | Supporting-source fixtures, shared runtime permits and causal reconciliation |
| 5. Verification | Five structured assessment gates, source/read validation, reportable/deferred/suppressed dispositions | Exact serialized-claim dedupe does not establish causal-root dedupe; full occurrence reconciliation, feedback invalidation and runtime-claim contracts remain incomplete | Source-bound causal identities, occurrence-preserving reconciliation, explicit observed-result requirements and complete candidate transition tests |
| 6. Reports/store | Selected finding retrieval, terminal/JSON/SARIF projections, immutable seals, atomic checkpoint bundle and Windows installed smoke | Public v2 envelope is still assembled as JSON values; official offline SARIF-schema validation and complete partial-result recovery are missing | Typed canonical report/lifecycle schemas, official licensed schema fixture/validator, failure-recovery and projection-parity cases |
| 7. Analyzers | Unsupported external execution rejected; default analyzer-free scan works | Required reviewed offline cargo-audit adapter absent | Explicit provisioned executable/database contract, contained fixed-argv adapter and hostile stub acceptance cases |
| 8. Evaluation | Independent causal-identity scorer exists in `davinci-evals/src/security_eval.rs`; synthetic plumbing tests | No held-out paired corpus, evaluator runner, repeated real-model measurements, per-family quality/calibration/latency report or approved recorded cost policy | Original Section 17 corpus and scoring gates, separate labels, recorded model/methodology/provisioning, repeated comparisons |
| 9. Production/platform | Windows release/installed hash and smoke; 2,257 workspace tests, formatting and workspace Clippy passed before subsequent conflict-validation source changes | No complete Linux/macOS acceptance, exhaustive adversarial effects corpus, durable partial-result and lab-containment acceptance | Full original platform/adversarial matrix, refreshed broad checks, final installed build and real quality gates |

Source-bound maps and supporting-source capture now have an integrated offline
cross-file contract test. The next analysis dependencies include causal
reconciliation and shared native runtime capacity. Source exclusions, conflict
bytes, durable partial-result recovery, model-quality and platform gates remain
open. Existing experimental behavior and all original acceptance requirements
remain in scope.

### Source-bound repository map contract

Added `recon.rs` with strict product-source and review-unit schemas. Units retain
entrypoint/decision, actor capabilities, assets, trust boundary, closest control,
sensitive operation, callers/data flow, configuration, immutable anchors,
disposition, rationale and unknowns. Worker packets describe that required output.
Each audit report retains its map and explicitly lists unmapped current/base paths.
Assigned audit identity comes from the coordinator's enclosing audit record.

Validation rejects fabricated/stale anchors, noncanonical side aliases, duplicate
source/unit identities, dangling or inconsistent cross-file links, empty causal
fields and oversized collections. Existing recursive worker read checks apply to
map anchors. Unmapped sources, unknown product surfaces, unresolved assumptions and
deferred units prevent complete coverage; file reads alone are insufficient.

Initial regression tests failed against permissive validation, then passed after
implementation. The interim focused run passed 174 security tests; formatting and
coding-agent all-target Clippy passed. Final cross-reference/read-ledger regression
results: 176 focused security tests passed, final coding-agent all-target Clippy,
workspace formatting check and tracked diff whitespace check passed. This contract does not prove model
analysis quality, complete the coordinator workflow, or change the installed
binary. External smoke fixtures must now return an honest `repositoryMap`; legacy
audit responses without it are rejected rather than silently fabricated.

### Coordinator mapping and independent audit assignments

`review.rs` now enters Mapping for Standard/Deep before Investigating. The same
restricted worker runner produces schema-validated, source/read-validated maps;
only the typed map enters the corresponding fresh audit context. Deep builds two
maps without sharing mapping transcripts or the other map/audit results. Quick
retains combined reconnaissance/investigation. Each report audit retains the
coordinator map separately from the resulting audit map.

Mapping receives one third of each audit's existing discovery turn/token
allocation. Investigation receives the remainder; the mode totals and validation
reserve are unchanged. Assignment IDs and exact source anchors must survive audit
reconciliation. Missing, renamed, deferred or source-replaced assigned units stay
in `unreviewedMappedUnits` and force partial coverage. Terminal output now includes
unmapped current/base sources, unreviewed assignments and map uncertainty counts.

The initial stage-order regression failed against the prior implementation, then
passed. Tests now cover Quick/Standard/Deep stage order, fresh contexts, no mapping
transcript inheritance, independent audit source retrieval, reserve preservation,
assignment identity checks, original-snapshot resume and terminal map coverage.
The unread-audit regression observed the full-file read rejection before the
more specific citation rejection; its expected diagnostic was corrected to match
that authoritative failure. Final focused run: 182 security tests passed. Broad
workspace run: 2,281 passed, one ignored across 31 suites. Coding-agent all-target
Clippy, workspace formatting check and tracked diff whitespace check passed.

This change is not installed yet. Supporting-source capture/authorization, full
cross-file quality evidence, causal reconciliation, durable partial-result
recovery, analyzer adapter, held-out repeated model evaluation and platform gates
remain required. No production-ready claim is made.

### Supporting-source capture and target boundaries

`supporting.rs` captures eligible same-repository evidence after target capture,
using the remaining byte/inventory limits. Snapshot supporting current/base maps
are distinct from target maps and included in snapshot identity and private
checkpoint checksums. Fixed Git object reads preserve selected revision sides;
changed scans retain worktree plus baseline helper bytes. Source tools and worker
inventory label target/supporting scope. Root-to-leaf policy resolution can now
use captured supporting policy documents.

`securityScan.supportingReads` defaults true; false confines reads to targets,
merges monotonically through trusted project policy and rejects wider checkpoints
on resume. Model output cannot expand target classification. Reportable records
derive scope from cited control anchors (or conservatively from all anchors when
no control is cited); supporting findings remain visible with outside-target
labels and do not broaden the target exit policy. Map completeness counts target
files, while relevant supporting evidence can participate in cross-file units.

Initial scoped-helper regression failed before implementation. Focused checks
cover hard-denied credential paths, immutable helper reads after worktree edits,
source-tool scope labels, private checkpoint reload, duplicate target/supporting
identity rejection, policy restriction merging, shared capture limits, committed
diff helpers versus dirty worktree helpers and target-only blocking policy. An
offline Standard audit retrieves both entrypoint and helper in fresh mapping/audit
contexts and completes with one target file plus one supporting file. This proves
the cross-file transport/coverage contract, not vulnerability detection accuracy.

Supporting capture currently performs a bounded second inventory/read pass;
target files are prioritized and repeated target reads in that pass can reduce
available supporting capture under tight limits. Supporting exclusions and budget
gaps remain explicit. Generated-directory exceptions and complete conflict-stage
content access are not implemented by this change. No live providers or analyzers
were called, and the installed executable has not been replaced.

Final validation for this milestone: workspace tests passed (2,293 passed, one
ignored across 31 suites); coding-agent all-target Clippy, workspace formatting
check and tracked diff whitespace check passed. Installed CLI/TUI smoke and the
remaining original production acceptance gates have not been claimed here.

### Mapping uncertainty reconciliation

Verified and fixed a coverage gap: a fresh audit could omit coordinator environment
assumptions or unit unknowns and thereby claim complete coverage. Repository maps
now accept typed `resolvedAssumptions` with exact assumption text, optional unit
identity, rationale and immutable source locations. Resolution evidence is bounded,
hash/side/range validated and subject to the worker's independent read ledger.
Unresolved coordinator assumptions are retained in per-audit coverage and terminal
output, and prevent complete coverage even if omitted from the returned audit map.

The omission regression failed against the permissive implementation and passed
after the change. Final focused security run: 196 tests passed. Coding-agent
all-target Clippy, formatting and tracked whitespace checks passed. This remains
structural proof of uncertainty accounting, not proof that a model's resolution
rationale is causally correct. Broader causal reconciliation and the other original
production gates remain open; no new installed build is claimed.

### Shared in-process completion admission

The old scan-only atomic request counter has been replaced with
`davinci-agent::runtime::capacity::REQUEST_CAPACITY`. Ordinary Agent completion
callbacks and restricted scan completion callbacks now share four in-process
slots. Scans can occupy at most two, leaving two available for foreground
completion requests. Cancellation is checked before admission and every 20 ms
while queued. RAII releases permits on success, errors and panic unwinding;
ordinary completions release before retry backoff and tool execution.

The three initial admission regressions failed before implementation. Five pool
tests now cover cancellation, both concurrency limits, foreground reservation,
queued wake-up and panic release. Agent tests passed (311); focused security
tests passed (196 executions across two suites). Coding-agent all-target Clippy,
formatting and tracked diff whitespace checks passed.

This pool is process-local and covers these two transport callback routes. It
does not yet provide shared accounting for graph/subagent child processes or
separate CLI processes, and therefore does not complete the original global
runtime-capacity gate. Status and cancellation do not acquire request permits.
The installed executable has not been replaced by this milestone.

Final workspace regression run after this change: 2,300 passed, one ignored
across 31 suites (`cargo test --workspace --offline --locked`).

### Resume source-policy and seal boundaries

Reproduced and fixed two recovery admission defects. Resume previously checked
file and inventory limits only against target maps, omitting supporting sources.
`ScanConfig::validate_snapshot` now checks all four source maps, supporting-read
authorization, total recorded bytes, total source inventory, per-file limits and
the separate SECURITY.md byte limit. Both resume admission and review execution
invoke it before any worker request.

A sealed partial report previously remained eligible for provider work under the
same scan ID. Resume now rejects every sealed report, irrespective of coverage,
after checking seal integrity. The regression observed one unauthorized resume
callback before the fix and zero afterward. Interrupted unsealed runs still pass
the existing original-snapshot resume test.

Final targeted validation: 202 security test executions passed across two suites;
coding-agent all-target Clippy, formatting and tracked diff whitespace checks
passed. These changes are not installed. Full checkpoint policy/methodology
binding, persisted remaining budgets and reusable partial worker results remain
required; these admission fixes do not establish complete durable recovery.

### Checkpoint configuration and methodology binding

New atomic checkpoint bundles include hashes of the validated effective ScanConfig
and embedded methodology manifest. Resume verifies both before reserving a new
generation or invoking a worker. The comparison is deliberately exact, including
resource/report settings; changing settings requires a new scan. Legacy snapshots
remain loadable for inspection, but missing bindings cannot authorize analysis.
Absent binding fields are omitted during serialization so existing bundle
checksum verification remains compatible.

The changed-policy regression failed before enforcement. The final focused run
passed 204 security test executions, including changed/missing methodology,
missing binding, legacy bundle integrity and original-snapshot resume cases.
Coding-agent all-target Clippy, formatting and tracked whitespace checks passed.
The installed executable is unchanged. This binds policy and embedded methodology;
it does not yet persist remaining budget, cache completed worker results, or prove
the full recovery or production acceptance gates.

### Durable aggregate request budgets

Added `budget.rs` with bounded, immutable per-request reservations and usage
settlements in the private Store. Review binds the original mode's aggregate
token/turn limits; workers publish reservations before calling the provider and
settle against the existing validated usage accounting afterward. An interrupted
or panicking callback without a settlement retains the reservation. Reopening
recounts all requests, rejects malformed/gapped/cross-run/policy-mismatched rows,
and preserves measured overages so subsequent requests cannot reset the limit.
Final reports include cumulative tokens, requests, remaining allowances and
unsettled request counts. A checkpoint binding version rejects pre-journal runs.

The interruption regression initially reopened at zero and failed. Implementing
the journal exposed a retained Store lease after run completion; finalization
now releases the budget's Store before announcing terminal status. The existing
original-snapshot resume regression passes again. New tests cover settlement
refunds, persisted overages, repeated settlement, malformed records, gaps,
identity/limit mismatch and failed publication refusing admission.

Final focused validation: 212 security test executions passed across two suites;
coding-agent all-target Clippy, formatting and tracked whitespace checks passed.
No installed build or live provider call was made. Aggregate request accounting
is now durable; phase-specific validation reserve and elapsed deadline accounting
across resumes, reusable completed worker artifacts, and partial-result reporting
remain required. This milestone does not establish full recovery acceptance.

### Validation reserve across resume

Durable reservation and settlement records now include a host-selected discovery
phase and the effective discovery token limit. Mapping/investigation requests
share the original discovery allocation across generations; admission protects
the validation token reserve and half of total model turns. Settlements must
retain their request's phase and policy. Measured usage adjusts token charges,
but never refunds a model turn. Reports include cumulative discovery usage and
remaining discovery allowances. Checkpoint budget binding version 2 rejects
older journals without phase accounting.

The resumed discovery-overrun regression failed before enforcement. Focused tests
passed (216 executions across two suites), including token reserve, turn reserve,
settlement after resume and changed discovery-policy rejection. Coding-agent
all-target Clippy, formatting and tracked diff whitespace checks passed. Elapsed
deadline persistence, reusable worker results and partial-result reporting remain
open, as do the other original production gates. No installed build is claimed.

Workspace regression after the combined resume/accounting changes: 2,320 passed,
one ignored across 31 suites (`cargo test --workspace --offline --locked`).

### Wall-clock deadline persistence

Added `deadline.rs`: before snapshotting, a new scan atomically publishes its
start, expiry, original time allowance and scan identity. Resume loads the same
record and installs only the remaining duration in the existing monotonic
cancellation mechanism. Storage elapsed time is deducted before installation.
Paused time counts against the original wall-clock ceiling. Missing/corrupt
records, changed allowance, identity mismatch, expiry and clocks preceding the
recorded start are rejected before worker execution. Published deadlines are
never overwritten. This uses the local system clock across process restarts.

The deterministic resume test first reproduced a fresh 300-second allowance
instead of the expected 100 seconds. Final focused validation passed 220 security
test executions across two suites, including deadline expiry/boundaries, missing
and corrupt artifacts, no-store resume and the existing original-snapshot resume
flow. Coding-agent all-target Clippy, formatting and tracked whitespace checks
passed. No live provider call or installed build was made. Reusable completed
worker evidence, durable partial-result reporting and other original production
gates remain unfinished; deadline persistence is not full recovery acceptance.

### Reusable completed worker evidence

Added bounded immutable worker recovery artifacts containing the returned JSON
and independently accumulated source-read ledger. Input keys include scan
identity, task assignment, immutable snapshot/source hashes, methodology/tool
schemas, runner provenance and request limits. Existing resume policy binding
is checked before this path. Recovery verifies artifact digest and identity,
rechecks the read ledger, and reruns the phase's typed validation. Only results
accepted by that validation are published. Corrupt artifacts fail explicitly;
they do not silently become cache misses. Mapping, audits and counterreviews use
this path; distinct audit assignments have distinct keys. Prior requests retain
their durable charges, and incomplete worker calls are not cached as completed.

The artifact-reopen regression first failed without loading. An end-to-end offline
interruption after mapping now resumes by reusing the completed map, asserts no
mapping provider call occurs on resume, independently reads source for the audit,
and completes with five cumulative requests across both generations. Additional
tests cover corrupt digests, invalid storage keys and assignment/provider/limit
separation. Final focused validation: 228 security test executions passed across
two suites; coding-agent all-target Clippy, formatting and tracked whitespace
checks passed. A dead-code Clippy failure was fixed by retaining the existing
nonpersistent runner path when no Store is configured.

The installed executable is unchanged. This is completed-worker reuse, not
durable presentation of every partial candidate or restoration of an interrupted
worker transcript. Partial-result reporting, remaining causal reconciliation,
analyzers, quality evaluation and platform gates remain unfinished.

### Durable partial report presentation

Review now publishes incomplete checkpoints after snapshot binding, completed
mapping, audits and each candidate disposition. Same-session retrieval overlays
the current lifecycle status; reopening by scan ID reads the latest bounded,
identity-checked and digest-checked artifact. Pending claims remain deferred and
coverage remains false. Final seals take precedence. Corrupt latest partial
artifacts fail explicitly instead of silently selecting older evidence.

The interrupted-map regression failed before publication wiring and passed after
it, including cross-session retrieval and successful resume without new mapping
requests. The focused security suite then passed 230 test executions. Additional
cancellation/pending-claim and storage validation changes passed four targeted
partial-report test executions across two suites. The interrupted broader test
command's result was unavailable and is not claimed. All-target coding-agent
Clippy, formatting and diff whitespace checks passed. No workspace rerun, live
provider request or installed build was needed for this checkpoint. Full typed
report validation, causal reconciliation, analyzer and quality/platform gates
remain open; this does not establish production readiness.

### SARIF evidence identity projection

SARIF now retains each source location's snapshot side, content hash and evidence
role in namespaced properties, and projects the existing versioned claim
fingerprint as a partial fingerprint. The first evidence anchor is the primary
location; remaining anchors retain numbered related locations. This prevents
baseline/head evidence from losing its identity in export. It does not infer an
execution order or invent code flows from an unordered anchor list.

The multi-location regression failed before the change and passed afterward:
two targeted SARIF test executions across two suites. The affected crate's
all-target Clippy passed. The official OASIS SARIF 2.1.0 schema was inspected as
reference; full offline schema validation and canonical typed report validation
remain outstanding. No broad test suite or live provider call was run.

### Offline official SARIF schema gate

Vendored the unmodified OASIS SARIF 2.1.0 schema at upstream commit
ed71d4f62db866ce3698a08a5ec3f7f2e775545d with its license notice, provenance
and SHA-256. Added a pinned test-only Python validator and an explicit ignored
inline Rust test invoking the real exporter. A dedicated CI workflow provisions
the validator and explicitly runs this gate; ordinary Rust tests need no Python
package installation. Remote schema retrieval is disabled during validation.

Local validation passed one targeted Rust test covering 24 real exports across
confirmed/likely, selected/whole/empty reports and four lifecycle states. Negative
sentinels reject wrong versions, invalid regions and unescaped URI spaces. The
initial command failed because shell expansion removed the test environment
variable; rerunning with literal shell quoting passed. Coding-agent all-target
Clippy passed. CI execution itself is unverified. This closes the missing offline
official-schema test mechanism, not every consumer-specific constraint, canonical
report validation, analysis quality or remaining production acceptance gate.

### Canonical report closure boundary

Added host-side closure validation before review returns, before any final
artifact is published, and when a sealed report is reopened. Required candidate,
finding, lead and limitation arrays must exist. Candidate IDs must be unique
UUIDs; dispositions must be recognized and duplicate references must resolve to
an earlier candidate. Findings and leads must exactly match their corresponding
candidate projections. Complete reports cannot contain deferred candidates,
limitations, partial status, skipped files, incomplete audit summaries or
duplicate/mismatched reviewed-path counts.

The storage regression first demonstrated an incomplete canonical object could
be sealed as complete. After enforcement, 119 focused library security tests
passed (one explicit external-validator test ignored), followed by two targeted
closure tests after adding duplicate/projection adversarial cases. Existing
storage fixtures now carry the required closure fields. This is a deterministic
closure gate, not yet the complete typed report schema or a substitute for
source-backed evidence validation. Remaining production gates are unchanged.

### Typed saved assessment validation

Saved reportable, suppressed and not-applicable candidates now deserialize into
the existing strict Claim and Assessment types. Publication/recovery reuse the
same claim-shape and assessment semantics as fresh worker validation. Extracted
these checks from source lookup without removing snapshot/hash/read-range checks
from live validation. Missing or wrong gate arrays, unknown classifications,
confirmation with proof gaps, missing rationale/remediation and suppression
without counterevidence are rejected at the saved report boundary.

An unsupported confirmed record first reproduced acceptance and now fails. The
focused library security tests passed 121 tests with the explicit Python gate
ignored; one targeted confirmation test then passed with additional mutations.
This validates saved assessment structure and classification rules, not renewed
source-hash verification on archive reopen or the full typed top-level schema.
Those and the other original production acceptance gates remain open.

### Unambiguous finding identities

Canonical reports now require canonical non-nil candidate UUIDs and unique
canonical finding/occurrence UUIDs for reportable records. Version-1 fingerprints
must retain a lowercase 64-character hexadecimal value. This checks the declared
identity format, not causal-root deduplication or recomputation after redaction.

The duplicate-finding-ID regression reproduced acceptance before enforcement.
Four targeted report-contract tests now pass, including missing/nil/noncanonical
IDs and malformed/unknown-version fingerprint cases. The stored-report finding
selection integration test also passed. No broad suite was rerun; complete
canonical schema, source revalidation and other production gates remain open.

### Archived candidate source revalidation

Final publication and sealed/partial report recovery now revalidate candidate
claims against the immutable saved checkpoint whenever candidates exist. The
report must identify that checkpoint; every claim anchor is checked for source
side, path, hash and line range. Full assessments rerun their source-backed gate
and counterevidence validation. The current worktree is never substituted.
Checkpoint loading retains existing bounded reads, confinement and digest checks.

An adversarial regression changed the report's claim hash and updated the report
seal; recovery previously accepted it and now rejects the source mismatch.
The valid stored-finding selection test passes. After extending the boundary to
publication and partial recovery, 123 focused library security tests passed with
the explicit external-validator test ignored. Zero-candidate coverage/source
inventory reconciliation, full report schema and other original gates remain
open. These consistency checks do not authenticate against an owner capable of
replacing all source, report and seal artifacts together.

### Snapshot-bound coverage, including empty findings

Removed the zero-candidate bypass from archive validation. Every final publication
and final/partial recovery now requires its saved checkpoint. The native in-memory
review path also checks inventory binding. Current and baseline counts, skipped
records and reviewed paths must match that snapshot. Every audit's reviewed and
deferred paths form an exact disjoint partition of captured source; complete
audits cannot defer source. Aggregate reviewed paths must equal the audit union.

The zero-finding omission regression first reproduced acceptance. It now rejects
wrong inventory, invented paths and audit omissions. Updated storage fixtures to
carry actual checkpoint/coverage bindings. Final focused library validation passed
124 tests, with the external Python gate explicitly ignored. No workspace suite
or provider calls were run. Typed mapping/assumption recomputation on archive
reopen, complete top-level schema and other production acceptance remain open.

### Reconstructed mapping and mode coverage

AuditCoverage now has a strict deserialization contract. Archive validation checks
the saved maps' source anchors, reconstructs coverage using the live calculation,
and compares the derived mapped/unmapped paths, deferred units, unresolved
coordinator assumptions and completion flag. Fresh review and recovery share the
coordinator-assignment calculation. Audit IDs must be ordered, and a complete
report must retain the requested mode's audit count (two for Deep, one otherwise).

The unresolved-environment-assumption regression reproduced false completion
before the recovery gate was connected. The focused library security suite then
passed 125 tests with the explicit Python schema gate ignored. A targeted Deep
audit-count test passed after adding mode binding. No broad workspace rerun or
provider request was made. Full top-level report schema and the original
reconciliation, analyzer, quality and release/platform gates remain unfinished.

### Static validation provenance gate

Final report validation now requires runtimeTestsExecuted=false. Successful
counterreviews require the exact native static-validation provenance; any
provenance supplied on other candidate dispositions is checked too. Missing,
altered or extra provenance fields fail publication/recovery validation. This
does not authenticate an archive against an owner replacing all its artifacts.

The new regression first reproduced acceptance of an invented runtime-test
claim. After the fix, 127 focused security library tests passed, with the explicit
Python SARIF gate ignored. Formatting and diff whitespace checks passed. The
complete typed report envelope and remaining production gates are still open.

### Strict final report envelope and source metadata

`report_envelope.rs` now defines strict serialization/deserialization contracts
for every final report field, including candidate variants, coverage audits,
source metadata, methodology, provider/injected-runner provenance, budgets,
usage, checks and failure policy. Round-trip equality rejects omitted nullable
or defaulted fields as well as unknown fields. Final reports cannot claim an
unsupported schema, generation, release status, hardening result or analyzer
check. Budget balances and runner quality labels are validated. Candidate
assessment semantics now also cover completed counterreviews that defer a lead.

Snapshot validation additionally binds revisions, conflict identities,
supporting inventories and skipped supporting files to the checkpoint. Finding
scope is recomputed from captured control evidence, preserving target/supporting
failure-policy behavior on recovery. Partial checkpoints retain their separate
wire contract.

Regressions first demonstrated acceptance of missing budgets, inconsistent
remaining-token totals, extra candidate fields and altered scope. The focused
security library suite passed 131 tests, with the explicit Python SARIF gate
ignored. Affected-crate all-target Clippy passed after boxing both candidate
variants; formatting and whitespace checks passed. No provider calls or broad
workspace reruns were made. This closes the structural final-envelope gap, not
the remaining causal reconciliation, artifact-index, analyzer, quality-evaluation
or release/platform acceptance requirements. The installed binary remains stale.

### Complete committed evidence index

Version-3 seals now include a sorted index of all committed evidence files in
the run directory, including checkpoint bundles, worker results, accounting,
generation markers and partial checkpoints. Report exports retain their three
explicit projection hashes. Recovery reconstructs and compares the evidence
index, detecting modified, deleted and newly added committed files. Current
seal, active lock and canonical unpublished temporary files are excluded from
the evidence pass. Prior experimental seals lacking the index fail recovery;
the user documentation records this compatibility boundary. Hashes still do
not authenticate against an actor able to replace the entire store and seal.

Hashing uses a 64 KiB buffer, per-file and aggregate byte caps, confined file
opens and a bounded inventory. Serialized seals have the same 8 MiB cap on
publication and recovery. A regression first reproduced acceptance of changed
worker evidence. The focused security suite then passed 132 tests with the
explicit Python SARIF gate ignored. Subsequent targeted tests passed for added,
changed and missing evidence, deterministic ordering, temporary-file exclusion,
exact byte limits and directory rejection. Affected-crate all-target Clippy
passed. No provider requests or broad workspace reruns were made. Full platform
recovery acceptance, reconciliation, analyzer integration and model-quality
evaluation remain unfinished; the installed executable has not been refreshed.

### Duplicate integrity and conservative contradiction detection

Final report validation now rejects duplicate chains and duplicate links that
change the causal claim or affected occurrence. All candidate claim shapes,
including deferred and duplicate records, are validated. A regression first
demonstrated that a changed entry point could disappear behind a duplicate link.

`reconciliation.rs` detects opposing reportable versus suppressed/not-applicable
assessments for matching actor, entry point, control, sink, impact, prerequisites
and source anchors. It ignores titles and proof-gap text, and normalizes ordering
and repetition in prerequisite/evidence arrays. Opposing assessments are retained,
not voted away. The live coordinator records their IDs as an unresolved limitation;
the publication/recovery gate rejects complete coverage or a hidden conflict.

The focused security library suite passed 136 tests with the explicit Python
SARIF gate ignored. Subsequent targeted tests passed for ordering normalization
and an offline full worker-pipeline conflict, including retained dispositions and
exit code 2. The latter uses explicitly synthetic usage to isolate reconciliation
from conservative unknown-usage reservations. All-target affected-crate Clippy
and formatting passed. This is not semantic root-cause reconciliation: differently
worded claims, canonical root-control/invariant fingerprints, occurrence grouping,
targeted conflict resolution and feedback invalidation remain open, along with
the analyzer, quality-evaluation and release/platform gates.

### Evidence-bound targeted conflict resolution

Detected conflicting causal packets now receive a fresh restricted reconciliation
worker. The typed response must assess every supplied candidate exactly once,
cannot refer to another candidate, and must satisfy the normal five-gate and
snapshot checks. The existing read ledger requires fresh source retrieval before
accepting cited evidence. Candidate identity and claims remain unchanged; prior
assessments are retained under `priorAssessment`, with explicit reconciliation
provenance. Final findings are rebuilt from the resulting dispositions. Missing
or invalid responses, budget exhaustion and remaining disagreement stay partial.
Archive recovery validates both current and prior source-bound assessments.

`validation_budget.rs` allocates unused discovery tokens to validation and reserves
one third of the validation capacity for reconciliation when multiple candidates
exist. Its immutable run-bound record fixes allocations across restart so cache
keys do not drift as requests consume the budget. The global request ledger still
enforces cumulative limits. Budget binding version 3 rejects older incompatible
resume state. Reconciliation objectives use stable candidate indexes and claims,
not regenerated display UUIDs, and use the existing validated worker-result cache.

The fresh-resolution pipeline regression failed before integration. The focused
security suite subsequently passed 139 tests, with the explicit Python SARIF
gate ignored. Three targeted pipeline tests then passed for unresolved conflict,
successful source-backed resolution and rejection of unread evidence; each also
reopened the sealed archive. A response-contract test rejects omitted/foreign/
duplicate candidates and wrong source hashes. The allocation test confirms stable
limits after prior token consumption. All-target affected-crate Clippy passed.
These are synthetic offline tests, not model-quality measurements or full crash
acceptance. Semantic discovery of differently worded equivalent claims, canonical
root fingerprints, occurrence grouping, feedback invalidation, analyzers and
quality/release/platform gates remain unfinished.

### Stable record identity across interrupted validation

The coordinator previously generated fresh UUIDs while replaying cached claims
and counterreviews. `identity.rs` now derives domain-separated custom UUIDs from
the scan ID, candidate position, immutable claim digest and record kind. Candidate,
finding and occurrence IDs remain distinct and stable across generations of that
scan. This is record identity, not the planned semantic root-cause fingerprint.
Checkpoint bindings now include `recordIdentityVersion: 1`; older checkpoints
remain readable but cannot resume with silently changed identity semantics.

A regression reproduced the old random-ID behavior. Three targeted tests passed:
deterministic identity and separation, checkpoint compatibility rejection, and an
offline pipeline interrupted after its first counterreview then resumed from
cached work through reconciliation and archive reopening. The latter preserves
all three IDs and the original claim in generation 2. Library Clippy passed with
warnings denied. No provider calls, broad workspace tests or executable install
were performed. Semantic root identity/grouping, feedback invalidation, analyzer
integration, model-quality evaluation and release/platform acceptance remain open.

### Source-bound root identity and version-2 fingerprints

`root_cause.rs` adds the required counterreview root-control, violated-invariant,
normalized sink/decision and attack-path fields. Its control anchor must be covered
by the control-semantics gate; the decision must be covered by reviewed gate evidence.
Normal snapshot/hash/range checks and worker read-ledger validation also apply to
these anchors. Semantic labels are bounded and must survive credential/terminal
sanitization unchanged. Reportable assessments without root identity are rejected.

The coordinator derives version-2 fingerprints from root semantic labels and paths,
excluding presentation, line positions and source hashes. Reconciliation refreshes
the fingerprint if its assessment changes. Publication/recovery recompute the root
fingerprint, and SARIF uses `davinci.root/v2`. Current schema validation rejects
older experimental reportable records without this identity. Updated methodology
bytes invalidate incompatible resume/cache bindings. Record UUIDs remain distinct
from root fingerprints and retain their prior generation-stable derivation.

The missing-root regression failed before implementation. The focused security
suite then passed 146 tests, with the explicit Python SARIF gate ignored. Coverage
includes all semantic fingerprint inputs, line/hash movement, unreviewed anchors,
wrong source side/hash/range, unsafe labels, normal reconciliation and resumed
archive recovery. All-target affected-crate Clippy passed. This establishes the
typed identity contract; it does not yet infer equivalence between different
semantic wording, group occurrences into canonical findings, or implement feedback
invalidation. Analyzer, quality-evaluation and release/platform gates remain open.

Follow-up checks passed for a changed root paired with a stale fingerprint and
a syntactically valid but incorrect hash. The explicit offline official SARIF
schema gate also passed with the version-2 renderer fixture (24 export packets
and three invalid sentinels). No model requests or executable install occurred.

### Canonical findings retain all affected occurrences

`grouping.rs` projects matching versioned root fingerprints within each scope into
one canonical finding. `candidateIds` retain all reportable and exact-duplicate
claim links; `occurrences` preserve complete assessed candidate records. Canonical
finding IDs derive from scan, fingerprint version/value and scope; candidate and
occurrence IDs remain independently stable. Reconciliation refreshes root IDs when
the root changes. Partial and final reports use the same grouped projection.

The summary names its `representativeCandidateId` and copies one actual assessment:
confirmed before likely, then severity, then discovery order. It does not aggregate
votes or replace another occurrence's classification. Failure policy checks every
occurrence. Strict final-report validation reconstructs the complete projection,
rejecting missing candidate links, dropped/changed occurrences, inconsistent group
identity and altered summaries. Finding selection retains the complete group.
Terminal and TUI share the full occurrence formatter, including locations, caller,
prerequisites, gaps and remediation. SARIF carries all occurrence metadata and
related locations in one result per canonical finding.

The grouping regression failed before implementation. After schema-fixture updates,
the focused suite passed 147 tests. Additional tests cover separated roots/scopes,
mixed confirmed/likely policy thresholds, omitted or altered occurrence evidence,
selection and exports. An offline full pipeline groups two distinct callers, keeps
two occurrence IDs, makes eight synthetic fixture requests and reopens the exact
sealed report. The TUI adapter test retains both assessments, locations and caveats.
The official offline SARIF gate passed 24 grouped export packets and its three
invalid sentinels.

A subsequent full focused run exposed Quick-mode budget exhaustion: every worker
received all five methodology phases, including unrelated instructions. Bundle
`native-security-2` now selects relevant phases by coordinator-owned role while
retaining shared restrictions; unknown roles conservatively retain all phases.
This changes methodology/cache bindings, not token accounting or budget ceilings.
The mode regression and restriction-selection test passed, followed by 150 focused
security tests (one explicit SARIF gate ignored). All-target Clippy was rerun after
the grouping fixes. Differently worded root-label reconciliation, feedback
invalidation, analyzers, real model quality and release/platform acceptance remain
unfinished. The installed executable has not been refreshed.

### Separate immutable merge-stage evidence

`conflict_capture.rs` now captures eligible regular Git index blobs with their
stage/object identity and content hash. Snapshot tools expose `index-base`,
`index-ours` and `index-theirs` independently of worktree/base/head. Inventory
pagination, literal search, exact source reads, source-bound mapping and the worker
read ledger preserve those sides. Target and supporting stage evidence retain
their scope; disabling supporting reads withholds outside-target stages and rejects
resume of a broader checkpoint. Metadata is paged with inventory rather than
putting the complete conflict list into every worker prompt.

Capture applies credential/generated-path denial before Git blob reads, rejects
non-regular entries, and enforces file/policy, aggregate-byte and source-inventory
limits. Binary, unsupported-encoding and private-key-shaped bodies are withheld
with stage-specific reasons. Repeated inventory does not duplicate bytes; changed
index identities between capture phases fail explicitly. Changed scans retain
deleted conflict paths even when other conflicted worktree files still exist.
Baseline reads also respect the shared source count after stage capture.

Private checkpoint recovery checks each stage source against its regular index
identity, unique sorted ordering, content hash, deferred coverage and byte ledger.
Capture/merge validation uses indexed identity sets, and stage lookups use binary
search. Reports never claim complete coverage for unresolved index conflicts;
standalone final-envelope validation also rejects invalid or duplicate stage IDs.
Reconnaissance does not treat reading all merge stages as resolving the merge.
Changed methodology bytes and tool schemas invalidate incompatible worker caches.

The missing-stage-read regression and standalone complete-report regression both
failed before their fixes. The focused scanner suite passed 155 tests (one separate
SARIF gate ignored), including Quick-mode budget behavior. After lookup/recovery
refinements, six targeted checks passed: three conflict capture/report tests, the
two-file merge fixture with deleted worktree source and scoped supporting capture,
checkpoint corruption/order validation, and canonical-side mapping. The merge
fixture reopens the exact private snapshot and verifies stage-aware tool inventory.
All-target affected-crate Clippy, formatting and scoped diff checks passed. These
are offline synthetic/local-Git checks, not live model or full crash/platform proof.

The original production gates remain open: semantic root reconciliation, feedback
invalidation, offline analyzer integration, held-out model-quality evaluation,
large-inventory/recovery acceptance, shared capacity across processes and release
acceptance on supported platforms. No executable install or provider call occurred.

### Checkpoint encoding shares the recovery limit

A regression with 6,000 excluded paths and a small source-byte policy reproduced
publication of a checkpoint that exceeded its own resume read limit. Source bytes
alone did not account for inventory metadata. `checkpoint_encoding.rs` now stops
JSON encoding at the effective byte limit before immutable publication. Both the
checkpoint payload and the surrounding scan-identity/checksum bundle are bounded.
Creation, resume, archive validation and evidence hashing share the same limit
calculation, capped by the archive reader's existing maximum. Serialization failure
leaves the publication slot available; no worker request has started at this point.

Nine focused checkpoint tests passed, covering escaped JSON and exact boundaries,
metadata overflow, bundle-only overflow, successful near-limit reopen/resume,
corruption and legacy recovery. The cached-validation resume pipeline and sealed
evidence tampering test also passed. Affected-crate all-target Clippy passed.
No broad suite, live provider call, dependency install or binary install was run.
This fixes an archive admission bug; the remaining original production/evaluation
gates listed above are still open.

### Confirmed-only scoring cannot masquerade as release acceptance

The evaluation helper previously emitted `releaseGatePassed: true` for a single
perfect canned finding. Its output now uses `confirmedThresholdsMet`, reports
confirmed result count, duplicate rate and coverage separately, and explicitly
documents confirmed-only recall. There are no production callers of the old field.
This metric helper has no authority to certify a held-out corpus, actual provider
execution, severity calibration, cross-platform safety or production readiness.

A second regression showed that an invalid duplicate's evidence was ignored and
that reversing valid/invalid results changed recall. Every confirmed result now
contributes to the denominator; invalid evidence is counted even on duplicates,
and a valid occurrence remains detected regardless of result order. Redundant
confirmed reports count as false positives. The descriptive duplicate threshold
uses the planned 5% rate; missing/unresolved positives remain misses and empty
denominators remain undefined.

Both regressions failed before the fix. Six targeted `davinci-evals` security tests
passed, including duplicate order, the 5% boundary and zero denominators. All-target
Clippy for that crate, formatting and scoped diff checks passed. This is corrected
scoring plumbing, not the missing held-out corpus/evaluator or measured model
quality. The original production goal remains unfinished.

### Corpus validator, held-out package, and cargo-audit adapter

Portable source paths now reject Windows reserved characters `<>:"|?*`,
backslashes, drive/ADS colon forms, over-long (255) components, reserved device
stems, `.` / `..` / `.git`, and trailing space/dot. Eight `security_corpus_*`
tests passed, including insufficient cross-file evidence, duplicate IDs,
missing/extra cases, invalid ground truth/anchors, unknown JSON fields, and
case-fold collisions. Worker projection still omits evaluator metadata; a
regression copies a label into source bytes and asserts it **does** appear in
the worker map. The synthetic contract fixture is not a real corpus.

The held-out corpus `held-out-security-2026-09` has 40 pairs / 80 cases, ten
families with four pairs each, multi-file evidence on every vulnerable case,
and labels outside staged worker files. Five representative cases are compiled
and executed in-process (owner check, final-path check, argv-only subprocess,
unused affected renderer, approval-gated tools). The independent runner stages
only source files and scores with the shipped confirmed-only scorer. Empty
findings yield recall 0, undefined precision, `confirmedThresholdsMet=false`,
and `blocking: "unverifiable"`. No live provider run was authorized.

`analyzers.rs` is a host-owned cargo-audit 0.22 adapter (MIT OR Apache-2.0
rechecked). Default scans keep analyzers disabled. Enabling `cargo-audit`
requires `DAVINCI_SECURITY_CARGO_AUDIT` and `DAVINCI_SECURITY_ADVISORY_DB`;
argv is fixed (`audit --json --no-fetch --no-yanked --color never --quiet --db
--file`); `.bat`/`.cmd` and repository-tree executables are denied; env is
cleared with `CARGO_NET_OFFLINE=true`; missing/stale db, timeout, and invalid
JSON are limitations, never success. Advisory JSON cannot carry exploitability
into the report check. The seven original analyzer tests plus the disabled-check
test passed against rustc stubs. This host has no provisioned cargo-audit
binary or advisory-db.

`davinci-evals` and `davinci-coding-agent` all-target Clippy (`-D warnings`)
and formatting passed for this milestone. Additional shipped-path named tests
passed: JSON has no terminal prefix; a legacy `validated` flag is not v2
confirmation; `/security-scan` is discoverable; memory indexing cannot be
enabled; missing actor/reachability, sanitizer-name suppression, test-only
runtime “counterevidence”, and unobserved runtime claims are rejected.

No workspace release suite, no installed-binary replacement, no network/provider
call. Current `target/debug/davinci.exe` reports `1.0.0`; that is not scan
acceptance.

### Current-build CLI/JSON/RPC fixture launches and remaining engineering tests

`target/debug/davinci.exe` was driven twice with `--offline` and
`PI_SECURITY_SCAN_FIXTURE` (absolute JSON of worker replies). Print `--format
json` and `--mode json` both produced schemaVersion 2, coverageComplete true,
empty findings, `runner: offline-fixture`, exit 0. JSON mode wrapped
`{"type":"security_scan_report","report":...}` with no terminal prefix. An
empty fixture produced status failed, coverageComplete false, exit 2. RPC
`/security-scan` returned immediately (`status: preflight`); later
`/sec-status` and `/sec-report` prompts observed snapshotting then a complete
v2 report without blocking the input loop. TUI/PTY was not launched.

Shipped-path tests added/passed: fingerprint+entrypoint reconciliation of
differently worded claims; grouping of same-root wording; feedback expired
after control hash change (wired into review); unauthorized start is rejected
before a run exists; scan slots share in-process `REQUEST_CAPACITY` (max two
scan permits). Clippy `-D warnings` passed for `davinci-coding-agent` and
`davinci-agent`.

Remaining: fourteen-skill rights audit, cross-process capacity, Unix
descendant cancel, lab containment, Linux/macOS, provisioned cargo-audit+db,
measured model quality, workspace release suite, live TUI. These stay
`blocking: "unverifiable"` or incomplete. Not production-ready.

### Fourteen-skill source-rights, legacy v1, and remaining named shipped-path tests

Native methodology stays native-authored (`METHODOLOGY_VERSION` native-security-2).
`SOURCE_SKILL_MIGRATION` records all fourteen original skill names: nine
`ActiveNative` contracts and five `DeferredInactive` (triage, track, fix,
verify-fix, hardening). Original `SKILL.md` bodies were not copied;
`plugins/security-scan` remains the untracked TypeScript plugin and is not
scan authority. Manifest `sourceSkillMigration` is part of the report envelope.

Legacy v1 `findings.json` is a read-only projection (`schemaVersion` 1,
`legacyValidatedFlag`, `coverageComplete` false, exit 2). It is not a resume
checkpoint and is not v2 confirmation. Sealed v2 reports remain preferred.

Windows-runnable named original tests added or completed on current source:
worker cannot read other-scan/host files; process output bounded without
newlines; cancel stops descendants via `taskkill /T` on Windows; effective
thinking level is the greatest supported level at or below the request;
cancel propagates to every worker on the same run; validation reserve cannot
be spent as discovery; SARIF omits parent/drive URIs; reports redact secrets
and terminal controls; store refuses a planted directory link; torn/tampered
checkpoints fail closed; shutdown leaves a resumable v2 checkpoint;
authorization is re-admitted on resume; every candidate gets a disposition;
deep mode requires two independent incomplete audits rather than duplicate
grep; unclosed candidates cannot complete; graph deterministic blockers still
block `approval_eligible` and an incomplete AI scan does not satisfy Always
mode.

`cargo test -p davinci-coding-agent --offline --locked --lib` of those named
tests passed. `cargo fmt --check` on the touched files and
`cargo clippy -p davinci-coding-agent --all-targets --offline --locked -- -D warnings`
passed.

Still open after the named Windows tests below: Unix process-group
descendants; Unix flock for shared scan slots; lab containment; Linux/macOS;
provisioned cargo-audit+db; measured held-out quality; workspace release
suite; live TUI. Not production-ready.

### Cross-process scan capacity and CLI fail-on / cancel 130

`RequestCapacity` now takes exclusive `scan-0.lock` / `scan-1.lock` files under
a bound directory (Windows `share_mode(0)`) in addition to the in-process
2-scan / 4-total pool. `configure_security_review` binds
`default_agent_dir()/capacity`. Isolated `RequestCapacity::new()` tests stay
in-process. Named tests
`security_scan_shared_lock_rejects_third_slot` and
`security_scan_capacity_is_shared_across_processes` passed (`davinci-agent`,
`--offline --locked`). Unix exclusive flock is not implemented; two Unix
processes can still both open the same lock file.

Current `target/debug/davinci.exe` was driven with `PI_SECURITY_SCAN_FIXTURE`
and `--offline`:

- print `--format json` and `--mode json` with a complete confirmed-high
  fixture both exited **1** (`security_cli_fail_on_exits_configured_blocker`).
- print cancel via `PI_SECURITY_SCAN_HOLD` + `PI_SECURITY_SCAN_INTERRUPT`
  (console Ctrl+C handler still installed) exited **130**
  (`security_cli_cancel_exits_130`).
- RPC `/security-scan` then `/sec-abort` then `/sec-report` reported
  cancelled (`security_rpc_abort_marks_cancelled`). RPC is a long-lived
  session and does not use process exit 130.

Clippy `-D warnings` passed for `davinci-agent` and `davinci-coding-agent`.
Not production-ready.

### Unix flock, process-group cancel, and remaining Phase 5/6 shipped-path tests

Unix scan-slot files now `flock(LOCK_EX|LOCK_NB)` after open (system C library,
no extra crate). Git and cargo-audit children use `process_group(0)` and
`kill -TERM -- -<pid>` on Unix; Windows still uses `taskkill /T`. These Unix
paths compile behind `cfg(unix)` and were not executed on this Windows host
(`blocking: "unverifiable"` for live Linux/macOS).

Review now loads repo-scoped `feedback.json` from the scan store parent and
runs `expire_stale_suppressions` on live dispositions. Named tests passed
`--offline --locked`:
`security_live_pipeline_expires_feedback_after_control_change`,
`security_reconciliation_keeps_source_bound_wording_as_one_finding`,
`security_legacy_v1_feedback_and_report_stay_outside_resume`,
`security_runtime_claim_requires_observed_result`,
`security_cancel_stops_descendants_on_supported_platform`.

Still unmet: live Linux/macOS/lab, measured held-out quality, workspace
release suite, live TUI. Not production-ready.

### Local Phase 1–6/9 close-out (Windows, current debug build)

CLI fail-on 1 and cancel 130 re-driven on `target/debug/davinci.exe` for both
print `--format json` and `--mode json`. RPC abort still reports cancelled.
Unauthorized start still rejects before a run exists. Command remains
discoverable. Complete/cancel race still has one terminal outcome.

Methodology: each of the five native phases declares Inputs/Output; nine
active-native skills keep contracts; worker roles load non-empty phase text
without Codex helpers (`security_methodology_active_phase_contracts_are_complete`).

Cross-file analysis: a live scan of `--scope entry.rs` with supporting
`helper.rs` records both paths on the candidate after snapshot reads of both
files, while coverage stays on the target (`security_cross_file_finding_requires_both_source_and_control_reads`).

Adversarial Windows scopes: reserved names (`con`, `nul`, `com1`, `lpt9`),
`<>"|?*`, trailing space/dot, `.git`, and 256-byte components are rejected
by `relative_scope` and skipped during inventory.

TUI/PTY still not launched. Securitas source+unit checks show cancelled,
failed, and incomplete status without success wording.

Clippy `-D warnings` passed for `davinci-coding-agent` and `davinci-tui`.
Not production-ready.

### Shipped-path semantic grouping and legacy v1 controller recovery

Reconciled remaining Phase 1–6/9 named items against current source. Unit tests
already covered fingerprint grouping, feedback expiry, store-level legacy v1,
and runtime-claim observed results. Two controller-path gaps remained:

- `security_live_pipeline_groups_differently_worded_same_root` drives
  `/security-scan` with two restated claims that share root identity. The live
  pipeline emits one canonical finding with two occurrences; sealed reopen
  preserves that projection. Presentation text is not a second root.
- `security_controller_legacy_v1_is_read_only_and_not_resume` plants a leftover
  `findings.json` under scan storage, then retrieves it through `sec-report`.
  The view is schema 1, `readOnly`, `legacy-readonly`, incomplete, exit 2.
  Finding selection is refused as non-v2. `sec-resume` fails without writing
  `report-1.json`, `seal-1.json`, or `checkpoint.bundle.json`.

Focused `--lib` run: 7 related tests passed (`--offline --locked`). Affected
crate all-target Clippy with warnings denied passed. Rust 1.83.0.

Workspace `cargo fmt --check` and workspace Clippy (`-D warnings`) passed.
The first workspace test run failed one lib test:
`security_conflict_capture_rejects_denied_links_and_cancellation_before_git`
because `ConflictStage::validate` applied user-scope `portable_relative` to
Git-reported `.git/config`. Identity validation now rejects only escapes and
malformed stage metadata; non-portable or excluded conflict paths skip as
`excluded conflict source`. Capture of `../` still fails closed.

After that fix, `cargo test --workspace --offline --locked` fail-fasted on
unrelated binary test `codex_fixture_login_persists_exact_provider_and_resolves_immediately`
(`davinci-coding-agent` `main.rs`, dirty harness/auth work). Security-scan
lib tests: 564 passed, 1 ignored. `--no-fail-fast` then completed the rest of
the workspace: that oauth login test is the only failure. Logs:
`{SCRATCH}/release-fmt.log`, `release-clippy.log`, `release-test.log`,
`release-test-nofailfast.log`.

Still unmet and not waived: measured held-out quality (no provider
authorization), live Linux/macOS/lab execution of Unix flock/process-group
paths, provisioned cargo-audit exe+db, live TUI/PTY. Not production-ready.

### Phase 1–6/9 named-task and TUI/Windows shipped-path pass

Original 65 named tests now exist on current source (exact names or wrappers
that still drive the shipped functions). Added controller-path coverage:

- Invalid flags (`--oops`) fail before a run or store exists and do not call
  the provider (`security_scan_invalid_arguments_create_no_artifact`).
- Resume with a changed supporting-reads policy fails without a provider call
  (`security_resume_rejects_changed_snapshot_or_policy`).
- TUI `security_sheet` now copies `status`/`scanId` from v2 JSON; missing
  status is `unknown`, never silently `completed`. Source-plus-unit render
  checks cancelled/failed/interrupted and empty-sheet admission.
- Inventory does not descend into Windows junctions/reparse directories;
  planted escape links skip and do not capture outside `secret.rs`.
- Inventory truncation records `inventory limit`.

Focused tests passed `--offline --locked` (lib, `davinci` bin TUI mapping,
davinci-tui, davinci-evals). Affected-crate Clippy `-D warnings` passed.

Named-task wrappers that only called sibling tests were replaced with
self-contained shipped-path tests (snapshot identity after mutation,
`--diff` rename both sides, policy text is not tool authority, supporting
reads false + store resume reject, inventory skip/truncation, scorer
zero-denominator). Focused tests passed.

Current-build `target/debug/davinci.exe` launched twice with
`PI_SECURITY_SCAN_FIXTURE` + `--offline`: print JSON and `--mode json`
exit 0, schemaVersion 2, coverageComplete true, empty findings,
`runner: offline-fixture`; RPC start returned preflight immediately,
later status/report completed. Logs: `{SCRATCH}/cli-json-rpc-1.log`,
`cli-json-rpc-2.log`. TUI launch without a PTY was killed after 2s with
no readback (`tui-launch-limit.log`); live TUI remains
`blocking: "unverifiable"`. Not production-ready.

### Continuation audit after paused provider goal

The previously reported workspace failure
`codex_fixture_login_persists_exact_provider_and_resolves_immediately` was
re-run directly from the current test binary and passed (1 passed, 799
filtered). A serial run of that same binary also passed the OAuth test. The
larger serial run reached 720 passed, 79 failed, and 1 ignored; the failures
were dominated by host `Access is denied` errors while spawning PowerShell,
Git, rustc, graph children, or creating the ACL-hardened security store. The
first hook failure poisoned its shared test mutex and caused the following hook
failures. This run is infrastructure-limited and does not supersede the earlier
focused security result.

The current debug executable entered the alternate-screen TUI and restored it
on Ctrl+C. A real offline interactive launch then failed before command entry
because the host denied creation of the session file. Consequently, this does
not close live `/security-scan` TUI acceptance.

The held-out evaluator now has a source-only `run_cases` path that executes all
80 cases from opaque directories, retains failed/incomplete cases as coverage
failures, aggregates latency and token usage, and attributes secure-case false
positives to per-family metrics. Authorized runs that miss the declared metric
thresholds remain blocked as `quality_thresholds_not_met`; authorization alone
cannot clear the gate. Three focused runner contracts cover full accounting,
partial failure, and refusal to reuse a pre-existing staging root. Direct
metadata compilation passed; this host denied `link.exe`, so the new tests could
not be linked or executed here. `git diff --check`
on the security scan, eval, TUI security view, and CLI integration paths passed.
The remaining production gates are unchanged: an authorized held-out provider
run meeting the declared precision/recall thresholds, provisioned cargo-audit
plus advisory database acceptance, live Linux/macOS process and capacity
checks, lab containment, a clean workspace release suite on a host that permits
required subprocesses/private directories, and live `/security-scan` TUI
acceptance. The feature remains not production-ready.

### Durable held-out capture and severity gates

The held-out runner now separates raw scanning from adjudication with two JSON
artifacts. The public capture contains opaque case tokens, raw reports, report
digests, coverage, latency and usage, but no labels. The private adjudication key
contains case/family/secure-vulnerable bindings and is bound to the complete
capture digest. Scoring rejects missing, duplicate, extra, stale or tampered
adjudications and captures. Both artifacts round-trip through strict serde
schemas, so evaluation can resume in a separate evaluator process without
exposing labels to scanner workers.

The independent evaluator now records severity on each adjudicated finding,
measures agreement with each vulnerable case's accepted severity band, and
counts confirmed High/Critical findings on secure cases. Release gating requires
every family to meet the existing confirmed thresholds, at least 85% severity
calibration with severity supplied for every true positive, and zero confirmed
High/Critical secure-case false positives. A focused contract covers a fully
calibrated 40/40 run and proves that one severe secure-case false positive closes
the gate.

Direct Rust 1.83 test-metadata compilation and rustfmt passed for the evaluation
module. This host still denies `link.exe`, so the newly added tests were compiled
but could not be linked or executed. No provider evaluation, analyzer
provisioning, platform/lab acceptance, clean release suite, installation, or live
TUI acceptance was performed. Those external and environment-dependent release
gates remain open; the feature is still experimental and not production-ready.

The evaluator also now rejects an authorized run unless it records model,
effective reasoning, immutable source digest, turn and token budgets. Run output
retains analyzer versions, exclusions, retries and known cost. Metadata is
bounded and validated before source staging and again when reopening a capture.
A reproducibility summary requires at least three authorized, configuration-
identical runs and reports precision/recall/severity ranges, p50/p95 latency,
aggregate tokens, retries and cost. Source or configuration drift rejects the
campaign instead of blending incomparable measurements. A campaign passes only
when every constituent run passes its quality gates.

Rustfmt and direct Rust 1.83 test-metadata compilation passed after these changes.
The exact new reproducibility test was attempted through Cargo, but Cargo could
not execute `rustc.exe` (`Access is denied`, error 5); therefore no linked-test
pass is claimed. The source remains metadata-compiled while full execution waits
for a host that permits compiler/linker subprocesses.

### Evaluation certification boundary hardening

The public metric-only `evaluate` helper could previously clear its blocker when
a caller supplied fabricated findings plus `authorizedProviderRun=true`. It now
always returns `blocking: "unverifiable"`; only `score_capture` can clear the
quality gate after verifying the full capture digest, exact per-case report
digests, complete one-to-one adjudications, coverage, every family threshold,
severity calibration and secure-case severe false positives. The older combined
`run_cases` helper and `CaseRun` type are test-only so production callers cannot
bypass independent adjudication.

Opaque evaluation tokens now use UUID v4 randomness rather than a wall-clock
nonce. Raw scanner reports are bounded to 8 MiB per case and 64 MiB per capture;
oversized output is replaced with a null report and retained as incomplete
coverage. Reopened captures are checked against the same bounds before scoring.
Focused regressions cover the metric-helper bypass and oversized raw output.

Direct Rust 1.83 test-metadata compilation passed. Direct `clippy-driver` with
warnings denied found one pre-existing needless struct update in the touched test;
after removing it, the same direct Clippy check passed. Cargo still cannot spawn
the compiler on this host, so linked execution remains unverified.

Campaign inputs are now bound to their exact scored capture and an internal
attestation over the complete run record. The three-run summary rejects duplicate
capture digests or any post-scoring mutation of metrics, provenance, usage, cost,
limitations or blocker status. This prevents a caller from modifying a valid
scored value before aggregation. Direct Rust 1.83 Clippy metadata compilation
with warnings denied passed after this addition.

### Unrestricted Windows release verification

After the host process and filesystem restrictions were removed, the held-out
evaluator suite executed successfully: 26 passed. `davinci-evals` all-target
Clippy with warnings denied and workspace formatting passed. The expanded
coding-agent security filter initially found three shipped-path regressions. A
legacy `findings.json` compatibility artifact is now excluded from the native v2
evidence index, so it cannot invalidate or outrank an already sealed v2 report.
The RPC cancellation acceptance recognizes the documented nonblocking
`cancelling` state. Rebuilding the debug executable proved the existing legacy
read-only exit policy returns 2. The three exact regressions passed, followed by
the full security filter: 452 passed, 2 ignored.

Affected coding-agent all-target Clippy with warnings denied passed. The single
offline locked workspace release run passed 2,595 tests across 31 suites, with 3
platform-specific ignores. A real Windows PTY accepted `/security-scan`, opened
the native Security Report sheet, displayed the scan identity and lifecycle, and
reported the deliberately exhausted offline fixture as failed with incomplete
coverage. This proves native TUI command dispatch and rendering without claiming
model quality.

The optimized Windows `davinci` 1.0.0 artifact built successfully at
`target/release/davinci.exe` (29,966,336 bytes, SHA-256
`D32CE9991283C65458E40597994785EF8B80568156EBC9B4063EF9EE503EFE7C`).
Its JSON-mode `/security-scan` smoke reached the native scanner and exited 2 with
a schema-v2 partial report when the empty offline fixture was exhausted.

Production release remains blocked by the plan's non-local acceptance gates:
three explicitly authorized, fixed-configuration held-out provider runs meeting
the quality and reproducibility thresholds; live Linux and macOS process-group,
shared-capacity, and lab-containment execution; and a deliberately provisioned
offline cargo-audit executable plus advisory database check. This host has only
the `x86_64-pc-windows-msvc` Rust target and no `cargo-audit` executable. The
feature therefore remains experimental and no installed user binary was
replaced.
