//! Policy-owned approval choices; no upstream TypeScript permission counterpart.
//! See vendor/davinci/packages/coding-agent/src/modes/rpc/rpc-mode.ts for the
//! host transport, and permission.rs for the existing policy authority.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalChallenge {
    pub schema_version: u16,
    pub id: Uuid,
    pub call_id: String,
    pub action_digest: String,
    pub policy_revision: u64,
    pub contract_revision: Option<u64>,
    pub mode: String,
    pub action_label: String,
    pub display_target: String,
    pub reason: String,
    pub legal_choices: Vec<ApprovalChoice>,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalReply {
    pub challenge_id: Uuid,
    pub choice_id: String,
    pub instructions: Option<String>,
}

/// Trusted host adapter, never installed from model-generated tool arguments.
pub type ApprovalCallback =
    dyn Fn(&crate::ToolApprovalRequest, &ApprovalChallenge) -> ApprovalReply + Send + Sync;

#[derive(Clone)]
pub struct ApprovalResponder(pub Arc<ApprovalCallback>);

impl std::fmt::Debug for ApprovalResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApprovalResponder(..)")
    }
}

const APPROVAL_TTL: Duration = Duration::from_secs(600);
const MAX_PENDING: usize = 16;

#[derive(Debug)]
struct Pending {
    challenge: ApprovalChallenge,
    issued: Instant,
}

/// Ephemeral engine-owned request table; it is never session-serialized.
#[derive(Debug)]
pub(crate) struct ApprovalRegistry {
    owner: Uuid,
    pending: Mutex<HashMap<Uuid, Pending>>,
}

impl Default for ApprovalRegistry {
    fn default() -> Self {
        Self {
            owner: Uuid::new_v4(),
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl ApprovalRegistry {
    pub(crate) fn revoke_all(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
    }

    pub(crate) fn digest(
        &self,
        request: &crate::ToolApprovalRequest,
        cwd: &std::path::Path,
        runtime: Option<&crate::RuntimeHandle>,
    ) -> String {
        // Include complete arguments, never bounded display text, in authority.
        let value = serde_json::json!([
            self.owner,
            runtime.map(|rt| (&rt.run_id, &rt.agent_id, &rt.session_id)),
            cwd.as_os_str().as_encoded_bytes(),
            request.tool_call_id,
            crate::permission::normalize_tool_name(&request.tool),
            request.args,
            request.subject,
            request.mode.as_str()
        ]);
        crate::tool_ledger::canonical_arguments_digest(&value)
    }

    pub(crate) fn issue(
        self: &Arc<Self>,
        request: &crate::ToolApprovalRequest,
        digest: String,
        revision: u64,
        now_ms: u64,
    ) -> Result<PendingApproval, &'static str> {
        let mut pending = self.pending.lock().unwrap_or_else(|err| err.into_inner());
        pending.retain(|_, entry| {
            entry.issued.elapsed() < APPROVAL_TTL && now_ms < entry.challenge.expires_at_ms
        });
        if pending.len() >= MAX_PENDING {
            return Err("approval queue is full; retry after a pending request resolves");
        }
        let challenge = ApprovalChallenge {
            schema_version: 1,
            id: Uuid::new_v4(),
            call_id: request.tool_call_id.clone(),
            action_digest: digest,
            policy_revision: revision,
            // Task-contract authority is integrated through F05, not inferred here.
            contract_revision: None,
            mode: request.mode.as_str().into(),
            action_label: display_text(&request.tool),
            display_target: display_text(&request.subject),
            reason: request.summary.clone(),
            legal_choices: request.legal_choices.clone(),
            expires_at_ms: now_ms.saturating_add(APPROVAL_TTL.as_millis() as u64),
        };
        pending.insert(
            challenge.id,
            Pending {
                challenge: challenge.clone(),
                issued: Instant::now(),
            },
        );
        Ok(PendingApproval {
            registry: self.clone(),
            challenge,
        })
    }
}

/// Dropping a request revokes it, including callback panic/cancellation paths.
pub(crate) struct PendingApproval {
    registry: Arc<ApprovalRegistry>,
    pub(crate) challenge: ApprovalChallenge,
}

impl PendingApproval {
    pub(crate) fn resolve(
        &self,
        reply: &ApprovalReply,
        digest: &str,
        revision: Option<u64>,
        now_ms: u64,
    ) -> Result<GrantScope, &'static str> {
        let entry = self
            .registry
            .pending
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&self.challenge.id)
            .ok_or("approval is no longer pending")?;
        let challenge = entry.challenge;
        if reply.challenge_id != challenge.id
            || challenge.action_digest != digest
            || Some(challenge.policy_revision) != revision
            || now_ms >= challenge.expires_at_ms
            || entry.issued.elapsed() >= APPROVAL_TTL
        {
            return Err("approval identity or freshness changed; request approval again");
        }
        let choice = challenge
            .legal_choices
            .iter()
            .find(|choice| choice.id == reply.choice_id)
            .ok_or("approval choice was not offered by policy")?;
        if reply
            .instructions
            .as_ref()
            .is_some_and(|text| text.len() > 4096)
            || (reply.instructions.is_some() && choice.scope != GrantScope::DenyWithInstructions)
        {
            return Err("invalid approval instructions");
        }
        Ok(choice.scope)
    }
}

impl Drop for PendingApproval {
    fn drop(&mut self) {
        self.registry
            .pending
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&self.challenge.id);
    }
}

impl ApprovalReply {
    /// Adapt a trusted host's legacy selection; the engine still validates it.
    pub fn from_legacy(
        challenge: &ApprovalChallenge,
        decision: crate::ToolApprovalDecision,
    ) -> Self {
        use crate::ToolApprovalDecision::*;
        Self {
            challenge_id: challenge.id,
            choice_id: match decision {
                AllowOnce => "once",
                AllowForSession => "session",
                AllowAlways => "project",
                Deny => "deny",
            }
            .into(),
            instructions: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantScope {
    Once,
    Session,
    Project,
    Deny,
    DenyWithInstructions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalChoice {
    pub id: String,
    pub scope: GrantScope,
    pub label: String,
}

pub(crate) fn display_text(text: &str) -> String {
    bounded_text(text, 1024)
}

pub(crate) fn instruction_text(text: &str) -> String {
    bounded_text(text, 4096)
}

fn bounded_text(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        let ch =
            if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                ch
            };
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out
}

pub fn offer_scopes(
    high_risk: bool,
    session_legal: bool,
    project_legal: bool,
) -> Vec<ApprovalChoice> {
    use GrantScope::*;
    [
        ("once", Once, "allow once", true),
        (
            "session",
            Session,
            "allow for this session",
            !high_risk && session_legal,
        ),
        (
            "project",
            Project,
            "always allow in this project",
            !high_risk && project_legal,
        ),
        ("deny", Deny, "deny", true),
        (
            "deny_with_instructions",
            DenyWithInstructions,
            "deny with instructions",
            true,
        ),
    ]
    .into_iter()
    .filter(|(_, _, _, legal)| *legal)
    .map(|(id, scope, label, _)| ApprovalChoice {
        id: id.into(),
        scope,
        label: label.into(),
    })
    .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn reply_matches(
    challenge: &str,
    reply: &str,
    issued_digest: &str,
    current_digest: &str,
    issued_revision: u64,
    current_revision: u64,
    now_ms: u64,
    expires_ms: u64,
) -> bool {
    challenge == reply
        && issued_digest == current_digest
        && issued_revision == current_revision
        && now_ms < expires_ms
}

pub fn grant_applies(
    issued_session: &str,
    current_session: &str,
    canonical_subject: &str,
    requested_subject: &str,
    explicit_deny: bool,
) -> bool {
    !explicit_deny && issued_session == current_session && canonical_subject == requested_subject
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> crate::ToolApprovalRequest {
        let policy = crate::PermissionPolicy::new(crate::PermissionMode::Ask);
        let crate::PermissionVerdict::Ask(request) = policy.decide(
            "call",
            "write",
            &serde_json::json!({"path":"approval.txt", "content":"fixture"}),
            std::path::Path::new("."),
        ) else {
            panic!("expected Ask")
        };
        request
    }

    #[test]
    fn f01_stale_reply_and_duplicate_resolution_fail_closed() {
        for failure in [
            "none",
            "id",
            "digest",
            "revision",
            "expiry",
            "choice",
            "instructions",
        ] {
            let registry = Arc::new(ApprovalRegistry::default());
            let pending = registry.issue(&request(), "digest".into(), 7, 0).unwrap();
            let mut reply = ApprovalReply::from_legacy(
                &pending.challenge,
                crate::ToolApprovalDecision::AllowOnce,
            );
            if failure == "id" {
                reply.challenge_id = Uuid::new_v4();
            }
            if failure == "choice" {
                reply.choice_id = "forged".into();
            }
            if failure == "instructions" {
                reply.instructions = Some("grant everything".into());
            }
            let result = pending.resolve(
                &reply,
                if failure == "digest" {
                    "changed"
                } else {
                    "digest"
                },
                Some(if failure == "revision" { 8 } else { 7 }),
                if failure == "expiry" { 600_000 } else { 1 },
            );
            assert_eq!(result.is_ok(), failure == "none", "{failure}");
            assert!(pending.resolve(&reply, "digest", Some(7), 1).is_err());
            assert!(registry.pending.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn f01_pending_capacity_and_drop_release_slots() {
        let registry = Arc::new(ApprovalRegistry::default());
        let mut leases = (0..MAX_PENDING)
            .map(|_| registry.issue(&request(), "digest".into(), 1, 0).unwrap())
            .collect::<Vec<_>>();
        assert!(registry.issue(&request(), "digest".into(), 1, 0).is_err());
        leases.pop();
        assert!(registry.issue(&request(), "digest".into(), 1, 0).is_ok());
        drop(leases);
        assert!(registry.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn f01_digest_binds_complete_arguments_and_runtime_identity() {
        let registry = ApprovalRegistry::default();
        let other = ApprovalRegistry::default();
        let request = request();
        let cwd = std::path::Path::new(".");
        let digest = registry.digest(&request, cwd, None);
        assert_ne!(digest, other.digest(&request, cwd, None));
        let mut changed = request.clone();
        changed.args["content"] = serde_json::json!("different");
        assert_ne!(digest, registry.digest(&changed, cwd, None));
        let runtime = crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        );
        let bound = registry.digest(&request, cwd, Some(&runtime));
        assert_ne!(digest, bound);
        let mut other_run = runtime.clone();
        other_run.run_id = crate::RunId::new();
        assert_ne!(bound, registry.digest(&request, cwd, Some(&other_run)));
    }

    #[test]
    fn f01_session_load_revokes_pending_and_session_consent() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = crate::Agent::new("fixture");
        agent.set_permission_mode(crate::PermissionMode::Ask);
        agent
            .permissions
            .lock()
            .unwrap()
            .remember("write(ordinary.txt)");
        let revision = agent.permissions.lock().unwrap().revision().unwrap();
        let registry = agent.approval_registry.clone();
        let lease = registry
            .issue(&request(), "digest".into(), revision, 0)
            .unwrap();
        let reply =
            ApprovalReply::from_legacy(&lease.challenge, crate::ToolApprovalDecision::AllowOnce);
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        agent.load_from_session(session).unwrap();
        let policy = agent.permissions.lock().unwrap();
        assert!(policy.session_allow.is_empty());
        assert_ne!(policy.revision(), Some(revision));
        assert_eq!(policy.mode, crate::PermissionMode::Ask);
        // Revocation must reach outstanding leases, including those held by clones.
        assert!(lease.resolve(&reply, "digest", Some(revision), 1).is_err());
        assert!(registry.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn f01_explanation_is_bounded_terminal_text() {
        let raw = format!("\u{1b}[31m\n\u{202e}{}", "e\u{301}".repeat(900));
        let shown = display_text(&raw);
        assert!(shown.len() <= 1024);
        assert!(!shown.chars().any(char::is_control));
        assert!(!shown.contains('\u{202e}'));
    }

    #[test]
    fn f01_compound_shell_has_no_ineffective_persistent_choice() {
        use crate::{PermissionMode, PermissionPolicy, PermissionVerdict, ToolApprovalDecision};
        let dir = tempfile::tempdir().unwrap();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy.project_trusted = true;
        for command in ["git status && cargo test --offline", "git status; git diff"] {
            let args = serde_json::json!({"command": command});
            let PermissionVerdict::Ask(request) =
                policy.decide("compound", "bash", &args, dir.path())
            else {
                panic!("expected Ask");
            };
            assert!(!request.allows(ToolApprovalDecision::AllowForSession));
            assert!(!request.allows(ToolApprovalDecision::AllowAlways));
            assert!(request.session_rule.is_empty());
        }
        let args = serde_json::json!({"command": "git status"});
        let PermissionVerdict::Ask(request) = policy.decide("single", "bash", &args, dir.path())
        else {
            panic!("expected Ask");
        };
        assert!(request.allows(ToolApprovalDecision::AllowForSession));
        policy.remember(&request.session_rule);
        assert!(matches!(
            policy.decide("repeat", "bash", &args, dir.path()),
            PermissionVerdict::Allow
        ));
    }

    #[test]
    fn f01_legal_scope() {
        let ids = |risk, session, project| {
            offer_scopes(risk, session, project)
                .into_iter()
                .map(|choice| choice.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(false, true, true),
            [
                "once",
                "session",
                "project",
                "deny",
                "deny_with_instructions"
            ]
        );
        assert_eq!(
            ids(true, true, true),
            ["once", "deny", "deny_with_instructions"]
        );
        assert_eq!(
            ids(false, false, false),
            ["once", "deny", "deny_with_instructions"]
        );
    }

    #[test]
    fn f01_stale_reply() {
        assert!(reply_matches("c1", "c1", "h1", "h1", 7, 7, 10, 11));
        assert!(!reply_matches("c1", "c1", "h1", "h2", 7, 7, 10, 11));
        assert!(!reply_matches("c1", "c1", "h1", "h1", 7, 8, 10, 11));
        assert!(!reply_matches("c1", "c1", "h1", "h1", 7, 7, 11, 11));
    }

    #[test]
    fn f01_grant_binding() {
        assert!(grant_applies(
            "s1",
            "s1",
            "web_fetch:https://a.example:443",
            "web_fetch:https://a.example:443",
            false
        ));
        assert!(!grant_applies("s1", "s2", "x", "x", false));
        assert!(!grant_applies("s1", "s1", "x", "x", true));
    }

    #[test]
    fn f01_policy_owns_persistent_scope_and_exact_file_rule() {
        use crate::{PermissionMode, PermissionPolicy, PermissionVerdict, ToolApprovalDecision};
        let dir = tempfile::tempdir().unwrap();
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        let args = serde_json::json!({"path":"ordinary.txt", "content":"fixture"});
        let PermissionVerdict::Ask(request) = policy.decide("c1", "write", &args, dir.path())
        else {
            panic!("expected Ask")
        };
        assert!(request.allows(ToolApprovalDecision::AllowForSession));
        assert!(!request.allows(ToolApprovalDecision::AllowAlways));
        assert_eq!(request.session_rule, "write(ordinary.txt)");
        policy.project_trusted = true;
        let PermissionVerdict::Ask(trusted_request) =
            policy.decide("trusted", "write", &args, dir.path())
        else {
            panic!("expected trusted Ask")
        };
        assert!(trusted_request.allows(ToolApprovalDecision::AllowAlways));
        policy.remember(&request.session_rule);
        assert!(matches!(
            policy.decide("c2", "write", &args, dir.path()),
            PermissionVerdict::Allow
        ));
        assert!(matches!(
            policy.decide(
                "c3",
                "write",
                &serde_json::json!({"path":"different.txt", "content":"fixture"}),
                dir.path()
            ),
            PermissionVerdict::Ask(_)
        ));
        for (tool, args) in [
            (
                "write",
                serde_json::json!({"path":".env", "content":"fixture"}),
            ),
            (
                "write",
                serde_json::json!({"path":"wild*.txt", "content":"fixture"}),
            ),
            (
                "bash",
                serde_json::json!({"command":"git status; sudo rm -rf /"}),
            ),
            (
                "web_fetch",
                serde_json::json!({"url":"https://a.example:443/page"}),
            ),
            (
                "web_fetch",
                serde_json::json!({"url":"https://a.example:444/page"}),
            ),
        ] {
            let PermissionVerdict::Ask(request) = policy.decide("risk", tool, &args, dir.path())
            else {
                panic!("expected risk Ask")
            };
            assert!(!request.allows(ToolApprovalDecision::AllowForSession));
            assert!(!request.allows(ToolApprovalDecision::AllowAlways));
        }
        policy.mode = PermissionMode::AlwaysApprove;
        policy.deny.push(crate::PermissionRule::bare("write"));
        assert!(matches!(
            policy.decide("denied", "write", &args, dir.path()),
            PermissionVerdict::Deny { .. }
        ));
        assert!(matches!(
            PermissionPolicy::new(PermissionMode::Ask).decide(
                "malformed",
                "write",
                &serde_json::json!({"content":"fixture"}),
                dir.path()
            ),
            PermissionVerdict::Deny { .. }
        ));
    }
}
