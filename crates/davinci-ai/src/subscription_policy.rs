//! Subscription-only admission is separate from public API token or USD caps.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexSubscriptionPolicy {
    pub model: String,
    pub effort: String,
}

impl CodexSubscriptionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.model.is_empty()
            || self.model.len() > 256
            || !matches!(self.effort.as_str(), "low" | "medium" | "high" | "xhigh")
        {
            return Err("subscription policy requires a pinned model and supported effort".into());
        }
        Ok(())
    }

    pub fn validate_request(
        &self,
        model: &crate::Model,
        auth: &crate::ResolvedAuth,
        options: &crate::StreamOptions,
        body: &Value,
        url: &str,
    ) -> Result<(), String> {
        self.validate()?;
        // SSE is selected explicitly: auto transport may silently retry or
        // downgrade after a websocket failure. No fixture or URL override is used.
        if model.provider != "openai-codex"
            || model.api != "openai-codex-responses"
            || model.id != self.model
            || body.get("model").and_then(Value::as_str) != Some(&self.model)
            || body.pointer("/reasoning/effort").and_then(Value::as_str) != Some(&self.effort)
            || !auth.source.eq_ignore_ascii_case("oauth")
            || auth.api_key.as_deref().is_none_or(str::is_empty)
            || url != "https://api.openai.com/v1/responses"
            || body.get("store").and_then(Value::as_bool) != Some(false)
            || body.get("stream").and_then(Value::as_bool) != Some(true)
            || options.transport.as_deref() != Some("sse")
            || options.max_retries.unwrap_or(0) != 0
            || ["max_output_tokens", "max_completion_tokens", "max_tokens"]
                .iter()
                .any(|key| body.get(key).is_some())
        {
            return Err("subscription-only admission denied: verified ChatGPT-plan OAuth, public /v1/responses, pinned model/effort and SSE transport required; API-key billing and fallback are forbidden".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_policy_rejects_every_billing_or_baseline_change() {
        let policy = CodexSubscriptionPolicy {
            model: "gpt-6-luna".into(),
            effort: "high".into(),
        };
        let model: crate::Model = serde_json::from_value(serde_json::json!({
            "id":"gpt-6-luna", "name":"fixture", "api":"openai-codex-responses",
            "provider":"openai-codex", "baseUrl":"https://api.openai.com/v1", "reasoning":true,
            "input":["text"], "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
            "contextWindow":1000,"maxTokens":100
        }))
        .unwrap();
        let auth = crate::ResolvedAuth {
            api_key: Some("fixture-plan-access".into()),
            headers: Default::default(),
            source: "OAuth".into(),
        };
        let options = crate::StreamOptions {
            transport: Some("sse".into()),
            max_retries: Some(0),
            ..Default::default()
        };
        let body = serde_json::json!({
            "model":"gpt-6-luna",
            "reasoning":{"effort":"high"},
            "store":false,
            "stream":true
        });
        let url = "https://api.openai.com/v1/responses";
        assert!(policy
            .validate_request(&model, &auth, &options, &body, url)
            .is_ok());
        for index in 0..11 {
            let mut m = model.clone();
            let mut a = auth.clone();
            let mut o = options.clone();
            let mut b = body.clone();
            let mut u = url;
            match index {
                0 => m.provider = "openai".into(),
                1 => m.api = "openai-responses".into(),
                2 => m.id = "other".into(),
                3 => b["model"] = "other".into(),
                4 => b["reasoning"]["effort"] = "low".into(),
                5 => a.source = "configured API key".into(),
                6 => a.api_key = None,
                7 => u = "https://chatgpt.com/backend-api/codex/responses",
                8 => o.transport = None,
                9 => o.max_retries = Some(1),
                _ => b["max_output_tokens"] = 100.into(),
            }
            assert!(
                policy.validate_request(&m, &a, &o, &b, u).is_err(),
                "case {index}"
            );
        }
    }
}
