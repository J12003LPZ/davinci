# DaVinci Prompt Engineering and Failure-Mode Mining Guide

This document outlines the architecture, principles, failure-mode intake workflow, and local telemetry design for DaVinci's behavioral prompt system.

---

## 1. Architectural Principles

DaVinci treats agent prompt text as an executable software subsystem rather than unversioned ad-hoc prose.

### Core Architecture
- **Layered PromptComposer**: System prompts are composed of typed, versioned modules (`PromptModule`).
- **Stable Prefix & Dynamic Suffix**:
  - The stable prefix contains behavioral contracts (identity, autonomy, exploration, scope discipline, change quality, tool strategy, verification, collaboration) and model-family adapters. It is byte-identical across ordinary turns for optimal provider prompt caching.
  - The dynamic suffix is bounded (budget <= 500 tokens) and includes only necessary per-turn state (permission mode, active plan, selected capabilities).
- **Authority vs Judgment**:
  - **Runtime code owns authority**: Plan mode mutations, filesystem sandbox bounds, permission rules, and tool execution barriers are enforced strictly by Rust runtime code.
  - **Prompt code owns judgment**: Exploration depth, precise diff scoping, avoiding unprompted refactoring, and verifying changes before claiming completion are guided by prompt instructions.

### Prompt Identity

Prompt profile answers "which release channel/version is this?" Model policy answers "which model-specific behavior adapter transformed that profile?" These are independent dimensions. The stable hash is the authoritative cache/session identity after both are applied.

For example, GPT-6 Astra can report `stable v2 · gpt6-astra v1` while remaining on the normal stable release channel. The default policy is omitted from compact `/status` output.

### GPT-6 Astra policy mapping

| Astra concern | DaVinci implementation |
| --- | --- |
| Follow-through | `model.astra.autonomy` |
| Instruction conflicts | `model.astra.instruction-priority` |
| Over-reading | `model.astra.exploration` |
| Over-testing | `model.astra.verification` |
| Early stopping / endless work | `model.astra.completion` |
| Skill bloat | Existing explicit skill-body expansion remains progressive |
| API/runtime settings | Model catalog and Responses transport, not prompt prose |

Current Astra model/API details must be rechecked against official OpenAI documentation before changing catalog limits, pricing, reasoning levels, or transport behavior.

---

## 2. Failure-Mode Mining & Intake Pipeline

When a model behavior bug or degradation is observed in real-world usage or dogfooding, developers follow this strict 9-step intake pipeline:

```text
1. Reproduce a bad behavior
      │
2. Save and minimize the behavioral trace
      │
3. Classify the root cause
      │
4. Add a deterministic regression scenario
      │
5. Change the smallest responsible module
      │
6. Run focused deterministic eval suite
      │
7. Run live A/B comparison (if judgment is involved)
      │
8. Canary in preview profile
      │
9. Graduate through release gates into stable profile
```

### The 9 Steps

1. **Reproduce a bad behavior**:
   Capture the exact user request, workspace context, and tool sequence where the agent misbehaved (e.g. edited files before reading relevant code, refactored unrelated files, or claimed tests passed without running them).

2. **Save and minimize the trace**:
   Extract the execution steps into a minimal reproduction trace (`BehaviorTrace`), identifying the specific turn and tool call where the behavioral contract failed.

3. **Classify the root cause**:
   - *Runtime bug*: An enforcement invariant failed in code (e.g. permission check bypassed, plan mode barrier failed). Fix in runtime code (`crates/davinci-agent/src/permission.rs`, `planning.rs`, etc.).
   - *Tool-description bug*: The model misunderstood a tool's parameters or intent. Fix in `crates/davinci-agent/src/tools.rs`.
   - *Prompt bug*: The model lacked clear scope or behavioral guidance. Fix in the specific prompt module in `crates/davinci-agent/src/prompt/`.
   - *Model-specific bug*: The failure only occurs on a specific model family. Fix in `crates/davinci-agent/src/prompt/provider.rs`.
   - *Eval bug*: The evaluation scorer or fixture had ambiguous expectations. Fix in `crates/davinci-evals`.

4. **Add a regression scenario**:
   Encode the scenario in `crates/davinci-evals/fixtures/behavior/core-200.json` or an inline test fixture with explicit required/forbidden tool sequences and semantic expectations.

5. **Change the smallest responsible module**:
   Never bloat the entire prompt. Only modify the specific module responsible (e.g. `coding.rs` for scope discipline, `verification.rs` for test confirmation). Adhere strictly to the token budget (<= 2,800 tokens for stable prompt prefix).

6. **Run focused deterministic suite**:
   Verify that the new scenario passes and all existing deterministic scenarios pass:
   ```bash
   cargo test -p davinci-evals behavior::
   cargo test -p davinci-agent prompt::
   ```

7. **Run live A/B when prompt judgment is involved**:
   If the change modifies qualitative judgment, run the paired A/B runner comparing candidate against baseline:
   ```bash
   cargo run -p davinci-evals -- run-ab --dataset core-200 --candidate preview
   ```

8. **Canary in preview profile**:
   Deploy the candidate prompt in `PromptProfile::Preview`. Allow internal users to opt-in with `--prompt-profile preview` (or `"promptProfile": "preview"` in settings).

9. **Graduate only through release gates**:
   The prompt graduates to `PromptProfile::Stable` only when:
   - PR gate passes with 0 regressions on core behavioral scenarios.
   - Token budget remains within <= 2,800 tokens.
   - Release quality gates in `crates/davinci-evals/src/behavior/gate.rs` pass.

---

## 3. Local-First Privacy-Safe Telemetry

DaVinci collects behavioral reliability metrics locally to monitor prompt performance and identify regressions.

### Privacy Boundary Invariant
Behavioral telemetry NEVER records:
- Raw user prompts
- Model completion text
- Source code or file contents
- Raw file paths outside approved scrubbed telemetry conventions
- Shell command text or terminal output
- API keys, credentials, or secrets

### Telemetry Schema (`BehaviorTelemetry`)
Only aggregate counters and version identifiers are recorded:
- `prompt_profile`: `"stable"`, `"preview"`, or `"legacy-v1"`
- `prompt_version`: e.g. `2`
- `prompt_stable_hash_prefix`: e.g. `"a1b2c3d4"`
- `model_family`: e.g. `"anthropic"`, `"gemini"`, `"openai-reasoning"`
- `model_policy`: e.g. `"default"` or `"gpt6-astra"`
- `model_policy_version`: model-policy schema/version identifier
- `model_turns`: count of model completions
- `tool_calls`: total tool calls
- `permission_prompts`: approvals requested
- `permission_denials`: tool calls denied
- `files_changed_count`: distinct files modified
- `verification_commands_run`: test/build commands run
- `verification_failures`: test/build commands failed
- `aborted`: whether the run was cancelled
- `user_steers`: steering inputs from user

### Inspecting Local Behavior Metrics
Users and developers can inspect local metrics at any time using `/status` in the interactive shell:

```text
Prompt profile: stable v2
Runs: 42
Median turns: 6
Verification failures recovered: 8
Permission prompts/run: 0.9
User steers/run: 0.4
```
