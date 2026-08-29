//! TS `fetchDeferred` / `cancelDeferred` (fixture-only in tests; no live LLM).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::Model;
use crate::stream::{AssistantMessage, ContentBlock, StopReason};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredHandle {
    pub provider: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
    pub api: String,
    pub id: String,
    #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(rename = "pollAfterMs", skip_serializing_if = "Option::is_none")]
    pub poll_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct DeferredFetchOptions {
    pub wait: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeferredFetchResult {
    pub message: AssistantMessage,
    pub deferred: Option<DeferredHandle>,
}

/// TS `Models.fetchDeferred` — fixture `PI_DEFERRED_FETCH_REPLY` / abort / dry-run.
pub fn fetch_deferred(
    model: &Model,
    handle: &DeferredHandle,
    options: &DeferredFetchOptions,
) -> DeferredFetchResult {
    let _ = options.wait;
    if handle.provider != model.provider || handle.model_id != model.id || handle.api != model.api {
        return error_result(model, format!("Unknown deferred response: {}", handle.id));
    }
    if matches!(
        std::env::var("PI_DEFERRED_FETCH_ABORT").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) {
        return DeferredFetchResult {
            message: AssistantMessage {
                id: handle.id.clone(),
                role: "assistant".into(),
                content: Vec::new(),
                model: model.id.clone(),
                usage: None,
                stop_reason: Some(StopReason::Aborted),
                error_message: Some("Request aborted".into()),
            },
            deferred: Some(handle.clone()),
        };
    }
    if let Ok(path) = std::env::var("PI_DEFERRED_FETCH_REQUEST") {
        let body = serde_json::json!({
            "provider": handle.provider,
            "modelId": handle.model_id,
            "api": handle.api,
            "id": handle.id,
            "wait": options.wait.unwrap_or(0),
        });
        let _ = std::fs::write(path, body.to_string());
    }
    let reply = std::env::var("PI_DEFERRED_FETCH_REPLY").ok();
    if reply.is_none() && (std::env::var("PI_DEFERRED_DRY_RUN").is_ok() || cfg!(test)) {
        return error_result(model, "No fixture response for deferred fetch".into());
    }
    let Some(reply) = reply else {
        return error_result(model, "Live deferred fetch is not enabled".into());
    };
    if reply == "pending" || reply.is_empty() {
        return DeferredFetchResult {
            message: AssistantMessage {
                id: handle.id.clone(),
                role: "assistant".into(),
                content: Vec::new(),
                model: model.id.clone(),
                usage: None,
                stop_reason: Some(StopReason::Deferred),
                error_message: None,
            },
            deferred: Some(handle.clone()),
        };
    }
    if let Ok(value) = serde_json::from_str::<Value>(&reply) {
        if value.get("stopReason").and_then(Value::as_str) == Some("deferred") {
            let next = value
                .get("deferred")
                .and_then(|item| serde_json::from_value(item.clone()).ok())
                .unwrap_or_else(|| handle.clone());
            return DeferredFetchResult {
                message: AssistantMessage {
                    id: handle.id.clone(),
                    role: "assistant".into(),
                    content: Vec::new(),
                    model: model.id.clone(),
                    usage: None,
                    stop_reason: Some(StopReason::Deferred),
                    error_message: None,
                },
                deferred: Some(next),
            };
        }
        if let Some(text) = value
            .get("content")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .or_else(|| value.get("text").and_then(Value::as_str))
        {
            return ready_text(model, handle, text);
        }
    }
    ready_text(model, handle, &reply)
}

/// TS `Models.cancelDeferred` — fixture `PI_DEFERRED_CANCEL_REPLY`.
pub fn cancel_deferred(model: &Model, handle: &DeferredHandle) -> Result<(), String> {
    if handle.provider != model.provider || handle.model_id != model.id || handle.api != model.api {
        return Err(format!("Unknown deferred response: {}", handle.id));
    }
    if let Ok(path) = std::env::var("PI_DEFERRED_CANCEL_REQUEST") {
        let body = serde_json::json!({
            "provider": handle.provider,
            "modelId": handle.model_id,
            "api": handle.api,
            "id": handle.id,
        });
        let _ = std::fs::write(path, body.to_string());
    }
    if let Ok(reply) = std::env::var("PI_DEFERRED_CANCEL_REPLY") {
        if reply == "error" {
            return Err(format!(
                "Faux deferred response was cancelled: {}",
                handle.id
            ));
        }
        return Ok(());
    }
    if std::env::var("PI_DEFERRED_DRY_RUN").is_ok() || cfg!(test) {
        return Ok(());
    }
    Err("Live deferred cancel is not enabled".into())
}

fn ready_text(model: &Model, handle: &DeferredHandle, text: &str) -> DeferredFetchResult {
    DeferredFetchResult {
        message: AssistantMessage {
            id: handle.id.clone(),
            role: "assistant".into(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            model: model.id.clone(),
            usage: None,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
        },
        deferred: None,
    }
}

fn error_result(model: &Model, error: String) -> DeferredFetchResult {
    DeferredFetchResult {
        message: AssistantMessage {
            id: String::new(),
            role: "assistant".into(),
            content: Vec::new(),
            model: model.id.clone(),
            usage: None,
            stop_reason: Some(StopReason::Error),
            error_message: Some(error),
        },
        deferred: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_builtin_models;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn handle_for(model: &Model) -> DeferredHandle {
        DeferredHandle {
            provider: model.provider.clone(),
            model_id: model.id.clone(),
            api: model.api.clone(),
            id: "resp_1".into(),
            expires_at: None,
            poll_after_ms: Some(25),
            data: None,
        }
    }

    #[test]
    fn fetch_deferred_pending_then_ready_from_fixtures() {
        let _guard = ENV_LOCK.lock().unwrap();
        let model = load_builtin_models().into_iter().next().expect("catalog");
        let handle = handle_for(&model);
        std::env::set_var("PI_DEFERRED_FETCH_REPLY", "pending");
        let pending = fetch_deferred(&model, &handle, &DeferredFetchOptions { wait: Some(0) });
        assert_eq!(pending.message.stop_reason, Some(StopReason::Deferred));
        assert_eq!(
            pending.deferred.as_ref().map(|item| item.id.as_str()),
            Some("resp_1")
        );
        std::env::set_var("PI_DEFERRED_FETCH_REPLY", "hello-deferred");
        let ready = fetch_deferred(&model, &handle, &DeferredFetchOptions { wait: Some(0) });
        assert_eq!(ready.message.stop_reason, Some(StopReason::Stop));
        assert!(matches!(
            ready.message.content.first(),
            Some(ContentBlock::Text { text }) if text == "hello-deferred"
        ));
        std::env::remove_var("PI_DEFERRED_FETCH_REPLY");
    }

    #[test]
    fn cancel_deferred_writes_request_and_records_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let path = dir.path().join("cancel.json");
        let model = load_builtin_models().into_iter().next().expect("catalog");
        let handle = handle_for(&model);
        std::env::set_var("PI_DEFERRED_CANCEL_REQUEST", path.display().to_string());
        std::env::set_var("PI_DEFERRED_CANCEL_REPLY", "ok");
        cancel_deferred(&model, &handle).unwrap();
        let recorded = std::fs::read_to_string(&path).unwrap();
        assert!(recorded.contains("resp_1"));
        std::env::set_var("PI_DEFERRED_CANCEL_REPLY", "error");
        assert!(cancel_deferred(&model, &handle)
            .unwrap_err()
            .contains("cancelled"));
        std::env::remove_var("PI_DEFERRED_CANCEL_REQUEST");
        std::env::remove_var("PI_DEFERRED_CANCEL_REPLY");
    }

    #[test]
    fn fetch_deferred_abort_fixture() {
        let _guard = ENV_LOCK.lock().unwrap();
        let model = load_builtin_models().into_iter().next().expect("catalog");
        let handle = handle_for(&model);
        std::env::set_var("PI_DEFERRED_FETCH_ABORT", "1");
        let result = fetch_deferred(&model, &handle, &DeferredFetchOptions::default());
        std::env::remove_var("PI_DEFERRED_FETCH_ABORT");
        assert_eq!(result.message.stop_reason, Some(StopReason::Aborted));
    }
}
