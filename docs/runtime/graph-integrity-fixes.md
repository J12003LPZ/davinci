# Graph integrity fixes

These fixes address six source-verified bugs reported against main commit
`d44028fbe822cd71975e91ceda69c7c901a82693`.

| Bug | Implementation | Regression evidence |
| --- | --- | --- |
| Older progress checkpoint overwrites an acknowledged control | `GraphExecution::apply_control` uses the same `checkpoint_io` lock and lock ordering as `checkpoint_with`. `store::save_run` serializes disk publication with a per-state OS lock and rejects older revisions before writing companions. Equal revisions remain allowed for progress updates. | Control waits for checkpoint lock; acknowledged pause survives another checkpoint; stale save leaves newer disk state intact. |
| Renamed dirty file changes do not invalidate replay | `compute_repo_state_hash` parses raw porcelain v1 NUL records, consumes rename/copy source records, and reads destination bytes. The versioned key includes HEAD, index entries, all untracked paths, and length-framed content hashes. Unreadable Git state produces a non-reusable fingerprint. | Rename with Unicode and surrounding spaces changes hash on successive content edits; nested untracked edits invalidate hash. |
| Large or missing original file bytes appear empty | `MutationBaseline::old_bytes` returns `Option`; blob length/hash are checked. `capture_graph_delta` emits explicit old/new SHA-256 and byte-length summaries when original bytes are unavailable. | Modified and deleted files above 512 KiB and missing small-file blobs retain accurate original fingerprints without fabricated text hunks. |
| Blob GC deletes historical checkpoint preimages | `collect_unreferenced_blobs` reads current and historical revision snapshots in both authority roots. Unreadable references stop collection. New-run retention runs before creating the unpublished directory; resuming an existing directory does not prune it. | Blob referenced only by revision history survives; unreadable history prevents orphan deletion; existing orphan collection remains covered. |
| Workspace scans follow external symlinks | Mutation walkers inspect directory entry types and skip links. Workspace reads reject redirected ancestors and symlink leaves. Replay hashes link text without reading its target. | External directory/file link fixtures in non-Git and Git workspaces; external content changes leave replay hash unchanged, link-text changes invalidate it. |
| Retry replaces previous diagnostics | `write_log` creates an immutable `task.attempt_N.log` while retaining `task.log` as the latest view. The controller writes diagnostics before returning for recovery-required results. | Two attempts retain distinct contents; duplicate attempt publication cannot overwrite the original; recovery-required worker diagnostics exist. |

## Validation

Seven initial regressions reproduced failures on the original implementation.
The attempt-log regression was added with the logging change. Eight focused
regressions passed after the initial fixes; a ninth covers Git enumeration errors.

- `cargo test -p davinci-coding-agent --lib native_extensions::graph --locked -j 2`:
  461 passed, 5 ignored (existing ignored tests), 0 failed.
- After the final unsupported-version retention guard,
  `cargo test -p davinci-coding-agent --lib native_extensions::graph::store --locked -j 2`:
  28 passed, 0 failed.
- `cargo check -p davinci-coding-agent --locked -j 2`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Native tests used Rust 1.88.0 with test/dev debug information and incremental
compilation disabled to keep build artifacts bounded. Independent read-only
review found the swallowed enumeration-error edge, which is fixed and covered.

## Compatibility and limits

Git replay keys use a new versioned representation, so previous Git artifact
fingerprints are conservatively incompatible. No serialized checkpoint schema
or dependency pins changed. Missing original bytes remain explicitly unavailable;
these summaries cannot reconstruct deleted content. Git mutation checkpoints use
UTF-8 path keys and reject non-UTF-8 names rather than read a lossy substitute;
Git replay hashing preserves raw Unix filename bytes. No live provider calls,
benchmarks, installed-binary replacement, or main-branch push are part of this fix.

Git file enumeration errors, including output-cap failures, now propagate to
baseline/delta callers instead of manufacturing empty evidence. The dedicated
invalid-repository regression covers that boundary. Filesystem scan checks
cover existing symlinks; they are not a sandbox against concurrent adversarial
ancestor replacement.
