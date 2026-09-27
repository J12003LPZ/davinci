//! Capability-scoped OpenAI Responses tool projection.
//!
//! The provider wire kind is part of the request contract.  This module keeps
//! the decision in one place so request construction, history replay, and
//! ledger migration cannot independently decide whether `apply_patch` was a
//! function or a freeform custom tool.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::catalog::Model;
use crate::codex_capabilities::CodexCapabilities;
use crate::{ChatMessage, ToolSpec};

pub const APPLY_PATCH_ROLLOUT_ENV: &str = "PI_CODEX_APPLY_PATCH";
pub const RESPONSES_TOOL_WIRE_KIND_KEY: &str = "responsesToolWireKind";
pub const RESPONSES_TOOL_WIRE_KINDS_KEY: &str = "responsesToolWireKinds";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesToolWireKind {
    Function,
    Custom,
}

impl ResponsesToolWireKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Custom => "custom",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "function" => Some(Self::Function),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedResponsesTools {
    pub tools: Vec<Value>,
    pub custom_tool_names: Vec<String>,
    pub schema_digest: String,
    pub capabilities: CodexCapabilities,
}

impl ResolvedResponsesTools {
    pub fn custom_tool_name_refs(&self) -> Vec<&str> {
        self.custom_tool_names
            .iter()
            .map(String::as_str)
            .collect()
    }

    pub fn is_custom(&self, name: &str) -> bool {
        self.custom_tool_names.iter().any(|candidate| candidate == name)
    }

    pub fn wire_tools(&self) -> Vec<Value> {
        self.tools.clone()
    }
}

/// Resolve the production rollout using the process flag.  An absent flag is
/// enabled so supported Codex backends exercise the new contract by default;
/// explicit false-like values are a safe rollback to JSON function tools.
pub fn resolve_responses_tools(
    model: &Model,
    base_url: Option<&str>,
    is_oauth: bool,
    tools: &[ToolSpec],
) -> ResolvedResponsesTools {
    resolve_responses_tools_with_preference(
        model,
        base_url,
        is_oauth,
        tools,
        rollout_enabled_from_env(),
    )
}

/// Resolve with an explicit rollout preference.  Tests and callers that have
/// already evaluated configuration use this entry point to avoid rereading
/// environment state.
pub fn resolve_responses_tools_with_preference(
    model: &Model,
    base_url: Option<&str>,
    is_oauth: bool,
    tools: &[ToolSpec],
    rollout_enabled: bool,
) -> ResolvedResponsesTools {
    let capabilities = CodexCapabilities::resolve(model, base_url, is_oauth);
    let custom_enabled = rollout_enabled && capabilities.custom_grammar_tools;
    let mut sorted_tools = tools.to_vec();
    sorted_tools.sort_by(|left, right| left.name.cmp(&right.name));

    let mut custom_tool_names = Vec::new();
    let wire_tools = sorted_tools
        .iter()
        .map(|tool| {
            if custom_enabled && tool.name == "apply_patch" {
                custom_tool_names.push(tool.name.clone());
                custom_tool_value(tool)
            } else {
                function_tool_value(tool)
            }
        })
        .collect::<Vec<_>>();
    let schema_digest = schema_digest(&wire_tools);

    ResolvedResponsesTools {
        tools: wire_tools,
        custom_tool_names,
        schema_digest,
        capabilities,
    }
}

pub fn rollout_enabled_from_env() -> bool {
    match std::env::var(APPLY_PATCH_ROLLOUT_ENV) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no" | "disabled" | "disable"
        ),
        Err(_) => true,
    }
}

pub fn function_tool_value(tool: &ToolSpec) -> Value {
    let mut function = json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
    });
    if crate::stream::resolve_json_schema_strict_sampling(tool).unwrap_or(false) {
        function["strict"] = Value::Bool(true);
    }
    function
}

fn custom_tool_value(tool: &ToolSpec) -> Value {
    json!({
        "type": "custom",
        "name": tool.name,
        "description": tool.description,
        "format": {
            "type": "grammar",
            "syntax": "lark",
            "definition": crate::apply_patch_grammar::APPLY_PATCH_LARK,
        }
    })
}

fn schema_digest(tools: &[Value]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"davinci.responses-tools.v1\0");
    hasher.update(serde_json::to_vec(tools).unwrap_or_default());
    format!("{:x}", hasher.finalize())
}

pub fn set_wire_kind(extra: &mut Map<String, Value>, call_id: &str, kind: ResponsesToolWireKind) {
    let mut kinds = extra
        .remove(RESPONSES_TOOL_WIRE_KINDS_KEY)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    kinds.insert(call_id.to_string(), Value::String(kind.as_str().into()));
    extra.insert(
        RESPONSES_TOOL_WIRE_KINDS_KEY.into(),
        Value::Object(kinds),
    );
}

pub fn set_single_wire_kind(
    extra: &mut Map<String, Value>,
    kind: ResponsesToolWireKind,
) {
    extra.insert(
        RESPONSES_TOOL_WIRE_KIND_KEY.into(),
        Value::String(kind.as_str().into()),
    );
}

fn explicit_wire_kind(message: &ChatMessage, call_id: Option<&str>) -> Option<ResponsesToolWireKind> {
    if let Some(kinds) = message
        .extra
        .get(RESPONSES_TOOL_WIRE_KINDS_KEY)
        .and_then(Value::as_object)
    {
        if let Some(call_id) = call_id {
            let short_id = call_id.split_once('|').map(|(prefix, _)| prefix);
            for candidate in [Some(call_id), short_id].into_iter().flatten() {
                if let Some(kind) = kinds
                    .get(candidate)
                    .and_then(Value::as_str)
                    .and_then(ResponsesToolWireKind::parse)
                {
                    return Some(kind);
                }
            }
        }
    }
    message
        .extra
        .get(RESPONSES_TOOL_WIRE_KIND_KEY)
        .and_then(Value::as_str)
        .and_then(ResponsesToolWireKind::parse)
}

/// Determine the wire kind for one historical call.  Explicit persisted
/// metadata always wins; the name/rollout fallback exists only for transcripts
/// written before metadata was introduced.
pub fn message_tool_wire_kind(
    message: &ChatMessage,
    call_id: Option<&str>,
    tool_name: Option<&str>,
    _custom_tool_names: &[&str],
) -> ResponsesToolWireKind {
    if let Some(kind) = explicit_wire_kind(message, call_id) {
        return kind;
    }
    // A transcript without metadata predates the freeform rollout.  Keep its
    // original JSON-function representation; the current request's tool
    // visibility must never rewrite historical call/result pairs.
    let _ = tool_name;
    ResponsesToolWireKind::Function
}

pub fn custom_tool_call_arguments(arguments: &Value) -> Option<&str> {
    arguments.get("input").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Model, ModelCost};

    fn model(api: &str, provider: &str, base_url: Option<&str>) -> Model {
        Model {
            id: "gpt-5-codex".into(),
            name: "test".into(),
            api: api.into(),
            provider: provider.into(),
            base_url: base_url.map(str::to_string),
            reasoning: true,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 200_000,
            max_tokens: 16_384,
            compat: serde_json::json!({}),
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    fn tools() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "read".into(),
                description: "Read a file".into(),
                parameters: serde_json::json!({"type":"object"}),
                constrained_sampling: None,
            },
            ToolSpec {
                name: "apply_patch".into(),
                description: "Apply a patch".into(),
                parameters: serde_json::json!({"type":"object"}),
                constrained_sampling: None,
            },
        ]
    }

    #[test]
    fn supported_codex_uses_one_custom_apply_patch() {
        let resolved = resolve_responses_tools_with_preference(
            &model("openai-codex-responses", "openai-codex", None),
            Some("https://chatgpt.com/backend-api"),
            true,
            &tools(),
            true,
        );
        assert_eq!(resolved.custom_tool_names, vec!["apply_patch"]);
        assert_eq!(resolved.tools.iter().filter(|tool| tool["name"] == "apply_patch").count(), 1);
        assert_eq!(resolved.tools.iter().filter(|tool| tool["type"] == "function" && tool["name"] == "apply_patch").count(), 0);
        assert_eq!(resolved.tools.iter().filter(|tool| tool["type"] == "custom").count(), 1);
    }

    #[test]
    fn unsupported_or_disabled_context_stays_functional() {
        let cases = [
            (model("openai-codex-responses", "openai-codex", None), Some("https://proxy.test/backend-api"), true),
            (model("azure-openai-responses", "azure", None), Some("https://api.openai.com/v1"), false),
            (model("openai-responses", "other", None), Some("https://api.openai.com/v1"), false),
            (model("openai-codex-responses", "openai-codex", None), Some("not a url"), true),
        ];
        for (model, url, oauth) in cases {
            let resolved = resolve_responses_tools_with_preference(&model, url, oauth, &tools(), true);
            assert!(resolved.custom_tool_names.is_empty());
            assert_eq!(resolved.tools.iter().filter(|tool| tool["name"] == "apply_patch").count(), 1);
            assert_eq!(resolved.tools.iter().find(|tool| tool["name"] == "apply_patch").unwrap()["type"], "function");
        }
        let disabled = resolve_responses_tools_with_preference(
            &model("openai-codex-responses", "openai-codex", None),
            Some("https://chatgpt.com/backend-api"),
            true,
            &tools(),
            false,
        );
        assert!(disabled.custom_tool_names.is_empty());
        assert_eq!(disabled.tools.iter().find(|tool| tool["name"] == "apply_patch").unwrap()["type"], "function");
    }

    #[test]
    fn schema_digest_tracks_effective_wire_grammar_and_rollout() {
        let model = model("openai-codex-responses", "openai-codex", None);
        let url = Some("https://chatgpt.com/backend-api");
        let custom = resolve_responses_tools_with_preference(&model, url, true, &tools(), true);
        let functions =
            resolve_responses_tools_with_preference(&model, url, true, &tools(), false);
        assert_ne!(
            custom.schema_digest, functions.schema_digest,
            "cache identity must follow the actual wire tool kind and grammar"
        );
        assert_eq!(
            custom.schema_digest,
            resolve_responses_tools_with_preference(&model, url, true, &tools(), true)
                .schema_digest,
            "digest computation must remain deterministic when inputs match"
        );
    }

    #[test]
    fn explicit_wire_kind_overrides_legacy_name_fallback() {
        let mut message = ChatMessage::tool_result("call|item", "apply_patch", "ok", false);
        set_wire_kind(&mut message.extra, "call|item", ResponsesToolWireKind::Function);
        assert_eq!(
            message_tool_wire_kind(&message, Some("call|item"), Some("apply_patch"), &[]),
            ResponsesToolWireKind::Function
        );
    }
}
