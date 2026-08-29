use crate::error::Result;
use crate::event_stream::AssistantMessageEventStream;
use crate::providers::Provider;
use crate::types::*;
use async_trait::async_trait;

#[derive(Default, Clone)]
pub struct FauxProvider;

impl FauxProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Provider for FauxProvider {
    fn id(&self) -> &str {
        "faux"
    }

    fn name(&self) -> &str {
        "Faux Provider"
    }

    fn get_models(&self) -> Vec<Model> {
        vec![Model {
            id: "faux-1".to_string(),
            name: "Faux Model".to_string(),
            api: "faux".to_string(),
            provider: "faux".to_string(),
            base_url: "http://localhost:0".to_string(),
            reasoning: true,
            thinking_level_map: None,
            input: vec!["text".to_string()],
            cost: ModelCost::default(),
            context_window: 100000,
            max_tokens: 4096,
            sampling_params: None,
            headers: None,
            compat: None,
        }]
    }

    async fn stream(
        &self,
        model: &Model,
        _context: &Context,
        _options: &StreamOptions,
    ) -> Result<AssistantMessageEventStream> {
        let (stream, sender) = AssistantMessageEventStream::new();
        let mut msg = AssistantMessage {
            role: "assistant".to_string(),
            content: vec![],
            api: model.api.clone(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            response_model: None,
            response_id: Some("faux-resp-1".to_string()),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            deferred: None,
            error_message: None,
            raw_stop_reason: None,
            end_turn: Some(true),
            timestamp: now_ms(),
        };

        tokio::spawn(async move {
            let _ = sender.send(AssistantMessageEvent::Start {
                partial: msg.clone(),
            });

            let text = "Hello from faux provider!";
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

        Ok(stream)
    }
}
