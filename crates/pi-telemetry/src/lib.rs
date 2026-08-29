//! Vendor-neutral telemetry contracts matching `@earendil-works/pi-telemetry`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpanAttributes {
    #[serde(flatten)]
    pub values: serde_json::Map<String, Value>,
}

impl SpanAttributes {
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<Value>) {
        self.values.insert(name.into(), value.into());
    }
}

#[derive(Debug, Clone)]
pub struct SpanOptions {
    pub name: String,
    pub attributes: Option<SpanAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status")]
pub enum SpanStatus {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "error")]
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<SpanErrorInfo>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpanErrorInfo {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelemetrySchemaDefinition {
    pub version: u32,
    pub spans: serde_json::Map<String, Value>,
}

pub fn define_telemetry_schema(schema: TelemetrySchemaDefinition) -> TelemetrySchemaDefinition {
    schema
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedTelemetryEvent {
    pub name: String,
    pub attributes: SpanAttributes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedTelemetrySpan {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub attributes: SpanAttributes,
    pub events: Vec<RecordedTelemetryEvent>,
    pub status: SpanStatus,
    pub settled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_sequence: Option<u64>,
}

#[derive(Debug, Default)]
struct MemoryState {
    spans: Vec<RecordedTelemetrySpan>,
    next_span_id: u64,
    next_end_sequence: u64,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryTelemetryContext {
    state: Arc<Mutex<MemoryState>>,
}

#[derive(Debug, Clone)]
pub struct TelemetrySpan {
    state: Option<Arc<Mutex<MemoryState>>>,
    id: u64,
}

impl TelemetrySpan {
    pub fn add_event(&self, name: &str, attributes: Option<SpanAttributes>) {
        let Some(state) = &self.state else {
            return;
        };
        let mut guard = state.lock().expect("telemetry lock");
        if let Some(span) = guard.spans.iter_mut().find(|s| s.id == self.id) {
            if span.settled {
                return;
            }
            span.events.push(RecordedTelemetryEvent {
                name: name.to_string(),
                attributes: attributes.unwrap_or_default(),
            });
        }
    }

    pub fn set_attributes(&self, attributes: SpanAttributes) {
        let Some(state) = &self.state else {
            return;
        };
        let mut guard = state.lock().expect("telemetry lock");
        if let Some(span) = guard.spans.iter_mut().find(|s| s.id == self.id) {
            if span.settled {
                return;
            }
            span.attributes.values.extend(attributes.values);
        }
    }

    pub fn set_status(&self, status: SpanStatus) {
        let Some(state) = &self.state else {
            return;
        };
        let mut guard = state.lock().expect("telemetry lock");
        if let Some(span) = guard.spans.iter_mut().find(|s| s.id == self.id) {
            if span.settled {
                return;
            }
            span.status = status;
        }
    }

    pub fn start_span<T>(
        &self,
        options: SpanOptions,
        callback: impl FnOnce(&TelemetrySpan) -> T,
    ) -> T {
        match &self.state {
            Some(state) => start_memory_span(state, Some(self.id), options, callback),
            None => callback(&NOOP_SPAN),
        }
    }
}

pub static NOOP_SPAN: TelemetrySpan = TelemetrySpan { state: None, id: 0 };

pub struct NoopTelemetry;

pub static NOOP_TELEMETRY_CONTEXT: NoopTelemetry = NoopTelemetry;

impl NoopTelemetry {
    pub fn start_span<T>(
        &self,
        _options: SpanOptions,
        callback: impl FnOnce(&TelemetrySpan) -> T,
    ) -> T {
        callback(&NOOP_SPAN)
    }
}

impl InMemoryTelemetryContext {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryState {
                spans: Vec::new(),
                next_span_id: 1,
                next_end_sequence: 1,
            })),
        }
    }

    pub fn start_span<T>(
        &self,
        options: SpanOptions,
        callback: impl FnOnce(&TelemetrySpan) -> T,
    ) -> T {
        start_memory_span(&self.state, None, options, callback)
    }

    pub fn get_spans(&self) -> Vec<RecordedTelemetrySpan> {
        self.state.lock().expect("telemetry lock").spans.clone()
    }
}

fn start_memory_span<T>(
    state: &Arc<Mutex<MemoryState>>,
    parent_id: Option<u64>,
    options: SpanOptions,
    callback: impl FnOnce(&TelemetrySpan) -> T,
) -> T {
    if let Some(parent) = parent_id {
        let settled = state
            .lock()
            .expect("telemetry lock")
            .spans
            .iter()
            .find(|s| s.id == parent)
            .map(|s| s.settled)
            .unwrap_or(true);
        if settled {
            return NOOP_TELEMETRY_CONTEXT.start_span(options, callback);
        }
    }

    let id = {
        let mut guard = state.lock().expect("telemetry lock");
        let id = guard.next_span_id;
        guard.next_span_id += 1;
        guard.spans.push(RecordedTelemetrySpan {
            id,
            parent_id,
            name: options.name,
            attributes: options.attributes.unwrap_or_default(),
            events: Vec::new(),
            status: SpanStatus::Ok,
            settled: false,
            end_sequence: None,
        });
        id
    };

    let span = TelemetrySpan {
        state: Some(Arc::clone(state)),
        id,
    };
    let result = callback(&span);
    let mut guard = state.lock().expect("telemetry lock");
    let end_sequence = guard.next_end_sequence;
    if let Some(recorded) = guard.spans.iter_mut().find(|s| s.id == id) {
        if !recorded.settled {
            recorded.settled = true;
            recorded.end_sequence = Some(end_sequence);
            guard.next_end_sequence += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn define_schema_is_identity() {
        let schema = define_telemetry_schema(TelemetrySchemaDefinition {
            version: 1,
            spans: serde_json::Map::new(),
        });
        assert_eq!(schema.version, 1);
    }

    #[test]
    fn memory_records_nested_spans() {
        let ctx = InMemoryTelemetryContext::new();
        ctx.start_span(
            SpanOptions {
                name: "operation".into(),
                attributes: None,
            },
            |span| {
                span.add_event("result", None);
                span.start_span(
                    SpanOptions {
                        name: "child".into(),
                        attributes: None,
                    },
                    |_| 1,
                )
            },
        );
        let spans = ctx.get_spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].name, "operation");
        assert_eq!(spans[1].parent_id, Some(spans[0].id));
        assert!(spans.iter().all(|s| s.settled));
    }
}
