//! Deterministic lifecycle event bus and subscription model.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use super::events::{RuntimeEvent, RuntimeEventEnvelope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDecision {
    Continue,
    Deny { reason: String },
}

pub trait RuntimeSubscriber: Send + Sync {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision;
}

#[derive(Default)]
struct RuntimeBusInner {
    subscribers: Mutex<Vec<Arc<dyn RuntimeSubscriber>>>,
}

#[derive(Clone, Default)]
pub struct RuntimeBus {
    inner: Arc<RuntimeBusInner>,
}

impl RuntimeBus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RuntimeBusInner {
                subscribers: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn subscribe(&self, subscriber: Arc<dyn RuntimeSubscriber>) {
        if let Ok(mut subs) = self.inner.subscribers.lock() {
            subs.push(subscriber);
        }
    }

    /// Emits an event to observers in registration order. Observer panics are
    /// caught at the boundary to fail open and protect the caller.
    pub fn emit_observe(&self, event: RuntimeEventEnvelope) {
        let subscribers = match self.inner.subscribers.lock() {
            Ok(subs) => subs.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };

        for subscriber in subscribers {
            let res = catch_unwind(AssertUnwindSafe(|| {
                subscriber.on_event(&event);
            }));
            if let Err(_panic_err) = res {
                eprintln!(
                    "[davinci-runtime] Warning: RuntimeSubscriber panicked during emit_observe for event sequence {}",
                    event.sequence
                );
            }
        }
    }

    /// Emits an event that can block execution. Returns Err(reason) if any
    /// subscriber returns Deny on an allowlisted decision event.
    /// Non-allowlisted event kinds cannot block and will ignore Deny.
    pub fn emit_decision(&self, event: RuntimeEventEnvelope) -> Result<(), String> {
        let is_decision = is_decision_event(&event.payload);

        let subscribers = match self.inner.subscribers.lock() {
            Ok(subs) => subs.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };

        for subscriber in subscribers {
            let decision = match catch_unwind(AssertUnwindSafe(|| subscriber.on_event(&event))) {
                Ok(dec) => dec,
                Err(_panic_err) => {
                    // Decision subscribers fail closed on panic for safety
                    return Err("RuntimeSubscriber panicked during decision evaluation".into());
                }
            };

            if let RuntimeDecision::Deny { reason } = decision {
                if is_decision {
                    return Err(reason);
                } else {
                    eprintln!(
                        "[davinci-runtime] Warning: Ignored Deny decision for non-decision event kind: {:?}",
                        event.payload
                    );
                }
            }
        }

        Ok(())
    }
}

/// Only explicitly allowlisted event kinds may block agent execution.
pub fn is_decision_event(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::UserPromptSubmitted
            | RuntimeEvent::PreToolUse { .. }
            | RuntimeEvent::PermissionRequested { .. }
            | RuntimeEvent::PreModelSwitch { .. }
            | RuntimeEvent::TaskCompleted { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ids::{AgentId, RunId};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct OrderRecorder {
        id: usize,
        order: Arc<Mutex<Vec<usize>>>,
        deny_on_pre_tool: bool,
    }

    impl RuntimeSubscriber for OrderRecorder {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            self.order.lock().unwrap().push(self.id);
            if self.deny_on_pre_tool && matches!(event.payload, RuntimeEvent::PreToolUse { .. }) {
                RuntimeDecision::Deny {
                    reason: format!("denied by subscriber {}", self.id),
                }
            } else {
                RuntimeDecision::Continue
            }
        }
    }

    struct PanickingSubscriber;

    impl RuntimeSubscriber for PanickingSubscriber {
        fn on_event(&self, _event: &RuntimeEventEnvelope) -> RuntimeDecision {
            panic!("subscriber failure test");
        }
    }

    #[test]
    fn subscribers_called_in_registration_order_for_observe() {
        let bus = RuntimeBus::new();
        let order = Arc::new(Mutex::new(Vec::new()));

        bus.subscribe(Arc::new(OrderRecorder {
            id: 1,
            order: order.clone(),
            deny_on_pre_tool: false,
        }));
        bus.subscribe(Arc::new(OrderRecorder {
            id: 2,
            order: order.clone(),
            deny_on_pre_tool: false,
        }));
        bus.subscribe(Arc::new(OrderRecorder {
            id: 3,
            order: order.clone(),
            deny_on_pre_tool: false,
        }));

        let envelope = RuntimeEventEnvelope::new(
            1,
            RunId::new(),
            None,
            Some(AgentId::new()),
            None,
            RuntimeEvent::TurnStarted,
        );

        bus.emit_observe(envelope);
        let recorded = order.lock().unwrap().clone();
        assert_eq!(recorded, vec![1, 2, 3]);
    }

    #[test]
    fn subscriber_deny_stops_subsequent_subscribers_for_decision() {
        let bus = RuntimeBus::new();
        let order = Arc::new(Mutex::new(Vec::new()));

        bus.subscribe(Arc::new(OrderRecorder {
            id: 1,
            order: order.clone(),
            deny_on_pre_tool: false,
        }));
        bus.subscribe(Arc::new(OrderRecorder {
            id: 2,
            order: order.clone(),
            deny_on_pre_tool: true, // Denies!
        }));
        bus.subscribe(Arc::new(OrderRecorder {
            id: 3,
            order: order.clone(),
            deny_on_pre_tool: false,
        }));

        let envelope = RuntimeEventEnvelope::new(
            1,
            RunId::new(),
            None,
            Some(AgentId::new()),
            None,
            RuntimeEvent::PreToolUse {
                call_id: "c1".into(),
                tool: "bash".into(),
                args: json!({"command": "rm -rf /"}),
            },
        );

        let result = bus.emit_decision(envelope);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "denied by subscriber 2");

        // Subscriber 3 was never called
        let recorded = order.lock().unwrap().clone();
        assert_eq!(recorded, vec![1, 2]);
    }

    #[test]
    fn panicking_subscriber_fails_open_for_observe() {
        let bus = RuntimeBus::new();
        let counter = Arc::new(AtomicUsize::new(0));

        struct CounterSub(Arc<AtomicUsize>);
        impl RuntimeSubscriber for CounterSub {
            fn on_event(&self, _event: &RuntimeEventEnvelope) -> RuntimeDecision {
                self.0.fetch_add(1, Ordering::SeqCst);
                RuntimeDecision::Continue
            }
        }

        bus.subscribe(Arc::new(CounterSub(counter.clone())));
        bus.subscribe(Arc::new(PanickingSubscriber));
        bus.subscribe(Arc::new(CounterSub(counter.clone())));

        let envelope = RuntimeEventEnvelope::new(
            1,
            RunId::new(),
            None,
            Some(AgentId::new()),
            None,
            RuntimeEvent::TurnStarted,
        );

        // Does not panic and executes third subscriber
        bus.emit_observe(envelope);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn non_allowlisted_decision_cannot_block() {
        let bus = RuntimeBus::new();
        let order = Arc::new(Mutex::new(Vec::new()));

        bus.subscribe(Arc::new(OrderRecorder {
            id: 1,
            order: order.clone(),
            deny_on_pre_tool: true,
        }));

        // TurnStarted is NOT in the decision allowlist
        let envelope = RuntimeEventEnvelope::new(
            1,
            RunId::new(),
            None,
            Some(AgentId::new()),
            None,
            RuntimeEvent::TurnStarted,
        );

        let result = bus.emit_decision(envelope);
        assert!(
            result.is_ok(),
            "Non-decision event must not block even if Deny is returned"
        );
    }
}
