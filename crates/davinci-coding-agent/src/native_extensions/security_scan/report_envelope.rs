//! Strict final-report wire contract; native scanner has no upstream counterpart.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Report {
    schema_version: u32,
    scan_id: String,
    generation: u64,
    snapshot_id: String,
    experimental: bool,
    coverage_complete: bool,
    source: Source,
    coverage: Coverage,
    findings: Vec<Finding>,
    unresolved_leads: Vec<Candidate>,
    candidates: Vec<Candidate>,
    limitations: Vec<String>,
    runtime_tests_executed: bool,
    fail_on: super::config::FailOn,
    methodology: Methodology,
    provenance: Provenance,
    budgets: Budgets,
    usage: Vec<super::usage::RequestUsage>,
    hardening: Vec<Value>,
    checks: Vec<Check>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    revisions: Option<(String, String)>,
    current_side: String,
    index_conflicts: Vec<super::snapshot::ConflictStage>,
    supporting_files: usize,
    supporting_base_files: usize,
    supporting_skipped: Vec<super::snapshot::SkippedFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Coverage {
    eligible_files: usize,
    reviewed_paths: Vec<String>,
    baseline_files: usize,
    reviewed_base_paths: Vec<String>,
    skipped: Vec<super::snapshot::SkippedFile>,
    audits: Vec<super::review::AuditCoverage>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Methodology {
    version: String,
    phases: Vec<Phase>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_skill_migration: Vec<SkillMigrationRow>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillMigrationRow {
    name: String,
    treatment: String,
    native_contract: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Phase {
    role: String,
    sha256: String,
    input: String,
    output: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Provenance {
    Provider(Provider),
    Injected(Injected),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Provider {
    provider: String,
    model_id: String,
    requested_thinking_level: String,
    effective_thinking_level: String,
    quality_evaluation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Injected {
    runner: String,
    quality_evaluation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Budgets {
    max_tokens: u64,
    max_turns: usize,
    validation_reserve_ratio: f64,
    discovery_per_audit: Allocation,
    cumulative: Cumulative,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Allocation {
    mapping_turns: usize,
    mapping_tokens: u64,
    audit_turns: usize,
    audit_tokens: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cumulative {
    accounted_tokens: u64,
    requests: usize,
    remaining_tokens: u64,
    remaining_requests: usize,
    unsettled_requests: usize,
    discovery_requests: usize,
    discovery_tokens: u64,
    max_discovery_tokens: u64,
    remaining_discovery_tokens: u64,
    remaining_discovery_requests: usize,
    persistent: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    name: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    signals: Vec<AnalyzerSignal>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerSignal {
    id: String,
    package: String,
    version: String,
    title: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Candidate {
    Assessed(Box<AssessedCandidate>),
    Host(Box<HostCandidate>),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Finding {
    finding_id: String,
    candidate_ids: Vec<String>,
    fingerprint: Fingerprint,
    scope: Scope,
    representative_candidate_id: String,
    claim: super::validation::Claim,
    assessment: super::validation::Assessment,
    validation: Validation,
    occurrences: Vec<AssessedCandidate>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssessedCandidate {
    candidate_id: String,
    finding_id: String,
    occurrence_id: String,
    fingerprint: Fingerprint,
    scope: Scope,
    claim: super::validation::Claim,
    assessment: super::validation::Assessment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prior_assessment: Option<super::validation::Assessment>,
    validation: Validation,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Scope {
    Target,
    Supporting,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fingerprint {
    version: u32,
    value: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Validation {
    method: String,
    runtime_reproduction: String,
    review_type: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostCandidate {
    candidate_id: String,
    claim: super::validation::Claim,
    assessment: HostAssessment,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
enum HostAssessment {
    Deferred {
        reason: String,
    },
    Duplicate {
        #[serde(rename = "duplicateOf")]
        duplicate_of: String,
        reason: String,
    },
}

pub(super) fn validate(value: &Value) -> Result<(), String> {
    let report: Report =
        serde_json::from_value(value.clone()).map_err(|_| "invalid final report envelope")?;
    // Round-trip equality also rejects missing nullable/defaulted fields.
    if serde_json::to_value(&report).map_err(|_| "cannot encode final report")? != *value {
        return Err("final report fields are missing or noncanonical".into());
    }
    if report.schema_version != 2 || !(1..=1024).contains(&report.generation) {
        return Err("unsupported final report version or generation".into());
    }
    let id = uuid::Uuid::parse_str(&report.scan_id).map_err(|_| "invalid report scan identity")?;
    if id.is_nil() || id.to_string() != report.scan_id {
        return Err("noncanonical report scan identity".into());
    }
    report.fail_on.validate()?;
    let mut conflict_ids = std::collections::BTreeSet::new();
    for stage in &report.source.index_conflicts {
        stage.validate()?;
        if !conflict_ids.insert((&stage.path, stage.stage)) {
            return Err("duplicate report conflict stage".into());
        }
    }
    if report.coverage_complete && !conflict_ids.is_empty() {
        return Err("unresolved merge prevents complete report coverage".into());
    }
    report.budgets.validate()?;
    match &report.provenance {
        Provenance::Provider(provider) => {
            if [
                &provider.provider,
                &provider.model_id,
                &provider.requested_thinking_level,
                &provider.effective_thinking_level,
            ]
            .iter()
            .any(|s| s.trim().is_empty())
                || provider.quality_evaluation != "unmeasured"
            {
                return Err("invalid provider provenance".into());
            }
        }
        Provenance::Injected(injected) => {
            if !matches!(
                (
                    injected.runner.as_str(),
                    injected.quality_evaluation.as_str()
                ),
                ("offline-fixture", "synthetic-only") | ("injected", "unmeasured")
            ) {
                return Err("invalid injected runner provenance".into());
            }
        }
    }
    if !report.experimental || !report.hardening.is_empty() {
        return Err("report claims unsupported release or hardening results".into());
    }
    if report.checks.len() != 2
        || report.checks[0].name != "runtime-reproduction"
        || report.checks[0].status != "not_attempted"
        || report.checks[1].name != "external-analyzers"
        || !matches!(
            report.checks[1].status.as_str(),
            "disabled" | "limitation" | "completed"
        )
    {
        return Err("report claims unsupported checks".into());
    }
    let analyzers = &report.checks[1];
    if analyzers.status == "disabled"
        && (analyzers.reason.is_some() || !analyzers.signals.is_empty())
        || analyzers.status == "limitation"
            && analyzers
                .reason
                .as_ref()
                .is_none_or(|reason| reason.trim().is_empty())
        || analyzers
            .signals
            .iter()
            .any(|signal| signal.id.trim().is_empty() || signal.package.trim().is_empty())
        || analyzers.status != "completed" && !analyzers.signals.is_empty()
    {
        return Err("report claims unsupported analyzer results".into());
    }
    Ok(())
}

impl Budgets {
    fn validate(&self) -> Result<(), String> {
        let cumulative = &self.cumulative;
        if self.max_tokens == 0
            || self.max_turns == 0
            || !self.validation_reserve_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.validation_reserve_ratio)
            || cumulative.remaining_tokens
                != self.max_tokens.saturating_sub(cumulative.accounted_tokens)
            || cumulative.remaining_requests != self.max_turns.saturating_sub(cumulative.requests)
            || cumulative.remaining_discovery_tokens
                != cumulative
                    .max_discovery_tokens
                    .saturating_sub(cumulative.discovery_tokens)
            || cumulative.remaining_discovery_requests
                != (self.max_turns / 2).saturating_sub(cumulative.discovery_requests)
            || cumulative.unsettled_requests > cumulative.requests
            || cumulative.discovery_requests > cumulative.requests
            || cumulative.discovery_tokens > cumulative.accounted_tokens
            || cumulative.max_discovery_tokens > self.max_tokens
        {
            return Err("inconsistent report budget accounting".into());
        }
        Ok(())
    }
}
