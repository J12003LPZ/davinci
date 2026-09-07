//! Incomplete report checkpoints, separate from immutable final seals.
use super::{controller::RunHandle, snapshot::Snapshot, store::Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub struct PartialReport {
    value: Value,
    sequence: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_partial_preserves_pending_claims_and_cancelled_exit() {
        let coordinator = super::super::controller::ScanCoordinator::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let run = coordinator
            .start(move |run| {
                let mut partial =
                    PartialReport::new(&run, &Snapshot::default(), &Default::default());
                let claim = serde_json::from_value(json!({"title":"Pending evidence",
                "actor":"actor","entrypoint":"input","control":"control","sink":"sink",
                "impact":"impact","prerequisites":[],"locations":[],"proofGaps":[]}))
                .unwrap();
                partial.candidates(&[claim], &[], &[]);
                partial.publish(&run, None).unwrap();
                ready_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                run.finish(Err("cancelled fixture".into()));
            })
            .unwrap();
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        coordinator.abort(None).unwrap();
        release_tx.send(()).unwrap();
        run.wait();
        let value = coordinator.partial_report().unwrap();
        assert_eq!(value["unresolvedLeads"].as_array().unwrap().len(), 1);
        assert_eq!(value["findings"], json!([]));
        assert_eq!(value["coverageComplete"], false);
        assert_eq!(super::super::report::exit_code(&value), 130);
        assert!(run.set_partial_report(value).is_err());
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Artifact {
    pub schema_version: u32,
    pub digest: String,
    pub report: Value,
}

impl PartialReport {
    pub fn new(run: &RunHandle, snapshot: &Snapshot, config: &super::config::ScanConfig) -> Self {
        Self {
            sequence: 0,
            value: json!({"schemaVersion":2,"scanId":run.status().scan_id,
            "generation":run.status().generation,"snapshotId":snapshot.id,"status":"interrupted",
            "partial":true,"coverageComplete":false,"experimental":true,"runtimeTestsExecuted":false,
            "source":{"revisions":snapshot.revisions,"currentSide":snapshot.current_side()},
            "coverage":{"eligibleFiles":snapshot.files.len(),"baselineFiles":snapshot.base_files.len(),
                "reviewedPaths":[],"reviewedBasePaths":[],"skipped":snapshot.skipped,"audits":[]},
            "repositoryMaps":[],"findings":[],"candidates":[],"unresolvedLeads":[],
            "limitations":["Incomplete checkpoint: review did not reach final publication"],
            "failOn":config.fail_on,"methodology":super::skills::manifest()}),
        }
    }

    pub fn maps(&mut self, maps: impl Serialize) {
        self.value["repositoryMaps"] = json!(maps);
    }
    pub fn audits(
        &mut self,
        audits: impl Serialize,
        reviewed: impl Serialize,
        baseline: impl Serialize,
    ) {
        self.value["coverage"]["audits"] = json!(audits);
        self.value["coverage"]["reviewedPaths"] = json!(reviewed);
        self.value["coverage"]["reviewedBasePaths"] = json!(baseline);
    }
    pub fn candidates(
        &mut self,
        pending: &[super::validation::Claim],
        dispositions: &[Value],
        findings: &[Value],
    ) {
        let mut records = dispositions.to_vec();
        records.extend(pending.iter().map(|claim| {
            json!({"claim":claim,"assessment":{"disposition":"deferred",
            "reason":"Validation pending at last saved checkpoint"}})
        }));
        self.value["unresolvedLeads"] = json!(records
            .iter()
            .filter(|record| record["assessment"]["disposition"] == "deferred")
            .collect::<Vec<_>>());
        self.value["candidates"] = json!(records);
        self.value["findings"] = json!(findings);
    }
    pub fn publish(&mut self, run: &RunHandle, store: Option<&Store>) -> Result<(), String> {
        self.sequence += 1;
        self.value["checkpointSequence"] = json!(self.sequence);
        self.value["usage"] = json!(run.status().usage);
        self.value["budgets"] = json!({"cumulative":run.budget_summary()});
        let mut value = self.value.clone();
        super::report::sanitize(&mut value);
        if let Some(store) = store {
            store.partial_report(&value)?;
        }
        run.set_partial_report(value)
    }
}
