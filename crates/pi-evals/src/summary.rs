//! TS `vendor/pi/packages/evals/src/vitest-evals/summary.ts`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessObservationOutcome {
    Scored,
    Unscored,
    Skipped,
    Pending,
    Errored,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessObservation {
    pub eval_set: String,
    pub group_key: String,
    pub test_name: String,
    pub file: String,
    pub harness: String,
    pub baseline: String,
    pub candidates: Vec<String>,
    pub repetition: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    pub outcome: HarnessObservationOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairedMetricSummary {
    pub total_pairs: usize,
    pub eligible_pairs: usize,
    pub baseline_mean: Option<f64>,
    pub candidate_mean: Option<f64>,
    pub mean_delta: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CorrectnessLiftSummary {
    pub total_pairs: usize,
    pub eligible_pairs: usize,
    pub baseline_pass_rate: Option<f64>,
    pub candidate_pass_rate: Option<f64>,
    pub lift: Option<f64>,
    pub baseline_wins: usize,
    pub candidate_wins: usize,
    pub ties: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessPairComparison {
    pub baseline: String,
    pub candidate: String,
    pub correctness: CorrectnessLiftSummary,
    pub total_tokens: PairedMetricSummary,
    pub total_ms: PairedMetricSummary,
    pub estimated_cost_usd: PairedMetricSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessComparisonReason {
    MissingObservation,
    DuplicateObservation,
    HarnessError,
    MissingScore,
    UnscorableOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessComparisonDiagnostic {
    pub eval_set: String,
    pub group_key: String,
    pub test_name: String,
    pub file: String,
    pub repetition: i64,
    pub harness: String,
    pub reason: HarnessComparisonReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessEvalSetReport {
    pub eval_set: String,
    pub comparisons: Vec<HarnessPairComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessComparisonReport {
    pub schema_version: u32,
    pub eval_sets: Vec<HarnessEvalSetReport>,
    pub diagnostics: Vec<HarnessComparisonDiagnostic>,
}

#[derive(Debug, Clone)]
struct HarnessDescriptor {
    name: String,
    index: usize,
}

#[derive(Debug, Clone)]
struct ObservationGroup {
    eval_set: String,
    group_key: String,
    test_name: String,
    file: String,
    repetition: i64,
    observations_by_harness: BTreeMap<String, Vec<HarnessObservation>>,
}

#[derive(Debug, Clone)]
struct EvalSetData {
    baseline: HarnessDescriptor,
    candidates_by_name: BTreeMap<String, HarnessDescriptor>,
    groups_by_key: BTreeMap<String, ObservationGroup>,
}

struct ObservationPair {
    baseline: HarnessObservation,
    candidate: HarnessObservation,
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

fn precise_difference(left: f64, right: f64) -> f64 {
    let raw = left - right;
    format!("{raw:.15e}").parse().unwrap_or(raw)
}

fn group_key(observation: &HarnessObservation) -> String {
    serde_json::json!([
        observation.file,
        observation.test_name,
        observation.group_key
    ])
    .to_string()
}

fn group_observations(observations: &[HarnessObservation]) -> BTreeMap<String, EvalSetData> {
    let mut eval_sets = BTreeMap::new();
    for observation in observations {
        let eval_set = eval_sets
            .entry(observation.eval_set.clone())
            .or_insert_with(|| EvalSetData {
                baseline: HarnessDescriptor {
                    name: observation.baseline.clone(),
                    index: 0,
                },
                candidates_by_name: BTreeMap::new(),
                groups_by_key: BTreeMap::new(),
            });
        for (index, name) in observation.candidates.iter().enumerate() {
            let existing = eval_set.candidates_by_name.get(name);
            if existing.is_none_or(|item| index < item.index) {
                eval_set.candidates_by_name.insert(
                    name.clone(),
                    HarnessDescriptor {
                        name: name.clone(),
                        index,
                    },
                );
            }
        }
        let group = eval_set
            .groups_by_key
            .entry(group_key(observation))
            .or_insert_with(|| ObservationGroup {
                eval_set: observation.eval_set.clone(),
                group_key: observation.group_key.clone(),
                test_name: observation.test_name.clone(),
                file: observation.file.clone(),
                repetition: observation.repetition,
                observations_by_harness: BTreeMap::new(),
            });
        group
            .observations_by_harness
            .entry(observation.harness.clone())
            .or_default()
            .push(observation.clone());
    }
    eval_sets
}

fn ordered_harnesses(eval_set: &EvalSetData) -> Vec<HarnessDescriptor> {
    let mut candidates: Vec<HarnessDescriptor> =
        eval_set.candidates_by_name.values().cloned().collect();
    candidates.sort_by(|left, right| {
        left.index
            .cmp(&right.index)
            .then(left.name.cmp(&right.name))
    });
    let mut harnesses = vec![eval_set.baseline.clone()];
    harnesses.extend(candidates);
    harnesses
}

fn ordered_candidates(eval_set: &EvalSetData) -> Vec<HarnessDescriptor> {
    let mut candidates: Vec<HarnessDescriptor> =
        eval_set.candidates_by_name.values().cloned().collect();
    candidates.sort_by(|left, right| {
        left.index
            .cmp(&right.index)
            .then(left.name.cmp(&right.name))
    });
    candidates
}

fn ordered_groups(eval_set: &EvalSetData) -> Vec<ObservationGroup> {
    let mut groups: Vec<ObservationGroup> = eval_set.groups_by_key.values().cloned().collect();
    groups.sort_by(|left, right| {
        left.group_key
            .cmp(&right.group_key)
            .then(left.repetition.cmp(&right.repetition))
    });
    groups
}

fn collect_diagnostics(
    harnesses: &[HarnessDescriptor],
    groups: &[ObservationGroup],
) -> Vec<HarnessComparisonDiagnostic> {
    let mut diagnostics = Vec::new();
    for group in groups {
        for harness in harnesses {
            let observations = group
                .observations_by_harness
                .get(&harness.name)
                .cloned()
                .unwrap_or_default();
            let reason = if observations.is_empty() {
                Some(HarnessComparisonReason::MissingObservation)
            } else if observations.len() > 1 {
                Some(HarnessComparisonReason::DuplicateObservation)
            } else if observations[0].outcome == HarnessObservationOutcome::Errored {
                Some(HarnessComparisonReason::HarnessError)
            } else if observations[0].outcome == HarnessObservationOutcome::Unscored {
                Some(HarnessComparisonReason::MissingScore)
            } else if observations[0].outcome != HarnessObservationOutcome::Scored {
                Some(HarnessComparisonReason::UnscorableOutcome)
            } else {
                None
            };
            if let Some(reason) = reason {
                diagnostics.push(HarnessComparisonDiagnostic {
                    eval_set: group.eval_set.clone(),
                    group_key: group.group_key.clone(),
                    test_name: group.test_name.clone(),
                    file: group.file.clone(),
                    repetition: group.repetition,
                    harness: harness.name.clone(),
                    reason,
                });
            }
        }
    }
    diagnostics
}

fn pair_observations(
    groups: &[ObservationGroup],
    baseline_harness: &str,
    candidate_harness: &str,
) -> Vec<ObservationPair> {
    let mut pairs = Vec::new();
    for group in groups {
        let baseline = group
            .observations_by_harness
            .get(baseline_harness)
            .cloned()
            .unwrap_or_default();
        let candidate = group
            .observations_by_harness
            .get(candidate_harness)
            .cloned()
            .unwrap_or_default();
        if baseline.len() == 1 && candidate.len() == 1 {
            pairs.push(ObservationPair {
                baseline: baseline[0].clone(),
                candidate: candidate[0].clone(),
            });
        }
    }
    pairs
}

fn summarize_metric(
    pairs: &[ObservationPair],
    select: impl Fn(&HarnessObservation) -> Option<f64>,
    total_pairs: usize,
) -> PairedMetricSummary {
    let mut baseline_values = Vec::new();
    let mut candidate_values = Vec::new();
    for pair in pairs {
        if pair.baseline.outcome != HarnessObservationOutcome::Scored
            || pair.candidate.outcome != HarnessObservationOutcome::Scored
        {
            continue;
        }
        let Some(baseline_value) = select(&pair.baseline) else {
            continue;
        };
        let Some(candidate_value) = select(&pair.candidate) else {
            continue;
        };
        if !baseline_value.is_finite() || !candidate_value.is_finite() {
            continue;
        }
        baseline_values.push(baseline_value);
        candidate_values.push(candidate_value);
    }
    let baseline_mean = mean(&baseline_values);
    let candidate_mean = mean(&candidate_values);
    PairedMetricSummary {
        total_pairs,
        eligible_pairs: baseline_values.len(),
        baseline_mean,
        candidate_mean,
        mean_delta: match (baseline_mean, candidate_mean) {
            (Some(baseline), Some(candidate)) => Some(precise_difference(candidate, baseline)),
            _ => None,
        },
    }
}

fn summarize_correctness(pairs: &[ObservationPair], total_pairs: usize) -> CorrectnessLiftSummary {
    let mut eligible_pairs = 0;
    let mut baseline_passes = 0;
    let mut candidate_passes = 0;
    let mut baseline_wins = 0;
    let mut candidate_wins = 0;
    let mut ties = 0;
    for pair in pairs {
        if pair.baseline.outcome != HarnessObservationOutcome::Scored
            || pair.candidate.outcome != HarnessObservationOutcome::Scored
        {
            continue;
        }
        eligible_pairs += 1;
        let baseline_passed = pair.baseline.score.unwrap_or(0.0) >= 1.0;
        let candidate_passed = pair.candidate.score.unwrap_or(0.0) >= 1.0;
        if baseline_passed {
            baseline_passes += 1;
        }
        if candidate_passed {
            candidate_passes += 1;
        }
        if baseline_passed == candidate_passed {
            ties += 1;
        } else if baseline_passed {
            baseline_wins += 1;
        } else {
            candidate_wins += 1;
        }
    }
    let baseline_pass_rate = if eligible_pairs == 0 {
        None
    } else {
        Some(baseline_passes as f64 / eligible_pairs as f64)
    };
    let candidate_pass_rate = if eligible_pairs == 0 {
        None
    } else {
        Some(candidate_passes as f64 / eligible_pairs as f64)
    };
    CorrectnessLiftSummary {
        total_pairs,
        eligible_pairs,
        baseline_pass_rate,
        candidate_pass_rate,
        lift: match (baseline_pass_rate, candidate_pass_rate) {
            (Some(baseline), Some(candidate)) => Some(precise_difference(candidate, baseline)),
            _ => None,
        },
        baseline_wins,
        candidate_wins,
        ties,
    }
}

fn compare_harnesses(
    baseline: &HarnessDescriptor,
    candidate: &HarnessDescriptor,
    groups: &[ObservationGroup],
) -> HarnessPairComparison {
    let pairs = pair_observations(groups, &baseline.name, &candidate.name);
    HarnessPairComparison {
        baseline: baseline.name.clone(),
        candidate: candidate.name.clone(),
        correctness: summarize_correctness(&pairs, groups.len()),
        total_tokens: summarize_metric(
            &pairs,
            |observation| observation.total_tokens,
            groups.len(),
        ),
        total_ms: summarize_metric(&pairs, |observation| observation.total_ms, groups.len()),
        estimated_cost_usd: summarize_metric(
            &pairs,
            |observation| observation.estimated_cost_usd,
            groups.len(),
        ),
    }
}

pub fn summarize_harness_comparisons(
    observations: &[HarnessObservation],
) -> HarnessComparisonReport {
    let mut eval_sets = Vec::new();
    let mut diagnostics = Vec::new();
    for (eval_set, data) in group_observations(observations) {
        let harnesses = ordered_harnesses(&data);
        let candidates = ordered_candidates(&data);
        let groups = ordered_groups(&data);
        eval_sets.push(HarnessEvalSetReport {
            eval_set,
            comparisons: candidates
                .iter()
                .map(|candidate| compare_harnesses(&data.baseline, candidate, &groups))
                .collect(),
        });
        diagnostics.extend(collect_diagnostics(&harnesses, &groups));
    }
    diagnostics.sort_by(|left, right| {
        left.eval_set
            .cmp(&right.eval_set)
            .then(left.file.cmp(&right.file))
            .then(left.group_key.cmp(&right.group_key))
            .then(left.repetition.cmp(&right.repetition))
            .then(left.harness.cmp(&right.harness))
    });
    HarnessComparisonReport {
        schema_version: 1,
        eval_sets,
        diagnostics,
    }
}

fn style(code: &str, text: &str) -> String {
    format!("\x1b[{code}m{text}\x1b[39m")
}

fn style_bold(text: &str) -> String {
    format!("\x1b[1m{text}\x1b[22m")
}

fn format_percentage(value: Option<f64>) -> String {
    match value {
        None => "unavailable".into(),
        Some(value) => format!("{:.1}%", value * 100.0),
    }
}

fn format_signed(value: f64, fraction_digits: usize) -> String {
    let sign = if value >= 0.0 { "+" } else { "" };
    format!("{sign}{value:.fraction_digits$}")
}

fn format_coverage(eligible_pairs: usize, total_pairs: usize) -> String {
    style("90", &format!("({eligible_pairs}/{total_pairs} pairs)"))
}

fn format_report_line(label: &str, value: &str) -> String {
    format!("    \x1b[90m{label:>9}\x1b[39m  {value}")
}

fn color_delta(value: f64, formatted: &str, positive_is_better: bool) -> String {
    if value == 0.0 {
        return style("90", formatted);
    }
    let improved = if positive_is_better {
        value > 0.0
    } else {
        value < 0.0
    };
    style(if improved { "32" } else { "31" }, formatted)
}

fn format_metric(
    label: &str,
    metric: &PairedMetricSummary,
    format_value: impl Fn(f64) -> String,
    format_delta: impl Fn(f64) -> String,
    comparison_pairs: usize,
) -> String {
    let coverage = if metric.eligible_pairs == 0 || metric.eligible_pairs == comparison_pairs {
        String::new()
    } else {
        format!(
            " {}",
            format_coverage(metric.eligible_pairs, metric.total_pairs)
        )
    };
    match (
        metric.baseline_mean,
        metric.candidate_mean,
        metric.mean_delta,
    ) {
        (Some(baseline), Some(candidate), Some(delta)) => {
            let colored = color_delta(delta, &format_delta(delta), false);
            let values = style(
                "90",
                &format!(
                    "(candidate {}, baseline {})",
                    format_value(candidate),
                    format_value(baseline)
                ),
            );
            format_report_line(label, &format!("{colored} {values}{coverage}"))
        }
        _ => format_report_line(label, &format!("{}{coverage}", style("33", "unavailable"))),
    }
}

pub fn format_harness_comparison_report(report: &HarnessComparisonReport) -> String {
    if report
        .eval_sets
        .iter()
        .all(|eval_set| eval_set.comparisons.is_empty())
    {
        return String::new();
    }
    let mut lines = vec![style_bold("Eval Comparisons")];
    for eval_set in &report.eval_sets {
        lines.push(format!("  {}", eval_set.eval_set));
        for (index, comparison) in eval_set.comparisons.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            let correctness = &comparison.correctness;
            lines.push(format_report_line("Baseline", &comparison.baseline));
            lines.push(format_report_line(
                "Candidate",
                &format!(
                    "{} {}",
                    comparison.candidate,
                    format_coverage(correctness.eligible_pairs, correctness.total_pairs)
                ),
            ));
            if let Some(lift) = correctness.lift {
                let lift_pp = lift * 100.0;
                let delta =
                    color_delta(lift_pp, &format!("{} pp", format_signed(lift_pp, 1)), true);
                let values = style(
                    "90",
                    &format!(
                        "(candidate {}, baseline {})",
                        format_percentage(correctness.candidate_pass_rate),
                        format_percentage(correctness.baseline_pass_rate)
                    ),
                );
                lines.push(format_report_line(
                    "Pass rate",
                    &format!("{delta} {values}"),
                ));
            } else {
                lines.push(format_report_line("Pass rate", &style("33", "unavailable")));
            }
            lines.push(format_metric(
                "Tokens",
                &comparison.total_tokens,
                |value| format!("{value:.1}"),
                |value| format_signed(value, 1),
                correctness.eligible_pairs,
            ));
            lines.push(format_metric(
                "Latency",
                &comparison.total_ms,
                |value| format!("{value:.1}ms"),
                |value| format!("{}ms", format_signed(value, 1)),
                correctness.eligible_pairs,
            ));
            lines.push(format_metric(
                "Est. cost",
                &comparison.estimated_cost_usd,
                |value| format!("${value:.4}"),
                |value| {
                    format!(
                        "{}${:.4}",
                        if value >= 0.0 { "+" } else { "-" },
                        value.abs()
                    )
                },
                correctness.eligible_pairs,
            ));
        }
    }
    if !report.diagnostics.is_empty() {
        lines.push(format!("  {}", style("33", "Incomplete observations")));
        for diagnostic in &report.diagnostics {
            let reason = match diagnostic.reason {
                HarnessComparisonReason::MissingObservation => "missing-observation",
                HarnessComparisonReason::DuplicateObservation => "duplicate-observation",
                HarnessComparisonReason::HarnessError => "harness-error",
                HarnessComparisonReason::MissingScore => "missing-score",
                HarnessComparisonReason::UnscorableOutcome => "unscorable-outcome",
            };
            lines.push(format!(
                "    {reason}: {}/{} repetition {}, harness {}",
                diagnostic.file, diagnostic.test_name, diagnostic.repetition, diagnostic.harness
            ));
        }
    }
    lines.join("\n")
}

pub fn strip_vt_control_characters(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        harness: &str,
        test_name: &str,
        result: &str,
        tokens: Option<f64>,
        ms: Option<f64>,
        cost: Option<f64>,
        baseline: &str,
        candidates: &[&str],
    ) -> HarnessObservation {
        let (outcome, score) = match result {
            "passed" => (HarnessObservationOutcome::Scored, Some(1.0)),
            "failed" => (HarnessObservationOutcome::Scored, Some(0.0)),
            "errored" => (HarnessObservationOutcome::Errored, None),
            "unscored" => (HarnessObservationOutcome::Unscored, None),
            "skipped" => (HarnessObservationOutcome::Skipped, None),
            "pending" => (HarnessObservationOutcome::Pending, None),
            other => panic!("unknown result {other}"),
        };
        HarnessObservation {
            eval_set: "tool access".into(),
            group_key: serde_json::json!([test_name, 1]).to_string(),
            test_name: test_name.into(),
            file: "src/tool-access.eval.ts".into(),
            harness: harness.into(),
            baseline: baseline.into(),
            candidates: candidates.iter().map(|name| (*name).to_string()).collect(),
            repetition: 1,
            total_tokens: tokens,
            total_ms: ms,
            estimated_cost_usd: cost,
            outcome,
            score,
        }
    }

    fn default_obs(
        harness: &str,
        test_name: &str,
        result: &str,
        tokens: Option<f64>,
        ms: Option<f64>,
        cost: Option<f64>,
    ) -> HarnessObservation {
        observation(
            harness,
            test_name,
            result,
            tokens,
            ms,
            cost,
            "without-tools",
            &["with-tools"],
        )
    }

    #[test]
    fn summarize_and_format_lock_ts_summary_tests() {
        let report = summarize_harness_comparisons(&[
            default_obs(
                "without-tools",
                "create",
                "failed",
                Some(100.0),
                Some(1000.0),
                Some(0.01),
            ),
            default_obs(
                "with-tools",
                "create",
                "passed",
                Some(120.0),
                Some(800.0),
                Some(0.02),
            ),
            default_obs(
                "without-tools",
                "inspect",
                "passed",
                Some(200.0),
                None,
                None,
            ),
            default_obs("with-tools", "inspect", "passed", Some(180.0), None, None),
        ]);
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.eval_sets.len(), 1);
        let comparison = &report.eval_sets[0].comparisons[0];
        assert_eq!(comparison.baseline, "without-tools");
        assert_eq!(comparison.candidate, "with-tools");
        assert_eq!(comparison.correctness.total_pairs, 2);
        assert_eq!(comparison.correctness.eligible_pairs, 2);
        assert_eq!(comparison.correctness.baseline_pass_rate, Some(0.5));
        assert_eq!(comparison.correctness.candidate_pass_rate, Some(1.0));
        assert_eq!(comparison.correctness.lift, Some(0.5));
        assert_eq!(comparison.correctness.baseline_wins, 0);
        assert_eq!(comparison.correctness.candidate_wins, 1);
        assert_eq!(comparison.correctness.ties, 1);
        assert_eq!(comparison.total_tokens.eligible_pairs, 2);
        assert_eq!(comparison.total_tokens.baseline_mean, Some(150.0));
        assert_eq!(comparison.total_tokens.candidate_mean, Some(150.0));
        assert_eq!(comparison.total_tokens.mean_delta, Some(0.0));
        assert_eq!(comparison.total_ms.eligible_pairs, 1);
        assert_eq!(comparison.total_ms.mean_delta, Some(-200.0));
        assert_eq!(comparison.estimated_cost_usd.mean_delta, Some(0.01));
        assert!(report.diagnostics.is_empty());

        let missing = summarize_harness_comparisons(&[
            default_obs("without-tools", "create", "failed", None, None, None),
            default_obs("with-tools", "create", "passed", None, None, None),
            default_obs("without-tools", "inspect", "passed", None, None, None),
        ]);
        let comparison = &missing.eval_sets[0].comparisons[0];
        assert_eq!(comparison.correctness.total_pairs, 2);
        assert_eq!(comparison.correctness.eligible_pairs, 1);
        assert_eq!(comparison.correctness.lift, Some(1.0));
        assert_eq!(comparison.total_tokens.eligible_pairs, 0);
        assert!(comparison.total_tokens.baseline_mean.is_none());
        assert!(missing.diagnostics.iter().any(|item| {
            item.test_name == "inspect"
                && item.harness == "with-tools"
                && item.reason == HarnessComparisonReason::MissingObservation
        }));

        let mut other_fail = default_obs("without-tools", "shared", "passed", None, None, None);
        other_fail.file = "src/other.eval.ts".into();
        let mut other_pass = default_obs("with-tools", "shared", "passed", None, None, None);
        other_pass.file = "src/other.eval.ts".into();
        let files = summarize_harness_comparisons(&[
            default_obs("without-tools", "shared", "failed", None, None, None),
            default_obs("with-tools", "shared", "passed", None, None, None),
            other_fail,
            other_pass,
        ]);
        assert_eq!(files.eval_sets[0].comparisons[0].correctness.total_pairs, 2);
        assert_eq!(
            files.eval_sets[0].comparisons[0].correctness.eligible_pairs,
            2
        );
        assert!(files.diagnostics.is_empty());

        let errored = summarize_harness_comparisons(&[
            default_obs(
                "without-tools",
                "create",
                "errored",
                Some(100.0),
                None,
                None,
            ),
            default_obs("with-tools", "create", "passed", Some(100.0), None, None),
        ]);
        assert_eq!(
            errored.eval_sets[0].comparisons[0]
                .correctness
                .eligible_pairs,
            0
        );
        assert_eq!(
            errored.eval_sets[0].comparisons[0]
                .total_tokens
                .eligible_pairs,
            0
        );
        assert!(errored.diagnostics.iter().any(|item| {
            item.harness == "without-tools" && item.reason == HarnessComparisonReason::HarnessError
        }));

        let unscored = summarize_harness_comparisons(&[
            default_obs("without-tools", "create", "unscored", None, None, None),
            default_obs("with-tools", "create", "unscored", None, None, None),
        ]);
        assert_eq!(
            unscored.eval_sets[0].comparisons[0]
                .correctness
                .eligible_pairs,
            0
        );
        assert_eq!(unscored.diagnostics.len(), 2);
        assert!(unscored
            .diagnostics
            .iter()
            .all(|item| item.reason == HarnessComparisonReason::MissingScore));

        let candidates = ["second", "third"];
        let multi = summarize_harness_comparisons(&[
            observation(
                "first",
                "input",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "second",
                "input",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "third",
                "input",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
        ]);
        let pairs: Vec<(&str, &str)> = multi.eval_sets[0]
            .comparisons
            .iter()
            .map(|item| (item.baseline.as_str(), item.candidate.as_str()))
            .collect();
        assert_eq!(pairs, vec![("first", "second"), ("first", "third")]);

        let retained = summarize_harness_comparisons(&[default_obs(
            "without-tools",
            "create",
            "failed",
            None,
            None,
            None,
        )]);
        assert_eq!(retained.eval_sets[0].comparisons.len(), 1);
        assert_eq!(
            retained.eval_sets[0].comparisons[0]
                .correctness
                .eligible_pairs,
            0
        );
        assert!(retained.diagnostics.iter().any(|item| {
            item.test_name == "create"
                && item.harness == "with-tools"
                && item.reason == HarnessComparisonReason::MissingObservation
        }));

        let mixed = summarize_harness_comparisons(&[
            observation(
                "first",
                "duplicate",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "first",
                "duplicate",
                "failed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "second",
                "duplicate",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "third",
                "duplicate",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "first",
                "skipped",
                "skipped",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "second",
                "skipped",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
            observation(
                "third",
                "skipped",
                "passed",
                None,
                None,
                None,
                "first",
                &candidates,
            ),
        ]);
        assert_eq!(
            mixed
                .diagnostics
                .iter()
                .filter(|item| item.reason == HarnessComparisonReason::DuplicateObservation)
                .count(),
            1
        );
        assert!(mixed.diagnostics.iter().any(|item| {
            item.reason == HarnessComparisonReason::DuplicateObservation
                && item.test_name == "duplicate"
                && item.harness == "first"
        }));
        assert!(mixed.diagnostics.iter().any(|item| {
            item.reason == HarnessComparisonReason::UnscorableOutcome
                && item.test_name == "skipped"
                && item.harness == "first"
        }));

        let formatted = summarize_harness_comparisons(&[
            default_obs(
                "without-tools",
                "create",
                "failed",
                None,
                Some(34853.7),
                None,
            ),
            default_obs("with-tools", "create", "passed", None, Some(30694.2), None),
        ]);
        let text = strip_vt_control_characters(&format_harness_comparison_report(&formatted));
        assert!(text.contains("Eval Comparisons"));
        assert!(text.contains(" Baseline  without-tools"));
        assert!(text.contains("Candidate  with-tools (1/1 pairs)"));
        assert!(text.contains("Pass rate  +100.0 pp (candidate 100.0%, baseline 0.0%)"));
        assert!(text.contains("   Tokens  unavailable"));
        assert!(text.contains("  Latency  -4159.5ms (candidate 30694.2ms, baseline 34853.7ms)"));
    }
}
