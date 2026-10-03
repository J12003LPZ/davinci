use std::collections::BTreeSet;

pub const ADDITIVE_THRESHOLD: f32 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionRollout {
    Shadow,
    GuardedAdditive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionalCapability {
    pub id: String,
    pub relevance: f32,
    pub available: bool,
    pub authorized: bool,
    pub vetoed: bool,
}

impl OptionalCapability {
    pub fn new(id: impl Into<String>, relevance: f32) -> Self {
        Self {
            id: id.into(),
            relevance,
            available: true,
            authorized: true,
            vetoed: false,
        }
    }
}

/// Merge provider suggestions without allowing them to remove deterministic
/// requirements or bypass availability, authorization, or vetoes.
pub fn add_optional_capabilities(
    deterministic: &[String],
    optional: &[OptionalCapability],
    rollout: DecisionRollout,
) -> Vec<String> {
    let mut result = deterministic.to_vec();
    if matches!(rollout, DecisionRollout::Shadow) {
        return result;
    }

    let mut existing: BTreeSet<String> = result.iter().cloned().collect();
    for candidate in optional {
        if candidate.relevance.is_finite()
            && candidate.relevance >= ADDITIVE_THRESHOLD
            && candidate.available
            && candidate.authorized
            && !candidate.vetoed
            && !existing.contains(&candidate.id)
        {
            existing.insert(candidate.id.clone());
            result.push(candidate.id.clone());
        }
    }
    result
}

pub fn merge_required_verification(
    deterministic: &[String],
    suggested: &[String],
    rollout: DecisionRollout,
) -> Vec<String> {
    add_optional_capabilities(
        deterministic,
        &suggested
            .iter()
            .cloned()
            .map(|id| OptionalCapability::new(id, 1.0))
            .collect::<Vec<_>>(),
        rollout,
    )
}
