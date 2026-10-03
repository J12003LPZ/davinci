use davinci_agent::verification::acceptance::{
    acceptance_pack, AcceptanceCheck as Check, ChangeRisk,
};
use davinci_coding_agent::completion_delivery::*;

#[test]
fn harness_acceptance_pack_matches_change_risk() {
    assert_eq!(
        acceptance_pack(ChangeRisk::Documentation).required,
        vec![Check::DocumentExamples]
    );
    for (risk, check) in [
        (ChangeRisk::Migration, Check::MigrationRollback),
        (ChangeRisk::Authorization, Check::AuthorizationNegative),
        (ChangeRisk::ApiContract, Check::ContractCompatibility),
        (ChangeRisk::UserInterface, Check::UserJourney),
        (ChangeRisk::Unknown, Check::Integration),
    ] {
        let pack = acceptance_pack(risk);
        assert!(pack.required.contains(&check));
        assert!(!pack.reason.is_empty());
    }
}

fn identity() -> DeliveryIdentity {
    DeliveryIdentity {
        source_digest: "a".repeat(64),
        configuration_digest: "b".repeat(64),
        artifact_sha256: Some("c".repeat(64)),
    }
}

fn fixture() -> DeliveryInput {
    let identity = identity();
    DeliveryInput {
        identity: identity.clone(),
        requirements: vec![DeliveryRequirement {
            id: "docs".into(),
            risk: ChangeRisk::Documentation,
        }],
        receipts: vec![AcceptanceReceipt {
            receipt_id: "receipt-1".into(),
            requirement_id: "docs".into(),
            check: Check::DocumentExamples,
            identity,
            passed: true,
        }],
        operational_references: vec!["runbook".into()],
        rollback_references: vec!["rollback".into()],
        deployment_authorized: false,
        installed_sha256: None,
        installed_resolution_verified: false,
    }
}

#[test]
fn harness_delivery_rejects_and_omits_invalid_identity_values() {
    let mut input = fixture();
    input.identity.source_digest = "private-value-not-a-hash".into();
    input.receipts[0].identity = input.identity.clone();
    let record = delivery_record(&input);
    assert_eq!(record.level, AcceptanceLevel::Implemented);
    assert!(!serde_json::to_string(&record)
        .unwrap()
        .contains("private-value-not-a-hash"));
}

#[test]
fn harness_delivery_level_requires_matching_evidence() {
    let input = fixture();
    assert_eq!(
        delivery_record(&input).level,
        AcceptanceLevel::LocallyVerified
    );
    let mut stale = input.clone();
    stale.identity.source_digest = "d".repeat(64);
    assert_eq!(delivery_record(&stale).level, AcceptanceLevel::Implemented);
    let mut uncovered = input.clone();
    uncovered.requirements.push(DeliveryRequirement {
        id: "auth".into(),
        risk: ChangeRisk::Authorization,
    });
    assert_eq!(
        delivery_record(&uncovered).level,
        AcceptanceLevel::Implemented
    );
    let mut changed_config = input.clone();
    changed_config.identity.configuration_digest = "e".repeat(64);
    assert_eq!(
        delivery_record(&changed_config).level,
        AcceptanceLevel::Implemented
    );
    let mut conflict = input;
    let mut duplicate = conflict.receipts[0].clone();
    duplicate.passed = false;
    conflict.receipts.push(duplicate);
    assert_eq!(
        delivery_record(&conflict).level,
        AcceptanceLevel::Implemented
    );
}

#[test]
fn harness_deploy_and_publish_need_separate_authority() {
    let mut input = fixture();
    for check in [
        Check::Integration,
        Check::ReleaseProvenance,
        Check::Rollback,
        Check::DeploymentHealth,
    ] {
        input.receipts.push(AcceptanceReceipt {
            receipt_id: format!("{check:?}"),
            requirement_id: "docs".into(),
            check,
            identity: input.identity.clone(),
            passed: true,
        });
    }
    input.installed_sha256 = input.identity.artifact_sha256.clone();
    input.installed_resolution_verified = true;
    assert_eq!(
        delivery_record(&input).level,
        AcceptanceLevel::ReleaseCandidate
    );
    input.deployment_authorized = true;
    assert_eq!(
        delivery_record(&input).level,
        AcceptanceLevel::DeployedOperationallyVerified
    );
    input.installed_sha256 = Some("f".repeat(64));
    let mismatched = delivery_record(&input);
    assert_eq!(mismatched.level, AcceptanceLevel::ReleaseCandidate);
    assert!(mismatched.gaps.iter().any(|gap| gap.contains("installed")));
    assert_eq!(mismatched.schema_version, 1);
}
