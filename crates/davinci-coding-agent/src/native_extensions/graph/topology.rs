//! Immutable execution graph topology, structural validation invariants,
//! and ready-frontier scheduling.

use super::types::{ArtifactKind, Classification, Complexity, GraphRun, Role, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphMode {
    Simple,
    Standard,
    Complex,
}

impl From<Complexity> for GraphMode {
    fn from(c: Complexity) -> Self {
        match c {
            Complexity::Trivial => GraphMode::Simple,
            Complexity::Standard => GraphMode::Standard,
            Complexity::Complex => GraphMode::Complex,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDefinition {
    pub id: String,
    pub role: Role,
    pub expect: ArtifactKind,
    pub required: bool,
    pub allows_mutation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeCondition {
    Always,
    OnSuccess,
    OnFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeDefinition {
    pub from: String,
    pub to: String,
    pub condition: EdgeCondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphDefinition {
    pub graph_id: String,
    pub version: u32,
    pub mode: GraphMode,
    pub nodes: Vec<NodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
}

impl GraphDefinition {
    pub fn node(&self, id: &str) -> Option<&NodeDefinition> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn incoming_edges<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a EdgeDefinition> {
        self.edges.iter().filter(move |e| e.to == id)
    }

    pub fn outgoing_edges<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a EdgeDefinition> {
        self.edges.iter().filter(move |e| e.from == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphTopologyError {
    EmptyGraph,
    UnknownNodeInEdge(String),
    CycleDetected,
    UnreachableRequiredNode(String),
    ReviewBypassed,
    MissingVerificationNode,
    ConcurrentWritersPossible,
}

impl std::fmt::Display for GraphTopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyGraph => write!(f, "graph definition has no nodes"),
            Self::UnknownNodeInEdge(id) => write!(f, "edge references unknown node: {id}"),
            Self::CycleDetected => write!(f, "cycle detected in graph topology"),
            Self::UnreachableRequiredNode(id) => write!(f, "required node is unreachable: {id}"),
            Self::ReviewBypassed => write!(f, "topology allows mutation without review"),
            Self::MissingVerificationNode => {
                write!(f, "verify-success edge without verification node")
            }
            Self::ConcurrentWritersPossible => {
                write!(f, "two mutation-capable writers can be ready concurrently")
            }
        }
    }
}

impl std::error::Error for GraphTopologyError {}

/// Validate the structural invariants of a graph execution definition.
pub fn validate_definition(definition: &GraphDefinition) -> Result<(), GraphTopologyError> {
    if definition.nodes.is_empty() {
        return Err(GraphTopologyError::EmptyGraph);
    }

    let node_ids: HashSet<&str> = definition.nodes.iter().map(|n| n.id.as_str()).collect();

    // 1. Edge references to unknown nodes
    for edge in &definition.edges {
        if !node_ids.contains(edge.from.as_str()) {
            return Err(GraphTopologyError::UnknownNodeInEdge(edge.from.clone()));
        }
        if !node_ids.contains(edge.to.as_str()) {
            return Err(GraphTopologyError::UnknownNodeInEdge(edge.to.clone()));
        }
    }

    // 2. Unbounded cycle detection (Kahn's algorithm for DAG validation)
    let mut in_degrees: HashMap<&str, usize> = HashMap::new();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for id in &node_ids {
        in_degrees.insert(id, 0);
        adjacency.insert(id, Vec::new());
    }
    for edge in &definition.edges {
        *in_degrees.get_mut(edge.to.as_str()).unwrap() += 1;
        adjacency
            .get_mut(edge.from.as_str())
            .unwrap()
            .push(edge.to.as_str());
    }

    let mut queue: VecDeque<&str> = in_degrees
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut visited_count = 0;
    while let Some(node) = queue.pop_front() {
        visited_count += 1;
        if let Some(neighbors) = adjacency.get(node) {
            for neighbor in neighbors {
                let deg = in_degrees.get_mut(neighbor).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(neighbor);
                }
            }
        }
    }

    if visited_count < definition.nodes.len() {
        return Err(GraphTopologyError::CycleDetected);
    }

    // 3. Reachability of required nodes from entry node
    let entry_node = definition.nodes.first().map(|n| n.id.as_str());

    let mut reachable = HashSet::new();
    if let Some(entry) = entry_node {
        let mut reachable_queue = VecDeque::from([entry]);
        while let Some(curr) = reachable_queue.pop_front() {
            if reachable.insert(curr) {
                for edge in definition.outgoing_edges(curr) {
                    if !reachable.contains(edge.to.as_str()) {
                        reachable_queue.push_back(edge.to.as_str());
                    }
                }
            }
        }
    }

    for node in &definition.nodes {
        if node.required && !reachable.contains(node.id.as_str()) {
            return Err(GraphTopologyError::UnreachableRequiredNode(node.id.clone()));
        }
    }

    // 4. Review bypass invariant:
    // In Standard or Complex mode, any mutation-capable writer must reach a reviewer node
    // before the end of the run.
    if matches!(definition.mode, GraphMode::Standard | GraphMode::Complex) {
        let writers: Vec<&NodeDefinition> = definition
            .nodes
            .iter()
            .filter(|n| n.allows_mutation)
            .collect();

        for writer in writers {
            let mut writer_reaches_review = false;
            let mut search_queue = VecDeque::from([writer.id.as_str()]);
            let mut seen = HashSet::new();
            while let Some(curr) = search_queue.pop_front() {
                if !seen.insert(curr) {
                    continue;
                }
                if let Some(node) = definition.node(curr) {
                    if node.role == Role::Reviewer {
                        writer_reaches_review = true;
                        break;
                    }
                }
                for edge in definition.outgoing_edges(curr) {
                    search_queue.push_back(edge.to.as_str());
                }
            }
            if !writer_reaches_review {
                return Err(GraphTopologyError::ReviewBypassed);
            }
        }
    }

    // 5. Verify-success edge without verification node:
    for edge in &definition.edges {
        if edge.from.starts_with("verify") && !definition.nodes.iter().any(|n| n.id == edge.from) {
            return Err(GraphTopologyError::MissingVerificationNode);
        }
    }

    // 6. Two mutation-capable writers that can be ready concurrently:
    let mutation_nodes: Vec<&NodeDefinition> = definition
        .nodes
        .iter()
        .filter(|n| n.allows_mutation)
        .collect();

    for i in 0..mutation_nodes.len() {
        for j in (i + 1)..mutation_nodes.len() {
            let n1 = mutation_nodes[i].id.as_str();
            let n2 = mutation_nodes[j].id.as_str();

            let n1_reaches_n2 = reaches(definition, n1, n2);
            let n2_reaches_n1 = reaches(definition, n2, n1);

            if !n1_reaches_n2 && !n2_reaches_n1 {
                return Err(GraphTopologyError::ConcurrentWritersPossible);
            }
        }
    }

    Ok(())
}

fn reaches(definition: &GraphDefinition, from: &str, to: &str) -> bool {
    let mut queue = VecDeque::from([from]);
    let mut seen = HashSet::new();
    while let Some(curr) = queue.pop_front() {
        if curr == to {
            return true;
        }
        if seen.insert(curr) {
            for edge in definition.outgoing_edges(curr) {
                queue.push_back(edge.to.as_str());
            }
        }
    }
    false
}

/// Build the explicit graph topology definition for simple or complex runs.
pub fn build_definition(mode: GraphMode, classification: &Classification) -> GraphDefinition {
    let version = 1;
    let graph_id = format!("graph-{:?}-v{version}", mode).to_lowercase();

    match mode {
        GraphMode::Simple => {
            let nodes = vec![
                NodeDefinition {
                    id: "classify".to_string(),
                    role: Role::Classifier,
                    expect: ArtifactKind::Classification,
                    required: true,
                    allows_mutation: false,
                },
                NodeDefinition {
                    id: "implement-1".to_string(),
                    role: Role::Writer,
                    expect: ArtifactKind::PatchReport,
                    required: true,
                    allows_mutation: true,
                },
            ];
            let edges = vec![EdgeDefinition {
                from: "classify".to_string(),
                to: "implement-1".to_string(),
                condition: EdgeCondition::OnSuccess,
            }];
            GraphDefinition {
                graph_id,
                version,
                mode,
                nodes,
                edges,
            }
        }
        GraphMode::Standard => {
            let mut nodes = vec![NodeDefinition {
                id: "classify".to_string(),
                role: Role::Classifier,
                expect: ArtifactKind::Classification,
                required: true,
                allows_mutation: false,
            }];
            let mut edges = Vec::new();

            let research_count = classification.research_tasks.len().max(1);
            let mut research_ids = Vec::new();
            for i in 1..=research_count {
                let id = format!("research-{i}");
                let role = classification
                    .research_tasks
                    .get(i - 1)
                    .map(|req| super::roles::role_for_research_kind(req.kind))
                    .unwrap_or(Role::Researcher);
                nodes.push(NodeDefinition {
                    id: id.clone(),
                    role,
                    expect: ArtifactKind::Evidence,
                    required: true,
                    allows_mutation: false,
                });
                edges.push(EdgeDefinition {
                    from: "classify".to_string(),
                    to: id.clone(),
                    condition: EdgeCondition::OnSuccess,
                });
                research_ids.push(id);
            }

            nodes.push(NodeDefinition {
                id: "plan-1".to_string(),
                role: Role::Planner,
                expect: ArtifactKind::Plan,
                required: true,
                allows_mutation: false,
            });
            for r_id in &research_ids {
                edges.push(EdgeDefinition {
                    from: r_id.clone(),
                    to: "plan-1".to_string(),
                    condition: EdgeCondition::OnSuccess,
                });
            }

            nodes.push(NodeDefinition {
                id: "implement-1".to_string(),
                role: Role::Writer,
                expect: ArtifactKind::PatchReport,
                required: true,
                allows_mutation: true,
            });
            edges.push(EdgeDefinition {
                from: "plan-1".to_string(),
                to: "implement-1".to_string(),
                condition: EdgeCondition::OnSuccess,
            });

            nodes.push(NodeDefinition {
                id: "review-1".to_string(),
                role: Role::Reviewer,
                expect: ArtifactKind::Review,
                required: true,
                allows_mutation: false,
            });
            edges.push(EdgeDefinition {
                from: "implement-1".to_string(),
                to: "review-1".to_string(),
                condition: EdgeCondition::OnSuccess,
            });

            GraphDefinition {
                graph_id,
                version,
                mode,
                nodes,
                edges,
            }
        }
        GraphMode::Complex => {
            let mut nodes = vec![NodeDefinition {
                id: "classify".to_string(),
                role: Role::Classifier,
                expect: ArtifactKind::Classification,
                required: true,
                allows_mutation: false,
            }];
            let mut edges = Vec::new();

            let research_count = classification.research_tasks.len().max(1);
            let mut research_ids = Vec::new();
            for i in 1..=research_count {
                let id = format!("research-{i}");
                let role = classification
                    .research_tasks
                    .get(i - 1)
                    .map(|req| super::roles::role_for_research_kind(req.kind))
                    .unwrap_or(Role::Researcher);
                nodes.push(NodeDefinition {
                    id: id.clone(),
                    role,
                    expect: ArtifactKind::Evidence,
                    required: true,
                    allows_mutation: false,
                });
                edges.push(EdgeDefinition {
                    from: "classify".to_string(),
                    to: id.clone(),
                    condition: EdgeCondition::OnSuccess,
                });
                research_ids.push(id);
            }

            nodes.push(NodeDefinition {
                id: "plan-1".to_string(),
                role: Role::Planner,
                expect: ArtifactKind::Plan,
                required: true,
                allows_mutation: false,
            });
            for r_id in &research_ids {
                edges.push(EdgeDefinition {
                    from: r_id.clone(),
                    to: "plan-1".to_string(),
                    condition: EdgeCondition::OnSuccess,
                });
            }

            let milestones = classification
                .milestones
                .as_ref()
                .map(|m| m.len())
                .unwrap_or(1)
                .max(1);

            let mut prev_head = "plan-1".to_string();
            for m in 1..=milestones {
                let impl_id = format!("implement-{m}");
                let rev_id = format!("review-{m}");

                nodes.push(NodeDefinition {
                    id: impl_id.clone(),
                    role: Role::Writer,
                    expect: ArtifactKind::PatchReport,
                    required: true,
                    allows_mutation: true,
                });
                edges.push(EdgeDefinition {
                    from: prev_head,
                    to: impl_id.clone(),
                    condition: EdgeCondition::OnSuccess,
                });

                nodes.push(NodeDefinition {
                    id: rev_id.clone(),
                    role: Role::Reviewer,
                    expect: ArtifactKind::Review,
                    required: true,
                    allows_mutation: false,
                });
                edges.push(EdgeDefinition {
                    from: impl_id,
                    to: rev_id.clone(),
                    condition: EdgeCondition::OnSuccess,
                });

                prev_head = rev_id;
            }

            GraphDefinition {
                graph_id,
                version,
                mode,
                nodes,
                edges,
            }
        }
    }
}

/// Represents the dynamic execution snapshot for scheduling.
#[derive(Debug, Clone, Default)]
pub struct GraphRunState {
    pub succeeded_tasks: HashSet<String>,
    pub failed_tasks: HashSet<String>,
    pub running_tasks: HashSet<String>,
}

impl GraphRunState {
    pub fn from_run(run: &GraphRun) -> Self {
        let mut succeeded_tasks = HashSet::new();
        let mut failed_tasks = HashSet::new();
        let mut running_tasks = HashSet::new();
        for task in &run.tasks {
            match task.status {
                TaskStatus::Succeeded => {
                    succeeded_tasks.insert(task.id.clone());
                }
                TaskStatus::Failed => {
                    failed_tasks.insert(task.id.clone());
                }
                TaskStatus::Running => {
                    running_tasks.insert(task.id.clone());
                }
                _ => {}
            }
        }
        Self {
            succeeded_tasks,
            failed_tasks,
            running_tasks,
        }
    }
}

/// Compute the ready-frontier of nodes eligible for execution.
pub fn ready_nodes(definition: &GraphDefinition, state: &GraphRunState) -> Vec<String> {
    let mut ready = Vec::new();
    let has_running_writer = state.running_tasks.iter().any(|id| {
        definition
            .node(id)
            .map(|n| n.allows_mutation)
            .unwrap_or(false)
    });

    for node in &definition.nodes {
        // Skip already finished or currently running tasks
        if state.succeeded_tasks.contains(&node.id)
            || state.failed_tasks.contains(&node.id)
            || state.running_tasks.contains(&node.id)
        {
            continue;
        }

        // Check if single-writer constraint is preserved
        if node.allows_mutation && has_running_writer {
            continue;
        }

        let incoming: Vec<&EdgeDefinition> = definition.incoming_edges(&node.id).collect();
        let dependencies_met = if incoming.is_empty() {
            true
        } else {
            incoming.iter().all(|edge| match edge.condition {
                EdgeCondition::Always => {
                    state.succeeded_tasks.contains(&edge.from)
                        || state.failed_tasks.contains(&edge.from)
                }
                EdgeCondition::OnSuccess => state.succeeded_tasks.contains(&edge.from),
                EdgeCondition::OnFailure => state.failed_tasks.contains(&edge.from),
            })
        };

        if dependencies_met {
            ready.push(node.id.clone());
        }
    }

    ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{ResearchKind, ResearchRequest, TaskClass};

    fn test_classification() -> Classification {
        Classification {
            task_class: TaskClass::Feature,
            complexity: Complexity::Standard,
            rationale: "test".into(),
            research_tasks: vec![ResearchRequest {
                kind: ResearchKind::CodeSearch,
                focus: "search".into(),
            }],
            milestones: None,
        }
    }

    #[test]
    fn graph_topology_rejection_of_edge_to_unknown_node() {
        let mut def = build_definition(GraphMode::Simple, &test_classification());
        def.edges.push(EdgeDefinition {
            from: "implement-1".into(),
            to: "unknown-node".into(),
            condition: EdgeCondition::Always,
        });
        let err = validate_definition(&def).unwrap_err();
        assert_eq!(
            err,
            GraphTopologyError::UnknownNodeInEdge("unknown-node".into())
        );
    }

    #[test]
    fn graph_topology_rejection_of_unbounded_cycle() {
        let mut def = build_definition(GraphMode::Simple, &test_classification());
        def.edges.push(EdgeDefinition {
            from: "implement-1".into(),
            to: "classify".into(),
            condition: EdgeCondition::Always,
        });
        let err = validate_definition(&def).unwrap_err();
        assert_eq!(err, GraphTopologyError::CycleDetected);
    }

    #[test]
    fn graph_topology_rejection_of_unreachable_required_node() {
        let mut def = build_definition(GraphMode::Simple, &test_classification());
        def.nodes.push(NodeDefinition {
            id: "orphaned".into(),
            role: Role::Researcher,
            expect: ArtifactKind::Evidence,
            required: true,
            allows_mutation: false,
        });
        let err = validate_definition(&def).unwrap_err();
        assert_eq!(
            err,
            GraphTopologyError::UnreachableRequiredNode("orphaned".into())
        );
    }

    #[test]
    fn graph_topology_rejection_of_review_bypass() {
        let mut def = build_definition(GraphMode::Standard, &test_classification());
        // Remove review node and edge
        def.nodes.retain(|n| n.id != "review-1");
        def.edges
            .retain(|e| e.to != "review-1" && e.from != "review-1");
        let err = validate_definition(&def).unwrap_err();
        assert_eq!(err, GraphTopologyError::ReviewBypassed);
    }

    #[test]
    fn graph_topology_rejection_of_missing_verification_node() {
        let mut def = build_definition(GraphMode::Simple, &test_classification());
        def.edges.push(EdgeDefinition {
            from: "verify-1".into(),
            to: "implement-1".into(),
            condition: EdgeCondition::OnSuccess,
        });
        let err = validate_definition(&def).unwrap_err();
        assert_eq!(
            err,
            GraphTopologyError::UnknownNodeInEdge("verify-1".into())
        );
    }

    #[test]
    fn graph_topology_rejection_of_concurrent_writers() {
        let mut def = build_definition(GraphMode::Simple, &test_classification());
        // Add a second writer with no ordering constraint between them
        def.nodes.push(NodeDefinition {
            id: "implement-2".into(),
            role: Role::Writer,
            expect: ArtifactKind::PatchReport,
            required: false,
            allows_mutation: true,
        });
        def.edges.push(EdgeDefinition {
            from: "classify".into(),
            to: "implement-2".into(),
            condition: EdgeCondition::OnSuccess,
        });
        let err = validate_definition(&def).unwrap_err();
        assert_eq!(err, GraphTopologyError::ConcurrentWritersPossible);
    }

    #[test]
    fn graph_topology_valid_definitions_pass_all_invariants() {
        for mode in [GraphMode::Simple, GraphMode::Standard, GraphMode::Complex] {
            let def = build_definition(mode, &test_classification());
            assert!(validate_definition(&def).is_ok());
        }
    }

    #[test]
    fn graph_topology_ready_nodes_frontier_progression() {
        let def = build_definition(GraphMode::Standard, &test_classification());
        let mut state = GraphRunState::default();

        // Initially only root node 'classify' is ready
        assert_eq!(ready_nodes(&def, &state), vec!["classify"]);

        // When classify succeeds, research-1 becomes ready
        state.succeeded_tasks.insert("classify".into());
        assert_eq!(ready_nodes(&def, &state), vec!["research-1"]);

        // When research-1 succeeds, plan-1 becomes ready
        state.succeeded_tasks.insert("research-1".into());
        assert_eq!(ready_nodes(&def, &state), vec!["plan-1"]);

        // When plan-1 succeeds, implement-1 becomes ready
        state.succeeded_tasks.insert("plan-1".into());
        assert_eq!(ready_nodes(&def, &state), vec!["implement-1"]);

        // When implement-1 is running, no other writer can be ready
        state.running_tasks.insert("implement-1".into());
        assert!(ready_nodes(&def, &state).is_empty());
    }
}
