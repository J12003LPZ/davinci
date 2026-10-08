//! Subscription-only adapter over DaVinci's existing provider transport.
use super::{context::DESIGN_POLICY, error::*};
use davinci_agent::{Agent, CompleteOutput};
use davinci_ai::{ChatMessage, Model, ResolvedAuth, ToolSpec};

fn ensure_online() -> DesignResult<()> {
    // Match the CLI's offline flag/environment semantics at every transport entry.
    if std::env::var("PI_OFFLINE")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
    {
        return Err(DesignError::Denied(
            "design provider requests are disabled in offline mode".into(),
        ));
    }
    Ok(())
}

pub struct DesignModelRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub deadline_ms: u64,
    pub abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
/// Trusted host transport injection for SDKs and recorded-response tests.
/// Browser/model payloads cannot select or construct this object.
pub trait DesignModel {
    fn validate(&self, agent: &Agent) -> DesignResult<()>;
    fn supports_images(&self) -> bool;
    fn complete(
        &mut self,
        agent: &Agent,
        request: &DesignModelRequest,
    ) -> Result<CompleteOutput, String>;
}

pub(crate) fn complete_checked(
    agent: &mut Agent,
    context: &super::admission::AuthorizedDesignContext,
    model: &mut dyn DesignModel,
    request: &DesignModelRequest,
) -> DesignResult<CompleteOutput> {
    use std::sync::atomic::{AtomicBool, Ordering};
    // Resolve the grant before spawning: an early error must not leave the
    // scoped monitor waiting until the original provider deadline.
    let grant = context.grant()?;
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stopped.load(Ordering::Acquire) {
                if context
                    .check("design_generate", &serde_json::json!({}))
                    .is_err()
                    || davinci_session::now_ms() >= request.deadline_ms
                {
                    request.abort.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
        let result = agent
            .complete_host_operation(grant, "design", |owner| model.complete(owner, request))
            .map_err(DesignError::MissingCapability);
        stopped.store(true, Ordering::Release);
        result
    })
}

pub struct SubscriptionModel {
    model: Model,
    auth: ResolvedAuth,
    effort: davinci_protocol::ThinkingLevel,
    /// Renew the stored login before each request: only for the route
    /// [`Self::configured`] built from it. A host that injects credentials
    /// keeps them as given.
    renew: bool,
}
impl SubscriptionModel {
    pub fn from_host(agent: &Agent, model: Model, auth: ResolvedAuth) -> DesignResult<Self> {
        let route = Self {
            model,
            auth,
            effort: agent.request_thinking_level(),
            renew: false,
        };
        route.validate(agent)?;
        Ok(route)
    }
    pub fn configured(agent: &Agent) -> DesignResult<Self> {
        ensure_online()?;
        if agent.provider != "openai-codex" {
            return Err(DesignError::Denied(
                "design generation requires the selected OpenAI subscription route".into(),
            ));
        }
        let directory = davinci_session::default_agent_dir();
        let mut models = davinci_ai::merge_models_store(
            &davinci_ai::load_builtin_models(),
            &davinci_ai::load_models_store(&directory),
        );
        models = davinci_ai::overlay_codex_models(&models, &directory);
        let config = davinci_ai::ModelConfig::load(&davinci_ai::models_json_path(&directory));
        models = davinci_ai::apply_models_config(&models, &config)
            .map_err(DesignError::MissingCapability)?;
        let model = davinci_ai::find_model(&models, &agent.provider, &agent.model_id)
            .cloned()
            .ok_or_else(|| {
                DesignError::MissingCapability(
                    "selected model capabilities are absent; refresh the model catalog explicitly"
                        .into(),
                )
            })?;
        let auth = subscription_auth(&agent.provider)?;
        let mut route = Self::from_host(agent, model, auth)?;
        route.renew = true;
        Ok(route)
    }
}

/// How long a token must still be valid before a design request uses it: a
/// generation request can run for minutes.
const AUTH_MIN_VALIDITY_MS: u64 = 15 * 60 * 1000;

/// The stored ChatGPT-plan login, renewed through its refresh token when it
/// is close to expiry, the way every other DaVinci request renews it. No
/// environment keys, config commands, new login or API-key fallback.
fn subscription_auth(provider: &str) -> DesignResult<ResolvedAuth> {
    let mut storage = davinci_ai::AuthStorage::create().map_err(|_| {
        DesignError::MissingCapability("subscription credentials are unavailable".into())
    })?;
    // A failed refresh leaves the stored token; resolving below decides.
    let _ = storage.maybe_refresh(provider, davinci_ai::now_ms(), AUTH_MIN_VALIDITY_MS, false);
    davinci_ai::resolve_provider_auth(provider, &storage, &Default::default(), false).ok_or_else(
        || {
            DesignError::MissingCapability(
                "subscription login is unavailable or expired; sign in through DaVinci".into(),
            )
        },
    )
}
fn design_stream_options(
    agent: &Agent,
    effort: davinci_protocol::ThinkingLevel,
    request: &DesignModelRequest,
    remaining: u64,
    output: u64,
) -> davinci_ai::StreamOptions {
    davinci_ai::StreamOptions {
        service_tier: Some(agent.service_tier),
        thinking_level: Some(effort),
        thinking_budgets: agent.thinking_budgets.clone(),
        timeout_ms: Some(
            agent
                .provider_timeout_ms
                .unwrap_or(remaining)
                .min(remaining),
        ),
        max_retries: Some(0),
        max_tokens: Some(output),
        // Subscription-only admission takes SSE: auto may retry or fall back
        // to another transport after a websocket failure.
        transport: Some("sse".into()),
        abort_signal: Some(request.abort.clone()),
        session_id: agent
            .session
            .as_ref()
            .map(|session| session.header.id.clone()),
        install_telemetry: Some(agent.install_telemetry),
        ..Default::default()
    }
}

impl DesignModel for SubscriptionModel {
    fn validate(&self, agent: &Agent) -> DesignResult<()> {
        if agent.provider != "openai-codex"
            || self.model.provider != agent.provider
            || self.model.id != agent.model_id
            || self.model.api != "openai-codex-responses"
            || !self.auth.source.eq_ignore_ascii_case("oauth")
            || self.model.base_url.as_deref().is_some_and(|url| {
                url.trim_end_matches('/')
                    != davinci_ai::DEFAULT_CODEX_BASE_URL.trim_end_matches('/')
            })
            || !self.model.headers.is_empty()
            || self.model.max_tokens == 0
            || agent.request_thinking_level() != self.effort
        {
            return Err(DesignError::Denied(
                "design route, model or effort differs from the authorized subscription session"
                    .into(),
            ));
        }
        Ok(())
    }
    fn supports_images(&self) -> bool {
        self.model.input.iter().any(|input| input == "image") && !self.model.input.is_empty()
    }
    fn complete(
        &mut self,
        agent: &Agent,
        request: &DesignModelRequest,
    ) -> Result<CompleteOutput, String> {
        ensure_online().map_err(|error| error.to_string())?;
        // Runs take minutes and retry: renew the login before each request
        // rather than once at the start. Only the same OAuth route passes.
        if self.renew {
            self.auth = subscription_auth(&agent.provider).map_err(|e| e.to_string())?;
        }
        self.validate(agent).map_err(|e| e.to_string())?;
        let remaining = request
            .deadline_ms
            .saturating_sub(davinci_session::now_ms());
        if remaining == 0 {
            return Err("design deadline reached".into());
        }
        let output = agent
            .provider_context_budget()
            .output_limit()
            .min(self.model.max_tokens);
        let tokens = request
            .messages
            .iter()
            .map(davinci_agent::provider_budget::message_token_ceiling)
            .sum::<u64>()
            .saturating_add(davinci_agent::provider_budget::text_token_ceiling(
                DESIGN_POLICY,
            ))
            .saturating_add(davinci_agent::provider_budget::text_token_ceiling(
                &serde_json::to_string(&request.tools).map_err(|e| e.to_string())?,
            ));
        let window = self.model.context_window.min(agent.context_window);
        if tokens.saturating_add(output) > window {
            return Err(format!(
                "complete design request does not fit the selected model context ({tokens} input + {output} output tokens > {window})"
            ));
        }
        davinci_ai::live_complete_streaming_with_sink_envelope(
            &self.model,
            &request.messages,
            &self.auth,
            Some(DESIGN_POLICY),
            &request.tools,
            &design_stream_options(agent, self.effort, request, remaining, output),
            &mut |_| {},
        )
        .map(|envelope| CompleteOutput {
            message: envelope.message,
            stream_events: None,
            native_responses_resume: None,
            streamed_live: false,
        })
    }
}

#[cfg(test)]
mod service_tier_regression_tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};

    #[test]
    fn design_inherits_tier_without_changing_effort_or_cancellation() {
        let mut agent = Agent::new("system");
        agent.provider_timeout_ms = Some(500);
        let request = DesignModelRequest {
            messages: Vec::new(),
            tools: Vec::new(),
            deadline_ms: 1000,
            abort: Arc::new(AtomicBool::new(false)),
        };
        for tier in [
            davinci_ai::CodexServiceTier::Standard,
            davinci_ai::CodexServiceTier::Fast,
            davinci_ai::CodexServiceTier::Flex,
        ] {
            agent.service_tier = tier;
            let options = design_stream_options(
                &agent,
                davinci_protocol::ThinkingLevel::High,
                &request,
                100,
                64,
            );
            assert_eq!(options.service_tier, Some(tier));
            assert_eq!(
                options.thinking_level,
                Some(davinci_protocol::ThinkingLevel::High)
            );
            assert_eq!(options.timeout_ms, Some(100));
            assert_eq!(options.max_retries, Some(0));
            assert_eq!(options.max_tokens, Some(64));
            assert_eq!(options.transport.as_deref(), Some("sse"));
            assert!(Arc::ptr_eq(
                options.abort_signal.as_ref().unwrap(),
                &request.abort
            ));
        }
    }
}
