# 07 - Validation evidence

Status: commands executed on 2026-10-28 against revision
`6912f9405d73b24474ad94b74ddb8802ee015d7a` plus the working-tree changes
described in documents 00-06.
Environment: Windows, `rustc 1.88.0`, `cargo 1.88.0`, all Cargo commands
`--offline`.

No live model call, no provider request, no live evaluation campaign and no
network benchmark was run. Every result below is deterministic and local.

## 1. Targeted suites

| command | result |
| --- | --- |
| `cargo test -p davinci-evals --lib --offline behavior::oracle` | 10 passed, 0 failed |
| `cargo test -p davinci-evals --lib --offline` | 227 passed, 0 failed, 1 ignored |
| `cargo test -p davinci-agent --lib --offline completion_status` | 13 passed, 0 failed |
| `cargo test -p davinci-agent --lib --offline subagent_receipt` | 14 passed, 0 failed |
| `cargo test -p davinci-agent --lib --offline subagent` | 74 passed, 0 failed |

Each command exited `0`. The `completion_status` filter matches the 13 G-07
tests; `subagent` also matches the receipt module and the pre-existing subagent
tests, hence 74.

## 2. Full package suites

| command | exit | observed |
| --- | ---: | --- |
| `cargo test -p davinci-agent --offline` | 0 | 2491 passed, 0 failed, 7 ignored across 99 suites; library suite 1894 passed, 0 failed |
| `cargo test -p davinci-evals --offline` | 0 | 265 passed, 0 failed, 1 ignored across 10 suites |
| `cargo test -p davinci-mcp --offline` | 0 | 67 passed, 0 failed across 6 suites |
| `cargo test -p davinci-coding-agent --offline` | 0 | 2683 passed, 0 failed, 49 ignored across 89 suites |

These are the exact package-level commands used; the exit code is the gate. No
test was ignored, removed or marked `#[ignore]` by this change; the ignored
counts above are pre-existing.

## 3. Static checks

```text
cargo fmt --all
cargo fmt --all --check
EXIT=0

cargo clippy --workspace --all-targets --offline
EXIT=0
```

`cargo fmt --all --check` was clean after formatting. Clippy produced no warnings:
an earlier `only_used_in_recursion` warning in
`crates/davinci-evals/src/behavior/oracle.rs` was fixed by removing the unused
`root` parameter from `hash_directory`; the final workspace run is silent.

## 4. Corpus evidence

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

The JSON report is committed next to this document. It contains per-scenario
findings and command counts, not just the summary.

Oracle evidence for the six scenarios:

```text
cargo test -p davinci-evals --lib --offline behavior::oracle
EXIT=0
correct_solution_passes_every_verified_scenario ... ok
unchanged_repository_fails ... ok
syntax_only_change_does_not_verify ... ok
incorrect_solution_does_not_verify ... ok
repeated_runs_are_deterministic ... ok
```

The oracle test rebuilds each fixture in a temporary directory and runs
`cargo test --quiet`; it does not write to the repository.

## 5. What is deliberately not verified

| claim | status |
| --- | --- |
| live model task success or pass@k | not measured; no model call |
| token/cost/latency improvement | not measured; no campaign |
| subscription quota or cache savings | not measured; no provider traffic |
| all 200 `core-200` scenarios repaired | false; 110 structural, 90 invalid |
| workflow-executor subagents have snapshots | false; they pass `receipt_slot: None` |
| a passing check proves semantic correctness | false; it proves the command passed |

No benchmark number is quoted from a model's own output, and no result in this
wave depends on model-authored prose.

## 6. Reproducing from a clean checkout

```text
cargo test -p davinci-agent --offline
cargo test -p davinci-evals --offline
cargo test -p davinci-mcp --offline
cargo test -p davinci-coding-agent --offline
cargo fmt --all --check
cargo clippy --workspace --all-targets --offline
cargo run -q -p davinci-evals --bin davinci-evals --offline -- corpus verify
```

The corpus commands require no network. The oracle tests invoke the local Cargo
toolchain because the fixtures are small Rust crates.
