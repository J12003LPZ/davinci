use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{
    events_from_messages, ContextVmConfig, ContextVmRuntime, RetrieveContextRequest,
};
use davinci_agent::{ContextItem, ContextPacket};
use davinci_ai::ChatMessage;

#[test]
fn retrieve_context_supports_exact_sources_bounded_lines_and_page_fault_metrics() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf())),
    );
    let packet = ContextPacket {
        items: vec![ContextItem {
            source: "repo:README.md".into(),
            content: "first line\nverified line\nthird line".into(),
            estimated_tokens: 8,
            priority: 100,
            stable_for_cache: true,
            provenance: serde_json::json!({"provenance_kind":"repository_fact"}),
        }],
        estimated_tokens: 8,
        cache_key: "fixture".into(),
    };
    let image = runtime
        .compile(
            &events_from_messages(&[ChatMessage::text("user", "read the README")]),
            &packet,
            1_000,
        )
        .unwrap();

    let source = runtime
        .retrieve(&RetrieveContextRequest {
            source_ref: Some("repo:README.md".into()),
            query: Some("line".into()),
            offset: 0,
            limit: 1,
            ..RetrieveContextRequest::default()
        })
        .unwrap();
    assert_eq!(source.content, "first line");
    assert!(source.truncated);
    assert_eq!(source.next_offset, Some(1));

    let checkpoint = image.root.checkpoint.as_ref().unwrap();
    let page = runtime
        .retrieve(&RetrieveContextRequest {
            page: Some(format!("ctx://page/{}", checkpoint.id)),
            ..RetrieveContextRequest::default()
        })
        .unwrap();
    assert!(page.source.starts_with("ctx://page/ctx:checkpoint:"));
    assert!(!page.content.contains("private reasoning"));

    let missing = runtime.retrieve(&RetrieveContextRequest {
        page: Some(format!("ctx:checkpoint:{}", "0".repeat(64))),
        ..RetrieveContextRequest::default()
    });
    assert_eq!(
        missing.unwrap_err(),
        "context page unavailable; replay/rebuild required"
    );
    let metrics = runtime.metrics();
    assert_eq!(metrics.page_faults, 2);
    assert_eq!(metrics.page_fault_hits, 1);
    assert_eq!(metrics.page_fault_misses, 1);
}
