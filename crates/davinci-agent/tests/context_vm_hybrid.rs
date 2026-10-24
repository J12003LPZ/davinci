//! Hybrid Context VM: the provider gets the off-mode transcript, append-only;
//! the VM keeps the task ledger and supplies it when the transcript compacts.
use davinci_agent::runtime::context_vm::{cited_source_refs, RetrieveContextRequest};
use davinci_agent::runtime::ContextVmMode;
use davinci_agent::{
    Agent, AgentId, CompactionSettings, RunId, RuntimeBus, RuntimeHandle, SummarizeResponse,
    Summarizer,
};
use davinci_ai::{content_text, AssistantMessage, ChatMessage, ContentBlock, StopReason};
use davinci_session::JsonlSession;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NEVER: &str = "Never modify files under migrations/.";
const CORRECTION: &str = "Correction: use tokio, not async-std, for the runtime.";

fn agent(mode: ContextVmMode) -> Agent {
    let mut agent = Agent::new("You are a coding agent.");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(mode);
    agent.auto_compaction = true;
    agent
}

/// A summarizer that proposes nothing: what survives compaction is what the
/// deterministic ledger kept on its own.
fn lazy_summarizer(calls: Arc<AtomicUsize>) -> Summarizer {
    Summarizer::new(move |request| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(SummarizeResponse {
            text: if request.label == "context state fold" {
                "{}".into()
            } else {
                "## Goal\nContinue.".into()
            },
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    })
}

/// A reply that reports `input` provider tokens for its request.
fn reply(input: u64) -> AssistantMessage {
    AssistantMessage {
        extra: Default::default(),
        id: "fixture".into(),
        role: "assistant".into(),
        model: "fixture".into(),
        usage: Some(davinci_protocol::Usage {
            input,
            output: 4,
            ..Default::default()
        }),
        error_message: None,
        content: vec![ContentBlock::Text {
            text: "done".into(),
        }],
        stop_reason: Some(StopReason::Stop),
    }
}

/// The request's own size, at four bytes a token, as the provider count.
fn measured(agent: &Agent) -> u64 {
    serde_json::to_vec(&davinci_ai::openai_responses_input(
        &agent.messages_for_provider(),
    ))
    .unwrap()
    .len() as u64
        / 4
}

fn provider_text(agent: &Agent) -> String {
    agent
        .messages_for_provider()
        .iter()
        .map(|message| content_text(&message.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drive 60 turns in a 12k window: enough for several compactions. Each
/// turn checks that both requirements are in the request once stated.
fn long_session(agent: &mut Agent) {
    agent.context_window = 12_000;
    agent.compaction = CompactionSettings {
        reserve_tokens: 2_000,
        keep_recent_tokens: 1_000,
        ..CompactionSettings::default()
    };
    for turn in 0..60 {
        let text = match turn {
            0 => format!("Build the scheduler. {NEVER}"),
            7 => CORRECTION.to_string(),
            _ => format!("Continue with step {turn}. {}", "notes ".repeat(300)),
        };
        agent.prompt(&text);
        agent
            .run_loop(|agent: &Agent| Ok::<_, String>(reply(measured(agent))))
            .expect("turn runs");
        let text = provider_text(agent);
        assert!(text.contains(NEVER), "turn {turn} lost the constraint");
        if turn >= 7 {
            assert!(text.contains(CORRECTION), "turn {turn} lost the correction");
        }
    }
}

#[test]
fn hybrid_sends_exactly_the_off_mode_transcript_until_it_compacts() {
    let mut off = agent(ContextVmMode::Off);
    let mut hybrid = agent(ContextVmMode::Hybrid);
    for agent in [&mut off, &mut hybrid] {
        agent.prompt(NEVER);
        agent.record_assistant("I will leave migrations alone.");
        agent.messages.push(ChatMessage::tool_result(
            "call-1",
            "bash",
            "test result: ok",
            false,
        ));
        agent.prompt("Continue.");
    }
    assert_eq!(off.messages_for_provider(), hybrid.messages_for_provider());
    // No compiled image, so no Context VM cache partition either.
    assert_eq!(hybrid.context_vm_cache_affinity(), None);
}

fn wire_model() -> davinci_ai::Model {
    davinci_ai::Model {
        id: "gpt-6-luna".into(),
        name: "synthetic wire fixture".into(),
        provider: "openai-codex".into(),
        api: "openai-codex-responses".into(),
        base_url: Some("https://chatgpt.com/backend-api".into()),
        reasoning: true,
        input: vec!["text".into()],
        cost: davinci_ai::ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 128_000,
        max_tokens: 8_000,
        compat: serde_json::json!({}),
        headers: Default::default(),
        thinking_level_map: Default::default(),
    }
}

fn wire_agent(mode: ContextVmMode) -> Agent {
    let mut current = agent(mode);
    current.provider = "openai-codex".into();
    current.model_id = "gpt-6-luna".into();
    current.context_window = 128_000;
    current.tool_surface = davinci_agent::ToolSurface::Full;
    current.thinking_level = davinci_protocol::ThinkingLevel::High;
    current
}

fn full_request(
    current: &Agent,
) -> (
    davinci_ai::PreparedProviderRequest,
    davinci_agent::CacheIdentity,
) {
    let model = wire_model();
    let messages = current.messages_for_provider();
    let system = current.provider_system_prompt();
    let tools = current
        .provider_tool_specs()
        .into_iter()
        .map(|tool| davinci_ai::ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
            constrained_sampling: None,
        })
        .collect::<Vec<_>>();
    let resolved =
        davinci_ai::resolve_responses_tools(&model, model.base_url.as_deref(), true, &tools);
    let visible = current.visible_tool_names();
    let names = visible.iter().map(String::as_str).collect::<Vec<_>>();
    let identity = davinci_agent::CacheIdentity {
        provider: model.provider.clone(),
        model_id: model.id.clone(),
        system_prompt_hash: davinci_agent::hash_system_prompt_with_manifest(
            &system,
            current.prompt_manifest.as_ref(),
        ),
        tool_schema_hash: resolved.schema_digest,
        permission_surface_hash: davinci_agent::hash_tool_names(&names),
        context_item_hashes: current.context_vm_cache_affinity().into_iter().collect(),
        agent_profile_hash: None,
        contract_hash: None,
        role: Some("root".into()),
    };
    let mut body = davinci_ai::request_body_with(
        &model,
        &messages,
        Some(&system),
        &tools,
        &davinci_ai::StreamOptions {
            thinking_level: Some(current.request_thinking_level()),
            service_tier: Some(davinci_ai::CodexServiceTier::Standard),
            responses_is_oauth: Some(true),
            session_id: Some("synthetic-same-session".into()),
            cache_key: Some(identity.cache_key()),
            cache_retention: Some("short".into()),
            transport: Some("sse".into()),
            native_responses_resume: current.native_responses_resume_record_for(&messages),
            output_schema: current.output_schema.clone(),
            ..Default::default()
        },
    );
    assert!(davinci_ai::supports_incremental_stream(&model));
    body["stream"] = serde_json::json!(true);
    (davinci_ai::PreparedProviderRequest::new(body), identity)
}

fn native_fixture_response(
    call_id: &str,
    name: &str,
    arguments: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": format!("resp_{call_id}"), "status": "completed",
        "output": [
            {"type":"reasoning","id":format!("rs_{call_id}"),"summary":[],
             "encrypted_content":format!("synthetic_opaque_{call_id}")},
            {"type":"function_call","id":format!("fc_{call_id}"),"call_id":call_id,
             "name":name,"arguments":arguments.to_string(),"status":"completed"}
        ]
    })
}

fn native_history_pair(
    call_id: &str,
    name: &str,
    arguments: serde_json::Value,
    result: &str,
    failed: bool,
) -> Vec<ChatMessage> {
    let model = wire_model();
    let response = native_fixture_response(call_id, name, &arguments);
    let corpus = format!(
        "data: {}\n\n",
        serde_json::json!({
            "type":"response.completed","response":response,
        })
    );
    let decoded = davinci_ai::fixture_complete(&model, &[], &corpus);
    let mut chat = davinci_ai::assistant_to_chat(&decoded);
    let internal_id = chat
        .content
        .iter()
        .find_map(|block| match block {
            davinci_ai::MessageContent::ToolCall { id, .. } => Some(id.clone()),
            _ => None,
        })
        .unwrap();
    let output = davinci_ai::NativeResponsesOutput::from_response_value(&response).unwrap();
    davinci_ai::attach_native_items(
        &mut chat,
        &output.output_items,
        &format!("{}/{}", model.provider, model.id),
    );
    vec![
        chat,
        ChatMessage::tool_result(internal_id, name, result, failed),
    ]
}

fn append_native_branch_record(session: &mut JsonlSession, call_id: &str, path: &str) {
    let mut loaded = JsonlSession::open(&session.path).unwrap();
    loaded.set_leaf(session.leaf_id.clone());
    let mut snapshot = wire_agent(ContextVmMode::Off);
    snapshot.load_from_session(loaded).unwrap();
    let messages = snapshot.messages_for_provider();
    let assistant_index = messages
        .iter()
        .position(|message| message.role == "assistant")
        .unwrap();
    let mut before = wire_agent(ContextVmMode::Off);
    before.messages = messages[..assistant_index].to_vec();
    let prepared = full_request(&before).0;
    let response = native_fixture_response(call_id, "read", &serde_json::json!({"path":path}));
    let output = davinci_ai::NativeResponsesOutput::from_response_value(&response).unwrap();
    let record = davinci_ai::NativeResponsesResumeRecord {
        turn: davinci_ai::NativeResponsesTurn::from_prepared(&prepared, output).unwrap(),
        resume_provider_message_count: assistant_index + 1,
        resume_provider_messages_fingerprint: davinci_ai::provider_messages_fingerprint(
            &messages[..=assistant_index],
        ),
    };
    assert!(!record.turn.request_input_items.is_empty());
    drop(snapshot);
    session
        .append_entry(davinci_session::SessionEntry {
            id: format!("native_{call_id}"),
            entry_type: "custom".into(),
            parent_id: None,
            seq: 0,
            timestamp: 1_700_000_000_000,
            message: None,
            custom_type: Some(davinci_ai::NATIVE_RESPONSES_TURN_ENTRY_TYPE.into()),
            extra: serde_json::json!({"data":record})
                .as_object()
                .unwrap()
                .clone(),
        })
        .unwrap();
}

#[test]
fn complete_prepared_requests_match_through_unpruned_hybrid_history() {
    // Off deliberately prunes large histories; Hybrid does not. This fixture
    // tests the common pre-pruning/pre-compaction contract, not that difference.
    let mut off = wire_agent(ContextVmMode::Off);
    let mut hybrid = wire_agent(ContextVmMode::Hybrid);
    let summaries = Arc::new(AtomicUsize::new(0));
    for current in [&mut off, &mut hybrid] {
        current.summarizer = Some(lazy_summarizer(summaries.clone()));
        current.messages.push(ChatMessage::text("developer", NEVER));
        current.prompt(&format!("Build the async-std scheduler. {NEVER}"));
    }
    let mut previous: Option<serde_json::Value> = None;
    for checkpoint in 0..6 {
        let additions = match checkpoint {
            0 => vec![],
            1 => native_history_pair(
                "read_1",
                "read",
                serde_json::json!({"path":"src/scheduler.rs"}),
                "async_std::task::spawn",
                false,
            ),
            2 => {
                let mut history = native_history_pair(
                    "missing",
                    "read",
                    serde_json::json!({"path":"src/missing.rs"}),
                    "ENOENT: src/missing.rs",
                    true,
                );
                history.extend(native_history_pair(
                    "read_2",
                    "read",
                    serde_json::json!({"path":"src/scheduler.rs"}),
                    "async_std::task::spawn",
                    false,
                ));
                history
            }
            3 => vec![ChatMessage::text("user", CORRECTION)],
            4 => native_history_pair(
                "verify_1",
                "bash",
                serde_json::json!({"command":"cargo test scheduler"}),
                "test result: FAILED. 1 passed; 1 failed",
                true,
            ),
            _ => {
                let mut history = native_history_pair(
                    "read_3",
                    "read",
                    serde_json::json!({"path":"src/scheduler.rs"}),
                    "tokio::spawn",
                    false,
                );
                history.extend(native_history_pair(
                    "verify_2",
                    "bash",
                    serde_json::json!({"command":"cargo test scheduler"}),
                    "test result: ok. 2 passed; 0 failed",
                    false,
                ));
                history.push(ChatMessage::text("user", "Report current verification."));
                history
            }
        };
        let mut captures = Vec::new();
        for current in [&mut off, &mut hybrid] {
            current.messages.extend(additions.clone());
            let history = current.messages.clone();
            let mut calls = 0;
            current
                .run_loop(|state: &Agent| {
                    calls += 1;
                    captures.push(full_request(state));
                    Ok::<_, String>(reply(1_000))
                })
                .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(&current.messages[..history.len()], history.as_slice());
            assert_eq!(current.run_stats().compactions, 0);
            assert_eq!(current.run_stats().provider_retries, 0);
            assert!(current.pruned_tool_results().is_empty());
            assert_eq!(current.context_vm_cache_affinity(), None);
        }
        let (off_request, off_identity) = &captures[0];
        let (hybrid_request, hybrid_identity) = &captures[1];
        assert_eq!(off_identity, hybrid_identity);
        assert_eq!(
            off_request.body().to_string().as_bytes(),
            hybrid_request.body().to_string().as_bytes(),
            "checkpoint {checkpoint}"
        );
        assert_eq!(off_request.manifest(), hybrid_request.manifest());
        assert_eq!(
            off_request.first_changed_cache_segment(hybrid_request),
            None
        );
        let body = off_request.body();
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["prompt_cache_key"], off_identity.cache_key());
        assert!(!body.to_string().contains("Task ledger"));
        assert!(!body["tools"].to_string().contains("retrieve_context"));
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        if let Some(previous) = previous.as_ref() {
            assert_eq!(
                davinci_agent::cache_stability::first_prefix_break(previous, body),
                None
            );
        }
        let input = body["input"].as_array().unwrap();
        for call in input.iter().filter(|item| item["type"] == "function_call") {
            let id = call["call_id"].as_str().unwrap();
            assert!(!id.is_empty());
            assert_eq!(
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call_output" && item["call_id"] == id)
                    .count(),
                1
            );
            let reasoning = input
                .iter()
                .find(|item| item["id"] == format!("rs_{id}"))
                .unwrap();
            assert_eq!(
                reasoning["encrypted_content"],
                format!("synthetic_opaque_{id}")
            );
        }
        if checkpoint == 5 {
            let text = body.to_string();
            for evidence in [
                NEVER,
                CORRECTION,
                "async_std::task::spawn",
                "tokio::spawn",
                "ENOENT",
                "FAILED",
                "2 passed",
            ] {
                assert!(text.contains(evidence), "missing {evidence}");
            }
            assert_eq!(
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call")
                    .count(),
                6
            );
            assert_eq!(off.run_stats().model_turns, 6);
            assert_eq!(hybrid.run_stats().model_turns, 6);
        }
        previous = Some(body.clone());
    }
    assert_eq!(summaries.load(Ordering::SeqCst), 0);
}

#[test]
fn complete_requests_preserve_selected_native_branch_across_reload() {
    let temp = tempfile::tempdir().unwrap();
    let mut seed = JsonlSession::create(temp.path(), "synthetic", None).unwrap();
    let append = |session: &mut JsonlSession, id: &str, message: ChatMessage| {
        session
            .append_entry(davinci_session::SessionEntry {
                id: id.into(),
                entry_type: "message".into(),
                parent_id: None,
                seq: 0,
                timestamp: 1_700_000_000_000,
                message: Some(serde_json::to_value(message).unwrap()),
                custom_type: None,
                extra: Default::default(),
            })
            .unwrap();
    };
    append(&mut seed, "common", ChatMessage::text("user", NEVER));
    for (index, message) in native_history_pair(
        "active_read",
        "read",
        serde_json::json!({"path":"src/scheduler.rs"}),
        CORRECTION,
        false,
    )
    .into_iter()
    .enumerate()
    {
        append(&mut seed, &format!("active_{index}"), message);
    }
    append_native_branch_record(&mut seed, "active_read", "src/scheduler.rs");
    let active = seed.leaf_id.clone();
    seed.set_leaf(Some("common".into()));
    for (index, message) in native_history_pair(
        "abandoned_read",
        "read",
        serde_json::json!({"path":"old.rs"}),
        "ABANDONED-BRANCH-MARKER",
        false,
    )
    .into_iter()
    .enumerate()
    {
        append(&mut seed, &format!("abandoned_{index}"), message);
    }
    append_native_branch_record(&mut seed, "abandoned_read", "old.rs");
    let abandoned = seed.leaf_id.clone();
    let source = seed.path.clone();
    drop(seed);
    let mut paired = Vec::new();
    for mode in [ContextVmMode::Off, ContextVmMode::Hybrid] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.jsonl");
        std::fs::copy(&source, &path).unwrap();
        let mut loaded_requests = Vec::new();
        for (index, leaf) in [&active, &active, &abandoned].into_iter().enumerate() {
            let mut session = JsonlSession::open(&path).unwrap();
            session.set_leaf(leaf.clone());
            let mut current = wire_agent(mode);
            current.load_from_session(session).unwrap();
            current.set_context_vm_mode(mode);
            let messages = current.messages_for_provider();
            let resume = current
                .native_responses_resume_record_for(&messages)
                .unwrap();
            assert_eq!(
                resume.turn.output.response_id.as_deref(),
                Some(if index < 2 {
                    "resp_active_read"
                } else {
                    "resp_abandoned_read"
                })
            );
            assert!(!resume.turn.request_input_items.is_empty());
            loaded_requests.push(full_request(&current).0.body().clone());
        }
        assert_eq!(loaded_requests[0], loaded_requests[1]);
        assert_ne!(loaded_requests[0], loaded_requests[2]);
        let active_text = loaded_requests[0].to_string();
        assert!(active_text.contains(CORRECTION));
        assert!(active_text.contains("synthetic_opaque_active_read"));
        assert!(!active_text.contains("abandoned_read"));
        assert!(!active_text.contains("ABANDONED-BRANCH-MARKER"));
        assert!(loaded_requests[2]
            .to_string()
            .contains("synthetic_opaque_abandoned_read"));
        paired.push(loaded_requests);
    }
    assert_eq!(paired[0], paired[1]);
}

#[test]
fn requirements_survive_repeated_compaction_without_a_session() {
    let mut agent = agent(ContextVmMode::Hybrid);
    let calls = Arc::new(AtomicUsize::new(0));
    agent.summarizer = Some(lazy_summarizer(calls.clone()));
    long_session(&mut agent);
    let compactions = agent.run_stats().compactions;
    assert!(compactions >= 2, "{compactions}");
    // At most one fold request per compaction, never one per turn. In this
    // small window the fold prompt may not fit; the deterministic ledger
    // then carries the requirements alone.
    assert!(calls.load(Ordering::SeqCst) as u64 <= compactions);
    let summary = content_text(&agent.messages[0].content);
    assert!(summary.contains("Task ledger"), "{summary}");
    // The ledger rides in a user message but is never taken for one.
    let state = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .unwrap();
    assert!(state
        .goals
        .iter()
        .all(|goal| !goal.value.contains("Task ledger")));
}

#[test]
fn requirements_survive_repeated_compaction_and_reload_from_the_session() {
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.session = Some(JsonlSession::create(sessions.path(), "hybrid", None).unwrap());
    agent.summarizer = Some(lazy_summarizer(Arc::default()));
    long_session(&mut agent);
    assert!(agent.run_stats().compactions >= 2);

    let path = agent.session.as_ref().unwrap().path.clone();
    let mut reloaded = self::agent(ContextVmMode::Hybrid);
    reloaded
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    reloaded.set_context_vm_mode(ContextVmMode::Hybrid);
    let text = provider_text(&reloaded);
    assert!(text.contains(NEVER) && text.contains(CORRECTION));
    // What the provider receives is the same; reloaded messages carry extra
    // session metadata that the wire projection drops.
    assert_eq!(
        davinci_ai::openai_responses_input(&reloaded.messages_for_provider()),
        davinci_ai::openai_responses_input(&agent.messages_for_provider())
    );
}

#[test]
fn the_provider_count_triggers_compaction_where_the_estimate_would_not() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.context_window = 100_000;
    agent.prompt("Build the scheduler.");
    // The provider counted far more than four bytes a token suggests.
    agent
        .run_loop(|_: &Agent| Ok::<_, String>(reply(95_000)))
        .unwrap();
    assert!(agent.estimated_context_tokens() < 10_000);
    assert!(agent.calibrated_context_tokens() > 95_000);
    assert_eq!(agent.run_stats().compactions, 0);

    agent.prompt("Continue.");
    agent
        .run_loop(|_: &Agent| Ok::<_, String>(reply(1_000)))
        .unwrap();
    assert_eq!(agent.run_stats().compactions, 1);
    // The count belonged to the history compaction replaced.
    assert!(agent.calibrated_context_tokens() < 95_000);
}

#[test]
fn a_context_overflow_refusal_compacts_once_and_retries() {
    for always_refuse in [false, true] {
        let mut agent = agent(ContextVmMode::Hybrid);
        for turn in 0..6 {
            agent.prompt(&format!("step {turn}"));
            agent.record_assistant("ok");
        }
        agent.prompt("Continue.");
        let requests = Arc::new(Mutex::new(0));
        let counter = requests.clone();
        let result = agent.run_loop(move |_: &Agent| {
            let mut requests = counter.lock().unwrap();
            *requests += 1;
            if always_refuse || *requests == 1 {
                Err("context_length_exceeded: input exceeds the context window".to_string())
            } else {
                Ok(reply(100))
            }
        });
        assert_eq!(
            *requests.lock().unwrap(),
            2,
            "refuse always: {always_refuse}"
        );
        assert_eq!(agent.run_stats().compactions, 1);
        assert_eq!(result.is_err(), always_refuse);
    }
}

/// Every source the ledger cites, retrieved the way the model would.
fn assert_cited_sources_retrievable(agent: &Agent, compactions: u64) {
    let vm = &agent.runtime.as_ref().unwrap().context_vm;
    let state = vm.load_state_from_root().unwrap();
    let cited = cited_source_refs(&state);
    assert!(
        !cited.is_empty(),
        "compaction {compactions}: ledger cites nothing"
    );
    let mut texts = String::new();
    for source_ref in &cited {
        let result = vm
            .retrieve(&RetrieveContextRequest {
                source_ref: Some(source_ref.clone()),
                ..Default::default()
            })
            .unwrap_or_else(|error| panic!("compaction {compactions}: {source_ref}: {error}"));
        texts.push_str(&result.content);
    }
    assert!(texts.contains(NEVER), "compaction {compactions}");
    assert!(texts.contains(CORRECTION), "compaction {compactions}");
}

/// Run turns until `target` compactions, checking the ledger's sources after
/// each one. Returns the turns taken.
fn compact_repeatedly(agent: &mut Agent, target: u64) -> usize {
    agent.context_window = 12_000;
    agent.compaction = CompactionSettings {
        reserve_tokens: 2_000,
        keep_recent_tokens: 1_000,
        ..CompactionSettings::default()
    };
    let mut checked = 0;
    for turn in 0..400 {
        let text = match turn {
            0 => format!("Build the scheduler. {NEVER}"),
            7 => CORRECTION.to_string(),
            _ => format!("Continue with step {turn}. {}", "notes ".repeat(300)),
        };
        agent.prompt(&text);
        agent
            .run_loop(|agent: &Agent| Ok::<_, String>(reply(measured(agent))))
            .expect("turn runs");
        let text = provider_text(agent);
        assert!(text.contains(NEVER), "turn {turn} lost the constraint");
        if turn >= 7 {
            assert!(text.contains(CORRECTION), "turn {turn} lost the correction");
        }
        let compactions = agent.run_stats().compactions;
        if compactions > checked && turn >= 7 {
            assert_cited_sources_retrievable(agent, compactions);
            checked = compactions;
        }
        if checked >= target {
            return turn + 1;
        }
    }
    panic!("only {checked} compactions");
}

#[test]
fn cited_sources_stay_retrievable_through_five_compactions_without_a_session() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.summarizer = Some(lazy_summarizer(Arc::default()));
    compact_repeatedly(&mut agent, 5);
}

#[test]
fn cited_sources_stay_retrievable_through_five_compactions_with_a_session() {
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.session = Some(JsonlSession::create(sessions.path(), "hybrid", None).unwrap());
    agent.summarizer = Some(lazy_summarizer(Arc::default()));
    compact_repeatedly(&mut agent, 5);

    let path = agent.session.as_ref().unwrap().path.clone();
    // Release the writer lease, as closing the first process would.
    drop(agent);
    let mut reloaded = self::agent(ContextVmMode::Hybrid);
    reloaded
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    reloaded.set_context_vm_mode(ContextVmMode::Hybrid);
    reloaded.prompt("Continue.");
    reloaded
        .run_loop(|agent: &Agent| Ok::<_, String>(reply(measured(agent))))
        .unwrap();
    assert_cited_sources_retrievable(&reloaded, 5);
}

/// A transcript of `turns` exchanges of about `tokens` tokens each.
fn filled(agent: &mut Agent, turns: usize, tokens: usize) {
    agent.prompt(&format!("Build the scheduler. {NEVER}"));
    agent.record_assistant("ok");
    for turn in 0..turns {
        agent.prompt(&format!("step {turn} {}", "x".repeat(tokens * 4)));
        agent.record_assistant("ok");
    }
}

#[test]
fn compaction_commits_only_a_replacement_under_the_threshold() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.context_window = 12_000;
    agent.compaction = CompactionSettings {
        reserve_tokens: 2_000,
        // Keeping this much verbatim plus the ledger cannot fit 10k.
        keep_recent_tokens: 9_990,
        ..CompactionSettings::default()
    };
    let threshold = davinci_agent::compaction_threshold(
        12_000,
        &CompactionSettings {
            enabled: true,
            ..agent.compaction
        },
    )
    .unwrap();
    filled(&mut agent, 30, 400);
    assert!(agent.estimated_context_tokens() > threshold);
    let result = agent.compact(None);
    assert!(result.compacted, "{}", result.summary);
    assert!(
        agent.estimated_context_tokens() <= threshold,
        "{} > {threshold}",
        agent.estimated_context_tokens()
    );
    // The cut moved to a later turn boundary; it never strands a tool result.
    assert_ne!(agent.messages[1].role, "toolResult");
    assert!(provider_text(&agent).contains(NEVER));
}

#[test]
fn compaction_that_cannot_fit_leaves_the_transcript_alone() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.context_window = 12_000;
    agent.compaction = CompactionSettings {
        reserve_tokens: 2_000,
        keep_recent_tokens: 1_000,
        ..CompactionSettings::default()
    };
    filled(&mut agent, 4, 400);
    // The newest request alone is over the threshold: no cut can fit it.
    agent.prompt(&"y".repeat(11_000 * 4));
    let before = agent.messages.clone();
    let result = agent.compact(None);
    assert!(!result.compacted);
    assert!(
        result.summary.contains("does not fit"),
        "{}",
        result.summary
    );
    assert_eq!(agent.messages, before);
}

#[test]
fn a_provider_count_is_not_reused_across_a_model_change() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.prompt("Build the scheduler.");
    agent
        .run_loop(|_: &Agent| Ok::<_, String>(reply(95_000)))
        .unwrap();
    assert!(agent.calibrated_context_tokens() > 95_000);
    agent.model_id = "another-model".into();
    assert_eq!(
        agent.calibrated_context_tokens(),
        agent.estimated_context_tokens()
    );
}

#[test]
fn a_provider_count_follows_request_configuration_changes() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.prompt("Build the scheduler.");
    agent
        .run_loop(|_: &Agent| Ok::<_, String>(reply(50_000)))
        .unwrap();
    let measured = agent.calibrated_context_tokens();
    let estimated = agent.estimated_context_tokens();
    // A larger system prompt and tool catalog add their estimated growth to
    // the measured count.
    agent.set_provider_system_prompt_suffix(Some("s".repeat(40_000)));
    agent.set_provider_context_overhead_tokens(Some(20_000));
    let growth = agent.estimated_context_tokens() - estimated;
    assert!(growth >= 10_000, "{growth}");
    assert_eq!(agent.calibrated_context_tokens(), measured + growth);
    // Removing them again returns to the count the provider reported.
    agent.set_provider_system_prompt_suffix(None);
    agent.set_provider_context_overhead_tokens(None);
    assert_eq!(agent.calibrated_context_tokens(), measured);
}

#[test]
fn feature_use_reports_context_vm_off_as_mode_only() {
    let agent = agent(ContextVmMode::Off);
    assert_eq!(
        agent.feature_use()["contextVm"],
        serde_json::json!({"mode": "off"})
    );
}

#[test]
fn feature_use_reports_hybrid_folds_and_retrieval_after_compaction() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.summarizer = Some(lazy_summarizer(Arc::default()));
    let before = agent.feature_use()["contextVm"].clone();
    assert_eq!(before["mode"], "hybrid");
    assert_eq!(before["folds"], 0);
    assert_eq!(before["retrievalOffered"], false);

    long_session(&mut agent);
    let used = agent.feature_use()["contextVm"].clone();
    assert_eq!(used["mode"], "hybrid");
    assert!(agent.run_stats().compactions >= 2);
    assert!(used["folds"].as_u64().unwrap() > 0, "{used}");
    assert_eq!(used["retrievalOffered"], true, "{used}");
    assert!(used["imagesCompiled"].is_u64() && used["tokensBeforeFold"].is_u64());
    // Every compaction entry is counted once, and an applied one adds its
    // before/after message estimates (so `after` is smaller).
    let hybrid = &used["hybridCompaction"];
    for key in [
        "attempts",
        "applied",
        "skippedNoFit",
        "skippedNoAssistantReply",
        "skippedBackoff",
        "notAppliedAfterFold",
        "notAppliedOther",
        "contextTokensBefore",
        "contextTokensAfter",
    ] {
        assert!(hybrid[key].is_u64(), "{key}: {hybrid}");
    }
    assert_eq!(hybrid["applied"], agent.run_stats().compactions, "{hybrid}");
    assert!(hybrid["contextTokensAfter"].as_u64() < hybrid["contextTokensBefore"].as_u64());
    let accounted: u64 = [
        "applied",
        "skippedNoFit",
        "skippedNoAssistantReply",
        "skippedBackoff",
        "notAppliedAfterFold",
        "notAppliedOther",
    ]
    .iter()
    .map(|key| hybrid[key].as_u64().unwrap())
    .sum();
    assert_eq!(hybrid["attempts"], accounted, "{hybrid}");
}
