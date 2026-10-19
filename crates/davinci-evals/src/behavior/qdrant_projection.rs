//! The promotion gate for the optional Qdrant projection.
//!
//! [`evaluate_qdrant_projection`] runs one fixed, seeded corpus against a
//! Qdrant at a URL and against exact local search, then reports:
//!
//! - retrieval p95 for both and the improvement (the decision doc requires at
//!   least [`PROMOTION_P95_IMPROVEMENT`] at equal quality);
//! - recall of the remote top-k against exact local top-k;
//! - namespace isolation, retired-record exclusion, model-change rebuild and
//!   outage recovery, each as its own gate.
//!
//! What the numbers mean depends on where they came from. Against the
//! in-process fake the functional gates are real but the latency figures only
//! measure a loopback fake, so the performance gate is `Unverified` and the
//! recommendation stays "keep local". Only a run against a real Qdrant
//! ([`LatencySource::Live`]) can produce `Go`. No default is changed here:
//! nothing in this module turns the projection on.

use davinci_coding_agent::native_extensions::memory_backends::qdrant::{
    collection_name, QdrantProjection,
};
use davinci_coding_agent::native_extensions::memory_backends::{
    MemoryProjectionBackend, ProjectionPoint, Secret, VectorProjection,
};
use davinci_coding_agent::native_extensions::vector_memory::{
    EmbeddingIdentity, VectorMemoryConfig,
};
use serde::Serialize;
use std::time::Instant;

/// From `2026-09-15-memory-projection-decision.md`: 30% better p95.
pub const PROMOTION_P95_IMPROVEMENT: f64 = 0.30;
/// Remote top-k must contain this share of the exact local top-k.
pub const MIN_RECALL_AT_K: f64 = 0.95;

#[derive(Debug, Clone)]
pub struct QdrantEvaluationConfig {
    pub records: usize,
    pub queries: usize,
    pub dimensions: usize,
    pub k: usize,
    pub seed: u64,
}

impl Default for QdrantEvaluationConfig {
    fn default() -> Self {
        Self {
            records: 1500,
            queries: 60,
            dimensions: 64,
            k: 10,
            seed: 0x5eed_0fc0_ffee,
        }
    }
}

/// Where the latency numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LatencySource {
    /// An in-process fake: proves behavior, says nothing about speed.
    OfflineFake,
    /// A real Qdrant the caller pointed the harness at.
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "detail", rename_all = "kebab-case")]
pub enum Gate {
    Pass,
    Fail(String),
    Unverified(String),
}

impl Gate {
    pub fn is_pass(&self) -> bool {
        matches!(self, Gate::Pass)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "reasons", rename_all = "kebab-case")]
pub enum Recommendation {
    /// Every gate passed on live numbers. A human still decides; no default
    /// is changed by this module.
    Go,
    /// Keep the local default: a gate failed or could not be verified.
    NoGo(Vec<String>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QdrantEvaluationReceipt {
    pub source: LatencySource,
    pub records: usize,
    pub queries: usize,
    pub dimensions: usize,
    pub k: usize,
    pub local_p95_ms: f64,
    pub qdrant_p95_ms: f64,
    /// `(local - qdrant) / local`; negative means Qdrant was slower.
    pub p95_improvement: f64,
    pub recall_at_k: f64,
    pub quality_parity: Gate,
    pub performance: Gate,
    pub namespace_isolation: Gate,
    pub retired_exclusion: Gate,
    pub model_change_rebuild: Gate,
    pub outage_recovery: Gate,
    /// Records not mirrored after the outage was over and the replay ran.
    pub lag_after_recovery: Option<usize>,
    pub recommendation: Recommendation,
}

/// Lets the caller take the Qdrant down and bring it back, for the outage
/// gate. `None` (a real Qdrant you do not control) leaves that gate
/// `Unverified`.
pub type OutageControl<'a> = &'a dyn Fn(bool);

/// Deterministic xorshift64*: no dependency, same corpus every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    vector.iter_mut().for_each(|x| *x /= norm);
    vector
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A clustered corpus and queries drawn near the same clusters.
fn corpus(config: &QdrantEvaluationConfig) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = Rng(config.seed | 1);
    let centroids: Vec<Vec<f32>> = (0..16)
        .map(|_| normalize((0..config.dimensions).map(|_| rng.unit()).collect()))
        .collect();
    let near = |rng: &mut Rng, index: usize| {
        let centroid = &centroids[index % centroids.len()];
        normalize(
            centroid
                .iter()
                .map(|value| value + 0.35 * rng.unit())
                .collect(),
        )
    };
    let records = (0..config.records).map(|i| near(&mut rng, i)).collect();
    let queries = (0..config.queries)
        .map(|i| near(&mut rng, i * 7 + 3))
        .collect();
    (records, queries)
}

/// Nearest-rank percentile.
fn p95(samples: &mut [f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let rank = ((samples.len() as f64) * 0.95).ceil() as usize;
    samples[rank.clamp(1, samples.len()) - 1]
}

fn top_k_local(corpus: &[Vec<f32>], query: &[f32], k: usize) -> Vec<String> {
    let mut scored: Vec<(f32, usize)> = corpus
        .iter()
        .enumerate()
        .map(|(index, vector)| (cosine(query, vector), index))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(k)
        .map(|(_, index)| record_id(index))
        .collect()
}

fn record_id(index: usize) -> String {
    format!("rec-{index:06}")
}

fn points(records: &[Vec<f32>], repo: &str, prefix: &str) -> Vec<ProjectionPoint> {
    records
        .iter()
        .enumerate()
        .map(|(index, vector)| ProjectionPoint {
            id: format!("{prefix}{}", record_id(index)),
            vector: vector.clone(),
            repo_id: repo.into(),
        })
        .collect()
}

fn config_for(url: &str, dimensions: usize, api_key: Option<&str>) -> VectorMemoryConfig {
    let mut config = VectorMemoryConfig {
        qdrant_url: url.to_string(),
        embedding_dimensions: dimensions,
        request_timeout_seconds: 30,
        ..VectorMemoryConfig::default()
    };
    config.projection.backend = MemoryProjectionBackend::Qdrant;
    config.projection.allow_remote = !url.contains("127.0.0.1") && !url.contains("localhost");
    config.projection.api_key = api_key.and_then(Secret::new);
    config
}

fn client(
    url: &str,
    dimensions: usize,
    model: &str,
    repo: &str,
    api_key: Option<&str>,
) -> Result<QdrantProjection, String> {
    let mut config = config_for(url, dimensions, api_key);
    config.embedding_model = model.into();
    QdrantProjection::new(&config, repo).map_err(|error| error.to_string())
}

/// Run the whole gate. Uses its own, uniquely named repos so a shared Qdrant
/// is not disturbed beyond the namespaces it creates, and deletes only those
/// namespaces afterwards. Another repo's vectors are written into the same
/// collection on purpose (worst case for isolation).
pub fn evaluate_qdrant_projection(
    config: &QdrantEvaluationConfig,
    url: &str,
    source: LatencySource,
    api_key: Option<&str>,
    outage: Option<OutageControl>,
) -> Result<QdrantEvaluationReceipt, String> {
    let run = format!("eval-{:x}", config.seed);
    let (repo_a, repo_b) = (format!("{run}-a"), format!("{run}-b"));
    let (records, queries) = corpus(config);
    let main = client(url, config.dimensions, "eval-model-1", &repo_a, api_key)?;

    // Load: repo A gets the corpus; repo B's identical vectors sit in the same
    // collection under other ids, so a missing filter shows up as a leak.
    main.upsert(&points(&records, &repo_a, ""))
        .map_err(|error| error.to_string())?;
    main.upsert(&points(&records, &repo_b, "foreign-"))
        .map_err(|error| error.to_string())?;

    // Latency and recall.
    for query in queries.iter().take(3) {
        let _ = main.query(query, &repo_a, config.k);
        let _ = top_k_local(&records, query, config.k);
    }
    let (mut local_ms, mut remote_ms, mut recall_sum) = (Vec::new(), Vec::new(), 0.0f64);
    for query in &queries {
        let started = Instant::now();
        let local = top_k_local(&records, query, config.k);
        local_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        let remote = main
            .query(query, &repo_a, config.k)
            .map_err(|error| error.to_string())?;
        remote_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let found = remote.iter().filter(|hit| local.contains(&hit.id)).count();
        recall_sum += found as f64 / config.k as f64;
    }
    let recall_at_k = recall_sum / queries.len().max(1) as f64;
    let (local_p95_ms, qdrant_p95_ms) = (p95(&mut local_ms), p95(&mut remote_ms));
    let p95_improvement = if local_p95_ms > 0.0 {
        (local_p95_ms - qdrant_p95_ms) / local_p95_ms
    } else {
        0.0
    };

    let quality_parity = if recall_at_k >= MIN_RECALL_AT_K {
        Gate::Pass
    } else {
        Gate::Fail(format!(
            "recall@{} was {recall_at_k:.3}, below {MIN_RECALL_AT_K}",
            config.k
        ))
    };
    let performance = match source {
        LatencySource::OfflineFake => Gate::Unverified(format!(
            "measured against an in-process fake (p95 {qdrant_p95_ms:.2} ms vs local {local_p95_ms:.2} ms); \
             run it against a real Qdrant to evaluate the {:.0}% gate",
            PROMOTION_P95_IMPROVEMENT * 100.0
        )),
        LatencySource::Live if p95_improvement >= PROMOTION_P95_IMPROVEMENT => Gate::Pass,
        LatencySource::Live => Gate::Fail(format!(
            "p95 improved {:.1}%, below the required {:.0}%",
            p95_improvement * 100.0,
            PROMOTION_P95_IMPROVEMENT * 100.0
        )),
    };

    // Namespace isolation: repo A never sees repo B's points, B never sees A's,
    // and A cannot delete B's.
    let probe = &queries[0];
    let from_a = main
        .query(probe, &repo_a, config.k)
        .map_err(|error| error.to_string())?;
    let from_b = main
        .query(probe, &repo_b, config.k)
        .map_err(|error| error.to_string())?;
    let foreign_ids: Vec<String> = from_b.iter().map(|hit| hit.id.clone()).collect();
    main.delete(&foreign_ids, &repo_a)
        .map_err(|error| error.to_string())?;
    let b_after = main
        .query(probe, &repo_b, config.k)
        .map_err(|error| error.to_string())?;
    let namespace_isolation = if from_a.len() == config.k
        && from_a.iter().all(|hit| !hit.id.starts_with("foreign-"))
        && from_b.len() == config.k
        && from_b.iter().all(|hit| hit.id.starts_with("foreign-"))
        && b_after.len() == config.k
    {
        Gate::Pass
    } else {
        Gate::Fail(format!(
            "A saw {} hits ({} foreign), B saw {} hits, B after A's delete {}",
            from_a.len(),
            from_a
                .iter()
                .filter(|hit| hit.id.starts_with("foreign-"))
                .count(),
            from_b.len(),
            b_after.len()
        ))
    };

    // Retired records: delete the top hits for the probe, then query again.
    let retired: Vec<String> = from_a.iter().take(3).map(|hit| hit.id.clone()).collect();
    main.delete(&retired, &repo_a)
        .map_err(|error| error.to_string())?;
    let after = main
        .query(probe, &repo_a, config.k)
        .map_err(|error| error.to_string())?;
    let retired_exclusion = if after.iter().all(|hit| !retired.contains(&hit.id)) {
        Gate::Pass
    } else {
        Gate::Fail("a deleted record was still returned".into())
    };

    // Model change: another model identity is another collection; the old one
    // is untouched and the new one starts empty until rebuilt.
    let rebuilt = client(url, config.dimensions, "eval-model-2", &repo_a, api_key)?;
    let identity = |model: &str| EmbeddingIdentity {
        model: model.into(),
        dimensions: config.dimensions,
        prefix_revision: 1,
    };
    let names_differ = collection_name("davinci", &repo_a, &identity("eval-model-1"))
        != collection_name("davinci", &repo_a, &identity("eval-model-2"));
    let empty_before = rebuilt
        .query(probe, &repo_a, config.k)
        .map(|hits| hits.is_empty())
        .unwrap_or(false);
    rebuilt
        .upsert(&points(&records, &repo_a, ""))
        .map_err(|error| error.to_string())?;
    let filled = rebuilt
        .query(probe, &repo_a, config.k)
        .map(|hits| hits.len() == config.k)
        .unwrap_or(false);
    let old_intact = main
        .query(probe, &repo_a, config.k)
        .map(|hits| !hits.is_empty())
        .unwrap_or(false);
    let model_change_rebuild = if names_differ && empty_before && filled && old_intact {
        Gate::Pass
    } else {
        Gate::Fail(format!(
            "names differ {names_differ}, new empty {empty_before}, rebuilt {filled}, old intact {old_intact}"
        ))
    };

    // Outage: while down, writes fail loudly (the caller keeps the record and
    // counts lag); after recovery the replay lands every record exactly once.
    let (outage_recovery, lag_after_recovery) = match outage {
        None => (
            Gate::Unverified("the harness cannot stop this Qdrant to test an outage".into()),
            None,
        ),
        Some(control) => {
            let batch = points(&records[..config.k.min(records.len())], &repo_a, "outage-");
            control(true);
            let failed_while_down = main.upsert(&batch).is_err();
            control(false);
            let replay = main.upsert(&batch);
            let again = main.upsert(&batch); // replay is idempotent
            let hits = main.query(&batch[0].vector, &repo_a, config.records + batch.len());
            let landed = hits
                .as_ref()
                .map(|hits| {
                    batch
                        .iter()
                        .filter(|point| hits.iter().filter(|hit| hit.id == point.id).count() == 1)
                        .count()
                })
                .unwrap_or(0);
            let lag = batch.len() - landed;
            let ok = failed_while_down && replay.is_ok() && again.is_ok();
            (
                if ok && lag == 0 {
                    Gate::Pass
                } else {
                    Gate::Fail(format!(
                        "down write failed {failed_while_down}, replay {replay:?}, lag {lag}"
                    ))
                },
                Some(lag),
            )
        }
    };

    // Clean up only what this run created.
    let _ = main.clear_namespace(&repo_a);
    let _ = main.clear_namespace(&repo_b);
    let _ = rebuilt.clear_namespace(&repo_a);

    let gates = [
        ("quality parity", &quality_parity),
        ("performance", &performance),
        ("namespace isolation", &namespace_isolation),
        ("retired exclusion", &retired_exclusion),
        ("model-change rebuild", &model_change_rebuild),
        ("outage recovery", &outage_recovery),
    ];
    let reasons: Vec<String> = gates
        .iter()
        .filter_map(|(name, gate)| match gate {
            Gate::Pass => None,
            Gate::Fail(detail) => Some(format!("{name} failed: {detail}")),
            Gate::Unverified(detail) => Some(format!("{name} is unverified: {detail}")),
        })
        .collect();
    let recommendation = if reasons.is_empty() {
        Recommendation::Go
    } else {
        Recommendation::NoGo(reasons)
    };

    Ok(QdrantEvaluationReceipt {
        source,
        records: config.records,
        queries: config.queries,
        dimensions: config.dimensions,
        k: config.k,
        local_p95_ms,
        qdrant_p95_ms,
        p95_improvement,
        recall_at_k,
        quality_parity,
        performance,
        namespace_isolation,
        retired_exclusion,
        model_change_rebuild,
        outage_recovery,
        lag_after_recovery,
        recommendation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_coding_agent::native_extensions::memory_backends::fake_qdrant::FakeQdrant;

    #[test]
    fn qdrant_optional_performance_and_outage_gate() {
        // No mandatory Qdrant: the defaults are local, in config and profile.
        let defaults = VectorMemoryConfig::default();
        assert_eq!(defaults.projection.backend, MemoryProjectionBackend::Local);
        assert!(defaults.projection.api_key.is_none());
        assert!(!defaults.projection.allow_remote);

        let fake = FakeQdrant::start();
        let control = |down: bool| fake.set_down(down);
        let receipt = evaluate_qdrant_projection(
            &QdrantEvaluationConfig::default(),
            &fake.url(),
            LatencySource::OfflineFake,
            None,
            Some(&control),
        )
        .expect("the harness runs against the fake");

        eprintln!(
            "qdrant-eval offline (fake, not a speed claim): local p95 {:.3} ms, fake qdrant p95 {:.3} ms, recall@{} {:.3}",
            receipt.local_p95_ms, receipt.qdrant_p95_ms, receipt.k, receipt.recall_at_k
        );
        // Every functional gate passes against the fake.
        assert!(receipt.quality_parity.is_pass(), "{receipt:#?}");
        assert!(receipt.namespace_isolation.is_pass(), "{receipt:#?}");
        assert!(receipt.retired_exclusion.is_pass(), "{receipt:#?}");
        assert!(receipt.model_change_rebuild.is_pass(), "{receipt:#?}");
        assert!(receipt.outage_recovery.is_pass(), "{receipt:#?}");
        assert_eq!(receipt.lag_after_recovery, Some(0));
        assert!(receipt.recall_at_k >= MIN_RECALL_AT_K);

        // The speed gate cannot be claimed from a fake, so the answer is "keep
        // local" with the reason on the record, whatever the numbers were.
        assert!(matches!(receipt.performance, Gate::Unverified(_)));
        let Recommendation::NoGo(reasons) = &receipt.recommendation else {
            panic!("an offline run must not recommend Go: {receipt:#?}");
        };
        assert!(reasons
            .iter()
            .any(|reason| reason.contains("performance is unverified")));
        assert!(receipt.local_p95_ms > 0.0 && receipt.qdrant_p95_ms > 0.0);

        // The run cleaned up its own namespaces.
        for name in fake.collection_names() {
            assert_eq!(fake.collection(&name).unwrap().points.len(), 0, "{name}");
        }
        // It is a plain, serializable receipt.
        let json = serde_json::to_value(&receipt).unwrap();
        assert_eq!(json["source"], "offline-fake");
        assert_eq!(json["recommendation"]["decision"], "no-go");
    }

    #[test]
    fn the_gate_is_deterministic_and_a_down_qdrant_is_an_error() {
        // Percentile and corpus are deterministic.
        let mut samples = vec![5.0, 1.0, 3.0, 2.0, 4.0, 9.0, 8.0, 7.0, 6.0, 10.0];
        assert_eq!(p95(&mut samples), 10.0);
        assert_eq!(p95(&mut []), 0.0);
        let config = QdrantEvaluationConfig {
            records: 40,
            queries: 4,
            dimensions: 8,
            k: 5,
            ..QdrantEvaluationConfig::default()
        };
        assert_eq!(corpus(&config), corpus(&config));
        // A Qdrant that is down at the start is an error, not a pass.
        let fake = FakeQdrant::start();
        fake.set_down(true);
        let outcome = evaluate_qdrant_projection(
            &config,
            &fake.url(),
            LatencySource::OfflineFake,
            None,
            None,
        );
        assert!(outcome.is_err());
    }

    #[test]
    fn the_gate_fails_when_the_remote_leaks_or_forgets() {
        let config = QdrantEvaluationConfig {
            records: 60,
            queries: 4,
            dimensions: 8,
            k: 5,
            ..QdrantEvaluationConfig::default()
        };
        let run = |fake: &FakeQdrant| {
            evaluate_qdrant_projection(&config, &fake.url(), LatencySource::OfflineFake, None, None)
                .expect("runs")
        };

        let healthy = FakeQdrant::start();
        let receipt = run(&healthy);
        assert!(receipt.namespace_isolation.is_pass() && receipt.retired_exclusion.is_pass());

        // A Qdrant that ignores the repo filter leaks across namespaces.
        let leaky = FakeQdrant::start();
        leaky.ignore_filters(true);
        let receipt = run(&leaky);
        assert!(
            matches!(receipt.namespace_isolation, Gate::Fail(_)),
            "{receipt:#?}"
        );
        assert!(matches!(receipt.recommendation, Recommendation::NoGo(_)));

        // A Qdrant that acknowledges deletes and keeps the points resurrects
        // retired memories.
        let forgetful = FakeQdrant::start();
        forgetful.ignore_deletes(true);
        let receipt = run(&forgetful);
        assert!(
            matches!(receipt.retired_exclusion, Gate::Fail(_)),
            "{receipt:#?}"
        );
    }

    /// Run against a real Qdrant you own:
    /// `DAVINCI_QDRANT_LIVE_URL=http://127.0.0.1:6333 cargo test -p davinci-evals
    /// qdrant_live_measurement -- --ignored --nocapture`. Optional key in
    /// `DAVINCI_QDRANT_LIVE_API_KEY`. It creates and removes only its own
    /// `eval-*` namespaces. Not run by default; no live number exists yet.
    #[test]
    #[ignore = "needs a real Qdrant; opt-in"]
    fn qdrant_live_measurement() {
        let Ok(url) = std::env::var("DAVINCI_QDRANT_LIVE_URL") else {
            eprintln!("DAVINCI_QDRANT_LIVE_URL is not set; nothing measured");
            return;
        };
        let key = std::env::var("DAVINCI_QDRANT_LIVE_API_KEY").ok();
        let config = QdrantEvaluationConfig {
            records: 20_000,
            dimensions: 768,
            ..QdrantEvaluationConfig::default()
        };
        let receipt =
            evaluate_qdrant_projection(&config, &url, LatencySource::Live, key.as_deref(), None)
                .expect("live run");
        println!("{}", serde_json::to_string_pretty(&receipt).unwrap());
    }
}
