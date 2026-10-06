//! Per-turn harness context delivered as an appended message instead of a
//! system prompt rewrite. No TypeScript counterpart; Codex-style routes keep
//! stable instructions and append changing harness state after the user turn.

use serde::{Deserialize, Serialize};

use crate::prompt::manifest::hash_text;
use crate::prompt::provider::{prompt_model_family, PromptModelFamily};

pub const TURN_CONTEXT_CUSTOM_TYPE: &str = "davinci.turn_context";

const HEADER: &str = "Harness context for the user request above. It is not a new user request.";

/// What the model is told once when Plan Mode ends: how to carry out an
/// approved plan. Sent with the transition, never in the system prompt, so
/// toggling modes leaves the cached prefix alone.
pub const EXECUTE_MODE_NOTICE: &str = "Plan Mode has ended. If the user approved a plan, implement it: follow its steps in dependency order, run each step's verify checks, and record progress with update_plan. If evidence shows a step is wrong, stop and say so with a proposed revision rather than silently changing the plan. Without an approved plan, work on the user's request directly.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnContextPlacement {
    /// Legacy: per-turn state is part of the system prompt.
    SystemPrompt,
    /// Per-turn state is appended to the conversation when it changes.
    Appended,
}

/// Routes whose provider benefits materially from an append-only prompt prefix.
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
    /// `path#tag` keys of the files this message attached
    /// (`prompt::named_files`); not part of the state hash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named_files: Vec<String>,
}

impl TurnContextState {
    /// The state carried by the newest turn-context message. After compaction
    /// removes it, the default makes the next turn restate everything.
    pub fn from_messages(messages: &[davinci_ai::ChatMessage]) -> Self {
        messages
            .iter()
            .rev()
            .find(|message| {
                message.role == "custom"
                    && message
                        .extra
                        .get("customType")
                        .and_then(|value| value.as_str())
                        == Some(TURN_CONTEXT_CUSTOM_TYPE)
            })
            .and_then(|message| message.extra.get("details"))
            .and_then(|details| serde_json::from_value(details.clone()).ok())
            .unwrap_or_default()
    }
}

pub struct TurnContextInput<'a> {
    /// Already-rendered dynamic prompt sections, including the runtime tag.
    pub runtime_state: &'a str,
    pub plan_mode_appendix: Option<&'a str>,
    pub living_plan: Option<(u64, &'a str)>,
    /// The rendered `<named_files>` block of this user turn, if any.
    pub named_files: Option<&'a str>,
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
        sections.push(input.runtime_state.trim().to_string());
    }
    if state_changed {
        match input.plan_mode_appendix {
            Some(appendix) => {
                sections.push(format!("<plan_mode>\n{}\n</plan_mode>", appendix.trim()))
            }
            None if previous.plan_mode => {
                sections.push(format!("<plan_mode>\n{EXECUTE_MODE_NOTICE}\n</plan_mode>"))
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

    if let Some(named_files) = input
        .named_files
        .map(str::trim)
        .filter(|block| !block.is_empty())
    {
        sections.push(named_files.to_string());
    }

    if let Some(memory) = input
        .memory
        .map(str::trim)
        .filter(|memory| !memory.is_empty())
    {
        sections.push(format!("<memory>\n{memory}\n</memory>"));
    }
    if sections.is_empty() {
        return None;
    }

    let text = format!(
        "<turn_context>\n{HEADER}\n{}\n</turn_context>",
        sections.join("\n")
    );
    Some((
        text,
        TurnContextState {
            state_hash: Some(state_hash),
            plan_revision,
            plan_mode,
            named_files: Vec::new(),
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
            named_files: None,
            memory,
        }
    }

    #[test]
    fn first_turn_states_runtime_state() {
        let (text, state) = render_turn_context(
            &TurnContextState::default(),
            &input(
                "<runtime_state>\nPermission mode: Ask.\n</runtime_state>",
                None,
            ),
        )
        .unwrap();
        assert!(text.contains("<runtime_state>\nPermission mode: Ask.\n</runtime_state>"));
        assert!(state.state_hash.is_some());
    }

    #[test]
    fn unchanged_state_without_memory_adds_nothing() {
        let (_, state) =
            render_turn_context(&TurnContextState::default(), &input("same", None)).unwrap();
        assert_eq!(render_turn_context(&state, &input("same", None)), None);
    }

    #[test]
    fn memory_is_added_without_repeating_state() {
        let (_, state) =
            render_turn_context(&TurnContextState::default(), &input("same", None)).unwrap();
        let (text, _) =
            render_turn_context(&state, &input("same", Some("prefers small diffs"))).unwrap();
        assert!(text.contains("<memory>\nprefers small diffs\n</memory>"));
        assert!(!text.contains("<runtime_state>"));
    }

    #[test]
    fn leaving_plan_mode_is_said_once() {
        let entering = TurnContextInput {
            runtime_state: "Permission mode: Plan Mode (read-only).",
            plan_mode_appendix: Some("PLAN APPENDIX"),
            living_plan: None,
            named_files: None,
            memory: None,
        };
        let (_, state) = render_turn_context(&TurnContextState::default(), &entering).unwrap();
        let (text, state) =
            render_turn_context(&state, &input("Permission mode: Ask.", None)).unwrap();
        assert!(text.contains("Plan Mode has ended."));
        // The exit carries how to execute an approved plan, once.
        assert!(text.contains("dependency order"), "{text}");
        assert!(text.contains("update_plan"), "{text}");
        assert!(text.contains("proposed revision"), "{text}");
        assert!(!state.plan_mode);
        assert!(render_turn_context(&state, &input("Permission mode: Ask.", None)).is_none());
    }

    #[test]
    fn named_files_ride_along_without_restating_state() {
        let (_, state) =
            render_turn_context(&TurnContextState::default(), &input("same", None)).unwrap();
        let with_files = TurnContextInput {
            runtime_state: "same",
            plan_mode_appendix: None,
            living_plan: None,
            named_files: Some(
                "<named_files untrusted=\"true\">
x
</named_files>",
            ),
            memory: None,
        };
        let (text, next) = render_turn_context(&state, &with_files).unwrap();
        assert!(text.contains("<named_files untrusted=\"true\">"));
        assert!(!text.contains("same"));
        // The block is not state: the next turn does not restate it.
        assert_eq!(next.state_hash, state.state_hash);
        assert_eq!(render_turn_context(&next, &input("same", None)), None);
    }

    #[test]
    fn plan_revision_is_sent_once() {
        let with_plan = TurnContextInput {
            runtime_state: "s",
            plan_mode_appendix: None,
            living_plan: Some((3, "step 1")),
            named_files: None,
            memory: None,
        };
        let (text, state) = render_turn_context(&TurnContextState::default(), &with_plan).unwrap();
        assert!(text.contains("<living_plan revision=\"3\">"));
        assert_eq!(render_turn_context(&state, &with_plan), None);
    }

    #[test]
    fn state_is_recovered_from_newest_turn_context_message() {
        let mut message = davinci_ai::ChatMessage::text("custom", "x");
        message
            .extra
            .insert("customType".into(), TURN_CONTEXT_CUSTOM_TYPE.into());
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
