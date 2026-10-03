//! Offline acceptance measurements. The AST consumer is a fixture, not a parser implementation.
use davinci_agent::runtime::cache::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier,
};
use std::time::{Duration, Instant};

#[test]
#[ignore = "explicit cache acceptance benchmark"]
fn immutable_reuse_restart_and_large_workspace_edit() {
    let cache = CacheRuntime::new(
        CacheConfig {
            max_entries: 20_000,
            persistent_enabled: false,
            ..Default::default()
        },
        None,
    );
    let parses = AtomicUsize::new(0);
    let parse = |i: usize, changed: bool| {
        let source = format!(
            "function item_{i}() {{ return {}; }}",
            if changed { 2 } else { 1 }
        );
        let key = CacheKey::new(
            CacheNamespace::Ast,
            "fixture-symbol-extractor",
            1,
            "fixture-v1",
            vec![CacheDependency::ContentHash(digest(source.as_bytes()))],
        );
        cache
            .get_or_compute(
                &CacheRequest::new(key, CachePolicy::MemoryOnly),
                || Ok(()),
                None,
                || {
                    parses.fetch_add(1, Ordering::SeqCst);
                    Ok(source
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect::<Vec<_>>())
                },
            )
            .unwrap()
    };
    let start = Instant::now();
    for i in 0..10_000 {
        std::hint::black_box(parse(i, false));
    }
    let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    for i in 0..10_000 {
        std::hint::black_box(parse(i, false));
    }
    let warm_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(parses.load(Ordering::SeqCst), 10_000);
    let start = Instant::now();
    for i in 0..10_000 {
        std::hint::black_box(parse(i, i == 42));
    }
    let edit_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(parses.load(Ordering::SeqCst), 10_001);

    let state = tempfile::tempdir().unwrap();
    let disk = CacheRuntime::new(CacheConfig::default(), Some(state.path().into()));
    let req = CacheRequest::new(
        CacheKey::new(
            CacheNamespace::Ast,
            "restart-fixture",
            1,
            "fixture-v1",
            vec![CacheDependency::ContentHash(digest(b"stable source"))],
        ),
        CachePolicy::PersistentImmutable,
    );
    let start = Instant::now();
    disk.put(&req, vec!["symbol".to_string(); 100], || Ok(()))
        .unwrap();
    let disk_write_ms = start.elapsed().as_secs_f64() * 1000.0;
    let restarted = CacheRuntime::new(CacheConfig::default(), Some(state.path().into()));
    let start = Instant::now();
    assert_eq!(
        restarted
            .get::<Vec<String>>(&req, || Ok(()))
            .unwrap()
            .unwrap()
            .len(),
        100
    );
    let restart_ms = start.elapsed().as_secs_f64() * 1000.0;
    let barrier = Arc::new(Barrier::new(10));
    let parallel = CacheRuntime::default();
    let workers: Vec<_> = (0..10)
        .map(|_| {
            let (barrier, cache) = (barrier.clone(), parallel.clone());
            let req = req.clone();
            std::thread::spawn(move || {
                barrier.wait();
                cache
                    .get_or_compute(
                        &req,
                        || Ok(()),
                        None,
                        || {
                            std::thread::sleep(Duration::from_millis(100));
                            Ok(7_u64)
                        },
                    )
                    .unwrap()
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(*worker.join().unwrap(), 7);
    }
    let stats = parallel.stats();
    assert_eq!(stats.namespaces[&CacheNamespace::Ast].computations, 1);
    assert_eq!(stats.namespaces[&CacheNamespace::Ast].waiter_reuse, 9);
    println!(
        "{}",
        serde_json::json!({"fixture_files":10000,"initial_computations":10000,
        "edit_computations":1,"unchanged_reused":9999,"cold_ms":cold_ms,"warm_ms":warm_ms,
        "one_edit_ms":edit_ms,"disk_write_ms":disk_write_ms,"restart_read_ms":restart_ms,
        "memory":cache.stats(),"disk":disk.sweep().unwrap(),"singleflight":stats})
    );
}
