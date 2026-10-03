# davinci-evals

`davinci-evals` contains the automated evaluation framework and benchmarking harness for Davinci.

---

## Key Capabilities

- **Evaluation Harness (`src/lib.rs`, `harness_eval.rs`)**:
  - Executes standardized benchmark tasks against candidate models and configurations.
  - Measures task pass rates, tool selection precision, edit patch accuracy, and token efficiency.
- **Behavioral Reliability & Core-200 Benchmark (`behavior/`)**:
  - Deterministic evaluation of agent judgment across 9 behavioral dimensions (200 scenarios):
    - `exploration`: target identification, callsite discovery, large-file range reading.
    - `scope_discipline`: targeted edits, preserving uncommitted changes, minimal diffs.
    - `verification_integrity`: evidence-before-completion, no false "tests pass" claims.
    - `tool_selection`: picking optimal tools (grep, find, edit, batch, bash).
    - `collaboration`: respecting user guidance, confirming destructive actions.
    - `planning`: read-only Plan Mode contracts and phased milestones.
    - `security_boundary`: resisting prompt injections, respecting deny lists, no credential leak.
    - `frontend_capability`: design intent polish and responsive/visual verification.
    - `anti_overengineering`: YAGNI compliance, no unsolicited abstractions or cascades.
  - Normalized traces (`BehaviorTrace`) and deterministic scoring (`score_trace`).
- **Competitor Differential Harness (`competitor/`)**:
  - Optional isolated runners for external tools like Claude Code (`claude -p`).
  - Strict filesystem isolation using temporary workspaces.
  - Fair comparison disclosures documenting model parity, timeout, and repo snapshot.
- **Reporting & Tables (`reporter.rs`, `harness_table.rs`)**:
  - Generates markdown report cards and terminal tables summarizing benchmark runs.
  - Records execution artifacts for offline inspection.

---

## Running Evals

```bash
cargo run -p davinci-evals -- corpus verify
cargo run -p davinci-evals -- behavior run --suite core-200
cargo run -p davinci-evals -- behavior ab \
  --suite core-200 \
  --baseline-profile stable \
  --candidate-profile preview \
  --provider openai-codex \
  --model gpt-5.6-sol \
  --repeats 3 \
  --davinci-bin ./target/release/davinci \
  --artifacts target/behavior-evals
cargo run -p davinci-evals -- behavior gate --artifacts target/behavior-evals
cargo run -p davinci-evals -- competitor run --binary claude
cargo run -p davinci-evals -- competitor compare --artifacts target/competitor-evals
```

`behavior run` and `behavior ab` execute the release DaVinci binary in an
isolated fixture workspace, persist JSON traces, verification results, diffs,
run dispositions, and content hashes. `behavior ab` also writes the paired
comparison and a manifest suitable for `behavior gate`; infrastructure and
configuration failures are retained as evidence and excluded from the
behavioral denominator. Provider and model values may be supplied by flags or
the `DAVINCI_PROVIDER`/`DAVINCI_MODEL` environment variables. API credentials
are passed only through the runner's explicit environment allow-list.

Competitor execution remains fail-closed until its matched runner and
persisted evidence path are wired.


