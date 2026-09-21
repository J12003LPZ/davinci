//! Finite native stage and artifact input bindings for saved engineering graphs.
#![allow(dead_code, unused_imports)]

use super::definitions::{
    compute_definition_digest, SavedBudgets, SavedGraphDefinitionV1, SavedStageBinding,
    SavedVerificationPolicy,
};
use super::topology::{GraphDefinition, NodeDefinition};
use super::types::{
    ArtifactKind, EvidenceArtifact, GraphRun, ImplementationPlan, ResearchKind, Role, TaskStatus,
    VerificationResult, VerifyCommandSpec,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn definition_for_run<'a>(saved: Option<&'a str>, generated: &'a str) -> &'a str {
    saved.unwrap_or(generated)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportedStage {
    Classify,
    Research,
    Plan,
    Implement,
    Verify,
    Security,
    Review,
}

impl SupportedStage {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "classify" => Some(Self::Classify),
            "research" | "investigate" => Some(Self::Research),
            "plan" => Some(Self::Plan),
            "implement" => Some(Self::Implement),
            "verify" => Some(Self::Verify),
            "security" | "security-verify" => Some(Self::Security),
            "review" => Some(Self::Review),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Classify => "classify",
            Self::Research => "research",
            Self::Plan => "plan",
            Self::Implement => "implement",
            Self::Verify => "verify",
            Self::Security => "security",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompiledNodeBinding {
    pub node_id: String,
    pub stage: SupportedStage,
    pub input_artifacts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CompiledGraphPlan {
    pub definition_digest: String,
    pub topology: GraphDefinition,
    pub bindings: HashMap<String, CompiledNodeBinding>,
    pub budgets: Option<SavedBudgets>,
    pub verification_policy: Option<SavedVerificationPolicy>,
}

pub fn compile_saved_definition(def: &SavedGraphDefinitionV1) -> Result<CompiledGraphPlan, String> {
    let graph_def = GraphDefinition::from(&def.graph);
    super::topology::validate_definition(&graph_def).map_err(|e| e.to_string())?;

    let mut bindings_map = HashMap::new();
    let node_map: HashMap<&str, &NodeDefinition> =
        def.graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    for binding in &def.bindings {
        let node = node_map
            .get(binding.node_id.as_str())
            .ok_or_else(|| format!("binding references unknown node '{}'", binding.node_id))?;

        let stage = SupportedStage::parse(&binding.stage).ok_or_else(|| {
            format!(
                "unsupported stage '{}' for node '{}'",
                binding.stage, binding.node_id
            )
        })?;

        match stage {
            SupportedStage::Classify => {
                if node.role != Role::Classifier {
                    return Err(format!(
                        "node '{}' with stage 'classify' has incompatible role {:?} (must be Classifier)",
                        binding.node_id, node.role
                    ));
                }
            }
            SupportedStage::Research => {
                if !matches!(
                    node.role,
                    Role::Researcher | Role::TestAnalyzer | Role::Historian
                ) {
                    return Err(format!(
                        "node '{}' with stage 'research' has incompatible role {:?}",
                        binding.node_id, node.role
                    ));
                }
            }
            SupportedStage::Plan => {
                if node.role != Role::Planner {
                    return Err(format!(
                        "node '{}' with stage 'plan' has incompatible role {:?} (must be Planner)",
                        binding.node_id, node.role
                    ));
                }
            }
            SupportedStage::Implement => {
                if node.role != Role::Writer {
                    return Err(format!(
                        "node '{}' with stage 'implement' has incompatible role {:?} (must be Writer)",
                        binding.node_id, node.role
                    ));
                }
            }
            SupportedStage::Verify => {}
            SupportedStage::Security => {
                if node.role != Role::TestAnalyzer {
                    return Err(format!(
                        "node '{}' with stage 'security' has incompatible role {:?} (must be TestAnalyzer)",
                        binding.node_id, node.role
                    ));
                }
            }
            SupportedStage::Review => {
                if node.role != Role::Reviewer {
                    return Err(format!(
                        "node '{}' with stage 'review' has incompatible role {:?} (must be Reviewer)",
                        binding.node_id, node.role
                    ));
                }
            }
        }

        for input_ref in &binding.input_artifacts {
            let is_contract = super::validate::is_known_artifact_contract(input_ref, 1);
            let is_node = node_map.contains_key(input_ref.as_str());
            if !is_contract && !is_node {
                return Err(format!(
                    "node '{}' references unknown input artifact or node '{}'",
                    binding.node_id, input_ref
                ));
            }
        }

        bindings_map.insert(
            binding.node_id.clone(),
            CompiledNodeBinding {
                node_id: binding.node_id.clone(),
                stage,
                input_artifacts: binding.input_artifacts.clone(),
            },
        );
    }

    for node in &def.graph.nodes {
        if !bindings_map.contains_key(&node.id) {
            return Err(format!("missing stage binding for node '{}'", node.id));
        }
    }

    let definition_digest = compute_definition_digest(def);

    Ok(CompiledGraphPlan {
        definition_digest,
        topology: graph_def,
        bindings: bindings_map,
        budgets: def.budgets.clone(),
        verification_policy: def.verification_policy.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_briefing_for_stage(
    node: &NodeDefinition,
    binding: &CompiledNodeBinding,
    goal: &str,
    run: &GraphRun,
    cwd: &Path,
    is_git_repo: bool,
    max_researchers: u32,
    diff: Option<&str>,
    verification: Option<&VerificationResult>,
) -> Result<String, String> {
    match binding.stage {
        SupportedStage::Classify => Ok(super::briefings::classify_briefing(
            &super::briefings::ClassifyInput {
                goal,
                is_git_repo,
                package_scripts: super::config::read_package_scripts(cwd),
                max_researchers,
            },
        )),
        SupportedStage::Research => {
            let kind = match node.role {
                Role::TestAnalyzer => ResearchKind::TestBaseline,
                Role::Historian => ResearchKind::History,
                _ => ResearchKind::CodeSearch,
            };
            Ok(super::briefings::research_briefing(goal, kind, goal))
        }
        SupportedStage::Plan => {
            let mut evidence_digest = String::new();
            for input_ref in &binding.input_artifacts {
                if let Some(task) = run.tasks.iter().find(|t| t.id == *input_ref) {
                    if let Some(ref file) = task.artifact_file {
                        let path = PathBuf::from(&run.cwd).join(file);
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if !evidence_digest.is_empty() {
                                evidence_digest.push_str("\n\n");
                            }
                            evidence_digest.push_str(&content);
                        }
                    }
                }
            }
            Ok(super::briefings::plan_briefing(
                goal,
                &evidence_digest,
                None,
            ))
        }
        SupportedStage::Implement => {
            let mut plan: Option<ImplementationPlan> = None;
            let mut evidence_digest = String::new();
            for input_ref in &binding.input_artifacts {
                if let Some(task) = run.tasks.iter().find(|t| t.id == *input_ref) {
                    if let Some(ref file) = task.artifact_file {
                        let path = PathBuf::from(&run.cwd).join(file);
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(p) = serde_json::from_str::<ImplementationPlan>(&content) {
                                plan = Some(p);
                            } else {
                                if !evidence_digest.is_empty() {
                                    evidence_digest.push_str("\n\n");
                                }
                                evidence_digest.push_str(&content);
                            }
                        }
                    }
                }
            }
            Ok(super::briefings::implement_briefing(
                goal,
                plan.as_ref(),
                &evidence_digest,
                None,
            ))
        }
        SupportedStage::Review => {
            let empty_diff = String::new();
            let effective_diff = diff.unwrap_or(&empty_diff);
            let mut changed_files = Vec::new();
            for task in &run.tasks {
                if let Some(ref m) = task.mutation {
                    for f in &m.files {
                        if !changed_files.contains(&f.path) {
                            changed_files.push(f.path.clone());
                        }
                    }
                }
            }
            let empty_ver = VerificationResult {
                progress: None,
                commands: vec![],
                passed: true,
            };
            let effective_ver = verification.unwrap_or(&empty_ver);
            Ok(super::briefings::review_briefing(
                &super::briefings::ReviewInput {
                    goal,
                    plan: None,
                    diff: effective_diff,
                    changed_files: &changed_files,
                    verification: effective_ver,
                    chunk: None,
                    chunk_summaries: None,
                    required_chunk_ids: None,
                    security: None,
                },
            ))
        }
        SupportedStage::Verify | SupportedStage::Security => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::definitions::SavedGraphTopology;
    use crate::native_extensions::graph::topology::{EdgeCondition, EdgeDefinition, GraphMode};
    use std::collections::BTreeMap;

    fn sample_saved_def() -> SavedGraphDefinitionV1 {
        SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "test-pipeline".into(),
            description: "Test pipeline".into(),
            graph: SavedGraphTopology {
                graph_id: "test-g".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![
                    NodeDefinition {
                        id: "classify".into(),
                        role: Role::Classifier,
                        expect: ArtifactKind::Classification,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "implement-1".into(),
                        role: Role::Writer,
                        expect: ArtifactKind::PatchReport,
                        required: true,
                        allows_mutation: true,
                    },
                ],
                edges: vec![EdgeDefinition {
                    from: "classify".into(),
                    to: "implement-1".into(),
                    condition: EdgeCondition::OnSuccess,
                }],
            },
            bindings: vec![
                SavedStageBinding {
                    node_id: "classify".into(),
                    stage: "classify".into(),
                    input_artifacts: vec![],
                },
                SavedStageBinding {
                    node_id: "implement-1".into(),
                    stage: "implement".into(),
                    input_artifacts: vec!["classify".into()],
                },
            ],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![],
        }
    }

    #[test]
    fn f12_frozen_definition() {
        assert_eq!(
            definition_for_run(Some("saved-hash"), "generated-hash"),
            "saved-hash"
        );
        assert_eq!(definition_for_run(None, "generated-hash"), "generated-hash");
    }

    #[test]
    fn test_compile_valid_saved_definition() {
        let def = sample_saved_def();
        let plan = compile_saved_definition(&def).expect("should compile valid definition");
        assert_eq!(plan.topology.nodes.len(), 2);
        assert_eq!(plan.bindings.len(), 2);
        assert!(!plan.definition_digest.is_empty());
    }

    #[test]
    fn test_compile_missing_binding_rejected() {
        let mut def = sample_saved_def();
        def.bindings.pop();
        let err = compile_saved_definition(&def).unwrap_err();
        assert!(err.contains("missing stage binding for node 'implement-1'"));
    }

    #[test]
    fn test_compile_unsupported_stage_rejected() {
        let mut def = sample_saved_def();
        def.bindings[0].stage = "arbitrary-script".into();
        let err = compile_saved_definition(&def).unwrap_err();
        assert!(err.contains("unsupported stage 'arbitrary-script'"));
    }

    #[test]
    fn test_compile_role_stage_mismatch_rejected() {
        let mut def = sample_saved_def();
        def.bindings[0].stage = "implement".into();
        let err = compile_saved_definition(&def).unwrap_err();
        assert!(err.contains("incompatible"));
    }

    #[test]
    fn test_compile_unknown_input_artifact_rejected() {
        let mut def = sample_saved_def();
        def.bindings[1].input_artifacts = vec!["nonexistent-ref".into()];
        let err = compile_saved_definition(&def).unwrap_err();
        assert!(err.contains("unknown input artifact or node 'nonexistent-ref'"));
    }
}
