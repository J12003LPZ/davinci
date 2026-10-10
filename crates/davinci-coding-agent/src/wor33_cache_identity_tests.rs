//! WOR-33: Auto/Plan switches must not churn the live provider cache key.

use super::*;
use davinci_agent::PermissionMode;
use davinci_protocol::ThinkingLevel;
use serde_json::Value;

fn codex_model() -> davinci_ai::Model {
    davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
        .expect("catalog has openai-codex/gpt-5.6-luna")
}

fn auth() -> ResolvedAuth {
    ResolvedAuth {
        api_key: Some("test".into()),
        headers: Default::default(),
        source: "oauth".into(),
    }
}

fn key_for(agent: &Agent) -> String {
    live_cache_key(
        &codex_model(),
        &auth(),
        &agent.provider_system_prompt(),
        agent,
        &provider_tools(agent),
    )
}

fn appended_agent() -> Agent {
    let mut agent = Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-5.6-luna".into();
    agent.turn_context_placement_override =
        Some(davinci_agent::turn_context::TurnContextPlacement::Appended);
    agent
}

#[test]
fn auto_plan_auto_keeps_the_live_cache_key() {
    let mut agent = appended_agent();
    agent.set_permission_mode(PermissionMode::Auto);
    let auto = key_for(&agent);

    agent.set_plan_mode(true);
    assert!(agent.is_plan_mode());
    let plan = key_for(&agent);

    agent.set_plan_mode(false);
    agent.set_permission_mode(PermissionMode::Auto);
    let back = key_for(&agent);

    assert_eq!(auto, plan, "entering Plan Mode changed the cache key");
    assert_eq!(auto, back, "leaving Plan Mode changed the cache key");
}

#[test]
fn every_permission_mode_projects_the_same_prefix_inputs() {
    let mut agent = appended_agent();
    agent.set_permission_mode(PermissionMode::Ask);
    let tools = serde_json::to_string(&provider_tools(&agent)).unwrap();
    let system = agent.provider_system_prompt();
    let visible = agent.visible_tool_names();
    let key = key_for(&agent);

    for mode in [
        PermissionMode::Edits,
        PermissionMode::Auto,
        PermissionMode::ReadOnly,
        PermissionMode::Ask,
    ] {
        agent.set_permission_mode(mode);
        assert_eq!(
            serde_json::to_string(&provider_tools(&agent)).unwrap(),
            tools,
            "tool schema differs in {mode:?}"
        );
        assert_eq!(
            agent.provider_system_prompt(),
            system,
            "system differs in {mode:?}"
        );
        assert_eq!(
            agent.visible_tool_names(),
            visible,
            "visible tools differ in {mode:?}"
        );
        assert_eq!(key_for(&agent), key, "cache key differs in {mode:?}");
    }
}

/// Exercise the real turn loop and provider request projection, retaining all
/// earlier input items as modes change. The caller supplies only the transport.
fn mode_switch_sequence(
    model: &davinci_ai::Model,
    auth: &ResolvedAuth,
    thinking: ThinkingLevel,
    report_path: Option<&std::path::Path>,
    mut complete: impl FnMut(
        &Agent,
        &str,
        &[ToolSpec],
        &StreamOptions,
    ) -> Result<CompleteOutput, String>,
) -> Vec<Value> {
    let root = tempfile::tempdir().unwrap();
    let mut agent = Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
    agent.cwd = root.path().to_path_buf();
    agent.provider = model.provider.clone();
    agent.model_id = model.id.clone();
    agent.thinking_level = thinking;
    agent.retry_attempts = 0;
    assert_eq!(agent.turn_context_placement(), davinci_agent::turn_context::TurnContextPlacement::Appended,
        "this acceptance test requires the default cache-sensitive route; remove a system-placement override");
    agent.freeze_tools_for_cache();
    let session_id = uuid::Uuid::new_v4().to_string();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
    let mut rows = Vec::new();
    let mut previous_input = Vec::new();
    let mut stable_contract = None;
    let mut first_key = None;

    for mode in ["auto_warm", "auto", "plan", "auto_return"] {
        if mode == "plan" {
            agent.set_plan_mode(true);
        } else {
            agent.set_plan_mode(false);
            agent.set_permission_mode(PermissionMode::Auto);
        }
        assert_eq!(agent.is_plan_mode(), mode == "plan");
        agent.messages.push(davinci_ai::ChatMessage::text("user",
            "This is a prompt-cache diagnostic turn. Reply with only OK. Do not invoke tools or change files."));
        // The application commits mode changes after the user message.
        agent.commit_turn_context(None);
        let mut calls = 0;
        agent.run_loop(|current| {
            calls += 1;
            if calls != 1 || std::time::Instant::now() >= deadline {
                return Err("mode-switch probe exceeded its request/time budget".into());
            }
            let system = current.provider_system_prompt();
            let tools = provider_tools(current);
            let messages = current.messages_for_provider();
            let key = live_cache_key(model, auth, &system, current, &tools);
            let options = StreamOptions {
                thinking_level: Some(thinking),
                service_tier: Some(current.service_tier),
                responses_is_oauth: Some(true),
                transport: Some("sse".into()),
                session_id: Some(session_id.clone()), cache_key: Some(key.clone()),
                timeout_ms: Some(60_000), max_retries: Some(0),
                native_responses_resume: current.native_responses_resume_record_for(&messages),
                ..Default::default()
            };
            let body = davinci_ai::request_body_with(model, &messages, Some(&system), &tools, &options);
            assert_eq!(body["prompt_cache_key"], key, "{mode}: wire cache key differs from live identity");
            let input = body["input"].as_array().expect("Responses input").clone();
            assert!(input.starts_with(&previous_input), "{mode}: earlier provider input changed");
            let previous_items = previous_input.len();
            let mut contract = body.clone();
            contract.as_object_mut().unwrap().remove("input");
            if let Some(first) = &stable_contract { assert_eq!(&contract, first, "{mode}: stable prefix changed"); }
            else { stable_contract = Some(contract.clone()); }
            if let Some(first) = &first_key { assert_eq!(&key, first, "{mode}: emitted cache key changed"); }
            else { first_key = Some(key.clone()); }
            previous_input = input.clone();
            let started = std::time::Instant::now();
            let observation = davinci_ai::provider_observation::ObservationScope::capture();
            davinci_ai::provider_observation::begin_request("cache_mode_switch_probe", &model.id,
                Some(&format!("{thinking:?}")), &effective_provider_tool_schema_hash(model, auth, &tools));
            let result = complete(current, &system, &tools, &options);
            let attempts = observation.finish(if result.is_ok() { "completed" } else { "failed" });
            let completed_attempts: Vec<_> = attempts.iter().filter(|event| event.kind == "attempt_end").collect();
            let raw_usage = completed_attempts.last().and_then(|event| event.raw_usage.as_ref());
            let usage = result.as_ref().ok().and_then(|out| out.message.usage.as_ref());
            let row = serde_json::json!({
                "mode":mode,"model":model.id,"thinking":format!("{thinking:?}"),
                "cacheKey":body["prompt_cache_key"],"stableContractHash":davinci_agent::runtime::compute_schema_hash(&contract),
                "inputHash":davinci_agent::runtime::compute_schema_hash(&serde_json::json!(input)),
                "previousInputItems":previous_items,"inputItems":input.len(),"previousInputPreserved":true,
                "nativeReplayAvailable":options.native_responses_resume.is_some(),
                "freshInputTokens":usage.map(|u|u.input),"cacheReadTokens":raw_usage.and_then(|u|u.cache_read),
                "cacheWriteTokens":raw_usage.and_then(|u|u.cache_write),
                "normalizedCacheReadTokens":usage.map(|u|u.cache_read),
                "normalizedCacheWriteTokens":usage.and_then(|u|(!u.cache_write_unreported).then_some(u.cache_write)),
                "providerAttempts":completed_attempts.len(),
                "outputTokens":usage.map(|u|u.output),"latencyMs":started.elapsed().as_millis(),
                "failure":result.as_ref().err().cloned().or_else(|| result.as_ref().ok()
                    .and_then(|out| out.message.error_message.clone())),
                "usageWindows":davinci_ai::codex_usage::latest()
            });
            rows.push(row);
            if let Some(path) = report_path {
                std::fs::write(path, serde_json::to_string_pretty(&rows).unwrap()).unwrap();
            }
            let output = result?;
            assert_eq!(completed_attempts.len(), 1, "unexpected provider attempt count");
            if output.message.stop_reason == Some(StopReason::Error)
                || output.message.content.iter().any(|c| matches!(c, ContentBlock::ToolCall { .. })) {
                return Err("probe provider returned an error or an unexpected tool call".into());
            }
            Ok(output)
        }).unwrap();
    }
    rows
}

#[test]
fn auto_plan_auto_preserves_emitted_prefix_and_turn_usage() {
    let model = codex_model();
    let corpus = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"wor33\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"OK\"}]}],\"usage\":{\"input_tokens\":1300,\"output_tokens\":2,\"input_tokens_details\":{\"cached_tokens\":1280}}}}\n\n";
    let rows = mode_switch_sequence(
        &model,
        &auth(),
        ThinkingLevel::Medium,
        None,
        |agent, _, _, _| {
            let attempt = davinci_ai::provider_observation::Attempt::start("fixture");
            let message =
                davinci_ai::fixture_complete(&model, &agent.messages_for_provider(), corpus);
            attempt.finish("completed", Some(200), message.usage.clone());
            Ok(CompleteOutput::from(message))
        },
    );
    assert_eq!(rows.len(), 4);
    // This validates decoding/projection only; these fixture counts are not live savings.
    assert!(rows
        .iter()
        .all(|r| r["cacheReadTokens"] == 1280 && r["cacheWriteTokens"].is_null()));
}

#[test]
#[ignore = "explicit subscription probe: four requests, no retries, 60s idle timeout, 240s admission deadline"]
fn live_auto_plan_auto_provider_cache_reads() {
    let model_id = std::env::var("WOR33_MODEL").expect("set WOR33_MODEL explicitly");
    let thinking = crate::args::is_valid_thinking_level(
        &std::env::var("WOR33_THINKING").expect("set WOR33_THINKING explicitly"),
    )
    .expect("valid effort");
    let report_path =
        std::env::var("WOR33_REPORT").expect("set a private WOR33_REPORT path outside Git");
    let parsed = Args {
        model: Some(model_id.clone()),
        provider: Some("openai-codex".into()),
        ..Default::default()
    };
    let (model, auth) = resolve_model_and_auth(&parsed, "openai-codex", &model_id).unwrap();
    assert_eq!(model.api, "openai-codex-responses");
    assert!(
        auth.source.eq_ignore_ascii_case("oauth"),
        "subscription OAuth required"
    );
    let rows = mode_switch_sequence(
        &model,
        &auth,
        thinking,
        Some(std::path::Path::new(&report_path)),
        |agent, system, tools, options| {
            let messages = agent.messages_for_provider();
            let envelope = live_complete_streaming_with_sink_envelope(
                &model,
                &messages,
                &auth,
                Some(system),
                tools,
                options,
                &mut |_| {},
            )?;
            let native_responses_resume = envelope.native_responses.map(|turn| {
                let mut projection = messages.clone();
                projection.push(davinci_ai::assistant_to_chat(&envelope.message));
                davinci_ai::NativeResponsesResumeRecord {
                    turn,
                    resume_provider_message_count: projection.len(),
                    resume_provider_messages_fingerprint: davinci_ai::provider_messages_fingerprint(
                        &projection,
                    ),
                }
            });
            Ok(CompleteOutput {
                message: envelope.message,
                stream_events: Some(envelope.stream_events),
                native_responses_resume,
                streamed_live: false,
            })
        },
    );
    std::fs::write(report_path, serde_json::to_string_pretty(&rows).unwrap()).unwrap();
    for row in &rows {
        println!("{row}");
    }
    assert!(
        rows[1..]
            .iter()
            .all(|row| row["cacheReadTokens"].as_u64().is_some_and(|n| n > 0)),
        "provider did not report warm cache reads across every transition"
    );
}
