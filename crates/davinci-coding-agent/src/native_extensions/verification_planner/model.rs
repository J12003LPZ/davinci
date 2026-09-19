use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

fn default_max_files() -> usize {
    64
}

fn default_max_steps() -> usize {
    32
}

fn default_timeout_ms() -> u64 {
    5_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationPlannerConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for VerificationPlannerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_files: default_max_files(),
            max_steps: default_max_steps(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

impl VerificationPlannerConfig {
    pub fn bounded(mut self) -> Self {
        self.max_files = self.max_files.clamp(1, 128);
        self.max_steps = self.max_steps.clamp(1, 64);
        self.timeout_ms = self.timeout_ms.clamp(100, 30_000);
        self
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationPlanArgs {
    pub files: Vec<String>,
    pub path: Option<String>,
    pub transaction_id: Option<String>,
    pub changed_symbols: Vec<String>,
    pub user_requirements: Vec<String>,
    pub force_full: bool,
    pub ci_required: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRequirement {
    pub kind: String,
    pub reason: String,
    pub evidence_kind: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationStep {
    pub id: String,
    pub tier: u8,
    pub required: bool,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    pub argv: Vec<String>,
    pub cwd: String,
    pub reason: String,
    pub evidence_kind: String,
    pub source_identity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationTelemetry {
    pub requests: u64,
    pub failures: u64,
    pub last_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationPlan {
    pub schema_version: u32,
    pub enabled: bool,
    pub source_identity: String,
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<String>,
    pub classification: Vec<String>,
    pub requirements: Vec<VerificationRequirement>,
    pub steps: Vec<VerificationStep>,
    pub warnings: Vec<String>,
    pub partial: bool,
    pub complete: bool,
    pub telemetry: VerificationTelemetry,
}
