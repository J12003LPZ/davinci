//! The half of the graph that lives INSIDE a worker process.
//!
//! Activated only when the parent set the worker environment:
//!
//! ```text
//! PI_GRAPH_ROLE          - the worker's role (drives bash policy + tool bans)
//! PI_GRAPH_EXPECT        - the artifact kind graph_submit validates against
//! PI_GRAPH_ARTIFACT_PATH - absolute path graph_submit writes to
//! PI_GRAPH_EXTRA_TOOLS   - the parent's full --tools allowlist
//! ```
//!
//! `graph_submit` is the worker's single exit door: validation errors come
//! back as tool errors, so the worker model fixes its own output inside its
//! own loop — no respawn, no prose parsing.

use super::roles::{is_bash_command_allowed, role_bash_policy, role_tools, GRAPH_SUBMIT_TOOL};
use super::store::write_artifact;
use super::types::{ArtifactKind, BashPolicy, Role};
use super::validate::{artifact_contract, artifact_schema, validate_artifact};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by the first accepted `graph_submit` in this worker process. The parent
/// already has everything it asked for, so anything the model does afterwards
/// is unpaid-for work on a finished node.
static SUBMITTED: AtomicBool = AtomicBool::new(false);

/// One process is one node, so `SUBMITTED` is process-wide. Tests share a
/// process: they take this lock and reset the flag rather than leaking it into
/// each other.
#[cfg(test)]
pub(crate) static SUBMIT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn submit_test_guard() -> std::sync::MutexGuard<'static, ()> {
    let guard = SUBMIT_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    SUBMITTED.store(false, Ordering::Relaxed);
    guard
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphWorkerContext {
    pub role: Role,
    pub expect: ArtifactKind,
    pub artifact_path: PathBuf,
    pub bash_policy: BashPolicy,
    pub allowed_tools: BTreeSet<String>,
}

impl GraphWorkerContext {
    /// `None` for an ordinary pi session (or a misconfigured worker): nothing
    /// is registered, and the parent treats a missing artifact as failure, so a
    /// broken worker cannot silently succeed.
    pub fn from_env() -> Option<Self> {
        Self::from_parts(
            std::env::var("PI_GRAPH_ROLE").ok().as_deref(),
            std::env::var("PI_GRAPH_EXPECT").ok().as_deref(),
            std::env::var("PI_GRAPH_ARTIFACT_PATH").ok().as_deref(),
            std::env::var("PI_GRAPH_EXTRA_TOOLS").ok().as_deref(),
        )
    }

    pub fn from_parts(
        role: Option<&str>,
        expect: Option<&str>,
        artifact_path: Option<&str>,
        extra_tools: Option<&str>,
    ) -> Option<Self> {
        let role = Role::parse(role?)?;
        let expect = ArtifactKind::parse(expect?)?;
        let artifact_path = artifact_path.filter(|path| !path.is_empty())?;
        let mut allowed_tools: BTreeSet<String> = role_tools(role).into_iter().collect();
        for tool in extra_tools.unwrap_or_default().split(',') {
            let tool = tool.trim();
            if !tool.is_empty() {
                allowed_tools.insert(tool.to_string());
            }
        }
        Some(Self {
            role,
            expect,
            artifact_path: PathBuf::from(artifact_path),
            bash_policy: role_bash_policy(role),
            allowed_tools,
        })
    }

    /// The tool carries the whole contract — a typed schema and the prose —
    /// so the worker model never has to guess a field name and burn a
    /// validation round trip (or give up, which is what an untyped
    /// `artifact` property used to produce).
    pub fn tool_spec(&self) -> (String, Value) {
        let mut artifact = artifact_schema(self.expect);
        artifact["description"] = Value::String(format!(
            "The {} artifact as a JSON object (not a string)",
            self.expect
        ));
        (
            format!(
                "Submit your final {} artifact. Call this exactly once, as your last action.\n\n{}",
                self.expect,
                artifact_contract(self.expect)
            ),
            json!({
                "type": "object",
                "properties": {"artifact": artifact},
                "required": ["artifact"],
            }),
        )
    }

    /// Validate and persist the worker's deliverable.
    pub fn submit(&self, params: &Value) -> Result<String, String> {
        let raw = params.get("artifact").unwrap_or(&Value::Null);
        let candidate = match raw {
            Value::String(text) => serde_json::from_str::<Value>(text).map_err(|_| {
                "artifact was a string but not valid JSON; pass the artifact as a JSON object"
                    .to_string()
            })?,
            other => other.clone(),
        };
        let artifact = validate_artifact(self.expect, &candidate).map_err(|errors| {
            format!(
                "artifact does not match the \"{}\" contract:\n- {}\nFix these fields and call {GRAPH_SUBMIT_TOOL} again.",
                self.expect,
                errors.join("\n- ")
            )
        })?;
        write_artifact(&self.artifact_path, &artifact)
            .map_err(|error| format!("could not write the artifact: {error}"))?;
        SUBMITTED.store(true, Ordering::Relaxed);
        Ok(format!(
            "Artifact recorded ({}). Your task is complete.",
            self.expect
        ))
    }

    /// Defense in depth behind `--tools`: block mutation tools for read-only
    /// roles even if the allowlist was mangled, and judge bash by its command
    /// text rather than by its name.
    pub fn block_reason(&self, tool_name: &str, args: &Value) -> Option<String> {
        if SUBMITTED.load(Ordering::Relaxed) {
            return Some(format!(
                "this {} node already submitted its artifact; stop now rather than doing more work",
                self.role
            ));
        }
        if !self.allowed_tools.contains(tool_name) {
            return Some(format!(
                "tool \"{tool_name}\" is not available to the {} role",
                self.role
            ));
        }
        if tool_name != "bash" && tool_name != "powershell" {
            return None;
        }
        let command = args.get("command").and_then(Value::as_str)?;
        match is_bash_command_allowed(self.bash_policy, command) {
            super::roles::BashDecision::Allowed => None,
            super::roles::BashDecision::Blocked(reason) => {
                Some(format!("{} role: {reason}. Command: {command}", self.role))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn submit_guard() -> std::sync::MutexGuard<'static, ()> {
        super::submit_test_guard()
    }

    fn context(role: Role, expect: ArtifactKind, path: &std::path::Path) -> GraphWorkerContext {
        GraphWorkerContext::from_parts(
            Some(role.as_str()),
            Some(expect.as_str()),
            Some(&path.to_string_lossy()),
            Some("retrieve_output"),
        )
        .expect("context")
    }

    #[test]
    fn an_ordinary_session_activates_nothing() {
        let _guard = submit_guard();
        assert!(GraphWorkerContext::from_parts(None, None, None, None).is_none());
        assert!(GraphWorkerContext::from_parts(
            Some("writer"),
            Some("patch-report"),
            Some(""),
            None
        )
        .is_none());
        assert!(
            GraphWorkerContext::from_parts(Some("wizard"), Some("plan"), Some("/tmp/a"), None)
                .is_none()
        );
    }

    #[test]
    fn a_valid_artifact_is_written_and_acknowledged() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let path = dir.path().join("artifact.json");
        let context = context(Role::Reviewer, ArtifactKind::Review, &path);
        let message = context
            .submit(&json!({"artifact": {"verdict": "approve", "issues": [], "notes": "ok"}}))
            .expect("accepted");
        assert!(message.contains("Artifact recorded (review)"));
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(stored["verdict"], "approve");
    }

    #[test]
    fn a_stringified_artifact_is_parsed_before_validation() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let path = dir.path().join("artifact.json");
        let context = context(Role::Reviewer, ArtifactKind::Review, &path);
        let payload = json!({"artifact": "{\"verdict\":\"approve\",\"issues\":[],\"notes\":\"\"}"});
        assert!(context.submit(&payload).is_ok());
    }

    #[test]
    fn an_invalid_artifact_comes_back_as_fixable_field_errors() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let path = dir.path().join("artifact.json");
        let context = context(Role::Reviewer, ArtifactKind::Review, &path);
        let error = context
            .submit(&json!({"artifact": {"verdict": "maybe"}}))
            .unwrap_err();
        assert!(error.contains("verdict must be one of"));
        assert!(error.contains("call graph_submit again"));
        assert!(!path.exists());
    }

    #[test]
    fn a_read_only_role_cannot_reach_a_mutation_tool() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let context = context(
            Role::Researcher,
            ArtifactKind::Evidence,
            &dir.path().join("a.json"),
        );
        assert!(context.block_reason("write", &json!({})).is_some());
        assert!(context.block_reason("read", &json!({})).is_none());
        // The parent's extra tools stay reachable.
        assert!(context
            .block_reason("retrieve_output", &json!({}))
            .is_none());
    }

    #[test]
    fn bash_is_judged_by_its_command_text_not_its_name() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let context = context(
            Role::Researcher,
            ArtifactKind::Evidence,
            &dir.path().join("a.json"),
        );
        assert!(context
            .block_reason("bash", &json!({"command": "rg needle"}))
            .is_none());
        let blocked = context
            .block_reason("bash", &json!({"command": "rm -rf target"}))
            .expect("blocked");
        assert!(blocked.contains("researcher role"));
        assert!(blocked.contains("rm -rf target"));
    }

    #[test]
    fn the_writer_is_stopped_at_git_state_changes() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let context = context(
            Role::Writer,
            ArtifactKind::PatchReport,
            &dir.path().join("a.json"),
        );
        assert!(context
            .block_reason("bash", &json!({"command": "cargo build"}))
            .is_none());
        assert!(context
            .block_reason("bash", &json!({"command": "git push origin main"}))
            .is_some());
    }

    #[test]
    fn a_node_that_already_submitted_may_not_keep_working() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let context = context(
            Role::Reviewer,
            ArtifactKind::Review,
            &dir.path().join("artifact.json"),
        );
        assert!(context.block_reason("read", &json!({})).is_none());
        context
            .submit(&json!({"artifact": {"verdict": "approve", "issues": [], "notes": ""}}))
            .expect("accepted");
        let blocked = context.block_reason("read", &json!({})).expect("blocked");
        assert!(blocked.contains("already submitted its artifact"));
    }

    #[test]
    fn the_submit_schema_names_the_expected_artifact() {
        let _guard = submit_guard();
        let dir = tempdir().unwrap();
        let context = context(
            Role::Planner,
            ArtifactKind::Plan,
            &dir.path().join("a.json"),
        );
        let (description, parameters) = context.tool_spec();
        assert!(description.contains("final plan artifact"));
        assert_eq!(parameters["required"][0], "artifact");
    }
}
