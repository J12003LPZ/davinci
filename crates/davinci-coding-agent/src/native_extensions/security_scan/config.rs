//! Native-only scan admission policy; no upstream TypeScript equivalent.

use super::command::ScanMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanConfig {
    pub default_mode: ScanMode,
    pub model: String,
    pub provider_access: String,
    pub tool_network: String,
    pub dynamic_validation: String,
    pub max_concurrency: usize,
    pub max_file_bytes: u64,
    pub max_policy_bytes: u64,
    pub max_inventory_entries: usize,
    pub max_snapshot_bytes: u64,
    pub validation_reserve_ratio: f64,
    pub analyzers: Vec<String>,
    pub retention_days: u32,
    pub index_into_memory: bool,
    pub supporting_reads: bool,
    pub fail_on: FailOn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FailOn {
    pub classifications: Vec<String>,
    pub minimum_severity: String,
}
impl Default for FailOn {
    fn default() -> Self {
        Self {
            classifications: vec!["confirmed".into()],
            minimum_severity: "high".into(),
        }
    }
}
impl FailOn {
    pub fn validate(&self) -> Result<(), String> {
        let unique = self
            .classifications
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.is_empty()
            || unique.len() != self.classifications.len()
            || self
                .classifications
                .iter()
                .any(|value| !["confirmed", "likely"].contains(&value.as_str()))
            || !["critical", "high", "medium", "low", "informational"]
                .contains(&self.minimum_severity.as_str())
        {
            return Err("invalid security finding failure policy".into());
        }
        Ok(())
    }
}
pub fn severity_rank(value: &str) -> u8 {
    match value {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            default_mode: ScanMode::Standard,
            model: "inherit".into(),
            provider_access: "existing-authorization-only".into(),
            tool_network: "deny".into(),
            dynamic_validation: "disabled".into(),
            max_concurrency: 2,
            max_file_bytes: 2 * 1024 * 1024,
            max_policy_bytes: 1024 * 1024,
            max_inventory_entries: 50_000,
            max_snapshot_bytes: 512 * 1024 * 1024,
            validation_reserve_ratio: 0.25,
            analyzers: Vec::new(),
            retention_days: 30,
            index_into_memory: false,
            supporting_reads: true,
            fail_on: FailOn::default(),
        }
    }
}

impl ScanConfig {
    /// JSON escaping plus bounded metadata, capped by the archive reader policy.
    pub(super) fn checkpoint_byte_limit(&self) -> u64 {
        self.max_snapshot_bytes
            .min(Self::default().max_snapshot_bytes)
            .saturating_mul(6)
            .saturating_add(1024 * 1024)
    }

    pub fn validate_snapshot(&self, snapshot: &super::snapshot::Snapshot) -> Result<(), String> {
        if snapshot.bytes > self.max_snapshot_bytes
            || snapshot.source_count() > self.max_inventory_entries
        {
            return Err("checkpoint exceeds current scan policy".into());
        }
        if !self.supporting_reads
            && (!snapshot.supporting_files.is_empty()
                || !snapshot.supporting_base_files.is_empty()
                || snapshot
                    .conflict_sources
                    .iter()
                    .any(|source| source.supporting))
        {
            return Err("checkpoint supporting source exceeds current read policy".into());
        }
        for (_, path, file) in snapshot.sources() {
            let limit = if path.rsplit('/').next() == Some("SECURITY.md") {
                self.max_file_bytes.min(self.max_policy_bytes)
            } else {
                self.max_file_bytes
            };
            if file.text.len() as u64 > limit {
                return Err("checkpoint source exceeds current file policy".into());
            }
        }
        Ok(())
    }

    pub fn narrow_with(&self, project: &Self) -> Result<Self, String> {
        self.validate()?;
        project.validate()?;
        Ok(Self {
            supporting_reads: self.supporting_reads && project.supporting_reads,
            max_concurrency: self.max_concurrency.min(project.max_concurrency),
            max_file_bytes: self.max_file_bytes.min(project.max_file_bytes),
            max_policy_bytes: self.max_policy_bytes.min(project.max_policy_bytes),
            max_inventory_entries: self
                .max_inventory_entries
                .min(project.max_inventory_entries),
            max_snapshot_bytes: self.max_snapshot_bytes.min(project.max_snapshot_bytes),
            validation_reserve_ratio: self
                .validation_reserve_ratio
                .max(project.validation_reserve_ratio),
            fail_on: FailOn {
                classifications: ["confirmed", "likely"]
                    .iter()
                    .filter(|class| {
                        self.fail_on
                            .classifications
                            .iter()
                            .chain(&project.fail_on.classifications)
                            .any(|value| value == **class)
                    })
                    .map(|class| (*class).to_string())
                    .collect(),
                minimum_severity: if severity_rank(&project.fail_on.minimum_severity)
                    < severity_rank(&self.fail_on.minimum_severity)
                {
                    project.fail_on.minimum_severity.clone()
                } else {
                    self.fail_on.minimum_severity.clone()
                },
            },
            ..self.clone()
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        self.fail_on.validate()?;
        if self.model != "inherit"
            || self.provider_access != "existing-authorization-only"
            || self.tool_network != "deny"
            || self.dynamic_validation != "disabled"
            || self.index_into_memory
        {
            return Err("security scan requires inherited model authorization, offline tools, and disabled dynamic validation and memory indexing".into());
        }
        let unique = self
            .analyzers
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != self.analyzers.len()
            || self.analyzers.iter().any(|name| name != "cargo-audit")
        {
            return Err("unsupported or duplicate security analyzer".into());
        }
        if !(1..=4).contains(&self.max_concurrency)
            || self.max_file_bytes == 0
            || self.max_policy_bytes == 0
            || self.max_inventory_entries == 0
            || self.max_snapshot_bytes < self.max_file_bytes
            || !self.validation_reserve_ratio.is_finite()
            || !(0.25..=0.75).contains(&self.validation_reserve_ratio)
        {
            return Err("invalid security scan resource limits".into());
        }
        Ok(())
    }

    // This grant is supplied by the host, never deserialized from project data.
    pub fn admit(&self, source_transmission_authorized: bool) -> Result<(), String> {
        self.validate()?;
        if !source_transmission_authorized {
            return Err("selected provider is not authorized for scan source transmission".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn security_resume_applies_limits_to_supporting_sources_on_both_sides() {
        use super::super::snapshot::{Snapshot, SourceFile};
        for baseline in [false, true] {
            let mut snapshot = Snapshot::default();
            let files = if baseline {
                &mut snapshot.supporting_base_files
            } else {
                &mut snapshot.supporting_files
            };
            files.insert(
                "helper.rs".into(),
                SourceFile {
                    hash: String::new(),
                    text: "x".repeat(1025),
                },
            );
            let config = ScanConfig {
                max_file_bytes: 1024,
                ..Default::default()
            };
            assert!(config.validate_snapshot(&snapshot).is_err());
            let config = ScanConfig {
                supporting_reads: false,
                ..Default::default()
            };
            assert!(config.validate_snapshot(&snapshot).is_err());
        }
    }

    #[test]
    fn security_resume_counts_supporting_inventory_and_policy_bytes() {
        use super::super::snapshot::{Snapshot, SourceFile};
        let mut snapshot = Snapshot::default();
        snapshot.files.insert(
            "main.rs".into(),
            SourceFile {
                hash: String::new(),
                text: "x".into(),
            },
        );
        snapshot.supporting_files.insert(
            "SECURITY.md".into(),
            SourceFile {
                hash: String::new(),
                text: "x".repeat(1025),
            },
        );
        let config = ScanConfig {
            max_inventory_entries: 1,
            ..Default::default()
        };
        assert!(config.validate_snapshot(&snapshot).is_err());
        let config = ScanConfig {
            max_policy_bytes: 1024,
            ..Default::default()
        };
        assert!(config.validate_snapshot(&snapshot).is_err());
        assert!(ScanConfig::default().validate_snapshot(&snapshot).is_ok());
    }
    #[test]
    fn security_failure_policy_merge_preserves_both_dimensions() {
        let parent = ScanConfig {
            fail_on: FailOn {
                classifications: vec!["confirmed".into(), "likely".into()],
                minimum_severity: "high".into(),
            },
            ..Default::default()
        };
        let project = ScanConfig {
            fail_on: FailOn {
                classifications: vec!["confirmed".into()],
                minimum_severity: "medium".into(),
            },
            ..Default::default()
        };
        let merged = parent.narrow_with(&project).unwrap();
        assert_eq!(merged.fail_on.classifications, ["confirmed", "likely"]);
        assert_eq!(merged.fail_on.minimum_severity, "medium");
        assert_eq!(
            project
                .narrow_with(&parent)
                .unwrap()
                .fail_on
                .classifications,
            ["confirmed", "likely"]
        );
        for invalid in [vec![], vec!["confirmed", "confirmed"], vec!["suspected"]] {
            let config = ScanConfig {
                fail_on: FailOn {
                    classifications: invalid.into_iter().map(String::from).collect(),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn security_scan_excludes_evidence_from_memory_learning_and_telemetry() {
        let config = ScanConfig::default();
        assert!(!config.index_into_memory);
        assert!(ScanConfig {
            index_into_memory: true,
            ..config
        }
        .validate()
        .is_err());
    }

    #[test]
    fn security_scan_settings_do_not_broaden_parent_permissions() {
        let config = ScanConfig::default();
        assert!(config.validate().is_ok());
        assert!(config.admit(false).is_err());
        assert!(config.admit(true).is_ok());
        for field in [
            "toolNetwork",
            "dynamicValidation",
            "indexIntoMemory",
            "model",
        ] {
            let mut value = serde_json::to_value(&config).unwrap();
            value[field] = if field == "indexIntoMemory" {
                serde_json::json!(true)
            } else {
                serde_json::json!("enabled")
            };
            let parsed = serde_json::from_value::<ScanConfig>(value);
            assert!(parsed.is_err() || parsed.unwrap().validate().is_err());
        }
        assert!(serde_json::from_str::<ScanConfig>(r#"{"unknownAuthority":true}"#).is_err());
        let narrower = ScanConfig {
            max_file_bytes: 1024,
            ..config.clone()
        };
        let effective = narrower.narrow_with(&config).unwrap();
        assert_eq!(effective.max_file_bytes, 1024);
        let enabled = ScanConfig {
            analyzers: vec!["cargo-audit".into()],
            ..config.clone()
        };
        assert!(enabled.validate().is_ok());
        assert!(config.narrow_with(&enabled).unwrap().analyzers.is_empty());
        assert_eq!(
            enabled.narrow_with(&config).unwrap().analyzers,
            ["cargo-audit"]
        );
        assert!(ScanConfig {
            analyzers: vec!["osv-scanner".into()],
            ..config
        }
        .validate()
        .is_err());
    }
}
