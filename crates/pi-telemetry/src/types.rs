use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AttributeValue {
    String(String),
    Number(f64),
    Boolean(bool),
    StringArray(Vec<String>),
    NumberArray(Vec<f64>),
    BooleanArray(Vec<bool>),
}

pub type SpanAttributes = HashMap<String, AttributeValue>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpanOptions {
    pub name: String,
    pub attributes: Option<SpanAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SpanStatus {
    Ok,
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<SpanErrorDetail>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpanErrorDetail {
    pub name: String,
    pub message: String,
}

#[async_trait]
pub trait TelemetryContext: Send + Sync {
    async fn start_span_boxed(
        &self,
        options: SpanOptions,
        callback: Box<
            dyn FnOnce(Arc<dyn TelemetrySpan>) -> futures::future::BoxFuture<'static, ()>
                + Send
                + 'static,
        >,
    );
}

pub trait TelemetrySpan: Send + Sync {
    fn add_event(&self, name: &str, attributes: Option<SpanAttributes>);
    fn set_attributes(&self, attributes: SpanAttributes);
    fn set_status(&self, status: SpanStatus);
}

#[derive(Debug, Clone, Default)]
pub struct NoopTelemetryContext;

#[async_trait]
impl TelemetryContext for NoopTelemetryContext {
    async fn start_span_boxed(
        &self,
        _options: SpanOptions,
        callback: Box<
            dyn FnOnce(Arc<dyn TelemetrySpan>) -> futures::future::BoxFuture<'static, ()>
                + Send
                + 'static,
        >,
    ) {
        let span = Arc::new(NoopTelemetrySpan);
        callback(span).await;
    }
}

pub struct NoopTelemetrySpan;

impl TelemetrySpan for NoopTelemetrySpan {
    fn add_event(&self, _name: &str, _attributes: Option<SpanAttributes>) {}
    fn set_attributes(&self, _attributes: SpanAttributes) {}
    fn set_status(&self, _status: SpanStatus) {}
}
