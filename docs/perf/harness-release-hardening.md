# Harness request and CLI hardening

This change addresses request overhead and headless automation contracts. It
is not evidence that Davinci beats Codex on every task. No new live model
campaign was run for this release; account-funded A/B measurements require
separate authorization.

## Outcomes and offline evidence

| Surface | Failure reproduced | Regression evidence |
| --- | --- | --- |
| Named-file capture | Installed library read hooks and runtime bindings did not prevent automatic reads; notices were treated as file requests | `davinci-agent/tests/named_file_boundaries.rs` installs a hook, binds a runtime, inserts a job notice, and checks that file bytes are absent |
| Bounded discovery | An incomplete basename scan could declare a duplicate unique; empty directories and ignore files escaped the file-count cap | The same suite checks incomplete discovery and oversized ignore files; `tools.rs::bounded_discovery_regressions` checks directory-entry limits |
| Text and path integrity | Invalid UTF-8 expanded into replacement characters; canonicalization discarded a deny on a path alias | Boundary tests reject invalid text and resolved unsafe names; Windows junction and Unix symlink cases exercise alias denies |
| First-request context | Safe automatic capture must still reach the actual CLI provider request | `output_schema.rs::named_file_bytes_reach_the_first_cli_provider_request` records one fixture request containing the named file body |
| Verification notes | Quoted text containing `; python` consumed the once-per-change note | `turn.rs::checker_quote_regressions` uses the existing quote-aware shell segment parser; gate-reason tests distinguish failed checks from incomplete coverage |
| Search exclusions | Directory-specific wildcard globs were overridden; negated globs could remove default exclusions | Search regressions include explicit directory wildcards, a negated glob, and a 40,000-star pattern using iterative matching |
| Structured output | Unsupported assertions could be reported valid; other providers never saw the schema | `output_schema_limits.rs` rejects unsupported/malformed schemas; an Anthropic fixture records schema instructions on the first and sole repair request |
| Numeric validation | Decimal rounding could certify fractional source text as an integer | Numeric tests reject value-changing parse round trips while retaining equivalent decimal/exponent spellings |
| Repair provenance | Generated repair text was marked as a genuine user instruction | An executable fixture checks both provider requests and the repair event's origin marker |
| Additional directories | The permission gate allowed a sibling write but the transaction layer rejected it | `exec_parity_flags.rs` performs actual absolute and relative sibling writes/edits, rejects unlisted siblings, and preserves explicit denies |

The new reproductions were run against the faulty code before their respective
fixes: five named-file boundary cases, two search/checker cases, and four
schema-boundary cases failed. The added-directory executable test separately
exposed `transaction target is outside the workspace` after permission was
allowed; the fixed path retains the transaction coordinator and journal.

## Deliberate limits

- Automatic file capture is enabled only on appended-context routes. It
  declines installed pre-tool hooks, runtime-bound turns, active Context VM,
  workers, missing read tools, and active contracts. Ordinary gated reads
  remain available in those configurations.
- Discovery counts at most 2,000 directory entries and requires a complete
  scan before declaring a basename unique. Ignore loading has separate byte,
  rule-count and ancestor limits. See [named-file context](../runtime/named-file-context.md).
- Structured output supports a documented JSON Schema subset. Unsupported
  assertions fail before a provider request; schema/reply size, nesting,
  validation work, and diagnostics are bounded. Precision-losing numeric
  literals are rejected rather than silently rounded.
- Added-directory mutation routing covers `write`, `edit`, and `notebook_edit`.
  Patch tools retain their existing primary-workspace-relative contract.
  Graph and isolated workers do not inherit additional roots. See
  [permission modes](../permission-modes.md#additional-writable-directories).
- Model latency, real-task pass rate, and token savings still require a live
  paired campaign. Fixture-provider request capture proves wiring, not model
  quality or production speed.

## Verification scope

The release checks cover the complete `davinci-agent`, `davinci-ai`, and
`davinci-coding-agent` offline test suites, all-target clippy with warnings
as errors, formatting, benchmark-helper tests, and Python verification-helper
tests. Windows compilation uses one Cargo job after parallel compilation
exhausted the paging file. The provider-selection fixture now explicitly
selects its provider and model rather than assuming the test process has no
other provider credentials.

Validation has three layers: deterministic unit regressions, executable
fixture-provider/permission tests, and offline smoke checks of the installed
release executable after backup and SHA-256 comparison. Unix-only tests are
not executed by the Windows release run. Exact run totals and installation
results belong in the PR and delivery report, not inferred from this checklist.
