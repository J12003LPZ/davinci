use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::request::DecisionRequest;
use super::risk::DecisionClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAuditOutcome {
    Success,
    Fallback,
    Rejected,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionAuditRecord {
    pub request_id: String,
    pub provider: String,
    pub model: String,
    pub decision_class: DecisionClass,
    pub question_ids: Vec<String>,
    pub state_hash: String,
    pub state_bytes: usize,
    pub latency_ms: u64,
    pub outcome: DecisionAuditOutcome,
    pub applied_actions: Vec<String>,
    pub disagreements: Vec<String>,
}

#[derive(Debug)]
pub struct DecisionAuditLog {
    records: Mutex<Vec<DecisionAuditRecord>>,
    max_records: usize,
}

impl Default for DecisionAuditLog {
    fn default() -> Self {
        Self::new(128)
    }
}

impl DecisionAuditLog {
    pub fn new(max_records: usize) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            max_records: max_records.max(1),
        }
    }

    pub fn push(&self, record: DecisionAuditRecord) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        records.push(record);
        if records.len() > self.max_records {
            let excess = records.len() - self.max_records;
            records.drain(..excess);
        }
    }

    pub fn snapshot(&self) -> Vec<DecisionAuditRecord> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

pub fn state_hash(state: &Value) -> String {
    let canonical = serde_json::to_vec(state).unwrap_or_default();
    let digest = Sha256::digest(canonical);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn record_for(
    request: &DecisionRequest,
    provider: impl Into<String>,
    model: impl Into<String>,
    latency_ms: u64,
    outcome: DecisionAuditOutcome,
) -> DecisionAuditRecord {
    DecisionAuditRecord {
        request_id: request.request_id.clone(),
        provider: provider.into(),
        model: model.into(),
        decision_class: request.decision_class,
        question_ids: request.questions.keys().cloned().collect(),
        state_hash: state_hash(&request.state),
        state_bytes: serde_json::to_vec(&request.state).map_or(0, |bytes| bytes.len()),
        latency_ms,
        outcome,
        applied_actions: Vec::new(),
        disagreements: Vec::new(),
    }
}
