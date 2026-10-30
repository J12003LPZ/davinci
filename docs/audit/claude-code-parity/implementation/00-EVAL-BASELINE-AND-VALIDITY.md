# 00 - Evaluation baseline and what was actually valid

Status: implementation evidence.
Baseline revision: `6912f9405d73b24474ad94b74ddb8802ee015d7a` on `J12003LPZ/cc-tui-parity`.
Toolchain: `rustc 1.88.0 (6b00bc388 2025-06-23)`, `cargo 1.88.0 (873a06493 2025-05-10)`.
All commands below ran offline (`--offline`). No live model call and no evaluation
campaign was executed for this change.

## 1. Why the previous success numbers were not success numbers

Before this change, `load_core_200_corpus()` mutated the corpus while loading it:

```rust
// before: crates/davinci-evals/src/behavior/scenario.rs
let mut scenarios: Vec<BehaviorScenario> = serde_json::from_str(&content)?;
for scenario in &mut scenarios {
    let is_mutating = scenario.requirements.iter().any(|requirement| {
        matches!(requirement, BehaviorRequirement::FileChanged { .. })
    });
    if is_mutating && scenario.verification_commands.is_empty() {
        scenario.verification_commands.push(VerificationCommand {
            command: "git diff --check".into(),
            timeout_seconds: 30,
            expected_exit: 0,
        });
    }
}
```

A test then asserted that this was independent verification:

```rust
// before: crates/davinci-evals/src/behavior/scenario.rs
fn mutating_core_scenarios_have_independent_verification() {
    // ...
    assert_eq!(scenario.verification_commands[0].command, "git diff --check");
}
```

`git diff --check` detects whitespace errors in a patch. It does not compile code,
run a test, or check behavior. Treating it as a verification command meant that a
mutating scenario could be reported as a verified success without any executable
oracle, and the corpus test certified the fallback rather than the repair.

The same corpus had a second problem. `core-200.json` references files that its
fixtures do not contain, so many scenarios cannot be executed as written. Measured
after classification (see `validity-report.json`):

| corpus | total | verified | structural-only | invalid | success-scored |
| --- | ---: | ---: | ---: | ---: | ---: |
| `verified-core` | 6 | 6 | 0 | 0 | 6 |
| `core-200` | 200 | 0 | 110 | 90 | 0 |

The `core-200` findings are: 200 `missing_verification_command`, 180
`missing_referenced_file`, and 25 `verification_kind_without_oracle`. No
`core-200` scenario is success-scored.

## 2. The baseline claim, stated precisely

The defensible statement after this change is:

> Six offline scenarios have a fixture, a failing reproducer, a correct solution and
> three negative solution variants, and all six pass the oracle. That is the only
> success-scored corpus in this repository. `core-200` remains loadable for
> structural and trace scoring but contributes zero to any success rate.

The following statements are **not** supported and are not made anywhere in these
documents:

- that DaVinci has a 6/6 coding-task success rate in general;
- that `core-200` previously measured 110 or 200 successful tasks;
- that a model's text claiming success is verification;
- that token, cache, latency or subscription numbers were measured in this wave.

## 3. Where the new boundary lives

| concern | owner | rule |
| --- | --- | --- |
| scenario classification | `behavior/validity.rs` | one classifier, no second opinion |
| offline proof that a scenario can score | `behavior/oracle.rs` | baseline must fail, correct must pass, negatives must fail |
| success denominator | `behavior/runner.rs` | only `ScenarioValidity::Verified` results are aggregated |
| terminal turn status | `davinci-agent/src/lib.rs` | derived from mutation state, never from model text |
| subagent receipt | `davinci-agent/src/subagent_receipt.rs` | derived from host snapshots plus runtime state |
| host receipt publication | `davinci-coding-agent/src/main.rs` | host writes the slot; model JSON cannot |

## 4. Reproduction

```text
cargo run -q -p davinci-evals --bin davinci-evals --offline -- corpus validity \
  --report docs/audit/claude-code-parity/implementation/validity-report.json
cargo run -q -p davinci-evals --bin davinci-evals --offline -- corpus verify
```

Observed:

```text
EXIT=0
verified-core: total=6 verified=6 structural=0 invalid=0 success=6
core-200:      total=200 verified=0 structural=110 invalid=90 success=0
verified 6 behavior scenarios
```

`corpus verify` fails closed through `CorpusValidityReport::require_all_verified()`
if any success-scored scenario is not classified `verified`.

## 5. Consequence for older artifacts

`ScenarioRunResult` gained a `validity` field with
`#[serde(default = "default_legacy_validity")]`, and the default is
`StructuralOnly`. Artifacts written before this change therefore cannot
retroactively claim verified success when they are re-read. This is deliberate:
the old artifact format contains no evidence from which validity could be
reconstructed.
