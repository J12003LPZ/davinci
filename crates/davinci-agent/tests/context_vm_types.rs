use davinci_agent::runtime::context_vm::{
    ContextFoldPolicy, ContextPageKind, ContextPageRef, ContextRoot, ContextVmConfig,
    ContextVmMode, FoldReason,
};

#[test]
fn context_vm_defaults_are_safe_and_disabled() {
    let config = ContextVmConfig::default();

    assert_eq!(config.mode, ContextVmMode::Off);
    assert_eq!(config.hot_event_tokens, 20_000);
    assert_eq!(config.max_delta_pages, 8);
    assert_eq!(config.max_delta_tokens, 8_000);
    assert_eq!(config.window_pressure_percent, 80);
}

#[test]
fn context_vm_env_parsing_is_exact_and_fail_closed() {
    assert_eq!(ContextVmMode::from_env_value(None), ContextVmMode::Off);
    assert_eq!(
        ContextVmMode::from_env_value(Some("off")),
        ContextVmMode::Off
    );
    assert_eq!(
        ContextVmMode::from_env_value(Some("garbage")),
        ContextVmMode::Off
    );
    assert_eq!(
        ContextVmMode::from_env_value(Some("shadow")),
        ContextVmMode::Shadow
    );
    assert_eq!(
        ContextVmMode::from_env_value(Some("active")),
        ContextVmMode::Active
    );
}

#[test]
fn context_root_roundtrips_without_losing_page_identity() {
    let root = ContextRoot {
        epoch: 4,
        cache_namespace: "namespace-v2".into(),
        checkpoint: Some(ContextPageRef {
            id: "ctx:checkpoint:abc".into(),
            kind: ContextPageKind::Checkpoint,
            content_hash: "abc".into(),
            estimated_tokens: 120,
        }),
        ..ContextRoot::default()
    };

    let bytes = serde_json::to_vec(&root).unwrap();
    let decoded: ContextRoot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, root);
}

#[test]
fn fold_policy_uses_explicit_trigger_precedence() {
    let policy = ContextFoldPolicy {
        max_delta_pages: 2,
        max_delta_tokens: 50,
        window_pressure_percent: 80,
    };
    let root = ContextRoot {
        deltas: vec![
            ContextPageRef {
                id: "ctx:delta:one".into(),
                kind: ContextPageKind::Delta,
                content_hash: "one".into(),
                estimated_tokens: 1,
            },
            ContextPageRef {
                id: "ctx:delta:two".into(),
                kind: ContextPageKind::Delta,
                content_hash: "two".into(),
                estimated_tokens: 1,
            },
        ],
        ..ContextRoot::default()
    };

    assert_eq!(
        policy.decide(&root, 100, 1, 100, true, true).reason,
        Some(FoldReason::Manual)
    );
    assert_eq!(
        policy.decide(&root, 100, 1, 100, false, true).reason,
        Some(FoldReason::PhaseBoundary)
    );
    assert_eq!(
        policy.decide(&root, 100, 80, 100, false, false).reason,
        Some(FoldReason::WindowPressure)
    );
    assert_eq!(
        policy.decide(&root, 1, 1, 100, false, false).reason,
        Some(FoldReason::DeltaDepth)
    );

    let empty_root = ContextRoot::default();
    assert!(
        !policy
            .decide(&empty_root, 1, 1, 100, false, false)
            .should_fold
    );
    assert_eq!(
        policy.decide(&empty_root, 50, 1, 100, false, false).reason,
        Some(FoldReason::DeltaTokens)
    );
}
