//! The budget a design request is accounted to.
//!
//! A host that started DaVinci with `--root-budget` already bound one root
//! ledger, and every design request uses it. Without one (an ordinary
//! interactive session) each design operation gets a ledger of its own: a
//! subscription-only budget of a few requests and minutes, pinned to the
//! session's model and effort. Ordinary turns are never accounted to it.
//!
//! The ledger and its limits are kept per operation, so retrying or resuming
//! the same operation reopens the same ledger with the same identity, and a
//! request it already spent stays spent.

use super::error::{DesignError, DesignResult};
use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};
use davinci_agent::Agent;
use std::path::{Path, PathBuf};

/// Requests and minutes for one design operation.
#[derive(Debug, Clone, Copy)]
pub struct OperationAllowance {
    pub requests: u64,
    pub minutes: u64,
}

/// Generation: at most twelve accounted attempts, two repair passes inside
/// them, and a fifteen-minute deadline.
pub const GENERATION: OperationAllowance = OperationAllowance {
    requests: 12,
    minutes: 15,
};

/// An implementation draft is one accounted request.
pub const HANDOFF_DRAFT: OperationAllowance = OperationAllowance {
    requests: 1,
    minutes: 15,
};

fn ledger_directory() -> PathBuf {
    davinci_session::default_agent_dir()
        .join("design")
        .join("budgets")
}

/// The host's root budget, or this operation's own.
pub fn for_operation(
    agent: &Agent,
    operation: &str,
    allowance: OperationAllowance,
) -> DesignResult<RootBudget> {
    if let Some(budget) = agent.root_budget() {
        return Ok(budget.clone());
    }
    open_scoped(&ledger_directory(), agent, operation, allowance)
}

pub(crate) fn open_scoped(
    directory: &Path,
    agent: &Agent,
    operation: &str,
    allowance: OperationAllowance,
) -> DesignResult<RootBudget> {
    if operation.is_empty()
        || operation.len() > 128
        || !operation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(DesignError::InvalidInput("design operation id".into()));
    }
    let effort = agent.request_thinking_level().as_str();
    if !matches!(effort, "low" | "medium" | "high" | "xhigh") {
        return Err(DesignError::Denied(format!(
            "design generation needs a thinking level of low, medium, high or xhigh, not {effort}"
        )));
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| DesignError::IoFailure(error.to_string()))?;
    let ledger = directory.join(format!("{operation}.json"));
    let limits_path = directory.join(format!("{operation}.limits.json"));
    // The limits, the deadline included, are fixed when the operation first
    // runs; a retry reads them back instead of starting a new clock.
    let limits = match read_limits(&limits_path)? {
        Some(limits) => limits,
        None => {
            let proposed = BudgetLimits {
                max_requests: allowance.requests,
                max_output_tokens: None,
                max_cost_microusd: None,
                codex_subscription: Some(
                    davinci_ai::subscription_policy::CodexSubscriptionPolicy {
                        model: agent.model_id.clone(),
                        effort: effort.into(),
                    },
                ),
                deadline_unix_ms: davinci_session::now_ms()
                    .saturating_add(allowance.minutes.saturating_mul(60_000)),
            };
            publish_limits(&limits_path, &proposed)?
        }
    };
    if limits
        .codex_subscription
        .as_ref()
        .is_none_or(|policy| policy.model != agent.model_id || policy.effort != effort)
    {
        return Err(DesignError::Conflict(
            "this design operation started with another model or effort".into(),
        ));
    }
    RootBudget::open(ledger, &format!("design-{operation}"), limits)
        .map_err(DesignError::MissingCapability)
}

fn read_limits(path: &Path) -> DesignResult<Option<BudgetLimits>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| DesignError::CorruptArtifact("design budget limits".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DesignError::IoFailure(error.to_string())),
    }
}

/// Fix the limits once. Two starts of one operation race here: each writes
/// a private file and links it into place, which never replaces an existing
/// file, so exactly one wins and the other adopts the winner's limits. A
/// reader never sees a half-written file.
fn publish_limits(path: &Path, proposed: &BudgetLimits) -> DesignResult<BudgetLimits> {
    let bytes =
        serde_json::to_vec_pretty(proposed).map_err(|e| DesignError::IoFailure(e.to_string()))?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| DesignError::IoFailure(error.to_string()))?;
    std::io::Write::write_all(&mut staged, &bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| DesignError::IoFailure(error.to_string()))?;
    match std::fs::hard_link(staged.path(), path) {
        Ok(()) => Ok(proposed.clone()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_limits(path)?
            .ok_or_else(|| DesignError::IoFailure("design budget limits vanished".into())),
        Err(error) => Err(DesignError::IoFailure(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::ThinkingLevel;

    fn agent(level: ThinkingLevel) -> Agent {
        let mut agent = Agent::new("budget fixture");
        agent.model_id = "gpt-6-luna".into();
        agent.thinking_level = level;
        agent
    }

    #[test]
    fn an_operation_without_a_host_budget_gets_its_own_and_keeps_it() {
        let temp = tempfile::tempdir().unwrap();
        let agent = agent(ThinkingLevel::High);
        let first = open_scoped(temp.path(), &agent, "op-1", GENERATION).unwrap();
        assert_eq!(first.limits().max_requests, 12);
        let policy = first.limits().codex_subscription.clone().unwrap();
        assert_eq!(
            (policy.model.as_str(), policy.effort.as_str()),
            ("gpt-6-luna", "high")
        );
        // A retry reopens the same ledger with the same identity, deadline
        // included, so a resumed run matches its recorded root.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let again = open_scoped(temp.path(), &agent, "op-1", GENERATION).unwrap();
        assert_eq!(first.binding_identity(), again.binding_identity());
        // Another operation is another ledger.
        let other = open_scoped(temp.path(), &agent, "op-2", HANDOFF_DRAFT).unwrap();
        assert_ne!(first.binding_identity(), other.binding_identity());
        assert_eq!(other.limits().max_requests, 1);
    }

    #[test]
    fn a_changed_route_or_unsupported_effort_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        open_scoped(temp.path(), &agent(ThinkingLevel::High), "op", GENERATION).unwrap();
        let error = open_scoped(temp.path(), &agent(ThinkingLevel::Low), "op", GENERATION)
            .unwrap_err()
            .to_string();
        assert!(error.contains("another model or effort"), "{error}");
        let error = open_scoped(temp.path(), &agent(ThinkingLevel::Off), "op2", GENERATION)
            .unwrap_err()
            .to_string();
        assert!(error.contains("low, medium, high or xhigh"), "{error}");
        assert!(open_scoped(temp.path(), &agent(ThinkingLevel::High), "../x", GENERATION).is_err());
    }

    #[test]
    fn concurrent_starts_of_one_operation_agree_on_one_deadline() {
        let temp = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let directory = temp.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    std::thread::sleep(std::time::Duration::from_millis(index));
                    open_scoped(&directory, &agent(ThinkingLevel::High), "op", GENERATION)
                        .unwrap()
                        .binding_identity()
                })
            })
            .collect();
        let identities: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            identities.windows(2).all(|pair| pair[0] == pair[1]),
            "{identities:?}"
        );
        // Only the published limits remain; every staged file is gone.
        let names: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| !name.starts_with("op.json"))
            .collect();
        assert_eq!(names, ["op.limits.json"], "{names:?}");
    }

    #[test]
    fn a_second_publish_adopts_the_first_limits_instead_of_replacing_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("op.limits.json");
        let limits = |deadline_unix_ms| BudgetLimits {
            max_requests: 12,
            max_output_tokens: None,
            max_cost_microusd: None,
            codex_subscription: None,
            deadline_unix_ms,
        };
        assert_eq!(
            publish_limits(&path, &limits(1_000))
                .unwrap()
                .deadline_unix_ms,
            1_000
        );
        assert_eq!(
            publish_limits(&path, &limits(2_000))
                .unwrap()
                .deadline_unix_ms,
            1_000
        );
        assert_eq!(read_limits(&path).unwrap().unwrap().deadline_unix_ms, 1_000);
    }
}
