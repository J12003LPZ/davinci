//! TS `packages/ai/src/images.ts` + `api/openrouter-images.ts`.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{calculate_usage, ImageContent, MessageContent, Model};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagesContext {
    pub input: Vec<MessageContent>,
}

#[derive(Debug, Clone, Default)]
pub struct GenerateImagesOptions {
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    /// Output modalities (`text`, `image`). Empty means `["image"]`.
    pub output: Vec<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantImages {
    pub api: String,
    pub provider: String,
    pub model: String,
    pub output: Vec<MessageContent>,
    #[serde(rename = "stopReason")]
    pub stop_reason: String,
    pub timestamp: u64,
    #[serde(rename = "responseId", skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(rename = "errorMessage", skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// TS `buildParams` for `openrouter-images`.
pub fn images_request_body(model: &Model, context: &ImagesContext, output: &[String]) -> Value {
    let content: Vec<Value> = context
        .input
        .iter()
        .map(|item| match item {
            MessageContent::Text { text } => json!({ "type": "text", "text": text }),
            MessageContent::Image { data, mime_type } => json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime_type};base64,{data}") }
            }),
            _ => json!({ "type": "text", "text": "" }),
        })
        .collect();
    let modalities = if output.iter().any(|item| item == "text") {
        vec!["image", "text"]
    } else {
        vec!["image"]
    };
    json!({
        "model": model.id,
        "messages": [{ "role": "user", "content": content }],
        "stream": false,
        "modalities": modalities,
    })
}

/// TS `generateImages` for the `openrouter-images` API.
pub fn generate_images(
    model: &Model,
    context: &ImagesContext,
    options: &GenerateImagesOptions,
) -> AssistantImages {
    let mut output = AssistantImages {
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        output: Vec::new(),
        stop_reason: "stop".into(),
        timestamp: now_ms(),
        response_id: None,
        usage: None,
        error_message: None,
    };
    if model.api != "openrouter-images" {
        output.stop_reason = "error".into();
        output.error_message = Some(format!("No API provider registered for api: {}", model.api));
        return output;
    }
    if options.api_key.as_deref().unwrap_or("").is_empty() {
        output.stop_reason = "error".into();
        output.error_message = Some(format!("No API key for provider: {}", model.provider));
        return output;
    }
    if matches!(
        std::env::var("PI_OPENROUTER_IMAGES_ABORT").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) {
        output.stop_reason = "aborted".into();
        output.error_message = Some("Request aborted".into());
        return output;
    }

    let body = images_request_body(model, context, &options.output);
    if let Ok(path) = std::env::var("PI_OPENROUTER_IMAGES_REQUEST") {
        let _ = std::fs::write(path, body.to_string());
    }

    let response = if let Ok(reply) = std::env::var("PI_OPENROUTER_IMAGES_REPLY") {
        if let Ok(value) = serde_json::from_str::<Value>(&reply) {
            value
        } else if let Ok(text) = std::fs::read_to_string(&reply) {
            serde_json::from_str(&text).unwrap_or(Value::Null)
        } else {
            serde_json::from_str(&reply).unwrap_or(Value::Null)
        }
    } else if std::env::var("PI_OPENROUTER_IMAGES_DRY_RUN").is_ok() || cfg!(test) {
        output.stop_reason = "error".into();
        output.error_message = Some("No fixture response for openrouter-images".into());
        return output;
    } else {
        match post_images(model, &body, options) {
            Ok(value) => value,
            Err(err) => {
                output.stop_reason = "error".into();
                output.error_message = Some(err);
                return output;
            }
        }
    };

    apply_images_response(&mut output, model, &response);
    output
}

fn post_images(
    model: &Model,
    body: &Value,
    options: &GenerateImagesOptions,
) -> Result<Value, String> {
    let base = model
        .base_url
        .as_deref()
        .unwrap_or("https://openrouter.ai/api/v1")
        .trim_end_matches('/');
    let url = format!("{base}/chat/completions");
    let mut request = ureq::post(&url);
    if let Some(timeout_ms) = options.timeout_ms {
        request = request.timeout(std::time::Duration::from_millis(timeout_ms));
    }
    if let Some(key) = &options.api_key {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }
    request = request.set("content-type", "application/json");
    for (key, value) in &model.headers {
        request = request.set(key, value);
    }
    for (key, value) in &options.headers {
        request = request.set(key, value);
    }
    let response = request
        .send_string(&body.to_string())
        .map_err(|err| err.to_string())?;
    response
        .into_json()
        .map_err(|err| format!("Unable to read provider response: {err}"))
}

fn apply_images_response(output: &mut AssistantImages, model: &Model, response: &Value) {
    if let Some(id) = response.get("id").and_then(Value::as_str) {
        output.response_id = Some(id.to_string());
    }
    if let Some(usage) = response.get("usage") {
        output.usage = Some(parse_images_usage(usage, model));
    }
    let Some(choice) = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
    else {
        return;
    };
    let message = choice.get("message").unwrap_or(choice);
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            output.output.push(MessageContent::Text {
                text: text.to_string(),
            });
        }
    }
    if let Some(images) = message.get("images").and_then(Value::as_array) {
        for image in images {
            let url = image
                .get("image_url")
                .and_then(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.get("url").and_then(Value::as_str).map(str::to_string))
                })
                .unwrap_or_default();
            if !url.starts_with("data:") {
                continue;
            }
            let Some(rest) = url.strip_prefix("data:") else {
                continue;
            };
            let Some((meta, data)) = rest.split_once(";base64,") else {
                continue;
            };
            output.output.push(MessageContent::Image {
                mime_type: meta.to_string(),
                data: data.to_string(),
            });
        }
    }
}

fn parse_images_usage(raw: &Value, model: &Model) -> Value {
    let prompt_tokens = raw
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let details = raw
        .get("prompt_tokens_details")
        .cloned()
        .unwrap_or(json!({}));
    let reported_cached = details
        .get("cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = details
        .get("cache_write_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = if cache_write > 0 {
        reported_cached.saturating_sub(cache_write)
    } else {
        reported_cached
    };
    let input = prompt_tokens
        .saturating_sub(cache_read)
        .saturating_sub(cache_write);
    let output_tokens = raw
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let usage = calculate_usage(model, input, output_tokens, cache_read, cache_write);
    serde_json::to_value(usage).unwrap_or(Value::Null)
}

/// Convenience wrapper matching TS `ImageContent`.
pub fn image_content(data: impl Into<String>, mime_type: impl Into<String>) -> ImageContent {
    ImageContent {
        kind: "image".into(),
        data: data.into(),
        mime_type: mime_type.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrouter_image_models;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_response() -> Value {
        json!({
            "id": "img-1",
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 34,
                "prompt_tokens_details": { "cached_tokens": 0 }
            },
            "choices": [{
                "message": {
                    "content": "Here is your image.",
                    "images": [{ "image_url": "data:image/png;base64,ZmFrZS1wbmc=" }]
                }
            }]
        })
    }

    #[test]
    fn images_request_body_matches_ts() {
        let model = openrouter_image_models().remove(0);
        let context = ImagesContext {
            input: vec![MessageContent::Text {
                text: "Generate a dog".into(),
            }],
        };
        let with_text = images_request_body(&model, &context, &["text".into(), "image".into()]);
        assert_eq!(with_text["stream"], false);
        assert_eq!(with_text["modalities"], json!(["image", "text"]));
        assert_eq!(
            with_text["messages"][0]["content"][0],
            json!({ "type": "text", "text": "Generate a dog" })
        );
        let image_only = images_request_body(&model, &context, &["image".into()]);
        assert_eq!(image_only["modalities"], json!(["image"]));
    }

    #[test]
    fn generate_images_parses_text_and_image_fixture() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("PI_OPENROUTER_IMAGES_REPLY").ok();
        std::env::set_var("PI_OPENROUTER_IMAGES_REPLY", fixture_response().to_string());
        let model = openrouter_image_models().remove(0);
        let context = ImagesContext {
            input: vec![MessageContent::Text {
                text: "Generate a dog".into(),
            }],
        };
        let output = generate_images(
            &model,
            &context,
            &GenerateImagesOptions {
                api_key: Some("test".into()),
                output: vec!["text".into(), "image".into()],
                ..GenerateImagesOptions::default()
            },
        );
        match previous {
            Some(value) => std::env::set_var("PI_OPENROUTER_IMAGES_REPLY", value),
            None => std::env::remove_var("PI_OPENROUTER_IMAGES_REPLY"),
        }
        assert_eq!(output.stop_reason, "stop");
        assert_eq!(output.response_id.as_deref(), Some("img-1"));
        assert!(matches!(
            output.output.first(),
            Some(MessageContent::Text { text }) if text == "Here is your image."
        ));
        assert!(matches!(
            output.output.get(1),
            Some(MessageContent::Image { mime_type, data })
                if mime_type == "image/png" && data == "ZmFrZS1wbmc="
        ));
    }

    #[test]
    fn generate_images_requires_api_key() {
        let model = openrouter_image_models().remove(0);
        let context = ImagesContext {
            input: vec![MessageContent::Text {
                text: "Generate a dog".into(),
            }],
        };
        let output = generate_images(&model, &context, &GenerateImagesOptions::default());
        assert_eq!(output.stop_reason, "error");
        assert_eq!(
            output.error_message.as_deref(),
            Some("No API key for provider: openrouter")
        );
    }

    #[test]
    fn generate_images_abort_fixture() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("PI_OPENROUTER_IMAGES_ABORT").ok();
        std::env::set_var("PI_OPENROUTER_IMAGES_ABORT", "1");
        let model = openrouter_image_models().remove(0);
        let output = generate_images(
            &model,
            &ImagesContext {
                input: vec![MessageContent::Text {
                    text: "Generate a dog".into(),
                }],
            },
            &GenerateImagesOptions {
                api_key: Some("test".into()),
                ..GenerateImagesOptions::default()
            },
        );
        match previous {
            Some(value) => std::env::set_var("PI_OPENROUTER_IMAGES_ABORT", value),
            None => std::env::remove_var("PI_OPENROUTER_IMAGES_ABORT"),
        }
        assert_eq!(output.stop_reason, "aborted");
        assert_eq!(output.error_message.as_deref(), Some("Request aborted"));
    }
}
