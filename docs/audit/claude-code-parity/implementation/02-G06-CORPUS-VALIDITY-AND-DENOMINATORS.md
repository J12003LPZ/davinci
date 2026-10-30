# 02 - G-06 corpus validity and success denominators

Status: implemented and verified offline.
Files: `crates/davinci-evals/src/behavior/validity.rs`,
`crates/davinci-evals/src/behavior/runner.rs`,
`crates/davinci-evals/src/behavior/executor.rs`,
`crates/davinci-evals/src/main.rs`.

## 1. One classifier, three outcomes

`ScenarioValidity` is the only success-scoring decision:

| value | meaning | success-scored |
| --- | --- | --- |
| `Verified` | has a real correctness oracle | yes |
| `StructuralOnly` | runnable, but only structure/behavior can be scored | no |
| `Invalid` | cannot be honestly executed or scored | no |

`classify_scenario(scenario, fixture_root)` is pure: it reads the scenario and the
fixture tree, executes nothing, and returns a `ScenarioValidityReport` with the
verification-command counts and a list of findings.

## 2. Findings

Blocking findings make a scenario `Invalid`:

- `empty_scenario_id`, `duplicate_scenario_id`, `empty_request`,
  `missing_requirements`;
- `missing_fixture`, `missing_referenced_file`;
- `unsafe_relative_path` (absolute paths, `..`, or fixture paths that escape);
- `invalid_setup_command`, `invalid_verification_command`;
- `zero_verification_timeout`.

Non-blocking findings make a runnable scenario `StructuralOnly`:

- `missing_verification_command` - no command at all;
- `structural_verification_only` - every command is structural;
- `verification_kind_without_oracle` - a requirement asks for `test`/`build`/etc.
  but no meaningful command of that kind exists.

Duplicate IDs are detected at corpus level in `CorpusValidityReport::build`; every
occurrence is marked invalid, not just the second one.

## 3. What counts as a meaningful oracle

`is_meaningful_oracle` normalizes whitespace and case, then rejects commands that
only inspect or format a patch:

```text
git diff, git status, git log, git show, git ls-files, git rev-parse,
true, ls, dir, cat, echo, pwd, test -e, test -f, test -d
```

This is the direct replacement for the old `git diff --check` fallback. A
mutating scenario with no real command is now `StructuralOnly`, not a verified
success, and the fallback is not injected at load time.

## 4. Denominator exclusion

`ScenarioRunResult` carries `validity`, populated by `execute_scenario` before
anything runs. `aggregate_scenario_results` filters behavioral results with:

```rust
result.is_success_scored()
    && matches!(result.disposition, Completed | VerificationFailed)
```

and records every excluded result:

```rust
ExcludedScenario { scenario_id, validity, reason }
```

so a report shows what ran but did not count instead of silently dropping it.
Infrastructure errors remain handled by the existing disposition logic; this
change adds validity as an independent gate, so an invalid scenario cannot
inflate a numerator even if its trace looks successful.

Old artifacts that lack `validity` deserialize as `StructuralOnly` (via
`default_legacy_validity`). They cannot retroactively become verified successes.

## 5. CLI contract

```text
davinci-evals corpus verify
davinci-evals corpus validity [--report <path>]
```

- `corpus verify` loads `verified-core`, builds the report, and calls
  `require_all_verified()`. It prints `verified N behavior scenarios` and exits
  non-zero if any scenario is not `Verified`, including the empty-corpus case.
- `corpus validity` writes a JSON report for `verified-core` and `core-200` and
  prints the same JSON. The committed report is
  `docs/audit/claude-code-parity/implementation/validity-report.json`.

Measured:

```text
cargo run -q -p davinci-evals --bin davinci-evals --offline -- corpus verify
EXIT=0
verified 6 behavior scenarios

cargo run -q -p davinci-evals --bin davinci-evals --offline -- corpus validity \
  --report docs/audit/claude-code-parity/implementation/validity-report.json
EXIT=0
verified-core: total=6 verified=6 structural=0 invalid=0 success=6
core-200:      total=200 verified=0 structural=110 invalid=90 success=0
```

`core-200` findings: 200 `missing_verification_command`, 180
`missing_referenced_file`, 25 `verification_kind_without_oracle`.

## 6. Regression tests

```text
cargo test -p davinci-evals --lib --offline
EXIT=0
test result: ok. 227 passed; 0 failed; 1 ignored
```

Relevant tests:

- `invalid_and_structural_results_are_excluded_from_success_denominators` -
  one verified, one structural and one invalid result produce
  `total_scenarios == 1`, `macro_pass_rate == 1.0`, and two `excluded_scenarios`.
- `mutating_core_scenarios_are_not_success_scored` - the 90 mutating
  `core-200` scenarios have no implied command, and the corpus reports zero
  success-scored scenarios with `require_all_verified()` returning `Err`.
- `verified_core_scenarios_are_all_success_scored` - all six are verified, every
  one requires a file change and a passing `test` oracle.
- `structural_commands_are_not_oracles`, `missing_fixture_is_invalid`,
  `missing_referenced_file_is_invalid`, `no_verification_is_structural_only_not_success`,
  `patch_formatting_is_structural_only`, `duplicate_ids_invalidate_every_occurrence`,
  `unsafe_paths_are_rejected`.
- `corpus_dispatch_is_offline_and_deterministic`,
  `corpus_validity_report_excludes_unscorable_scenarios`.

## 7. Limits

The classifier cannot prove that a command is semantically sufficient. It rejects
known non-oracles and validates that the scenario is executable, and the offline
oracle (document 01) carries the stronger proof for the six verified scenarios.
`core-200` was not rewritten; it is retained as a structural corpus with an
explicit zero success denominator.
