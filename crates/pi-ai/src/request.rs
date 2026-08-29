//! Provider request-body builders matching TypeScript `packages/ai/src/api`.

use serde_json::{json, Value};

use crate::catalog::Model;
use crate::types::{ContentBlock, Message, Role};

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Value>,
    pub max_tokens: u64,
    pub stream: bool,
}

impl Default for RequestContext {
    fn default() -> Self {
        Self {
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: 16_384,
            stream: true,
        }
    }
}

pub fn resolve_api(model: Option<&Model>, provider: &str) -> String {
    if let Some(model) = model {
        if !model.api.is_empty() {
            return model.api.clone();
        }
    }
    match provider {
        "anthropic" | "ant-ling" => "anthropic-messages".into(),
        "google" => "google-generative-ai".into(),
        "google-vertex" => "google-vertex".into(),
        "amazon-bedrock" => "bedrock-converse-stream".into(),
        "mistral" => "mistral-conversations".into(),
        "openai-codex" => "openai-codex-responses".into(),
        "azure-openai-responses" => "azure-openai-responses".into(),
        _ => "openai-completions".into(),
    }
}

pub fn endpoint_url(api: &str, base_url: &str, model_id: &str, api_key: Option<&str>) -> String {
    let base = base_url.trim_end_matches('/');
    match api {
        "anthropic-messages" => format!("{base}/v1/messages"),
        "google-generative-ai" => {
            let key = api_key.unwrap_or("");
            format!("{base}/v1beta/models/{model_id}:streamGenerateContent?alt=sse&key={key}")
        }
        "google-vertex" => {
            let project = std::env::var("GOOGLE_CLOUD_PROJECT")
                .or_else(|_| std::env::var("GCLOUD_PROJECT"))
                .unwrap_or_else(|_| "project".into());
            let location =
                std::env::var("GOOGLE_CLOUD_LOCATION").unwrap_or_else(|_| "us-central1".into());
            format!(
                "{base}/v1/projects/{project}/locations/{location}/publishers/google/models/{model_id}:streamGenerateContent"
            )
        }
        "openai-responses" | "openai-codex-responses" | "azure-openai-responses" => {
            format!("{base}/v1/responses")
        }
        "bedrock-converse-stream" => format!("{base}/model/{model_id}/converse-stream"),
        "mistral-conversations" => format!("{base}/v1/conversations"),
        _ => format!("{base}/v1/chat/completions"),
    }
}

pub fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "https://api.anthropic.com",
        "openai" | "openai-codex" => "https://api.openai.com",
        "google" => "https://generativelanguage.googleapis.com",
        "openrouter" => "https://openrouter.ai/api",
        "groq" => "https://api.groq.com/openai",
        "deepseek" => "https://api.deepseek.com",
        "mistral" => "https://api.mistral.ai",
        "xai" => "https://api.x.ai",
        "together" => "https://api.together.xyz",
        "fireworks" => "https://api.fireworks.ai/inference",
        "cerebras" => "https://api.cerebras.ai",
        "amazon-bedrock" => "https://bedrock-runtime.us-east-1.amazonaws.com",
        _ => "https://api.openai.com",
    }
}

pub fn bedrock_runtime_base(region: &str) -> String {
    format!("https://bedrock-runtime.{region}.amazonaws.com")
}

pub fn request_headers(api: &str, provider: &str, api_key: &str) -> Vec<(String, String)> {
    let mut headers = vec![
        ("content-type".into(), "application/json".into()),
        ("user-agent".into(), "pi/0.84.4".into()),
    ];
    match api {
        "anthropic-messages" => {
            headers.push(("x-api-key".into(), api_key.into()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        }
        "google-generative-ai" => {}
        _ => {
            headers.push(("authorization".into(), format!("Bearer {api_key}")));
            if provider == "github-copilot" {
                headers.push(("editor-version".into(), "pi/0.84.4".into()));
                headers.push(("copilot-integration-id".into(), "vscode-chat".into()));
            }
        }
    }
    headers
}

pub fn build_request_body(api: &str, model_id: &str, ctx: &RequestContext) -> Value {
    match api {
        "anthropic-messages" => anthropic_messages(model_id, ctx),
        "google-generative-ai" | "google-vertex" => google_generative_ai(model_id, ctx),
        "pi-messages" => anthropic_messages(model_id, ctx),
        "openrouter-images" => openai_completions(model_id, ctx),
        "openai-responses" | "openai-codex-responses" | "azure-openai-responses" => {
            openai_responses(model_id, ctx)
        }
        "bedrock-converse-stream" => bedrock_converse(model_id, ctx),
        "mistral-conversations" => mistral_conversations(model_id, ctx),
        _ => openai_completions(model_id, ctx),
    }
}

fn text_of(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn anthropic_messages(model_id: &str, ctx: &RequestContext) -> Value {
    let mut messages = Vec::new();
    for message in &ctx.messages {
        let role = match message.role {
            Role::Assistant => "assistant",
            Role::Tool => "user",
            _ => "user",
        };
        let mut content: Vec<Value> = Vec::new();
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    content.push(json!({"type":"text","text": text}));
                }
                ContentBlock::Image { data, mime_type } => {
                    content.push(json!({
                        "type":"image",
                        "source":{"type":"base64","media_type": mime_type, "data": data}
                    }));
                }
                ContentBlock::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                } => {
                    content.push(json!({
                        "type":"tool_use",
                        "id": tool_call_id,
                        "name": tool_name,
                        "input": input
                    }));
                }
                ContentBlock::Thinking { thinking, .. } => {
                    content.push(json!({"type":"thinking","thinking": thinking}));
                }
            }
        }
        if message.role == Role::Tool {
            content = vec![json!({
                "type":"tool_result",
                "tool_use_id": "tool",
                "content": text_of(&message.content)
            })];
        }
        messages.push(json!({"role": role, "content": content}));
    }
    let mut body = json!({
        "model": model_id,
        "max_tokens": ctx.max_tokens,
        "stream": ctx.stream,
        "messages": messages,
    });
    if let Some(system) = &ctx.system {
        body["system"] = json!(system);
    }
    if !ctx.tools.is_empty() {
        body["tools"] = Value::Array(
            ctx.tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.get("name"),
                        "description": tool.get("description").cloned().unwrap_or(json!("")),
                        "input_schema": tool.get("parameters").cloned().unwrap_or(json!({"type":"object"}))
                    })
                })
                .collect(),
        );
    }
    body
}

fn openai_completions(model_id: &str, ctx: &RequestContext) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = &ctx.system {
        messages.push(json!({"role":"system","content": system}));
    }
    for message in &ctx.messages {
        let role = match message.role {
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::System => "system",
            Role::User => "user",
        };
        messages.push(json!({
            "role": role,
            "content": text_of(&message.content)
        }));
    }
    let mut body = json!({
        "model": model_id,
        "stream": ctx.stream,
        "messages": messages,
    });
    if !ctx.tools.is_empty() {
        body["tools"] = Value::Array(
            ctx.tools
                .iter()
                .map(|tool| {
                    json!({
                        "type":"function",
                        "function":{
                            "name": tool.get("name"),
                            "description": tool.get("description").cloned().unwrap_or(json!("")),
                            "parameters": tool.get("parameters").cloned().unwrap_or(json!({"type":"object"}))
                        }
                    })
                })
                .collect(),
        );
    }
    body
}

fn openai_responses(model_id: &str, ctx: &RequestContext) -> Value {
    let mut input = Vec::new();
    for message in &ctx.messages {
        input.push(json!({
            "role": match message.role {
                Role::Assistant => "assistant",
                _ => "user",
            },
            "content": text_of(&message.content)
        }));
    }
    let mut body = json!({
        "model": model_id,
        "stream": ctx.stream,
        "input": input,
    });
    if let Some(system) = &ctx.system {
        body["instructions"] = json!(system);
    }
    body
}

fn bedrock_converse(model_id: &str, ctx: &RequestContext) -> Value {
    let mut messages = Vec::new();
    for message in &ctx.messages {
        messages.push(json!({
            "role": if message.role == Role::Assistant { "assistant" } else { "user" },
            "content": [{"text": text_of(&message.content)}]
        }));
    }
    let mut body = json!({
        "modelId": model_id,
        "messages": messages,
    });
    if let Some(system) = &ctx.system {
        body["system"] = json!([{"text": system}]);
    }
    if !ctx.tools.is_empty() {
        body["toolConfig"] = json!({
            "tools": ctx.tools.iter().map(|tool| {
                json!({"toolSpec":{
                    "name": tool.get("name"),
                    "description": tool.get("description").cloned().unwrap_or(json!("")),
                    "inputSchema": {"json": tool.get("parameters").cloned().unwrap_or(json!({"type":"object"}))}
                }})
            }).collect::<Vec<_>>()
        });
    }
    body
}

fn mistral_conversations(model_id: &str, ctx: &RequestContext) -> Value {
    let mut body = openai_completions(model_id, ctx);
    body["stream"] = json!(ctx.stream);
    body
}

fn google_generative_ai(_model_id: &str, ctx: &RequestContext) -> Value {
    let mut contents = Vec::new();
    for message in &ctx.messages {
        let role = if message.role == Role::Assistant {
            "model"
        } else {
            "user"
        };
        contents.push(json!({
            "role": role,
            "parts": [{"text": text_of(&message.content)}]
        }));
    }
    let mut body = json!({
        "contents": contents,
    });
    if let Some(system) = &ctx.system {
        body["systemInstruction"] = json!({"parts":[{"text": system}]});
    }
    if !ctx.tools.is_empty() {
        body["tools"] = json!([{
            "functionDeclarations": ctx.tools
        }]);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentBlock;

    fn user(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: None,
        }
    }

    #[test]
    fn anthropic_body_matches_ts_shape() {
        let body = build_request_body(
            "anthropic-messages",
            "claude-sonnet-4-5",
            &RequestContext {
                system: Some("sys".into()),
                messages: vec![user("hi")],
                tools: vec![
                    json!({"name":"read","description":"Read","parameters":{"type":"object"}}),
                ],
                max_tokens: 1024,
                stream: true,
            },
        );
        assert_eq!(body["model"], "claude-sonnet-4-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "sys");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["tools"][0]["name"], "read");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(
            endpoint_url(
                "anthropic-messages",
                "https://api.anthropic.com",
                "claude-sonnet-4-5",
                None
            ),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn openai_and_google_bodies() {
        let ctx = RequestContext {
            system: Some("sys".into()),
            messages: vec![user("hi")],
            tools: vec![json!({"name":"bash","parameters":{"type":"object"}})],
            ..RequestContext::default()
        };
        let openai = build_request_body("openai-completions", "gpt-4o", &ctx);
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["tools"][0]["type"], "function");
        let google = build_request_body("google-generative-ai", "gemini-2.5-flash", &ctx);
        assert_eq!(google["contents"][0]["role"], "user");
        assert_eq!(google["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(resolve_api(None, "anthropic"), "anthropic-messages");
        assert_eq!(resolve_api(None, "openai"), "openai-completions");
        assert_eq!(resolve_api(None, "google"), "google-generative-ai");
        assert_eq!(resolve_api(None, "google-vertex"), "google-vertex");
        let vertex = build_request_body("google-vertex", "gemini-2.5-flash", &ctx);
        assert_eq!(vertex["contents"][0]["role"], "user");
        let pi_messages = build_request_body("pi-messages", "pi", &ctx);
        assert_eq!(pi_messages["messages"][0]["role"], "user");
        let bedrock =
            build_request_body("bedrock-converse-stream", "anthropic.claude-sonnet-4", &ctx);
        assert_eq!(bedrock["messages"][0]["role"], "user");
        assert_eq!(bedrock["system"][0]["text"], "sys");
        assert_eq!(
            resolve_api(None, "amazon-bedrock"),
            "bedrock-converse-stream"
        );
        assert_eq!(resolve_api(None, "mistral"), "mistral-conversations");
    }
}
