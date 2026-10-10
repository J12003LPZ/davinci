# Remediation of the final bug-audit report

Baseline: `ae974c6c` (the same source tree as merged PR #171, `origin/main@4b3c482a`).
The supplied audit described the older `e61aa242` checkout. Its three preexisting,
untracked report files are preserved and excluded from this change.

## Scope and findings

| Finding | Resolution in this change |
|---|---|
| DVA-001 | Clone the selected ancestry, excluding abandoned siblings and later branches. |
| DVA-002 | Keep session-scoped entry IDs, preserving compaction and label references, as fork already does. |
| DVA-003 | Bound inspector strings at a UTF-8 character boundary. |
| DVA-004 | Return a failed RPC response on rename persistence failure; emit no success event. |
| DVA-005 | Emit the persisted name after every successful rename, including `null` when clearing it. |
| DVA-006 | Mask complete quoted assignments before token masks, shared by source/evidence/report redaction. |
| DVA-007 | Reject completed malformed JSONL records without rewriting them; retain existing locked torn-tail recovery. |
| DVA-008 | Already fixed by PR #167 in this baseline; no notebook changes needed. |
| DVA-009 | Answer every server request in an SSE or JSON batch before selecting the correlated response, including an SSE event ending at EOF. |
| Cancellation suspicion | Reproduced with parent and child callback panics. Finish callbacks and descendant cancellation, then resume the first panic. |
| Semantic transport suspicion | Reproduced unbounded headers, cancellation history, and incoming frame queuing. Bound headers/buffer, queue eight frames, compact old tombstones behind a monotonic horizon, and discard stderr in fixed byte buffers. |
| Vector-memory suspicion | Reproduced through message-to-memory extraction; share complete quoted-assignment masking with Security Scan. |

## Regression evidence

New tests were first run against the vulnerable behavior and failed for the
reported invariants: clone ancestry/context, completed JSONL corruption,
Unicode truncation, cancellation propagation, failed rename, cleared-name event,
quoted report/source masking, memory extraction, SSE batch ordering, LSP header
and parser buffering, cancellation-history bounds, and idle incoming queue growth.

Redaction controls include escaped quotes, Unicode, multiline quoted values,
unterminated quotes, and a final incomplete escape. Existing unquoted/token masks
and ordinary source/prose controls remain in the owning suites. Semantic controls
cover ordinary split/multiple frames, the existing LSP subprocess flow, and a
maximum-size frame; old cancelled replies remain ignored while older live
requests still resolve. The incoming-queue fixture also emits newline-free,
invalid-UTF-8 stderr and exercises owned child cleanup.

## Validation

Focused session-integrity (9), MCP library (46), and RPC (25) tests passed.
All four affected library suites passed with
`rtk proxy cargo test --locked -p davinci-session -p davinci-agent -p davinci-mcp -p davinci-coding-agent --lib -- --test-threads=4`.
`rtk proxy cargo fmt --all -- --check`,
`rtk cargo check --locked --workspace --all-targets --all-features`,
`rtk cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
and `rtk proxy git diff --check` passed. The final source diff was reviewed solo,
including the redaction callers and alternate input/ordinary-input controls.
`rtk cargo test --locked --workspace --no-fail-fast -- --test-threads=4` passed:
RTK reported 6,222 pass events, 57 ignored events, and 9 filtered events across
219 summaries, with no failures. These include nested fixture subprocesses and
are not a count of unique tests. Ignored fixture helpers can be exercised by
their parent tests; other ignored tests remain outside these validation claims.

Final review found and corrected a notification-handling regression in the SSE
patch. The 46-test MCP library suite and its all-target/all-feature Clippy gate
were rerun and passed after that correction; the workspace run used the earlier
compiled version. No application source changed after those affected checks.

No live provider calls, subscription probes, new dependencies, credentials,
installed binary replacement, subagents, or agent worktrees are part of this fix.
Runtime changes require rebuilding and restarting the application to take effect.
These repairs do not establish exhaustive repository coverage or release readiness.
