use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{
    events_from_messages, ContextEvent, ContextVmConfig, ContextVmRuntime, FoldReason,
    RetrieveContextRequest,
};
use davinci_agent::ContextPacket;
use davinci_ai::ChatMessage;

fn runtime(config: CacheConfig, dir: Option<std::path::PathBuf>) -> ContextVmRuntime {
    ContextVmRuntime::new(ContextVmConfig::default(), CacheRuntime::new(config, dir))
}

fn conversation() -> Vec<ContextEvent> {
    events_from_messages(&[
        ChatMessage::text("user", "constraint: keep the public API stable"),
        ChatMessage::text("assistant", "I will inspect the API"),
        ChatMessage::tool_result("call-1", "bash", "test suite passed", false),
    ])
}

fn retrieve_page(vm: &ContextVmRuntime, page: &str) -> Result<String, String> {
    vm.retrieve(&RetrieveContextRequest {
        page: Some(page.into()),
        ..RetrieveContextRequest::default()
    })
    .map(|result| result.content)
}

/// Every cache shape that silently drops writes: disabled, too small for a
/// page, and a persistent store whose directory cannot be written.
fn lossy_caches() -> Vec<(
    &'static str,
    CacheConfig,
    Option<std::path::PathBuf>,
    tempfile::TempDir,
)> {
    let blocked = tempfile::tempdir().unwrap();
    let not_a_directory = blocked.path().join("agent-file");
    std::fs::write(
        &not_a_directory,
        "a file where the cache directory should be",
    )
    .unwrap();
    vec![
        (
            "disabled",
            CacheConfig {
                enabled: false,
                ..CacheConfig::default()
            },
            None,
            tempfile::tempdir().unwrap(),
        ),
        (
            "size limit",
            CacheConfig {
                max_object_bytes: 16,
                ..CacheConfig::default()
            },
            None,
            tempfile::tempdir().unwrap(),
        ),
        (
            "disk failure",
            CacheConfig {
                memory_enabled: false,
                ..CacheConfig::default()
            },
            Some(not_a_directory),
            blocked,
        ),
    ]
}

#[test]
fn wor23_skipped_cache_writes_are_reported_and_never_leave_dangling_pages() {
    for (label, config, dir, _guard) in lossy_caches() {
        let vm = runtime(config, dir);
        let events = conversation();
        let image = vm
            .compile(&events, &ContextPacket::empty(), 20_000)
            .unwrap();
        let checkpoint = image.root.checkpoint.clone().unwrap();
        let content = retrieve_page(&vm, &checkpoint.id)
            .unwrap_or_else(|error| panic!("{label}: saved checkpoint not loadable: {error}"));
        assert!(content.contains("keep the public API stable"), "{label}");
        let metrics = vm.metrics();
        assert!(
            metrics.page_writes_not_durable >= 1,
            "{label}: a non-durable write was reported as success"
        );
        assert!(metrics.pinned_page_loads >= 1, "{label}");
    }
}

#[test]
fn wor23_durable_writes_are_not_reported_as_lost() {
    let dir = tempfile::tempdir().unwrap();
    let vm = runtime(CacheConfig::default(), Some(dir.path().to_path_buf()));
    vm.compile(&conversation(), &ContextPacket::empty(), 20_000)
        .unwrap();
    assert_eq!(vm.metrics().page_writes_not_durable, 0);
}

#[test]
fn wor36_restart_after_page_eviction_rebuilds_once_and_fails_closed_on_lost_pages() {
    let events = conversation();
    let first_dir = tempfile::tempdir().unwrap();
    let before = runtime(CacheConfig::default(), Some(first_dir.path().to_path_buf()));
    before
        .compile(&events, &ContextPacket::empty(), 20_000)
        .unwrap();
    let persisted = before.fold(FoldReason::Manual, &events).unwrap();
    let evicted_episode = persisted.episodes.first().unwrap().id.clone();
    drop(before);
    // Restart against a cache that lost every page (eviction, cleared cache dir).
    std::fs::remove_dir_all(first_dir.path()).unwrap();
    std::fs::create_dir_all(first_dir.path()).unwrap();
    let after = runtime(CacheConfig::default(), Some(first_dir.path().to_path_buf()));
    after.install_root(persisted, 3);

    for turn in 0..3 {
        let image = after
            .compile(&events, &ContextPacket::empty(), 20_000)
            .unwrap_or_else(|error| panic!("turn {turn} failed after restart: {error}"));
        assert!(image
            .entries
            .iter()
            .any(|entry| entry.content.contains("keep the public API stable")));
    }
    let metrics = after.metrics();
    assert_eq!(
        (metrics.rebuild_attempts, metrics.rebuild_successes),
        (1, 1),
        "the lost pages must be rebuilt once, not on every dispatch"
    );
    assert_eq!(
        retrieve_page(&after, &evicted_episode).unwrap_err(),
        "context page unavailable; replay/rebuild required"
    );
}

#[test]
fn wor36_disabled_cache_does_not_rebuild_on_every_dispatch() {
    let vm = runtime(
        CacheConfig {
            enabled: false,
            ..CacheConfig::default()
        },
        None,
    );
    let mut messages = vec![ChatMessage::text("user", "constraint: keep the API")];
    for turn in 0..4 {
        messages.push(ChatMessage::text("user", &format!("step {turn}")));
        let events = events_from_messages(&messages);
        vm.append_delta(&events).unwrap();
        vm.compile(&events, &ContextPacket::empty(), 20_000)
            .unwrap();
    }
    assert_eq!(vm.metrics().rebuild_attempts, 1);
}
