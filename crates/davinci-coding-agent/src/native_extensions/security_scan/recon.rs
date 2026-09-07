//! Native source-bound reconnaissance contract; no upstream TypeScript equivalent.

use super::{
    snapshot::Snapshot,
    validation::{validate_location, Location},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryMap {
    pub sources: Vec<MappedSource>,
    pub units: Vec<ReviewUnit>,
    pub environment_assumptions: Vec<String>,
    #[serde(default)]
    pub resolved_assumptions: Vec<AssumptionResolution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssumptionResolution {
    pub unit_id: Option<String>,
    pub assumption: String,
    pub rationale: String,
    pub locations: Vec<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurface {
    Shipped,
    Example,
    Fixture,
    Build,
    Vendored,
    OperatorConfiguration,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MappedSource {
    pub location: Location,
    pub surface: ProductSurface,
    pub rationale: String,
    pub language: String,
    pub build_context: String,
    pub unit_ids: Vec<String>,
    /// Required when no entrypoint or privileged decision was identified.
    pub no_unit_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDisposition {
    Reviewed,
    Deferred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewUnit {
    pub id: String,
    pub entrypoint: String,
    pub actor_capabilities: String,
    pub assets: String,
    pub trust_boundary: String,
    pub closest_control: String,
    pub sensitive_operation: String,
    pub data_flow: String,
    pub related_configuration: String,
    pub locations: Vec<Location>,
    pub disposition: CoverageDisposition,
    pub rationale: String,
    pub unknowns: Vec<String>,
}

impl RepositoryMap {
    pub fn unresolved_assignments(&self, audit: &Self) -> Vec<String> {
        self.environment_assumptions
            .iter()
            .map(|assumption| (None, assumption))
            .chain(self.units.iter().flat_map(|unit| {
                unit.unknowns
                    .iter()
                    .map(move |unknown| (Some(unit.id.as_str()), unknown))
            }))
            .filter(|(unit, assumption)| {
                !audit.resolved_assumptions.iter().any(|resolution| {
                    resolution.unit_id.as_deref() == *unit && resolution.assumption == **assumption
                })
            })
            .map(|(unit, assumption)| format!("{}: {assumption}", unit.unwrap_or("environment")))
            .collect()
    }
    /// Coordinator assignments survive an independent audit even when omitted or
    /// relabeled by its output. A matching name alone cannot replace source identity.
    pub fn unreviewed_units(&self, audit: &Self) -> Vec<String> {
        self.units
            .iter()
            .filter(|assigned| {
                !audit.units.iter().any(|reviewed| {
                    reviewed.id == assigned.id
                        && reviewed.disposition == CoverageDisposition::Reviewed
                        && assigned.locations.iter().all(|expected| {
                            reviewed.locations.iter().any(|actual| {
                                actual.path == expected.path
                                    && actual.snapshot_side == expected.snapshot_side
                                    && actual.content_hash == expected.content_hash
                                    && actual.start_line == expected.start_line
                                    && actual.end_line == expected.end_line
                                    && actual.role == expected.role
                            })
                        })
                })
            })
            .map(|unit| unit.id.clone())
            .collect()
    }

    pub fn validate(&self, snapshot: &Snapshot) -> Result<(), String> {
        if self.sources.len() > snapshot.source_count() || self.units.len() > 256 {
            return Err("repository map exceeds inventory or unit limits".into());
        }
        texts(&self.environment_assumptions)?;
        if self.resolved_assumptions.len() > 256 {
            return Err("too many assumption resolutions".into());
        }
        let mut resolutions = BTreeSet::new();
        for resolution in &self.resolved_assumptions {
            text(&resolution.assumption)?;
            text(&resolution.rationale)?;
            if resolution.locations.is_empty()
                || resolution.locations.len() > 32
                || !resolutions.insert((&resolution.unit_id, &resolution.assumption))
                || resolution
                    .unit_id
                    .as_ref()
                    .is_some_and(|id| !self.units.iter().any(|unit| &unit.id == id))
            {
                return Err("invalid or duplicate assumption resolution".into());
            }
            for location in &resolution.locations {
                anchor(location, snapshot)?;
            }
        }
        let mut units = BTreeMap::new();
        for unit in &self.units {
            unit.validate(snapshot)?;
            if units.insert(unit.id.as_str(), unit).is_some() {
                return Err("duplicate repository review unit".into());
            }
        }
        let mut sources = BTreeSet::new();
        let mut assigned = BTreeSet::new();
        for source in &self.sources {
            anchor(&source.location, snapshot)?;
            let key = (&source.location.snapshot_side, &source.location.path);
            if !sources.insert(key) {
                return Err("duplicate mapped source".into());
            }
            for value in [&source.rationale, &source.language, &source.build_context] {
                text(value)?;
            }
            if source.unit_ids.len() > 256 {
                return Err("source unit references exceed limit".into());
            }
            if source.unit_ids.is_empty() {
                text(
                    source
                        .no_unit_reason
                        .as_deref()
                        .ok_or("unexplained unmapped source")?,
                )?;
            } else if source.no_unit_reason.is_some() {
                return Err("source cannot both reference units and claim no unit".into());
            }
            let mut references = BTreeSet::new();
            for id in &source.unit_ids {
                let unit = units.get(id.as_str()).ok_or("unknown source review unit")?;
                if !references.insert(id)
                    || !unit.locations.iter().any(|anchor| {
                        anchor.path == source.location.path
                            && anchor.snapshot_side == source.location.snapshot_side
                    })
                {
                    return Err("duplicate or unanchored source review unit".into());
                }
                assigned.insert(id.as_str());
            }
        }
        if assigned.len() != units.len()
            || self.units.iter().any(|unit| {
                unit.locations.iter().any(|anchor| {
                    !self.sources.iter().any(|source| {
                        source.location.path == anchor.path
                            && source.location.snapshot_side == anchor.snapshot_side
                            && source.unit_ids.contains(&unit.id)
                    })
                })
            })
        {
            return Err("review unit contains an unaccounted source".into());
        }
        Ok(())
    }

    /// Missing source rows remain in the denominator, including baseline sources.
    pub fn complete(&self, snapshot: &Snapshot) -> bool {
        self.validate(snapshot).is_ok()
            && snapshot
                .sources()
                .filter(|(side, path, _)| snapshot.is_target(path, side))
                .all(|(side, path, _)| {
                    self.sources.iter().any(|source| {
                        source.location.path == path && source.location.snapshot_side == side
                    })
                })
            && snapshot.skipped.is_empty()
            && snapshot.conflicts.is_empty()
            && self.environment_assumptions.is_empty()
            && self
                .sources
                .iter()
                .all(|source| !matches!(source.surface, ProductSurface::Unknown))
            && self.units.iter().all(|unit| {
                unit.disposition == CoverageDisposition::Reviewed && unit.unknowns.is_empty()
            })
    }
}

impl ReviewUnit {
    fn validate(&self, snapshot: &Snapshot) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            || self.locations.is_empty()
            || self.locations.len() > 32
        {
            return Err("invalid review unit identity or anchor count".into());
        }
        for value in [
            &self.entrypoint,
            &self.actor_capabilities,
            &self.assets,
            &self.trust_boundary,
            &self.closest_control,
            &self.sensitive_operation,
            &self.data_flow,
            &self.related_configuration,
            &self.rationale,
        ] {
            text(value)?;
        }
        texts(&self.unknowns)?;
        for location in &self.locations {
            anchor(location, snapshot)?;
        }
        if !self
            .locations
            .iter()
            .any(|location| matches!(location.role.as_str(), "entrypoint" | "control"))
        {
            return Err("review unit requires an entrypoint or control anchor".into());
        }
        Ok(())
    }
}

fn anchor(location: &Location, snapshot: &Snapshot) -> Result<(), String> {
    if !["base", "index-base", "index-ours", "index-theirs"]
        .contains(&location.snapshot_side.as_str())
        && location.snapshot_side != snapshot.current_side()
    {
        return Err("repository map requires a canonical snapshot side".into());
    }
    validate_location(location, snapshot)
}

fn text(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 8192 {
        return Err("repository map text is empty or exceeds limit".into());
    }
    Ok(())
}

fn texts(values: &[String]) -> Result<(), String> {
    if values.len() > 32 {
        return Err("repository map assumptions exceed limit".into());
    }
    values.iter().try_for_each(|value| text(value))
}

pub fn result_schema() -> serde_json::Value {
    let location = serde_json::json!({"path":"repository-relative path", "startLine":1,
        "endLine":1,"contentHash":"snapshot content hash","snapshotSide":"worktree|head|base|index-base|index-ours|index-theirs",
        "role":"source|entrypoint|control|sink|configuration|test"});
    serde_json::json!({
        "sources":[{"location":location,"surface":"shipped|example|fixture|build|vendored|operator_configuration|unknown",
            "rationale":"source-backed product classification", "language":"language or explicit unknown",
            "buildContext":"observed build/deployment metadata or explicit unknown",
            "unitIds":["unit-id"],"noUnitReason":"null when units exist; otherwise evidence-backed reason no entrypoint or privileged decision exists"}],
        "units":[{"id":"unique ASCII alphanumeric, underscore or hyphen, max 64 bytes",
            "entrypoint":"entrypoint or privileged decision", "actorCapabilities":"who controls which inputs with what authority",
            "assets":"protected assets", "trustBoundary":"supported security guarantee and crossing",
            "closestControl":"actual check semantics or observed absence", "sensitiveOperation":"sink or decision",
            "dataFlow":"callers, transformations and downstream uses", "relatedConfiguration":"source-backed configuration or explicit unknown",
            "locations":[location],"disposition":"reviewed|deferred", "rationale":"analysis performed or reason deferred",
            "unknowns":[]}],
        "environmentAssumptions":[],
        "resolvedAssumptions":[{"unitId":null,"assumption":"exact coordinator environment assumption, or unit unknown with its unitId",
            "rationale":"what retrieved source establishes and why the uncertainty is resolved","locations":[location]}]
    })
}

#[cfg(test)]
mod tests {
    use super::super::{sha256_hex, snapshot::SourceFile};
    use super::*;

    fn fixture() -> (Snapshot, RepositoryMap) {
        let text = "fn main() {}\n";
        let file = SourceFile {
            hash: sha256_hex(text.as_bytes()),
            text: text.into(),
        };
        let snapshot = Snapshot {
            files: [("main.rs".into(), file.clone())].into(),
            ..Snapshot::default()
        };
        let map = RepositoryMap {
            sources: vec![MappedSource {
                location: Location {
                    path: "main.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    content_hash: file.hash,
                    role: "entrypoint".into(),
                    snapshot_side: "worktree".into(),
                },
                surface: ProductSurface::Shipped,
                rationale: "Empty main function".into(),
                language: "Rust".into(),
                build_context: "Standalone fixture".into(),
                unit_ids: vec![],
                no_unit_reason: Some("No input or privileged operation in empty main".into()),
            }],
            units: vec![],
            environment_assumptions: vec![],
            resolved_assumptions: vec![],
        };
        (snapshot, map)
    }

    #[test]
    fn security_recon_rejects_invented_and_stale_source() {
        let (snapshot, mut map) = fixture();
        assert!(map.validate(&snapshot).is_ok());
        map.sources[0].location.path = "invented.rs".into();
        assert!(map.validate(&snapshot).is_err());
        map.sources[0].location.path = "main.rs".into();
        map.sources[0].location.content_hash = "stale".into();
        assert!(map.validate(&snapshot).is_err());
    }

    #[test]
    fn security_mapping_uncertainty_requires_explicit_source_bound_resolution() {
        let (snapshot, mut planned) = fixture();
        planned
            .environment_assumptions
            .push("Deployment path is unknown".into());
        let mut audit = planned.clone();
        audit.environment_assumptions.clear();
        assert_eq!(planned.unresolved_assignments(&audit).len(), 1);
        audit.resolved_assumptions.push(AssumptionResolution {
            unit_id: None,
            assumption: "Deployment path is unknown".into(),
            rationale: "Fixture has only the empty main entry".into(),
            locations: vec![audit.sources[0].location.clone()],
        });
        assert!(audit.validate(&snapshot).is_ok());
        assert!(planned.unresolved_assignments(&audit).is_empty());
        audit.resolved_assumptions[0].locations.clear();
        assert!(audit.validate(&snapshot).is_err());
    }

    #[test]
    fn security_recon_requires_unique_source_accounting_and_missing_rows_are_partial() {
        let (snapshot, mut map) = fixture();
        assert!(map.complete(&snapshot));
        map.sources.push(map.sources[0].clone());
        assert!(map.validate(&snapshot).is_err());
        map.sources.clear();
        assert!(map.validate(&snapshot).is_ok());
        assert!(!map.complete(&snapshot));
    }

    #[test]
    fn security_recon_rejects_dangling_units_and_unexplained_exclusions() {
        let (snapshot, mut map) = fixture();
        map.sources[0].unit_ids.push("invented".into());
        assert!(map.validate(&snapshot).is_err());
        map.sources[0].unit_ids.clear();
        map.sources[0].no_unit_reason = None;
        assert!(map.validate(&snapshot).is_err());
    }

    #[test]
    fn security_recon_baseline_cannot_be_replaced_by_current_alias() {
        let (mut snapshot, mut map) = fixture();
        snapshot.base_files = snapshot.files.clone();
        assert!(!map.complete(&snapshot));
        let mut second = map.sources[0].clone();
        second.location.snapshot_side = "current".into();
        map.sources.push(second);
        assert!(map.validate(&snapshot).is_err());
        map.sources[1].location.snapshot_side = "base".into();
        assert!(map.complete(&snapshot));
    }

    #[test]
    fn security_recon_preserves_index_side_and_never_completes_unresolved_merge() {
        use super::super::snapshot::{ConflictSource, ConflictStage};
        let (mut snapshot, mut map) = fixture();
        let identity = ConflictStage {
            path: "main.rs".into(),
            stage: 2,
            object_id: "a".repeat(40),
            mode: "100644".into(),
        };
        snapshot.conflicts.push(identity.clone());
        snapshot.conflict_sources.push(ConflictSource {
            identity,
            source: SourceFile {
                hash: sha256_hex(b"fn ours() {}\n"),
                text: "fn ours() {}\n".into(),
            },
            supporting: false,
        });
        let mut index = map.sources[0].clone();
        index.location.snapshot_side = "index-ours".into();
        index.location.content_hash = sha256_hex(b"fn ours() {}\n");
        map.sources.push(index);
        assert!(map.validate(&snapshot).is_ok());
        assert!(
            !map.complete(&snapshot),
            "reading all stages does not resolve the merge"
        );
        map.sources[1].location.snapshot_side = "worktree".into();
        assert!(map.validate(&snapshot).is_err());
        map.sources[1].location.snapshot_side = "index-theirs".into();
        assert!(map.validate(&snapshot).is_err());
    }

    #[test]
    fn security_recon_unit_requires_causal_fields_anchors_and_disposition() {
        let (snapshot, mut map) = fixture();
        let unit: ReviewUnit = serde_json::from_value(serde_json::json!({
            "id":"main","entrypoint":"main function","actorCapabilities":"process invoker",
            "assets":"process state","trustBoundary":"no crossing observed in empty main",
            "closestControl":"no checks needed in empty function","sensitiveOperation":"process entry",
            "dataFlow":"no parameters or callees","relatedConfiguration":"no configuration read",
            "locations":[map.sources[0].location],"disposition":"reviewed",
            "rationale":"body has no operations","unknowns":[]
        })).unwrap();
        map.sources[0].unit_ids = vec!["main".into()];
        map.sources[0].no_unit_reason = None;
        map.units.push(unit);
        assert!(map.complete(&snapshot));
        let assigned = map.clone();
        assert!(assigned.unreviewed_units(&map).is_empty());
        map.units[0].id = "renamed-to-hide-assignment".into();
        assert_eq!(assigned.unreviewed_units(&map), ["main"]);
        map.units[0].id = "main".into();
        map.units[0].locations[0].content_hash = "replaced source".into();
        assert_eq!(assigned.unreviewed_units(&map), ["main"]);
        map.units[0].locations = assigned.units[0].locations.clone();
        map.units[0].disposition = CoverageDisposition::Deferred;
        assert_eq!(assigned.unreviewed_units(&map), ["main"]);
        assert!(map.validate(&snapshot).is_ok());
        assert!(!map.complete(&snapshot));
        map.units[0].disposition = CoverageDisposition::Reviewed;
        map.units[0]
            .unknowns
            .push("Deployment not established".into());
        assert!(!map.complete(&snapshot));
        map.units[0].unknowns.clear();
        map.units[0].locations[0].role = "source".into();
        assert!(map.validate(&snapshot).is_err());
        map.units[0].locations[0].role = "entrypoint".into();
        map.units[0].data_flow.clear();
        assert!(map.validate(&snapshot).is_err());
        map.units[0].data_flow = "No downstream calls".into();
        map.units.push(map.units[0].clone());
        assert!(map.validate(&snapshot).is_err());
    }

    #[test]
    fn security_recon_unknown_surfaces_and_environment_never_prove_complete() {
        let (snapshot, mut map) = fixture();
        map.sources[0].surface = ProductSurface::Unknown;
        assert!(!map.complete(&snapshot));
        map.sources[0].surface = ProductSurface::Shipped;
        map.environment_assumptions
            .push("Unproven deployment".into());
        assert!(!map.complete(&snapshot));
        map.environment_assumptions = vec!["unknown".into(); 33];
        assert!(map.validate(&snapshot).is_err());
        let mut value = serde_json::to_value(map).unwrap();
        value["authorizedTools"] = serde_json::json!(["bash"]);
        assert!(serde_json::from_value::<RepositoryMap>(value).is_err());
    }
}
