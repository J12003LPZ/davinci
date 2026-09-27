# Request-efficiency evidence correction

Status: **prior promotion claims withdrawn; no promotion or parity is established.**
The September 27 audit found invalid controls, missing task tooling, incorrect
aggregation, and an incomplete grading boundary. The previous version of this
report must not be used to approve defaults or infer a performance improvement.

## Evidence available for this correction

The repository contains benchmark source, frozen public fixture definitions,
and historical prose reports. It does not contain the raw Windows campaign
rows, transcripts, binary checkpoints, or boundary probe referenced by the
previous FINAL. Their recorded paths under
`C:\Users\sergi\davinci-bench-evidence\request-efficiency\` are not available in
this checkout. No historical aggregate, corrected token total, confidence
interval, or per-stratum denominator has been reconstructed from rounded prose.

The prior candidate source identifiers and dirty-tree builds are not a usable
clean committed source attribution. The audit also identified the control as
an older ancestor rather than the PR's actual merge-base. Binary hashes written
in a report do not repair those provenance defects. The old combined
"Candidate" rows mixed DaVinci and Codex while "Parent" was DaVinci only; that
headline comparison is withdrawn in full.

The previous "Uncached input" column was total input. Exact replacement values
require the original per-row total and cached counts. Those data are unavailable
here, so this correction deliberately supplies no replacement numbers. The
claim that DaVinci request telemetry was unavailable is also withdrawn: the audit
found candidate request observations, while parent request counts were missing.
Candidate observations cannot establish reduction against an uninstrumented
parent.

## Promotion-rule disposition

The numerical findings below are **audit-reported historical observations**, not
newly recomputed campaign results. They explain rejection; they are not repaired
performance evidence.

| Rule | Required | Disposition of prior claim |
| --- | --- | --- |
| Rebuildable candidate and valid parent | Clean committed candidate; parent at actual merge-base | **Fail.** Dirty/unavailable candidate attribution and an older parent control invalidate code-effect attribution. |
| Usable task verification | Python and pytest available to each measured arm | **Fail.** The audit found missing Python/pytest in the DaVinci container and no gate-reminder exercise across the reviewed promotion rows. |
| Median improvement | At least 10% lower median against parent DaVinci | **Fail.** Audit-reported improvements were 6.9%, 0.6%, and 5.0% for the two windows and rerun labelled holdout. |
| Tail latency | At most 10% p90 regression | **Fail.** The audit reported 14.1% worse p90 in window 1. |
| Aggregate correctness | Candidate passes at least as many as parent | **Fail in window 1.** Audit-reported DaVinci passes were 73 versus 74. |
| Regression review | Inspect every parent-pass/candidate-fail pair | **Fail.** The audit identified 11 uninspected discordances. Aggregate passes do not explain them. |
| Request and continuation reduction | Comparable complete request telemetry | **Unavailable.** Parent request telemetry cannot establish the required reduction; candidate metrics must be shown when available. |
| Token evidence | Real uncached and output tokens by task/stratum | **Invalidated.** Total input was mislabeled uncached; exact corrected totals require absent raw rows. |
| Uncertainty | Paired analysis respecting task clustering | **Not established.** The old report omitted intervals; the audit says the reviewed bootstrap intervals included no change. No interval is invented here. |
| Independent windows | Sequential campaigns without competing measurement load | **Fail.** The audit found parent and candidate campaigns overlapping. |
| Independent hidden grading | No live writer at grading; grader/answers inaccessible to tools | **Fail.** Timed-out containers survived the Docker client, auth was readable, and public generators expose graders and solutions over the network. |
| Untouched holdout | New withheld cases with a protected boundary | **Fail.** Fresh repetitions of the same public task families are not an untouched holdout. |
| Legacy and larger-task coverage | Separate results, full denominators, scoped conclusions | **Unavailable here.** The mixed headline table obscured strata. Raw evidence is required for an exact corrected table. |
| Native Windows confirmation | The platform evidence required by the plan | **Not performed in the claimed promotion evidence.** No current platform confirmation is claimed. |

A campaign completeness check verifies row membership and identity. It does not
make any failed or unavailable promotion rule pass. No general DaVinci-versus-
Codex advantage follows from these historical campaigns.

## Harness corrections

The current benchmark implementation makes the failure modes explicit:

- The supplied Dockerfile includes Python, pytest, Git, Bash, certificates, and
  ripgrep. Preflight executes task command names in each arm and the grader
  before any live usage; versions are recorded.
- Container runs use unique names and CID files. The runner removes the actual
  container, verifies its absence, and reaps the client. Missing creation
  identity after any abnormal client exit is unproven cleanup. Failed cleanup
  records an ungraded failure and stops before grader injection or inspection.
- Native execution requires a Linux child subreaper with a matching process
  namespace view. It terminates and reaps owned descendants, including detached
  children, before allowing grading. Unsupported hosts refuse before launch.
  Windows and macOS native execution remain unavailable until an equivalent
  lifetime owner is implemented.
- Artifact scanning is separate from transaction scanning and includes reserved
  grader/credential/artifact paths and symlinks, including ignored files. Its
  bounded filesystem scope is recorded; it is not a proof of no data access.
- Public fixture generators and readable agent credentials explicitly make both
  supported native and container arms diagnostic. Promotion eligibility is
  false. There is no claim of an independently protected hidden-test boundary.
- Live runs require clean committed source identities and source trees that
  exist locally. A checkpoint build helper records before/after source identity
  and copied binary bytes. Offline comparison verifies the real merge-base of
  candidate and declared PR target.
- A machine-level kernel lock and durable launch-intent marker exclude competing
  cooperating campaigns, including after coordinator death with unproven cleanup.
  Imported comparisons check timestamp envelopes across every arm, including Codex.
- Summaries separate harnesses, legacy/large strata, and tasks. Uncached input is
  computed from per-row total minus cached. Missing metrics and incomplete
  telemetry keep explicit denominators; uninstrumented Jev usage is unavailable.
- Paired all-run and both-successful results are separate. Deterministic
  task-cluster bootstrap intervals and each promotion rule's pass/fail/unavailable
  status are emitted. One paired diagnostic window cannot certify promotion.
- Unit tests generate their own large fixtures in temporary directories. Tree
  hashes use a declared portable relative POSIX path order. The algorithm tag
  and affected public-tree hashes changed; fixture contents did not. Specialist
  JSON also uses explicit LF newlines, and both manifests now match the frozen
  LF fixture bytes across checkouts. Earlier
  manifests remain attached to their historical evidence and are not rewritten.

Usage, exact metric definitions, boundary limits, and the new checkpoint format
are documented in [the benchmark README](../../../scripts/bench/README.md).

## Current verification and limits

The original clean-checkout run reproduced the audit failure: 64 tests passed
and the large-fixture metadata test errored because ignored generated tasks
were absent. The final corrected offline benchmark suite runs 96 tests: 93
pass and 3 native-process execution tests are skipped on this host. Tests create
their own temporary fixtures. These are deterministic harness regressions, not model
performance measurements. Current runtime regression evidence belongs to the
runtime fixes and must not be inferred from old checkpoint prose.

Pytest 8.3.5 was installed in an isolated host environment for fixture validation.
All 12 broken starting fixtures fail their hidden grader, all 12 reference
solutions pass it, and the reference workspaces pass 107 combined public and
hidden tests. This is host-side offline fixture validation, not a measured model
campaign or proof of a protected grading boundary. Docker is not installed, so
no Docker smoke test was performed. This host also exposes different process IDs
to Python and `/proc`; the native owner therefore refuses execution. The three
skips require actual matching-namespace process ownership. The real refusal and
crash-lock regressions pass, while deterministic supervisor tests cover scoped
child cleanup; no live containment success is claimed on this host.
The offline unit suite tests
preflight rejection, cleanup ordering/failure, independent leakage flags,
provenance, lock exclusion, report partitioning, promotion-rule failures, and
cluster uncertainty. No live or paid campaign, historical rerun, native Windows
promotion campaign, installation, CI change, or release promotion was performed.

Historical [F1](F1.md), [P1](P1.md), and [L1](L1.md) are retained as historical
records, with their acceptance claims superseded by this correction. In
particular, prose that earlier described persistence, EOF behavior, empty-edit
rejection, or family reachability as already verified is not evidence for the
current implementation. Newly fixed behavior must be supported by current
deterministic tests, and any later performance decision needs fresh, valid
measurement evidence.
