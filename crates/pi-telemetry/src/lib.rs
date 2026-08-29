//! Telemetry contracts matching `@earendil-works/pi-telemetry`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type AttributeValue = Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpanAttributes {
    #[serde(flatten)]
    pub values: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanOptions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<SpanAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SpanStatus {
    Ok,
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<SpanError>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanError {
    pub name: String,
    pub message: String,
}

pub trait TelemetrySpan: Send {
    fn add_event(&mut self, name: &str, attributes: Option<SpanAttributes>);
    fn set_attributes(&mut self, attributes: SpanAttributes);
    fn set_status(&mut self, status: SpanStatus);
}

pub trait TelemetryContext: Send + Sync {
    fn start_span<T, F>(&self, options: SpanOptions, callback: F) -> T
    where
        F: FnOnce(&mut dyn TelemetrySpan) -> T;
}

#[derive(Debug, Default)]
pub struct NoopSpan;

impl TelemetrySpan for NoopSpan {
    fn add_event(&mut self, _name: &str, _attributes: Option<SpanAttributes>) {}
    fn set_attributes(&mut self, _attributes: SpanAttributes) {}
    fn set_status(&mut self, _status: SpanStatus) {}
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTelemetryContext;

impl TelemetryContext for NoopTelemetryContext {
    fn start_span<T, F>(&self, _options: SpanOptions, callback: F) -> T
    where
        F: FnOnce(&mut dyn TelemetrySpan) -> T,
    {
        let mut span = NoopSpan;
        callback(&mut span)
    }
}

pub const NOOP_TELEMETRY_CONTEXT: NoopTelemetryContext = NoopTelemetryContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEvent {
    pub name: String,
    pub attributes: SpanAttributes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpan {
    pub name: String,
    pub attributes: SpanAttributes,
    pub events: Vec<MemoryEvent>,
    pub status: SpanStatus,
}

#[derive(Debug, Default)]
pub struct MemoryTelemetryContext {
    pub spans: std::sync::Mutex<Vec<MemorySpan>>,
}

impl TelemetryContext for MemoryTelemetryContext {
    fn start_span<T, F>(&self, options: SpanOptions, callback: F) -> T
    where
        F: FnOnce(&mut dyn TelemetrySpan) -> T,
    {
        let mut span = RecordingSpan {
            name: options.name,
            attributes: options.attributes.unwrap_or_default(),
            events: Vec::new(),
            status: SpanStatus::Ok,
        };
        let result = callback(&mut span);
        if let Ok(mut spans) = self.spans.lock() {
            spans.push(MemorySpan {
                name: span.name,
                attributes: span.attributes,
                events: span.events,
                status: span.status,
            });
        }
        result
    }
}

struct RecordingSpan {
    name: String,
    attributes: SpanAttributes,
    events: Vec<MemoryEvent>,
    status: SpanStatus,
}

impl TelemetrySpan for RecordingSpan {
    fn add_event(&mut self, name: &str, attributes: Option<SpanAttributes>) {
        self.events.push(MemoryEvent {
            name: name.to_string(),
            attributes: attributes.unwrap_or_default(),
        });
    }

    fn set_attributes(&mut self, attributes: SpanAttributes) {
        self.attributes = attributes;
    }

    fn set_status(&mut self, status: SpanStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetryAttributeDefinition {
    #[serde(rename = "type")]
    pub type_name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetrySpanDefinition {
    pub description: String,
    #[serde(default)]
    pub start_attributes: serde_json::Map<String, Value>,
    #[serde(default)]
    pub end_attributes: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetrySchemaDefinition {
    pub version: u32,
    pub spans: serde_json::Map<String, Value>,
}

pub fn define_telemetry_schema(schema: TelemetrySchemaDefinition) -> TelemetrySchemaDefinition {
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_context_records_spans() {
        let context = MemoryTelemetryContext::default();
        context.start_span(
            SpanOptions {
                name: "agent.turn".into(),
                attributes: None,
            },
            |span| {
                span.add_event("start", None);
                span.set_status(SpanStatus::Ok);
            },
        );
        let spans = context.spans.lock().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "agent.turn");
        assert_eq!(spans[0].events[0].name, "start");
        let span = TelemetrySpanDefinition {
            description: "One agent turn".into(),
            start_attributes: {
                let mut attrs = serde_json::Map::new();
                attrs.insert(
                    "provider".into(),
                    serde_json::to_value(TelemetryAttributeDefinition {
                        type_name: "string".into(),
                        description: "Provider id".into(),
                        required: true,
                        sensitive: None,
                    })
                    .unwrap(),
                );
                attrs
            },
            end_attributes: serde_json::Map::new(),
        };
        let schema = define_telemetry_schema(TelemetrySchemaDefinition {
            version: 1,
            spans: {
                let mut spans = serde_json::Map::new();
                spans.insert("agent.turn".into(), serde_json::to_value(span).unwrap());
                spans
            },
        });
        assert_eq!(schema.version, 1);
        assert!(schema.spans.contains_key("agent.turn"));
    }
}
