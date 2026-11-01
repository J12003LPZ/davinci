use davinci_ai::{
    load_builtin_models, request_body_with, ChatMessage, PreparedProviderRequest, StreamOptions,
};

fn sol() -> davinci_ai::Model {
    load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai" && model.id == "gpt-5.6-sol")
        .expect("gpt-5.6-sol")
}

#[test]
fn real_responses_builder_manifest_finds_first_changed_tail_segment() {
    let model = sol();
    let options = StreamOptions {
        cache_key: Some("partition".into()),
        cache_retention: Some("short".into()),
        ..StreamOptions::default()
    };
    let first = request_body_with(
        &model,
        &[ChatMessage::text("user", "tail one")],
        Some("stable bootstrap"),
        &[],
        &options,
    );
    let second = request_body_with(
        &model,
        &[ChatMessage::text("user", "tail two")],
        Some("stable bootstrap"),
        &[],
        &options,
    );
    let first = PreparedProviderRequest::new(first);
    let second = PreparedProviderRequest::new(second);

    assert!(first.first_changed_cache_segment(&second).is_some());
    assert_ne!(
        first.manifest().ordered_prefix_fingerprint,
        second.manifest().ordered_prefix_fingerprint
    );
    assert!(first
        .manifest()
        .segments
        .iter()
        .any(|segment| segment.cache_sensitive));
}

#[test]
fn cache_partition_change_is_routing_not_prefix_content() {
    let model = sol();
    let a = request_body_with(
        &model,
        &[ChatMessage::text("user", "same")],
        Some("stable bootstrap"),
        &[],
        &StreamOptions {
            cache_key: Some("a".into()),
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    let b = request_body_with(
        &model,
        &[ChatMessage::text("user", "same")],
        Some("stable bootstrap"),
        &[],
        &StreamOptions {
            cache_key: Some("b".into()),
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    let a = PreparedProviderRequest::new(a);
    let b = PreparedProviderRequest::new(b);
    assert_eq!(a.first_changed_cache_segment(&b), None);
    assert_eq!(
        a.manifest().ordered_prefix_fingerprint,
        b.manifest().ordered_prefix_fingerprint
    );
}

#[test]
fn prepared_manifest_is_deterministic_content_free_and_noninterfering() {
    let secret = "SYNTHETIC_PRIVATE_BEARER_COOKIE_PATH_REASONING";
    let model = sol();
    let body = request_body_with(
        &model,
        &[ChatMessage::text("user", secret)],
        Some(secret),
        &[davinci_ai::ToolSpec {
            name: "fixture_read".into(),
            description: secret.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": secret}}
            }),
            constrained_sampling: None,
        }],
        &StreamOptions {
            cache_key: Some(secret.into()),
            thinking_level: Some(davinci_protocol::ThinkingLevel::High),
            ..StreamOptions::default()
        },
    );
    let original_bytes = serde_json::to_vec(&body).unwrap();
    let prepared = PreparedProviderRequest::new(body.clone());
    let repeated = PreparedProviderRequest::new(body.clone());
    assert_eq!(prepared.body(), &body);
    assert_eq!(serde_json::to_vec(prepared.body()).unwrap(), original_bytes);
    assert_eq!(prepared.manifest(), repeated.manifest());
    let metadata = serde_json::to_string(prepared.manifest()).unwrap();
    assert!(!metadata.contains(secret));
    assert!(!metadata.contains("fixture_read"));
    assert!(prepared.manifest().request_bytes_before_compression > 0);

    let absent = PreparedProviderRequest::new(serde_json::json!({}));
    assert_eq!(absent.body(), &serde_json::json!({}));
    assert!(absent.manifest().segments.is_empty());
}

#[test]
fn script_mode_prefix_is_stable() {
    let model = sol();
    let options = StreamOptions {
        cache_key: Some("partition".into()),
        cache_retention: Some("short".into()),
        ..StreamOptions::default()
    };
    let tools = vec![
        davinci_ai::ToolSpec {
            name: "read".into(),
            description: "read".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
            constrained_sampling: None,
        },
        davinci_ai::ToolSpec {
            name: "codemode".into(),
            description: "script".into(),
            parameters: serde_json::json!({"type":"object","properties":{"code":{"type":"string"}}}),
            constrained_sampling: None,
        },
    ];
    let build = |text: &str, specs: &[davinci_ai::ToolSpec]| {
        PreparedProviderRequest::new(request_body_with(
            &model,
            &[ChatMessage::text("user", text)],
            Some("stable bootstrap"),
            specs,
            &options,
        ))
    };
    let a = build("first turn", &tools);
    let replay = build("first turn", &tools);
    // The same request prefixes byte-identically, including a retry.
    assert_eq!(a.cache_prefix_manifest(), replay.cache_prefix_manifest());
    assert_eq!(
        a.manifest().ordered_prefix_fingerprint,
        replay.manifest().ordered_prefix_fingerprint
    );

    // A mode flip or a new tool binding must not rewrite earlier segments:
    // only the trailing segment may differ.
    let tail_changed = build("second turn", &tools);
    let before = a.cache_prefix_manifest();
    let after = tail_changed.cache_prefix_manifest();
    assert!(before.len() >= 2);
    for (early_before, early_after) in before.iter().zip(after.iter()).take(before.len() - 1) {
        assert_eq!(
            (early_before.category, &early_before.digest),
            (early_after.category, &early_after.digest)
        );
    }

    // Tool catalog order is normalized, so a caller that reorders the same set
    // keeps the identical cached prefix.
    let mut reordered = tools.clone();
    reordered.reverse();
    let changed = build("first turn", &reordered);
    assert_eq!(a.cache_prefix_manifest(), changed.cache_prefix_manifest());

    // A genuine catalog change does alter the tool segment, and only it.
    let mut with_extra = tools.clone();
    with_extra.push(davinci_ai::ToolSpec {
        name: "grep".into(),
        description: "grep".into(),
        parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"}}}),
        constrained_sampling: None,
    });
    let extra = build("first turn", &with_extra);
    let extra_manifest = extra.cache_prefix_manifest();
    let tool = |manifest: &[davinci_ai::WireSegmentDigest]| {
        manifest
            .iter()
            .find(|segment| {
                matches!(
                    segment.category,
                    davinci_ai::WireSegmentCategory::ToolContract
                )
            })
            .map(|segment| segment.digest.clone())
            .unwrap()
    };
    assert_ne!(tool(&before), tool(&extra_manifest));
    assert_eq!(before[0].digest, extra_manifest[0].digest);
}

#[test]
fn unsupported_hosted_tools_fail_closed() {
    use davinci_ai::codex_capabilities::{CodexCapabilities, HostedToolKind, ToolSearchMode};

    // Public documentation is not evidence: every hosted shape is off.
    let unverified = CodexCapabilities::default();
    for kind in [
        HostedToolKind::ToolSearch,
        HostedToolKind::AdditionalTools,
        HostedToolKind::ProgrammaticToolCalling,
        HostedToolKind::CustomScript,
    ] {
        assert!(!unverified.admits_hosted_tool(kind), "{kind:?}");
    }

    // The authenticated Codex profile stays on the function fallback: tool
    // search is emulated, and freeform/programmatic shapes are not claimed.
    let codex = CodexCapabilities::for_chatgpt_codex(&sol());
    assert!(!codex.admits_hosted_tool(HostedToolKind::ToolSearch));
    assert!(!codex.admits_hosted_tool(HostedToolKind::AdditionalTools));
    assert!(!codex.admits_hosted_tool(HostedToolKind::ProgrammaticToolCalling));
    assert!(!codex.admits_hosted_tool(HostedToolKind::CustomScript));

    // A recorded observation is what flips a shape on.
    let observed = CodexCapabilities {
        tool_search: ToolSearchMode::Native,
        additional_tools: true,
        programmatic_tool_calling: true,
        custom_script: true,
        ..CodexCapabilities::for_chatgpt_codex(&sol())
    };
    for kind in [
        HostedToolKind::ToolSearch,
        HostedToolKind::AdditionalTools,
        HostedToolKind::ProgrammaticToolCalling,
        HostedToolKind::CustomScript,
    ] {
        assert!(observed.admits_hosted_tool(kind), "{kind:?}");
    }
    // The capability snapshot round-trips with the new fields intact.
    let json = serde_json::to_string(&observed).unwrap();
    let restored: CodexCapabilities = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, observed);
}
