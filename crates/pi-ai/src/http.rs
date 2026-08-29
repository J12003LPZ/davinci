//! Live provider HTTP. Tests never call this; they inject SSE/HTTP fixtures.

use std::io::Read;
use std::time::Duration;

use crate::request::{
    build_request_body, default_base_url, endpoint_url, request_headers, RequestContext,
};
use crate::stream::StreamError;

#[derive(Debug, Clone)]
pub struct LiveRequest {
    pub provider: String,
    pub api: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: String,
    pub context: RequestContext,
    pub extra_headers: Vec<(String, String)>,
}

pub fn post_sse(request: &LiveRequest) -> Result<String, StreamError> {
    if std::env::var("PI_DISABLE_NETWORK").ok().as_deref() == Some("1") {
        return Err(StreamError::Message(
            "Network disabled (PI_DISABLE_NETWORK=1)".into(),
        ));
    }
    let base = request
        .base_url
        .clone()
        .unwrap_or_else(|| default_base_url(&request.provider).to_string());
    let url = endpoint_url(
        &request.api,
        &base,
        &request.model,
        Some(request.api_key.as_str()),
    );
    let body = build_request_body(&request.api, &request.model, &request.context);
    let mut req = ureq::post(&url).timeout(Duration::from_secs(120));
    for (name, value) in request_headers(&request.api, &request.provider, &request.api_key) {
        req = req.set(&name, &value);
    }
    for (name, value) in &request.extra_headers {
        req = req.set(name, value);
    }
    let response = req
        .send_json(body)
        .map_err(|e| StreamError::Message(format_ureq_error(&request.provider, e)))?;
    let mut reader = response.into_reader();
    let mut raw = String::new();
    reader
        .read_to_string(&mut raw)
        .map_err(|e| StreamError::Message(e.to_string()))?;
    Ok(raw)
}

fn format_ureq_error(provider: &str, error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            format!("Provider {provider} returned HTTP {code}: {body}")
        }
        other => format!("Provider {provider} request failed: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_network_blocks_live_calls() {
        std::env::set_var("PI_DISABLE_NETWORK", "1");
        let err = post_sse(&LiveRequest {
            provider: "openai".into(),
            api: "openai-completions".into(),
            model: "gpt-4o".into(),
            base_url: None,
            api_key: "sk-test".into(),
            context: RequestContext::default(),
            extra_headers: vec![],
        })
        .unwrap_err();
        assert!(err.to_string().contains("Network disabled"));
        std::env::remove_var("PI_DISABLE_NETWORK");
    }
}
