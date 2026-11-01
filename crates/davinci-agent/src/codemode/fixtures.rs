//! Deterministic fixture coverage for Droid Script behavior, labelled
//! honestly. `BEHAVIOR_PARITY` means an offline test proves the described
//! behavior; `UNTESTED` means no offline oracle exists on this host and no
//! parity is claimed.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptFixtureStatus {
    pub id: &'static str,
    pub scenario: &'static str,
    pub evidence: &'static str,
}

pub fn droid_script_fixture_status() -> Vec<ScriptFixtureStatus> {
    use ScriptFixtureStatus as S;
    vec![
        S {
            id: "01",
            scenario: "nested read",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "02",
            scenario: "guarded command",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "03",
            scenario: "parallel reads",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "04",
            scenario: "tool error",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "05",
            scenario: "syntax error",
            evidence: "UNTESTED",
        },
        S {
            id: "06",
            scenario: "hostile input",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "07",
            scenario: "head and tail output",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "08",
            scenario: "sandbox escape",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "09",
            scenario: "long job",
            evidence: "UNTESTED",
        },
        S {
            id: "10",
            scenario: "background notify",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "11",
            scenario: "wait and kill",
            evidence: "BEHAVIOR_PARITY",
        },
        S {
            id: "12",
            scenario: "restart reconciliation",
            evidence: "BEHAVIOR_PARITY",
        },
    ]
}
