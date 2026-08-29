use crate::auth::{AuthResolved, Credential, CredentialStore};
use crate::cost::calculate_cost;
use crate::event_stream::{create_assistant_message_event_stream, AssistantMessageEventStream};
use crate::types::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, Context, Model, ModelCost,
    ModelCostRates, SimpleStreamOptions, StopReason, Usage,
};
use std::collections::HashMap;
use std::sync::Arc;

pub fn default_anthropic_model() -> Model {
    Model {
        id: "claude-sonnet-4-5".to_string(),
        name: "Claude Sonnet 4.5".to_string(),
        api: "anthropic-messages".to_string(),
        provider: "anthropic".to_string(),
        base_url: "https://api.anthropic.com".to_string(),
        reasoning: true,
        thinking_level_map: None,
        input: vec!["text".to_string(), "image".to_string()],
        cost: ModelCost {
            rates: ModelCostRates {
                input: 3.0,
                output: 15.0,
                cache_read: 0.3,
                cache_write: 3.75,
            },
            tiers: None,
        },
        context_window: 200_000,
        max_tokens: 8192,
        sampling_params: None,
        headers: None,
        compat: None,
    }
}

pub fn default_openai_model() -> Model {
    Model {
        id: "gpt-5.6-turbo".to_string(),
        name: "GPT-5.6 Turbo".to_string(),
        api: "openai-responses".to_string(),
        provider: "openai".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        reasoning: true,
        thinking_level_map: None,
        input: vec!["text".to_string(), "image".to_string()],
        cost: ModelCost {
            rates: ModelCostRates {
                input: 2.5,
                output: 10.0,
                cache_read: 1.25,
                cache_write: 2.5,
            },
            tiers: None,
        },
        context_window: 128_000,
        max_tokens: 4096,
        sampling_params: None,
        headers: None,
        compat: None,
    }
}

pub struct Models {
    credential_store: Arc<dyn CredentialStore>,
    providers: HashMap<String, Vec<Model>>,
}

impl Models {
    pub fn new(credential_store: Arc<dyn CredentialStore>) -> Self {
        let mut providers = HashMap::new();
        providers.insert("anthropic".to_string(), vec![default_anthropic_model()]);
        providers.insert("openai".to_string(), vec![default_openai_model()]);

        Self {
            credential_store,
            providers,
        }
    }

    pub fn get_models(&self, provider: Option<&str>) -> Vec<Model> {
        if let Some(p) = provider {
            self.providers.get(p).cloned().unwrap_or_default()
        } else {
            self.providers.values().flatten().cloned().collect()
        }
    }

    pub fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        self.providers
            .get(provider)
            .and_then(|models| models.iter().find(|m| m.id == id).cloned())
    }

    pub async fn resolve_auth(&self, provider_id: &str) -> Option<AuthResolved> {
        if let Some(cred) = self.credential_store.read(provider_id).await {
            match cred {
                Credential::ApiKey { key, env } => Some(AuthResolved {
                    api_key: Some(key),
                    headers: None,
                    env,
                    source: "stored credential".to_string(),
                }),
                Credential::OAuth { token, .. } => {
                    let mut headers = HashMap::new();
                    headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                    Some(AuthResolved {
                        api_key: None,
                        headers: Some(headers),
                        env: None,
                        source: "OAuth".to_string(),
                    })
                }
            }
        } else {
            let env_var = match provider_id {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openai" => "OPENAI_API_KEY",
                "google" => "GEMINI_API_KEY",
                _ => return None,
            };
            if let Ok(val) = std::env::var(env_var) {
                Some(AuthResolved {
                    api_key: Some(val),
                    headers: None,
                    env: None,
                    source: env_var.to_string(),
                })
            } else {
                None
            }
        }
    }

    pub fn stream_simple(
        &self,
        model: &Model,
        context: &Context,
        options: Option<&SimpleStreamOptions>,
    ) -> AssistantMessageEventStream {
        let (mut sender, stream) = create_assistant_message_event_stream();
        let model_clone = model.clone();
        let context_clone = context.clone();
        let _options_clone = options.cloned();

        tokio::spawn(async move {
            let mut usage = Usage {
                input: 10,
                output: 20,
                cache_read: 0,
                cache_write: 0,
                cache_write_1h: None,
                reasoning: None,
                total_tokens: 30,
                cost: Default::default(),
            };
            calculate_cost(&model_clone, &mut usage);

            let initial_msg = AssistantMessage {
                role: "assistant".to_string(),
                content: vec![],
                api: model_clone.api.clone(),
                provider: model_clone.provider.clone(),
                model: model_clone.id.clone(),
                response_model: None,
                response_id: Some("resp-123".to_string()),
                usage: usage.clone(),
                stop_reason: StopReason::Pending,
                deferred: None,
                error_message: None,
                raw_stop_reason: None,
                end_turn: Some(true),
                timestamp: chrono::Utc::now().timestamp_millis(),
            };

            sender.push(AssistantMessageEvent::Start {
                partial: initial_msg.clone(),
            });

            // Simulate a response or parse mock
            let text_part = format!("Hello! You sent {} messages.", context_clone.messages.len());

            let mut final_msg = initial_msg.clone();
            final_msg
                .content
                .push(AssistantContent::Text(crate::types::TextContent {
                    content_type: "text".to_string(),
                    text: text_part.clone(),
                    text_signature: None,
                }));
            final_msg.stop_reason = StopReason::Stop;

            sender.push(AssistantMessageEvent::TextStart {
                content_index: 0,
                partial: final_msg.clone(),
            });

            sender.push(AssistantMessageEvent::TextDelta {
                content_index: 0,
                delta: text_part.clone(),
                partial: final_msg.clone(),
            });

            sender.push(AssistantMessageEvent::TextEnd {
                content_index: 0,
                content: text_part,
                partial: final_msg.clone(),
            });

            sender.push(AssistantMessageEvent::Done {
                reason: StopReason::Stop,
                message: final_msg.clone(),
            });

            sender.end(final_msg);
        });

        stream
    }
}
