//! Typed graph-engineer runtime.
//!
//! The controller is deliberately bounded: plans have a fixed topology,
//! dependencies are validated before execution, retries and budgets are
//! explicit, and all persisted state is written beneath the repository's
//! .pi/graph/runs directory.

use pi_agent::{ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphMode {
    Simple,
    Complex,
}

impl Default for GraphMode {
    fn default() -> Self {
        Self::Simple
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphPhase {
    Classify,
    Plan,
    Implement,
    Verify,
    Complete,
    Aborted,
}

impl Default for GraphPhase {
    fn default() -> Self {
        Self::Classify
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Pending,
    Ready,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphNode {
    pub id: String,
    pub phase: GraphPhase,
    pub role: String,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub status: NodeStatus,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub max_attempts: u32,
    #[serde(default)]
    pub authority: Vec<String>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphArtifact {
    pub id: String,
    pub node_id: String,
    pub path: String,
    pub sha256: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphRun {
    pub id: String,
    pub goal: String,
    pub mode: GraphMode,
    pub phase: GraphPhase,
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub artifacts: Vec<GraphArtifact>,
    #[serde(default)]
    pub budget: u64,
    #[serde(default)]
    pub spent: u64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub aborted: bool,
}

impl GraphRun {
    pub fn validate(&self) -> Result<(), String> {
        validate_nodes(&self.nodes)?;
        if self.goal.trim().is_empty() {
            return Err("graph goal cannot be empty".into());
        }
        if self.spent > self.budget && self.budget > 0 {
            return Err("graph budget exceeded".into());
        }
        Ok(())
    }

    pub fn ready_nodes(&self) -> Vec<String> {
        let statuses = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.status))
            .collect::<HashMap<_, _>>();
        self.nodes
            .iter()
            .filter(|node| {
                matches!(node.status, NodeStatus::Pending | NodeStatus::Ready)
                    && node.depends_on.iter().all(|dependency| {
                        statuses.get(dependency.as_str()) == Some(&NodeStatus::Succeeded)
                    })
            })
            .map(|node| node.id.clone())
            .collect()
    }

    pub fn with_node_status(
        &self,
        node_id: &str,
        status: NodeStatus,
        error: Option<String>,
    ) -> Result<Self, String> {
        let mut next = self.clone();
        let node_index = next
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .ok_or_else(|| format!("unknown graph node: {node_id}"))?;
        let current_status = next.nodes[node_index].status;
        let node_id_for_error = next.nodes[node_index].id.clone();
        let dependencies = next.nodes[node_index].depends_on.clone();
        if matches!(current_status, NodeStatus::Succeeded | NodeStatus::Skipped)
            && status != current_status
        {
            return Err("terminal graph nodes cannot transition".into());
        }
        let statuses = next
            .nodes
            .iter()
            .map(|candidate| (candidate.id.as_str(), candidate.status))
            .collect::<HashMap<_, _>>();
        let dependencies_complete = dependencies
            .iter()
            .all(|dependency| statuses.get(dependency.as_str()) == Some(&NodeStatus::Succeeded));
        match status {
            NodeStatus::Ready | NodeStatus::Running if !dependencies_complete => {
                return Err(format!(
                    "node {node_id_for_error} dependencies are not complete"
                ));
            }
            NodeStatus::Running => {
                if !matches!(
                    current_status,
                    NodeStatus::Pending | NodeStatus::Ready | NodeStatus::Failed
                ) {
                    return Err("node is not runnable".into());
                }
                if next.nodes[node_index].attempts >= next.nodes[node_index].max_attempts {
                    return Err(format!("node {node_id_for_error} exceeded retry budget"));
                }
                let node = &mut next.nodes[node_index];
                node.attempts = node.attempts.saturating_add(1);
            }
            NodeStatus::Succeeded | NodeStatus::Failed if current_status != NodeStatus::Running => {
                return Err("node must be running before it can finish".into());
            }
            _ => {}
        }
        let node = &mut next.nodes[node_index];
        node.status = status;
        node.error = error;
        next.phase = phase_for_nodes(&next.nodes, next.aborted);
        next.validate()?;
        Ok(next)
    }

    pub fn with_spent(&self, amount: u64) -> Result<Self, String> {
        let mut next = self.clone();
        next.spent = next.spent.saturating_add(amount);
        next.validate()?;
        Ok(next)
    }

    pub fn abort(&self) -> Self {
        let mut next = self.clone();
        next.aborted = true;
        next.phase = GraphPhase::Aborted;
        next
    }
}

pub fn validate_nodes(nodes: &[GraphNode]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for node in nodes {
        if node.id.trim().is_empty() || !ids.insert(node.id.as_str()) {
            return Err(format!("duplicate or empty graph node id: {}", node.id));
        }
        if node.max_attempts == 0 {
            return Err(format!("node {} has no retry budget", node.id));
        }
        if node
            .authority
            .iter()
            .any(|authority| authority.trim().is_empty())
        {
            return Err(format!("node {} has empty authority", node.id));
        }
    }
    for node in nodes {
        for dependency in &node.depends_on {
            if !ids.contains(dependency.as_str()) {
                return Err(format!(
                    "node {} depends on unknown node {dependency}",
                    node.id
                ));
            }
        }
    }
    let mut indegree = nodes
        .iter()
        .map(|node| (node.id.as_str(), node.depends_on.len()))
        .collect::<HashMap<_, _>>();
    let mut edges = HashMap::<&str, Vec<&str>>::new();
    for node in nodes {
        for dependency in &node.depends_on {
            edges
                .entry(dependency.as_str())
                .or_default()
                .push(node.id.as_str());
        }
    }
    let mut queue = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect::<VecDeque<_>>();
    let mut visited = 0;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        for child in edges.get(id).into_iter().flatten() {
            if let Some(degree) = indegree.get_mut(child) {
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(child);
                }
            }
        }
    }
    (visited == nodes.len())
        .then_some(())
        .ok_or_else(|| "graph contains a dependency cycle".into())
}

fn phase_for_nodes(nodes: &[GraphNode], aborted: bool) -> GraphPhase {
    if aborted {
        return GraphPhase::Aborted;
    }
    if nodes
        .iter()
        .all(|node| matches!(node.status, NodeStatus::Succeeded | NodeStatus::Skipped))
    {
        return GraphPhase::Complete;
    }
    nodes
        .iter()
        .find(|node| !matches!(node.status, NodeStatus::Succeeded | NodeStatus::Skipped))
        .map(|node| node.phase)
        .unwrap_or(GraphPhase::Complete)
}

pub fn plan_graph(
    goal: &str,
    mode: GraphMode,
    run_id: impl Into<String>,
    dry_run: bool,
) -> GraphRun {
    let run_id = run_id.into();
    let mut nodes = vec![
        GraphNode {
            id: "classify".into(),
            phase: GraphPhase::Classify,
            role: "planner".into(),
            depends_on: vec![],
            status: NodeStatus::Ready,
            attempts: 0,
            max_attempts: 1,
            authority: vec!["read".into()],
            artifact_ids: vec![],
            error: None,
        },
        GraphNode {
            id: "plan".into(),
            phase: GraphPhase::Plan,
            role: "planner".into(),
            depends_on: vec!["classify".into()],
            status: NodeStatus::Pending,
            attempts: 0,
            max_attempts: 2,
            authority: vec!["read".into(), "plan".into()],
            artifact_ids: vec![],
            error: None,
        },
        GraphNode {
            id: "verify".into(),
            phase: GraphPhase::Verify,
            role: "reviewer".into(),
            depends_on: vec!["plan".into()],
            status: NodeStatus::Pending,
            attempts: 0,
            max_attempts: 2,
            authority: vec!["read".into(), "verify".into()],
            artifact_ids: vec![],
            error: None,
        },
    ];
    if mode == GraphMode::Complex {
        nodes.insert(
            2,
            GraphNode {
                id: "implement".into(),
                phase: GraphPhase::Implement,
                role: "worker".into(),
                depends_on: vec!["plan".into()],
                status: NodeStatus::Pending,
                attempts: 0,
                max_attempts: 3,
                authority: vec!["read".into(), "write".into(), "test".into()],
                artifact_ids: vec![],
                error: None,
            },
        );
        if let Some(verify) = nodes.iter_mut().find(|node| node.id == "verify") {
            verify.depends_on = vec!["implement".into()];
        }
    }
    let run = GraphRun {
        id: run_id,
        goal: goal.trim().to_string(),
        mode,
        phase: GraphPhase::Classify,
        nodes,
        artifacts: vec![],
        budget: if mode == GraphMode::Complex { 12 } else { 6 },
        spent: 0,
        dry_run,
        aborted: false,
    };
    debug_assert!(run.validate().is_ok());
    run
}

#[derive(Debug, Clone)]
pub struct GraphStore {
    root: PathBuf,
}

impl GraphStore {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            root: cwd.into().join(".pi").join("graph").join("runs"),
        }
    }

    pub fn save(&self, run: &GraphRun) -> Result<(), ToolError> {
        run.validate().map_err(ToolError::Failed)?;
        fs::create_dir_all(&self.root).map_err(|err| ToolError::Failed(err.to_string()))?;
        let path = self.path(&run.id)?;
        let content =
            serde_json::to_vec_pretty(run).map_err(|err| ToolError::Failed(err.to_string()))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let temporary = self
            .root
            .join(format!(".{}.{}.{}.tmp", run.id, std::process::id(), nonce));
        if let Err(error) = fs::write(&temporary, content) {
            let _ = fs::remove_file(&temporary);
            return Err(ToolError::Failed(error.to_string()));
        }
        match fs::rename(&temporary, &path) {
            Ok(()) => Ok(()),
            Err(_rename_error) if path.exists() => {
                // Windows does not replace an existing file with rename. Move the
                // previous complete snapshot aside, then publish the new one.
                let backup = self.root.join(format!(".{}.{}.bak", run.id, nonce));
                if let Err(error) = fs::rename(&path, &backup) {
                    let _ = fs::remove_file(&temporary);
                    return Err(ToolError::Failed(error.to_string()));
                }
                match fs::rename(&temporary, &path) {
                    Ok(()) => {
                        let _ = fs::remove_file(backup);
                        Ok(())
                    }
                    Err(error) => {
                        let _ = fs::rename(&backup, &path);
                        let _ = fs::remove_file(&temporary);
                        Err(ToolError::Failed(error.to_string()))
                    }
                }
            }
            Err(rename_error) => {
                let _ = fs::remove_file(&temporary);
                Err(ToolError::Failed(rename_error.to_string()))
            }
        }
    }

    pub fn load(&self, run_id: &str) -> Result<GraphRun, ToolError> {
        let path = self.path(run_id)?;
        let content = fs::read(path).map_err(|err| ToolError::Failed(err.to_string()))?;
        let run: GraphRun =
            serde_json::from_slice(&content).map_err(|err| ToolError::Failed(err.to_string()))?;
        run.validate().map_err(ToolError::Failed)?;
        Ok(run)
    }

    pub fn latest(&self) -> Result<Option<GraphRun>, ToolError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ToolError::Failed(error.to_string())),
        };
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| ToolError::Failed(error.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(run_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if !is_safe_run_id(run_id) {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            candidates.push((modified, run_id.to_string()));
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let Some((_, run_id)) = candidates.into_iter().next() else {
            return Ok(None);
        };
        self.load(&run_id).map(Some)
    }

    fn path(&self, run_id: &str) -> Result<PathBuf, ToolError> {
        if !is_safe_run_id(run_id) {
            return Err(ToolError::Failed("invalid graph run id".into()));
        }
        Ok(self.root.join(format!("{run_id}.json")))
    }
}

fn is_safe_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

#[derive(Debug, Clone)]
pub struct GraphController {
    cwd: PathBuf,
    pub store: GraphStore,
    current: Option<GraphRun>,
}

impl Default for GraphController {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl GraphController {
    pub fn new(cwd: PathBuf) -> Self {
        let store = GraphStore::new(cwd.clone());
        Self {
            cwd,
            store,
            current: None,
        }
    }

    pub fn start(
        &mut self,
        goal: &str,
        mode: GraphMode,
        dry_run: bool,
    ) -> Result<GraphRun, ToolError> {
        if goal.trim().is_empty() {
            return Err(ToolError::Failed("graph goal cannot be empty".into()));
        }
        let mode_tag = match mode {
            GraphMode::Simple => "simple",
            GraphMode::Complex => "complex",
        };
        let identity = format!("{}\0{}\0{}", goal.trim(), mode_tag, dry_run);
        let run_id = format!(
            "run-{}",
            &crate::native_extensions::vector_memory::sha256_hex(identity.as_bytes())[..12]
        );
        let run = plan_graph(goal, mode, run_id, dry_run);
        self.store.save(&run)?;
        self.current = Some(run.clone());
        Ok(run)
    }

    pub fn current(&self) -> Option<GraphRun> {
        self.current.clone()
    }

    pub fn status(&mut self, run_id: Option<&str>) -> Result<GraphRun, ToolError> {
        if let Some(run) = self
            .current
            .as_ref()
            .filter(|run| run_id.is_none() || Some(run.id.as_str()) == run_id)
        {
            return Ok(run.clone());
        }
        if let Some(id) = run_id {
            return self.store.load(id);
        }
        let run = self
            .store
            .latest()?
            .ok_or_else(|| ToolError::Failed("no graph run is active".into()))?;
        self.current = Some(run.clone());
        Ok(run)
    }

    pub fn transition(
        &mut self,
        node_id: &str,
        status: NodeStatus,
        error: Option<String>,
    ) -> Result<GraphRun, ToolError> {
        let run = self.status(None)?;
        let next = run
            .with_node_status(node_id, status, error)
            .map_err(ToolError::Failed)?;
        self.store.save(&next)?;
        self.current = Some(next.clone());
        Ok(next)
    }

    pub fn abort(&mut self) -> Result<GraphRun, ToolError> {
        let run = self.status(None)?.abort();
        self.store.save(&run)?;
        self.current = Some(run.clone());
        Ok(run)
    }

    pub fn execute_tool(&mut self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        let run = match name {
            "graph_run" => {
                let goal = args.get("goal").and_then(Value::as_str).unwrap_or_default();
                let mode = match args.get("mode").and_then(Value::as_str) {
                    Some("complex") => GraphMode::Complex,
                    _ => GraphMode::Simple,
                };
                let dry_run = args.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
                self.start(goal, mode, dry_run)?
            }
            "graph_status" => self.status(args.get("runId").and_then(Value::as_str))?,
            _ => return Err(ToolError::Unknown(name.to_string())),
        };
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&run).unwrap_or_else(|_| "{}".into()),
            is_error: false,
            details: Some(json!({"graph": run})),
        })
    }

    pub fn command(&mut self, name: &str, args: &str) -> Result<Option<Value>, String> {
        match name {
            "graph" => {
                let mode = if args.contains("--complex") {
                    GraphMode::Complex
                } else {
                    GraphMode::Simple
                };
                let dry_run = args.contains("--dry-run");
                let goal = args
                    .replace("--complex", "")
                    .replace("--simple", "")
                    .replace("--dry-run", "");
                Ok(Some(
                    serde_json::to_value(
                        self.start(goal.trim(), mode, dry_run)
                            .map_err(|err| err.to_string())?,
                    )
                    .map_err(|err| err.to_string())?,
                ))
            }
            "graph-status" | "graph-view" | "graph-resume" => Ok(Some(
                serde_json::to_value(self.status(None).map_err(|err| err.to_string())?)
                    .map_err(|err| err.to_string())?,
            )),
            "graph-abort" => Ok(Some(
                serde_json::to_value(self.abort().map_err(|err| err.to_string())?)
                    .map_err(|err| err.to_string())?,
            )),
            _ => Ok(None),
        }
    }

    /// Launch a child worker only after the caller has validated the node's
    /// role and authority. No shell is involved, so goal text cannot become
    /// command syntax.
    pub fn spawn_worker(
        &self,
        executable: &Path,
        node: &GraphNode,
    ) -> Result<std::process::Child, ToolError> {
        if node.role != "worker" {
            return Err(ToolError::Failed(
                "only worker nodes may spawn children".into(),
            ));
        }
        Command::new(executable)
            .arg("--offline")
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| ToolError::Failed(err.to_string()))
    }
}

pub fn role_allows(role: &str, operation: &str) -> bool {
    match role {
        "planner" => matches!(operation, "read" | "plan" | "verify"),
        "worker" => matches!(operation, "read" | "write" | "test"),
        "reviewer" => matches!(operation, "read" | "verify"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn complex_plan_is_acyclic_and_ready_nodes_are_dependency_safe() {
        let run = plan_graph("ship feature", GraphMode::Complex, "run-test", true);
        run.validate().unwrap();
        assert_eq!(run.ready_nodes(), vec!["classify"]);
        let next = run
            .with_node_status("classify", NodeStatus::Running, None)
            .unwrap()
            .with_node_status("classify", NodeStatus::Succeeded, None)
            .unwrap();
        assert_eq!(next.ready_nodes(), vec!["plan"]);
    }

    #[test]
    fn cycle_and_unknown_dependency_are_rejected() {
        let mut run = plan_graph("bad", GraphMode::Simple, "run-test", true);
        run.nodes[0].depends_on.push("plan".into());
        assert!(run.validate().is_err());
        run.nodes[0].depends_on.clear();
        run.nodes[1].depends_on = vec!["missing".into()];
        assert!(run.validate().is_err());
    }

    #[test]
    fn role_policy_is_narrow_and_explicit() {
        assert!(role_allows("worker", "write"));
        assert!(!role_allows("planner", "write"));
        assert!(!role_allows("unknown", "read"));
    }

    #[test]
    fn transitions_require_ready_dependencies_and_bounded_retries() {
        let run = plan_graph("retry", GraphMode::Simple, "run-test", true);
        assert!(run
            .with_node_status("plan", NodeStatus::Ready, None)
            .is_err());
        assert!(run
            .with_node_status("plan", NodeStatus::Running, None)
            .is_err());
        assert!(run
            .with_node_status("classify", NodeStatus::Succeeded, None)
            .is_err());

        let run = run
            .with_node_status("classify", NodeStatus::Running, None)
            .expect("first attempt");
        let run = run
            .with_node_status("classify", NodeStatus::Failed, Some("failed".into()))
            .expect("record failure");
        assert!(run
            .with_node_status("classify", NodeStatus::Running, None)
            .is_err());
    }

    #[test]
    fn controller_run_ids_include_mode_and_dry_run() {
        let dir = tempdir().unwrap();
        let mut controller = GraphController::new(dir.path().to_path_buf());
        let simple = controller
            .start("same goal", GraphMode::Simple, false)
            .unwrap();
        let complex = controller
            .start("same goal", GraphMode::Complex, false)
            .unwrap();
        let dry_run = controller
            .start("same goal", GraphMode::Simple, true)
            .unwrap();

        assert_ne!(simple.id, complex.id);
        assert_ne!(simple.id, dry_run.id);
        assert_ne!(complex.id, dry_run.id);
    }

    #[test]
    fn a_new_controller_can_resume_the_latest_persisted_run() {
        let dir = tempdir().unwrap();
        let first = GraphController::new(dir.path().to_path_buf());
        let mut first = first;
        let started = first
            .start("persist me", GraphMode::Complex, false)
            .unwrap();

        let mut restarted = GraphController::new(dir.path().to_path_buf());
        let resumed = restarted.status(None).unwrap();
        assert_eq!(resumed.id, started.id);
        assert_eq!(resumed.goal, "persist me");
    }
}
