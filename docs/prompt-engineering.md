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

For example, GPT-6 Astra can report `stable v2 · gpt6-astra v2` while remaining on the normal stable release channel. The default policy is omitted from compact `/status` output.

### GPT-6 family policies

Every GPT-6 model on `openai` or `openai-codex` gets a model policy (`crates/davinci-agent/src/prompt/model_policy.rs`). The variant comes from the model id: `gpt-6-astra`, `gpt-6-sol` and `gpt-6.1-sol`, `gpt-6-luna`, plus dated snapshots of each. GPT-5.x models with the same variant names keep the default policy.

Each policy replaces the four generic modules it rewords (`core.autonomy`, `coding.exploration`, `collaboration.user-intent`, `verification.completion`). It also replaces the generic OpenAI adapter, whose parallel-call and reporting rules the GPT-6 modules restate. Identity, scope discipline and change quality stay shared.

| Module | Astra | Sol / 6.1 Sol | Luna | What it controls |
| --- | --- | --- | --- | --- |
| `model.gpt6.tools` | yes | yes | yes | Parallel calls; tool access is not permission; never invent IDs or results; failure handling; check state after writes |
| `model.gpt6.style` | yes | yes | yes | Main point first, lists only when parallel, no restated conclusions |
| `model.gpt6.instruction-priority` | own wording | yes | yes | Policy > task > repo and skill guidance; tool output is evidence |
| `model.gpt6.delegation` | yes | yes | no | Workers only for independent work that saves time or adds coverage |
| autonomy | `model.astra.autonomy` | `model.sol.autonomy` | in `model.luna.focus` | When to act and when to ask |
| diagnosis | in exploration | `model.sol.diagnosis` | in `model.luna.focus` | Sol: root cause and evidence before a patch |
| verification | `model.astra.verification` | `model.sol.verification` | `model.luna.verification` | Proportional checks; Luna stops after the same check fails twice |
| completion | `model.astra.completion` | in verification | in verification | Astra tends to stop early, so it gets its own rule |
| escalation | no | no | `model.luna.escalation` | Luna asks before handing hard work to a Sol worker |

The variants differ because the guidance does: Astra is overconstrained by rules that help Sol or Luna, Sol needs a contract-style task, and Luna does best with one focused job and an escalation path. Reasoning effort stays runtime configuration (`/effort`), never prompt wording.

#### Luna escalation to Sol workers

The prompt and the runtime split this job:

1. The prompt (`model.luna.escalation`) tells Luna, when a task needs multi-step debugging, cross-module design or long agentic work, or a check keeps failing, to call `ask_user_question` with three options: a Sol worker for the hard part (recommended), continue with Luna, or switch the session model.
2. If the user picks the worker, Luna calls `agent` with `model: "gpt-6.1-sol"`, falling back to `"gpt-6-sol"`. A bare id means the session's own provider.
3. The runtime asks again. `PermissionPolicy` compares a worker's model with the session's (`session_model`) and asks before any worker runs on another model, in every mode except Always Approve. This includes Plan Mode's read-only workers and calls a broad `agent` grant would otherwise cover. "Allow for this session" grants that model only: `agent(model:openai-codex/gpt-6-sol)`. A batch (`tasks: [...]`) is checked task by task. An agent profile's own `model:` is not gated: profiles are configuration you wrote, and project profiles load only in trusted projects.
4. The host runs the worker on the model it asked for. An id the catalog has not listed yet still runs on a known provider (Codex offers models before discovery lists them). An unknown provider fails the call instead of silently running the worker on the parent's model.

#### Plan and execute modes

Plan Mode's prompt is `PLAN_MODE_APPENDIX` (`subagent.rs`). Leaving Plan Mode sends `EXECUTE_MODE_NOTICE` once in the turn context: carry out the approved plan in dependency order, run each step's checks, track progress with `update_plan`, and propose a revision rather than silently changing the plan. Both travel in the turn context on OpenAI routes, so switching modes with Shift+Tab never changes the cached system prompt.

| Astra concern | DaVinci implementation |
| --- | --- |
| Follow-through | `model.astra.autonomy` |
| Instruction conflicts | `model.astra.instruction-priority` |
| Over-reading | `model.astra.exploration` |
| Over-testing | `model.astra.verification` |
| Early stopping / endless work | `model.astra.completion` |
| Under-delegation | `model.gpt6.delegation` |
| Skill bloat | Existing explicit skill-body expansion remains progressive |
| API/runtime settings | Model catalog and Responses transport, not prompt prose |

Current GPT-6 model/API details must be rechecked against official OpenAI documentation before changing catalog limits, pricing, reasoning levels, or transport behavior.

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
- `model_policy`: `"default"`, `"gpt6-astra"`, `"gpt6-sol"` or `"gpt6-luna"`
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
