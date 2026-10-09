//! WOR-37: the provider cache counters keep an unreported write count apart
//! from a measured zero.

use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};

#[test]
fn an_omitted_write_count_is_not_added_as_zero() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage_with_write(100, 20, None);
    assert_eq!(cache.provider_cache_write_coverage(), (0, 1));
    assert_eq!(cache.stats().provider.cache_write_tokens, 0);
    assert_eq!(cache.stats().provider.cache_read_tokens, 20);
}

#[test]
fn a_measured_zero_counts_as_reported() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage_with_write(100, 20, Some(0));
    assert_eq!(cache.provider_cache_write_coverage(), (1, 0));
}

#[test]
fn mixed_requests_sum_only_reported_writes() {
    let cache = CacheRuntime::new(CacheConfig::default(), None);
    cache.record_provider_usage(10, 0, 7);
    cache.record_provider_usage_with_write(10, 0, None);
    cache.record_provider_usage_with_write(10, 0, Some(3));
    assert_eq!(cache.provider_cache_write_coverage(), (2, 1));
    assert_eq!(cache.stats().provider.cache_write_tokens, 10);
}
