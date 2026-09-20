use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_manifest::ProvenanceKind;
use davinci_agent::runtime::context_vm::{
    context_checkpoint_entry, events_from_messages, events_from_session_branch,
    latest_persisted_root, CheckpointProposal, ContextEvent, ContextEventKind, ContextObject,
    ContextPageKind, ContextPageRef, ContextRoot, ContextStateReducer, ContextVmConfig,
    ContextVmRuntime, FoldReason, ProposedStateValue, RetrieveContextRequest,
};
use davinci_agent::{ContextItem, ContextPacket};
use davinci_ai::{ChatMessage, MessageContent};
use davinci_session::SessionEntry;

#[test]
fn historical_tool_evidence_never_becomes_an_orphan_protocol_result() {
    let runtime = ContextVmRuntime::new(ContextVmConfig::default(), CacheRuntime::default());
    let events = events_from_messages(&[
        ChatMessage::text("user", "inspect"),
        ChatMessage::tool_result("old-call", "read", "historical evidence", false),
        ChatMessage::text("user", "continue"),
    ]);
    let image = runtime
        .compile(&events, &ContextPacket::empty(), 100_000)
        .unwrap();
    assert!(image.messages.iter().all(|m| m.role != "toolResult"));
    let wire = davinci_ai::openai_responses_input(&image.messages);
    assert!(wire
        .iter()
        .all(|item| item["type"] != "function_call_output"));
    assert!(image.messages.iter().any(|m| m.role == "custom"
        && davinci_ai::content_text(&m.content).contains("historical evidence")));
}

#[test]
fn active_vm_keeps_the_current_tool_exchange_paired_on_the_wire() {
    use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
    let mut agent = Agent::new("system");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(davinci_agent::runtime::ContextVmMode::Active);
    agent.messages = vec![
        ChatMessage::text("user", "read"),
        ChatMessage {
            role: "assistant".into(),
            content: vec![MessageContent::ToolCall {
                id: "live-call".into(),
                name: "read".into(),
                arguments: serde_json::json!({"path":"a"}),
            }],
            ..Default::default()
        },
        ChatMessage::tool_result("live-call", "read", "result", false),
    ];
    let wire = davinci_ai::openai_responses_input(&agent.context_vm_image().unwrap().messages);
    let calls: Vec<_> = wire
        .iter()
        .filter(|v| v["type"] == "function_call")
        .collect();
    let results: Vec<_> = wire
        .iter()
        .filter(|v| v["type"] == "function_call_output")
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(results.len(), 1);
    assert_eq!(calls[0]["call_id"], "live-call");
    assert_eq!(results[0]["call_id"], "live-call");
}

fn session_message(
    id: &str,
    parent_id: Option<&str>,
    seq: u64,
    message: ChatMessage,
) -> SessionEntry {
    SessionEntry {
        id: id.into(),
        entry_type: "message".into(),
        parent_id: parent_id.map(str::to_string),
        seq,
        timestamp: 0,
        message: Some(serde_json::to_value(message).unwrap()),
        custom_type: None,
        extra: serde_json::Map::new(),
    }
}

#[test]
fn branch_replay_follows_the_active_leaf_and_ignores_siblings() {
    let entries = vec![
        session_message("root", None, 1, ChatMessage::text("user", "keep this")),
        session_message(
            "active",
            Some("root"),
            2,
            ChatMessage::text("assistant", "active"),
        ),
        session_message(
            "sibling",
            Some("root"),
            3,
            ChatMessage::text("assistant", "ignore"),
        ),
    ];

    let events = events_from_session_branch(&entries, Some("active"));
    let visible = events
        .iter()
        .map(|event| event.visible_text.as_str())
        .collect::<Vec<_>>();

    assert_eq!(visible, vec!["keep this", "active"]);
    assert!(events
        .iter()
        .all(|event| event.source_ref != "session:sibling"));
}

#[test]
fn replay_is_repeatable_and_never_projects_thinking() {
    let messages = vec![
        ChatMessage::text("user", "visible request"),
        ChatMessage {
            role: "assistant".into(),
            content: vec![
                MessageContent::Thinking {
                    thinking: "private reasoning".into(),
                    redacted: None,
                },
                MessageContent::Text {
                    text: "visible answer".into(),
                },
                MessageContent::ToolCall {
                    id: "call-1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path":"src/lib.rs"}),
                },
            ],
            ..ChatMessage::default()
        },
        ChatMessage::tool_result("call-1", "read", "verified evidence", false),
    ];

    let first = events_from_messages(&messages);
    let second = events_from_messages(&messages);
    assert_eq!(first, second);
    assert_eq!(first.len(), 3);
    assert!(first
        .iter()
        .all(|event| !event.visible_text.contains("private")));
    assert!(first
        .iter()
        .any(|event| event.visible_text.contains("tool_call read")));
    assert_eq!(first[0].kind, ContextEventKind::User);
    assert_eq!(first[2].provenance_kind, ProvenanceKind::ToolEvidence);
}

#[test]
fn tool_governor_artifact_metadata_survives_context_projection() {
    let mut message = ChatMessage::tool_result("call-1", "read", "compact view", false);
    message.extra.insert(
        "tokenGovernor".into(),
        serde_json::json!({
            "compressed": true,
            "outputId": "out-exact",
            "reference": "governor://out-exact",
            "contentHash": "a".repeat(64)
        }),
    );

    let events = events_from_messages(&[message]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].artifact_refs.len(), 1);
    assert_eq!(events[0].artifact_refs[0].uri, "governor://out-exact");
    assert_eq!(events[0].artifact_refs[0].content_hash, "a".repeat(64));
}

#[test]
fn reducer_requires_resolvable_provenance_and_preserves_parent_state() {
    let events = vec![
        ContextEvent {
            source_ref: "user:1".into(),
            seq: 1,
            kind: ContextEventKind::User,
            provenance_kind: ProvenanceKind::UserDecision,
            content_hash: "user-hash".into(),
            visible_text: "preserve the public API".into(),
            artifact_refs: Vec::new(),
        },
        ContextEvent {
            source_ref: "tool:1".into(),
            seq: 2,
            kind: ContextEventKind::ToolResult,
            provenance_kind: ProvenanceKind::ToolEvidence,
            content_hash: "tool-hash".into(),
            visible_text: "test passed".into(),
            artifact_refs: Vec::new(),
        },
    ];
    let parent =
        ContextStateReducer::deterministic_delta(&Default::default(), &events).checkpoint_patch;
    let proposal = CheckpointProposal {
        goals: vec![ProposedStateValue {
            value: "preserve the public API".into(),
            source_refs: vec!["user:1".into()],
            provenance_kind: ProvenanceKind::UserDecision,
        }],
        verification: vec![ProposedStateValue {
            value: "test passed".into(),
            source_refs: vec!["tool:1".into()],
            provenance_kind: ProvenanceKind::ToolEvidence,
        }],
        narrative: Some(ProposedStateValue {
            value: "invented without a source".into(),
            source_refs: vec!["missing".into()],
            provenance_kind: ProvenanceKind::AgentInference,
        }),
        ..CheckpointProposal::default()
    };

    let reduced = ContextStateReducer::validate_proposal(&parent, &events, proposal);
    assert!(reduced
        .goals
        .iter()
        .any(|value| value.value == "preserve the public API"));
    assert!(reduced
        .verification
        .iter()
        .any(|value| value.value == "test passed"));
    assert!(!reduced
        .narrative
        .as_ref()
        .is_some_and(|value| value.value.contains("invented")));
    assert_eq!(reduced.through_seq, 2);
    assert!(reduced
        .goals
        .iter()
        .flat_map(|value| &value.provenance)
        .all(|provenance| !provenance.source_refs.is_empty()));
}

#[test]
fn repeated_delta_and_fold_retain_original_visible_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf())),
    );
    let initial = events_from_messages(&[ChatMessage::text(
        "user",
        "constraint: preserve the public API",
    )]);
    let all = events_from_messages(&[
        ChatMessage::text("user", "constraint: preserve the public API"),
        ChatMessage::tool_result("call-1", "test", "failed test evidence", true),
    ]);

    runtime.rebuild_from_events(&initial).unwrap();
    let with_delta = runtime.append_delta(&all).unwrap();
    assert!(with_delta.deltas.is_empty());
    assert_eq!(with_delta.updates_since_fold, 1);
    let folded = runtime.fold(FoldReason::Manual, &all).unwrap();
    assert!(folded.deltas.is_empty());
    let folded_again = runtime.fold(FoldReason::PhaseBoundary, &all).unwrap();
    assert!(folded_again.deltas.is_empty());

    let checkpoint = folded_again.checkpoint.as_ref().unwrap();
    let recovered = runtime
        .retrieve(&RetrieveContextRequest {
            page: Some(checkpoint.id.clone()),
            ..RetrieveContextRequest::default()
        })
        .unwrap();
    assert!(recovered.content.contains("preserve the public API"));
    assert!(recovered.content.contains("failed test evidence"));
    let state: ContextObject = serde_json::from_str(&recovered.content).unwrap();
    let ContextObject::Checkpoint(state) = state else {
        panic!("fold retrieval did not return a checkpoint")
    };
    let expected_user_ref = events_from_messages(&[ChatMessage::text(
        "user",
        "constraint: preserve the public API",
    )])[0]
        .source_ref
        .clone();
    assert!(state.goals.iter().any(|value| {
        value.value == "constraint: preserve the public API"
            && value
                .provenance
                .iter()
                .any(|provenance| provenance.source_refs.contains(&expected_user_ref))
    }));
    assert!(state.verification.iter().any(|value| {
        value.value == "failed test evidence"
            && value
                .provenance
                .iter()
                .any(|provenance| provenance.kind == ProvenanceKind::ToolEvidence)
    }));
    assert_eq!(
        runtime.last_fold_reason().as_deref(),
        Some("phase_boundary")
    );
    assert_eq!(runtime.metrics().folds, 2);
}

#[test]
fn compiler_omits_optional_episode_under_tight_budget_but_keeps_checkpoint_manifest_entry() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        davinci_agent::runtime::cache::CacheRuntime::new(
            davinci_agent::runtime::cache::CacheConfig::default(),
            Some(directory.path().to_path_buf()),
        ),
    );
    let events = events_from_messages(&[ChatMessage::text("user", "retain this constraint")]);
    runtime.fold(FoldReason::Manual, &events).unwrap();

    let complete = runtime
        .compile(&events, &ContextPacket::empty(), 10_000)
        .unwrap();
    let required_budget = complete
        .entries
        .iter()
        .filter(|entry| entry.category != "episode")
        .map(|entry| entry.estimated_tokens)
        .sum();
    let image = runtime
        .compile(&events, &ContextPacket::empty(), required_budget)
        .unwrap();
    assert!(image.estimated_tokens <= required_budget);
    assert!(image
        .entries
        .iter()
        .all(|entry| entry.category != "episode"));
    let manifest = runtime.manifest_entries(&image);
    let checkpoint = manifest
        .iter()
        .find(|entry| entry.category == "checkpoint")
        .unwrap();
    assert!(checkpoint.mandatory);
    assert!(checkpoint.selected);
}

#[test]
fn compiler_orders_pages_broker_context_and_hot_events() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        davinci_agent::runtime::cache::CacheRuntime::new(
            davinci_agent::runtime::cache::CacheConfig::default(),
            Some(directory.path().to_path_buf()),
        ),
    );
    let events = events_from_messages(&[
        ChatMessage::text("user", "constraint: preserve the API"),
        ChatMessage::tool_result("call-1", "test", "verified test result", false),
    ]);
    runtime.fold(FoldReason::Manual, &events).unwrap();
    let broker_packet = ContextPacket {
        items: vec![ContextItem {
            source: "repo:README.md".into(),
            content: "repository fact".into(),
            estimated_tokens: 4,
            priority: 10,
            stable_for_cache: true,
            provenance: serde_json::json!({"provenance_kind":"repository_fact"}),
        }],
        estimated_tokens: 4,
        cache_key: "fixture".into(),
    };

    let image = runtime.compile(&events, &broker_packet, 10_000).unwrap();
    let categories = image
        .entries
        .iter()
        .map(|entry| entry.category.as_str())
        .collect::<Vec<_>>();
    assert_eq!(categories[0], "checkpoint");
    assert!(
        categories
            .iter()
            .position(|category| *category == "episode")
            < categories
                .iter()
                .position(|category| *category == "broker_context")
    );
    assert!(
        categories
            .iter()
            .position(|category| *category == "broker_context")
            < categories
                .iter()
                .position(|category| *category == "hot_user")
    );
    assert!(image
        .entries
        .iter()
        .filter(|entry| entry.category == "episode")
        .all(|entry| entry.estimated_tokens <= 1024));
    assert!(runtime
        .manifest_entries(&image)
        .iter()
        .all(|entry| entry.selection_reason.as_deref() == Some("context_vm_selected")));
}

#[test]
fn missing_delta_page_replays_from_visible_events_before_projection() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        davinci_agent::runtime::cache::CacheRuntime::new(
            davinci_agent::runtime::cache::CacheConfig::default(),
            Some(directory.path().to_path_buf()),
        ),
    );
    let initial = events_from_messages(&[ChatMessage::text("user", "keep the API")]);
    let all = events_from_messages(&[
        ChatMessage::text("user", "keep the API"),
        ChatMessage::tool_result("call-1", "test", "failed test evidence", true),
    ]);
    runtime.rebuild_from_events(&initial).unwrap();
    let root = runtime.append_delta(&all).unwrap();
    assert!(root.deltas.is_empty());

    let mut broken = root;
    broken.deltas.push(ContextPageRef {
        id: format!("ctx:delta:{}", "f".repeat(64)),
        kind: ContextPageKind::Delta,
        content_hash: "f".repeat(64),
        estimated_tokens: 1,
    });
    runtime.install_root(broken, 2);

    let image = runtime
        .compile(&all, &ContextPacket::empty(), 10_000)
        .unwrap();
    assert!(image
        .entries
        .iter()
        .any(|entry| entry.content.contains("keep the API")));
    assert_eq!(runtime.metrics().retrieval_hits, 0);
    assert!(runtime.metrics().rebuild_successes >= 1);
}

#[test]
fn persisted_root_loader_follows_checkpoint_metadata() {
    let root = ContextRoot {
        epoch: 7,
        ..ContextRoot::default()
    };
    let entry = context_checkpoint_entry(&root, 9, "prefix-digest", None, 10);
    let (loaded, through_seq) =
        latest_persisted_root(std::slice::from_ref(&entry), Some(entry.id.as_str()))
            .expect("checkpoint metadata should be recoverable from the active branch");

    assert_eq!(loaded, root);
    assert_eq!(through_seq, 9);
}

#[test]
fn malformed_live_tool_exchanges_are_evidence_not_protocol_messages() {
    use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
    for (calls, results) in [
        (vec![""], vec![""]),
        (vec!["a"], vec!["unknown"]),
        (vec!["a", "a"], vec!["a", "a"]),
        (vec!["a", "a"], vec!["a"]),
        (vec!["a", ""], vec!["a"]),
        (vec!["a", "b"], vec!["a"]),
        (vec!["a"], vec!["a", "a"]),
    ] {
        let mut agent = Agent::new("system");
        agent.set_runtime(RuntimeHandle::new(
            RunId::new(),
            AgentId::new(),
            RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(davinci_agent::runtime::ContextVmMode::Active);
        agent.messages = vec![
            ChatMessage::text("user", "inspect"),
            ChatMessage {
                role: "assistant".into(),
                content: calls
                    .into_iter()
                    .map(|id| MessageContent::ToolCall {
                        id: id.into(),
                        name: "read".into(),
                        arguments: serde_json::json!({}),
                    })
                    .collect(),
                ..Default::default()
            },
        ];
        agent.messages.extend(
            results
                .into_iter()
                .map(|id| ChatMessage::tool_result(id, "read", "evidence", false)),
        );
        let image = agent.prepared_context_image().unwrap();
        let wire = davinci_ai::openai_responses_input(&image.messages);
        assert!(
            wire.iter()
                .all(|item| item["type"] != "function_call_output"
                    && item["type"] != "function_call"),
            "{wire:?}"
        );
        assert!(image
            .messages
            .iter()
            .any(|m| davinci_ai::content_text(&m.content).contains("evidence")));
    }
}

#[test]
fn session_sources_stay_on_disk_and_are_checked_against_authoritative_hashes() {
    let directory = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap();
    session
        .append_entry(session_message(
            "user-1",
            None,
            1,
            ChatMessage::text("user", "retain requirement"),
        ))
        .unwrap();
    session
        .append_entry(session_message(
            "tool-1",
            Some("user-1"),
            2,
            ChatMessage::tool_result("call", "read", "large source evidence", false),
        ))
        .unwrap();
    let events = events_from_session_branch(&session.entries, session.leaf_id.as_deref());
    let runtime = ContextVmRuntime::new(ContextVmConfig::default(), CacheRuntime::default());
    runtime.bind_session_source(session.path.clone(), session.header.id.clone());
    runtime.rebuild_from_events(&events).unwrap();
    assert_eq!(runtime.resident_source_bytes(), 0);
    let request = RetrieveContextRequest {
        source_ref: Some("session:tool-1".into()),
        ..Default::default()
    };
    assert_eq!(
        runtime.retrieve(&request).unwrap().content,
        "large source evidence"
    );
    let original = std::fs::read_to_string(&session.path).unwrap();
    std::fs::write(
        &session.path,
        original.replace("large source evidence", "corrupted source evidence"),
    )
    .unwrap();
    assert!(runtime
        .retrieve(&request)
        .unwrap_err()
        .contains("integrity"));
    std::fs::write(
        &session.path,
        original.replace(&session.header.id, "other-session"),
    )
    .unwrap();
    assert!(runtime.retrieve(&request).unwrap_err().contains("identity"));
}
