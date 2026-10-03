//! Host startup binding, shared by print, RPC, interactive and child CLIs.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};
use serde::{Deserialize, Serialize};

const INHERITED: &str = "DAVINCI_INHERITED_ROOT_BUDGET";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootBudgetConfig {
    pub root_id: String,
    pub ledger: PathBuf,
    pub limits: BudgetLimits,
    pub per_attempt_max_output_tokens: Option<u64>,
    #[serde(default)]
    pub parent_actor_id: Option<String>,
}

impl RootBudgetConfig {
    pub fn parse(bytes: &[u8], base: &Path) -> Result<Self, String> {
        if bytes.len() > 16_384 {
            return Err("root budget configuration is too large".into());
        }
        let mut config: Self = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid root budget configuration: {e}"))?;
        config.limits.validate()?;
        match (config.per_attempt_max_output_tokens, config.limits.max_output_tokens) {
            (None, None) => {},
            (Some(attempt), Some(root)) if attempt > 0 && attempt <= root => {},
            _ => return Err("per-attempt output limit must fit the positive root allowance or be absent for subscription-only admission".into()),
        }
        config.ledger =
            std::path::absolute(base.join(&config.ledger)).map_err(|e| e.to_string())?;
        Ok(config)
    }

    pub fn bind(&self, inherited: bool) -> Result<RootBudget, String> {
        if inherited {
            RootBudget::reopen(self.ledger.clone(), &self.root_id, self.limits.clone())
        } else {
            RootBudget::open(self.ledger.clone(), &self.root_id, self.limits.clone())
        }
    }
}

/// Called once, before agent construction/worker startup. The immutable JSON
/// descriptor is inherited by child CLIs; they never reread a mutable config
/// file or silently create another ledger. A child cannot replace its root via
/// a CLI flag. This is host authority, not a sandbox for arbitrary shell code.
pub fn initialize(
    path: Option<&Path>,
    launch_dir: &Path,
    resuming: bool,
) -> Result<Option<RootBudget>, String> {
    let inherited = std::env::var(INHERITED).map_err(|error| match error {
        std::env::VarError::NotPresent => String::new(),
        _ => "inherited root budget is not valid Unicode".into(),
    });
    let (mut config, is_child) = match inherited {
        Ok(json) => {
            if path.is_some() {
                return Err("a child cannot replace its inherited root budget".into());
            }
            (RootBudgetConfig::parse(json.as_bytes(), launch_dir)?, true)
        }
        Err(error) if error.is_empty() => {
            let Some(path) = path else { return Ok(None) };
            let path = launch_dir.join(path);
            let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if metadata.len() > 16_384 {
                return Err("root budget configuration is too large".into());
            }
            (
                RootBudgetConfig::parse(
                    &std::fs::read(&path).map_err(|e| e.to_string())?,
                    path.parent()
                        .ok_or("root budget configuration has no parent")?,
                )?,
                false,
            )
        }
        Err(error) => return Err(error),
    };
    let budget = config.bind(is_child || resuming)?;
    if resuming && !is_child {
        budget.recover_pending()?;
    }
    let actor = uuid::Uuid::new_v4().to_string();
    davinci_ai::provider_observation::install_process_budget(
        config.root_id.clone(),
        actor.clone(),
        config.parent_actor_id.clone(),
        config.per_attempt_max_output_tokens,
        Arc::new(budget.clone()),
    )?;
    config.parent_actor_id = Some(actor);
    std::env::set_var(
        INHERITED,
        serde_json::to_string(&config).map_err(|e| e.to_string())?,
    );
    Ok(Some(budget))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_config_has_request_and_time_limits_without_api_spend_or_output_claims() {
        let directory = tempfile::tempdir().unwrap();
        let value = serde_json::json!({"root_id":"subscription", "ledger":"root.json", "limits": {
            "max_requests":20, "max_output_tokens":null, "max_cost_microusd":null,
            "deadline_unix_ms":18446744073709551615u64,
            "codex_subscription":{"model":"gpt-6-luna","effort":"high"}},
            "per_attempt_max_output_tokens":null});
        let parse = |value: &serde_json::Value| {
            RootBudgetConfig::parse(&serde_json::to_vec(value).unwrap(), directory.path())
        };
        let config = parse(&value).unwrap();
        assert!(config.bind(false).is_ok());
        for field in ["max_output_tokens", "max_cost_microusd"] {
            let mut invalid = value.clone();
            invalid["limits"][field] = serde_json::json!(100);
            assert!(parse(&invalid).is_err(), "{field}");
        }
        let mut invalid = value.clone();
        invalid["limits"]["codex_subscription"] = serde_json::Value::Null;
        assert!(parse(&invalid).is_err());
        let mut invalid = value;
        invalid["per_attempt_max_output_tokens"] = serde_json::json!(10);
        assert!(parse(&invalid).is_err());
    }

    #[test]
    fn config_is_finite_and_children_reopen_the_same_allowance() {
        let directory = tempfile::tempdir().unwrap();
        let value = serde_json::json!({"root_id":"r", "ledger":"root.json", "limits": {
            "max_requests":2, "max_output_tokens":100, "max_cost_microusd":null,
            "deadline_unix_ms":18446744073709551615u64}, "per_attempt_max_output_tokens":50});
        let config =
            RootBudgetConfig::parse(&serde_json::to_vec(&value).unwrap(), directory.path())
                .unwrap();
        assert!(config.bind(true).is_err());
        let parent = config.bind(false).unwrap();
        parent.reserve("p", "a", 50, None).unwrap();
        let child = config.bind(true).unwrap();
        child.reserve("c", "b", 50, None).unwrap();
        assert!(parent.reserve("p", "c", 1, None).is_err());
        let mut invalid = value;
        invalid["per_attempt_max_output_tokens"] = serde_json::json!(101);
        assert!(
            RootBudgetConfig::parse(&serde_json::to_vec(&invalid).unwrap(), directory.path())
                .is_err()
        );
    }
}
