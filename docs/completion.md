# Completion review

DaVinci asks for one requirement review when an assistant first tries to finish
a prompt that changed files and either changed at least two files or contains
at least three nonempty clauses separated by sentences, commas, semicolons or
newlines. Plan Mode and read-only runs do not trigger it. This initial threshold
is deterministic; it has not been tuned or accepted against a private dev split.

The review is not sent when a check already passed on the latest change, that
is, when the completion evidence is `verified`: the review ends in "rerun the
appropriate checks", and that turn would only repeat the check. Every other
evidence state (no check, a failed check, a check that covered part of the
change, a pass that a later edit made stale) still gets the review, and a later
unverified edit in the same prompt is reviewed. `RunStats` counts these as
`completionRequirementReviewsSkipped`, at most one per prompt.

Two limits. The skip needs a check that ran tests. A passing check that ran
none (`cargo check`, `cargo clippy`, `cargo fmt`, `tsc`, `eslint`, `go vet`,
`npm run lint`/`typecheck`/`build`) stays `verified` for the verification gate
but does not skip the review, because it says nothing about tests for each
requirement. A chain such as `cargo check && cargo test` counts, since it ran
tests, and a test pass on the latest change still counts after a later lint or
build check. Targeted scripts that run the changed code (inline Python checks with
assertions) count as tests; syntax-only checks do not. For test suites, "ran
tests" is judged from the command, so a test command that
selects or lists no tests (`cargo test nomatch`, `pytest --collect-only`) and a
project `npm test` script that only lints still count, as they already did for
the verification gate. And the review is decided
before the harness's automatic rerun of the last check, so an edit made after
the last passing check still gets the review even when that rerun would pass.

Deliberate limitation: `make check` and `npm`/`yarn`/`pnpm check` are classified
as checks, never as test runs, even though a project may wire them to its test
suite. A pass of one of them is a verifier for the verification gate but does
not skip the requirement review. The extra review is the safe direction; a
project that wants the skip runs its test command (`make test`, `npm test`).

A failed command clears accumulated coverage when it runs a checker, and that
recognition is deliberately loose: a directory, upper case, a `.exe`, `.cmd`,
`.bat` or `.ps1` suffix, an `env` prefix (with its `--`, `-i` and `-u NAME`
options, also when `env` is given by path), simple quoting and the PowerShell
`&` call operator are ignored, so `.\node_modules\.bin\eslint.cmd .` counts.
The segment splitter follows POSIX rules, so a PowerShell argument with a
trailing backslash inside quotes (`".\src\"`) is malformed and not recognized.
A recognized checker anywhere in a chain counts even if an earlier segment
failed first and it never ran (`grep -q X f && eslint .`); that clears coverage
and causes an extra review, which is the safe direction. Earning test credit
still requires the recognized command form, so `./tools/pytest tests/` credits
nothing. This recognition is still a heuristic over command text, so an unusual
spelling of a failed checker can still keep coverage.

Known limitations. The gate classifies shell command text heuristically, and a
command it cannot parse falls back to unknown: no coverage is credited, so the
review runs. The one exception is the failure case in the first item below.

- Output redirection on a checker (`tsc --noEmit > /dev/null`) is not recognised
  as a verifier. If it fails, accumulated coverage is not cleared.
- A `cd` whose target comes from a variable credits nothing.
- `make check` and `npm`/`yarn`/`pnpm check` are not treated as tests (see above).
- A test run credits only edited paths under the directory it ran in, and a
  full-workspace run only those under the workspace root. A path outside it, such
  as an extra authorized root, needs its own passing test run.

The reminder asks the same agent to list explicit requirements, identify a check
for each, add missing coverage in existing test files, update obsolete tests and
rerun checks. The existing mutation verification gate still follows this step.
Further edits do not cause a second requirement review for the same user prompt.
A genuine steering or follow-up prompt starts a new review budget; injected
harness context does not.

The change list combines a run-start Git status observation, bounded file
identities and successful tool mutation paths. It includes shell changes and
marks newly observed files. Pre-existing unchanged edits are omitted, and an
observed edit that returns to its original bytes is omitted. File observation
checks read permissions and workspace boundaries, excludes protected paths and
does not run content reads through a hook, decision subscriber or worker. Where
content identities are unavailable, Git status and tool mutation facts still
contribute; the reminder states that its list may be incomplete. Up to 64 paths
are shown. Paths are quoted file data. The harness never deletes files.

Requirement reminders and blocking completion-hook feedback are transient
provider overlays. Each stays at the position where it was appended during the
prompt so later requests preserve the existing provider prefix. They are
excluded from saved sessions, native replay records, compaction input and
the final answer. The Stable prompt hash and provider tool schema are unchanged.
Reason-only `completion_reminder` events identify `completion.requirements` and
`completion.hook_block`; `RunStats` exposes `completionRequirementReminders`,
`completionHookBlocks` and `completionHookLimitHits` for status, RPC and bench
telemetry. After three hook blocks, the next attempted finish emits a visible
`completion.hook_limit` notice rather than invoking the hook again. The host hook
receives `stop_hook_active=true` after a block, until it allows completion or a
new real prompt arrives.

Offline fixtures cover the shared agent loop. Acceptance still requires paired
private dev-split and holdout campaigns measuring success, unrelated edits and
uncached token growth against an agreed ceiling. No accuracy or cost improvement
is claimed from fixture results. The source rows behind the readiness plan's
22/36 benchmark are not present in this checkout; its four unrelated-edit causes
cannot be established here.

The host can disable requirement review for a baseline arm with
`requirementReview: false` (or `DAVINCI_REQUIREMENT_REVIEW=0`). This control does
not disable existing verification or completion hooks and does not modify the
Stable prompt or tool schema.
