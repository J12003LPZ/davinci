//! Codex Capabilities snapshot and discovery matching §6.1.
//! Immutable per-lineage snapshot derived from authenticated backend, model metadata, and conservative probes.

use crate::catalog::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchMode {
    None,
    Emulated,
    Native,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexCapabilities {
    pub responses_items: bool,
    pub websocket_transport: bool,
    pub incremental_continuation: bool,
    pub generate_false_prewarm: bool,
    pub stream_multiplexing: bool,
    pub turn_state_headers: bool,
    pub encrypted_reasoning: bool,
    pub assistant_phases: bool,
    pub custom_grammar_tools: bool,
    pub tool_namespaces: bool,
    pub tool_search: ToolSearchMode,
    pub explicit_cache_breakpoints: bool,
    pub server_side_compaction: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    pub zero_data_retention: bool,
}

impl Default for CodexCapabilities {
    fn default() -> Self {
        Self {
            responses_items: false,
            websocket_transport: false,
            incremental_continuation: false,
            generate_false_prewarm: false,
            stream_multiplexing: false,
            turn_state_headers: false,
            encrypted_reasoning: false,
            assistant_phases: false,
            custom_grammar_tools: false,
            tool_namespaces: false,
            tool_search: ToolSearchMode::None,
            explicit_cache_breakpoints: false,
            server_side_compaction: false,
            service_tier: None,
            zero_data_retention: false,
        }
    }
}

impl CodexCapabilities {
    /// Create a capabilities snapshot for ChatGPT/Codex OAuth backend.
    pub fn for_chatgpt_codex(model: &Model) -> Self {
        let grammar = model
            .compat
            .get("supportsOpenAIGrammarTools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // Enabled by default for ChatGPT Codex profile
        Self {
            responses_items: true,
            websocket_transport: true,
            incremental_continuation: true,
            generate_false_prewarm: true,
            stream_multiplexing: true,
            turn_state_headers: true,
            encrypted_reasoning: true,
            assistant_phases: true,
            custom_grammar_tools: grammar,
            tool_namespaces: true,
            tool_search: ToolSearchMode::Emulated,
            explicit_cache_breakpoints: true,
            server_side_compaction: false,
            service_tier: None,
            zero_data_retention: false,
        }
    }

    /// Create a capabilities snapshot for public OpenAI Responses API.
    pub fn for_public_responses(model: &Model) -> Self {
        let grammar = model
            .compat
            .get("supportsOpenAIGrammarTools")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let explicit_cache = model
            .compat
            .get("supportsExplicitPromptCacheMode")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Self {
            responses_items: true,
            websocket_transport: true,
            incremental_continuation: true,
            generate_false_prewarm: false,
            stream_multiplexing: false,
            turn_state_headers: false,
            encrypted_reasoning: false,
            assistant_phases: false,
            custom_grammar_tools: grammar,
            tool_namespaces: false,
            tool_search: ToolSearchMode::Emulated,
            explicit_cache_breakpoints: explicit_cache,
            server_side_compaction: false,
            service_tier: None,
            zero_data_retention: false,
        }
    }

    /// Resolve capabilities from model, backend URL, and authentication type.
    pub fn resolve(model: &Model, base_url: Option<&str>, is_oauth: bool) -> Self {
        let url = base_url
            .or(model.base_url.as_deref())
            .unwrap_or(crate::codex::DEFAULT_CODEX_BASE_URL);
        if is_oauth || url.contains("chatgpt.com") || model.api == "openai-codex-responses" {
            Self::for_chatgpt_codex(model)
        } else if model.api == "openai-responses" || model.api == "azure-openai-responses" {
            Self::for_public_responses(model)
        } else {
            Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Model, ModelCost};
    use serde_json::json;

    fn test_model(api: &str) -> Model {
        Model {
            id: "gpt-5-codex".into(),
            name: "GPT-5 Codex".into(),
            api: api.into(),
            provider: "openai".into(),
            base_url: None,
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
            compat: json!({}),
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    #[test]
    fn resolves_oauth_codex_profile() {
        let model = test_model("openai-codex-responses");
        let caps =
            CodexCapabilities::resolve(&model, Some("https://chatgpt.com/backend-api"), true);
        assert!(caps.responses_items);
        assert!(caps.websocket_transport);
        assert!(caps.incremental_continuation);
        assert!(caps.generate_false_prewarm);
        assert!(caps.encrypted_reasoning);
        assert!(caps.assistant_phases);
        assert!(caps.custom_grammar_tools);
        assert_eq!(caps.tool_search, ToolSearchMode::Emulated);
    }

    #[test]
    fn resolves_public_responses_profile() {
        let model = test_model("openai-responses");
        let caps = CodexCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        assert!(caps.responses_items);
        assert!(caps.websocket_transport);
        assert!(caps.incremental_continuation);
        assert!(!caps.generate_false_prewarm);
        assert!(!caps.encrypted_reasoning);
        assert!(!caps.assistant_phases);
    }

    #[test]
    fn default_capabilities_for_other_providers() {
        let model = test_model("anthropic-messages");
        let caps = CodexCapabilities::resolve(&model, Some("https://api.anthropic.com"), false);
        assert!(!caps.responses_items);
        assert!(!caps.websocket_transport);
    }
}
