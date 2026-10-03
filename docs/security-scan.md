# Native security review

`/security-scan` runs an experimental, read-only AI review using the selected
model and existing provider authorization. It captures source before review and
gives the reviewer only snapshot listing, literal search, and bounded source reads.
It does not run repository commands, modify source, or use ordinary agent tools.

```text
/security-scan
/security-scan --scope crates/davinci-agent --mode quick
/security-scan --changed
/security-scan --diff main..HEAD --mode deep
/security-scan --format json
/security-scan --format sarif
/security-scan --new
/security-scan --report
/security-scan --finding <finding-id>
```

`/security-scan` is the only security command. Like the Codex Security plugin,
security review is a property of the session rather than a set of lifecycle
commands:

- **Status is automatic.** Progress appears in the security sheet, and a finished
  scan leaves one notice in the transcript with its finding count, how many meet
  the failure policy, and whether coverage is complete. `/security-scan` with no
  arguments while a scan runs reopens its progress instead of starting another
  (a request that selects something else is refused while one runs).
- **Resume belongs to the session.** When an interactive or RPC session starts,
  or `/security-scan` runs, an interrupted scan of this repository continues
  from its checkpoint and the transcript says so. When it cannot continue (its
  wall-clock deadline passed, or the policy or methodology binding changed) the
  session says so once and the next `/security-scan` starts a new scan. A bare
  `/security-scan`, or one with the same request, resumes; a different request
  or `--new` discards the interrupted scan and starts fresh. There is no abort
  command: a running scan stops when the session ends and stays resumable while
  its deadline allows. The deadline counts paused time (Quick 5 minutes,
  Standard 20, Deep 45), so a scan interrupted long ago is reported unresumable.
- **Results.** `--report` shows the current or latest report without starting a
  scan; `--finding <finding-id>` selects one finding. The TUI selects that row,
  and terminal output displays its causal details. JSON retains the complete
  findings and adds `selectedFindingId` and `selectedFinding`. Selection does not
  alter the sealed report, scan coverage, or policy exit status. Without a scan
  in this session, the newest stored reports of the repository are searched.

The TUI sheet and RPC polling still read two internal views, `sec-status` and
`sec-report`, the way `/graph` uses `graph-status`; they are not advertised.

## Session security watch

The session security watch is off by default and opt-in through the owner's
agent-directory settings. Project settings can disable or slow the watch, but
cannot enable it without owner permission. When enabled, the session watches its
own work. After a settled interactive or RPC turn
whose uncommitted working-tree state differs from the last watched state, a
background Quick review of the changes (`--changed`) runs on the session's model
and provider authorization, with the same restricted snapshot tools. One watch
review runs at a time, a new turn never cancels it, and reviews start at most
once per `securityScan.watch.minIntervalMs` (default 600000). Findings whose
classification and severity meet `securityScan.failOn` are reported as a notice
and injected into the next turn's context as a bounded block marked
`untrusted="true"`, so the agent can verify and fix them. A clean review adds
nothing to the transcript. `/status` shows the watch state and last result.
Watch reviews leave no checkpoint to resume. The watch never runs in print mode
or graph workers (`PI_GRAPH_ROLE`), outside Git repositories, or when
`securityScan.watch.enabled` is `false` or `DAVINCI_SECURITY_WATCH=0`. It shares
`securityScan.maxConcurrency` with an explicit scan: at `1` the watch waits while
`/security-scan` runs.

```json
{"securityScan":{"watch":{"enabled":true,"minIntervalMs":600000}}}
```

## Graph security gate

Graph runs in `risk` (high-risk changes) or `always` mode pass their changes
through a deterministic gate before review. It judges only the lines the graph's
mutation delta adds, so text that predates the change never blocks it; a file
whose delta has no line detail (binary or oversized patches) is judged whole.
Rules are token-bounded shapes: private-key blocks (critical), API credentials
such as `sk-`/`sk-proj-`, `ghp_`, `AKIA`, `xox?-` and `AIza` keys with realistic
length and character mix (high), free `eval(` calls that are not methods,
definitions or comments (medium), and `shell=True` (medium). A finding blocks
only at or above `securityScan.failOn.minimumSeverity`; lower ones are listed
in the passing verdict. The writer receives each blocker as
`path:line rule (severity) - message`. Deleted files are counted; binary and
policy-excluded paths are named in the verdict; a changed text file that could
not be scanned (larger than `maxFileBytes`, unreadable, or outside the
repository) makes coverage incomplete, reported as an unavailable verification
that blocks approval. In `risk` mode the gate runs only for high-risk changes,
so an unavailable scanner fails closed there as in `always` mode. Invalid
`securityScan` settings also make the gate unavailable. Gate artifacts are
written under the agent directory's `security-scans` folder, never inside the
repository, and `.davinci/security-scans/` is never treated as a graph mutation.

Scoped and changed/diff scans capture eligible supporting source inside the same
repository, within the remaining snapshot limits. Supporting bytes are immutable
and identified separately in inventory, reads, searches and reports. Diff helpers
come from the selected commits; changed scans retain worktree and baseline helpers.
They do not enlarge the target coverage denominator. Findings whose cited control
lies outside the target are labeled supporting/outside target and do not trigger
the target's failure policy. Without a control anchor, mixed target/supporting
evidence is conservatively classified as supporting.

Set `securityScan.supportingReads` to `false` in trusted settings to restrict all
source reads to selected targets. A project cannot widen a parent restriction.
Resume rejects a checkpoint containing supporting source under that narrower
policy. Credential exclusions apply to supporting capture as well. Captured
supporting files are not automatically sent as source text to the model; the
worker retrieves needed lines with the same restricted snapshot tools.

Standard mode builds a source-bound map, then starts a fresh audit context, followed
by independent candidate validation. Deep builds two maps in separate contexts and
runs a fresh audit for each; neither audit receives the other audit's map or results.
Quick combines mapping and investigation in one scoped audit. These workers run inside the
native scanner; they do not launch generic subagents. Findings must cite source
actually retrieved by the worker and pass the five assessment gates. This checks
evidence structure, not the truth of every model assertion.

Each audit retains its own reviewed and deferred paths, including baseline files.
It also returns a source-bound repository map: product classifications, entrypoints
or privileged decisions, callers/data flow, actors, assets, controls and explicit
unknowns. Source anchors must match captured bytes and have been retrieved by that
worker. JSON retains this map under each coverage audit, together with unmapped
current and baseline paths. Full file reads alone cannot establish complete
coverage: missing map rows, deferred units and unresolved assumptions keep it partial.
The initial coordinator map is retained separately. Assigned units omitted,
renamed or replaced with different source anchors remain explicitly unreviewed.
Mapping reads cannot satisfy the independent audit's source-read requirements.
Coordinator assumptions also survive handoff: the audit must explicitly resolve
each assumption with independently retrieved source evidence or leave coverage
partial. Removing an assumption from the audit map does not resolve it.
Deep coverage is complete only when both audits cover the captured corpus; the
union of two partial audits is insufficient. Terminal output lists up to 50
deferred paths per side and audit, with remaining counts; JSON retains every row.

Audit and validation turn allocations share the mode's total limit.
Mapping receives one third of each Standard/Deep audit's discovery allocation;
investigation receives the rest. The validation reserve remains separate.
Unused discovery tokens may fund validation. With multiple candidates, one third
of the remaining token allocation and validation turns is reserved for final
reconciliation. This allocation is stored once and reused after interruption;
resuming does not recompute larger allowances or refund consumed requests.
If candidate count exceeds the validation turn reserve, candidates without an allowance remain
deferred. Status and reports retain per-request token accounting and provider wall
time, including failed requests. Missing or inconsistent usage retains a
conservative reservation; measured usage stays unknown. Catalog-based cost
estimates are separate from token accounting, and unavailable prices remain unknown.

The default TUI shows progress in the security sheet. Print mode waits for a
terminal outcome:

```powershell
davinci -p "/security-scan --format json"
```

Print exit codes are `0` for complete coverage without a configured blocker, `1`
for complete coverage with a confirmed finding at the configured severity, `2`
for failure or incomplete coverage, and `130` for cancellation. JSON mode wraps
the report in a `security_scan_report` event. RPC clients issue the slash commands
as prompts (`/security-scan`, then `/security-scan --report`) without blocking
the RPC input loop; transcript notices arrive as `notify` UI requests.

The default blocking policy is confirmed High/Critical. Trusted settings may add
likely findings or lower the severity threshold, for example:

```json
{"securityScan":{"failOn":{"classifications":["confirmed","likely"],"minimumSeverity":"medium"}}}
```

Project policy merges preserve the union of blocking classifications and the
stricter severity threshold. A project cannot remove a parent blocker. Choosing
to block likely findings does not promote their classification to confirmed.
Terminal reports retain causal details, source locations, remediation, proof gaps
and unresolved leads; incomplete coverage still takes precedence over finding exits.

Exact duplicates must link directly to an earlier canonical candidate with the
same claim and evidence. A different entry point or affected instance cannot be
discarded as an exact duplicate. Opposing reportable and suppressed/not-applicable
assessments with matching causal evidence trigger a fresh restricted review.
It must reread decisive source and return a valid assessment for every conflicting
candidate. Successful reassessments retain `priorAssessment` and identify their
validation as `independent-reconciliation`. Missing evidence, invalid responses,
exhausted budgets or continuing disagreement keep coverage incomplete. Conflict
detection ignores titles, proof-gap wording and evidence ordering; it does not
yet recognize all semantically equivalent claims expressed differently.

Reportable assessments now require `rootCause`: the qualified control or missing
control context, violated invariant, normalized sink/decision and attack-path
class, with source anchors covered by reviewed evidence gates. Version-2 finding
fingerprints combine those labels and source paths; titles, line numbers and
source hashes are excluded. Current publication and recovery recompute that
fingerprint. SARIF exposes it as `davinci.root/v2`. Semantic labels must be bounded
and free of credential values or terminal controls. Matching fingerprints group
findings separately within target and supporting scope. Each canonical finding
retains all `candidateIds` and full `occurrences`, including their individual
classifications, severity, evidence, validation and prior assessments. Exact
duplicate claims retain their candidate links without inventing another affected
instance. `representativeCandidateId` identifies the displayed summary: confirmed
evidence first, then severity, with discovery order breaking ties. This does not
replace other occurrence assessments; failure policy checks every occurrence.
Terminal and TUI details show all occurrences; SARIF includes their metadata and
related source locations. Earlier experimental reportable records
without this root identity cannot pass the current final-report contract; create
a new scan. Changed methodology hashes also prevent incompatible cached resume.

Reports are experimental. An empty report is not a guarantee of security. Interrupted
reviews retain bounded, immutable partial checkpoints after completed mapping,
audits, and candidate dispositions. `/security-scan --report` exposes the latest checkpoint
if no final report exists; a new session resumes it or reports it unresumable. Checkpoints
always have `partial: true` and `coverageComplete: false`. Pending validation stays
deferred. Archived checkpoint status is `interrupted`; the active session overlays
its actual status, including cancellation. A corrupt latest checkpoint fails
explicitly, and a sealed final report takes precedence. Skipped
source, missing reads, exhausted budgets, and unresolved validation are reported
as incomplete coverage. Changed/diff review retains baseline source and current
or head source separately; it does not execute Git hooks or fetch missing objects.

Unresolved merges retain eligible Git index blobs as separate `index-base`,
`index-ours`, and `index-theirs` snapshot sides. Inventory entries include the
stage, object ID and captured content hash. Workers must read the exact stage
they cite; a worktree or other-stage read cannot substitute. Stage source obeys
the same credential, regular-file, encoding, policy-file, total-byte and inventory
limits, with explicit reasons for withheld stages. Supporting stages retain their
outside-target scope and are unavailable when supporting reads are disabled.
Captured stages are evidence of the unresolved merge, not a deployed combined
program. Reading all stages never makes its coverage complete.

The scanner stores private snapshots and sealed JSON, terminal, and SARIF report
generations under the agent directory's `security-scans` folder, outside the
reviewed repository. Treat that directory as sensitive: checkpoints contain source.
New version-3 seals cover the checkpoint, worker results, accounting, generation
markers and other committed evidence, alongside the three report exports.
Recovery rejects changed, added or missing committed files. Active lock files
and unpublished temporary writes are excluded. Hashing uses bounded streaming
reads. Older experimental seals without this evidence index cannot pass current
final-report recovery; run a new scan to produce the required artifacts.
Report hashes detect changes relative to the local seal; they are not signatures
against an actor who can rewrite the entire store. Resume reuses the original
snapshot in a new generation for interrupted, unsealed runs. All sealed reports,
including partial reports, are immutable; further analysis requires a new scan.
Resume checks current byte, inventory, policy-file and supporting-read limits
against target and supporting sources, including captured index stages.
New checkpoints publish their source data, checksum, and scan identity together
in one immutable bundle. Existing two-file checkpoints remain readable when both
files are intact. A corrupt published bundle cannot fall back to an older pair.
Checkpoint encoding counts escaped JSON, source inventory and the bundle's own
identity/checksum against the recovery byte limit. If metadata exceeds that limit,
the scan fails before checkpoint publication and model review; it does not leave
an immutable checkpoint that its own reader cannot reopen.
New checkpoints also bind the effective configuration and embedded methodology
manifest by hash. Resume requires exact matches; changed settings or methodology
require a new scan. Older checkpoints without these bindings remain readable but
cannot resume analysis.
Request reservations are atomically persisted before provider dispatch. Resume
counts prior requests and token usage against the same aggregate limits. A request
interrupted before usage was recorded retains its conservative reservation;
recorded usage replaces that reservation. Reports include cumulative accounting
across generations. Resume requires the current budget binding (version 3),
including durable validation/reconciliation allocation; older bindings cannot resume.
Record identity is separately bound to version 1. Candidate, finding and occurrence
IDs are deterministic within a scan, including cached replay after interruption.
Older checkpoints without this identity binding require a new scan. Record IDs
are run-scoped. Canonical finding IDs use the run, versioned
root fingerprint and scope, so adding another occurrence retains the finding ID.

Methodology bundle `native-security-3` sends only the relevant phases to mapping,
audit, counterreview and reconciliation workers. Audits receive reconnaissance,
discovery and reporting; attack-path and severity calibration go to the
counterreview that judges each candidate. Version 3 adds guidance adapted from
the Codex Security skills: diff reviews cover deleted files and removed
controls, per-call-site candidates, scope and counterevidence checks before
severity, severity examples, validation rubrics and sibling closure. Shared reporting restrictions
remain present. This avoids spending Quick-mode capacity on unrelated validation
instructions. Conservative unknown-usage reservations remain enforced. Older
methodology bindings require a new scan.
Discovery requests also retain their phase across resume: they cannot consume
the validation token reserve or the half of model turns allocated to validation.
Settled usage adjusts token consumption; a consumed model turn is never refunded.
The original wall-clock deadline also survives resume. Time spent paused counts
toward the mode's ceiling. Expired runs require a new scan; missing, corrupt or
mismatched deadline records cannot grant a fresh timeout.
Completed mapping, audit, counterreview and reconciliation results are stored with their source
read ledgers. Resume can reuse them when the scan, snapshot, task, provider
provenance, methodology and request limits match, after rechecking evidence and
typed validation. This reuses prior completed work; it does not count as another
independent audit or refund its original request usage.

`securityScan` settings are validated strictly. Resource limits can be narrowed
by trusted project settings. Enabled provider wire tracing also prevents admission.

| Setting | Default | Meaning |
| --- | --- | --- |
| `defaultMode` | `standard` | Mode when `--mode` is absent. |
| `maxFileBytes` | 2097152 | Per-file limit for reviews and the graph gate. |
| `maxPolicyBytes`, `maxInventoryEntries`, `maxSnapshotBytes` | 1 MiB, 50000, 512 MiB | Snapshot limits. |
| `validationReserveRatio` | 0.25 | Share of tokens reserved for validation (0.25-0.75). |
| `supportingReads` | `true` | Whether supporting source outside the target may be read. |
| `includeHidden` | `true` | Whether dot-files are scanned by the graph gate. |
| `failOn` | confirmed, `high` | Blocking classifications and minimum severity. |
| `maxConcurrency` | 2 | Security reviews running at once in a session (explicit scan plus watch), 1-4. |
| `retentionDays` | 30 | Scan directories older than this are swept when `/security-scan` runs (1-3650). |
| `watch.enabled`, `watch.minIntervalMs` | `false`, 600000 | Opt-in session security watch. |
| `analyzers` | `[]` | Only `cargo-audit` is accepted. |

A project can only narrow: limits and retention take the minimum, reads, hidden
files and the watch require both levels to allow them, the watch interval takes
the maximum, and failure policies merge as described above. The watch settings
are not part of a checkpoint's policy binding, so changing them never makes an
interrupted scan unresumable; every other field is.

`model` (`inherit`), `providerAccess` (`existing-authorization-only`),
`toolNetwork` (`deny`), `dynamicValidation` (`disabled`) and `indexIntoMemory`
(`false`) are accepted only at those values; they document fixed behaviour, and
any other value is rejected rather than ignored. The scanner therefore never uses
the network, never executes repository code, and never indexes evidence into
memory.

The optional `cargo-audit` analyzer is enabled by `"analyzers":["cargo-audit"]`
but must also be provisioned by the host: `DAVINCI_SECURITY_CARGO_AUDIT` names an
absolute cargo-audit executable (not a script, not inside the repository) and
`DAVINCI_SECURITY_ADVISORY_DB` a local RustSec advisory database no older than
30 days. The analyzer runs offline against the snapshot's `Cargo.lock` in an
isolated directory. Without both variables the report records a limitation that
names them. Advisory output is a lead, not proof of exploitability.
The selected provider must already have usable authorization; scanner admission
does not run authentication helpers or silently choose another model.

Offline regression tests can explicitly set `PI_SECURITY_SCAN_FIXTURE` to an
absolute JSON file and enable `--offline`. The file is an array of assistant
content-block arrays, consumed in order. This path is marked `offline-fixture` /
`synthetic-only` in provenance and never establishes real-model detection quality.
Offline mode without that explicit fixture does not make provider calls.

Measured precision/recall, a held-out model evaluation, cross-platform release
acceptance, and the optional offline analyzer adapter remain outstanding. See the
[implementation checkpoint](superpowers/plans/2026-09-06-native-security-scan-execution.md)
for the validation performed and remaining plan work.

Watch request receipts appear separately in `/cost`, `/status`, and RPC `get_session_stats.background.securityWatch`. Missing provider usage and unknown pricing remain unknown; reservation budgets are not measurements. Accounting is per session within the current process, including failures and calls still running during a session switch. See [learning cost visibility](learning.md#background-cost-visibility).
