use serde::{Deserialize, Serialize};

/// Risk classes are ordered from the least to the most authoritative use of
/// decision intelligence. The provider can suggest additions, but it cannot
/// make any of these classes authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionRisk {
    Ranking,
    Planning,
    Verification,
    Change,
    Security,
    Transaction,
}

pub type DecisionClass = DecisionRisk;

impl DecisionRisk {
    pub const fn is_authoritative(self) -> bool {
        matches!(
            self,
            Self::Verification | Self::Change | Self::Security | Self::Transaction
        )
    }

    pub const fn allows_behavior_change(self) -> bool {
        !self.is_authoritative()
    }
}
