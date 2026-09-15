//! File-level deterministic security evidence reuse.

use super::{redact_evidence, sha256_hex, SecurityCandidate, SecurityCoverage, SecurityFinding};
use crate::native_extensions::security_scan::SecurityScanConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bump when the deterministic scanner rules change in a way that invalidates
/// cached file evidence.
pub const SECURITY_RULESET_VERSION: &str = "security-rules-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityFileCacheKey {
    pub content_sha256: String,
    pub ruleset_version: String,
    pub config_sha256: String,
}

impl SecurityFileCacheKey {
    pub fn new(content: &[u8], ruleset_version: &str, config: &SecurityScanConfig) -> Self {
        let config_bytes = serde_json::to_vec(config).unwrap_or_default();
        Self {
            content_sha256: sha256_hex(content),
            ruleset_version: ruleset_version.to_string(),
            config_sha256: sha256_hex(&config_bytes),
        }
    }

    pub fn for_file(content: &[u8], config: &SecurityScanConfig) -> Self {
        Self::new(content, SECURITY_RULESET_VERSION, config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CachedFileScan {
    pub key: SecurityFileCacheKey,
    pub findings: Vec<SecurityFinding>,
    pub candidates: Vec<SecurityCandidate>,
    pub coverage: SecurityCoverage,
}

impl CachedFileScan {
    fn is_safe_for_path(&self, relative: &str) -> bool {
        self.findings.iter().all(|finding| {
            finding.file == relative && finding.evidence == redact_evidence(&finding.evidence)
        }) && self
            .candidates
            .iter()
            .all(|candidate| candidate.file == relative)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncrementalSecurityTelemetry {
    pub files_scanned_cold: usize,
    pub files_reused: usize,
    pub files_rescanned: usize,
    pub cache_read_errors: usize,
    pub cache_write_errors: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncrementalSecurityCache {
    entries: BTreeMap<String, CachedFileScan>,
    #[serde(skip)]
    pub telemetry: IncrementalSecurityTelemetry,
}

impl IncrementalSecurityCache {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn reset_telemetry(&mut self) {
        self.telemetry = IncrementalSecurityTelemetry::default();
    }

    pub fn contains_path(&self, relative: &str) -> bool {
        self.entries.contains_key(relative)
    }

    pub fn lookup(&mut self, relative: &str, key: &SecurityFileCacheKey) -> Option<CachedFileScan> {
        let entry = self.entries.get(relative)?;
        if entry.key != *key {
            return None;
        }
        if !entry.is_safe_for_path(relative) {
            self.telemetry.cache_read_errors = self.telemetry.cache_read_errors.saturating_add(1);
            return None;
        }
        Some(entry.clone())
    }

    pub fn insert(&mut self, relative: impl Into<String>, scan: CachedFileScan) {
        self.entries.insert(relative.into(), scan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::security_scan::{FindingSeverity, SecurityScanConfig};

    fn clean_scan(relative: &str, content: &[u8]) -> CachedFileScan {
        CachedFileScan {
            key: SecurityFileCacheKey::for_file(content, &SecurityScanConfig::default()),
            findings: vec![SecurityFinding {
                id: "finding".into(),
                rule_id: "rule".into(),
                severity: FindingSeverity::Low,
                file: relative.into(),
                line: 1,
                message: "safe fixture".into(),
                evidence: "safe fixture".into(),
                validated: false,
                false_positive: false,
            }],
            candidates: vec![SecurityCandidate {
                id: "candidate".into(),
                rule_id: "rule".into(),
                file: relative.into(),
                line: 1,
                reason: "safe fixture".into(),
                validated: false,
                disposition: None,
                validation_reason: None,
                attack_path: None,
            }],
            coverage: SecurityCoverage {
                files_scanned: 1,
                files_skipped: 0,
                bytes_scanned: content.len() as u64,
                candidate_count: 1,
                finding_count: 1,
                network_used: false,
                files_scanned_cold: 0,
                files_reused: 0,
                files_rescanned: 0,
                cache_read_errors: 0,
                cache_write_errors: 0,
            },
        }
    }

    #[test]
    fn security_file_cache_key_is_content_rules_and_config_sensitive() {
        let config = SecurityScanConfig::default();
        let same = SecurityFileCacheKey::new(b"same", "rules-v1", &config);
        assert_eq!(
            same,
            SecurityFileCacheKey::new(b"same", "rules-v1", &config)
        );
        assert_ne!(
            same,
            SecurityFileCacheKey::new(b"changed", "rules-v1", &config)
        );
        assert_ne!(
            same,
            SecurityFileCacheKey::new(b"same", "rules-v2", &config)
        );
        let changed_config = SecurityScanConfig {
            max_file_bytes: config.max_file_bytes + 1,
            ..config.clone()
        };
        assert_ne!(
            same,
            SecurityFileCacheKey::new(b"same", "rules-v1", &changed_config)
        );
    }

    #[test]
    fn incremental_cache_reuses_unchanged_evidence() {
        let mut cache = IncrementalSecurityCache::new();
        let entry = clean_scan("fixture.rs", b"same");
        let key = entry.key.clone();
        cache.insert("fixture.rs", entry.clone());
        assert_eq!(cache.lookup("fixture.rs", &key), Some(entry));
        assert_eq!(cache.telemetry.cache_read_errors, 0);
    }

    #[test]
    fn incremental_cache_misses_changed_content_and_records_corrupt_evidence() {
        let mut cache = IncrementalSecurityCache::new();
        let entry = clean_scan("fixture.rs", b"same");
        cache.insert("fixture.rs", entry.clone());
        let changed = SecurityFileCacheKey::for_file(b"changed", &SecurityScanConfig::default());
        assert!(cache.lookup("fixture.rs", &changed).is_none());

        let mut corrupt = entry;
        corrupt.findings[0].evidence = "sk-raw-secret".into();
        cache.insert("fixture.rs", corrupt);
        assert!(cache
            .lookup(
                "fixture.rs",
                &SecurityFileCacheKey::for_file(b"same", &SecurityScanConfig::default())
            )
            .is_none());
        assert_eq!(cache.telemetry.cache_read_errors, 1);
    }
}
