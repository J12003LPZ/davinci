use crate::error::Result;
use crate::event_stream::AssistantMessageEventStream;
use crate::types::*;
use async_trait::async_trait;

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn get_models(&self) -> Vec<Model>;
    async fn stream(
        &self,
        model: &Model,
        context: &Context,
        options: &StreamOptions,
    ) -> Result<AssistantMessageEventStream>;
}

pub mod anthropic;
pub mod faux_provider;
pub mod google;
pub mod openai;
pub mod openrouter;

pub use anthropic::AnthropicProvider;
pub use faux_provider::FauxProvider;
pub use google::GoogleProvider;
pub use openai::OpenAiProvider;
pub use openrouter::OpenRouterProvider;

pub async fn stream_simple(
    model: &Model,
    context: &Context,
    options: &SimpleStreamOptions,
) -> Result<AssistantMessageEventStream> {
    match model.api.as_str() {
        "anthropic-messages" => {
            AnthropicProvider::new()
                .stream(model, context, &options.base)
                .await
        }
        "openai-completions" | "openai-responses" => {
            OpenAiProvider::new()
                .stream(model, context, &options.base)
                .await
        }
        "google-generative-ai" => {
            GoogleProvider::new()
                .stream(model, context, &options.base)
                .await
        }
        "faux" => {
            FauxProvider::new()
                .stream(model, context, &options.base)
                .await
        }
        other => Err(crate::error::Error::UnsupportedApi(other.to_string())),
    }
}
