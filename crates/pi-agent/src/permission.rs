//! Tool permission gates matching TypeScript `packages/agent/src/permission.ts`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

pub trait PermissionPolicy: Send + Sync {
    fn decide(&self, tool_name: &str) -> PermissionDecision;
}

#[derive(Debug, Default)]
pub struct AllowAllPermissionPolicy;

impl PermissionPolicy for AllowAllPermissionPolicy {
    fn decide(&self, _tool_name: &str) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

#[derive(Debug, Default)]
pub struct DenyAllPermissionPolicy;

impl PermissionPolicy for DenyAllPermissionPolicy {
    fn decide(&self, _tool_name: &str) -> PermissionDecision {
        PermissionDecision::Deny
    }
}

pub struct NamedPermissionPolicy {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl PermissionPolicy for NamedPermissionPolicy {
    fn decide(&self, tool_name: &str) -> PermissionDecision {
        if self.deny.iter().any(|n| n == tool_name) {
            return PermissionDecision::Deny;
        }
        if self.allow.is_empty() || self.allow.iter().any(|n| n == tool_name) {
            PermissionDecision::Allow
        } else {
            PermissionDecision::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_policy_denies() {
        let policy = NamedPermissionPolicy {
            allow: vec!["read".into()],
            deny: vec!["bash".into()],
        };
        assert_eq!(policy.decide("read"), PermissionDecision::Allow);
        assert_eq!(policy.decide("bash"), PermissionDecision::Deny);
        assert_eq!(policy.decide("write"), PermissionDecision::Deny);
    }
}
