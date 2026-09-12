//! Deterministic statistics for repeated paired behavior evaluations.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_PROMOTION_REPEATS: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepeatedMetric {
    pub samples: Vec<f64>,
    pub mean: f64,
    pub median: f64,
    pub p10: f64,
    pub p90: f64,
}

impl RepeatedMetric {
    pub fn from_samples<S>(samples: S) -> Self
    where
        S: AsRef<[f64]>,
    {
        let mut sorted = samples.as_ref().to_vec();
        sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

        let mean = if sorted.is_empty() {
            0.0
        } else {
            sorted.iter().sum::<f64>() / sorted.len() as f64
        };

        Self {
            samples: samples.as_ref().to_vec(),
            mean,
            median: median(&sorted),
            p10: percentile(&sorted, 0.10),
            p90: percentile(&sorted, 0.90),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioRunSample {
    pub scenario_id: String,
    pub repetition: u32,
    pub passed: bool,
    pub wall_ms: u64,
    pub tool_calls: u64,
}

pub type ScenarioObservation = ScenarioRunSample;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedScenarioObservation {
    pub scenario_id: String,
    pub repetition: u32,
    pub baseline_passed: bool,
    pub candidate_passed: bool,
    pub baseline_wall_ms: u64,
    pub candidate_wall_ms: u64,
    pub baseline_tool_calls: u64,
    pub candidate_tool_calls: u64,
}

pub type PairedObservation = PairedScenarioObservation;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairedScenarioDelta {
    pub scenario_id: String,
    pub baseline_pass_rate: f64,
    pub candidate_pass_rate: f64,
    pub wall_ms_delta_median: f64,
    pub tool_call_delta_median: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioWinTieLoss {
    pub scenario_id: String,
    pub wins: usize,
    pub ties: usize,
    pub losses: usize,
}

pub type ScenarioWinLoss = ScenarioWinTieLoss;

pub fn pair_by_scenario_repetition(
    baseline: &[ScenarioRunSample],
    candidate: &[ScenarioRunSample],
) -> Result<Vec<PairedScenarioObservation>, String> {
    let baseline_by_key = index_samples("baseline", baseline)?;
    let candidate_by_key = index_samples("candidate", candidate)?;

    if baseline_by_key.len() != candidate_by_key.len() {
        return Err(format!(
            "baseline/candidate sample counts differ: {} != {}",
            baseline_by_key.len(),
            candidate_by_key.len()
        ));
    }

    let mut paired = Vec::with_capacity(baseline_by_key.len());
    for (key, baseline_sample) in baseline_by_key {
        let candidate_sample = candidate_by_key.get(&key).ok_or_else(|| {
            format!(
                "candidate is missing scenario '{}' repetition {}",
                key.0, key.1
            )
        })?;
        paired.push(PairedScenarioObservation {
            scenario_id: key.0,
            repetition: key.1,
            baseline_passed: baseline_sample.passed,
            candidate_passed: candidate_sample.passed,
            baseline_wall_ms: baseline_sample.wall_ms,
            candidate_wall_ms: candidate_sample.wall_ms,
            baseline_tool_calls: baseline_sample.tool_calls,
            candidate_tool_calls: candidate_sample.tool_calls,
        });
    }
    Ok(paired)
}

fn index_samples(
    label: &str,
    samples: &[ScenarioRunSample],
) -> Result<BTreeMap<(String, u32), ScenarioRunSample>, String> {
    let mut indexed = BTreeMap::new();
    for sample in samples {
        let key = (sample.scenario_id.clone(), sample.repetition);
        if indexed.insert(key.clone(), sample.clone()).is_some() {
            return Err(format!(
                "duplicate {label} sample for scenario '{}' repetition {}",
                key.0, key.1
            ));
        }
    }
    Ok(indexed)
}

pub fn paired_scenario_deltas(
    observations: &[PairedScenarioObservation],
) -> Vec<PairedScenarioDelta> {
    let mut grouped: BTreeMap<String, Vec<&PairedScenarioObservation>> = BTreeMap::new();
    for observation in observations {
        grouped
            .entry(observation.scenario_id.clone())
            .or_default()
            .push(observation);
    }

    grouped
        .into_iter()
        .map(|(scenario_id, observations)| {
            let count = observations.len() as f64;
            let baseline_pass_rate = observations
                .iter()
                .filter(|observation| observation.baseline_passed)
                .count() as f64
                / count;
            let candidate_pass_rate = observations
                .iter()
                .filter(|observation| observation.candidate_passed)
                .count() as f64
                / count;
            let wall_deltas: Vec<f64> = observations
                .iter()
                .map(|observation| {
                    observation.candidate_wall_ms as f64 - observation.baseline_wall_ms as f64
                })
                .collect();
            let tool_deltas: Vec<f64> = observations
                .iter()
                .map(|observation| {
                    observation.candidate_tool_calls as f64 - observation.baseline_tool_calls as f64
                })
                .collect();

            PairedScenarioDelta {
                scenario_id,
                baseline_pass_rate,
                candidate_pass_rate,
                wall_ms_delta_median: RepeatedMetric::from_samples(wall_deltas).median,
                tool_call_delta_median: RepeatedMetric::from_samples(tool_deltas).median,
            }
        })
        .collect()
}

pub fn scenario_win_tie_loss(
    observations: &[PairedScenarioObservation],
) -> Vec<ScenarioWinTieLoss> {
    let mut grouped: BTreeMap<String, ScenarioWinTieLoss> = BTreeMap::new();
    for observation in observations {
        let entry = grouped
            .entry(observation.scenario_id.clone())
            .or_insert_with(|| ScenarioWinTieLoss {
                scenario_id: observation.scenario_id.clone(),
                wins: 0,
                ties: 0,
                losses: 0,
            });
        match (observation.baseline_passed, observation.candidate_passed) {
            (false, true) => entry.wins += 1,
            (true, false) => entry.losses += 1,
            _ => entry.ties += 1,
        }
    }
    grouped.into_values().collect()
}

pub fn bootstrap_pass_delta_ci95(
    observations: &[PairedScenarioObservation],
    seed: u64,
    resamples: usize,
) -> (f64, f64) {
    let deltas: Vec<f64> = observations
        .iter()
        .map(|observation| {
            f64::from(observation.candidate_passed as u8)
                - f64::from(observation.baseline_passed as u8)
        })
        .collect();
    let observed = mean(&deltas);
    if deltas.is_empty() || resamples == 0 {
        return (observed, observed);
    }

    let mut state = if seed == 0 { 0x9e3779b97f4a7c15 } else { seed };
    let mut bootstrap_means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut total = 0.0;
        for _ in 0..deltas.len() {
            let index = (next_random(&mut state) as usize) % deltas.len();
            total += deltas[index];
        }
        bootstrap_means.push(total / deltas.len() as f64);
    }
    bootstrap_means
        .sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    (
        percentile(&bootstrap_means, 0.025),
        percentile(&bootstrap_means, 0.975),
    )
}

pub fn deterministic_bootstrap_pass_delta_ci(
    observations: &[PairedScenarioObservation],
    seed: u64,
    resamples: usize,
) -> (f64, f64) {
    bootstrap_pass_delta_ci95(observations, seed, resamples)
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn median(sorted: &[f64]) -> f64 {
    match sorted.len() {
        0 => 0.0,
        length if length % 2 == 1 => sorted[length / 2],
        length => (sorted[length / 2 - 1] + sorted[length / 2]) / 2.0,
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64) * pct).floor() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        scenario_id: &str,
        repetition: u32,
        passed: bool,
        wall_ms: u64,
        tool_calls: u64,
    ) -> ScenarioRunSample {
        ScenarioRunSample {
            scenario_id: scenario_id.into(),
            repetition,
            passed,
            wall_ms,
            tool_calls,
        }
    }

    #[test]
    fn repeated_metric_is_sorted_and_deterministic() {
        let metric = RepeatedMetric::from_samples([4.0, 1.0, 3.0, 2.0]);
        assert_eq!(metric.samples, vec![4.0, 1.0, 3.0, 2.0]);
        assert_eq!(metric.mean, 2.5);
        assert_eq!(metric.median, 2.5);
        assert_eq!(metric.p10, 1.0);
        assert_eq!(metric.p90, 4.0);
    }

    #[test]
    fn pairing_requires_exact_scenario_repetition_keys() {
        let baseline = vec![sample("b", 0, true, 100, 2)];
        let candidate = vec![sample("b", 1, true, 110, 3)];
        let error = pair_by_scenario_repetition(&baseline, &candidate).unwrap_err();
        assert!(error.contains("missing scenario") || error.contains("counts differ"));
    }

    #[test]
    fn paired_delta_and_win_tie_loss_are_grouped_by_scenario() {
        let baseline = vec![
            sample("a", 0, false, 100, 2),
            sample("a", 1, true, 120, 4),
            sample("b", 0, true, 90, 1),
        ];
        let candidate = vec![
            sample("a", 0, true, 130, 3),
            sample("a", 1, true, 100, 2),
            sample("b", 0, false, 95, 1),
        ];
        let paired = pair_by_scenario_repetition(&baseline, &candidate).unwrap();
        let deltas = paired_scenario_deltas(&paired);
        assert_eq!(deltas[0].scenario_id, "a");
        assert_eq!(deltas[0].baseline_pass_rate, 0.5);
        assert_eq!(deltas[0].candidate_pass_rate, 1.0);
        assert_eq!(deltas[0].wall_ms_delta_median, 5.0);

        let outcomes = scenario_win_tie_loss(&paired);
        assert_eq!(outcomes[0].wins, 1);
        assert_eq!(outcomes[0].ties, 1);
        assert_eq!(outcomes[0].losses, 0);
        assert_eq!(outcomes[1].losses, 1);
    }

    #[test]
    fn bootstrap_ci_is_seeded_and_contains_observed_direction() {
        let observations = vec![
            PairedScenarioObservation {
                scenario_id: "a".into(),
                repetition: 0,
                baseline_passed: false,
                candidate_passed: true,
                baseline_wall_ms: 1,
                candidate_wall_ms: 1,
                baseline_tool_calls: 1,
                candidate_tool_calls: 1,
            },
            PairedScenarioObservation {
                scenario_id: "b".into(),
                repetition: 0,
                baseline_passed: true,
                candidate_passed: true,
                baseline_wall_ms: 1,
                candidate_wall_ms: 1,
                baseline_tool_calls: 1,
                candidate_tool_calls: 1,
            },
        ];
        let first = bootstrap_pass_delta_ci95(&observations, 42, 500);
        assert_eq!(first, bootstrap_pass_delta_ci95(&observations, 42, 500));
        assert!(first.0 >= 0.0);
        assert!(first.1 >= first.0);
    }
}
