//! Offline host-boundary checks; no provider or pricing assumptions.
use davinci_agent::Agent;
use davinci_ai::AssistantMessage;

#[test]
fn harness_latency_separates_queue_provider_and_retry() {
    let mut agent = Agent::new("latency fixture");
    agent.auto_retry = true;
    agent.retry_attempts = 1;
    agent.retry_base_delay_ms = 10;
    agent.prompt("latency fixture");
    let result = agent.run_loop(|_| {
        std::thread::sleep(std::time::Duration::from_millis(10));
        Err::<AssistantMessage, String>("overloaded_error".into())
    });
    assert!(result.is_err());
    let stats = serde_json::to_value(agent.run_stats()).unwrap();
    for field in [
        "wallMs",
        "preparationMs",
        "queueMs",
        "providerMs",
        "retryWaitMs",
    ] {
        assert!(
            stats[field].is_u64(),
            "missing observed stage {field}: {stats}"
        );
    }
    assert!(stats["providerMs"].as_u64().unwrap() >= 20);
    assert!(stats["retryWaitMs"].as_u64().unwrap() >= 10);
    assert!(stats["wallMs"].as_u64().unwrap() >= stats["modelWallMs"].as_u64().unwrap());
    // No integration or digest/retrieval phase ran in this fixture.
    assert!(stats["integrationMs"].is_null());
    assert!(stats["digestRetrievalMs"].is_null());
}

#[test]
fn harness_old_stats_keep_unobserved_time_unknown() {
    let mut value = serde_json::to_value(davinci_agent::RunStats::default()).unwrap();
    for field in [
        "wallMs",
        "preparationMs",
        "queueMs",
        "providerMs",
        "retryWaitMs",
        "verificationWorkMs",
        "integrationMs",
        "digestRetrievalMs",
        "diagnosticsMs",
        "diagnosticComparableOperations",
        "diagnosticUnknownOperations",
        "repeatedReads",
        "repeatedSearches",
        "workerDuplicateOperations",
    ] {
        value.as_object_mut().unwrap().remove(field);
    }
    let restored: davinci_agent::RunStats = serde_json::from_value(value).unwrap();
    let json = serde_json::to_value(restored).unwrap();
    for field in [
        "wallMs",
        "queueMs",
        "providerMs",
        "verificationWorkMs",
        "diagnosticsMs",
        "repeatedReads",
        "workerDuplicateOperations",
    ] {
        assert_eq!(json.get(field), Some(&serde_json::Value::Null));
    }
}

#[test]
fn harness_context_preparation_records_digest_retrieval_overhead() {
    let agent = Agent::new("context timing fixture");
    assert!(agent.run_stats().digest_retrieval_ms.is_none());
    let _ = agent.prepared_context_image();
    assert!(agent.run_stats().digest_retrieval_ms.is_some());
}
