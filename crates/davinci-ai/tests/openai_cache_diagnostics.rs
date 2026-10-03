use davinci_ai::{
    sampled_comparison_response_id, AppliedPromptCachePolicy, Model, ModelCost,
    NativeResponsesOutput, NativeResponsesResumeRecord, NativeResponsesTurn,
    OpenAiCacheCapabilities, ProviderPromptCacheDiagnostics, StreamOptions, WireManifest,
};
use serde_json::json;

fn public_model(base_url: &str, explicit: bool) -> Model {
    Model {
        id: if explicit { "gpt-5.6-sol" } else { "gpt-5" }.into(),
        name: "diagnostics".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: Some(base_url.into()),
        reasoning: true,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.2,
        },
        context_window: 272_000,
        max_tokens: 128_000,
        compat: if explicit {
            json!({"supportsExplicitPromptCacheMode": true})
        } else {
            json!({})
        },
        headers: Default::default(),
        thinking_level_map: Default::default(),
    }
}

fn resume_record(response_id: &str) -> NativeResponsesResumeRecord {
    NativeResponsesResumeRecord {
        turn: NativeResponsesTurn {
            request_input_items: vec![],
            output: NativeResponsesOutput {
                response_id: Some(response_id.into()),
                output_items: vec![],
                final_response: None,
                terminal_event_type: "response.completed".into(),
            },
            wire_manifest: WireManifest {
                schema_version: 1,
                ordered_prefix_fingerprint: "prefix".into(),
                request_bytes_before_compression: 0,
                segments: vec![],
            },
        },
        resume_provider_message_count: 0,
        resume_provider_messages_fingerprint: davinci_ai::provider_messages_fingerprint(&[]),
    }
}

#[test]
fn comparison_diagnostics_are_capability_scoped_and_opt_in() {
    let supported = public_model("https://api.openai.com/v1", true);
    let unsupported = public_model("https://api.openai.com.evil.test/v1", true);
    let options = StreamOptions {
        native_responses_resume: Some(resume_record("resp_base")),
        ..StreamOptions::default()
    };

    assert_eq!(
        sampled_comparison_response_id(&supported, &options, 1).as_deref(),
        Some("resp_base")
    );
    assert_eq!(
        sampled_comparison_response_id(&unsupported, &options, 1),
        None
    );
    assert!(
        OpenAiCacheCapabilities::resolve(&supported, supported.base_url.as_deref(), false)
            .supports_cache_diagnostics
    );
}

#[test]
fn explicit_cache_disable_never_requests_comparison_diagnostics() {
    let model = public_model("https://api.openai.com/v1", true);
    let options = StreamOptions {
        cache_retention: Some("none".into()),
        native_responses_resume: Some(resume_record("resp_base")),
        ..StreamOptions::default()
    };
    assert_eq!(sampled_comparison_response_id(&model, &options, 1), None);
}

#[test]
fn terminal_diagnostics_are_bounded_and_do_not_retain_prompt_or_opaque_items() {
    let response = json!({
        "id":"resp_2",
        "prompt_cache_options":{
            "mode":"implicit",
            "ttl":"30m",
            "comparison_response_id":"resp_1"
        },
        "prompt_cache_diagnostics":{
            "type":"cache_miss",
            "reason":"tools_changed",
            "comparison_reusable_tokens":5629,
            "cache_missed_tokens":5629,
            "provider_future_field":"ignore-me"
        },
        "output":[{
            "type":"reasoning",
            "encrypted_content":"OPAQUE-SECRET"
        }],
        "debug_prompt":"RAW-PROMPT-MUST-NOT-LEAK"
    });
    let output = NativeResponsesOutput::from_response_value(&response).unwrap();
    let diag = ProviderPromptCacheDiagnostics::from_native_output(&output).unwrap();
    let policy = AppliedPromptCachePolicy::from_native_output(&output).unwrap();

    assert_eq!(diag.reported_hit(), Some(false));
    assert_eq!(diag.reason.as_deref(), Some("tools_changed"));
    assert_eq!(diag.comparison_reusable_tokens, Some(5629));
    assert_eq!(policy.mode.as_deref(), Some("implicit"));
    assert_eq!(policy.ttl.as_deref(), Some("30m"));
    assert!(policy.comparison_requested);

    let diagnostic_json = serde_json::to_string(&(diag, policy)).unwrap();
    assert!(!diagnostic_json.contains("RAW-PROMPT-MUST-NOT-LEAK"));
    assert!(!diagnostic_json.contains("OPAQUE-SECRET"));
    assert!(!diagnostic_json.contains("resp_1"));
}

#[test]
fn duplicate_terminal_events_use_the_last_provider_diagnostic_once() {
    let events = vec![
        json!({
            "type":"response.completed",
            "response":{
                "id":"resp_old",
                "prompt_cache_diagnostics":{"type":"unavailable"}
            }
        }),
        json!({
            "type":"response.completed",
            "response":{
                "id":"resp_new",
                "prompt_cache_diagnostics":{
                    "type":"cache_hit",
                    "comparison_reusable_tokens":2048
                }
            }
        }),
    ];
    let output = NativeResponsesOutput::from_events(&events).unwrap();
    let diag = ProviderPromptCacheDiagnostics::from_native_output(&output).unwrap();
    assert_eq!(output.response_id.as_deref(), Some("resp_new"));
    assert_eq!(diag.reported_hit(), Some(true));
    assert_eq!(diag.comparison_reusable_tokens, Some(2048));
}
