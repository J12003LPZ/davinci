//! Deterministic OpenAI cache benchmark contracts.
//!
//! This module contains no provider client. Offline fixtures are the default.
//! A future live runner must first pass `validate_live_authorization` with an
//! explicit provider/model-matched request and cost cap.

use crate::codex_eval::{CodexBenchmarkRunMetrics, RunOutcome};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const OPENAI_CACHE_EVAL_SCHEMA_VERSION: u32 = 1;
pub const OPENAI_CACHE_LIVE_AUTHORIZATION_ENV: &str =
    "DAVINCI_OPENAI_CACHE_LIVE_AUTHORIZATION";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCacheFeature {
    ExplicitStableBoundary,
    NativeResponsesReplay,
    WorkerBootstrapAffinity,
    CacheDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkSource {
    OfflineFixture,
    LiveAuthorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkProfile {
    Baseline,
    Treatment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCacheBenchmarkCase {
    pub id: String,
    pub class: String,
    pub fixture: String,
    pub expected_outcome: RunOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveBenchmarkAuthorization {
    pub approval_id: String,
    pub provider: String,
    pub model: String,
    pub max_requests: u32,
    pub max_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCacheBenchmarkManifest {
    pub schema_version: u32,
    pub revision: String,
    pub provider: String,
    pub model: String,
    pub endpoint_family: String,
    pub policy_contract_revision: String,
    pub feature: OpenAiCacheFeature,
    pub baseline_profile: String,
    pub treatment_profile: String,
    pub repetitions: u32,
    pub cases: Vec<OpenAiCacheBenchmarkCase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_authorization: Option<LiveBenchmarkAuthorization>,
}

impl OpenAiCacheBenchmarkManifest {
    pub fn fingerprint(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("benchmark manifest is JSON");
        format!("{:x}", Sha256::digest(bytes))
    }

    pub fn validate_offline(&self) -> Result<(), String> {
        self.validate_common()?;
        if self.live_authorization.is_some() {
            return Err(
                "offline manifest must not embed live provider authorization".into(),
            );
        }
        Ok(())
    }

    fn validate_common(&self) -> Result<(), String> {
        if self.schema_version != OPENAI_CACHE_EVAL_SCHEMA_VERSION {
            return Err(format!(
                "unsupported OpenAI cache eval schema {}",
                self.schema_version
            ));
        }
        if !is_git_revision(&self.revision) {
            return Err("benchmark revision must be an exact 40-hex git commit".into());
        }
        for (label, value) in [
            ("provider", self.provider.as_str()),
            ("model", self.model.as_str()),
            ("endpoint family", self.endpoint_family.as_str()),
            ("policy contract revision", self.policy_contract_revision.as_str()),
            ("baseline profile", self.baseline_profile.as_str()),
            ("treatment profile", self.treatment_profile.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{label} must be explicit"));
            }
        }
        if self.repetitions == 0 {
            return Err("benchmark repetitions must be at least one".into());
        }
        if self.cases.is_empty() {
            return Err("benchmark manifest must contain at least one case".into());
        }
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if case.id.trim().is_empty()
                || case.class.trim().is_empty()
                || case.fixture.trim().is_empty()
            {
                return Err("benchmark case id/class/fixture must be explicit".into());
            }
            if !ids.insert(case.id.as_str()) {
                return Err(format!("duplicate benchmark case id: {}", case.id));
            }
        }
        Ok(())
    }
}

fn is_git_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// This validates an authorization object only; it never opens a network
/// connection. Product code must also require a deliberate operator action
/// before constructing a live runner.
pub fn validate_live_authorization<'a>(
    manifest: &'a OpenAiCacheBenchmarkManifest,
    environment_approval: Option<&str>,
) -> Result<&'a LiveBenchmarkAuthorization, String> {
    manifest.validate_common()?;
    let authorization = manifest
        .live_authorization
        .as_ref()
        .ok_or_else(|| "live benchmark authorization is missing".to_string())?;
    let env = environment_approval
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "live benchmark requires explicit {} confirmation",
                OPENAI_CACHE_LIVE_AUTHORIZATION_ENV
            )
        })?;
    if env != authorization.approval_id {
        return Err("live benchmark approval id does not match manifest".into());
    }
    if authorization.provider != manifest.provider || authorization.model != manifest.model {
        return Err("live benchmark authorization must match manifest provider/model".into());
    }
    if authorization.max_requests == 0 || !authorization.max_cost_usd.is_finite() {
        return Err("live benchmark requires finite nonzero request and cost caps".into());
    }
    if authorization.max_cost_usd <= 0.0 {
        return Err("live benchmark max_cost_usd must be greater than zero".into());
    }
    Ok(authorization)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCacheBenchmarkRow {
    pub manifest_fingerprint: String,
    pub task_id: String,
    pub repetition: u32,
    pub profile: BenchmarkProfile,
    pub source: BenchmarkSource,
    pub outcome: RunOutcome,
    pub metrics: CodexBenchmarkRunMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_cache_diagnostic_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairDisposition {
    IncludedEfficiency,
    FailedOrBlocked,
    MissingBaseline,
    MissingTreatment,
    DuplicateBaseline,
    DuplicateTreatment,
    MixedSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairAudit {
    pub task_id: String,
    pub repetition: u32,
    pub disposition: PairDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_outcome: Option<RunOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment_outcome: Option<RunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCacheBenchmarkReport {
    pub manifest_fingerprint: String,
    pub source: BenchmarkSource,
    pub total_rows: usize,
    pub total_pairs_expected: usize,
    pub included_efficiency_pairs: usize,
    pub outcome_counts: BTreeMap<String, u64>,
    pub pair_audit: Vec<PairAudit>,
    pub baseline_raw_input_tokens: u128,
    pub treatment_raw_input_tokens: u128,
    pub baseline_cache_read_tokens: u128,
    pub treatment_cache_read_tokens: u128,
    pub baseline_cache_write_tokens: u128,
    pub treatment_cache_write_tokens: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input_delta_pct: Option<f64>,
}

/// Build an auditable paired report. Failed/blocked cases remain in
/// `outcome_counts` and `pair_audit`; only mutually verified-success pairs
/// contribute to the efficiency-pair count.
pub fn build_paired_report(
    manifest: &OpenAiCacheBenchmarkManifest,
    source: BenchmarkSource,
    rows: &[OpenAiCacheBenchmarkRow],
) -> Result<OpenAiCacheBenchmarkReport, String> {
    match source {
        BenchmarkSource::OfflineFixture => manifest.validate_offline()?,
        BenchmarkSource::LiveAuthorized => {
            return Err(
                "live rows require validate_live_authorization before report construction"
                    .into(),
            )
        }
    }
    build_paired_report_validated(manifest, source, rows)
}

/// Same report construction after the caller has separately passed the live
/// authorization gate. Kept explicit so tests and future runners cannot
/// accidentally treat offline validation as live authorization.
pub fn build_live_paired_report(
    manifest: &OpenAiCacheBenchmarkManifest,
    rows: &[OpenAiCacheBenchmarkRow],
    environment_approval: Option<&str>,
) -> Result<OpenAiCacheBenchmarkReport, String> {
    validate_live_authorization(manifest, environment_approval)?;
    build_paired_report_validated(manifest, BenchmarkSource::LiveAuthorized, rows)
}

fn build_paired_report_validated(
    manifest: &OpenAiCacheBenchmarkManifest,
    source: BenchmarkSource,
    rows: &[OpenAiCacheBenchmarkRow],
) -> Result<OpenAiCacheBenchmarkReport, String> {
    let fingerprint = manifest.fingerprint();
    if rows.iter().any(|row| row.manifest_fingerprint != fingerprint) {
        return Err("benchmark row manifest fingerprint mismatch".into());
    }
    if rows.iter().any(|row| row.source != source) {
        return Err("offline and live benchmark rows must never share one report".into());
    }
    let known: BTreeSet<_> = manifest.cases.iter().map(|case| case.id.as_str()).collect();
    if rows.iter().any(|row| !known.contains(row.task_id.as_str())) {
        return Err("benchmark row contains a task outside the manifest".into());
    }
    if rows
        .iter()
        .any(|row| row.repetition == 0 || row.repetition > manifest.repetitions)
    {
        return Err("benchmark row repetition is outside the manifest".into());
    }

    let mut groups: BTreeMap<(String, u32), Vec<&OpenAiCacheBenchmarkRow>> = BTreeMap::new();
    let mut outcome_counts = BTreeMap::new();
    let mut baseline_raw = 0u128;
    let mut treatment_raw = 0u128;
    let mut baseline_read = 0u128;
    let mut treatment_read = 0u128;
    let mut baseline_write = 0u128;
    let mut treatment_write = 0u128;

    for row in rows {
        *outcome_counts
            .entry(format!("{:?}", row.outcome).to_ascii_lowercase())
            .or_insert(0) += 1;
        groups
            .entry((row.task_id.clone(), row.repetition))
            .or_default()
            .push(row);
        match row.profile {
            BenchmarkProfile::Baseline => {
                baseline_raw += row.metrics.raw_input_tokens();
                baseline_read += row.metrics.cached_input_tokens as u128;
                baseline_write += row.metrics.cache_write_tokens as u128;
            }
            BenchmarkProfile::Treatment => {
                treatment_raw += row.metrics.raw_input_tokens();
                treatment_read += row.metrics.cached_input_tokens as u128;
                treatment_write += row.metrics.cache_write_tokens as u128;
            }
        }
    }

    let mut pair_audit = Vec::new();
    let mut included_efficiency_pairs = 0usize;
    for case in &manifest.cases {
        for repetition in 1..=manifest.repetitions {
            let pair_rows = groups
                .get(&(case.id.clone(), repetition))
                .cloned()
                .unwrap_or_default();
            let baselines = pair_rows
                .iter()
                .filter(|row| row.profile == BenchmarkProfile::Baseline)
                .copied()
                .collect::<Vec<_>>();
            let treatments = pair_rows
                .iter()
                .filter(|row| row.profile == BenchmarkProfile::Treatment)
                .copied()
                .collect::<Vec<_>>();
            let baseline = baselines.first().copied();
            let treatment = treatments.first().copied();

            let disposition = if baselines.len() > 1 {
                PairDisposition::DuplicateBaseline
            } else if treatments.len() > 1 {
                PairDisposition::DuplicateTreatment
            } else if baseline.is_none() {
                PairDisposition::MissingBaseline
            } else if treatment.is_none() {
                PairDisposition::MissingTreatment
            } else if baseline.unwrap().source != treatment.unwrap().source {
                PairDisposition::MixedSource
            } else if baseline.unwrap().outcome != RunOutcome::VerifiedSuccess
                || treatment.unwrap().outcome != RunOutcome::VerifiedSuccess
            {
                PairDisposition::FailedOrBlocked
            } else {
                included_efficiency_pairs += 1;
                PairDisposition::IncludedEfficiency
            };

            pair_audit.push(PairAudit {
                task_id: case.id.clone(),
                repetition,
                disposition,
                baseline_outcome: baseline.map(|row| row.outcome),
                treatment_outcome: treatment.map(|row| row.outcome),
            });
        }
    }

    let raw_input_delta_pct = (baseline_raw > 0).then(|| {
        (treatment_raw as f64 - baseline_raw as f64) / baseline_raw as f64 * 100.0
    });

    Ok(OpenAiCacheBenchmarkReport {
        manifest_fingerprint: fingerprint,
        source,
        total_rows: rows.len(),
        total_pairs_expected: manifest.cases.len() * manifest.repetitions as usize,
        included_efficiency_pairs,
        outcome_counts,
        pair_audit,
        baseline_raw_input_tokens: baseline_raw,
        treatment_raw_input_tokens: treatment_raw,
        baseline_cache_read_tokens: baseline_read,
        treatment_cache_read_tokens: treatment_read,
        baseline_cache_write_tokens: baseline_write,
        treatment_cache_write_tokens: treatment_write,
        raw_input_delta_pct,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> OpenAiCacheBenchmarkManifest {
        OpenAiCacheBenchmarkManifest {
            schema_version: OPENAI_CACHE_EVAL_SCHEMA_VERSION,
            revision: "63f3d2a6d1cb848b768d26935377eb152485235f".into(),
            provider: "openai".into(),
            model: "gpt-5.6-sol".into(),
            endpoint_family: "openai-responses".into(),
            policy_contract_revision: "openai-cache-2026-09-21-v1".into(),
            feature: OpenAiCacheFeature::ExplicitStableBoundary,
            baseline_profile: "legacy-provider-default".into(),
            treatment_profile: "explicit-stable-boundary".into(),
            repetitions: 1,
            cases: vec![
                OpenAiCacheBenchmarkCase {
                    id: "append".into(),
                    class: "append_only".into(),
                    fixture: "append.json".into(),
                    expected_outcome: RunOutcome::VerifiedSuccess,
                },
                OpenAiCacheBenchmarkCase {
                    id: "failure".into(),
                    class: "negative_control".into(),
                    fixture: "failure.json".into(),
                    expected_outcome: RunOutcome::VerifiedFailure,
                },
            ],
            live_authorization: None,
        }
    }

    fn row(
        manifest: &OpenAiCacheBenchmarkManifest,
        task: &str,
        profile: BenchmarkProfile,
        outcome: RunOutcome,
        raw: (u64, u64, u64),
    ) -> OpenAiCacheBenchmarkRow {
        OpenAiCacheBenchmarkRow {
            manifest_fingerprint: manifest.fingerprint(),
            task_id: task.into(),
            repetition: 1,
            profile,
            source: BenchmarkSource::OfflineFixture,
            outcome,
            metrics: CodexBenchmarkRunMetrics {
                success: outcome == RunOutcome::VerifiedSuccess,
                uncached_input_tokens: raw.0,
                cached_input_tokens: raw.1,
                cache_write_tokens: raw.2,
                ..Default::default()
            },
            provider_cache_diagnostic_type: None,
        }
    }

    #[test]
    fn offline_manifest_requires_exact_revision_and_no_live_authorization() {
        let manifest = manifest();
        manifest.validate_offline().unwrap();
        let mut invalid = manifest.clone();
        invalid.revision = "branch-name".into();
        assert!(invalid.validate_offline().is_err());
        invalid = manifest.clone();
        invalid.live_authorization = Some(LiveBenchmarkAuthorization {
            approval_id: "approved".into(),
            provider: "openai".into(),
            model: "gpt-5.6-sol".into(),
            max_requests: 10,
            max_cost_usd: 5.0,
        });
        assert!(invalid.validate_offline().is_err());
    }

    #[test]
    fn live_mode_requires_matching_explicit_approval_and_hard_caps() {
        let mut manifest = manifest();
        manifest.live_authorization = Some(LiveBenchmarkAuthorization {
            approval_id: "ticket-123".into(),
            provider: "openai".into(),
            model: "gpt-5.6-sol".into(),
            max_requests: 12,
            max_cost_usd: 3.0,
        });
        assert!(validate_live_authorization(&manifest, None).is_err());
        assert!(validate_live_authorization(&manifest, Some("wrong")).is_err());
        assert_eq!(
            validate_live_authorization(&manifest, Some("ticket-123"))
                .unwrap()
                .max_requests,
            12
        );

        manifest.live_authorization.as_mut().unwrap().max_cost_usd = 0.0;
        assert!(validate_live_authorization(&manifest, Some("ticket-123")).is_err());
    }

    #[test]
    fn failed_pairs_stay_in_outcomes_but_are_excluded_from_efficiency() {
        let manifest = manifest();
        let rows = vec![
            row(
                &manifest,
                "append",
                BenchmarkProfile::Baseline,
                RunOutcome::VerifiedSuccess,
                (1000, 0, 0),
            ),
            row(
                &manifest,
                "append",
                BenchmarkProfile::Treatment,
                RunOutcome::VerifiedSuccess,
                (300, 600, 100),
            ),
            row(
                &manifest,
                "failure",
                BenchmarkProfile::Baseline,
                RunOutcome::VerifiedFailure,
                (500, 0, 0),
            ),
            row(
                &manifest,
                "failure",
                BenchmarkProfile::Treatment,
                RunOutcome::VerifiedFailure,
                (200, 0, 0),
            ),
        ];
        let report =
            build_paired_report(&manifest, BenchmarkSource::OfflineFixture, &rows).unwrap();
        assert_eq!(report.total_pairs_expected, 2);
        assert_eq!(report.included_efficiency_pairs, 1);
        assert_eq!(
            report
                .pair_audit
                .iter()
                .find(|pair| pair.task_id == "failure")
                .unwrap()
                .disposition,
            PairDisposition::FailedOrBlocked
        );
        assert_eq!(report.outcome_counts.get("verifiedfailure"), Some(&2));
        assert_eq!(report.treatment_cache_write_tokens, 100);
    }

    #[test]
    fn missing_and_duplicate_pairs_are_published_not_silently_dropped() {
        let mut manifest = manifest();
        manifest.cases.truncate(1);
        let baseline = row(
            &manifest,
            "append",
            BenchmarkProfile::Baseline,
            RunOutcome::VerifiedSuccess,
            (100, 0, 0),
        );
        let missing =
            build_paired_report(&manifest, BenchmarkSource::OfflineFixture, &[baseline.clone()])
                .unwrap();
        assert_eq!(
            missing.pair_audit[0].disposition,
            PairDisposition::MissingTreatment
        );

        let duplicate = build_paired_report(
            &manifest,
            BenchmarkSource::OfflineFixture,
            &[baseline.clone(), baseline],
        )
        .unwrap();
        assert_eq!(
            duplicate.pair_audit[0].disposition,
            PairDisposition::DuplicateBaseline
        );
    }

    #[test]
    fn manifest_fingerprint_changes_with_revision_or_feature() {
        let a = manifest();
        let mut b = a.clone();
        b.revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        assert_ne!(a.fingerprint(), b.fingerprint());
        b = a.clone();
        b.feature = OpenAiCacheFeature::NativeResponsesReplay;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
