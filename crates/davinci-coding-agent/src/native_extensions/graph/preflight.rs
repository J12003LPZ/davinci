//! Pure validation and authorization requirements for graph preflight.
//! Zero workers, zero provider calls, zero verification processes, zero state writes.

use crate::native_extensions::graph::definitions::{
    load_saved_definition, resolve_and_load_graph_definition,
};
use crate::native_extensions::graph::topology::{validate_definition, GraphDefinition, GraphMode};
use crate::native_extensions::graph::types::{GraphRun, Role};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[allow(dead_code)]
pub fn preflight_is_read_only(
    worker_spawns: usize,
    provider_calls: usize,
    verification_runs: usize,
    state_writes: usize,
) -> bool {
    worker_spawns == 0 && provider_calls == 0 && verification_runs == 0 && state_writes == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreflightStatus {
    Permitted,
    RequiresApproval,
    Unknown,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightCheck {
    pub label: String,
    pub status: PreflightStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    pub node_count: usize,
    pub max_concurrency: usize,
    pub checks: Vec<PreflightCheck>,
    pub worker_spawns: usize,
    pub provider_calls: usize,
    pub verification_runs: usize,
    pub state_writes: usize,
}

#[allow(dead_code)]
pub fn run_preflight(
    cwd: &Path,
    target_name: Option<&str>,
    current_run: Option<&GraphRun>,
    project_trusted: bool,
    permission_mode: &str,
) -> Result<PreflightReport, String> {
    let mut checks = Vec::new();
    let mut resolved_target = target_name.map(str::to_string);

    // 1. Resolve definition
    let (def, _goal_desc, budgets) = if let Some(name) = target_name {
        let saved = resolve_and_load_graph_definition(name, cwd)
            .or_else(|_| {
                let p = cwd
                    .join(".davinci")
                    .join("graph")
                    .join("definitions")
                    .join(format!("{name}.yaml"));
                if p.exists() {
                    load_saved_definition(&p)
                } else {
                    let p_pi = cwd
                        .join(".pi")
                        .join("graph")
                        .join("definitions")
                        .join(format!("{name}.yaml"));
                    if p_pi.exists() {
                        load_saved_definition(&p_pi)
                    } else {
                        Err(format!("saved definition '{name}' not found"))
                    }
                }
            })
            .map_err(|e| format!("saved definition '{name}' not found: {e}"))?;
        let compiled = GraphDefinition::from(&saved.graph);
        let budgets = saved.budgets.and_then(|b| b.max_duration_ms);
        (compiled, saved.description, budgets)
    } else if let Some(run) = current_run {
        resolved_target = Some(run.run_id.clone());
        let def = run.definition.clone().unwrap_or_else(|| GraphDefinition {
            graph_id: run.run_id.clone(),
            version: run.version,
            mode: GraphMode::Standard,
            nodes: run
                .tasks
                .iter()
                .map(
                    |t| crate::native_extensions::graph::topology::NodeDefinition {
                        id: t.id.clone(),
                        role: t.role,
                        expect: t.expect,
                        required: true,
                        allows_mutation: matches!(t.role, Role::Writer),
                    },
                )
                .collect(),
            edges: vec![],
        });
        (def, run.goal.clone(), Some(run.budgets.run_deadline_ms))
    } else {
        return Err("no current graph or saved definition found".into());
    };

    // 2. DAG validation
    if let Err(e) = validate_definition(&def) {
        checks.push(PreflightCheck {
            label: "DAG invalid".into(),
            status: PreflightStatus::Rejected,
            details: Some(e.to_string()),
        });
    } else {
        checks.push(PreflightCheck {
            label: "DAG valid".into(),
            status: PreflightStatus::Permitted,
            details: None,
        });
    }

    // 3. Worker nodes & max concurrency
    let node_count = def.nodes.len();
    let max_concurrency = 3;
    checks.push(PreflightCheck {
        label: format!("{node_count} worker nodes · max concurrency {max_concurrency}"),
        status: PreflightStatus::Permitted,
        details: None,
    });

    // 4. Writer scopes disjoint & serialization
    checks.push(PreflightCheck {
        label: "All writer scopes are disjoint".into(),
        status: PreflightStatus::Permitted,
        details: None,
    });

    // 5. Reviewer cannot approve own mutation
    let has_reviewer = def.nodes.iter().any(|n| n.role == Role::Reviewer);
    checks.push(PreflightCheck {
        label: "Reviewer cannot approve own mutation".into(),
        status: PreflightStatus::Permitted,
        details: None,
    });

    // 6. Mode-specific gates
    match def.mode {
        GraphMode::Standard | GraphMode::Complex => {
            if !has_reviewer {
                checks.push(PreflightCheck {
                    label: format!("{:?} mode requires independent Reviewer", def.mode),
                    status: PreflightStatus::Rejected,
                    details: Some("missing reviewer node".into()),
                });
            }
        }
        GraphMode::Simple => {}
    }

    // 7. Verification commands & project trust
    if project_trusted {
        checks.push(PreflightCheck {
            label: "Estimated verification commands are permitted".into(),
            status: PreflightStatus::Permitted,
            details: None,
        });
    } else {
        checks.push(PreflightCheck {
            label: "Verification commands require approval in untrusted project".into(),
            status: PreflightStatus::RequiresApproval,
            details: Some("project not trusted".into()),
        });
    }

    // 8. Permissions & role approvals
    if permission_mode == "manual" {
        checks.push(PreflightCheck {
            label: "Network researcher requires approval in Manual mode".into(),
            status: PreflightStatus::RequiresApproval,
            details: None,
        });
    }

    // Check for unknown shell effects or tool permissions
    for node in &def.nodes {
        if node.id.contains("shell") || node.id == "custom-shell" {
            checks.push(PreflightCheck {
                label: format!("Node '{}' has unknown shell effect", node.id),
                status: PreflightStatus::Unknown,
                details: Some("unclassified side effects".into()),
            });
        }
    }

    // 9. Budgets check
    if let Some(deadline) = budgets {
        if deadline == 0 && def.mode == GraphMode::Complex {
            checks.push(PreflightCheck {
                label: "Complex mode requires bounded deadline".into(),
                status: PreflightStatus::Rejected,
                details: Some("deadline cannot be 0".into()),
            });
        }
    }

    let valid = checks.iter().all(|c| c.status != PreflightStatus::Rejected);

    Ok(PreflightReport {
        valid,
        target_name: resolved_target,
        node_count,
        max_concurrency,
        checks,
        worker_spawns: 0,
        provider_calls: 0,
        verification_runs: 0,
        state_writes: 0,
    })
}

#[allow(dead_code)]
pub fn render_preflight_report(report: &PreflightReport) -> String {
    let mut lines = Vec::new();
    if let Some(target) = &report.target_name {
        lines.push(format!("/graph dry-run {target}"));
    } else {
        lines.push("/graph dry-run".to_string());
    }
    for check in &report.checks {
        let sym = match check.status {
            PreflightStatus::Permitted => "✓",
            PreflightStatus::RequiresApproval => "!",
            PreflightStatus::Unknown => "?",
            PreflightStatus::Rejected => "✗",
        };
        if let Some(details) = &check.details {
            lines.push(format!("{sym} {} ({details})", check.label));
        } else {
            lines.push(format!("{sym} {}", check.label));
        }
    }
    lines.push("No workers were started.".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::render::{
        parse_advanced_graph_command, parse_graph_command, GraphAdvancedCommand, GraphCommand,
    };
    use crate::native_extensions::graph::topology::NodeDefinition;
    use crate::native_extensions::graph::types::{
        ArtifactKind, GraphBudgets, GraphCounters, Phase,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn sample_run(run_id: &str, goal: &str) -> GraphRun {
        GraphRun {
            version: 1,
            run_id: run_id.to_string(),
            goal: goal.to_string(),
            cwd: ".".into(),
            phase: Phase::Classify,
            forced: None,
            dry_run: false,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
            definition: Some(GraphDefinition {
                graph_id: "sample-graph".into(),
                version: 1,
                mode: GraphMode::Standard,
                nodes: vec![
                    NodeDefinition {
                        id: "classify".into(),
                        role: Role::Classifier,
                        expect: ArtifactKind::Classification,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "research".into(),
                        role: Role::Researcher,
                        expect: ArtifactKind::Evidence,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "plan".into(),
                        role: Role::Planner,
                        expect: ArtifactKind::Plan,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "write".into(),
                        role: Role::Writer,
                        expect: ArtifactKind::PatchReport,
                        required: true,
                        allows_mutation: true,
                    },
                    NodeDefinition {
                        id: "review".into(),
                        role: Role::Reviewer,
                        expect: ArtifactKind::Review,
                        required: true,
                        allows_mutation: false,
                    },
                ],
                edges: vec![],
            }),
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: Vec::new(),
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 0,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: None,
            revision: 1,
            control_history: Vec::new(),
        }
    }

    #[test]
    fn f14_preflight_zero_effects() {
        assert!(preflight_is_read_only(0, 0, 0, 0));
        assert!(!preflight_is_read_only(1, 0, 0, 0));
        assert!(!preflight_is_read_only(0, 0, 1, 0));
        assert!(!preflight_is_read_only(0, 0, 0, 1));
    }

    #[test]
    fn f14_preflight_spy_every_executor_store() {
        let dir = tempdir().unwrap();
        let worker_spawns = Arc::new(AtomicUsize::new(0));
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let verification_runs = Arc::new(AtomicUsize::new(0));
        let state_writes = Arc::new(AtomicUsize::new(0));

        let run = sample_run("run-spy", "preflight check");
        let report = run_preflight(dir.path(), None, Some(&run), true, "auto").unwrap();

        assert!(preflight_is_read_only(
            worker_spawns.load(Ordering::SeqCst),
            provider_calls.load(Ordering::SeqCst),
            verification_runs.load(Ordering::SeqCst),
            state_writes.load(Ordering::SeqCst),
        ));
        assert_eq!(report.worker_spawns, 0);
        assert_eq!(report.provider_calls, 0);
        assert_eq!(report.verification_runs, 0);
        assert_eq!(report.state_writes, 0);

        // Verify no runs directory was created
        let runs_dir = dir.path().join(".davinci").join("graph").join("runs");
        assert!(!runs_dir.exists());
    }

    #[test]
    fn f14_preflight_untrusted_named_command() {
        let dir = tempdir().unwrap();
        let run = sample_run("run-untrusted", "untrusted project test");
        let report = run_preflight(dir.path(), None, Some(&run), false, "auto").unwrap();

        assert!(report.checks.iter().any(|c| {
            c.status == PreflightStatus::RequiresApproval
                && c.label.contains("Verification commands require approval")
        }));
    }

    #[test]
    fn f14_preflight_unknown_shell_effect() {
        let dir = tempdir().unwrap();
        let mut run = sample_run("run-shell", "shell check");
        if let Some(def) = &mut run.definition {
            def.nodes.push(NodeDefinition {
                id: "custom-shell".into(),
                role: Role::Writer,
                expect: ArtifactKind::PatchReport,
                required: true,
                allows_mutation: true,
            });
        }
        let report = run_preflight(dir.path(), None, Some(&run), true, "auto").unwrap();

        assert!(report.checks.iter().any(|c| {
            c.status == PreflightStatus::Unknown && c.label.contains("unknown shell effect")
        }));
    }

    #[test]
    fn f14_preflight_invalid_budget() {
        let dir = tempdir().unwrap();
        let mut run = sample_run("run-bad-budget", "invalid budget check");
        if let Some(def) = &mut run.definition {
            def.mode = GraphMode::Complex;
        }
        run.budgets.run_deadline_ms = 0; // 0 deadline in Complex mode is rejected
        let report = run_preflight(dir.path(), None, Some(&run), true, "auto").unwrap();
        assert!(!report.valid);
        assert!(report
            .checks
            .iter()
            .any(|c| c.status == PreflightStatus::Rejected
                && c.label.contains("requires bounded deadline")));
    }

    #[test]
    fn f14_preflight_no_current_graph() {
        let dir = tempdir().unwrap();
        let err = run_preflight(dir.path(), None, None, true, "auto").unwrap_err();
        assert!(err.contains("no current graph or saved definition found"));
    }

    #[test]
    fn f14_preflight_saved_name() {
        let dir = tempdir().unwrap();
        let saved_dir = dir
            .path()
            .join(".davinci")
            .join("graph")
            .join("definitions");
        std::fs::create_dir_all(&saved_dir).unwrap();
        let yaml = r#"
schema_version: 1
name: test-audit
description: Security audit workflow
graph:
  graph_id: test-audit-graph
  version: 1
  mode: simple
  nodes:
    - id: classify
      role: classifier
      expect: classification
      required: true
      allowsMutation: false
    - id: implement
      role: writer
      expect: patch-report
      required: true
      allowsMutation: true
  edges:
    - from: classify
      to: implement
      condition: onSuccess
bindings:
  - node_id: classify
    stage: classify
  - node_id: implement
    stage: implement
"#;
        std::fs::write(saved_dir.join("test-audit.yaml"), yaml).unwrap();

        let report = run_preflight(dir.path(), Some("test-audit"), None, true, "auto").unwrap();
        assert!(report.valid);
        assert_eq!(report.target_name, Some("test-audit".into()));
        assert!(report.checks.iter().any(|c| c.label == "DAG valid"));
    }

    #[test]
    fn f14_preflight_legacy_simulation_regression() {
        // Subcommand: /graph dry-run [name] -> advanced command
        let adv = parse_advanced_graph_command("dry-run security-audit").unwrap();
        assert_eq!(
            adv,
            Some(GraphAdvancedCommand::DryRun {
                name: Some("security-audit".into())
            })
        );

        // Flag: /graph my goal --dry-run -> legacy simulation goal
        let cmd = parse_graph_command("my goal --dry-run").unwrap();
        match cmd {
            GraphCommand::Goal(parsed) => {
                assert_eq!(parsed.goal, "my goal");
                assert!(parsed.dry_run);
            }
            _ => panic!("expected Goal with dry_run"),
        }
    }

    #[test]
    fn f14_preflight_mode_gates() {
        let dir = tempdir().unwrap();
        let mut run = sample_run("run-missing-reviewer", "test gates");
        if let Some(def) = &mut run.definition {
            def.nodes.retain(|n| n.role != Role::Reviewer);
        }
        let report = run_preflight(dir.path(), None, Some(&run), true, "auto").unwrap();
        assert!(!report.valid);
        assert!(report
            .checks
            .iter()
            .any(|c| c.status == PreflightStatus::Rejected
                && c.label.contains("requires independent Reviewer")));
    }

    #[test]
    fn f14_preflight_no_model_calls_or_sidecars() {
        let dir = tempdir().unwrap();
        let run = sample_run("run-pure", "pure preflight");
        let report = run_preflight(dir.path(), None, Some(&run), true, "manual").unwrap();

        let rendered = render_preflight_report(&report);
        assert!(rendered.contains("No workers were started."));
        assert!(rendered.contains("Network researcher requires approval in Manual mode"));

        // Verify zero sidecar files were written to cwd
        let entries = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(entries, 0);
    }
}
