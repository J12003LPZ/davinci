use davinci_agent::{Agent, PermissionMode};
use davinci_coding_agent::design::{
    admission::*, records::CreateDesign, store::DesignStore, types::*,
};
use davinci_session::JsonlSession;

#[test]
fn authority_revocation_blocks_cleanup_and_forged_owner_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let store = DesignStore::new(root.join("design"));
    let artifact = store
        .create(
            &ctx,
            agent.session.as_mut().unwrap(),
            CreateDesign {
                title: "Private".into(),
                brief: "A session-owned artifact".into(),
                kind: DesignKind::Product,
                variants: 1,
                operation_id: OperationId::new(),
            },
        )
        .unwrap();
    let mut foreign = Agent::new("foreign");
    foreign.set_permission_mode(PermissionMode::AlwaysApprove);
    foreign.session = Some(
        JsonlSession::create_in_directory(&root.join("other"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    let foreign_ctx = AuthorizedDesignContext::from_agent(&foreign, &root).unwrap();
    assert!(store
        .manifest(&foreign_ctx, agent.session.as_ref().unwrap(), artifact.id)
        .is_err());
    assert!(store
        .manifest(&ctx, foreign.session.as_ref().unwrap(), artifact.id)
        .is_err());
    agent.set_permission_mode(PermissionMode::ReadOnly);
    assert!(store
        .prune_unreferenced(&ctx, agent.session.as_mut().unwrap())
        .is_err());
    assert_eq!(
        store
            .list(&ctx, agent.session.as_ref().unwrap())
            .unwrap()
            .len(),
        1
    );
}
