//! Versioned native scan lifecycle, separate from legacy deterministic signals.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Created,
    Preflight,
    Snapshotting,
    Mapping,
    Investigating,
    Validating,
    Reporting,
    Completed,
    Cancelling,
    Cancelled,
    Failed,
    Interrupted,
}

impl RunStatus {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunProgress {
    pub schema_version: u32,
    pub scan_id: String,
    pub generation: u64,
    pub status: RunStatus,
    pub coverage_complete: bool,
    pub experimental: bool,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub usage: Vec<super::usage::RequestUsage>,
}
