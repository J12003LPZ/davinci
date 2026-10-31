# 08 - Final implementation report

Status: complete for the three gaps, with the limitations listed in section 7.
Branch: `J12003LPZ/cc-tui-parity` (PR branch created from it).
Base revision: `6912f9405d73b24474ad94b74ddb8802ee015d7a`.

## 1. What this wave delivers

| gap | deliverable | evidence |
| --- | --- | --- |
| G-06 | a real offline oracle, 6 success-scored scenarios, 3-way scenario validity, success denominators that exclude invalid/structural scenarios | docs 00-02, `validity-report.json`, 10 oracle tests |
| G-07 | terminal `CompletionStatus` derived from mutation state, persisted per turn, restored on reload/compaction/rewind | docs 03-04, 13 targeted tests |
| G-02 | host-derived `VerificationReceipt` from runtime state plus before/after snapshots, per-worker slots, single/multi/async attachment | docs 05-06, 14 unit + 4 integration tests |

The old behavior is gone: the `core-200` loader no longer injects
`git diff --check`, the test that certified that fallback was replaced by one
that rejects it, and a worker's prose cannot produce a verified receipt.

## 2. Behavior changes a user can observe

1. `davinci-evals corpus verify` is a fail-closed gate over the six verified
   scenarios.
2. `davinci-evals corpus validity --report <path>` emits per-scenario findings.
3. A turn that edited files and never ran a check now ends with
   `Verification was not observed: ...` instead of the generic incomplete text,
   and the session records `verification_unavailable`.
4. A delegated worker result now ends with a host-owned
   `[verification receipt] status: ...` block and exposes the same data in
   `details.verificationReceipt` or `details.verificationReceipts`.
5. A worker that produced no receipt is reported `unavailable`, even when its
   text says everything passed.

## 3. Files

### G-06

Added:

- `crates/davinci-evals/src/behavior/validity.rs`
- `crates/davinci-evals/src/behavior/oracle.rs`
- `crates/davinci-evals/fixtures/behavior/verified-core.json`
- `crates/davinci-evals/fixtures/repos/verified-{range,slice,parser,dedupe,errors,path}-fixture/`
- `crates/davinci-evals/fixtures/solutions/verified-*/{correct,incorrect,syntax_only}/src/lib.rs`

Modified:

- `crates/davinci-evals/src/behavior/scenario.rs` - removed the implied oracle,
  added the verified-core loader and corpus tests
- `crates/davinci-evals/src/behavior/executor.rs` - `ScenarioRunResult.validity`
- `crates/davinci-evals/src/behavior/runner.rs` - denominator exclusion and
  `excluded_scenarios`
- `crates/davinci-evals/src/behavior/mod.rs` - exports
- `crates/davinci-evals/src/main.rs` - `corpus verify` / `corpus validity`

### G-07

Added:

- `crates/davinci-agent/src/completion_status_tests.rs`

Modified:

- `crates/davinci-agent/src/lib.rs` - `CompletionStatus`,
  `CompletionStatusRecord`, persistence and restore
- `crates/davinci-agent/src/verification_notice.rs` - turn-end recording and text
- `crates/davinci-agent/src/events.rs` - notice status type
- `crates/davinci-agent/src/lib_tests.rs` - type and receipt-block updates
- `crates/davinci-coding-agent/src/main_tests.rs`,
  `crates/davinci-coding-agent/src/davinci_interactive_tests.rs` - event type

### G-02

Added:

- `crates/davinci-agent/src/subagent_receipt.rs`

Modified:

- `crates/davinci-agent/src/subagent.rs` - `receipt_slot`, per-worker slots,
  attachment, 4 integration tests
- `crates/davinci-agent/src/lib.rs` - module and re-exports
- `crates/davinci-agent/src/runtime/workflow/executor.rs` -
  `receipt_slot: None` for the snapshot-less path
- `crates/davinci-coding-agent/src/main.rs` - host snapshot and receipt
  publication around `run_worker_turn`

### Documentation

- `docs/audit/claude-code-parity/implementation/00` through `08`
- `docs/audit/claude-code-parity/implementation/validity-report.json`

## 4. Compatibility

- `core-200` still loads and can still be used for structural metrics. It no
  longer contributes to a success rate.
- Old evaluation artifacts deserialize with `validity: structural_only`.
- Old sessions without a `completion_status` entry restore as "no recorded
  status" and derive the live status from mutation state.
- Old `verification_notice` JSON decodes for all previously existing status
  names.
- `CompletionEvidence` and the reminder budget are unchanged.
- `SubagentFn` did not change; the receipt slot is an additive field on
  `SubagentRequest`.
- The openai-codex subscription path, selected model and effort were not touched;
  no API-key or paid-API fallback was added.

## 5. Validation summary

All commands and counts are in document 07. The gates that passed:

```text
cargo test -p davinci-agent --offline        EXIT=0
cargo test -p davinci-evals --offline        EXIT=0
cargo test -p davinci-mcp --offline          EXIT=0
cargo test -p davinci-coding-agent --offline EXIT=0
cargo fmt --all --check                      EXIT=0
cargo clippy --workspace --all-targets --offline EXIT=0
davinci-evals corpus verify                  EXIT=0  verified 6 behavior scenarios
```

No live model call was made.

## 6. Rollback

| gap | rollback |
| --- | --- |
| G-06 | remove `verified-core`, `validity`, `oracle` and the two CLI arms; restore the loader and the old test only if the invalid fallback is intentionally being restored (not recommended) |
| G-07 | drop the `completion_status` entry writer/reader and the two new enum cases; old sessions remain readable |
| G-02 | remove `receipt_slot` and the host wrapper; workers revert to text-only results |

Each gap is independent: G-07 and G-02 do not read G-06's corpus, and G-06 does
not depend on the agent crate.

## 7. Limitations and residual risks

1. **Six scenarios are not a benchmark.** They prove the oracle mechanism; they
   do not estimate model capability or pass@k.
2. **`core-200` is not repaired.** 110 scenarios are structural-only and 90 are
   invalid because they reference missing files. Treating any of them as success
   evidence remains unsupported.
3. **The classifier is conservative, not omniscient.** It rejects known
   non-oracles and validates executability; semantic sufficiency is proven only
   by the offline oracle for the six verified scenarios.
4. **Workflow-executor workers have no snapshots.** They pass
   `receipt_slot: None` and therefore report `Unavailable`. Extending snapshots
   to that path is follow-up work.
5. **The receipt is per turn and per workspace.** It is not a multi-turn proof
   and it intentionally excludes build output, caches and VCS metadata.
6. **No statistical or cost claim is made.** There is no live campaign, no
   provider traffic and no token/cache measurement in this wave.
7. **Snapshot cost.** Capturing a large workspace twice per turn has a cost that
   was not benchmarked here; the 50,000-file / 512 MiB bound fails closed rather
   than producing a partial receipt.

## 8. PR contents

Included: the G-06, G-07 and G-02 source, tests, fixtures and
`docs/audit/claude-code-parity/implementation/`.

Intentionally left uncommitted:

- G-01 work: `crates/davinci-agent/src/{mcp.rs,tool_ledger.rs,tools.rs,turn.rs}`,
  `crates/davinci-mcp/*`, `crates/davinci-agent/tests/common/codemode.rs`,
  `crates/davinci-coding-agent/src/browser_integration_tests.rs`;
- workspace instructions and planning material: `AGENTS.md`, `CLAUDE.md`,
  `docs/superpowers/*`;
- the earlier top-level audit documents `docs/audit/claude-code-parity/0*.md`
  and `08-*.md`, which are pre-existing untracked output from the audit phase and
  are not part of this implementation wave.
