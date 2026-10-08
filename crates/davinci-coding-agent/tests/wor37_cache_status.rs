//! WOR-37: `cache-status` reports unknown, not 0, for unreported cache writes.

use davinci_coding_agent::native_extensions::NativeExtensionHost;

fn status(record: impl FnOnce(&NativeExtensionHost)) -> serde_json::Value {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut host =
        NativeExtensionHost::new_with_agent_dir("wor37", root.path(), Some(state.path()));
    record(&host);
    host.command("cache-status", "").unwrap().unwrap()["provider"].clone()
}

#[test]
fn omitted_write_count_reports_null_and_unreported() {
    let provider = status(|host| host.cache.record_provider_usage_with_write(100, 30, None));
    assert!(provider["cacheWriteTokens"].is_null());
    assert_eq!(provider["cacheWriteStatus"], "unreported");
    assert_eq!(provider["cacheWriteUnreportedRequests"], 1);
    assert_eq!(provider["cacheReadTokens"], 30);
}

#[test]
fn measured_zero_reports_zero() {
    let provider = status(|host| {
        host.cache
            .record_provider_usage_with_write(100, 30, Some(0))
    });
    assert_eq!(provider["cacheWriteTokens"], 0);
    assert_eq!(provider["cacheWriteStatus"], "reported");
}

#[test]
fn mixed_requests_report_a_partial_lower_bound() {
    let provider = status(|host| {
        host.cache.record_provider_usage_with_write(100, 0, Some(4));
        host.cache.record_provider_usage_with_write(100, 0, None);
    });
    assert_eq!(provider["cacheWriteTokens"], 4);
    assert_eq!(provider["cacheWriteStatus"], "partial");
}

#[test]
fn no_requests_is_not_called_reported() {
    let provider = status(|_| {});
    assert_eq!(provider["cacheWriteStatus"], "none");
}
