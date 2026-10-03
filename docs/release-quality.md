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

Harness optimization changes also require the deterministic offline gate:

```bash
cargo run -p davinci-evals -- optimization gate --offline
```

The gate covers the eight named ablations in `davinci-evals`, persists only
provider usage supplied by a real adapter, and rejects a candidate that loses
baseline correctness. It does not invoke a provider or competitor executable.

### Required protection for `main`

The desired repository policy is:

```text
main:
  require pull request or equivalent protected update path
  require CI / quality
  require Workflow lint / actionlint
  require Security SARIF interoperability / schema
  block force pushes
  block branch deletion
```

The current workflow and job names are `CI / quality`, `Workflow lint /
actionlint`, and `Security SARIF interoperability / schema`. These names are
the stable checks to select when configuring protection or an equivalent
ruleset. Branch protection is a GitHub administration setting, not a repository
file change. It must not be described as enabled without a credentialed API
readback after an authorized administration action.

Main CI must be green before any prompt, native capability, or evaluation feature is
graduated. Workflow syntax and lint validity are therefore release prerequisites,
not advisory checks.

The live behavioral matrix in `.github/workflows/behavior-live.yml` is the
credentialed product-path workflow. It must build the release binaries, run
real `behavior ab` turns, retain raw evidence for 30 days, and report missing
provider configuration explicitly. Offline Rust tests alone do not satisfy
the live A/B or promotion gates. Model-specific prompt-policy candidates use the
same release gates; their model-family regression suite and policy-only live A/B
requirements are defined in [`docs/behavioral-evals.md`](behavioral-evals.md).

Every live run also records its disposition. Infrastructure failures are
excluded from behavioral pass-rate denominators, configuration failures are
reported separately, and a scheduled run fails when infrastructure failures
exceed 10% of all attempted runs.

The manual `.github/workflows/prompt-promotion.yml` workflow checks out the
exact candidate commit, runs the offline workspace suite, binds the evidence
candidate to the supplied Preview and Stable hashes, and invokes
`behavior promote-check` against the content-addressed artifacts. It emits a
certification JSON artifact only after recording whether the gate passed; it
does not push or auto-promote a branch.

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

---

## 4. Maturity Scorecard Vector Integration

Release candidates report a 10-dimensional vector (0–100) as specified in [`docs/behavioral-evals.md`](behavioral-evals.md):

1. **Correctness reliability**: >= 85.0
2. **Exploration discipline**: >= 90.0
3. **Scope precision**: >= 92.0
4. **Verification integrity**: >= 95.0
5. **Tool efficiency**: >= 80.0
6. **Interaction efficiency**: >= 85.0
7. **Cross-model consistency**: >= 80.0
8. **Long-horizon stability**: >= 75.0
9. **Safety-boundary behavior**: 100.0 (Zero tolerance)
10. **Prompt efficiency**: >= 80.0

No candidate may graduate if any dimension drops by more than 5.0 points from the current stable baseline.

### Native capability quality dimensions

Release reports must expose the following typed dimensions independently:

- frontend precision and recall;
- visual verification rate and design diversity;
- debugging reproducer capture, same-signal verification, and symptom-suppression pass rate;
- review trigger precision, critical-defect recall, false-positive rate, and duplicate rate;
- profile product-path coverage;
- successful A/B run count and infrastructure failure rate; and
- Claude matched-run delta, when a matched competitor run is available.

The evaluator’s `CapabilityQualityReport` intentionally does not collapse
these dimensions into an opaque composite score. Missing live or competitor
evidence remains unavailable rather than being represented as a fabricated
result.

---

## 5. External Competitive Comparison Standards

When comparing against Claude Code or other external harnesses:
- Must state the **Evidence Level** (Level 0: Architectural, Level 1: Scenario, Level 2: Category, Level 3: Suite).
- Level 3 suite win requires >= 3 independent runs, >= 150 shared scenarios, DaVinci pass rate >= Competitor + 3.0 pp, unrelated-edit rate <= Competitor, unverified-claim rate <= Competitor, median wall time <= Competitor + 15%, and 0 boundary failures.
- If models or versions differ, label as **product-system comparison** rather than pure harness comparison.
