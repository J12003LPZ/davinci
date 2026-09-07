//! Source-bound semantic identity supplied by native counterreview, not discovery.
//! No upstream TypeScript counterpart. Equivalence still requires causal review.
use super::validation::{Gate, Location};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootCause {
    pub root_control: String,
    pub violated_invariant: String,
    pub sink_or_decision: String,
    pub attack_path_class: String,
    pub control: Location,
    pub decision: Location,
}

impl RootCause {
    pub(super) fn validate(&self, gates: &[Gate; 5]) -> Result<(), String> {
        for field in [
            &self.root_control,
            &self.violated_invariant,
            &self.sink_or_decision,
            &self.attack_path_class,
        ] {
            if field.trim().is_empty()
                || field.len() > 2048
                || field.chars().any(char::is_control)
                || super::redaction::text(field) != *field
            {
                return Err("root identity requires bounded semantic fields".into());
            }
        }
        if self.control.role != "control"
            || !["sink", "control"].contains(&self.decision.role.as_str())
            || !gates[2]
                .locations
                .iter()
                .any(|location| covers(location, &self.control))
            || !gates
                .iter()
                .flat_map(|gate| &gate.locations)
                .any(|location| covers(location, &self.decision))
        {
            return Err(
                "root identity must be bound to reviewed control and decision evidence".into(),
            );
        }
        Ok(())
    }

    pub(super) fn fingerprint(&self) -> serde_json::Value {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"davinci.security-scan.root/v2");
        // Preserve case and source paths. Whitespace normalization is not semantic
        // equivalence, and never authorizes discarding a different occurrence.
        for field in [
            self.control.path.clone(),
            normalize(&self.root_control),
            normalize(&self.violated_invariant),
            self.decision.path.clone(),
            normalize(&self.sink_or_decision),
            normalize(&self.attack_path_class),
        ] {
            hash.update((field.len() as u64).to_le_bytes());
            hash.update(field.as_bytes());
        }
        serde_json::json!({"version":2,"value":format!("{:x}", hash.finalize())})
    }
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn covers(reviewed: &Location, identity: &Location) -> bool {
    reviewed.path == identity.path
        && reviewed.snapshot_side == identity.snapshot_side
        && reviewed.content_hash == identity.content_hash
        && identity.start_line > 0
        && identity.start_line <= identity.end_line
        && reviewed.start_line <= identity.start_line
        && reviewed.end_line >= identity.end_line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_root_identity_rejects_unreviewed_controls_and_unsafe_labels() {
        let record =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let assessment: super::super::validation::Assessment =
            serde_json::from_value(record["assessment"].clone()).unwrap();
        let root = assessment.root_cause.unwrap();
        assert!(root.validate(&assessment.gates).is_ok());
        for (key, value) in [
            ("path", serde_json::json!("unreviewed.rs")),
            ("contentHash", serde_json::json!("0".repeat(64))),
            ("startLine", serde_json::json!(0)),
            ("endLine", serde_json::json!(2)),
            ("snapshotSide", serde_json::json!("base")),
            ("role", serde_json::json!("test")),
        ] {
            for anchor in ["control", "decision"] {
                let mut changed = serde_json::to_value(&root).unwrap();
                changed[anchor][key] = value.clone();
                let changed: RootCause = serde_json::from_value(changed).unwrap();
                assert!(
                    changed.validate(&assessment.gates).is_err(),
                    "{anchor}/{key}"
                );
            }
        }
        for value in [
            "".to_string(),
            "x".repeat(2049),
            "password=fixture".into(),
            "a\nlabel".into(),
        ] {
            let mut changed = root.clone();
            changed.root_control = value;
            assert!(changed.validate(&assessment.gates).is_err());
        }
    }

    #[test]
    fn security_root_fingerprint_uses_semantics_not_presentation_or_source_lines() {
        let record =
            super::super::report_contract::finding_fixture(&uuid::Uuid::new_v4().to_string());
        let root: RootCause =
            serde_json::from_value(record["assessment"]["rootCause"].clone()).unwrap();
        let original = root.fingerprint();
        let mut moved = root.clone();
        moved.control.start_line += 20;
        moved.control.end_line += 20;
        moved.control.content_hash = "b".repeat(64);
        moved.decision.start_line += 30;
        moved.decision.end_line += 30;
        moved.root_control = format!("  {}  ", root.root_control);
        assert_eq!(original, moved.fingerprint());
        for field in [
            "rootControl",
            "violatedInvariant",
            "sinkOrDecision",
            "attackPathClass",
        ] {
            let mut changed = serde_json::to_value(&root).unwrap();
            changed[field] = serde_json::json!("Different semantic identity");
            let changed: RootCause = serde_json::from_value(changed).unwrap();
            assert_ne!(original, changed.fingerprint());
        }
        let mut other_file = root;
        other_file.control.path = "another.rs".into();
        assert_ne!(original, other_file.fingerprint());
    }
}
