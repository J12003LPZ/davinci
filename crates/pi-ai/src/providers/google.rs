use crate::error::{Error, Result};
use crate::event_stream::AssistantMessageEventStream;
use crate::providers::Provider;
use crate::types::*;
use async_trait::async_trait;

#[derive(Default, Clone)]
pub struct GoogleProvider;

impl GoogleProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    fn id(&self) -> &str {
        "google"
    }

    fn name(&self) -> &str {
        "Google"
    }

    fn get_models(&self) -> Vec<Model> {
        crate::models::BUILTIN_MODELS
            .get("google")
            .cloned()
            .unwrap_or_default()
    }

    async fn stream(
        &self,
        model: &Model,
        _context: &Context,
        options: &StreamOptions,
    ) -> Result<AssistantMessageEventStream> {
        let api_key = options
            .api_key
            .clone()
            .or_else(|| crate::auth::get_env_api_key_for_provider("google"))
            .ok_or_else(|| Error::NoApiKey("google".to_string()))?;

        let (stream, sender) = AssistantMessageEventStream::new();
        let model_id = model.id.clone();
        let api = model.api.clone();
        let provider = model.provider.clone();

        tokio::spawn(async move {
            let mut msg = AssistantMessage {
                role: "assistant".to_string(),
                content: vec![],
                api,
                provider,
                model: model_id,
                response_model: None,
                response_id: Some("gen_test".to_string()),
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
                deferred: None,
                error_message: None,
                raw_stop_reason: None,
                end_turn: Some(true),
                timestamp: now_ms(),
            };

            let _ = sender.send(AssistantMessageEvent::Start {
                partial: msg.clone(),
            });

            let text = "Google response placeholder";
            msg.content.push(ContentBlock::Text(TextContent::new(text)));

            let _ = sender.send(AssistantMessageEvent::TextStart {
                content_index: 0,
                partial: msg.clone(),
            });
            let _ = sender.send(AssistantMessageEvent::TextDelta {
                content_index: 0,
                delta: text.to_string(),
                partial: msg.clone(),
            });
            let _ = sender.send(AssistantMessageEvent::TextEnd {
                content_index: 0,
                content: text.to_string(),
                partial: msg.clone(),
            });

            let _ = sender.send(AssistantMessageEvent::Done {
                reason: StopReason::Stop,
                message: msg,
            });
        });

        let _ = api_key;
        Ok(stream)
    }
}
