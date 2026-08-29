//! Event bus matching TypeScript `packages/coding-agent/src/core/event-bus.ts`.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type Handler = Arc<dyn Fn(&Value) + Send + Sync>;

#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    listeners: HashMap<String, Vec<Handler>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&self, channel: &str, data: Value) {
        let handlers = {
            let guard = self.inner.lock().expect("event bus");
            guard.listeners.get(channel).cloned().unwrap_or_default()
        };
        for handler in handlers {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&data)));
            if let Err(err) = result {
                eprintln!("Event handler error ({channel}): {err:?}");
            }
        }
    }

    pub fn on(&self, channel: &str, handler: Handler) -> impl Fn() + Send + Sync + 'static {
        let inner = Arc::clone(&self.inner);
        let channel = channel.to_string();
        {
            let mut guard = inner.lock().expect("event bus");
            guard
                .listeners
                .entry(channel.clone())
                .or_default()
                .push(handler);
        }
        let inner_off = Arc::clone(&self.inner);
        move || {
            let mut guard = inner_off.lock().expect("event bus");
            guard.listeners.remove(&channel);
        }
    }

    pub fn clear(&self) {
        self.inner.lock().expect("event bus").listeners.clear();
    }

    pub fn channel_count(&self) -> usize {
        self.inner.lock().expect("event bus").listeners.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn emit_on_clear() {
        let bus = EventBus::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = Arc::clone(&hits);
        let off = bus.on(
            "agent_start",
            Arc::new(move |_| {
                hits2.fetch_add(1, Ordering::SeqCst);
            }),
        );
        bus.emit("agent_start", json!({}));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        off();
        bus.emit("agent_start", json!({}));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let _keep = bus.on("x", Arc::new(|_| {}));
        bus.clear();
        assert_eq!(bus.channel_count(), 0);
    }
}
