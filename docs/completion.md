# Completion review

DaVinci asks for one requirement review when an assistant first tries to finish
a prompt that changed files and either changed at least two files or contains
at least three nonempty clauses separated by sentences, commas, semicolons or
newlines. Plan Mode and read-only runs do not trigger it. This initial threshold
is deterministic; it has not been tuned or accepted against a private dev split.

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
