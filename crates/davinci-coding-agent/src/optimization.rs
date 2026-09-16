//! Deterministic, provider-free measurements for the harness optimization gate.

use crate::native_extensions::graph::roles::{initial_worker_tools, role_tools};
use crate::native_extensions::graph::{retry_decision, RetryDecision, Role, WorkerFailureClass};
use crate::native_extensions::{
    memory_freshness, source_state_hash_for_paths, MemoryKind, MemoryRecord,
    SecurityScanController, TokenGovernor, TokenGovernorConfig,
};
use davinci_agent::semantic::text_fallback_definition;
use davinci_agent::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeterministicAblation {
    pub name: String,
    pub baseline_correct: bool,
    pub candidate_correct: bool,
    pub baseline_units: u64,
    pub candidate_units: u64,
    pub unit_name: String,
}

impl DeterministicAblation {
    fn new(
        name: &str,
        baseline_correct: bool,
        candidate_correct: bool,
        baseline_units: u64,
        candidate_units: u64,
        unit_name: &str,
    ) -> Self {
        Self {
            name: name.into(),
            baseline_correct,
            candidate_correct,
            baseline_units,
            candidate_units,
            unit_name: unit_name.into(),
        }
    }
}

struct TempFixture(PathBuf);

impl TempFixture {
    fn new(label: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("davinci-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn governor_content_routing() -> Result<DeterministicAblation, String> {
    let fixture = TempFixture::new("governor-ablation")?;
    let log = (0..160)
        .map(|line| {
            if line == 83 {
                "test auth_refresh ... FAILED\nassertion failed: expected 200 actual 401".into()
            } else {
                format!("test case_{line} ... ok")
            }
        })
        .collect::<Vec<String>>()
        .join("\n");
    let run = |content_aware: bool, suffix: &str| -> Result<(u64, bool), String> {
        let mut governor = TokenGovernor::new(
            format!("offline-{suffix}"),
            TokenGovernorConfig {
                compress_threshold_bytes: 1,
                compress_threshold_lines: 1,
                content_aware,
                store_dir: Some(fixture.path().join(suffix)),
                ..Default::default()
            },
        );
        let result = governor.after_tool(
            "bash",
            &json!({"command": "cargo test"}),
            ToolResult {
                content: log.clone(),
                details: None,
                is_error: false,
            },
        );
        let details = result
            .details
            .as_ref()
            .and_then(|value| value.get("tokenGovernor"))
            .ok_or_else(|| "token governor omitted measurement details".to_string())?;
        let id = details
            .get("outputId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "token governor omitted output id".to_string())?;
        let recovered = governor
            .retrieve(&json!({"id": id}))
            .map_err(|e| e.to_string())?;
        let reconstructed = recovered
            .content
            .lines()
            .map(|line| line.split_once(": ").map_or(line, |(_, body)| body))
            .collect::<Vec<_>>()
            .join("\n");
        Ok((result.content.len() as u64, reconstructed == log))
    };
    let (generic_bytes, generic_correct) = run(false, "generic")?;
    let (specialized_bytes, specialized_correct) = run(true, "specialized")?;
    Ok(DeterministicAblation::new(
        "governor-content-routing-vnext",
        generic_correct,
        specialized_correct,
        generic_bytes,
        specialized_bytes,
        "serialized_output_bytes",
    ))
}

fn graph_deferred_schemas() -> DeterministicAblation {
    let mut authorized = role_tools(Role::Researcher);
    authorized.extend(["mcp.catalog.search".into(), "mcp.catalog.write".into()]);
    let visible = initial_worker_tools(Role::Researcher, &authorized);
    let candidate_correct = visible.iter().all(|tool| authorized.contains(tool))
        && visible.contains(&"tool_search".to_string())
        && visible.contains(&"graph_submit".to_string());
    DeterministicAblation::new(
        "graph-deferred-schemas",
        true,
        candidate_correct,
        authorized.len() as u64,
        visible.len() as u64,
        "visible_tool_schemas",
    )
}

fn graph_failure_aware_retry() -> DeterministicAblation {
    let decision = retry_decision(WorkerFailureClass::Environment, 1);
    DeterministicAblation::new(
        "graph-failure-aware-retry",
        true,
        decision == RetryDecision::Stop,
        2,
        if decision == RetryDecision::Stop {
            1
        } else {
            2
        },
        "worker_attempts",
    )
}

fn memory_freshness_ablation() -> Result<DeterministicAblation, String> {
    let fixture = TempFixture::new("memory-ablation")?;
    let relative = "source.rs".to_string();
    fs::write(fixture.path().join(&relative), "fn before() {}\n")
        .map_err(|error| error.to_string())?;
    let expected = source_state_hash_for_paths(fixture.path(), std::slice::from_ref(&relative))
        .ok_or_else(|| "failed to hash memory source fixture".to_string())?;
    let record = MemoryRecord {
        id: "offline-memory".into(),
        repo_id: "offline".into(),
        kind: MemoryKind::Discovery,
        text: "source-backed discovery".into(),
        source: "agent".into(),
        content_hash: "fixture".into(),
        importance: 1.0,
        created_at: 0,
        embedding: None,
        embedding_identity: None,
        confidence: Some(1.0),
        source_session_id: None,
        source_turn: None,
        verification: None,
        source_paths: vec![relative.clone()],
        source_state_hash: Some(expected),
        verified_at_revision: None,
        use_count: 0,
        last_used_at: None,
        agent_profile_name: None,
        memory_scope: None,
    };
    let fresh = memory_freshness(&record, fixture.path());
    fs::write(fixture.path().join(relative), "fn after() {}\n")
        .map_err(|error| error.to_string())?;
    let stale = memory_freshness(&record, fixture.path());
    Ok(DeterministicAblation::new(
        "memory-freshness",
        fresh == 1.0,
        stale < fresh,
        (fresh * 10_000.0) as u64,
        (stale * 10_000.0) as u64,
        "freshness_score_basis_points",
    ))
}

fn security_incremental() -> Result<DeterministicAblation, String> {
    let fixture = TempFixture::new("security-ablation")?;
    fs::write(
        fixture.path().join("fixture.rs"),
        "fn validate(input: &str) -> bool { !input.is_empty() }\n",
    )
    .map_err(|error| error.to_string())?;
    let mut controller = SecurityScanController::new(fixture.path().to_path_buf());
    let cold = controller.start(None).map_err(|error| error.to_string())?;
    let warm = controller.start(None).map_err(|error| error.to_string())?;
    let cold_work = cold.coverage.files_scanned_cold + cold.coverage.files_rescanned;
    let warm_work = warm.coverage.files_scanned_cold + warm.coverage.files_rescanned;
    Ok(DeterministicAblation::new(
        "security-incremental",
        cold.coverage.files_scanned > 0,
        cold.findings == warm.findings && warm.coverage.files_reused == cold.coverage.files_scanned,
        cold_work as u64,
        warm_work as u64,
        "files_rescanned",
    ))
}

fn semantic_navigation() -> Result<DeterministicAblation, String> {
    let fixture = TempFixture::new("semantic-ablation")?;
    let source = format!(
        "fn target() {{}}\n{}",
        (0..200)
            .map(|index| format!("fn caller_{index}() {{ target(); }}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let path = fixture.path().join("fixture.rs");
    fs::write(&path, &source).map_err(|error| error.to_string())?;
    let result = text_fallback_definition(fixture.path(), "target", Some(&path), None)?;
    let encoded = serde_json::to_vec(&result).map_err(|error| error.to_string())?;
    let correct = result.locations.len() == 1 && result.locations[0].range.start.line == 0;
    Ok(DeterministicAblation::new(
        "semantic-navigation",
        source.contains("fn target()"),
        correct,
        source.len() as u64,
        encoded.len() as u64,
        "returned_navigation_bytes",
    ))
}

/// Execute every optimization ablation against the production implementation.
pub fn deterministic_ablation_measurements() -> Result<Vec<DeterministicAblation>, String> {
    let (root_base_ok, root_candidate_ok, root_base, root_candidate) =
        davinci_agent::deferred_root_schema_ablation()?;
    let (toolbox_base_ok, toolbox_candidate_ok, toolbox_base, toolbox_candidate) =
        davinci_agent::capability_toolbox_ablation()?;
    Ok(vec![
        governor_content_routing()?,
        DeterministicAblation::new(
            "root-context-budgeting",
            root_base_ok,
            root_candidate_ok,
            root_base,
            root_candidate,
            "provider_tool_schema_bytes",
        ),
        DeterministicAblation::new(
            "capability-toolbox-node-query",
            toolbox_base_ok,
            toolbox_candidate_ok,
            toolbox_base,
            toolbox_candidate,
            "provider_tool_schema_bytes",
        ),
        graph_deferred_schemas(),
        graph_failure_aware_retry(),
        memory_freshness_ablation()?,
        security_incremental()?,
        semantic_navigation()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_failure_does_not_burn_graph_retry() {
        let result = graph_failure_aware_retry();
        assert!(result.candidate_correct);
        assert_eq!(result.candidate_units, 1);
    }

    #[test]
    fn graph_deferred_schema_ablation() {
        let result = graph_deferred_schemas();
        assert!(result.candidate_correct);
        assert!(result.candidate_units < result.baseline_units);
    }

    #[test]
    fn graph_retry_policy_ablation() {
        assert_eq!(
            retry_decision(WorkerFailureClass::PlanInvalidated, 1),
            RetryDecision::Replan
        );
        assert_eq!(
            retry_decision(WorkerFailureClass::VerificationFailed, 1),
            RetryDecision::ReviseWriter
        );
        assert_eq!(
            retry_decision(WorkerFailureClass::Environment, 1),
            RetryDecision::Stop
        );
    }
}
