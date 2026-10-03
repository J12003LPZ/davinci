//! Risk-specific acceptance requirements. Selection is explicit; a file name or
//! one green command cannot establish semantic coverage of a requirement.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeRisk {
    Documentation,
    LocalBehavior,
    Migration,
    Authorization,
    ApiContract,
    UserInterface,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceCheck {
    DocumentExamples,
    Normal,
    Boundary,
    InvalidInput,
    Concurrent,
    Persistence,
    AuthorizationNegative,
    ContractCompatibility,
    UserJourney,
    Accessibility,
    MigrationRollback,
    Security,
    Integration,
    ReleaseProvenance,
    Rollback,
    DeploymentHealth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptancePack {
    pub risk: ChangeRisk,
    pub reason: String,
    pub required: Vec<AcceptanceCheck>,
}

pub fn acceptance_pack(risk: ChangeRisk) -> AcceptancePack {
    use AcceptanceCheck::*;
    let (reason, required) = match risk {
        ChangeRisk::Documentation => ("Documentation examples and links require direct verification.", vec![DocumentExamples]),
        ChangeRisk::LocalBehavior => ("A localized behavior change needs normal, boundary and invalid-input coverage.", vec![Normal, Boundary, InvalidInput]),
        ChangeRisk::Migration => ("Persistent data changes require restart, concurrent access and rollback evidence.", vec![Normal, Boundary, InvalidInput, Concurrent, Persistence, MigrationRollback, Security]),
        ChangeRisk::Authorization => ("Authorization changes require denied-path and concurrent-session evidence.", vec![Normal, Boundary, InvalidInput, Concurrent, AuthorizationNegative, Security]),
        ChangeRisk::ApiContract => ("Contract changes require compatibility and concurrent-client coverage.", vec![Normal, Boundary, InvalidInput, Concurrent, ContractCompatibility, Security]),
        ChangeRisk::UserInterface => ("UI changes require the actual user journey, loading/error/empty states and accessibility checks.", vec![Normal, Boundary, InvalidInput, UserJourney, Accessibility]),
        ChangeRisk::Unknown => ("Impact is uncertain; retain the broader contract and integration gate until it is scoped.", vec![Normal, Boundary, InvalidInput, Concurrent, Persistence, AuthorizationNegative, ContractCompatibility, Security, Integration]),
    };
    AcceptancePack {
        risk,
        reason: reason.into(),
        required,
    }
}
