//! WOR-107: the persistent cache tier evicts by write age, not by read recency.
//!
//! This is the intended policy, pinned here so a change is deliberate: every
//! read of a persisted object is promoted into the in-memory tier, which is the
//! true LRU (see `memory_lru_and_authority_are_enforced_on_every_hit`).  The
//! disk tier is a write-ordered backing store, and reads must not need write
//! access to the cache directory to touch timestamps.

use davinci_agent::runtime::cache::*;
use std::time::{Duration, SystemTime};

fn request(name: &str) -> CacheRequest {
    CacheRequest::new(
        CacheKey::new(CacheNamespace::Ast, name, 1, "parser-1", vec![]),
        CachePolicy::PersistentImmutable,
    )
}

fn objects(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(root.join("cache-runtime/v1"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    found.sort();
    found
}

#[test]
fn wor107_disk_eviction_follows_write_age_not_read_recency() {
    let root = tempfile::tempdir().unwrap();
    let probe = CacheRuntime::new(CacheConfig::default(), Some(root.path().into()));
    probe
        .get_or_compute(&request("probe"), || Ok(()), None, || Ok(vec![1_u64; 64]))
        .unwrap();
    let object_size = std::fs::metadata(&objects(root.path())[0]).unwrap().len();
    std::fs::remove_dir_all(root.path().join("cache-runtime")).unwrap();

    // Room for exactly two objects.
    let config = CacheConfig {
        persistent_max_bytes: object_size * 2 + object_size / 2,
        ..Default::default()
    };
    let cache = CacheRuntime::new(config.clone(), Some(root.path().into()));
    let (a, b, c) = (request("a"), request("b"), request("c"));
    cache
        .get_or_compute(&a, || Ok(()), None, || Ok(vec![1_u64; 64]))
        .unwrap();
    cache
        .get_or_compute(&b, || Ok(()), None, || Ok(vec![2_u64; 64]))
        .unwrap();
    // Make the write order unambiguous regardless of clock granularity.
    let files = objects(root.path());
    assert_eq!(files.len(), 2);
    let mut by_content: Vec<_> = files
        .iter()
        .map(|path| (path.clone(), std::fs::read_to_string(path).unwrap()))
        .collect();
    by_content.sort_by_key(|(_, text)| text.contains("\"logical\":\"b\""));
    let now = SystemTime::now();
    for (age, (path, _)) in [(60, &by_content[0]), (30, &by_content[1])] {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(now - Duration::from_secs(age))
            .unwrap();
    }

    // A fresh process reads A repeatedly: A is hot, B is cold.
    let reader = CacheRuntime::new(config.clone(), Some(root.path().into()));
    for _ in 0..3 {
        assert!(reader.get::<Vec<u64>>(&a, || Ok(())).unwrap().is_some());
    }
    reader
        .get_or_compute(&c, || Ok(()), None, || Ok(vec![3_u64; 64]))
        .unwrap();

    let survivors: Vec<_> = objects(root.path())
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    assert_eq!(survivors.len(), 2, "budget holds two objects");
    assert!(
        !survivors
            .iter()
            .any(|text| text.contains("\"logical\":\"a\"")),
        "the oldest write is evicted even though it was read most recently"
    );
}
