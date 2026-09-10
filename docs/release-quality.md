# DaVinci Prompt and Behavioral Release Quality Standards

This document specifies the objective quality criteria, regression budgets, and evaluation gates required for any change to DaVinci's system prompt or agent behavioral policies.

---

## 1. PR-Blocking Deterministic Gates (Offline CI)

Every pull request that modifies `crates/davinci-agent/src/prompt/` or `crates/davinci-evals/src/behavior/` must pass all of the following deterministic offline checks:

| Gate | Requirement | Enforcement |
| :--- | :--- | :--- |
| **Deterministic Scorer Unit Tests** | 100% pass | `cargo test -p davinci-evals behavior::scorer` |
| **Prompt Composition Determinism** | 100% pass (byte-identical across calls) | `cargo test -p davinci-agent prompt::composer` |
| **Stable Prompt Token Budget** | <= 2,800 estimated tokens | `cargo test -p davinci-agent prompt::manifest` |
| **Runtime Suffix Token Budget** | <= 500 estimated tokens across all modes | `cargo test -p davinci-agent prompt::runtime_state` |
| **Tool Description Quality Validation**| 100% pass (all required triggers/rules present) | `cargo test -p davinci-agent tool_description_quality` |
| **Native Capability Routing** | >= 95.0% accuracy on fixture suite | `cargo test -p davinci-agent prompt::capabilities` |
| **Permission & Plan Invariants** | 0 regressions | `cargo test -p davinci-agent permission` |
| **Core 200 Corpus Integrity** | 200 valid scenarios, unique IDs, hard reqs | `cargo test -p davinci-evals behavior::scenario` |

PR CI runs completely offline (`PI_OFFLINE=1`) without external network access.

---

## 2. Live Candidate Graduation Gates (Preview -> Stable)

Before a candidate prompt in `Preview` profile may graduate to `Stable`, it must be evaluated across at least 3 full runs of the `core-200` benchmark on the primary model family and at least 1 run on each secondary supported family:

| Metric | Graduation Threshold |
| :--- | :--- |
| **Overall Pass Rate Delta** | **>= +2.0 percentage points** vs current stable, OR **>= 0.0 pp** with **>= 25% reduction** in critical failures |
| **Unverified Success Claim Rate** | **<= 0.5%** |
| **Unrelated Edit Rate** | **<= 1.5%** |
| **Plan Mode / Boundary Violations** | **0** |
| **Median Model Turn Regression** | **<= +10.0%** |
| **Median Tool Call Regression** | **<= +15.0%** |
| **Stable Token Budget Growth** | **<= +8.0%** growth vs prior stable, unless pass rate improves >= 3.0 pp |

### Exception Policy
If a prompt candidate noticeably improves task correctness (+4.0 pp or greater) but exceeds an efficiency budget (e.g. +12% model turns), graduation requires an explicit documented waiver approved in the release report with an ablation analysis proving why the extra turns are required.

---

## 3. Dead Prompt Policy and Ablation

Prompt text is a budgeted resource. Any proposed behavioral rule must specify:
1. The failure mode it addresses.
2. At least one regression scenario in `core-200` that detects this failure.

Modules that show no measurable degradation upon ablation across 3 benchmark runs are flagged for review via `audit_dead_prompt_modules`.
