//! Deterministic fixture coverage for Droid Script behavior, labelled
//! honestly. `BEHAVIOR_PARITY` means a test in this repository proves the
//! described behavior; `UNTESTED` means no oracle exists and no parity is
//! claimed.
//!
//! Broker-level fixtures live in `tests/common/codemode.rs`
//! (`reference_script_fixture_suite_passes`). The two that need the real guest
//! runtime live in `crates/davinci-coding-agent/tests/codemode_reference_fixtures.rs`
//! and are `#[ignore]`d until `DAVINCI_CODEMODE_FIXTURE_*` points at an admitted
//! Node 24.21.0 bundle.
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
            evidence: "BEHAVIOR_PARITY",
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
            evidence: "BEHAVIOR_PARITY",
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
