# Stage 1 addendum to the Stage 0 report

The preregistered [Stage 0 report](stage0-report.md) is unchanged and remains
the source of the measured 17-task numbers. This addendum records only what
Stage 1 added, without re-measuring Stage 0 or restating its numbers as new
results.

## Behavioral evidence added in Stage 1

- 12/12 Droid Script reference scenarios labelled `BEHAVIOR_PARITY`
  (`reference_script_fixture_suite_passes`, plus the real-guest syntax-error and
  long-job fixtures in `codemode_reference_fixtures.rs`).
- Nested Read/Execute, aliases, patch-once, structured exit codes, revocation
  while queued, ambiguous-effect recovery, and detached run ownership
  (start, status, wait, kill, once-only wakeup).
- Controlled Script and `wait_for_script` are model-facing, and the Script
  reference is composed into the stable prompt prefix only in controlled mode.

## Cache evidence added in Stage 1

- An identical wire prefix with fewer cache reads is classified
  `providerOrUnknown`; missing usage stays null.
- No TTL, breakpoint or hosted tool shape is sent to the unverified
  `chatgpt.com` Codex route.
- Resume, fork and rollback produce truthful manifests; digest equality never
  becomes a cached-token claim.

## What is still not measured

No new performance measurement was taken: no matched suite, no allowance
window, no dollar or quota claim. The promotion gate returns **`off`** with the
unknowns listed in [stage1-promotion.json](stage1-promotion.json) and
[stage1-promotion.md](stage1-promotion.md).
