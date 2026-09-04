# davinci-evals

`davinci-evals` contains the automated evaluation framework and benchmarking harness for Davinci.

---

## Key Capabilities

- **Evaluation Harness (`harness.rs`)**:
  - Executes standardized benchmark tasks against candidate models and configurations.
  - Measures task pass rates, tool selection precision, edit patch accuracy, and token efficiency.
- **Reporting & Tables (`reporter.rs`, `harness_table.rs`)**:
  - Generates markdown report cards and terminal tables summarizing benchmark runs.
  - Records execution artifacts for offline inspection.

---

## Running Evals

```bash
cargo test -p davinci-evals
```
