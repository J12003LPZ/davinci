//! Independent held-out runner. Labels never enter staged worker directories.

use super::corpus::{source_path, Corpus, Family, Severity, ValidatedCorpus, FAMILIES};
use super::{score, AdjudicatedFinding, CausalIdentity, SecurityScore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use uuid::Uuid;

pub const SCANNER_SCHEMA_VERSION: u32 = 2;
pub const METHODOLOGY_VERSION: &str = "native-security-2";
const MAX_CAPTURED_REPORT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunMeta {
    pub model: Option<String>,
    pub mode: String,
    pub analyzers_enabled: bool,
    pub counter_review_enabled: bool,
    pub authorized_provider_run: bool,
    #[serde(default)]
    pub effective_reasoning: Option<String>,
    #[serde(default)]
    pub source_digest: Option<String>,
    #[serde(default)]
    pub analyzer_versions: BTreeMap<String, String>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub model_turn_budget: Option<u64>,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub retry_count: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RawCaseRun {
    pub report: Value,
    pub coverage_complete: bool,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapturedCase {
    pub token: String,
    pub report_digest: String,
    pub report: Value,
    pub coverage_complete: bool,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub limitation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationCapture {
    pub schema_version: u32,
    pub corpus_id: String,
    pub scanner_schema_version: u32,
    pub methodology_version: String,
    pub meta: RunMeta,
    pub cases: Vec<CapturedCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdjudicationKey {
    pub schema_version: u32,
    pub corpus_id: String,
    pub capture_digest: String,
    pub bindings: BTreeMap<String, CaseBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseBinding {
    pub case_id: String,
    pub family: Family,
    pub vulnerable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseAdjudication {
    pub token: String,
    pub report_digest: String,
    pub findings: Vec<AdjudicatedFinding>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldOutRun {
    pub corpus_id: String,
    pub scanner_schema_version: u32,
    pub methodology_version: String,
    pub model: Option<String>,
    pub mode: String,
    pub analyzers_enabled: bool,
    pub counter_review_enabled: bool,
    pub authorized_provider_run: bool,
    pub effective_reasoning: Option<String>,
    pub source_digest: Option<String>,
    pub analyzer_versions: BTreeMap<String, String>,
    pub exclusions: Vec<String>,
    pub model_turn_budget: Option<u64>,
    pub token_budget: Option<u64>,
    pub retry_count: u64,
    pub cost_usd: Option<f64>,
    pub family_scores: BTreeMap<Family, SecurityScore>,
    pub overall: SecurityScore,
    pub cases_attempted: usize,
    pub cases_completed: usize,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub severity_assessed: usize,
    pub severity_within_band: usize,
    pub severity_calibration: Option<f64>,
    pub secure_high_critical_false_positives: usize,
    pub limitations: Vec<String>,
    pub capture_digest: Option<String>,
    attestation_digest: Option<String>,
    /// Quality gates stay unverifiable without an authorized held-out model run.
    pub blocking: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricRange {
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReproducibilityReport {
    pub schema_version: u32,
    pub run_count: usize,
    pub corpus_id: String,
    pub scanner_schema_version: u32,
    pub methodology_version: String,
    pub model: String,
    pub mode: String,
    pub effective_reasoning: String,
    pub source_digest: String,
    pub precision: Option<MetricRange>,
    pub recall: Option<MetricRange>,
    pub severity_calibration: Option<MetricRange>,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_retries: u64,
    pub total_cost_usd: Option<f64>,
    pub quality_gate_passed: bool,
    pub blocking: Option<String>,
}

/// Summarize at least three authorized runs made with one fixed configuration.
pub fn summarize_reproducibility(runs: &[HeldOutRun]) -> Result<ReproducibilityReport, String> {
    if runs.len() < 3 {
        return Err("reproducibility requires at least three runs".into());
    }
    let first = &runs[0];
    let model = first.model.clone().ok_or("evaluation run omitted model")?;
    let effective_reasoning = first
        .effective_reasoning
        .clone()
        .ok_or("evaluation run omitted effective reasoning")?;
    let source_digest = first
        .source_digest
        .clone()
        .ok_or("evaluation run omitted source digest")?;
    if !is_digest(&source_digest) {
        return Err("evaluation source digest is invalid".into());
    }
    let capture_digests: BTreeSet<_> = runs
        .iter()
        .filter_map(|run| run.capture_digest.as_deref())
        .collect();
    if capture_digests.len() != runs.len()
        || runs.iter().any(|run| {
            !run.authorized_provider_run
                || run.corpus_id != first.corpus_id
                || run.scanner_schema_version != first.scanner_schema_version
                || run.methodology_version != first.methodology_version
                || run.model.as_deref() != Some(model.as_str())
                || run.mode != first.mode
                || run.effective_reasoning.as_deref() != Some(effective_reasoning.as_str())
                || run.source_digest.as_deref() != Some(source_digest.as_str())
                || run.analyzers_enabled != first.analyzers_enabled
                || run.counter_review_enabled != first.counter_review_enabled
                || run.analyzer_versions != first.analyzer_versions
                || run.exclusions != first.exclusions
                || run.model_turn_budget != first.model_turn_budget
                || run.token_budget != first.token_budget
                || run
                    .cost_usd
                    .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
                || run.attestation_digest.as_deref() != run_attestation(run).ok().as_deref()
        })
    {
        return Err("reproducibility runs are unauthorized or not comparable".into());
    }
    let mut latencies: Vec<_> = runs.iter().map(|run| run.latency_ms).collect();
    latencies.sort_unstable();
    let total_cost_usd = runs
        .iter()
        .map(|run| run.cost_usd)
        .collect::<Option<Vec<_>>>()
        .map(|costs| costs.into_iter().sum());
    let quality_gate_passed = runs.iter().all(|run| run.blocking.is_none());
    Ok(ReproducibilityReport {
        schema_version: 1,
        run_count: runs.len(),
        corpus_id: first.corpus_id.clone(),
        scanner_schema_version: first.scanner_schema_version,
        methodology_version: first.methodology_version.clone(),
        model,
        mode: first.mode.clone(),
        effective_reasoning,
        source_digest,
        precision: metric_range(runs.iter().map(|run| run.overall.precision)),
        recall: metric_range(runs.iter().map(|run| run.overall.recall)),
        severity_calibration: metric_range(runs.iter().map(|run| run.severity_calibration)),
        latency_p50_ms: percentile(&latencies, 50),
        latency_p95_ms: percentile(&latencies, 95),
        total_input_tokens: runs
            .iter()
            .fold(0u64, |total, run| total.saturating_add(run.input_tokens)),
        total_output_tokens: runs
            .iter()
            .fold(0u64, |total, run| total.saturating_add(run.output_tokens)),
        total_retries: runs
            .iter()
            .fold(0u64, |total, run| total.saturating_add(run.retry_count)),
        total_cost_usd,
        quality_gate_passed,
        blocking: (!quality_gate_passed).then(|| "quality_thresholds_not_met".into()),
    })
}

fn metric_range(values: impl Iterator<Item = Option<f64>>) -> Option<MetricRange> {
    let values: Option<Vec<_>> = values.collect();
    let values = values?;
    let minimum = values.iter().copied().reduce(f64::min)?;
    let maximum = values.into_iter().reduce(f64::max)?;
    Some(MetricRange { minimum, maximum })
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    let rank = (percent * sorted.len()).div_ceil(100).saturating_sub(1);
    sorted[rank]
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
struct CaseRun {
    pub findings: Vec<AdjudicatedFinding>,
    pub coverage_complete: bool,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub fn stage_worker_case(root: &Path, files: &BTreeMap<String, String>) -> Result<(), String> {
    for (path, body) in files {
        source_path(path)?;
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err("worker path escaped staging directory".into());
        }
        let dest = root.join(relative);
        if !dest.starts_with(root) {
            return Err("worker path escaped staging directory".into());
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(dest, body).map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub fn evaluate(
    corpus: &Corpus,
    validated: &ValidatedCorpus<'_>,
    findings: &[AdjudicatedFinding],
    meta: RunMeta,
) -> HeldOutRun {
    let mut family_expected: BTreeMap<Family, Vec<CausalIdentity>> = FAMILIES
        .into_iter()
        .map(|family| (family, Vec::new()))
        .collect();
    for pair in &corpus.pairs {
        family_expected
            .get_mut(&pair.family)
            .expect("validated corpus families")
            .push(pair.expected.identity.clone());
    }
    let family_scores: BTreeMap<Family, SecurityScore> = family_expected
        .iter()
        .map(|(family, expected)| {
            let expected_set: BTreeSet<_> = expected.iter().cloned().collect();
            let scoped: Vec<_> = findings
                .iter()
                .filter(|finding| expected_set.contains(&finding.identity))
                .cloned()
                .collect();
            (
                *family,
                score(
                    expected,
                    &scoped,
                    validated.summary().pairs_by_family[family] >= 4,
                ),
            )
        })
        .collect();
    let overall_expected: Vec<_> = corpus
        .pairs
        .iter()
        .map(|pair| pair.expected.identity.clone())
        .collect();
    let overall = score(&overall_expected, findings, true);
    // Metrics without an exact raw-capture/adjudication binding cannot certify
    // release quality. `score_capture` is the only path that can clear this.
    let blocking = Some("unverifiable".into());
    HeldOutRun {
        corpus_id: corpus.id.clone(),
        scanner_schema_version: SCANNER_SCHEMA_VERSION,
        methodology_version: METHODOLOGY_VERSION.into(),
        model: meta.model,
        mode: meta.mode,
        analyzers_enabled: meta.analyzers_enabled,
        counter_review_enabled: meta.counter_review_enabled,
        authorized_provider_run: meta.authorized_provider_run,
        effective_reasoning: meta.effective_reasoning,
        source_digest: meta.source_digest,
        analyzer_versions: meta.analyzer_versions,
        exclusions: meta.exclusions,
        model_turn_budget: meta.model_turn_budget,
        token_budget: meta.token_budget,
        retry_count: meta.retry_count,
        cost_usd: meta.cost_usd,
        family_scores,
        overall,
        cases_attempted: 0,
        cases_completed: 0,
        latency_ms: 0,
        input_tokens: 0,
        output_tokens: 0,
        severity_assessed: 0,
        severity_within_band: 0,
        severity_calibration: None,
        secure_high_critical_false_positives: 0,
        limitations: Vec::new(),
        capture_digest: None,
        attestation_digest: None,
        blocking,
    }
}

/// Capture scanner output for independent adjudication without exposing labels
/// or vulnerable/secure identity to the scanner callback or public artifact.
pub fn capture_cases<F>(
    corpus: &Corpus,
    validated: &ValidatedCorpus<'_>,
    staging_root: &Path,
    meta: RunMeta,
    mut scan: F,
) -> Result<(EvaluationCapture, AdjudicationKey), String>
where
    F: FnMut(&Path) -> Result<RawCaseRun, String>,
{
    validate_run_meta(&meta)?;
    if staging_root.exists() {
        return Err("held-out staging root must not already exist".into());
    }
    std::fs::create_dir_all(staging_root).map_err(|err| err.to_string())?;
    let nonce = capture_nonce();
    let mut cases: Vec<_> = corpus
        .pairs
        .iter()
        .flat_map(|pair| {
            [
                (pair.family, true, &pair.vulnerable),
                (pair.family, false, &pair.secure),
            ]
        })
        .collect();
    cases.sort_by_key(|(_, _, case)| opaque_case_key(&nonce, &case.id));

    let mut captured = Vec::with_capacity(cases.len());
    let mut bindings = BTreeMap::new();
    let mut captured_bytes = 0usize;
    for (index, (family, vulnerable, case)) in cases.into_iter().enumerate() {
        let token = opaque_case_key(&nonce, &format!("token-{index}"));
        let directory = staging_root.join(opaque_case_key(&nonce, &format!("dir-{index}")));
        std::fs::create_dir(&directory).map_err(|err| err.to_string())?;
        stage_worker_case(
            &directory,
            validated
                .worker_files(&case.id)
                .ok_or("validated corpus omitted a case")?,
        )?;
        let mut result = match scan(&directory) {
            Ok(result) => result,
            Err(_) => RawCaseRun {
                report: Value::Null,
                coverage_complete: false,
                latency_ms: 0,
                input_tokens: 0,
                output_tokens: 0,
            },
        };
        let report_bytes = serde_json::to_vec(&result.report).map_err(|err| err.to_string())?;
        let oversized = report_bytes.len() > MAX_CAPTURED_REPORT_BYTES
            || captured_bytes.saturating_add(report_bytes.len()) > MAX_CAPTURE_BYTES;
        if oversized {
            result.report = Value::Null;
            result.coverage_complete = false;
        } else {
            captured_bytes += report_bytes.len();
        }
        let report_digest = value_digest(&result.report)?;
        captured.push(CapturedCase {
            token: token.clone(),
            report_digest,
            report: result.report,
            coverage_complete: result.coverage_complete,
            latency_ms: result.latency_ms,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            limitation: oversized
                .then(|| "scanner report exceeded evaluation capture limit".into())
                .or_else(|| (!result.coverage_complete).then(|| "scan incomplete".into())),
        });
        if bindings
            .insert(
                token,
                CaseBinding {
                    case_id: case.id.clone(),
                    family,
                    vulnerable,
                },
            )
            .is_some()
        {
            return Err("opaque case token collision".into());
        }
    }
    let capture = EvaluationCapture {
        schema_version: 1,
        corpus_id: corpus.id.clone(),
        scanner_schema_version: SCANNER_SCHEMA_VERSION,
        methodology_version: METHODOLOGY_VERSION.into(),
        meta,
        cases: captured,
    };
    let capture_digest = value_digest(
        &serde_json::to_value(&capture).map_err(|err| format!("encode capture: {err}"))?,
    )?;
    Ok((
        capture,
        AdjudicationKey {
            schema_version: 1,
            corpus_id: corpus.id.clone(),
            capture_digest,
            bindings,
        },
    ))
}

/// Score only independently adjudicated findings bound to the exact captured
/// report bytes. Missing, duplicate, extra or stale adjudications fail closed.
pub fn score_capture(
    corpus: &Corpus,
    validated: &ValidatedCorpus<'_>,
    capture: &EvaluationCapture,
    key: &AdjudicationKey,
    adjudications: &[CaseAdjudication],
) -> Result<HeldOutRun, String> {
    validate_run_meta(&capture.meta)?;
    validate_capture_bounds(capture)?;
    if capture.schema_version != 1
        || capture.corpus_id != corpus.id
        || capture.scanner_schema_version != SCANNER_SCHEMA_VERSION
        || capture.methodology_version != METHODOLOGY_VERSION
        || capture.cases.len() != validated.summary().cases
        || key.schema_version != 1
        || key.corpus_id != corpus.id
        || key.bindings.len() != capture.cases.len()
        || key.capture_digest
            != value_digest(
                &serde_json::to_value(capture).map_err(|err| format!("encode capture: {err}"))?,
            )?
    {
        return Err("evaluation capture identity mismatch".into());
    }
    let captured: BTreeMap<_, _> = capture
        .cases
        .iter()
        .map(|case| (case.token.as_str(), case))
        .collect();
    if captured.len() != capture.cases.len() || adjudications.len() != capture.cases.len() {
        return Err("every captured case requires one adjudication".into());
    }
    let mut seen = BTreeSet::new();
    let mut findings = Vec::new();
    let mut family_findings: BTreeMap<Family, Vec<AdjudicatedFinding>> = FAMILIES
        .into_iter()
        .map(|family| (family, Vec::new()))
        .collect();
    let mut severity_assessed = 0usize;
    let mut severity_within_band = 0usize;
    let mut secure_high_critical_false_positives = 0usize;
    for adjudication in adjudications {
        if !seen.insert(adjudication.token.as_str()) {
            return Err("duplicate case adjudication".into());
        }
        let case = captured
            .get(adjudication.token.as_str())
            .ok_or("adjudication references an unknown case")?;
        if adjudication.report_digest != case.report_digest
            || value_digest(&case.report)? != case.report_digest
        {
            return Err("adjudication report digest mismatch".into());
        }
        let binding = key
            .bindings
            .get(&adjudication.token)
            .ok_or("adjudication key omitted a case")?;
        let pair = corpus
            .pairs
            .iter()
            .find(|pair| {
                let case = if binding.vulnerable {
                    &pair.vulnerable
                } else {
                    &pair.secure
                };
                case.id == binding.case_id && pair.family == binding.family
            })
            .ok_or("adjudication key references an unknown corpus case")?;
        if binding.vulnerable {
            if let Some(severity) = adjudication
                .findings
                .iter()
                .find(|finding| {
                    finding.confirmed
                        && finding.evidence_valid
                        && finding.gates_satisfied
                        && finding.identity == pair.expected.identity
                })
                .and_then(|finding| finding.severity)
            {
                severity_assessed += 1;
                severity_within_band += usize::from(
                    severity >= pair.expected.minimum_severity
                        && severity <= pair.expected.maximum_severity,
                );
            }
        } else {
            secure_high_critical_false_positives += adjudication
                .findings
                .iter()
                .filter(|finding| {
                    finding.confirmed
                        && matches!(finding.severity, Some(Severity::High | Severity::Critical))
                })
                .count();
        }
        family_findings
            .get_mut(&binding.family)
            .expect("validated corpus family")
            .extend(adjudication.findings.iter().cloned());
        findings.extend(adjudication.findings.iter().cloned());
    }

    let coverage_complete = capture.cases.iter().all(|case| case.coverage_complete);
    let mut run = evaluate(corpus, validated, &findings, capture.meta.clone());
    for (family, family_score) in &mut run.family_scores {
        let expected: Vec<_> = corpus
            .pairs
            .iter()
            .filter(|pair| pair.family == *family)
            .map(|pair| pair.expected.identity.clone())
            .collect();
        *family_score = score(
            &expected,
            family_findings
                .get(family)
                .expect("validated corpus family findings"),
            coverage_complete,
        );
    }
    run.overall.coverage_complete &= coverage_complete;
    run.overall.confirmed_thresholds_met &= coverage_complete;
    run.cases_attempted = capture.cases.len();
    run.cases_completed = capture
        .cases
        .iter()
        .filter(|case| case.coverage_complete)
        .count();
    run.latency_ms = capture
        .cases
        .iter()
        .fold(0, |total, case| total.saturating_add(case.latency_ms));
    run.input_tokens = capture
        .cases
        .iter()
        .fold(0, |total, case| total.saturating_add(case.input_tokens));
    run.output_tokens = capture
        .cases
        .iter()
        .fold(0, |total, case| total.saturating_add(case.output_tokens));
    run.severity_assessed = severity_assessed;
    run.severity_within_band = severity_within_band;
    run.severity_calibration =
        (severity_assessed > 0).then(|| severity_within_band as f64 / severity_assessed as f64);
    run.secure_high_critical_false_positives = secure_high_critical_false_positives;
    run.limitations = capture
        .cases
        .iter()
        .filter_map(|case| case.limitation.clone())
        .collect();
    let family_thresholds_met = run
        .family_scores
        .values()
        .all(|family_score| family_score.confirmed_thresholds_met);
    if !capture.meta.authorized_provider_run || !coverage_complete {
        run.blocking = Some("unverifiable".into());
    } else if !run.overall.confirmed_thresholds_met
        || !family_thresholds_met
        || severity_assessed != run.overall.true_positives
        || run.severity_calibration.is_none_or(|rate| rate < 0.85)
        || secure_high_critical_false_positives > 0
    {
        run.blocking = Some("quality_thresholds_not_met".into());
    } else {
        run.blocking = None;
    }
    run.capture_digest = Some(key.capture_digest.clone());
    run.attestation_digest = Some(run_attestation(&run)?);
    Ok(run)
}

fn capture_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

fn value_digest(value: &Value) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn run_attestation(run: &HeldOutRun) -> Result<String, String> {
    let mut unsigned = run.clone();
    unsigned.attestation_digest = None;
    value_digest(&serde_json::to_value(unsigned).map_err(|err| err.to_string())?)
}

fn validate_run_meta(meta: &RunMeta) -> Result<(), String> {
    let bounded = |value: &str| !value.trim().is_empty() && value.len() <= 256;
    if !bounded(&meta.mode)
        || meta.model.as_deref().is_some_and(|value| !bounded(value))
        || meta
            .effective_reasoning
            .as_deref()
            .is_some_and(|value| !bounded(value))
        || meta
            .source_digest
            .as_deref()
            .is_some_and(|value| !is_digest(value))
        || meta.analyzer_versions.len() > 32
        || meta
            .analyzer_versions
            .iter()
            .any(|(name, version)| !bounded(name) || !bounded(version))
        || meta.exclusions.len() > 10_000
        || meta.exclusions.iter().any(|value| value.len() > 4096)
        || meta.model_turn_budget == Some(0)
        || meta.token_budget == Some(0)
        || meta
            .cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
    {
        return Err("invalid evaluation run metadata".into());
    }
    if meta.authorized_provider_run
        && (meta.model.is_none()
            || meta.effective_reasoning.is_none()
            || meta.source_digest.is_none()
            || meta.model_turn_budget.is_none()
            || meta.token_budget.is_none())
    {
        return Err("authorized evaluation run omitted required provenance".into());
    }
    Ok(())
}

fn validate_capture_bounds(capture: &EvaluationCapture) -> Result<(), String> {
    let mut total = 0usize;
    for case in &capture.cases {
        let bytes = serde_json::to_vec(&case.report).map_err(|err| err.to_string())?;
        if bytes.len() > MAX_CAPTURED_REPORT_BYTES {
            return Err("captured scanner report exceeds per-case limit".into());
        }
        total = total
            .checked_add(bytes.len())
            .ok_or("evaluation capture byte count overflow")?;
        if total > MAX_CAPTURE_BYTES {
            return Err("evaluation capture exceeds aggregate limit".into());
        }
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Run every held-out case from an opaque, source-only staging directory.
///
/// The callback is the only scanner adapter. It receives neither case IDs,
/// vulnerable/secure labels, families, ground truth nor control rationales.
/// A failed or incomplete case remains a coverage failure instead of being
/// dropped from the score denominator.
#[cfg(test)]
fn run_cases<F>(
    corpus: &Corpus,
    validated: &ValidatedCorpus<'_>,
    staging_root: &Path,
    meta: RunMeta,
    mut scan: F,
) -> Result<HeldOutRun, String>
where
    F: FnMut(&Path) -> Result<CaseRun, String>,
{
    validate_run_meta(&meta)?;
    if staging_root.exists() {
        return Err("held-out staging root must not already exist".into());
    }
    std::fs::create_dir_all(staging_root).map_err(|err| err.to_string())?;

    let nonce = capture_nonce();
    let mut cases: Vec<_> = corpus
        .pairs
        .iter()
        .flat_map(|pair| [(pair.family, &pair.vulnerable), (pair.family, &pair.secure)])
        .collect();
    cases.sort_by_key(|(_, case)| opaque_case_key(&nonce, &case.id));

    let mut findings = Vec::new();
    let mut family_findings: BTreeMap<Family, Vec<AdjudicatedFinding>> = FAMILIES
        .into_iter()
        .map(|family| (family, Vec::new()))
        .collect();
    let mut completed = 0usize;
    let mut latency_ms = 0u64;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut limitations = Vec::new();
    for (index, (family, case)) in cases.iter().enumerate() {
        let files = validated
            .worker_files(&case.id)
            .ok_or("validated corpus omitted a case")?;
        let directory = staging_root.join(opaque_case_key(&nonce, &index.to_string()));
        std::fs::create_dir(&directory).map_err(|err| err.to_string())?;
        stage_worker_case(&directory, files)?;
        match scan(&directory) {
            Ok(result) => {
                completed += usize::from(result.coverage_complete);
                latency_ms = latency_ms.saturating_add(result.latency_ms);
                input_tokens = input_tokens.saturating_add(result.input_tokens);
                output_tokens = output_tokens.saturating_add(result.output_tokens);
                family_findings
                    .get_mut(family)
                    .expect("validated corpus family")
                    .extend(result.findings.iter().cloned());
                findings.extend(result.findings);
                if !result.coverage_complete {
                    limitations.push(format!("case {} coverage incomplete", index + 1));
                }
            }
            Err(error) => limitations.push(format!("case {} failed: {error}", index + 1)),
        }
    }

    let coverage_complete = completed == cases.len();
    let mut run = evaluate(corpus, validated, &findings, meta);
    for (family, family_score) in &mut run.family_scores {
        let expected: Vec<_> = corpus
            .pairs
            .iter()
            .filter(|pair| pair.family == *family)
            .map(|pair| pair.expected.identity.clone())
            .collect();
        *family_score = score(
            &expected,
            family_findings
                .get(family)
                .expect("validated corpus family findings"),
            coverage_complete,
        );
    }
    run.overall.coverage_complete &= coverage_complete;
    run.overall.confirmed_thresholds_met &= coverage_complete;
    run.cases_attempted = cases.len();
    run.cases_completed = completed;
    run.latency_ms = latency_ms;
    run.input_tokens = input_tokens;
    run.output_tokens = output_tokens;
    run.limitations = limitations;
    if !coverage_complete {
        run.blocking = Some("unverifiable".into());
    }
    Ok(run)
}

fn opaque_case_key(nonce: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(format!("{nonce}\0{value}").as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security_eval::held_out;
    use crate::security_eval::AdjudicatedFinding;

    fn meta(authorized: bool) -> RunMeta {
        RunMeta {
            model: Some("fixture-none".into()),
            mode: "standard".into(),
            analyzers_enabled: false,
            counter_review_enabled: false,
            authorized_provider_run: authorized,
            effective_reasoning: Some("medium".into()),
            source_digest: Some("a".repeat(64)),
            analyzer_versions: BTreeMap::new(),
            exclusions: Vec::new(),
            model_turn_budget: Some(24),
            token_budget: Some(100_000),
            retry_count: 0,
            cost_usd: Some(0.0),
        }
    }

    #[test]
    fn security_eval_runner_stages_source_without_evaluator_metadata() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let root = tempfile::tempdir().unwrap();
        let files = validated
            .worker_files(&corpus.pairs[0].vulnerable.id)
            .unwrap();
        stage_worker_case(root.path(), files).unwrap();
        let staged = std::fs::read_to_string(root.path().join("handler.rs")).unwrap();
        assert!(root.path().join("routes.rs").exists());
        assert!(!root.path().join("labels.json").exists());
        assert!(!staged.contains("owner predicate on document lookup"));
        assert!(
            stage_worker_case(root.path(), &[("../escape.rs".into(), "x".into())].into()).is_err()
        );
    }

    #[test]
    fn security_eval_separates_fixture_plumbing_from_live_quality() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let run = evaluate(&corpus, &validated, &[], meta(false));
        assert_eq!(run.blocking.as_deref(), Some("unverifiable"));
        assert!(!run.authorized_provider_run);
        assert_eq!(run.overall.true_positives, 0);
        assert_eq!(run.overall.false_negatives, 40);
        assert_eq!(run.overall.recall, Some(0.0));
        assert!(run.overall.precision.is_none());
        assert!(!run.overall.confirmed_thresholds_met);
        let encoded = serde_json::to_value(&run).unwrap();
        assert!(encoded.get("releaseGatePassed").is_none());
    }

    #[test]
    fn security_eval_metric_helper_cannot_bypass_capture_adjudication_binding() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let findings: Vec<_> = corpus
            .pairs
            .iter()
            .map(|pair| AdjudicatedFinding {
                identity: pair.expected.identity.clone(),
                confirmed: true,
                evidence_valid: true,
                severity: Some(pair.expected.minimum_severity),
                gates_satisfied: true,
            })
            .collect();
        let run = evaluate(&corpus, &validated, &findings, meta(true));
        assert!(run.overall.confirmed_thresholds_met);
        assert_eq!(run.blocking.as_deref(), Some("unverifiable"));
    }

    #[test]
    fn security_eval_preserves_per_category_metrics() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let run = evaluate(&corpus, &validated, &[], meta(false));
        assert_eq!(run.family_scores.len(), 10);
        for family in FAMILIES {
            let family_score = &run.family_scores[&family];
            assert_eq!(family_score.false_negatives, 4);
            assert!(family_score.coverage_complete);
        }
    }

    #[test]
    fn security_eval_records_model_and_methodology_versions() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let run = evaluate(&corpus, &validated, &[], meta(false));
        assert_eq!(run.methodology_version, METHODOLOGY_VERSION);
        assert_eq!(run.scanner_schema_version, SCANNER_SCHEMA_VERSION);
        assert_eq!(run.model.as_deref(), Some("fixture-none"));
        assert_eq!(run.mode, "standard");
        assert!(!run.analyzers_enabled);
        assert!(!run.counter_review_enabled);
    }

    #[test]
    fn security_eval_reproducibility_requires_three_comparable_authorized_runs() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let mut runs = Vec::new();
        for (index, (latency, precision, recall, cost)) in [
            (100, 0.90, 0.80, 1.0),
            (300, 0.95, 0.85, 1.5),
            (200, 1.00, 0.90, 2.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut run = evaluate(&corpus, &validated, &[], meta(true));
            run.overall.precision = Some(precision);
            run.overall.recall = Some(recall);
            run.severity_calibration = Some(0.9);
            run.latency_ms = latency;
            run.input_tokens = 10;
            run.output_tokens = 5;
            run.retry_count = 1;
            run.cost_usd = Some(cost);
            run.capture_digest = Some(format!("{:064x}", index + 1));
            run.attestation_digest = Some(run_attestation(&run).unwrap());
            runs.push(run);
        }
        assert!(summarize_reproducibility(&runs[..2]).is_err());
        let report = summarize_reproducibility(&runs).unwrap();
        assert_eq!(report.run_count, 3);
        assert_eq!((report.latency_p50_ms, report.latency_p95_ms), (200, 300));
        assert_eq!(
            (report.total_input_tokens, report.total_output_tokens),
            (30, 15)
        );
        assert_eq!(report.total_retries, 3);
        assert_eq!(report.total_cost_usd, Some(4.5));
        assert_eq!(report.precision.unwrap().minimum, 0.90);
        assert!(!report.quality_gate_passed);

        let mut mismatched = runs;
        mismatched[2].source_digest = Some("b".repeat(64));
        assert!(summarize_reproducibility(&mismatched).is_err());
    }

    #[test]
    fn security_eval_scores_root_cause_not_title_substrings() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let finding = AdjudicatedFinding {
            identity: CausalIdentity {
                control: "owner predicate on document lookup".into(),
                trust_boundary: "unrelated".into(),
                sink: "document body return".into(),
                occurrence: "GET document by id".into(),
            },
            confirmed: true,
            evidence_valid: true,
            severity: None,
            gates_satisfied: true,
        };
        let run = evaluate(&corpus, &validated, &[finding], meta(false));
        assert_eq!(run.overall.true_positives, 0);
        assert_eq!(run.overall.false_positives, 1);
    }

    #[test]
    fn security_eval_counts_deferred_positive_as_miss_for_recall() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let finding = AdjudicatedFinding {
            identity: corpus.pairs[0].expected.identity.clone(),
            confirmed: false,
            evidence_valid: true,
            severity: None,
            gates_satisfied: false,
        };
        let run = evaluate(&corpus, &validated, &[finding], meta(false));
        assert_eq!(run.overall.true_positives, 0);
        assert_eq!(run.overall.false_negatives, 40);
        assert_eq!(run.overall.recall, Some(0.0));
    }

    #[test]
    fn security_eval_runner_executes_every_label_free_case_and_accounts_usage() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("held-out-run");
        let mut calls = 0usize;
        let run = run_cases(&corpus, &validated, &root, meta(true), |case_root| {
            calls += 1;
            assert!(case_root.file_name().unwrap().to_string_lossy().len() == 64);
            assert!(!case_root.join("labels.json").exists());
            Ok(CaseRun {
                findings: (calls == 1)
                    .then(|| AdjudicatedFinding {
                        identity: CausalIdentity {
                            control: "unmatched control".into(),
                            trust_boundary: "unmatched boundary".into(),
                            sink: "unmatched sink".into(),
                            occurrence: "unmatched occurrence".into(),
                        },
                        confirmed: true,
                        evidence_valid: true,
                        severity: None,
                        gates_satisfied: true,
                    })
                    .into_iter()
                    .collect(),
                coverage_complete: true,
                latency_ms: 2,
                input_tokens: 3,
                output_tokens: 1,
            })
        })
        .unwrap();
        assert_eq!(calls, 80);
        assert_eq!((run.cases_attempted, run.cases_completed), (80, 80));
        assert_eq!(
            (run.latency_ms, run.input_tokens, run.output_tokens),
            (160, 240, 80)
        );
        assert!(run.limitations.is_empty());
        assert_eq!(run.overall.false_negatives, 40);
        assert_eq!(run.overall.false_positives, 1);
        assert_eq!(run.blocking.as_deref(), Some("unverifiable"));
        assert_eq!(
            run.family_scores
                .values()
                .map(|score| score.false_positives)
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn security_eval_runner_keeps_failed_and_incomplete_cases_blocking() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("partial-run");
        let mut calls = 0usize;
        let run = run_cases(&corpus, &validated, &root, meta(true), |_| {
            calls += 1;
            if calls == 1 {
                Err("scanner unavailable".into())
            } else {
                Ok(CaseRun {
                    coverage_complete: calls != 2,
                    ..Default::default()
                })
            }
        })
        .unwrap();
        assert_eq!(calls, 80);
        assert_eq!((run.cases_attempted, run.cases_completed), (80, 78));
        assert_eq!(run.blocking.as_deref(), Some("unverifiable"));
        assert!(!run.overall.coverage_complete);
        assert!(!run.overall.confirmed_thresholds_met);
        assert_eq!(run.limitations.len(), 2);
    }

    #[test]
    fn security_eval_runner_refuses_a_preexisting_staging_root() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(
            run_cases(&corpus, &validated, root.path(), meta(false), |_| {
                Ok(CaseRun::default())
            })
            .is_err()
        );
    }

    #[test]
    fn security_eval_capture_is_label_free_and_binds_every_raw_report() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let (capture, key) = capture_cases(
            &corpus,
            &validated,
            &scratch.path().join("capture"),
            meta(true),
            |case_root| {
                assert!(case_root.file_name().unwrap().to_string_lossy().len() == 64);
                Ok(RawCaseRun {
                    report: serde_json::json!({"schemaVersion": 2, "findings": []}),
                    coverage_complete: true,
                    latency_ms: 1,
                    input_tokens: 2,
                    output_tokens: 3,
                })
            },
        )
        .unwrap();
        assert_eq!(capture.cases.len(), 80);
        assert_eq!(key.bindings.len(), 80);
        let public = serde_json::to_string(&capture).unwrap();
        assert!(!public.contains(&corpus.pairs[0].vulnerable.id));
        assert!(!public.contains(&corpus.pairs[0].secure.id));
        assert!(!public.contains(&corpus.pairs[0].secure_control_rationale));
        assert!(!public.contains(&corpus.pairs[0].expected.identity.control));
        assert!(capture
            .cases
            .iter()
            .all(|case| case.token.len() == 64 && case.report_digest.len() == 64));
        let reopened_capture: EvaluationCapture = serde_json::from_str(&public).unwrap();
        let private = serde_json::to_string(&key).unwrap();
        let reopened_key: AdjudicationKey = serde_json::from_str(&private).unwrap();
        assert_eq!(reopened_capture.cases.len(), capture.cases.len());
        assert_eq!(reopened_key.capture_digest, key.capture_digest);
        assert!(reopened_key
            .bindings
            .values()
            .any(|binding| binding.vulnerable));
        assert!(reopened_key
            .bindings
            .values()
            .any(|binding| !binding.vulnerable));
    }

    #[test]
    fn security_eval_capture_bounds_untrusted_raw_reports() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let mut calls = 0usize;
        let (capture, _) = capture_cases(
            &corpus,
            &validated,
            &scratch.path().join("capture"),
            meta(true),
            |_| {
                calls += 1;
                Ok(RawCaseRun {
                    report: if calls == 1 {
                        Value::String("x".repeat(MAX_CAPTURED_REPORT_BYTES + 1))
                    } else {
                        serde_json::json!({"schemaVersion": 2})
                    },
                    coverage_complete: true,
                    latency_ms: 1,
                    input_tokens: 1,
                    output_tokens: 1,
                })
            },
        )
        .unwrap();
        assert_eq!(calls, 80);
        assert_eq!(
            capture
                .cases
                .iter()
                .filter(|case| !case.coverage_complete)
                .count(),
            1
        );
        assert!(capture.cases.iter().any(|case| {
            case.report.is_null()
                && case.limitation.as_deref()
                    == Some("scanner report exceeded evaluation capture limit")
        }));
    }

    #[test]
    fn security_eval_adjudication_requires_exact_complete_capture_binding() {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let (capture, key) = capture_cases(
            &corpus,
            &validated,
            &scratch.path().join("capture"),
            meta(true),
            |_| {
                Ok(RawCaseRun {
                    report: serde_json::json!({"schemaVersion": 2, "findings": []}),
                    coverage_complete: true,
                    latency_ms: 1,
                    input_tokens: 2,
                    output_tokens: 3,
                })
            },
        )
        .unwrap();
        let adjudications: Vec<_> = capture
            .cases
            .iter()
            .map(|case| CaseAdjudication {
                token: case.token.clone(),
                report_digest: case.report_digest.clone(),
                findings: Vec::new(),
            })
            .collect();
        let run = score_capture(&corpus, &validated, &capture, &key, &adjudications).unwrap();
        assert_eq!((run.cases_attempted, run.cases_completed), (80, 80));
        assert_eq!(
            (run.latency_ms, run.input_tokens, run.output_tokens),
            (80, 160, 240)
        );
        assert_eq!(run.blocking.as_deref(), Some("quality_thresholds_not_met"));

        assert!(score_capture(&corpus, &validated, &capture, &key, &adjudications[..79]).is_err());
        let mut tampered_capture = capture.clone();
        tampered_capture.meta.model = Some("tampered".into());
        assert!(
            score_capture(&corpus, &validated, &tampered_capture, &key, &adjudications).is_err()
        );
        let mut stale = adjudications;
        stale[0].report_digest = "0".repeat(64);
        assert!(score_capture(&corpus, &validated, &capture, &key, &stale).is_err());
    }

    #[test]
    fn security_eval_release_gate_requires_calibrated_severity_and_no_severe_secure_false_positive()
    {
        let (corpus, sources) = held_out::corpus();
        let validated = corpus.validate(&sources).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let (capture, key) = capture_cases(
            &corpus,
            &validated,
            &scratch.path().join("capture"),
            meta(true),
            |_| {
                Ok(RawCaseRun {
                    report: serde_json::json!({"schemaVersion": 2}),
                    coverage_complete: true,
                    latency_ms: 1,
                    input_tokens: 1,
                    output_tokens: 1,
                })
            },
        )
        .unwrap();
        let mut adjudications: Vec<_> = capture
            .cases
            .iter()
            .map(|case| {
                let binding = &key.bindings[&case.token];
                let pair = corpus
                    .pairs
                    .iter()
                    .find(|pair| {
                        pair.vulnerable.id == binding.case_id || pair.secure.id == binding.case_id
                    })
                    .unwrap();
                CaseAdjudication {
                    token: case.token.clone(),
                    report_digest: case.report_digest.clone(),
                    findings: binding
                        .vulnerable
                        .then(|| AdjudicatedFinding {
                            identity: pair.expected.identity.clone(),
                            confirmed: true,
                            evidence_valid: true,
                            severity: Some(pair.expected.minimum_severity),
                            gates_satisfied: true,
                        })
                        .into_iter()
                        .collect(),
                }
            })
            .collect();
        let run = score_capture(&corpus, &validated, &capture, &key, &adjudications).unwrap();
        assert_eq!((run.severity_assessed, run.severity_within_band), (40, 40));
        assert_eq!(run.severity_calibration, Some(1.0));
        assert_eq!(run.secure_high_critical_false_positives, 0);
        assert_eq!(run.blocking, None);

        let secure = adjudications
            .iter_mut()
            .find(|item| !key.bindings[&item.token].vulnerable)
            .unwrap();
        secure.findings.push(AdjudicatedFinding {
            identity: CausalIdentity {
                control: "false positive".into(),
                trust_boundary: "secure case".into(),
                sink: "none".into(),
                occurrence: "none".into(),
            },
            confirmed: true,
            evidence_valid: true,
            severity: Some(Severity::High),
            gates_satisfied: true,
        });
        let run = score_capture(&corpus, &validated, &capture, &key, &adjudications).unwrap();
        assert_eq!(run.secure_high_critical_false_positives, 1);
        assert_eq!(run.blocking.as_deref(), Some("quality_thresholds_not_met"));
    }
}
