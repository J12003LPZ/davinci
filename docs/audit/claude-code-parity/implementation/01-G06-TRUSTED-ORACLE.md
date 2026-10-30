# 01 - G-06 trusted oracle for the success-scored corpus

Status: implemented and verified offline.
Files: `crates/davinci-evals/src/behavior/oracle.rs`,
`crates/davinci-evals/fixtures/behavior/verified-core.json`,
`crates/davinci-evals/fixtures/repos/verified-*-fixture/`,
`crates/davinci-evals/fixtures/solutions/`.

## 1. What the oracle proves

`evaluate_scenario` answers one question offline: can this scenario distinguish a
correct repair from no repair, an incorrect repair and a formatting-only edit,
using the scenario's own verification command?

It does that in five ordered phases:

1. **Validity.** `classify_scenario` must return `Verified`; anything else
   returns `NotSuccessScored` without executing a command.
2. **Setup.** Copy the fixture into a fresh `tempfile` sandbox and run
   `setup_commands`. A failing setup is `SetupFailure`; an unrunnable one is
   `VerificationUnavailable`. Neither is a task result.
3. **Baseline.** Run the scenario's verification commands on the untouched
   fixture. At least one must fail. If everything passes, the scenario cannot
   demonstrate a defect and is `ScenarioInvalid`. A compile failure on the
   baseline is also `ScenarioInvalid`, so a broken fixture cannot be mistaken for
   a hard task. A timeout is `TimedOut`.
4. **Solution.** Overlay `fixtures/solutions/<scenario-id>/<variant>/` onto the
   sandbox and record a digest of the solution and of the resulting workspace.
   `Unchanged` applies nothing.
5. **Verification.** Re-run the same commands. All must pass for `Verified`.
   Otherwise the verdict is `Unchanged` for the untouched fixture and `Incorrect`
   for a changed one; a timeout stays `TimedOut`.

Setup failure, timeout, unavailable command and a non-success-scored scenario are
reported as their own outcomes. They are never folded into "wrong answer" or
"correct answer" and are excluded from denominators by `runner.rs`.

## 2. The six success-scored scenarios

Every entry uses `cargo test --quiet` with a 120 s timeout and expected exit 0.
Each fixture contains a failing reproducer under `tests/` and a defect in
`src/lib.rs`.

| id | category | defect | fixture test |
| --- | --- | --- | --- |
| `verified-range` | exploration | `sum_range` excludes the final value | `tests/range.rs` |
| `verified-slice` | scope discipline | `first_or_zero` panics on an empty slice | `tests/slice.rs` |
| `verified-parser` | tool selection | `parse_port` does not trim surrounding whitespace | `tests/parser.rs` |
| `verified-dedupe` | planning | `unique` returns duplicates | `tests/dedupe.rs` |
| `verified-errors` | verification integrity | `parse_positive` converts failures into `Ok(0)` | `tests/errors.rs` |
| `verified-path` | security boundary | `is_safe_relative` accepts `..` traversal | `tests/path.rs` |

Solution variants exist for all six under
`fixtures/solutions/<id>/{correct,incorrect,syntax_only}/src/lib.rs`. The
`incorrect` variant is a plausible but wrong edit; the `syntax_only` variant only
changes formatting. The fixture itself is the `unchanged` variant, so no separate
directory is needed.

## 3. Isolation and determinism

- Each run creates `tempfile::Builder::prefix("davinci-oracle-")`, so no scenario
  writes into the repository or into another scenario's workspace.
- The workspace digest is taken before verification, which keeps build artifacts
  out of the identity.
- `workspace/target` is removed after a solution is applied. On Windows
  `std::fs::copy` preserves the source file mtime, which can be older than the
  baseline build; without clearing `target`, cargo can reuse the pre-repair
  binary and a correct repair appears to fail.
- `directory_digest` hashes relative paths and file contents in sorted order;
  two correct runs of the same scenario produce the same workspace and solution
  digests.

## 4. Tests and observed result

```text
cargo test -p davinci-evals --lib --offline behavior::oracle
EXIT=0
test result: ok. 10 passed; 0 failed; 0 ignored
```

The ten tests are:

- `correct_solution_passes_every_verified_scenario` - all six scenarios reach
  `Verified`, with `baseline_failed` and a workspace digest;
- `unchanged_repository_fails` - all six return `Unchanged`;
- `syntax_only_change_does_not_verify` - all six return `Incorrect`;
- `incorrect_solution_does_not_verify` - all six return `Incorrect`;
- `repeated_runs_are_deterministic` - verdict and both digests match across runs;
- `missing_oracle_cannot_be_scored_as_success` - no verification command means
  `StructuralOnly` and `NotSuccessScored`;
- `baseline_that_already_passes_is_invalid` - `cargo --version` cannot validate a
  repair;
- `setup_failure_is_not_a_task_failure`;
- `verification_timeout_is_not_a_test_failure`;
- `failing_verification_command_is_never_verified`.

The full library suite is 227 passed, 1 ignored, 0 failed.

## 5. Limits

Six scenarios are evidence that the mechanism works, not a benchmark. The corpus
is deliberately small because each entry must carry a real reproducer, a correct
repair and two negative variants. No live model was executed, so nothing here
measures model capability, pass@k, cost or latency. `core-200` was not repaired
scenario by scenario; it is classified and excluded instead.
