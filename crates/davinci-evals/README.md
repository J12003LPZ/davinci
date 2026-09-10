# davinci-evals

`davinci-evals` contains the automated evaluation framework and benchmarking harness for Davinci.

---

## Key Capabilities

- **Evaluation Harness (`harness.rs`)**:
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
cargo test -p davinci-evals
cargo test -p davinci-evals behavior::
cargo test -p davinci-evals competitor::
```


