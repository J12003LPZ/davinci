# OpenAI Subscription Efficiency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make DaVinci spend fewer ChatGPT-plan credits per task than Codex CLI on the same OpenAI model, with equal or better task success, by keeping every request append-only for the prompt cache, replaying the model's own reasoning, using the tool formats the models were trained on, and making usage limits visible.

**Architecture:** Three parts, each its own branch and PR, each shippable alone. Part A removes the cache breakers that live entirely in DaVinci (per-turn system prompt rewrites, injected memory, plan context, pruning, growing tool list). Part B starts with a live probe of the ChatGPT Codex backend, then turns on only the backend features the probe accepted (cache options, per-message native replay, freeform `apply_patch`, an OpenAI prompt adapter). Part C adds cost controls and proof (cache-sharing compaction, usage-limit awareness, cheaper models for side roles, credit estimates, and a live comparison against Codex CLI).

**Tech Stack:** Rust 1.83.0 workspace (`davinci-ai`, `davinci-agent`, `davinci-coding-agent`), `serde_json`, `ureq =2.10.1`, Node for measurement scripts.

## Global Constraints

- Toolchain is pinned to Rust 1.83.0. Add no new dependency. Existing dependencies keep their `=` exact pins.
- Tests are fixture-only and offline. No test may reach a network host. Live calls happen only through the explicit `--codex-probe` command and the measurement scripts, run by a person.
- The compaction and branch prompt constants in `crates/davinci-agent/src/compaction.rs` and `crates/davinci-agent/src/branch.rs` are part of the TypeScript parity contract. Do not reword them. Where a task changes how they are delivered, document the divergence in `docs/openai-efficiency.md`.
- New default behavior applies only when `davinci_agent::prompt::provider::prompt_model_family(provider, model_id)` returns `PromptModelFamily::OpenAiReasoning`. Other provider families keep today's behavior byte for byte.
- Every behavior change has an environment rollback switch, listed in `docs/openai-efficiency.md`.
- Never commit on `main`. One branch and one PR per part. PR text and docs use no em dashes.
- On Windows, commit with `git -c core.hooksPath=NUL commit --no-verify -m "..."` (the repo hook needs `/bin/bash`).
- Use `rtk` for cargo commands (`rtk cargo test ...`).
- Each part ends with the delivery rule from `CLAUDE.md`: release build, back up the installed `davinci.exe`, replace it, verify by SHA-256, smoke-check, tell the user to restart sessions.

---

## Evidence this plan fixes

Each row was read in the code on `origin/main` at `95b82024`.

| # | Problem | Where |
| --- | --- | --- |
| E1 | The system prompt is recomposed every user turn with runtime state (permission mode, plan revision, contract, visual backend) and capability modules chosen from the user's words. Any change rewrites `instructions`, so the whole history is billed uncached. | `crates/davinci-agent/src/lib.rs:979-1038` (`prepare_builtin_prompt_for_user_turn`), `crates/davinci-agent/src/prompt/turn.rs:19-88` |
| E2 | Plan Mode appends `PLAN_MODE_APPENDIX` to the system prompt, so `/plan`, `/act` and Shift+Tab rewrite `instructions`. | `crates/davinci-agent/src/lib.rs:712-718`, `crates/davinci-coding-agent/src/main.rs:1979-1990` |
| E3 | Retrieved memory is inserted before the latest user message and never saved. Next turn it is gone, so the prefix changes at the previous user message and the native replay fingerprint stops matching. | `crates/davinci-coding-agent/src/main.rs:1995-2001`, `crates/davinci-agent/src/lib.rs:1199-1224`, `crates/davinci-coding-agent/src/main.rs:2317-2330` |
| E4 | The living plan text is inserted before the latest user message on every request and changes whenever the plan is revised. | `crates/davinci-agent/src/lib.rs:1199-1224`, `crates/davinci-agent/src/planning.rs:417-440` |
| E5 | Pruning starts at 50% of the window and rewrites old tool results, which forces the rest of the history to be written to the cache again. | `crates/davinci-agent/src/pruning.rs:35-45`, `crates/davinci-agent/src/lib.rs:1727-1760` |
| E6 | The provider tool list grows during a session when `tool_search` activates a deferred tool, which changes the `tools` array at the front of the prefix. | `crates/davinci-agent/src/lib.rs:568-578`, `crates/davinci-agent/src/lib.rs:2222-2255`, `crates/davinci-agent/src/tools.rs:981` |
| E7 | The ChatGPT Codex route never resolves cache capabilities; it gets `OpenAiCacheCapabilities::unknown()` and sends only `prompt_cache_key`. | `crates/davinci-ai/src/stream.rs:1066-1078` |
| E8 | Declared Codex capabilities (explicit breakpoints, prewarm, turn-state headers, phases, grammar tools) are never read by the request builder. | `crates/davinci-ai/src/codex_capabilities.rs:57-80` |
| E9 | The generic Responses input builder drops reasoning items and assistant `phase`. They survive only when the one-record native resume matches, which E3 and E5 break. | `crates/davinci-ai/src/stream.rs:1016-1073` |
| E10 | `apply_patch` is sent as a JSON function with a string argument, not as the freeform tool the Codex models were trained on. The catalog marks `supportsOpenAIGrammarTools: true` but nothing uses it. | `crates/davinci-agent/src/tools.rs:397-409`, `crates/davinci-ai/src/catalogs.json` |
| E11 | No OpenAI-specific prompt guidance: `provider_adapter` returns `None` for every family. | `crates/davinci-agent/src/prompt/provider.rs:52-60` |
| E12 | Compaction sends the whole conversation as a new prompt with `cache_retention: "none"` on the main model, so it is never cached. | `crates/davinci-coding-agent/src/main.rs:969-1090`, `crates/davinci-agent/src/compaction.rs:460-535` |
| E13 | No usage-limit awareness. The 5-hour and weekly windows are not read, shown, or used. | no code |
| E14 | Every side call (compaction, graph classifier, researchers) uses the main model. | `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:423-436` |
| E15 | `text.verbosity` is hard-coded to `low` and `reasoning.summary` to `auto`. | `crates/davinci-ai/src/stream.rs:1138-1143`, `crates/davinci-ai/src/stream.rs:1188-1192` |
| E16 | No live comparison against Codex CLI exists, so "better than Codex" cannot be shown. | `crates/davinci-evals` (offline only) |

Why this order: on a ChatGPT plan, credits are charged per token, cached input costs about 10% of fresh input, and output costs 6 to 8 times fresh input (Codex rate card, OpenAI Help Center article 20001106). One prefix break on a 150k-token conversation costs about as much as ten cached requests. Part A removes the breaks; Part B improves what each request carries; Part C reduces what is spent outside the main loop and proves the result.

## File structure

| File | Part | Responsibility |
| --- | --- | --- |
| `crates/davinci-agent/src/cache_stability.rs` (new) | A | `first_prefix_break` and `wire_body_for_next_request`: the append-only invariant used by every test in this plan. |
| `crates/davinci-agent/src/turn_context.rs` (new) | A | Placement decision, turn-context rendering, and the state recovered from history. Pure functions. |
| `crates/davinci-agent/src/prompt/turn.rs` | A | `split_turn_prompt`: stable instructions versus per-turn state. |
| `crates/davinci-agent/src/lib.rs` | A, B | Agent wiring: stable instructions, `commit_turn_context`, no per-request plan or memory insertion, cache-aware pruning, tool freeze, native items on assistant messages. |
| `crates/davinci-agent/src/planning.rs` | A | `plan_turn_context` returns the plan text with its revision. |
| `crates/davinci-agent/src/pruning.rs` | A | `PruneSettings::cached_route`. |
| `crates/davinci-coding-agent/src/main.rs` | A, B, C | Call sites: commit turn context, freeze tools, settings export, codex probe dispatch, compaction conversation slot. |
| `crates/davinci-coding-agent/src/settings.rs` | A, C | New settings keys. |
| `crates/davinci-ai/src/stream.rs` | A, B, C | Verbosity and summary settings, Codex cache policy, per-message native replay, freeform tools, `tool_choice`, raw probe POST, usage-limit capture. |
| `crates/davinci-ai/src/apply_patch_grammar.rs` (new) | B | The Lark grammar for freeform `apply_patch`. |
| `crates/davinci-ai/src/codex_usage.rs` (new) | C | Parse and hold Codex usage-limit state. |
| `crates/davinci-ai/src/provider_retry.rs` | C | Do not retry `usage_limit_reached`. |
| `crates/davinci-coding-agent/src/codex_probe.rs` (new) | B | Live backend probe command and report. |
| `crates/davinci-agent/src/prompt/provider.rs` | B | OpenAI adapter module. |
| `crates/davinci-agent/src/compaction.rs` | C | `SummarizeConversation` slot on `Summarizer`. |
| `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` | C | Economy role models for `openai-codex`. |
| `crates/davinci-coding-agent/src/native_extensions/mod.rs` | C | Credits and usage in `/cache-status`. |
| `crates/davinci-coding-agent/src/davinci_interactive.rs` | C | Usage-limit warning line after a turn. |
| `scripts/measure-codex-cache.mjs` (new) | A | Reads a session file and prints cache ratio per request. |
| `scripts/compare-codex.mjs` (new) | C | Runs the same tasks in Codex CLI and DaVinci and compares credits and success. |
| `docs/openai-efficiency.md` | A, B, C | Operator guide and rollback switches. |
| `docs/cache/codex-backend-probe.md` (new) | B | Probe results that justify each enabled backend feature. |

---

# Part A: Cache-stable conversation

Branch: `openai-efficiency-a-cache-stable` from `origin/main`.

### Task A1: Append-only invariant helpers

**Files:**
- Create: `crates/davinci-agent/src/cache_stability.rs`
- Modify: `crates/davinci-agent/src/lib.rs` (module list near the other `pub mod` lines at the top)

**Interfaces:**
- Produces: `pub enum PrefixBreak { Instructions, Tools, Input { index: usize }, Shorter }`, `pub fn first_prefix_break(previous: &serde_json::Value, next: &serde_json::Value) -> Option<PrefixBreak>`, `pub fn wire_body_for_next_request(agent: &Agent, model: &davinci_ai::Model) -> serde_json::Value`, `pub fn codex_test_model() -> davinci_ai::Model` (test helper, `#[cfg(test)]`).

- [ ] **Step 1: Write the failing tests**

Create `crates/davinci-agent/src/cache_stability.rs`:

```rust
//! Append-only request invariants for prompt-cache routes. No TypeScript
//! counterpart: pi does not test prefix stability.

use serde_json::Value;

use crate::Agent;

/// Where a later provider request stops extending an earlier one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixBreak {
    Instructions,
    Tools,
    Input { index: usize },
    Shorter,
}

/// The first place `next` does not extend `previous`, or `None` when every
/// cache-relevant part of `previous` is an exact prefix of `next`.
pub fn first_prefix_break(previous: &Value, next: &Value) -> Option<PrefixBreak> {
    if previous.get("instructions") != next.get("instructions") {
        return Some(PrefixBreak::Instructions);
    }
    if previous.get("tools") != next.get("tools") {
        return Some(PrefixBreak::Tools);
    }
    let empty = Vec::new();
    let before = previous
        .get("input")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let after = next.get("input").and_then(Value::as_array).unwrap_or(&empty);
    if after.len() < before.len() {
        return Some(PrefixBreak::Shorter);
    }
    before
        .iter()
        .zip(after)
        .position(|(left, right)| left != right)
        .map(|index| PrefixBreak::Input { index })
}

/// The exact body the agent would send next, built offline.
pub fn wire_body_for_next_request(agent: &Agent, model: &davinci_ai::Model) -> Value {
    let system = agent.provider_system_prompt();
    let tools: Vec<davinci_ai::ToolSpec> = agent
        .provider_tool_specs()
        .into_iter()
        .map(|tool| davinci_ai::ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
            constrained_sampling: None,
        })
        .collect();
    davinci_ai::request_body_with(
        model,
        &agent.messages_for_provider(),
        Some(&system),
        &tools,
        &davinci_ai::StreamOptions {
            thinking_level: Some(agent.thinking_level),
            ..Default::default()
        },
    )
}

#[cfg(test)]
pub(crate) fn codex_test_model() -> davinci_ai::Model {
    davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
        .expect("catalog has openai-codex/gpt-5.6-luna")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_appended_item_is_not_a_break() {
        let first = json!({"instructions": "a", "tools": [], "input": [{"n": 1}]});
        let second = json!({"instructions": "a", "tools": [], "input": [{"n": 1}, {"n": 2}]});
        assert_eq!(first_prefix_break(&first, &second), None);
    }

    #[test]
    fn each_kind_of_break_is_named() {
        let base = json!({"instructions": "a", "tools": [1], "input": [{"n": 1}]});
        assert_eq!(
            first_prefix_break(&base, &json!({"instructions": "b", "tools": [1], "input": [{"n": 1}]})),
            Some(PrefixBreak::Instructions)
        );
        assert_eq!(
            first_prefix_break(&base, &json!({"instructions": "a", "tools": [2], "input": [{"n": 1}]})),
            Some(PrefixBreak::Tools)
        );
        assert_eq!(
            first_prefix_break(&base, &json!({"instructions": "a", "tools": [1], "input": [{"n": 9}]})),
            Some(PrefixBreak::Input { index: 0 })
        );
        assert_eq!(
            first_prefix_break(&base, &json!({"instructions": "a", "tools": [1], "input": []})),
            Some(PrefixBreak::Shorter)
        );
    }

    #[test]
    fn today_a_new_capability_rewrites_instructions() {
        // Documents E1 before the fix. Task A3 flips this for OpenAI routes
        // and keeps it for other families.
        let model = codex_test_model();
        let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
        agent.provider = "openai-codex".into();
        agent.model_id = "gpt-5.6-luna".into();
        agent.turn_context_placement_override =
            Some(crate::turn_context::TurnContextPlacement::SystemPrompt);
        agent.prompt("Add a verbose flag to the parser");
        let first = wire_body_for_next_request(&agent, &model);
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Done."));
        agent.prompt("Diagnose the root cause of this failure.");
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), Some(PrefixBreak::Instructions));
    }
}
```

Add to `crates/davinci-agent/src/lib.rs`, next to the other module declarations:

```rust
pub mod cache_stability;
```

The third test references `turn_context_placement_override`, which Task A2 adds. It is expected to fail to compile until then.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test -p davinci-agent cache_stability --offline`
Expected: compile error `no field turn_context_placement_override on type Agent` and `could not find turn_context in the crate root`.

- [ ] **Step 3: Continue to Task A2**

The field and module come from Task A2. Do not commit a crate that does not compile; commit A1 and A2 together at the end of A2.

### Task A2: Turn-context module and placement

**Files:**
- Create: `crates/davinci-agent/src/turn_context.rs`
- Modify: `crates/davinci-agent/src/lib.rs` (module list; `Agent` struct fields near line 436; `Agent::new` initializer near line 552)

**Interfaces:**
- Consumes: `crate::prompt::provider::{prompt_model_family, PromptModelFamily}`, `crate::prompt::manifest::hash_text(&str) -> String`.
- Produces:
  - `pub const TURN_CONTEXT_CUSTOM_TYPE: &str = "davinci.turn_context";`
  - `pub enum TurnContextPlacement { SystemPrompt, Appended }`
  - `pub fn is_cache_sensitive_route(provider: &str, model_id: &str) -> bool`
  - `pub fn default_placement(provider: &str, model_id: &str) -> TurnContextPlacement`
  - `pub struct TurnContextState { pub state_hash: Option<String>, pub plan_revision: Option<u64>, pub plan_mode: bool }` with `pub fn from_messages(messages: &[davinci_ai::ChatMessage]) -> Self`
  - `pub struct TurnContextInput<'a> { pub runtime_state: &'a str, pub plan_mode_appendix: Option<&'a str>, pub living_plan: Option<(u64, &'a str)>, pub memory: Option<&'a str> }`
  - `pub fn render_turn_context(previous: &TurnContextState, input: &TurnContextInput<'_>) -> Option<(String, TurnContextState)>`
  - Agent field `pub turn_context_placement_override: Option<TurnContextPlacement>` and method `pub fn turn_context_placement(&self) -> TurnContextPlacement`.

- [ ] **Step 1: Write the module with its tests**

Create `crates/davinci-agent/src/turn_context.rs`:

```rust
//! Per-turn harness context delivered as an appended message instead of a
//! system prompt rewrite. No TypeScript counterpart; Codex CLI sends the same
//! kind of state as user-role context items after the stable instructions.

use serde::{Deserialize, Serialize};

use crate::prompt::manifest::hash_text;
use crate::prompt::provider::{prompt_model_family, PromptModelFamily};

pub const TURN_CONTEXT_CUSTOM_TYPE: &str = "davinci.turn_context";

const HEADER: &str = "Harness context for the user request above. It is not a new user request.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnContextPlacement {
    /// Legacy: per-turn state is part of the system prompt.
    SystemPrompt,
    /// Per-turn state is appended to the conversation when it changes.
    Appended,
}

/// Routes whose provider bills a cached prefix at a steep discount, so a
/// prefix rewrite costs more than the tokens it saves.
pub fn is_cache_sensitive_route(provider: &str, model_id: &str) -> bool {
    prompt_model_family(provider, model_id) == PromptModelFamily::OpenAiReasoning
}

/// `DAVINCI_TURN_CONTEXT=system|appended` overrides the family default.
pub fn default_placement(provider: &str, model_id: &str) -> TurnContextPlacement {
    match std::env::var("DAVINCI_TURN_CONTEXT").ok().as_deref() {
        Some("system") => TurnContextPlacement::SystemPrompt,
        Some("appended") => TurnContextPlacement::Appended,
        _ if is_cache_sensitive_route(provider, model_id) => TurnContextPlacement::Appended,
        _ => TurnContextPlacement::SystemPrompt,
    }
}

/// What the model has already been told, recovered from history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnContextState {
    #[serde(default)]
    pub state_hash: Option<String>,
    #[serde(default)]
    pub plan_revision: Option<u64>,
    #[serde(default)]
    pub plan_mode: bool,
}

impl TurnContextState {
    /// The state carried by the newest turn-context message. After a
    /// compaction removes it, the default makes the next turn restate
    /// everything.
    pub fn from_messages(messages: &[davinci_ai::ChatMessage]) -> Self {
        messages
            .iter()
            .rev()
            .find(|message| {
                message.role == "custom"
                    && message.extra.get("customType").and_then(|value| value.as_str())
                        == Some(TURN_CONTEXT_CUSTOM_TYPE)
            })
            .and_then(|message| message.extra.get("details"))
            .and_then(|details| serde_json::from_value(details.clone()).ok())
            .unwrap_or_default()
    }
}

pub struct TurnContextInput<'a> {
    pub runtime_state: &'a str,
    pub plan_mode_appendix: Option<&'a str>,
    pub living_plan: Option<(u64, &'a str)>,
    pub memory: Option<&'a str>,
}

/// The message to append for this turn and the state it establishes, or
/// `None` when nothing changed and there is no memory to add.
pub fn render_turn_context(
    previous: &TurnContextState,
    input: &TurnContextInput<'_>,
) -> Option<(String, TurnContextState)> {
    let plan_mode = input.plan_mode_appendix.is_some();
    let state_hash = hash_text(&format!("{}\0{}", input.runtime_state.trim(), plan_mode));
    let state_changed = previous.state_hash.as_deref() != Some(state_hash.as_str());

    let mut sections = Vec::new();
    if state_changed && !input.runtime_state.trim().is_empty() {
        sections.push(format!(
            "<runtime_state>\n{}\n</runtime_state>",
            input.runtime_state.trim()
        ));
    }
    if state_changed {
        match input.plan_mode_appendix {
            Some(appendix) => sections.push(format!("<plan_mode>\n{}\n</plan_mode>", appendix.trim())),
            None if previous.plan_mode => {
                sections.push("<plan_mode>\nPlan Mode has ended.\n</plan_mode>".to_string())
            }
            None => {}
        }
    }
    let mut plan_revision = previous.plan_revision;
    if let Some((revision, text)) = input.living_plan {
        if previous.plan_revision != Some(revision) {
            sections.push(format!(
                "<living_plan revision=\"{revision}\">\n{}\n</living_plan>",
                text.trim()
            ));
            plan_revision = Some(revision);
        }
    }
    if let Some(memory) = input.memory.map(str::trim).filter(|memory| !memory.is_empty()) {
        sections.push(format!("<memory>\n{memory}\n</memory>"));
    }
    if sections.is_empty() {
        return None;
    }
    let text = format!("<turn_context>\n{HEADER}\n{}\n</turn_context>", sections.join("\n"));
    Some((
        text,
        TurnContextState {
            state_hash: Some(state_hash),
            plan_revision,
            plan_mode,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(state: &'a str, memory: Option<&'a str>) -> TurnContextInput<'a> {
        TurnContextInput {
            runtime_state: state,
            plan_mode_appendix: None,
            living_plan: None,
            memory,
        }
    }

    #[test]
    fn the_first_turn_states_the_runtime_state() {
        let (text, state) =
            render_turn_context(&TurnContextState::default(), &input("Permission mode: Ask.", None))
                .unwrap();
        assert!(text.contains("<runtime_state>\nPermission mode: Ask.\n</runtime_state>"));
        assert!(state.state_hash.is_some());
    }

    #[test]
    fn an_unchanged_state_without_memory_adds_nothing() {
        let (_, state) =
            render_turn_context(&TurnContextState::default(), &input("same", None)).unwrap();
        assert_eq!(render_turn_context(&state, &input("same", None)), None);
    }

    #[test]
    fn memory_is_added_every_turn_but_the_state_only_when_it_changes() {
        let (_, state) =
            render_turn_context(&TurnContextState::default(), &input("same", None)).unwrap();
        let (text, _) = render_turn_context(&state, &input("same", Some("prefers small diffs"))).unwrap();
        assert!(text.contains("<memory>\nprefers small diffs\n</memory>"));
        assert!(!text.contains("<runtime_state>"));
    }

    #[test]
    fn leaving_plan_mode_is_said_once() {
        let entering = TurnContextInput {
            runtime_state: "Permission mode: Plan Mode (read-only).",
            plan_mode_appendix: Some("PLAN APPENDIX"),
            living_plan: None,
            memory: None,
        };
        let (text, state) = render_turn_context(&TurnContextState::default(), &entering).unwrap();
        assert!(text.contains("<plan_mode>\nPLAN APPENDIX\n</plan_mode>"));
        assert!(state.plan_mode);
        let (text, state) = render_turn_context(&state, &input("Permission mode: Ask.", None)).unwrap();
        assert!(text.contains("Plan Mode has ended."));
        assert!(!state.plan_mode);
    }

    #[test]
    fn a_plan_revision_is_sent_once() {
        let with_plan = TurnContextInput {
            runtime_state: "s",
            plan_mode_appendix: None,
            living_plan: Some((3, "step 1")),
            memory: None,
        };
        let (text, state) = render_turn_context(&TurnContextState::default(), &with_plan).unwrap();
        assert!(text.contains("<living_plan revision=\"3\">"));
        assert_eq!(render_turn_context(&state, &with_plan), None);
    }

    #[test]
    fn state_is_recovered_from_the_newest_turn_context_message() {
        let mut message = davinci_ai::ChatMessage::text("custom", "x");
        message.extra.insert("customType".into(), TURN_CONTEXT_CUSTOM_TYPE.into());
        message.extra.insert(
            "details".into(),
            serde_json::json!({"stateHash": "h", "planRevision": 2, "planMode": true}),
        );
        let state = TurnContextState::from_messages(&[message]);
        assert_eq!(state.state_hash.as_deref(), Some("h"));
        assert_eq!(state.plan_revision, Some(2));
        assert!(state.plan_mode);
    }

    #[test]
    fn only_openai_reasoning_routes_default_to_appended() {
        assert!(is_cache_sensitive_route("openai-codex", "gpt-5.6-luna"));
        assert!(!is_cache_sensitive_route("anthropic", "claude-opus-4-5"));
    }
}
```

Check that `hash_text` is public: `crates/davinci-agent/src/prompt/manifest.rs` must export `pub fn hash_text(text: &str) -> String`. It is used by `prompt/turn.rs`, so it is at least `pub(crate)`; `pub(crate)` is enough here.

- [ ] **Step 2: Add the module and the Agent field**

In `crates/davinci-agent/src/lib.rs` add `pub mod turn_context;` beside `pub mod cache_stability;`.

Add to `pub struct Agent` (next to `ephemeral_context: Vec<ChatMessage>,` near line 436):

```rust
    /// Tests and hosts may force a placement; `None` follows the route.
    pub turn_context_placement_override: Option<turn_context::TurnContextPlacement>,
    /// Per-turn prompt state prepared for the next `commit_turn_context`.
    turn_state_pending: Option<String>,
```

Add to the `Agent::new` initializer (next to `ephemeral_context: Vec::new(),` near line 552):

```rust
            turn_context_placement_override: None,
            turn_state_pending: None,
```

Add this method to `impl Agent` next to `set_ephemeral_context`:

```rust
    pub fn turn_context_placement(&self) -> turn_context::TurnContextPlacement {
        self.turn_context_placement_override
            .unwrap_or_else(|| turn_context::default_placement(&self.provider, &self.model_id))
    }
```

- [ ] **Step 3: Run the tests**

Run: `rtk cargo test -p davinci-agent turn_context --offline`
Expected: 7 passed.

Run: `rtk cargo test -p davinci-agent cache_stability --offline`
Expected: 3 passed (`today_a_new_capability_rewrites_instructions` passes because the override forces `SystemPrompt`).

- [ ] **Step 4: Commit**

```bash
git add crates/davinci-agent/src/cache_stability.rs crates/davinci-agent/src/turn_context.rs crates/davinci-agent/src/lib.rs
git -c core.hooksPath=NUL commit --no-verify -m "test(cache): add append-only request invariant and turn-context module"
```

### Task A3: Stable instructions, per-turn state appended

**Files:**
- Modify: `crates/davinci-agent/src/prompt/turn.rs` (extract `session_appends`, add `split_turn_prompt`)
- Modify: `crates/davinci-agent/src/lib.rs:712-718` (`reset_system_prompt_to_base`), `:979-1038` and `:1040-1106` (both prepare functions), new `commit_turn_context`
- Modify: `crates/davinci-agent/src/planning.rs` (add `plan_turn_context`)
- Modify: `crates/davinci-coding-agent/src/main.rs:1979-2001`
- Test: `crates/davinci-agent/src/cache_stability.rs` (new scenario tests)

**Interfaces:**
- Consumes: Task A2 items.
- Produces: `pub struct TurnPromptParts { pub instructions: String, pub turn_state: String }`, `pub fn split_turn_prompt(session: &PromptSessionState, composed: &ComposedPrompt) -> TurnPromptParts` in `prompt::turn`; `Agent::commit_turn_context(&mut self, memory: Option<String>)`; `Agent::plan_turn_context(&self) -> Option<(u64, String)>` (crate-visible).

- [ ] **Step 1: Write the failing scenario tests**

Append to the `tests` module in `crates/davinci-agent/src/cache_stability.rs`:

```rust
    fn appended_codex_agent() -> (Agent, davinci_ai::Model) {
        let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
        agent.provider = "openai-codex".into();
        agent.model_id = "gpt-5.6-luna".into();
        agent.turn_context_placement_override =
            Some(crate::turn_context::TurnContextPlacement::Appended);
        (agent, codex_test_model())
    }

    fn user_turn(agent: &mut Agent, text: &str, memory: Option<&str>) {
        agent.prompt(text);
        agent.commit_turn_context(memory.map(str::to_string));
    }

    #[test]
    fn a_new_capability_no_longer_rewrites_instructions() {
        let (mut agent, model) = appended_codex_agent();
        user_turn(&mut agent, "Add a verbose flag to the parser", None);
        let first = wire_body_for_next_request(&agent, &model);
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Done."));
        user_turn(&mut agent, "Diagnose the root cause of this failure.", None);
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
        let last = second["input"].as_array().unwrap().last().unwrap().to_string();
        assert!(last.contains("<turn_context>"), "{last}");
    }

    #[test]
    fn plan_mode_round_trip_keeps_the_prefix() {
        let (mut agent, model) = appended_codex_agent();
        user_turn(&mut agent, "Look at the parser", None);
        let first = wire_body_for_next_request(&agent, &model);
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Read it."));
        agent.set_plan_mode(true);
        user_turn(&mut agent, "Plan the change", None);
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
        assert!(second.to_string().contains(crate::PLAN_MODE_APPENDIX.trim().lines().next().unwrap()));
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Plan ready."));
        agent.set_plan_mode(false);
        user_turn(&mut agent, "Go ahead", None);
        let third = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&second, &third), None);
    }

    #[test]
    fn injected_memory_stays_where_it_was_injected() {
        let (mut agent, model) = appended_codex_agent();
        user_turn(&mut agent, "Fix the parser", Some("user prefers small diffs"));
        let first = wire_body_for_next_request(&agent, &model);
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Fixed."));
        user_turn(&mut agent, "Now the lexer", Some("lexer lives in src/lex.rs"));
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
    }

    #[test]
    fn other_families_keep_per_turn_state_in_the_system_prompt() {
        let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
        agent.provider = "anthropic".into();
        agent.model_id = "claude-opus-4-5".into();
        agent.prompt("Diagnose the root cause of this failure.");
        assert!(agent.system_prompt.contains("Permission mode:"));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-agent cache_stability --offline`
Expected: compile error `no method named commit_turn_context`.

- [ ] **Step 3: Split the composed prompt**

In `crates/davinci-agent/src/prompt/turn.rs`, replace the inline appends computation inside `compose_turn_prompt` (the `let appends = session.append_text...join("\n\n");` block) with a call to a new function, and add `split_turn_prompt`:

```rust
/// User `--append-system-prompt` text. Stable for the session.
pub(crate) fn session_appends(session: &PromptSessionState) -> String {
    session
        .append_text
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The stable instructions and the per-turn state of a composed turn prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnPromptParts {
    /// Stable modules plus user appends: the provider `instructions`.
    pub instructions: String,
    /// Runtime state and capability modules for this turn.
    pub turn_state: String,
}

pub fn split_turn_prompt(session: &PromptSessionState, composed: &ComposedPrompt) -> TurnPromptParts {
    let appends = session_appends(session);
    let turn_state = if appends.is_empty() {
        composed.dynamic_text.clone()
    } else {
        composed
            .dynamic_text
            .strip_suffix(appends.as_str())
            .map(|rest| rest.trim_end().to_string())
            .unwrap_or_else(|| composed.dynamic_text.clone())
    };
    let instructions = match (composed.stable_text.is_empty(), appends.is_empty()) {
        (_, true) => composed.stable_text.clone(),
        (true, false) => appends,
        (false, false) => format!("{}\n\n{}", composed.stable_text, appends),
    };
    TurnPromptParts {
        instructions,
        turn_state,
    }
}
```

and inside `compose_turn_prompt` use `let appends = session_appends(session);`.

Add a unit test to the `tests` module of `turn.rs`:

```rust
    #[test]
    fn split_keeps_appends_in_the_instructions() {
        let mut session = PromptSessionState::builtin(PromptProfile::Stable);
        session.append_text.push("Always answer in English.".into());
        let ctx = PromptContext {
            provider: "openai-codex",
            model_id: "gpt-5.6-luna",
            permission_mode: PermissionMode::Ask,
            plan_active: false,
        };
        let state = RuntimePromptState {
            permission_mode: PermissionMode::Ask,
            plan_revision: None,
            plan_approved: false,
            active_contract: false,
            visual_verification_available: false,
        };
        let caps = CapabilityDecision { capabilities: Vec::new(), reasons: Vec::new(), evidence: Vec::new() };
        let composed = compose_turn_prompt(&session, &ctx, &caps, &state).unwrap();
        let parts = split_turn_prompt(&session, &composed);
        assert!(parts.instructions.ends_with("Always answer in English."));
        assert!(parts.turn_state.contains("Permission mode: Ask."));
        assert!(!parts.turn_state.contains("Always answer in English."));
        assert!(!parts.instructions.contains("Permission mode:"));
    }
```

If `append_text` is not `pub` on `PromptSessionState`, use the existing setter that `--append-system-prompt` uses (search `append_text` in `prompt/session.rs`).

- [ ] **Step 4: Use the parts in the agent**

In `crates/davinci-agent/src/lib.rs`, in `prepare_builtin_prompt_for_user_turn`, replace

```rust
        self.system_prompt = composed.text.clone();
        self.base_system_prompt = composed.text.clone();
```

with

```rust
        self.apply_composed_turn_prompt(&composed);
```

Do the same in `prepare_builtin_prompt_for_user_turn_batch`. Add the helper to `impl Agent`:

```rust
    fn apply_composed_turn_prompt(&mut self, composed: &prompt::composer::ComposedPrompt) {
        match self.turn_context_placement() {
            turn_context::TurnContextPlacement::SystemPrompt => {
                self.system_prompt = composed.text.clone();
                self.base_system_prompt = composed.text.clone();
                self.turn_state_pending = None;
            }
            turn_context::TurnContextPlacement::Appended => {
                let parts = prompt::turn::split_turn_prompt(&self.prompt_session, composed);
                self.system_prompt = parts.instructions.clone();
                self.base_system_prompt = parts.instructions;
                self.turn_state_pending = Some(parts.turn_state);
            }
        }
    }
```

Change `reset_system_prompt_to_base` so the appendix stays out of the instructions on appended routes:

```rust
    pub fn reset_system_prompt_to_base(&mut self) {
        self.system_prompt = self.base_system_prompt.clone();
        if self.plan_mode()
            && self.turn_context_placement() == turn_context::TurnContextPlacement::SystemPrompt
        {
            self.system_prompt.push_str("\n\n");
            self.system_prompt.push_str(crate::PLAN_MODE_APPENDIX);
        }
    }
```

In `crates/davinci-agent/src/planning.rs`, next to `plan_provider_context`, add:

```rust
    pub(crate) fn plan_turn_context(&self) -> Option<(u64, String)> {
        let revision = self
            .tool_context
            .living_plan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revision;
        self.plan_provider_context().map(|text| (revision, text))
    }
```

Add `commit_turn_context` to `impl Agent` in `lib.rs`:

```rust
    /// Append this turn's harness context after the user message. Only a
    /// fresh user turn gets one, and only what changed is restated.
    pub fn commit_turn_context(&mut self, memory: Option<String>) {
        if self.turn_context_placement() != turn_context::TurnContextPlacement::Appended {
            return;
        }
        if self.messages.last().map(|message| message.role.as_str()) != Some("user") {
            return;
        }
        let previous = turn_context::TurnContextState::from_messages(&self.messages);
        let runtime_state = self.turn_state_pending.clone().unwrap_or_default();
        let plan = self.plan_turn_context();
        let input = turn_context::TurnContextInput {
            runtime_state: &runtime_state,
            plan_mode_appendix: self.is_plan_mode().then_some(crate::PLAN_MODE_APPENDIX),
            living_plan: plan.as_ref().map(|(revision, text)| (*revision, text.as_str())),
            memory: memory.as_deref(),
        };
        if let Some((text, state)) = turn_context::render_turn_context(&previous, &input) {
            self.record_custom_message(&serde_json::json!({
                "customType": turn_context::TURN_CONTEXT_CUSTOM_TYPE,
                "content": text,
                "display": false,
                "details": state,
            }));
        }
    }
```

- [ ] **Step 5: Call it from the host**

In `crates/davinci-coding-agent/src/main.rs`, the extension-prompt branch at lines 1979-1990 re-appends the plan appendix. Guard it:

```rust
            if agent.is_plan_mode()
                && agent.turn_context_placement()
                    == davinci_agent::turn_context::TurnContextPlacement::SystemPrompt
                && !agent
                    .system_prompt
                    .contains(davinci_agent::PLAN_MODE_APPENDIX)
            {
```

Replace the memory block at lines 1995-2001:

```rust
        let suppress_memory = std::env::var_os("PI_GRAPH_SUPPRESS_MEMORY_INJECT").is_some()
            || std::env::var_os("PI_GRAPH_ROLE").is_some();
        let memory = if suppress_memory {
            None
        } else {
            host.native_memory_inject(&prompt)
        };
        match agent.turn_context_placement() {
            davinci_agent::turn_context::TurnContextPlacement::Appended => {
                agent.commit_turn_context(memory);
            }
            davinci_agent::turn_context::TurnContextPlacement::SystemPrompt => {
                if let Some(memory) = memory {
                    agent.set_ephemeral_context(vec![davinci_ai::ChatMessage::text(
                        "custom", memory,
                    )]);
                }
            }
        }
```

- [ ] **Step 6: Run the tests**

Run: `rtk cargo test -p davinci-agent cache_stability --offline`
Expected: 7 passed.

Run: `rtk cargo test -p davinci-agent --offline`
Expected: all pass. Tests that assert the plan appendix in `system_prompt` build agents with `Agent::new("x")`, whose empty provider maps to `Generic` and keeps `SystemPrompt`. If a test builds an `openai-codex` agent and asserts the appendix in `system_prompt`, set `turn_context_placement_override = Some(SystemPrompt)` in that test; do not change the assertion.

Run: `rtk cargo test -p davinci-coding-agent --offline`
Expected: all pass. Fix any `openai-codex` fixture test the same way.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-agent/src crates/davinci-coding-agent/src/main.rs
git -c core.hooksPath=NUL commit --no-verify -m "fix(cache): keep instructions stable and append per-turn state on OpenAI routes"
```

### Task A4: Living plan no longer inserted per request

**Files:**
- Modify: `crates/davinci-agent/src/lib.rs:1199-1224` (`legacy_messages_for_provider`)
- Test: `crates/davinci-agent/src/cache_stability.rs`

**Interfaces:**
- Consumes: Task A3 (`plan_turn_context` sends the plan once per revision at turn start).

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_plan_revision_mid_turn_keeps_the_prefix() {
        let (mut agent, model) = appended_codex_agent();
        agent.set_plan_mode(true);
        user_turn(&mut agent, "Plan the parser change", None);
        agent
            .tool_context
            .living_plan
            .lock()
            .unwrap()
            .revision = 1;
        let first = wire_body_for_next_request(&agent, &model);
        agent
            .tool_context
            .living_plan
            .lock()
            .unwrap()
            .revision = 2;
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Revised the plan."));
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
    }
```

If `tool_context` or `living_plan` is not visible from this module, add a `#[cfg(test)] pub(crate) fn set_plan_revision_for_test(&self, revision: u64)` to `impl Agent` in `planning.rs` and use it.

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p davinci-agent a_plan_revision_mid_turn --offline`
Expected: FAIL with `left: Some(Input { index: .. })`.

- [ ] **Step 3: Skip per-request plan insertion on appended routes**

In `legacy_messages_for_provider`, change the first statement to:

```rust
        let plan_context = (self.turn_context_placement()
            == turn_context::TurnContextPlacement::SystemPrompt)
            .then(|| self.plan_provider_context())
            .flatten()
            .map(|text| ChatMessage::text("custom", text));
```

On appended routes the plan reaches the model through the turn-context message (once per revision) and through the `propose_plan` tool result that changed it.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-agent --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src
git -c core.hooksPath=NUL commit --no-verify -m "fix(cache): send the living plan once per revision on OpenAI routes"
```

### Task A5: Cache-aware pruning

**Files:**
- Modify: `crates/davinci-agent/src/pruning.rs` (add `cached_route`, `from_profile`)
- Modify: `crates/davinci-agent/src/lib.rs:1727-1737` (`prune_context`)

**Interfaces:**
- Produces: `impl PruneSettings { pub fn cached_route() -> Self; pub fn for_route(base: &PruneSettings, cache_sensitive: bool) -> PruneSettings }`.

Reasoning to keep in the doc comment: a prune pass rewrites everything after the first pruned result. With cached input at about 10% of fresh and cache writes at up to 1.25 times fresh, frequent small passes cost more than they save. Fewer, larger passes pay back after roughly 25 requests. `DAVINCI_PRUNE_PROFILE` lets the live measurement in Task A8 tune this.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module of `pruning.rs`:

```rust
    #[test]
    fn cached_routes_prune_later_and_deeper() {
        let settings = PruneSettings::cached_route();
        assert!(settings.enabled);
        assert_eq!(settings.start_fraction, 0.65);
        assert_eq!(settings.target_fraction, 0.35);
    }

    #[test]
    fn a_custom_setting_is_never_overridden() {
        let custom = PruneSettings { keep_recent: 2, ..PruneSettings::default() };
        assert_eq!(PruneSettings::for_route(&custom, true), custom);
    }

    #[test]
    fn default_settings_follow_the_route() {
        assert_eq!(
            PruneSettings::for_route(&PruneSettings::default(), true),
            PruneSettings::cached_route()
        );
        assert_eq!(
            PruneSettings::for_route(&PruneSettings::default(), false),
            PruneSettings::default()
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-agent pruning --offline`
Expected: compile error `no function or associated item named cached_route`.

- [ ] **Step 3: Implement**

Add to `pruning.rs`:

```rust
impl PruneSettings {
    /// For routes whose cached prefix is billed at a steep discount: prune in
    /// fewer, larger passes, because each pass rewrites the cached suffix.
    pub fn cached_route() -> Self {
        Self {
            start_fraction: 0.65,
            target_fraction: 0.35,
            ..Self::default()
        }
    }

    /// `DAVINCI_PRUNE_PROFILE=default|cached|off` overrides the route. A
    /// caller that changed any field keeps its own settings.
    pub fn for_route(base: &PruneSettings, cache_sensitive: bool) -> PruneSettings {
        if *base != PruneSettings::default() {
            return *base;
        }
        match std::env::var("DAVINCI_PRUNE_PROFILE").ok().as_deref() {
            Some("off") => PruneSettings { enabled: false, ..*base },
            Some("default") => *base,
            Some("cached") => PruneSettings::cached_route(),
            _ if cache_sensitive => PruneSettings::cached_route(),
            _ => *base,
        }
    }
}
```

In `prune_context` in `lib.rs`, replace `&self.prune_settings,` with:

```rust
            &pruning::PruneSettings::for_route(
                &self.prune_settings,
                turn_context::is_cache_sensitive_route(&self.provider, &self.model_id),
            ),
```

Check that `plan_prune` returns an empty plan when `settings.enabled` is false; if it does not, add `if !settings.enabled { return Vec::new(); }` at its top with a test.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-agent --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/pruning.rs crates/davinci-agent/src/lib.rs
git -c core.hooksPath=NUL commit --no-verify -m "fix(cache): prune in fewer, larger passes on cached OpenAI routes"
```

### Task A6: Fixed tool list for the session

**Files:**
- Modify: `crates/davinci-agent/src/lib.rs` (new `freeze_tools_for_cache` next to `expose_active_tools` near line 2186)
- Modify: `crates/davinci-coding-agent/src/main.rs` (call it in the Appended arm from Task A3 Step 5)
- Test: `crates/davinci-agent/src/cache_stability.rs`

**Interfaces:**
- Produces: `Agent::freeze_tools_for_cache(&self)`.

Trade-off to record in the doc comment: exposing every authorized tool adds its schema once, and later it is read from the cache at about 10% cost. One tool-list change forces the whole conversation to be written again.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn tool_search_does_not_change_the_tool_list_on_cached_routes() {
        let (mut agent, model) = appended_codex_agent();
        agent.set_runtime(crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        ));
        agent.freeze_tools_for_cache();
        user_turn(&mut agent, "Search the web for the changelog", None);
        let first = wire_body_for_next_request(&agent, &model);
        crate::execute_tool_with(
            std::path::Path::new("."),
            "tool_search",
            &serde_json::json!({"query": "web_search"}),
            &agent.tool_context,
        )
        .unwrap();
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Found it."));
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
        assert!(first["tools"].to_string().contains("web_search"));
    }
```

Use the same import paths as `tool_search_activates_authorized_deferred_schema` in `lib.rs:5701` (`RuntimeHandle`, `RunId`, `AgentId`, `RuntimeBus`, `execute_tool_with`).

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p davinci-agent tool_search_does_not_change --offline`
Expected: compile error `no method named freeze_tools_for_cache`.

- [ ] **Step 3: Implement**

```rust
    /// On cached routes, expose every authorized tool from the first request
    /// so a later `tool_search` activation cannot change the tool list.
    pub fn freeze_tools_for_cache(&self) {
        if self.turn_context_placement() != turn_context::TurnContextPlacement::Appended {
            return;
        }
        self.expose_active_tools();
        let authorized = self
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(runtime) = &self.runtime {
            let mut exposure = self
                .tool_context
                .tool_exposure
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for capability in runtime.capability_registry.list() {
                if capability.schema.is_some() {
                    exposure.activate_authorized(
                        &capability.name,
                        authorized.contains(&capability.name),
                    );
                }
            }
        }
    }
```

If `web_search` is a registry capability that is not in `self.tools`, `authorized.contains` is false and the test fails. In that case read how `tools.rs:981` authorizes registry capabilities (it passes `true`) and use the same authorization rule here. Do not widen authorization beyond what `tool_search` already grants.

In `main.rs`, in the Appended arm from Task A3 Step 5, call `agent.freeze_tools_for_cache();` before `agent.commit_turn_context(memory);`.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-agent --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src crates/davinci-coding-agent/src/main.rs
git -c core.hooksPath=NUL commit --no-verify -m "fix(cache): expose the full tool list once on cached OpenAI routes"
```

### Task A7: Verbosity and reasoning summary settings

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs:1138-1143` and `:1188-1192`
- Modify: `crates/davinci-coding-agent/src/settings.rs` (two fields near `transport` at line 315)
- Modify: `crates/davinci-coding-agent/src/main.rs:835-850` (export to env, same pattern as `PI_WEBSOCKET_CONNECT_TIMEOUT_MS`)

**Interfaces:**
- Produces: `fn openai_verbosity() -> &'static str` and `fn reasoning_summary() -> Option<&'static str>` in `stream.rs`; settings keys `openaiVerbosity` and `reasoningSummary`; env `DAVINCI_OPENAI_VERBOSITY` and `DAVINCI_REASONING_SUMMARY`.

- [ ] **Step 1: Write the failing tests**

Add to the tests in `stream.rs` (they are pure; they read a parameter, not the environment):

```rust
    #[test]
    fn verbosity_accepts_only_known_values() {
        assert_eq!(super::verbosity_from(Some("medium")), "medium");
        assert_eq!(super::verbosity_from(Some("loud")), "low");
        assert_eq!(super::verbosity_from(None), "low");
    }

    #[test]
    fn a_none_summary_omits_the_field() {
        assert_eq!(super::summary_from(Some("none")), None);
        assert_eq!(super::summary_from(Some("concise")), Some("concise"));
        assert_eq!(super::summary_from(None), Some("auto"));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-ai verbosity_accepts --offline`
Expected: compile error `cannot find function verbosity_from`.

- [ ] **Step 3: Implement**

In `stream.rs`:

```rust
fn verbosity_from(value: Option<&str>) -> &'static str {
    match value {
        Some("medium") => "medium",
        Some("high") => "high",
        _ => "low",
    }
}

fn summary_from(value: Option<&str>) -> Option<&'static str> {
    match value {
        Some("none") => None,
        Some("concise") => Some("concise"),
        Some("detailed") => Some("detailed"),
        _ => Some("auto"),
    }
}

fn openai_verbosity() -> &'static str {
    verbosity_from(std::env::var("DAVINCI_OPENAI_VERBOSITY").ok().as_deref())
}

fn reasoning_summary() -> Option<&'static str> {
    summary_from(std::env::var("DAVINCI_REASONING_SUMMARY").ok().as_deref())
}
```

Replace `body["text"] = serde_json::json!({"verbosity": "low"});` with `body["text"] = serde_json::json!({"verbosity": openai_verbosity()});`.

Replace the reasoning block body:

```rust
            if let Some(effort) = mapped {
                let mut reasoning = serde_json::json!({ "effort": effort });
                if let Some(summary) = reasoning_summary() {
                    reasoning["summary"] = Value::String(summary.into());
                }
                body["reasoning"] = reasoning;
            }
```

In `settings.rs`, beside `transport`:

```rust
    #[serde(default, rename = "openaiVerbosity")]
    pub openai_verbosity: Option<String>,
    #[serde(default, rename = "reasoningSummary")]
    pub reasoning_summary: Option<String>,
```

In `main.rs`, after the `PI_WEBSOCKET_CONNECT_TIMEOUT_MS` export:

```rust
    if let Some(value) = settings.openai_verbosity.as_deref() {
        std::env::set_var("DAVINCI_OPENAI_VERBOSITY", value);
    }
    if let Some(value) = settings.reasoning_summary.as_deref() {
        std::env::set_var("DAVINCI_REASONING_SUMMARY", value);
    }
```

Graph workers inherit the environment, so they follow the same settings.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-ai --offline` then `rtk cargo test -p davinci-coding-agent settings --offline`
Expected: all pass. If a settings round-trip test lists every key, add the two keys there.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src/stream.rs crates/davinci-coding-agent/src/settings.rs crates/davinci-coding-agent/src/main.rs
git -c core.hooksPath=NUL commit --no-verify -m "feat(openai): make verbosity and reasoning summary configurable"
```

### Task A8: Measure, document, deliver Part A

**Files:**
- Create: `scripts/measure-codex-cache.mjs`
- Modify: `docs/openai-efficiency.md` (new section "Cache-stable turns" and rollback switches)

- [ ] **Step 1: Write the measurement script**

Create `scripts/measure-codex-cache.mjs`:

```js
// Prints provider-reported cache use for each assistant request in a DaVinci
// session file. Usage: node scripts/measure-codex-cache.mjs <session.jsonl>
import { readFileSync } from "node:fs";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/measure-codex-cache.mjs <session.jsonl>");
  process.exit(2);
}
let totalFresh = 0;
let totalRead = 0;
let totalWrite = 0;
let request = 0;
for (const line of readFileSync(file, "utf8").split("\n")) {
  if (!line.trim()) continue;
  let entry;
  try {
    entry = JSON.parse(line);
  } catch {
    continue;
  }
  const message = entry.message;
  if (!message || message.role !== "assistant" || !message.usage) continue;
  request += 1;
  const fresh = message.usage.input ?? 0;
  const read = message.usage.cacheRead ?? 0;
  const write = message.usage.cacheWrite ?? 0;
  totalFresh += fresh;
  totalRead += read;
  totalWrite += write;
  const raw = fresh + read + write;
  const ratio = raw > 0 ? ((read / raw) * 100).toFixed(1) + "%" : "n/a";
  console.log(`#${request} fresh=${fresh} read=${read} write=${write} readRatio=${ratio}`);
}
const raw = totalFresh + totalRead + totalWrite;
console.log(
  `total requests=${request} fresh=${totalFresh} read=${totalRead} write=${totalWrite} readRatio=${
    raw > 0 ? ((totalRead / raw) * 100).toFixed(1) + "%" : "n/a"
  }`
);
```

- [ ] **Step 2: Run the live measurement before and after (a person runs this; it spends a small amount of plan credit)**

In a throwaway repository with a few source files, run the same five turns twice: once with the installed build from before Part A, once with the Part A build. Use one session file per run.

```powershell
$s = "$env:TEMP\cache-a.jsonl"
davinci -p --session $s --model openai-codex/gpt-5.6-luna "Read src and list the modules."
davinci -p --session $s --model openai-codex/gpt-5.6-luna "Diagnose the root cause of this failure: the parser test fails on empty input."
davinci -p --session $s --model openai-codex/gpt-5.6-luna --permission-mode read-only "Plan a fix."
davinci -p --session $s --model openai-codex/gpt-5.6-luna --permission-mode auto "Apply the plan."
davinci -p --session $s --model openai-codex/gpt-5.6-luna "Review the diff you just made."
node scripts/measure-codex-cache.mjs $s
```

Expected after Part A: from request 2 on, the read ratio of the first request of each new turn is close to the previous request's raw input divided by its own raw input (the prefix was reused). Before Part A, the turns that change capability or mode show a read ratio near the size of the stable system prompt only. Record both tables in `docs/openai-efficiency.md`.

- [ ] **Step 3: Document**

Add to `docs/openai-efficiency.md` a section "Cache-stable turns" that explains:
- On `OpenAiReasoning` routes the instructions hold only stable modules and user appends; runtime state, plan mode, living plan and memory are appended after the user message as a hidden `davinci.turn_context` message when they change.
- Pruning uses the cached profile (start 65%, target 35%).
- The whole authorized tool list is sent from the first request.
- Rollback switches: `DAVINCI_TURN_CONTEXT=system`, `DAVINCI_PRUNE_PROFILE=default|off`, `DAVINCI_OPENAI_VERBOSITY`, `DAVINCI_REASONING_SUMMARY`.
- The before and after tables from Step 2.

- [ ] **Step 4: Full verification**

Run: `rtk cargo fmt --all -- --check`
Run: `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`
Run: `rtk cargo test --workspace --offline`
Expected: all clean, all pass. Close any running `davinci.exe` first; a locked binary fails the test build on Windows.

- [ ] **Step 5: Commit, push, PR**

```bash
git add scripts/measure-codex-cache.mjs docs/openai-efficiency.md
git -c core.hooksPath=NUL commit --no-verify -m "docs(openai): record cache-stable turns and rollback switches"
git push -u origin openai-efficiency-a-cache-stable
gh pr create --title "Keep OpenAI requests append-only across turns" --body-file <written PR body>
```

- [ ] **Step 6: Deliver to the installed executable**

Follow `CLAUDE.md` "Deliver changes to the executable the user actually runs": `Get-Command davinci -All`, `rtk cargo build -p davinci-coding-agent --release --offline`, back up the resolved `davinci.exe` with a dated `.bak` name, copy the new build over it, compare `Get-FileHash` of both, run `davinci --version` and one `davinci -p` smoke prompt, then tell the user to restart open sessions.

---

# Part B: Use what the Codex backend supports

Branch: `openai-efficiency-b-codex-features` from `origin/main` after Part A merges.

### Task B1: Live backend probe

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs` (new `RawProviderReply`, `raw_provider_post`)
- Create: `crates/davinci-coding-agent/src/codex_probe.rs`
- Modify: `crates/davinci-coding-agent/src/args.rs` (flag `--codex-probe <path>`), `crates/davinci-coding-agent/src/main.rs` (dispatch before mode selection; extract `resolve_model_and_auth` from `complete_simple_summarization` lines 1010-1052)
- Create: `docs/cache/codex-backend-probe.md`

**Interfaces:**
- Produces: `pub struct RawProviderReply { pub status: u16, pub headers: Vec<(String, String)>, pub body: String }`, `pub fn raw_provider_post(model: &Model, auth: &ResolvedAuth, url: &str, body: &Value) -> Result<RawProviderReply, String>`; `pub(crate) fn resolve_model_and_auth(parsed: &Args, provider: &str, model_id: &str) -> Result<(davinci_ai::Model, ResolvedAuth), String>`; `pub(crate) struct ProbeCase { pub name: &'static str, pub path_suffix: &'static str, pub body: Value }`, `pub(crate) fn probe_cases(model_id: &str, bootstrap: &str) -> Vec<ProbeCase>`, `pub(crate) fn classify(reply: &RawProviderReply) -> ProbeOutcome`, `pub(crate) enum ProbeOutcome { Accepted { cached_tokens: Option<u64>, cache_write_tokens: Option<u64> }, Rejected { status: u16, message: String } }`.

- [ ] **Step 1: Write the failing offline tests**

Create `crates/davinci-coding-agent/src/codex_probe.rs`:

```rust
//! Live probe of the ChatGPT Codex Responses backend. Run by a person with
//! `davinci --codex-probe <report.json>`; never run by tests. No TypeScript
//! counterpart.

use davinci_ai::RawProviderReply;
use serde_json::{json, Value};

pub(crate) struct ProbeCase {
    pub name: &'static str,
    pub path_suffix: &'static str,
    pub body: Value,
}

#[derive(Debug, PartialEq)]
pub(crate) enum ProbeOutcome {
    Accepted {
        cached_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
    },
    Rejected {
        status: u16,
        message: String,
    },
}

fn base(model_id: &str, input: Value) -> Value {
    json!({
        "model": model_id,
        "store": false,
        "stream": true,
        "instructions": "Reply with the single word OK.",
        "input": input,
    })
}

fn user(text: &str) -> Value {
    json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]})
}

/// `bootstrap` is a stable text of at least 1,200 tokens so a second request
/// can report cached tokens.
pub(crate) fn probe_cases(model_id: &str, bootstrap: &str) -> Vec<ProbeCase> {
    let read_tool = json!({"type": "function", "name": "read", "description": "Read a file.",
        "parameters": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}});
    let mut cases = Vec::new();
    cases.push(ProbeCase { name: "baseline", path_suffix: "", body: base(model_id, json!([user("hi")])) });
    let mut warm = base(model_id, json!([user(bootstrap), user("hi")]));
    warm["prompt_cache_key"] = json!("davinci-probe");
    cases.push(ProbeCase { name: "cache_warm", path_suffix: "", body: warm.clone() });
    cases.push(ProbeCase { name: "cache_reuse", path_suffix: "", body: warm });
    let mut options = base(model_id, json!([user("hi")]));
    options["prompt_cache_options"] = json!({"mode": "implicit", "ttl": "30m"});
    cases.push(ProbeCase { name: "prompt_cache_options", path_suffix: "", body: options });
    let mut breakpoint = base(model_id, json!([
        {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": bootstrap,
            "prompt_cache_breakpoint": {"mode": "explicit"}}]},
        user("hi")
    ]));
    breakpoint.as_object_mut().unwrap().remove("instructions");
    breakpoint["prompt_cache_options"] = json!({"mode": "implicit"});
    cases.push(ProbeCase { name: "explicit_breakpoint_without_instructions", path_suffix: "", body: breakpoint });
    let mut prewarm = base(model_id, json!([user(bootstrap)]));
    prewarm["prompt_cache_options"] = json!({"prewarm": true});
    cases.push(ProbeCase { name: "prewarm", path_suffix: "", body: prewarm });
    let mut allowed = base(model_id, json!([user("hi")]));
    allowed["tools"] = json!([read_tool.clone()]);
    allowed["tool_choice"] = json!({"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "read"}]});
    cases.push(ProbeCase { name: "allowed_tools", path_suffix: "", body: allowed });
    let mut additional = base(model_id, json!([user("hi"), {"type": "additional_tools", "tools": [read_tool.clone()]}]));
    additional["tools"] = json!([]);
    cases.push(ProbeCase { name: "additional_tools", path_suffix: "", body: additional });
    let mut custom = base(model_id, json!([user("hi")]));
    custom["tools"] = json!([{"type": "custom", "name": "apply_patch", "description": "Apply a patch.",
        "format": {"type": "grammar", "syntax": "lark", "definition": davinci_ai::APPLY_PATCH_LARK}}]);
    cases.push(ProbeCase { name: "custom_grammar_tool", path_suffix: "", body: custom });
    let phase = base(model_id, json!([user("hi"),
        {"type": "message", "role": "assistant", "phase": "commentary", "content": [{"type": "output_text", "text": "Checking."}]},
        user("continue")]));
    cases.push(ProbeCase { name: "assistant_phase", path_suffix: "", body: phase });
    let mut none = base(model_id, json!([user("hi")]));
    none["tools"] = json!([read_tool]);
    none["tool_choice"] = json!("none");
    cases.push(ProbeCase { name: "tool_choice_none", path_suffix: "", body: none });
    cases.push(ProbeCase {
        name: "remote_compact",
        path_suffix: "/compact",
        body: json!({"model": model_id, "instructions": "Reply with OK.", "input": [user("hi")]}),
    });
    cases
}

fn completed_usage(body: &str) -> Option<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.completed")
        .and_then(|event| event["response"].get("usage").cloned())
}

pub(crate) fn classify(reply: &RawProviderReply) -> ProbeOutcome {
    if !(200..300).contains(&reply.status) {
        return ProbeOutcome::Rejected {
            status: reply.status,
            message: reply.body.chars().take(400).collect(),
        };
    }
    let usage = completed_usage(&reply.body);
    let detail = |key: &str| {
        usage
            .as_ref()
            .and_then(|usage| usage["input_tokens_details"][key].as_u64())
    };
    ProbeOutcome::Accepted {
        cached_tokens: detail("cached_tokens"),
        cache_write_tokens: detail("cache_write_tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> RawProviderReply {
        RawProviderReply { status, headers: Vec::new(), body: body.into() }
    }

    #[test]
    fn a_completed_stream_reports_cache_usage() {
        let body = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1300,\"input_tokens_details\":{\"cached_tokens\":1280,\"cache_write_tokens\":0}}}}\n";
        assert_eq!(
            classify(&reply(200, body)),
            ProbeOutcome::Accepted { cached_tokens: Some(1280), cache_write_tokens: Some(0) }
        );
    }

    #[test]
    fn an_error_status_is_a_rejection_with_the_message() {
        let outcome = classify(&reply(400, "{\"detail\":\"Unsupported parameter: prompt_cache_options\"}"));
        assert!(matches!(outcome, ProbeOutcome::Rejected { status: 400, ref message } if message.contains("prompt_cache_options")));
    }

    #[test]
    fn every_case_has_a_unique_name() {
        let cases = probe_cases("gpt-5.6-luna", "x");
        let mut names: Vec<_> = cases.iter().map(|case| case.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), cases.len());
    }
}
```

Add `mod codex_probe;` to `main.rs`.

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-coding-agent codex_probe --offline`
Expected: compile errors for `davinci_ai::RawProviderReply` and `davinci_ai::APPLY_PATCH_LARK`.

- [ ] **Step 3: Add the raw POST and the grammar constant**

In `crates/davinci-ai/src/stream.rs`:

```rust
/// One raw provider exchange, for the manual Codex probe only.
#[derive(Debug, Clone)]
pub struct RawProviderReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub fn raw_provider_post(
    model: &Model,
    auth: &ResolvedAuth,
    url: &str,
    body: &Value,
) -> Result<RawProviderReply, String> {
    let headers = collect_request_headers(model, auth, &StreamOptions::default());
    let mut request = ureq::post(url).timeout(Duration::from_secs(120));
    for (key, value) in &headers {
        request = request.set(key, value);
    }
    let result = if model.api == "openai-codex-responses" {
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(body);
        if compressed {
            request = request.set("content-encoding", "zstd");
        }
        request.send_bytes(&bytes)
    } else {
        request.send_string(&body.to_string())
    };
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(error.to_string()),
    };
    let status = response.status();
    let headers = response
        .headers_names()
        .into_iter()
        .filter_map(|name| response.header(&name).map(|value| (name.clone(), value.to_string())))
        .collect();
    let body = response.into_string().map_err(|error| error.to_string())?;
    Ok(RawProviderReply { status, headers, body })
}
```

Export both from `crates/davinci-ai/src/lib.rs` in the `pub use stream::{...}` list.

Create `crates/davinci-ai/src/apply_patch_grammar.rs`. The grammar accepts exactly what `crates/davinci-agent/src/apply_patch.rs::parse_codex_patch` accepts (Add, Delete, Update, `@@` context lines, and `+`, `-`, space lines). It omits `*** Move to:` and `*** End of File`, which that parser does not support.

```rust
//! Lark grammar for the freeform `apply_patch` tool. Mirrors the Codex CLI
//! grammar reduced to the forms `davinci_agent::apply_patch::parse_codex_patch`
//! accepts. Keep both in step.

pub const APPLY_PATCH_LARK: &str = r#"start: begin_patch hunk+ end_patch
begin_patch: "*** Begin Patch" LF
end_patch: "*** End Patch" LF?

hunk: add_hunk | delete_hunk | update_hunk
add_hunk: "*** Add File: " filename LF add_line+
delete_hunk: "*** Delete File: " filename LF
update_hunk: "*** Update File: " filename LF change+

filename: /(.+)/
add_line: "+" /(.*)/ LF -> line

change: change_context | change_line
change_context: ("@@" | "@@ " /(.+)/) LF
change_line: ("+" | "-" | " ") /(.*)/ LF

%import common.LF
"#;
```

Add `mod apply_patch_grammar;` and `pub use apply_patch_grammar::APPLY_PATCH_LARK;` to `davinci-ai/src/lib.rs`.

In `crates/davinci-agent/src/apply_patch.rs` tests, add a test that a patch using every grammar form parses:

```rust
    #[test]
    fn every_form_in_the_freeform_grammar_parses() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+one\n*** Delete File: b.txt\n*** Update File: c.txt\n@@ fn main\n context\n-old\n+new\n*** End Patch\n";
        let parsed = parse_codex_patch(patch).expect("grammar forms parse");
        assert_eq!(parsed.actions.len(), 3);
    }
```

- [ ] **Step 4: Add the command**

In `main.rs`, extract lines 1010-1052 of `complete_simple_summarization` into:

```rust
pub(crate) fn resolve_model_and_auth(
    parsed: &Args,
    provider: &str,
    model_id: &str,
) -> Result<(davinci_ai::Model, ResolvedAuth), String>
```

returning the same `model` and `auth` values, and call it from `complete_simple_summarization`. Behavior must not change; the existing summarization tests cover it.

In `codex_probe.rs` add the runner:

```rust
pub(crate) fn run(parsed: &crate::Args, report_path: &std::path::Path) -> Result<(), String> {
    let model_id = parsed.model.clone().unwrap_or_else(|| "gpt-5.6-luna".into());
    let (model, auth) = crate::resolve_model_and_auth(parsed, "openai-codex", &model_id)?;
    let url = davinci_ai::request_url(&model, &auth);
    let bootstrap = "DaVinci probe stable text. ".repeat(260);
    let mut rows = Vec::new();
    for case in probe_cases(&model.id, &bootstrap) {
        let reply = davinci_ai::raw_provider_post(&model, &auth, &format!("{url}{}", case.path_suffix), &case.body);
        let row = match reply {
            Ok(reply) => {
                let codex_headers: Vec<_> = reply
                    .headers
                    .iter()
                    .filter(|(name, _)| name.to_ascii_lowercase().starts_with("x-codex"))
                    .cloned()
                    .collect();
                json!({"case": case.name, "outcome": format!("{:?}", classify(&reply)), "codexHeaders": codex_headers})
            }
            Err(error) => json!({"case": case.name, "outcome": format!("transport error: {error}")}),
        };
        eprintln!("{row}");
        rows.push(row);
    }
    std::fs::write(report_path, serde_json::to_string_pretty(&rows).unwrap())
        .map_err(|error| error.to_string())
}
```

In `args.rs`, add `pub codex_probe: Option<String>` to `Args` and parse `--codex-probe <path>` beside `--export`. In `main.rs` `run()`, before mode selection:

```rust
    if let Some(path) = parsed.codex_probe.as_deref() {
        return match codex_probe::run(&parsed, std::path::Path::new(path)) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("codex probe failed: {error}");
                1
            }
        };
    }
```

Match the return type `run()` uses; if it returns `Result`, return `Ok(())` and `Err(error.into())`. Do not list `--codex-probe` in `--help`; it is a maintainer command.

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test -p davinci-coding-agent codex_probe --offline` then `rtk cargo test -p davinci-agent every_form_in_the_freeform --offline`
Expected: all pass.

- [ ] **Step 6: Run the probe once (a person runs this; about 13 small Luna requests)**

```powershell
.\target\debug\davinci.exe --codex-probe "$env:TEMP\codex-probe.json" --model gpt-5.6-luna
```

Write `docs/cache/codex-backend-probe.md` with the date, model, and one row per case: accepted or rejected, the error text when rejected, `cached_tokens` for `cache_reuse`, and the `x-codex-*` header names seen. Do not paste tokens or account ids. The later tasks read this file:

| Probe case | Gates |
| --- | --- |
| `prompt_cache_options` | B2 TTL |
| `explicit_breakpoint_without_instructions` | B2 bootstrap breakpoint |
| `custom_grammar_tool` | B4 |
| `assistant_phase` | B3 phase replay (items are replayed raw either way) |
| `allowed_tools`, `additional_tools` | B5 |
| `tool_choice_none` | C1 |
| `remote_compact` | C2 |
| `x-codex-*` headers | C3 |

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-ai/src crates/davinci-agent/src/apply_patch.rs crates/davinci-coding-agent/src docs/cache/codex-backend-probe.md
git -c core.hooksPath=NUL commit --no-verify -m "feat(codex): add a maintainer probe for Codex backend request features"
```

### Task B2: Cache policy on the Codex route

Gate: `prompt_cache_options` or `explicit_breakpoint_without_instructions` accepted in `docs/cache/codex-backend-probe.md`. If both were rejected, skip this task and record that in the probe doc.

**Files:**
- Modify: `crates/davinci-ai/src/codex_capabilities.rs:58-80` (`for_chatgpt_codex`)
- Modify: `crates/davinci-ai/src/openai_cache_policy.rs:169-187` (chatgpt branch)
- Modify: `crates/davinci-ai/src/stream.rs:1066-1135`
- Modify: `crates/davinci-ai/src/catalogs.json` (compat flags on the `openai-codex` models the probe covered)

**Interfaces:**
- Consumes: compat keys `supportsExplicitPromptCacheMode` (bool) and new `supportsPromptCacheTtl` (bool).
- Produces: Codex requests with `prompt_cache_options` and, when supported, the bootstrap breakpoint.

- [ ] **Step 1: Write the failing tests**

Add to the tests in `stream.rs`:

```rust
    fn codex_model_with(compat: serde_json::Value) -> Model {
        let mut model = crate::load_builtin_models()
            .into_iter()
            .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
            .unwrap();
        if !model.compat.is_object() {
            model.compat = serde_json::json!({});
        }
        for (key, value) in compat.as_object().unwrap() {
            model.compat[key.as_str()] = value.clone();
        }
        model
    }

    #[test]
    fn codex_requests_carry_cache_options_when_the_backend_accepts_them() {
        let model = codex_model_with(serde_json::json!({
            "supportsExplicitPromptCacheMode": true,
            "supportsPromptCacheTtl": true
        }));
        let options = StreamOptions { cache_key: Some("k".into()), ..Default::default() };
        let body = request_body_with(&model, &[ChatMessage::text("user", "hi")], Some("stable"), &[], &options);
        assert_eq!(body["prompt_cache_options"]["ttl"], "30m");
        assert_eq!(body["prompt_cache_key"], "k");
    }

    #[test]
    fn codex_requests_are_unchanged_without_the_compat_flags() {
        let model = codex_model_with(serde_json::json!({
            "supportsExplicitPromptCacheMode": false,
            "supportsPromptCacheTtl": false
        }));
        let body = request_body_with(&model, &[ChatMessage::text("user", "hi")], Some("stable"), &[], &StreamOptions::default());
        assert!(body.get("prompt_cache_options").is_none());
        assert_eq!(body["instructions"], "stable");
    }
```

`Model::compat` is a `serde_json::Value` (`catalog.rs:184`), so the helper indexes it rather than inserting into a map.

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-ai codex_requests_carry --offline`
Expected: FAIL, `prompt_cache_options` is null.

- [ ] **Step 3: Implement**

In `codex_capabilities.rs::for_chatgpt_codex`, read the breakpoint flag from compat instead of hard-coding `true`:

```rust
        let explicit = model
            .compat
            .get("supportsExplicitPromptCacheMode")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
```

and set `explicit_cache_breakpoints: explicit`.

In `openai_cache_policy.rs`, chatgpt branch, set `supports_ttl_30m` from `model.compat.get("supportsPromptCacheTtl").and_then(|v| v.as_bool()).unwrap_or(false)`.

In `stream.rs::openai_responses_body`, resolve capabilities for both Responses routes:

```rust
    let cache_capabilities = match model.api.as_str() {
        "openai-responses" => crate::openai_cache_policy::OpenAiCacheCapabilities::resolve(
            model,
            model.base_url.as_deref(),
            false,
        ),
        "openai-codex-responses" => crate::openai_cache_policy::OpenAiCacheCapabilities::resolve(
            model,
            model.base_url.as_deref(),
            true,
        ),
        _ => crate::openai_cache_policy::OpenAiCacheCapabilities::unknown(),
    };
```

Replace the `"openai-codex-responses"` match arm so it emits the key as before and the planned options:

```rust
        "openai-codex-responses" => {
            if retention != crate::cache::CacheRetention::None {
                if let Some(key) = session_key {
                    body["prompt_cache_key"] = Value::String(key);
                }
            }
            if let Some(mode) = cache_plan.prompt_cache_mode {
                let mut prompt_cache_options = serde_json::json!({"mode": mode});
                if let Some(ttl) = cache_plan.prompt_cache_ttl {
                    prompt_cache_options["ttl"] = Value::String(ttl.into());
                }
                body["prompt_cache_options"] = prompt_cache_options;
            }
        }
```

If the probe accepted `prompt_cache_options` but rejected `explicit_breakpoint_without_instructions`, set only `supportsPromptCacheTtl: true` and leave `supportsExplicitPromptCacheMode: false`. The planner then keeps `instructions` and adds no breakpoint. Add a test for that combination: `prompt_cache_options` absent (the planner emits a mode only for `ExplicitBoundaries`) and `instructions` present; if the probe shows the TTL alone is worth sending, extend `PromptCacheWirePlan::resolve` with a `LegacyImplicit`-plus-TTL case and test it.

In `catalogs.json`, add the flags the probe justified to each probed `openai-codex` model's `compat`.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-ai --offline`
Expected: all pass. Existing tests that asserted Codex bodies without cache options still pass because the catalog flags default to off for unprobed models.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src
git -c core.hooksPath=NUL commit --no-verify -m "feat(codex): send probed prompt cache options on the ChatGPT route"
```

### Task B3: Replay each assistant message's native items

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs:1016-1073` (`openai_responses_input` gains options), `:1266-1270` (native resume tail uses the same options)
- Modify: `crates/davinci-ai/src/responses_ledger.rs:255-262` (fingerprint ignores the new keys)
- Modify: `crates/davinci-agent/src/turn.rs:280-283` and `:3588-3612` (attach and persist items)

**Interfaces:**
- Produces: `pub const NATIVE_ITEMS_KEY: &str = "responsesOutputItems";`, `pub const NATIVE_MODEL_KEY: &str = "responsesOutputModel";`, `#[derive(Default)] pub struct ResponsesInputOptions<'a> { pub native_items_model: Option<&'a str>, pub custom_tools: &'a [&'a str] }`, `pub fn openai_responses_input_with(messages: &[ChatMessage], options: &ResponsesInputOptions<'_>) -> Vec<Value>`, `pub fn attach_native_items(chat: &mut ChatMessage, items: &[Value], model_key: &str)`.

- [ ] **Step 1: Write the failing tests**

In `stream.rs` tests:

```rust
    #[test]
    fn an_assistant_message_replays_its_own_reasoning_and_phase() {
        let items = vec![
            serde_json::json!({"type": "reasoning", "id": "rs_1", "encrypted_content": "enc", "summary": []}),
            serde_json::json!({"type": "message", "role": "assistant", "phase": "final_answer",
                "content": [{"type": "output_text", "text": "Done."}]}),
        ];
        let mut assistant = ChatMessage::text("assistant", "Done.");
        crate::attach_native_items(&mut assistant, &items, "openai-codex/gpt-5.6-luna");
        let messages = [ChatMessage::text("user", "go"), assistant, ChatMessage::text("user", "next")];
        let input = crate::openai_responses_input_with(
            &messages,
            &crate::ResponsesInputOptions { native_items_model: Some("openai-codex/gpt-5.6-luna"), custom_tools: &[] },
        );
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["encrypted_content"], "enc");
        assert_eq!(input[2]["phase"], "final_answer");
        assert_eq!(input.len(), 4);
    }

    #[test]
    fn items_from_another_model_are_not_replayed() {
        let items = vec![serde_json::json!({"type": "reasoning", "id": "rs_1", "encrypted_content": "enc"})];
        let mut assistant = ChatMessage::text("assistant", "Done.");
        crate::attach_native_items(&mut assistant, &items, "openai-codex/gpt-5.6-sol");
        let input = crate::openai_responses_input_with(
            &[assistant],
            &crate::ResponsesInputOptions { native_items_model: Some("openai-codex/gpt-5.6-luna"), custom_tools: &[] },
        );
        assert_eq!(input[0]["role"], "assistant");
        assert!(input.iter().all(|item| item["type"] != "reasoning"));
    }
```

In `responses_ledger.rs` tests:

```rust
    #[test]
    fn native_items_do_not_change_the_projection_fingerprint() {
        let plain = ChatMessage::text("assistant", "Done.");
        let mut with_items = plain.clone();
        crate::attach_native_items(&mut with_items, &[serde_json::json!({"type": "reasoning"})], "p/m");
        assert_eq!(
            provider_messages_fingerprint(&[plain]),
            provider_messages_fingerprint(&[with_items])
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-ai native_items --offline`
Expected: compile errors for `attach_native_items` and `openai_responses_input_with`.

- [ ] **Step 3: Implement the input options**

In `stream.rs`:

```rust
pub const NATIVE_ITEMS_KEY: &str = "responsesOutputItems";
pub const NATIVE_MODEL_KEY: &str = "responsesOutputModel";

#[derive(Debug, Default, Clone, Copy)]
pub struct ResponsesInputOptions<'a> {
    /// `provider/model` whose saved output items may be replayed verbatim.
    pub native_items_model: Option<&'a str>,
    /// Tools sent as Responses custom (freeform) tools.
    pub custom_tools: &'a [&'a str],
}

pub fn attach_native_items(chat: &mut ChatMessage, items: &[Value], model_key: &str) {
    if items.is_empty() {
        return;
    }
    chat.extra.insert(NATIVE_ITEMS_KEY.into(), Value::Array(items.to_vec()));
    chat.extra.insert(NATIVE_MODEL_KEY.into(), Value::String(model_key.into()));
}

fn native_items<'m>(message: &'m ChatMessage, model_key: Option<&str>) -> Option<&'m Vec<Value>> {
    let model_key = model_key?;
    if message.extra.get(NATIVE_MODEL_KEY).and_then(Value::as_str) != Some(model_key) {
        return None;
    }
    message.extra.get(NATIVE_ITEMS_KEY).and_then(Value::as_array)
}
```

Rename the body of `openai_responses_input` to `openai_responses_input_with(messages, options)` and keep:

```rust
pub fn openai_responses_input(messages: &[ChatMessage]) -> Vec<Value> {
    openai_responses_input_with(messages, &ResponsesInputOptions::default())
}
```

At the top of the `if message.role == "assistant"` branch add:

```rust
            if let Some(items) = native_items(message, options.native_items_model) {
                input.extend(items.iter().cloned());
                continue;
            }
```

In `openai_responses_body`, build the options and use them:

```rust
    let model_key = format!("{}/{}", model.provider, model.id);
    let input_options = ResponsesInputOptions {
        native_items_model: Some(&model_key),
        custom_tools: &[],
    };
    let mut input = openai_responses_input_with(messages, &input_options);
```

In `apply_native_responses_resume`, replace `openai_responses_input(&messages[resume.resume_provider_message_count..])` with `openai_responses_input_with(&messages[resume.resume_provider_message_count..], &ResponsesInputOptions { native_items_model: Some(&format!("{}/{}", model.provider, model.id)), custom_tools: &[] })`. Task B4 fills `custom_tools` in both places.

Export `NATIVE_ITEMS_KEY`, `NATIVE_MODEL_KEY`, `ResponsesInputOptions`, `attach_native_items`, `openai_responses_input_with` from `lib.rs`.

In `responses_ledger.rs::provider_messages_fingerprint`, hash copies without the two keys:

```rust
pub fn provider_messages_fingerprint(messages: &[ChatMessage]) -> String {
    let stripped: Vec<ChatMessage> = messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            message.extra.remove(crate::NATIVE_ITEMS_KEY);
            message.extra.remove(crate::NATIVE_MODEL_KEY);
            message
        })
        .collect();
    let bytes = serde_json::to_vec(&stripped).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"davinci.responses-provider-projection.v1\0");
    hasher.update((stripped.len() as u64).to_le_bytes());
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
```

- [ ] **Step 4: Attach and persist in the agent**

In `crates/davinci-agent/src/turn.rs` at line 280, replace `let chat = assistant_to_chat(&assistant);` with:

```rust
            let mut chat = assistant_to_chat(&assistant);
            if let Some(record) = &native_responses_resume {
                davinci_ai::attach_native_items(
                    &mut chat,
                    &record.turn.output.output_items,
                    &format!("{}/{}", self.provider, self.model_id),
                );
            }
```

In `persist_assistant`, after `"timestamp": timestamp,` is set, copy the two keys into the saved message:

```rust
            for key in [davinci_ai::NATIVE_ITEMS_KEY, davinci_ai::NATIVE_MODEL_KEY] {
                if let Some(value) = chat.extra.get(key) {
                    message[key] = value.clone();
                }
            }
```

Add a test in `turn.rs` tests (or `lib.rs` tests, wherever `run_loop` fixture tests live) that runs one fixture turn whose `CompleteOutput` carries a `native_responses_resume` record with one reasoning item, and asserts the pushed assistant message's `extra[NATIVE_ITEMS_KEY]` has that item. Use the existing fixture that constructs a `NativeResponsesResumeRecord` (search `NativeResponsesResumeRecord {` in `davinci-agent` tests) as the template.

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test -p davinci-ai --offline` then `rtk cargo test -p davinci-agent --offline`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-ai/src crates/davinci-agent/src
git -c core.hooksPath=NUL commit --no-verify -m "feat(openai): replay each assistant message's own reasoning and phase"
```

### Task B4: Freeform `apply_patch` on the Codex route

Gate: `custom_grammar_tool` accepted in the probe doc.

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs` (tool mapping in `openai_responses_body`; `openai_responses_input_with` custom calls)

**Interfaces:**
- Consumes: `APPLY_PATCH_LARK` (B1), `ResponsesInputOptions::custom_tools` (B3).
- Produces: `fn freeform_tools(model: &Model) -> &'static [&'static str]`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn codex_sends_apply_patch_as_a_grammar_tool() {
        let model = codex_model_with(serde_json::json!({"supportsOpenAIGrammarTools": true}));
        let tools = [ToolSpec {
            name: "apply_patch".into(),
            description: "Apply a patch.".into(),
            parameters: serde_json::json!({"type": "object"}),
            constrained_sampling: None,
        }];
        let body = request_body_with(&model, &[ChatMessage::text("user", "hi")], Some("s"), &tools, &StreamOptions::default());
        assert_eq!(body["tools"][0]["type"], "custom");
        assert_eq!(body["tools"][0]["format"]["syntax"], "lark");
    }

    #[test]
    fn a_freeform_call_and_its_result_replay_as_custom_items() {
        let mut assistant = ChatMessage::default();
        assistant.role = "assistant".into();
        assistant.content = vec![MessageContent::ToolCall {
            id: "call_1|ct_1".into(),
            name: "apply_patch".into(),
            arguments: serde_json::json!({"input": "*** Begin Patch\n*** End Patch\n"}),
        }];
        let mut result = ChatMessage::text("toolResult", "applied");
        result.tool_call_id = Some("call_1|ct_1".into());
        result.tool_name = Some("apply_patch".into());
        let input = crate::openai_responses_input_with(
            &[assistant, result],
            &crate::ResponsesInputOptions { native_items_model: None, custom_tools: &["apply_patch"] },
        );
        assert_eq!(input[0]["type"], "custom_tool_call");
        assert_eq!(input[0]["input"], "*** Begin Patch\n*** End Patch\n");
        assert_eq!(input[1]["type"], "custom_tool_call_output");
        assert_eq!(input[1]["call_id"], "call_1");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-ai freeform --offline` and `rtk cargo test -p davinci-ai codex_sends_apply_patch --offline`
Expected: FAIL, tool type is `function`.

- [ ] **Step 3: Implement**

```rust
/// Tools sent as Responses custom tools. `DAVINCI_OPENAI_GRAMMAR_TOOLS=0`
/// turns this off.
fn freeform_tools(model: &Model) -> &'static [&'static str] {
    let enabled = model.api == "openai-codex-responses"
        && model
            .compat
            .get("supportsOpenAIGrammarTools")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && std::env::var("DAVINCI_OPENAI_GRAMMAR_TOOLS").as_deref() != Ok("0");
    if enabled {
        &["apply_patch"]
    } else {
        &[]
    }
}
```

In `openai_responses_body`, set `custom_tools: freeform_tools(model)` in `input_options` and in the resume tail. In the tools mapping:

```rust
                .map(|tool| {
                    if freeform_tools(model).contains(&tool.name.as_str()) {
                        return serde_json::json!({
                            "type": "custom",
                            "name": tool.name,
                            "description": tool.description,
                            "format": {
                                "type": "grammar",
                                "syntax": "lark",
                                "definition": crate::APPLY_PATCH_LARK,
                            },
                        });
                    }
                    let mut function = serde_json::json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    });
                    if resolve_json_schema_strict_sampling(tool).unwrap_or(false) {
                        function["strict"] = Value::Bool(true);
                    }
                    function
                })
```

In `openai_responses_input_with`, the `toolResult` branch:

```rust
        if message.role == "toolResult" {
            let call_id = responses_call_id(message.tool_call_id.as_deref().unwrap_or_default());
            let custom = message
                .tool_name
                .as_deref()
                .is_some_and(|name| options.custom_tools.contains(&name));
            input.push(serde_json::json!({
                "type": if custom { "custom_tool_call_output" } else { "function_call_output" },
                "call_id": call_id,
                "output": content_text(&message.content),
            }));
            continue;
        }
```

and the tool-call loop in the assistant branch:

```rust
                if let MessageContent::ToolCall { id, name, arguments } = block {
                    if options.custom_tools.contains(&name.as_str()) {
                        input.push(serde_json::json!({
                            "type": "custom_tool_call",
                            "call_id": responses_call_id(id),
                            "name": name,
                            "input": arguments.get("input").and_then(Value::as_str).unwrap_or_default(),
                        }));
                    } else {
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": responses_call_id(id),
                            "name": name,
                            "arguments": arguments.to_string(),
                        }));
                    }
                }
```

The decoder already turns `custom_tool_call` into a `ToolCall` with `{"input": ...}` (`stream_decoder.rs:326-345`), and the `apply_patch` tool reads `input`, so execution needs no change.

Update the `apply_patch` description in `crates/davinci-agent/src/tools.rs:398` only if the probe shows the model misuses the tool; the grammar already constrains the format.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-ai --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src
git -c core.hooksPath=NUL commit --no-verify -m "feat(codex): send apply_patch as a freeform grammar tool"
```

### Task B5: Late tools without changing the tool list

Gate: `additional_tools` accepted in the probe doc. If it was rejected, skip this task: Task A6 already keeps the list fixed by sending every authorized tool from the first request. Record the skip in the probe doc.

**Files:**
- Modify: `crates/davinci-agent/src/lib.rs` (`freeze_tools_for_cache` records the frozen set; new `late_tool_specs`)
- Modify: `crates/davinci-coding-agent/src/main.rs` (pass late tools as an input item)
- Modify: `crates/davinci-ai/src/stream.rs` (append `additional_tools` item)

**Interfaces:**
- Produces: `Agent::frozen_tool_names: Option<BTreeSet<String>>` (private), `Agent::late_tool_specs(&self) -> Vec<AgentTool>`, `StreamOptions` is not changed; late tools reach `request_body_with` as a `ChatMessage` with role `"custom"` and `extra["additionalTools"]`, converted by `openai_responses_input_with` into `{"type": "additional_tools", "tools": [...]}`.

- [ ] **Step 1: Write the failing test**

In `cache_stability.rs`:

```rust
    #[test]
    fn a_tool_activated_after_the_freeze_arrives_as_an_additional_tools_item() {
        let (mut agent, model) = appended_codex_agent();
        agent.additional_tools_enabled = true;
        agent.freeze_tools_for_cache();
        user_turn(&mut agent, "hi", None);
        let first = wire_body_for_next_request(&agent, &model);
        agent.messages.push(davinci_ai::ChatMessage::text("assistant", "Looking."));
        agent.record_late_tool_activation(
            "mcp__docs__search",
            "Search docs.",
            serde_json::json!({"type": "object"}),
        );
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
        assert!(second["input"].to_string().contains("additional_tools"));
    }
```

`additional_tools_enabled` is a new `pub additional_tools_enabled: bool` field on `Agent` (default `false`). `main.rs` sets it to `true` when `DAVINCI_ADDITIONAL_TOOLS=1` and the placement is `Appended`. Tests set the field directly, never the environment, because tests run in parallel.

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p davinci-agent additional_tools_item --offline`
Expected: compile error `no method named record_late_tool_activation`.

- [ ] **Step 3: Implement**

`record_late_tool_activation(name, description, parameters)` appends a persisted custom message with `customType: "davinci.additional_tools"`, `display: false`, content `"Tool available: {name}"`, and `details: {"tools": [{"type": "function", "name": ..., "description": ..., "parameters": ...}]}`. Call it from the `tool_search` activation path (`crates/davinci-agent/src/tools.rs:981`) when the agent's `additional_tools_enabled` is true and the tool is not in the frozen set; do not add the tool to the provider tool list.

In `openai_responses_input_with`, before the generic `custom` handling, turn such a message into:

```rust
        if message.extra.get("customType").and_then(Value::as_str) == Some("davinci.additional_tools") {
            if let Some(tools) = message.extra.get("details").and_then(|details| details.get("tools")) {
                input.push(serde_json::json!({"type": "additional_tools", "tools": tools}));
                continue;
            }
        }
```

`convert_to_llm` must keep `customType` and `details` on the converted message; check `compaction.rs:732-745` and keep those keys in the converted `extra` if they are dropped today.

Non-Responses providers never enable `additional_tools_enabled`, so they never see this item.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-agent --offline` then `rtk cargo test -p davinci-ai --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src crates/davinci-ai/src crates/davinci-coding-agent/src/main.rs
git -c core.hooksPath=NUL commit --no-verify -m "feat(codex): deliver late tool activations as additional_tools items"
```

### Task B6: OpenAI prompt adapter

**Files:**
- Modify: `crates/davinci-agent/src/prompt/provider.rs:52-60`

**Interfaces:**
- Produces: `provider_adapter(PromptModelFamily::OpenAiReasoning)` returns `Some(PromptModule)` with id `provider.openai-reasoning`, `PromptCacheClass::Stable`, at most `PROVIDER_ADAPTER_MAX_TOKENS` (250).

The text follows the Codex prompting guide: parallel reads, no requests for upfront plans or status updates (the guide warns these can make the model stop early), and a short finish.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn openai_reasoning_models_get_a_stable_adapter() {
        let module = provider_adapter(PromptModelFamily::OpenAiReasoning).expect("adapter");
        assert_eq!(module.id, "provider.openai-reasoning");
        assert_eq!(module.cache_class, PromptCacheClass::Stable);
        assert!(module.body.contains("parallel"));
        assert!(estimate_tokens_from_str(&module.body) <= PROVIDER_ADAPTER_MAX_TOKENS);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p davinci-agent openai_reasoning_models_get --offline`
Expected: FAIL, `adapter` is `None`.

- [ ] **Step 3: Implement**

```rust
const OPENAI_REASONING_ADAPTER: &str = "\
OpenAI model guidance:
- When you know several paths or searches you need, request them in one message as parallel tool calls. Do not read files one at a time unless each read depends on the previous one.
- Use apply_patch for edits that touch more than one hunk or file. Keep patches minimal, with enough context lines to match once.
- After a change, report what changed and how you verified it. Do not repeat whole files or the plan.
- Stop when the task is done and verified. Do not start work the user did not ask for.";

pub fn provider_adapter(family: PromptModelFamily) -> Option<PromptModule> {
    match family {
        PromptModelFamily::OpenAiReasoning => {
            Some(create_family_adapter(family, OPENAI_REASONING_ADAPTER))
        }
        PromptModelFamily::Anthropic
        | PromptModelFamily::Gemini
        | PromptModelFamily::Mistral
        | PromptModelFamily::Generic => None,
    }
}
```

This changes the stable prompt hash for OpenAI routes once; the prompt manifest tests that pin a hash for an OpenAI context must be updated to the new value, and only those.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-agent prompt --offline` then `rtk cargo test -p davinci-agent --offline`
Expected: all pass after updating pinned OpenAI hashes.

- [ ] **Step 5: Commit, verify, PR, deliver**

```bash
git add crates/davinci-agent/src/prompt
git -c core.hooksPath=NUL commit --no-verify -m "feat(prompt): add an OpenAI reasoning model adapter"
```

Then run the full verification from Task A8 Step 4, rerun the Task A8 Step 2 measurement with the Part B build, add the table to `docs/openai-efficiency.md`, push `openai-efficiency-b-codex-features`, open the PR, and deliver the executable as in Task A8 Step 6.

---

# Part C: Cost controls and proof

Branch: `openai-efficiency-c-cost-proof` from `origin/main` after Part B merges.

### Task C1: Compaction that reads from the cache

Gate: `tool_choice_none` accepted in the probe doc. Depends on B3 (the summary request must rebuild the same input items the conversation sent).

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs` (`StreamOptions.tool_choice`; honor it in `openai_responses_body`)
- Modify: every `StreamOptions { ... }` literal the compiler reports (add `tool_choice: None,`)
- Modify: `crates/davinci-agent/src/compaction.rs:89-131` (`SummarizeConversation`, slot on `Summarizer`)
- Modify: `crates/davinci-agent/src/lib.rs` (`Agent::compact` fills and clears the slot)
- Modify: `crates/davinci-coding-agent/src/main.rs:969-1090` (conversation mode in the summarizer; extract `root_cache_key`)

**Interfaces:**
- Produces: `StreamOptions::tool_choice: Option<String>`; `pub struct SummarizeConversation { pub system: String, pub tools: Vec<davinci_ai::ToolSpec>, pub messages: Vec<ChatMessage>, pub cache_key: Option<String>, pub session_id: Option<String> }`; `Summarizer::conversation_slot(&self) -> Arc<Mutex<Option<SummarizeConversation>>>`; `pub fn summary_instruction_tail(prompt: &str) -> &str` in `compaction.rs`; `fn root_cache_key(agent: &Agent, model: &Model, system: &str) -> Option<String>` in `main.rs`.

- [ ] **Step 1: Write the failing tests**

In `compaction.rs` tests:

```rust
    #[test]
    fn the_instruction_tail_drops_the_serialized_conversation() {
        let prompt = build_history_prompt(&[ChatMessage::text("user", "hello")], None, Some("focus on tests"));
        let tail = summary_instruction_tail(&prompt);
        assert!(tail.starts_with(SUMMARIZATION_PROMPT));
        assert!(tail.ends_with("Additional focus: focus on tests"));
        assert!(!tail.contains("<conversation>"));
    }

    #[test]
    fn the_update_prompt_tail_skips_the_previous_summary() {
        let prompt = build_history_prompt(&[ChatMessage::text("user", "hello")], Some("old"), None);
        assert_eq!(summary_instruction_tail(&prompt), UPDATE_SUMMARIZATION_PROMPT);
    }
```

In `stream.rs` tests:

```rust
    #[test]
    fn tool_choice_none_keeps_the_tool_list() {
        let model = codex_model_with(serde_json::json!({}));
        let tools = [ToolSpec { name: "read".into(), description: "r".into(), parameters: serde_json::json!({"type": "object"}), constrained_sampling: None }];
        let options = StreamOptions { tool_choice: Some("none".into()), ..Default::default() };
        let body = request_body_with(&model, &[ChatMessage::text("user", "hi")], Some("s"), &tools, &options);
        assert_eq!(body["tool_choice"], "none");
        assert_eq!(body["tools"][0]["name"], "read");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-agent instruction_tail --offline` and `rtk cargo test -p davinci-ai tool_choice_none --offline`
Expected: compile errors.

- [ ] **Step 3: Implement the pieces**

In `compaction.rs`:

```rust
/// The instruction part of a history prompt: everything after the serialized
/// conversation and any previous summary. The constants are not reworded.
pub fn summary_instruction_tail(prompt: &str) -> &str {
    if let Some((_, tail)) = prompt.rsplit_once("</previous-summary>\n\n") {
        return tail;
    }
    prompt
        .rsplit_once("</conversation>\n\n")
        .map(|(_, tail)| tail)
        .unwrap_or(prompt)
}

/// The live provider view a summarizer may reuse so the summary request
/// shares the conversation's cached prefix.
#[derive(Debug, Clone)]
pub struct SummarizeConversation {
    pub system: String,
    pub tools: Vec<davinci_ai::ToolSpec>,
    pub messages: Vec<ChatMessage>,
    pub cache_key: Option<String>,
    pub session_id: Option<String>,
}
```

Add a field `conversation: std::sync::Arc<std::sync::Mutex<Option<SummarizeConversation>>>` to `Summarizer`, initialized empty in `Summarizer::new`, and:

```rust
    pub fn conversation_slot(&self) -> std::sync::Arc<std::sync::Mutex<Option<SummarizeConversation>>> {
        self.conversation.clone()
    }
```

In `stream.rs` add `pub tool_choice: Option<String>,` to `StreamOptions`, then in `openai_responses_body` after the codex block:

```rust
    if let Some(choice) = options.tool_choice.as_deref() {
        body["tool_choice"] = Value::String(choice.into());
    }
```

Run `rtk cargo build --workspace --all-targets --offline` and add `tool_choice: None,` to each literal it reports.

- [ ] **Step 4: Fill the slot and use it**

In `Agent::compact` (`lib.rs:2494`), before the legacy compaction call, when the placement is `Appended` and a summarizer exists:

```rust
        if self.turn_context_placement() == turn_context::TurnContextPlacement::Appended {
            if let Some(summarizer) = &self.summarizer {
                let tools = self
                    .provider_tool_specs()
                    .into_iter()
                    .map(|tool| davinci_ai::ToolSpec {
                        name: tool.name,
                        description: tool.description,
                        parameters: tool.parameters,
                        constrained_sampling: None,
                    })
                    .collect();
                *summarizer.conversation_slot().lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(compaction::SummarizeConversation {
                        system: self.provider_system_prompt(),
                        tools,
                        messages: self.messages_for_provider(),
                        cache_key: self.last_cache_key.clone(),
                        session_id: self.session.as_ref().map(|s| s.header.id.clone()),
                    });
            }
        }
```

and clear the slot (`= None`) after the compaction call returns, on every path.

`last_cache_key` is a new `pub last_cache_key: Option<String>` field on `Agent`. In `main.rs`, extract the `CacheIdentity { ... }.cache_key()` expression at lines 2290-2306 into `fn root_cache_key(agent: &Agent, model: &davinci_ai::Model, system: &str) -> Option<String>`, use it for `StreamOptions.cache_key`, and store the value in `current.last_cache_key` before the request.

In `live_compaction_summarizer`, capture `let slot = summarizer.conversation_slot();` after creating the `Summarizer` (build it first, then read its slot; if the closure needs the slot, create the `Arc` first and add `Summarizer::with_conversation_slot(f, slot)`). In the closure, before `complete_simple_summarization`:

```rust
        let conversation = slot.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if request.label == "Summarization"
            && std::env::var("DAVINCI_COMPACTION_MODE").as_deref() != Ok("legacy")
        {
            if let Some(conversation) = conversation {
                return conversation_summarization(&parsed, request, &conversation, timeout_ms, max_retries, max_retry_delay_ms, thinking_level, thinking_budgets.clone());
            }
        }
```

`conversation_summarization` resolves model and auth with `resolve_model_and_auth` (Task B1), then calls `live_complete_streaming_with_sink_envelope` with:
- messages: `conversation.messages` plus `ChatMessage::text("user", format!("{SUMMARIZATION_SYSTEM_PROMPT}\n\n{}", summary_instruction_tail(&request.prompt)))`,
- system: `conversation.system`,
- tools: `conversation.tools`,
- `StreamOptions { tool_choice: Some("none".into()), cache_key: conversation.cache_key.clone(), session_id: conversation.session_id.clone(), max_tokens: Some(request.max_tokens), thinking_level, thinking_budgets, timeout_ms, max_retries, max_retry_delay_ms, ..Default::default() }`,

and maps the reply into `SummarizeResponse` the same way `complete_simple_summarization` does. On error it falls back to `complete_simple_summarization`.

The summary covers the whole provider view, including the recent tail that compaction keeps. That duplicates a little context and is accepted; document it.

Add a test in `main.rs` tests with an env summarizer fixture is not possible for the live path; instead add a unit test for the message assembly by extracting `fn conversation_summary_messages(conversation: &SummarizeConversation, prompt: &str) -> Vec<ChatMessage>` and asserting the last message starts with `SUMMARIZATION_SYSTEM_PROMPT` and the earlier messages equal `conversation.messages`.

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test --workspace --offline`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates
git -c core.hooksPath=NUL commit --no-verify -m "feat(compaction): summarize from the cached conversation on OpenAI routes"
```

### Task C2: Remote compaction decision

Gate: `remote_compact` accepted in the probe doc.

This task is a decision record, not code. Codex CLI uses the backend's `/responses/compact` endpoint, which returns an encrypted compaction item. Public reports show it failing for some models (openai/codex issue 19400). Adopting it replaces the TypeScript summary format with an opaque item, which is a large parity divergence.

- [ ] **Step 1:** After C1 ships, run the Task C6 comparison twice on a long task: once with C1 conversation compaction, once with a spike branch that calls `/responses/compact` (use `raw_provider_post` from B1 with the conversation input) and replays the returned items.
- [ ] **Step 2:** Record in `docs/cache/codex-backend-probe.md`: credits for the compaction request, credits for the next five requests, and whether the task still succeeded.
- [ ] **Step 3:** If remote compaction is not at least 20% cheaper over those six requests with equal success, stop here. Otherwise write a separate plan for it; do not merge the spike.

### Task C3: Usage-limit awareness

**Files:**
- Create: `crates/davinci-ai/src/codex_usage.rs`
- Modify: `crates/davinci-ai/src/stream.rs` (record headers from the SSE response before decoding)
- Modify: `crates/davinci-ai/src/provider_retry.rs:55-60` (no retry on `usage_limit_reached`)
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs:1713-1721` (warning line after `turn_outcome`)
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs:771-797` (`/cache-status` shows usage)

**Interfaces:**
- Produces: `pub struct UsageWindow { pub used_percent: f64, pub window_minutes: Option<u64>, pub resets_in_seconds: Option<u64> }`, `pub struct CodexUsageSnapshot { pub primary: Option<UsageWindow>, pub secondary: Option<UsageWindow> }`, `pub fn parse_usage_headers(headers: &[(String, String)]) -> Option<CodexUsageSnapshot>`, `pub fn record(snapshot: CodexUsageSnapshot)`, `pub fn latest() -> Option<CodexUsageSnapshot>`, `pub fn take_warning() -> Option<String>`, `pub fn is_usage_limit_error(message: &str) -> bool`.

Header names: use the names recorded by the B1 probe. The parser below accepts the forms Codex CLI is known to use (`x-codex-primary-used-percent`, `x-codex-primary-window-minutes`, `x-codex-primary-reset-after-seconds`, and the `secondary` equivalents). If the probe recorded different names, change the constants and the test fixtures to the recorded names before merging.

- [ ] **Step 1: Write the failing tests**

Create `crates/davinci-ai/src/codex_usage.rs` with the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    #[test]
    fn both_windows_are_read_from_headers() {
        let snapshot = parse_usage_headers(&[
            h("x-codex-primary-used-percent", "82.5"),
            h("x-codex-primary-window-minutes", "300"),
            h("x-codex-primary-reset-after-seconds", "1200"),
            h("x-codex-secondary-used-percent", "40"),
        ])
        .unwrap();
        let primary = snapshot.primary.unwrap();
        assert_eq!(primary.used_percent, 82.5);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_in_seconds, Some(1200));
        assert_eq!(snapshot.secondary.unwrap().used_percent, 40.0);
    }

    #[test]
    fn no_codex_headers_means_no_snapshot() {
        assert_eq!(parse_usage_headers(&[h("content-type", "text/event-stream")]), None);
    }

    #[test]
    fn a_warning_is_given_once_per_threshold() {
        let _guard = test_lock();
        reset_for_test();
        record(parse_usage_headers(&[h("x-codex-primary-used-percent", "85")]).unwrap());
        assert!(take_warning().unwrap().contains("85%"));
        assert_eq!(take_warning(), None);
        record(parse_usage_headers(&[h("x-codex-primary-used-percent", "86")]).unwrap());
        assert_eq!(take_warning(), None);
        record(parse_usage_headers(&[h("x-codex-primary-used-percent", "96")]).unwrap());
        assert!(take_warning().unwrap().contains("96%"));
    }

    #[test]
    fn the_usage_limit_error_is_recognized() {
        assert!(is_usage_limit_error("{\"error\":{\"type\":\"usage_limit_reached\",\"resets_in_seconds\":900}}"));
        assert!(!is_usage_limit_error("{\"error\":{\"type\":\"rate_limit_exceeded\"}}"));
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-ai codex_usage --offline`
Expected: compile errors.

- [ ] **Step 3: Implement**

Above the tests in `codex_usage.rs`:

```rust
//! ChatGPT plan usage windows reported by the Codex backend. No TypeScript
//! counterpart.

use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub used_percent: f64,
    pub window_minutes: Option<u64>,
    pub resets_in_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct CodexUsageSnapshot {
    pub primary: Option<UsageWindow>,
    pub secondary: Option<UsageWindow>,
}

const WARN_AT: [f64; 2] = [80.0, 95.0];

struct State {
    latest: Option<CodexUsageSnapshot>,
    warned_primary: f64,
    pending: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    latest: None,
    warned_primary: 0.0,
    pending: None,
});

fn window(headers: &[(String, String)], prefix: &str) -> Option<UsageWindow> {
    let get = |suffix: &str| {
        let name = format!("x-codex-{prefix}-{suffix}");
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&name))
            .map(|(_, value)| value.trim().to_string())
    };
    let used_percent = get("used-percent")?.parse().ok()?;
    Some(UsageWindow {
        used_percent,
        window_minutes: get("window-minutes").and_then(|value| value.parse().ok()),
        resets_in_seconds: get("reset-after-seconds").and_then(|value| value.parse().ok()),
    })
}

pub fn parse_usage_headers(headers: &[(String, String)]) -> Option<CodexUsageSnapshot> {
    let snapshot = CodexUsageSnapshot {
        primary: window(headers, "primary"),
        secondary: window(headers, "secondary"),
    };
    (snapshot.primary.is_some() || snapshot.secondary.is_some()).then_some(snapshot)
}

pub fn record(snapshot: CodexUsageSnapshot) {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(primary) = &snapshot.primary {
        let crossed = WARN_AT
            .iter()
            .rev()
            .find(|threshold| primary.used_percent >= **threshold && state.warned_primary < **threshold);
        if let Some(threshold) = crossed {
            state.warned_primary = *threshold;
            let reset = primary
                .resets_in_seconds
                .map(|seconds| format!(" · resets in {} min", seconds.div_ceil(60)))
                .unwrap_or_default();
            state.pending = Some(format!(
                "ChatGPT plan usage at {:.0}% of the current window{reset}",
                primary.used_percent
            ));
        }
        if primary.used_percent < 50.0 {
            state.warned_primary = 0.0;
        }
    }
    state.latest = Some(snapshot);
}

pub fn latest() -> Option<CodexUsageSnapshot> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).latest.clone()
}

pub fn take_warning() -> Option<String> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).pending.take()
}

pub fn is_usage_limit_error(message: &str) -> bool {
    message.contains("usage_limit_reached")
}

#[cfg(test)]
fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
fn reset_for_test() {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.latest = None;
    state.warned_primary = 0.0;
    state.pending = None;
}
```

Add `pub mod codex_usage;` to `davinci-ai/src/lib.rs`.

In `stream.rs::live_complete_streaming_with_sink_envelope`, right after `crate::trace::log(&format!("sse status {}", response.status()));`:

```rust
    if model.api == "openai-codex-responses" {
        let headers: Vec<(String, String)> = response
            .headers_names()
            .into_iter()
            .filter_map(|name| response.header(&name).map(|value| (name.clone(), value.to_string())))
            .collect();
        if let Some(snapshot) = crate::codex_usage::parse_usage_headers(&headers) {
            crate::codex_usage::record(snapshot);
        }
    }
```

WebSocket turns do not pass through this code. If the probe or `PI_AI_TRACE` shows a WebSocket event carrying the same fields, parse it in `codex_ws.rs` into the same `record` call, with a fixture test built from the traced event. Do not guess its shape.

In `provider_retry.rs`, where a 429 is treated as retryable (line 58), return not retryable when `crate::codex_usage::is_usage_limit_error(&error.message)`. Add a test with a 429 `ProviderError` whose message contains `usage_limit_reached` and assert the retry loop makes one attempt.

In `davinci_interactive.rs`, after the `for entry in turn_outcome(...)` loop at line 1713-1721:

```rust
    if let Some(warning) = davinci_ai::codex_usage::take_warning() {
        model.transcript.push(Entry::Gap);
        model
            .transcript
            .push(Entry::tool(State::Attention, "usage", &warning, None));
    }
```

In `/cache-status` (`native_extensions/mod.rs:778`), add `"codexUsage": davinci_ai::codex_usage::latest(),` to the JSON.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-ai --offline` then `rtk cargo test -p davinci-coding-agent --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates
git -c core.hooksPath=NUL commit --no-verify -m "feat(codex): show ChatGPT plan usage and stop retrying a reached limit"
```

### Task C4: Cheaper models for side roles

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:423-436` (`role_models`)
- Modify: `crates/davinci-coding-agent/src/settings.rs` (key `graphEconomyModel`), `main.rs` settings export (`DAVINCI_GRAPH_ECONOMY_MODEL`)

**Interfaces:**
- Produces: `fn economy_role_models(session_model: Option<&str>) -> BTreeMap<Role, String>`.

Rule: only when the session model is an `openai-codex/*` model, no role models are configured, and `DAVINCI_GRAPH_ECONOMY_MODEL` is not `off`. Classifier, Researcher, TestAnalyzer and Historian use the economy model (default `openai-codex/gpt-5.6-luna`). Planner, Writer and Reviewer keep the session model. The session model is used for compaction because C1 needs the same model to hit the cache.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn codex_sessions_send_read_only_roles_to_the_economy_model() {
        let models = economy_role_models_with(Some("openai-codex/gpt-5.6-sol"), None);
        assert_eq!(models.get(&Role::Researcher).map(String::as_str), Some("openai-codex/gpt-5.6-luna"));
        assert!(models.get(&Role::Writer).is_none());
        assert!(models.get(&Role::Reviewer).is_none());
    }

    #[test]
    fn other_providers_and_the_off_switch_get_no_economy_models() {
        assert!(economy_role_models_with(Some("anthropic/claude-opus-4-5"), None).is_empty());
        assert!(economy_role_models_with(Some("openai-codex/gpt-5.6-sol"), Some("off")).is_empty());
    }

    #[test]
    fn the_economy_model_can_be_chosen() {
        let models = economy_role_models_with(Some("openai-codex/gpt-5.6-sol"), Some("openai-codex/gpt-5.4-mini"));
        assert_eq!(models.get(&Role::Classifier).map(String::as_str), Some("openai-codex/gpt-5.4-mini"));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `rtk cargo test -p davinci-coding-agent economy --offline`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
const DEFAULT_ECONOMY_MODEL: &str = "openai-codex/gpt-5.6-luna";

fn economy_role_models_with(
    session_model: Option<&str>,
    setting: Option<&str>,
) -> std::collections::BTreeMap<Role, String> {
    let mut models = std::collections::BTreeMap::new();
    let Some(session_model) = session_model else { return models };
    if !session_model.starts_with("openai-codex/") || setting == Some("off") {
        return models;
    }
    let economy = setting.filter(|value| !value.is_empty()).unwrap_or(DEFAULT_ECONOMY_MODEL);
    if economy == session_model {
        return models;
    }
    for role in [Role::Classifier, Role::Researcher, Role::TestAnalyzer, Role::Historian] {
        models.insert(role, economy.to_string());
    }
    models
}

fn economy_role_models(session_model: Option<&str>) -> std::collections::BTreeMap<Role, String> {
    economy_role_models_with(
        session_model,
        std::env::var("DAVINCI_GRAPH_ECONOMY_MODEL").ok().as_deref(),
    )
}
```

In `role_models`, when the resolved map is empty, return `economy_role_models(self.session_model.as_deref())`. Check the format of `session_model` (it may be `provider/id` or only `id`); if it is only `id`, pass `format!("openai-codex/{id}")` when the session provider is `openai-codex`, and add a test for that form.

Add `graphEconomyModel` to `settings.rs` and export it to `DAVINCI_GRAPH_ECONOMY_MODEL` in `main.rs` like Task A7.

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test -p davinci-coding-agent graph --offline`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src
git -c core.hooksPath=NUL commit --no-verify -m "feat(graph): run read-only roles on an economy model for Codex sessions"
```

### Task C5: Credit estimates in `/cache-status`

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs:771-797`

**Interfaces:**
- Produces: `fn codex_credits_estimate(usd: f64) -> f64` using `const CODEX_CREDITS_PER_USD: f64 = 25.0;`.

The ratio comes from the Codex rate card: GPT-5.6 Sol input is 125 credits per million tokens at $5 per million (25 per dollar), and Luna input is 5 credits per million at $0.20 per million (25 per dollar). Record the source and date in the doc comment, and label the value an estimate in the JSON.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn credits_follow_the_rate_card_ratio() {
        assert_eq!(codex_credits_estimate(5.0), 125.0);
        assert_eq!(codex_credits_estimate(0.2), 5.0);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `rtk cargo test -p davinci-coding-agent credits_follow --offline`
Expected: compile error.

- [ ] **Step 3: Implement**

Add the function and, in `/cache-status`, a field `"codexCreditsEstimate"` computed from the session's total cost in dollars. Find the session cost the same way `/cost` does (search `"cost"` in `native_extensions/mod.rs` or `get_session_stats` in `main.rs`) and pass it through; add `"creditsSource": "estimate: session USD cost x 25, Codex rate card 2026-09"`.

- [ ] **Step 4: Run the tests and commit**

Run: `rtk cargo test -p davinci-coding-agent --offline`
Expected: all pass.

```bash
git add crates/davinci-coding-agent/src/native_extensions/mod.rs
git -c core.hooksPath=NUL commit --no-verify -m "feat(cache): estimate ChatGPT plan credits in /cache-status"
```

### Task C6: Live comparison against Codex CLI

**Files:**
- Create: `scripts/compare-codex.mjs`
- Create: `docs/cache/codex-comparison.md` (results)

**Interfaces:**
- Consumes: `codex exec --json` (Codex CLI) and `davinci --mode json -p` (DaVinci), the same model on both, fresh copies of the same fixture repository per run.

- [ ] **Step 1: Write the script**

Create `scripts/compare-codex.mjs`:

```js
// Runs the same tasks in Codex CLI and DaVinci on the same model and prints
// tokens, estimated credits, wall time and test results. A person runs it; it
// spends ChatGPT plan credits.
// Usage: node scripts/compare-codex.mjs <fixture-repo> <model> <tasks.json>
import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [fixture, model, tasksFile] = process.argv.slice(2);
if (!fixture || !model || !tasksFile) {
  console.error("usage: node scripts/compare-codex.mjs <fixture-repo> <model> <tasks.json>");
  process.exit(2);
}
const tasks = JSON.parse(readFileSync(tasksFile, "utf8"));

function freshCopy() {
  const dir = mkdtempSync(join(tmpdir(), "cmp-"));
  cpSync(fixture, dir, { recursive: true });
  return dir;
}

function sumUsage(lines, pick) {
  const total = { input: 0, cached: 0, output: 0 };
  for (const line of lines) {
    let event;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    const usage = pick(event);
    if (!usage) continue;
    total.input += usage.input ?? 0;
    total.cached += usage.cached ?? 0;
    total.output += usage.output ?? 0;
  }
  return total;
}

function run(harness, task) {
  const cwd = freshCopy();
  const started = Date.now();
  const result =
    harness === "codex"
      ? spawnSync("codex", ["exec", "--json", "-m", model, task.prompt], { cwd, encoding: "utf8", shell: true })
      : spawnSync("davinci", ["--mode", "json", "--model", `openai-codex/${model}`, "--permission-mode", "auto", "-p", task.prompt], { cwd, encoding: "utf8", shell: true });
  const wallMs = Date.now() - started;
  const lines = (result.stdout ?? "").split("\n");
  const usage =
    harness === "codex"
      ? sumUsage(lines, (event) => event.type === "turn.completed" && event.usage
          ? { input: event.usage.input_tokens - (event.usage.cached_input_tokens ?? 0), cached: event.usage.cached_input_tokens, output: event.usage.output_tokens }
          : null)
      : sumUsage(lines, (event) => event.type === "message_end" && event.message?.role === "assistant" && event.message.usage
          ? { input: event.message.usage.input, cached: event.message.usage.cacheRead, output: event.message.usage.output }
          : null);
  const check = spawnSync(task.check, { cwd, encoding: "utf8", shell: true });
  return { harness, task: task.name, wallMs, ...usage, passed: check.status === 0 };
}

const rows = [];
for (const task of tasks) {
  for (const harness of ["codex", "davinci"]) {
    const row = run(harness, task);
    console.log(JSON.stringify(row));
    rows.push(row);
  }
}
console.table(rows);
```

`tasks.json` is a list of `{ "name", "prompt", "check" }` where `check` is a shell command that exits 0 when the task is done (for example `cargo test` or `npm test`). Before the first real run, run one task per harness and confirm both usage pickers return nonzero numbers; the event names in Codex CLI's `--json` output and DaVinci's `--mode json` output must match what the pickers read. Fix the pickers to the observed event shapes if they differ.

- [ ] **Step 2: Run it (a person runs this)**

Use at least five tasks of mixed size and three runs per harness per task. Use Luna for the first pass because it costs the least.

- [ ] **Step 3: Record**

Write `docs/cache/codex-comparison.md` with the date, model, DaVinci commit, Codex CLI version, the tasks, and a table of mean tokens (fresh, cached, output), estimated credits (Luna input 5, cached 0.5, output from the rate card), wall time, and pass rate per harness. State plainly where DaVinci is worse.

- [ ] **Step 4: Full verification, PR, delivery**

Run the Task A8 Step 4 checks, commit, push `openai-efficiency-c-cost-proof`, open the PR with the comparison table in its body, and deliver the executable as in Task A8 Step 6.

```bash
git add scripts/compare-codex.mjs docs/cache/codex-comparison.md docs/openai-efficiency.md
git -c core.hooksPath=NUL commit --no-verify -m "docs(openai): add a live comparison against Codex CLI"
```

---

## Rollback switches (summary for `docs/openai-efficiency.md`)

| Switch | Effect |
| --- | --- |
| `DAVINCI_TURN_CONTEXT=system` | Per-turn state, plan mode, plan and memory go back into the system prompt (pre-Part A). Also turns off the tool freeze. |
| `DAVINCI_PRUNE_PROFILE=default` or `off` | Legacy pruning thresholds, or no pruning. |
| `DAVINCI_OPENAI_VERBOSITY=low|medium|high` | `text.verbosity`. |
| `DAVINCI_REASONING_SUMMARY=auto|concise|detailed|none` | `reasoning.summary`; `none` omits it. |
| `DAVINCI_OPENAI_GRAMMAR_TOOLS=0` | `apply_patch` goes back to a JSON function. |
| `DAVINCI_ADDITIONAL_TOOLS` unset | Late tools are not sent as `additional_tools` items. |
| `DAVINCI_COMPACTION_MODE=legacy` | Compaction sends the serialized conversation as before. |
| `DAVINCI_GRAPH_ECONOMY_MODEL=off` | Graph roles all use the session model. |
| Existing `PI_OPENAI_CACHE_*` switches | Unchanged. |

## Items considered and dropped

- **Staggering or prewarming parallel graph workers.** The shared worker prefix is a few thousand tokens and only 2 to 3 researchers run at once, so the saving is small next to the added wall time. Revisit only if the C6 comparison shows graph runs dominate spend.
- **Moving AGENTS.md out of the instructions.** It is stable within a session, so it does not break the cache.
- **Removing the provider identity suffix.** It changes only with the model or thinking level, and both already change the cache.

## Self-review

- Coverage: E1 (A3), E2 (A3), E3 (A3), E4 (A4), E5 (A5), E6 (A6, B5), E7 (B2), E8 (B2, B4), E9 (B3), E10 (B4), E11 (B6), E12 (C1, C2), E13 (C3), E14 (C4), E15 (A7), E16 (A8, C6).
- Gated tasks (B2, B4, B5, C1, C2, C3 header names) name the probe case that gates them and what to do when it is rejected.
- Names used across tasks: `TurnContextPlacement`, `turn_context_placement()`, `turn_context_placement_override`, `commit_turn_context`, `freeze_tools_for_cache`, `split_turn_prompt`, `first_prefix_break`, `wire_body_for_next_request`, `codex_test_model`, `ResponsesInputOptions`, `openai_responses_input_with`, `attach_native_items`, `NATIVE_ITEMS_KEY`, `NATIVE_MODEL_KEY`, `APPLY_PATCH_LARK`, `raw_provider_post`, `RawProviderReply`, `resolve_model_and_auth`, `SummarizeConversation`, `summary_instruction_tail`, `codex_usage::{parse_usage_headers, record, latest, take_warning, is_usage_limit_error}`.
