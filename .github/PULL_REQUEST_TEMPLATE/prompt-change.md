# Prompt / Behavioral Change Pull Request

Every PR modifying:
- `crates/davinci-agent/src/prompt/**`,
- Tool descriptions (`crates/davinci-agent/src/tools.rs`),
- Capability routing (`crates/davinci-agent/src/prompt/capabilities.rs`),

must complete the following evidence checklist before review.

---

## Required Evidence

```text
Failure mode:
Affected prompt module(s):
Regression scenario ids:
Stable prompt token delta:
Capability routing delta:
Offline tests:
Live A/B runs:
Correctness delta:
Turn/tool efficiency delta:
Provider-family results:
Rollback profile:
```

---

## Evaluation Evidence Checklist

- [ ] **Targeted Failure Mode**: Names the concrete observed failure mode this change prevents.
- [ ] **Regression Scenario**: Includes at least one scenario in `crates/davinci-evals/fixtures/behavior/core-200.json` or unit test that fails without this change.
- [ ] **Token Budgets**:
  - Stable prefix budget <= 2,800 tokens (`cargo test -p davinci-agent prompt::manifest`)
  - Dynamic runtime state suffix budget <= 500 tokens (`cargo test -p davinci-agent prompt::runtime_state`)
- [ ] **Offline Behavioral Gates**:
  ```bash
  cargo test -p davinci-evals behavior::
  cargo test -p davinci-agent prompt::
  ```
- [ ] **Live A/B Validation** (Required if qualitative judgment or prompt wording changed):
  - Baseline vs Candidate runs documented.
  - Correctness, unverified claims, and unrelated edit rates reported.
- [ ] **Canary Rollout & Rollback**:
  - Change is staged in `PromptProfile::Preview` before graduation to `PromptProfile::Stable`.
  - Instant rollback verified via `--prompt-profile legacy-v1` or `DAVINCI_PROMPT_PROFILE=legacy-v1`.
