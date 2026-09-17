use davinci_agent::runtime::cache::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier,
};
use std::time::Duration;

fn request(name: &str, deps: Vec<CacheDependency>) -> CacheRequest {
    CacheRequest::new(
        CacheKey::new(CacheNamespace::Ast, name, 1, "parser-1", deps),
        CachePolicy::MemoryOnly,
    )
}

#[test]
fn canonical_keys_isolate_versions_namespaces_and_workspaces() {
    let a = CacheDependency::ContentHash("a".into());
    let b = CacheDependency::ConfigHash("b".into());
    assert_eq!(
        request("x", vec![a.clone(), b.clone()]).key,
        request("x", vec![b, a]).key
    );
    let key = request("x", vec![]).key;
    assert_ne!(
        key,
        CacheKey::new(CacheNamespace::Repo, "x", 1, "parser-1", vec![])
    );
    assert_ne!(
        key,
        CacheKey::new(CacheNamespace::Ast, "x", 2, "parser-1", vec![])
    );
    assert_ne!(
        key,
        CacheKey::new(CacheNamespace::Ast, "x", 1, "parser-2", vec![])
    );
}

#[test]
fn memory_lru_and_authority_are_enforced_on_every_hit() {
    let cache = CacheRuntime::new(
        CacheConfig {
            max_entries: 2,
            ..Default::default()
        },
        None,
    );
    let a = request("a", vec![]);
    let b = request("b", vec![]);
    let c = request("c", vec![]);
    for req in [&a, &b] {
        cache
            .get_or_compute(req, || Ok(()), None, || Ok(42_u64))
            .unwrap();
    }
    assert_eq!(*cache.get::<u64>(&a, || Ok(())).unwrap().unwrap(), 42);
    cache
        .get_or_compute(&c, || Ok(()), None, || Ok(43_u64))
        .unwrap();
    assert!(cache.get::<u64>(&b, || Ok(())).unwrap().is_none());
    assert!(cache.get::<u64>(&a, || Err(CacheError::Denied)).is_err());
    assert_eq!(cache.stats().namespaces[&CacheNamespace::Ast].evictions, 1);
}

#[test]
fn invalidation_preserves_unrelated_content() {
    let cache = CacheRuntime::default();
    let dep = CacheDependency::ContentHash("old".into());
    let changed = request("file-a", vec![dep.clone()]);
    let other = request("file-b", vec![CacheDependency::ContentHash("other".into())]);
    for req in [&changed, &other] {
        cache
            .get_or_compute(req, || Ok(()), None, || Ok(1_u64))
            .unwrap();
    }
    cache.invalidate(&dep, LocalMissReason::ContentChanged);
    assert!(cache.get::<u64>(&changed, || Ok(())).unwrap().is_none());
    assert!(cache.get::<u64>(&other, || Ok(())).unwrap().is_some());
}

#[test]
fn ten_callers_compute_once_and_errors_can_retry() {
    let cache = Arc::new(CacheRuntime::default());
    let barrier = Arc::new(Barrier::new(10));
    let calls = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..10)
        .map(|_| {
            let (cache, barrier, calls) = (cache.clone(), barrier.clone(), calls.clone());
            std::thread::spawn(move || {
                barrier.wait();
                cache
                    .get_or_compute(
                        &request("parallel", vec![]),
                        || Ok(()),
                        None,
                        || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(80));
                            Ok(7_u64)
                        },
                    )
                    .unwrap()
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(*thread.join().unwrap(), 7);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let req = request("error", vec![]);
    assert!(cache
        .get_or_compute::<u64>(
            &req,
            || Ok(()),
            None,
            || Err(CacheError::Compute("fixture".into()))
        )
        .is_err());
    assert_eq!(
        *cache
            .get_or_compute(&req, || Ok(()), None, || Ok(9_u64))
            .unwrap(),
        9
    );
}

#[test]
fn cancelled_and_panicking_computations_do_not_poison_keys() {
    let cache = CacheRuntime::default();
    let req = request("cancel", vec![]);
    let cancel = davinci_agent::CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        cache.get_or_compute(&req, || Ok(()), Some(&cancel), || Ok(1_u64)),
        Err(CacheError::Cancelled)
    ));
    assert!(matches!(
        cache.get_or_compute::<u64>(&req, || Ok(()), None, || panic!("fixture")),
        Err(CacheError::Panicked)
    ));
    assert_eq!(
        *cache
            .get_or_compute(&req, || Ok(()), None, || Ok(2_u64))
            .unwrap(),
        2
    );
}

#[test]
fn persistent_restart_corruption_and_budget_fail_open() {
    let dir = tempfile::tempdir().unwrap();
    let req = CacheRequest::new(
        request("persistent", vec![]).key,
        CachePolicy::PersistentImmutable,
    );
    let config = CacheConfig {
        memory_enabled: false,
        persistent_max_bytes: 4096,
        ..Default::default()
    };
    let cache = CacheRuntime::new(config.clone(), Some(dir.path().into()));
    assert!(
        !dir.path().join("cache-runtime").exists(),
        "construction is lazy"
    );
    cache
        .get_or_compute(&req, || Ok(()), None, || Ok(vec![7_u64; 20]))
        .unwrap();
    let restarted = CacheRuntime::new(config.clone(), Some(dir.path().into()));
    assert_eq!(
        *restarted.get::<Vec<u64>>(&req, || Ok(())).unwrap().unwrap(),
        vec![7; 20]
    );
    assert_eq!(
        restarted.stats().namespaces[&CacheNamespace::Ast].persistent_hits,
        1
    );
    let object = std::fs::read_dir(dir.path().join("cache-runtime/v1"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .unwrap()
        .path();
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&object).unwrap()).unwrap();
    envelope["format"] = serde_json::json!(99);
    std::fs::write(&object, serde_json::to_vec(&envelope).unwrap()).unwrap();
    assert!(restarted
        .get::<Vec<u64>>(&req, || Ok(()))
        .unwrap()
        .is_none());
    assert!(restarted.stats().namespaces[&CacheNamespace::Ast]
        .miss_reasons
        .contains_key(&LocalMissReason::SchemaChanged));
    std::fs::write(object, b"{broken").unwrap();
    assert_eq!(
        *restarted
            .get_or_compute(&req, || Ok(()), None, || Ok(vec![8_u64; 20]))
            .unwrap(),
        vec![8; 20]
    );
    assert!(restarted.stats().namespaces[&CacheNamespace::Ast].corrupt_entries > 0);
    for i in 0..30 {
        let req = CacheRequest::new(
            request(&format!("item-{i}"), vec![]).key,
            CachePolicy::PersistentImmutable,
        );
        restarted
            .get_or_compute(&req, || Ok(()), None, || Ok(vec![1_u64; 100]))
            .unwrap();
    }
    let swept = restarted.sweep().unwrap();
    assert!(swept.bytes <= 4096);
}

#[test]
fn ordinary_reads_reuse_derived_work_but_reread_current_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.ts"), "const user = 1;\n").unwrap();
    let context = davinci_agent::ToolContext::default();
    let args = serde_json::json!({"path":"a.ts"});
    for _ in 0..2 {
        let result =
            davinci_agent::tools::execute_tool_with(dir.path(), "read", &args, &context).unwrap();
        assert_eq!(result.content, "const user = 1;");
    }
    assert!(context.cache.stats().namespaces[&CacheNamespace::File].memory_hits > 0);
    std::fs::write(dir.path().join("a.ts"), "const user = 2;\n").unwrap();
    let result =
        davinci_agent::tools::execute_tool_with(dir.path(), "read", &args, &context).unwrap();
    assert_eq!(result.content, "const user = 2;");
}

#[test]
fn disk_hits_promote_to_memory_without_rewriting_disk() {
    let dir = tempfile::tempdir().unwrap();
    let req = CacheRequest::new(
        request("promotion", vec![]).key,
        CachePolicy::PersistentImmutable,
    );
    CacheRuntime::new(CacheConfig::default(), Some(dir.path().into()))
        .put(&req, 42_u64, || Ok(()))
        .unwrap();
    let restarted = CacheRuntime::new(CacheConfig::default(), Some(dir.path().into()));
    for _ in 0..2 {
        assert_eq!(*restarted.get::<u64>(&req, || Ok(())).unwrap().unwrap(), 42);
    }
    let stats = &restarted.stats().namespaces[&CacheNamespace::Ast];
    assert_eq!(stats.persistent_hits, 1);
    assert_eq!(stats.memory_hits, 1);
    assert_eq!(stats.bytes_written, 0);
}

#[test]
fn invalidation_blocks_disk_and_inflight_publication() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CacheRuntime::new(CacheConfig::default(), Some(dir.path().into()));
    let dep = CacheDependency::ConfigHash("revoked".into());
    let req = CacheRequest::new(
        request("inflight", vec![dep.clone()]).key,
        CachePolicy::PersistentImmutable,
    );
    let result = cache.get_or_compute(
        &req,
        || Ok(()),
        None,
        || {
            cache.invalidate(&dep, LocalMissReason::ConfigChanged);
            Ok(7_u64)
        },
    );
    assert!(matches!(result, Err(CacheError::Invalidated)));
    assert!(cache.get::<u64>(&req, || Ok(())).unwrap().is_none());
    let restarted = CacheRuntime::new(CacheConfig::default(), Some(dir.path().into()));
    assert!(restarted.get::<u64>(&req, || Ok(())).unwrap().is_none());
}

#[test]
fn worktrees_share_immutable_artifacts_but_not_mutable_views() {
    let state = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let inputs = vec![("src/a.ts".into(), digest(b"const a = 1;"))];
    let first = WorkspaceSnapshot::new(a.path(), inputs.clone()).unwrap();
    let second = WorkspaceSnapshot::new(b.path(), inputs.clone()).unwrap();
    assert_ne!(first.dependencies(), second.dependencies());
    assert_eq!(
        first,
        WorkspaceSnapshot::new(a.path(), inputs.clone()).unwrap()
    );
    let parsed = CacheRequest::new(
        CacheKey::new(
            CacheNamespace::Ast,
            "typescript",
            1,
            "fixture-parser-v1",
            vec![CacheDependency::ContentHash(inputs[0].1.clone())],
        ),
        CachePolicy::PersistentImmutable,
    );
    let cache = CacheRuntime::new(CacheConfig::default(), Some(state.path().into()));
    cache
        .put(&parsed, vec!["a".to_string()], || Ok(()))
        .unwrap();
    let restarted = CacheRuntime::new(CacheConfig::default(), Some(state.path().into()));
    assert_eq!(
        *restarted
            .get::<Vec<String>>(&parsed, || Ok(()))
            .unwrap()
            .unwrap(),
        vec!["a"]
    );
    let view = |snapshot: &WorkspaceSnapshot| {
        CacheRequest::new(
            CacheKey::new(
                CacheNamespace::Repo,
                "map",
                1,
                "fixture-map-v1",
                snapshot.dependencies(),
            ),
            CachePolicy::PersistentWorkspaceBound,
        )
    };
    cache
        .put(&view(&first), "workspace-a".to_string(), || Ok(()))
        .unwrap();
    assert!(restarted
        .get::<String>(&view(&second), || Ok(()))
        .unwrap()
        .is_none());
    let changed =
        WorkspaceSnapshot::new(a.path(), vec![("src/a.ts".into(), digest(b"const b = 2;"))])
            .unwrap();
    assert!(restarted
        .get::<String>(&view(&changed), || Ok(()))
        .unwrap()
        .is_none());
}

#[test]
fn package_and_git_versions_are_independent_of_workspace_edits() {
    let cache = CacheRuntime::default();
    let manifest = br#"{"name":"fixture","exports":"./index.js"}"#;
    let req = CacheRequest::new(
        CacheKey::new(
            CacheNamespace::Package,
            "package-manifest",
            1,
            "json-v1",
            vec![
                CacheDependency::PackageManifestHash(digest(manifest)),
                CacheDependency::LockfileHash(digest(b"lock-v1")),
            ],
        ),
        CachePolicy::MemoryOnly,
    );
    for _ in 0..2 {
        let result = cache
            .get_or_compute(
                &req,
                || Ok(()),
                None,
                || {
                    serde_json::from_slice::<serde_json::Value>(manifest)
                        .map_err(|e| CacheError::Compute(e.to_string()))
                },
            )
            .unwrap();
        assert_eq!(result["name"], "fixture");
    }
    cache.invalidate(
        &CacheDependency::ContentHash("changed-source".into()),
        LocalMissReason::ContentChanged,
    );
    assert!(cache
        .get::<serde_json::Value>(&req, || Ok(()))
        .unwrap()
        .is_some());
    assert_eq!(
        cache.stats().namespaces[&CacheNamespace::Package].computations,
        1
    );
    let git = request(
        "commit-metadata",
        vec![CacheDependency::GitObject("a".repeat(40))],
    );
    cache.put(&git, 1_u64, || Ok(())).unwrap();
    assert!(cache.get::<u64>(&git, || Ok(())).unwrap().is_some());
}

#[test]
fn negative_entries_expire_and_workspace_policy_requires_scope() {
    let cache = CacheRuntime::default();
    let req = CacheRequest::new(
        request("missing", vec![]).key,
        CachePolicy::Negative { ttl_ms: 1 },
    );
    cache.put(&req, false, || Ok(())).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    assert!(cache.get::<bool>(&req, || Ok(())).unwrap().is_none());
    let invalid = CacheRequest::new(
        request("no-workspace", vec![]).key,
        CachePolicy::PersistentWorkspaceBound,
    );
    cache.put(&invalid, true, || Ok(())).unwrap();
    assert!(cache.get::<bool>(&invalid, || Ok(())).unwrap().is_none());
}

#[test]
fn persistent_process_writers_publish_complete_objects() {
    const CHILD: &str = "DAVINCI_CACHE_FIXTURE_ROOT";
    let req = CacheRequest::new(
        request("process-object", vec![]).key,
        CachePolicy::PersistentImmutable,
    );
    if let Some(root) = std::env::var_os(CHILD) {
        let cache = CacheRuntime::new(CacheConfig::default(), Some(root.into()));
        cache.put(&req, vec![17_u64; 4096], || Ok(())).unwrap();
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let mut children: Vec<_> = (0..4)
        .map(|_| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persistent_process_writers_publish_complete_objects",
                ])
                .env(CHILD, root.path())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let cache = CacheRuntime::new(CacheConfig::default(), Some(root.path().into()));
    assert_eq!(
        *cache.get::<Vec<u64>>(&req, || Ok(())).unwrap().unwrap(),
        vec![17; 4096]
    );
    let abandoned = root
        .path()
        .join("cache-runtime/v1")
        .join(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&abandoned, b"partial object").unwrap();
    assert_eq!(cache.sweep().unwrap().objects, 1);
    assert!(!abandoned.exists());
}

#[test]
fn confined_reads_reject_traversal_and_bound_bytes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"12345").unwrap();
    assert!(
        read_current_file(root.path(), std::path::Path::new("../file"), 10, || Ok(())).is_err()
    );
    assert!(read_current_file(root.path(), std::path::Path::new("file"), 4, || Ok(())).is_err());
    assert!(
        read_current_file(root.path(), std::path::Path::new("file"), 10, || Err(
            CacheError::Denied
        ))
        .is_err()
    );
    let snapshot =
        read_current_file(root.path(), std::path::Path::new("file"), 5, || Ok(())).unwrap();
    assert_eq!(snapshot.content_hash, digest(b"12345"));
}

#[test]
fn cancelled_waiter_does_not_cancel_leader_and_timeout_is_bounded() {
    let flight = Arc::new(SingleFlight::default());
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let leader_flight = flight.clone();
    let leader = std::thread::spawn(move || {
        leader_flight
            .run("wait", Duration::from_secs(1), None, || {
                started_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                Ok(42_u64)
            })
            .unwrap()
    });
    started_rx.recv().unwrap();
    assert!(matches!(
        flight.run("wait", Duration::ZERO, None, || Ok(0_u64)),
        Err(CacheError::Timeout)
    ));
    let cancel = davinci_agent::CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        flight.run("wait", Duration::from_secs(1), Some(&cancel), || Ok(0_u64)),
        Err(CacheError::Cancelled)
    ));
    finish_tx.send(()).unwrap();
    assert_eq!(*leader.join().unwrap().0, 42);
    assert_eq!(
        *flight
            .run("wait", Duration::from_secs(1), None, || Ok(43_u64))
            .unwrap()
            .0,
        43
    );
}

#[test]
fn optional_cache_failures_and_disabled_mode_preserve_reads() {
    let root = tempfile::tempdir().unwrap();
    let invalid_state = root.path().join("not-a-directory");
    std::fs::write(&invalid_state, "existing data").unwrap();
    std::fs::write(root.path().join("input.txt"), "first\nsecond\nthird\n").unwrap();
    let uncached = davinci_agent::ToolContext {
        cache: CacheRuntime::new(
            CacheConfig {
                enabled: false,
                ..Default::default()
            },
            None,
        ),
        ..Default::default()
    };
    let unavailable = davinci_agent::ToolContext {
        cache: CacheRuntime::new(CacheConfig::default(), Some(invalid_state.clone())),
        ..Default::default()
    };
    let args = serde_json::json!({"path":"input.txt","offset":2,"limit":1});
    let expected =
        davinci_agent::tools::execute_tool_with(root.path(), "read", &args, &uncached).unwrap();
    for _ in 0..2 {
        let actual =
            davinci_agent::tools::execute_tool_with(root.path(), "read", &args, &unavailable)
                .unwrap();
        assert_eq!(actual.content, expected.content);
        assert_eq!(actual.details, expected.details);
    }
    assert_eq!(
        std::fs::read_to_string(invalid_state).unwrap(),
        "existing data"
    );
    assert_eq!(unavailable.cache.stats().provider.input_tokens, 0);
    unavailable.cache.record_provider_usage(100, 20, 10);
    assert_eq!(unavailable.cache.stats().provider.cache_read_tokens, 20);
}

#[cfg(unix)]
#[test]
fn persistent_and_file_read_boundaries_reject_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("cache-runtime")).unwrap();
    let cache = CacheRuntime::new(CacheConfig::default(), Some(root.path().into()));
    let req = CacheRequest::new(
        request("escape", vec![]).key,
        CachePolicy::PersistentImmutable,
    );
    cache.put(&req, 9_u64, || Ok(())).unwrap();
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    std::fs::write(outside.path().join("secret"), b"private").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
    assert!(read_current_file(
        root.path(),
        std::path::Path::new("linked/secret"),
        100,
        || Ok(())
    )
    .is_err());
}
