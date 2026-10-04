use super::CodeModeError;
use crate::permission::{PermissionRule, ReadOutsideRootPolicy, RuleSpecifier};
use crate::runtime::operations::PayloadDigest;
use crate::Agent;
use serde_json::{json, Value};

fn rules(rules: &[PermissionRule]) -> Vec<Value> {
    rules
        .iter()
        .map(|rule| {
            let specifier = match &rule.specifier {
                None => Value::Null,
                Some(RuleSpecifier::Wildcard) => json!({"kind":"wildcard"}),
                Some(RuleSpecifier::Subject(value)) => json!({"kind":"subject","value":value}),
                Some(RuleSpecifier::Parameter { param, value }) => {
                    json!({"kind":"parameter","param":param,"value":value})
                }
            };
            json!({"tool":rule.tool,"pattern":rule.pattern,"specifier":specifier})
        })
        .collect()
}

/// A conservative cache-access binding, never a grant or a substitute for dispatch.
/// Store only its digest in the existing durable result, not rules or source paths.
pub(crate) fn fingerprint(agent: &Agent) -> Result<String, CodeModeError> {
    let policy = agent
        .permissions
        .lock()
        .map_err(|_| CodeModeError::new("DENIED", "permission policy unavailable"))?;
    if policy.revision().is_none() {
        return Err(CodeModeError::new(
            "DENIED",
            "permission revision unavailable",
        ));
    }
    let boundary = &policy.filesystem_boundary;
    let paths = serde_json::to_value((&boundary.root, &boundary.repo_root, &boundary.extra_roots))
        .map_err(|_| CodeModeError::new("DENIED", "boundary binding unavailable"))?;
    let outside = match boundary.read_outside_root {
        ReadOutsideRootPolicy::Allow => "allow",
        ReadOutsideRootPolicy::Ask => "ask",
        ReadOutsideRootPolicy::Deny => "deny",
    };
    let policy_value = json!({"projectTrusted":policy.project_trusted,"mode":policy.mode,
        "allow":rules(&policy.allow),"deny":rules(&policy.deny),"sessionAllow":rules(&policy.session_allow),
        "mcpReadOnly":policy.mcp_read_only,"boundary":{"root":paths[0],"repoRoot":paths[1],
            "readOutside":outside,"enforceMutations":boundary.enforce_root_for_mutations,
            "allowGitMetadata":boundary.allow_git_metadata,"extraRoots":paths[2]}});
    drop(policy);
    let runtime = agent
        .runtime
        .as_ref()
        .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "runtime unavailable"))?;
    let workspace = agent
        .cwd
        .canonicalize()
        .map_err(|_| CodeModeError::new("DENIED", "workspace unavailable"))?;
    let workspace = serde_json::to_value(workspace)
        .map_err(|_| CodeModeError::new("DENIED", "workspace binding unavailable"))?;
    PayloadDigest::of_json(
        &json!({"version":1,"policy":policy_value,"workspace":workspace,"contract":agent.active_contract().map(|contract| contract.digest),
        "capabilities":runtime.capability_registry.hash_tool_capabilities(&agent.tools)}),
    )
    .map(|digest| digest.to_string())
    .map_err(|_| CodeModeError::new("DENIED", "authority binding unavailable"))
}
