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

pub struct CallbackPermissionPolicy {
    callback: Box<dyn Fn(&str) -> PermissionDecision + Send + Sync>,
}

impl CallbackPermissionPolicy {
    pub fn new(callback: impl Fn(&str) -> PermissionDecision + Send + Sync + 'static) -> Self {
        Self {
            callback: Box::new(callback),
        }
    }
}

impl PermissionPolicy for CallbackPermissionPolicy {
    fn decide(&self, tool_name: &str) -> PermissionDecision {
        (self.callback)(tool_name)
    }
}

/// Interactive Ask: prompt stdin unless auto-approve is on.
pub struct StdinAskPermissionPolicy {
    pub auto_approve: bool,
}

impl PermissionPolicy for StdinAskPermissionPolicy {
    fn decide(&self, tool_name: &str) -> PermissionDecision {
        if self.auto_approve || matches!(tool_name, "read" | "ls" | "grep" | "find") {
            return PermissionDecision::Allow;
        }
        eprint!("Allow tool `{tool_name}`? [y/N] ");
        let _ = std::io::Write::flush(&mut std::io::stderr());
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("y") {
            PermissionDecision::Allow
        } else {
            PermissionDecision::Deny
        }
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
