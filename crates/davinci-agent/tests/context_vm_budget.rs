use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::runtime::context::{ContextItem, ContextPacket};
use davinci_agent::runtime::context_vm::{
    events_from_messages, CheckpointState, ContextCompileRequest, ContextCompiler, ContextObject,
    ContextObjectStore, ContextRoot, ContextVmMode, StateDelta, StateValue,
};
use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::ChatMessage;

#[test]
fn provider_budget_includes_every_reservation_before_selection() {
    let mut agent = Agent::new("s".repeat(2048));
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.context_window = 4096;
    agent.thinking_level = davinci_protocol::ThinkingLevel::High;
    agent.set_provider_context_overhead_tokens(Some(1024));
    agent.messages.push(ChatMessage::text("user", "hello"));
    assert!(
        agent.context_vm_image().is_err(),
        "system, tools and output exceed window"
    );
}

#[test]
fn mandatory_pages_and_newest_event_respect_compile_budget() {
    let store = ContextObjectStore::new(CacheRuntime::default());
    let compiler = ContextCompiler::new(store.clone());
    let state = CheckpointState {
        narrative: Some(StateValue {
            value: "x".repeat(4096),
            provenance: Vec::new(),
        }),
        ..Default::default()
    };
    let checkpoint = store
        .save(&ContextObject::Checkpoint(state.clone()))
        .unwrap();
    let delta = store
        .save(&ContextObject::Delta(StateDelta {
            from_seq: 0,
            through_seq: 1,
            checkpoint_patch: state,
        }))
        .unwrap();
    let roots = [
        ContextRoot {
            checkpoint: Some(checkpoint),
            ..Default::default()
        },
        ContextRoot {
            deltas: vec![delta],
            ..Default::default()
        },
        ContextRoot::default(),
    ];
    let packet = ContextPacket::empty();
    for root in &roots {
        let events = events_from_messages(&[ChatMessage::text("user", "x".repeat(4096))]);
        let request = |max_tokens| ContextCompileRequest {
            root,
            hot_events: &events,
            broker_packet: &packet,
            max_tokens,
        };
        let image = compiler.compile(request(10_000)).unwrap();
        assert!(image.estimated_tokens <= 10_000);
        assert!(compiler.compile(request(image.estimated_tokens)).is_ok());
        assert!(
            compiler.compile(request(1)).is_err(),
            "oversized mandatory context was accepted"
        );
    }
}

#[test]
fn active_mode_falls_back_without_discarding_authoritative_messages() {
    let mut agent = Agent::new("system authority");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.messages = vec![ChatMessage::text("user", "x".repeat(4096))];
    agent.context_window = 1;
    agent.set_context_vm_mode(ContextVmMode::Off);
    let legacy = agent.messages_for_provider();
    agent.set_context_vm_mode(ContextVmMode::Active);
    assert!(agent.context_vm_image().is_err());
    assert_eq!(agent.messages_for_provider(), legacy);
}

#[test]
fn oversized_active_context_blocks_the_provider_and_marks_the_manifest() {
    let mut agent = Agent::new("system authority");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.messages = vec![ChatMessage::text("user", "x".repeat(4096))];
    agent.context_window = 1;
    agent.auto_compaction = false;
    agent.set_context_vm_mode(ContextVmMode::Active);

    // Exercise dispatch without relying on a previously prepared manifest.
    let mut provider_called = false;
    let result = agent.run_loop(|_| {
        provider_called = true;
        Err::<davinci_ai::AssistantMessage, _>("provider must not be reached".to_string())
    });
    assert!(!provider_called, "oversized context reached the provider");
    assert!(result.unwrap_err().contains("compilation token budget"));
    assert!(!agent.is_streaming);
    assert_eq!(agent.run_stats().model_turns, 0);
    let manifest = agent.prepare_context_manifest("request", RunId::new(), 1, 1);
    assert!(manifest.is_mandatory_violated());
}

#[test]
fn newest_event_takes_priority_over_optional_broker_context() {
    let compiler = ContextCompiler::new(ContextObjectStore::new(CacheRuntime::default()));
    let root = ContextRoot::default();
    let events = events_from_messages(&[ChatMessage::text("user", "next")]);
    let packet = ContextPacket {
        items: vec![ContextItem {
            source: "optional".into(),
            content: "x".repeat(80),
            estimated_tokens: 20,
            priority: 0,
            stable_for_cache: false,
            provenance: serde_json::Value::Null,
        }],
        estimated_tokens: 20,
        cache_key: "test".into(),
    };
    let image = compiler
        .compile(ContextCompileRequest {
            root: &root,
            hot_events: &events,
            broker_packet: &packet,
            max_tokens: 240,
        })
        .unwrap();
    assert_eq!(image.messages, vec![ChatMessage::text("user", "next")]);
    assert!(image.estimated_tokens <= 240);
}

#[test]
fn oversized_mandatory_broker_context_cannot_be_silently_dropped() {
    let compiler = ContextCompiler::new(ContextObjectStore::new(CacheRuntime::default()));
    let root = ContextRoot::default();
    let events = events_from_messages(&[ChatMessage::text("user", "next")]);
    for (source, provenance) in [
        ("mandatory:test", serde_json::Value::Null),
        (
            "policy",
            serde_json::json!({"provenance_kind":"mandatory_policy"}),
        ),
    ] {
        let packet = ContextPacket {
            items: vec![ContextItem {
                source: source.into(),
                content: "x".repeat(4096),
                estimated_tokens: 1,
                priority: 0,
                stable_for_cache: false,
                provenance,
            }],
            estimated_tokens: 1,
            cache_key: "test".into(),
        };
        let error = compiler
            .compile(ContextCompileRequest {
                root: &root,
                hot_events: &events,
                broker_packet: &packet,
                max_tokens: 512,
            })
            .unwrap_err();
        assert!(error.contains("compilation token budget"));
    }
}

#[test]
fn broker_preselection_cannot_hide_mandatory_overflow_from_dispatch() {
    use davinci_agent::runtime::context::{ContextRequest, ContextSource};
    struct MandatorySource;
    impl ContextSource for MandatorySource {
        fn collect(&self, request: &ContextRequest) -> Vec<ContextItem> {
            vec![ContextItem {
                source: "policy".into(),
                content: "required policy".repeat(20_000),
                estimated_tokens: request.max_tokens.saturating_add(1),
                priority: 0,
                stable_for_cache: true,
                provenance: serde_json::json!({"mandatory":true}),
            }]
        }
    }
    let mut agent = Agent::new("system");
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    runtime
        .context_broker
        .register_context_source(std::sync::Arc::new(MandatorySource));
    agent.set_runtime(runtime);
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.messages.push(ChatMessage::text("user", "hello"));
    agent.auto_compaction = false;
    let mut called = false;
    let result = agent.run_loop(|_| {
        called = true;
        Err::<davinci_ai::AssistantMessage, _>("must not call provider".into())
    });
    assert!(!called);
    assert!(result.unwrap_err().contains("compilation token budget"));
}

#[test]
fn latest_user_request_is_mandatory_after_many_tool_events() {
    use davinci_agent::runtime::context_vm::{ContextVmConfig, ContextVmRuntime};
    let vm = ContextVmRuntime::new(
        ContextVmConfig {
            hot_event_tokens: 64,
            ..Default::default()
        },
        CacheRuntime::default(),
    );
    let request = format!("{} essential final instruction", "requirement ".repeat(120));
    let mut messages = vec![ChatMessage::text("user", &request)];
    for n in 0..100 {
        messages.push(ChatMessage::tool_result(
            format!("call-{n}"),
            "read",
            "evidence",
            false,
        ));
    }
    let events = events_from_messages(&messages);
    let image = vm
        .compile(&events, &ContextPacket::empty(), 100_000)
        .unwrap();
    assert!(image
        .messages
        .iter()
        .any(|m| m.role == "user" && davinci_ai::content_text(&m.content) == request));
    assert!(image
        .entries
        .iter()
        .any(|e| e.source_ref == events[0].source_ref && e.mandatory));
}

#[test]
fn unrecoverable_page_storage_cannot_dispatch_an_unbounded_legacy_fallback() {
    use davinci_agent::runtime::{
        cache::CacheConfig,
        context_vm::{ContextVmConfig, ContextVmRuntime},
    };
    let mut runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    runtime.context_vm = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(
            CacheConfig {
                enabled: false,
                ..Default::default()
            },
            None,
        ),
    );
    let mut agent = Agent::new("system");
    agent.set_runtime(runtime);
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.auto_compaction = false;
    agent.context_window = 32_000;
    agent.set_provider_output_limit(Some(1024));
    agent
        .messages
        .push(ChatMessage::text("user", "source ".repeat(10_000)));
    let error = agent.prepared_context_image().unwrap_err();
    assert!(!error.contains("compilation token budget"), "{error}");
    let mut called = false;
    let result = agent.run_loop(|_| {
        called = true;
        Err::<davinci_ai::AssistantMessage, _>("must not dispatch".into())
    });
    assert!(
        !called,
        "a non-budget compilation error bypassed provider admission"
    );
    assert!(result.is_err());
    assert_eq!(agent.run_stats().model_turns, 0);
}

#[test]
fn oversized_complete_live_tool_exchange_blocks_dispatch() {
    use davinci_ai::MessageContent;
    let mut agent = Agent::new("system");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.set_provider_output_limit(Some(1024));
    agent.context_window = 16_000;
    agent.auto_compaction = false;
    agent.messages = vec![
        ChatMessage::text("user", "inspect"),
        ChatMessage {
            role: "assistant".into(),
            content: vec![MessageContent::ToolCall {
                id: "live".into(),
                name: "read".into(),
                arguments: serde_json::json!({}),
            }],
            ..Default::default()
        },
        ChatMessage::tool_result("live", "read", "source ".repeat(10_000), false),
    ];
    let mut called = false;
    let result = agent.run_loop(|_| {
        called = true;
        Err::<davinci_ai::AssistantMessage, _>("must not dispatch".into())
    });
    assert!(!called);
    assert!(result.unwrap_err().contains("compilation token budget"));
    assert_eq!(agent.run_stats().model_turns, 0);
}
