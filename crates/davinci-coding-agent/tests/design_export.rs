use davinci_agent::{Agent, PermissionMode};
use davinci_coding_agent::design::{
    admission::*, commands::ExportFormat, export::*, records::*, store::*, types::*,
};
use davinci_session::JsonlSession;
use std::collections::BTreeMap;

#[test]
fn source_export_is_exact_private_metadata_free_and_never_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("private-transcript-canary");
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let store = DesignStore::new(root.join("design"));
    let session = agent.session.as_mut().unwrap();
    let manifest = store
        .create(
            &ctx,
            session,
            CreateDesign {
                title: "Fixture".into(),
                brief: "private-brief-canary".into(),
                kind: DesignKind::Landing,
                operation_id: OperationId::new(),
                variants: 1,
            },
        )
        .unwrap();
    let files = BTreeMap::from([("index.html".into(), "<h1>Source fidelity</h1>".into())]);
    let sources = store
        .store_sources(&ctx, session, &files, vec!["index.html".into()])
        .unwrap();
    let revision = store
        .commit_revision(
            &ctx,
            session,
            RevisionWrite {
                artifact_id: manifest.id,
                expected_revision: RevisionId(0),
                operation_id: OperationId::new(),
                sources,
                variants: vec![Variant {
                    id: VariantId::new(),
                    title: "A".into(),
                    artboards: vec![Artboard {
                        id: ArtboardId::new(),
                        title: "Home".into(),
                        entry_point: "index.html".into(),
                    }],
                }],
                bindings: vec![],
                assets: vec![],
                profile_refs: vec![],
                system_snapshot: None,
            },
        )
        .unwrap();
    let request = ExportRequest {
        artifact_id: manifest.id,
        revision: revision.revision,
        format: ExportFormat::Source,
        destination: root.join("export").to_string_lossy().into(),
        artboard_id: None,
        viewport: None,
        operation_id: OperationId::new(),
    };
    let receipt = export_revision(&store, &ctx, session, &request, None).unwrap();
    assert_eq!(receipt.source_hash, revision.source_hash);
    assert_eq!(
        std::fs::read_to_string(root.join("export/source/index.html")).unwrap(),
        files["index.html"]
    );
    let exported = std::fs::read_to_string(root.join("export/design.json")).unwrap();
    assert!(!exported.contains("private-brief-canary"));
    assert!(!exported.contains("owner_session"));
    assert!(export_revision(&store, &ctx, session, &request, None).is_err());
    let mut bad = request.clone();
    // Windows verbatim PathBuf::join normalizes parent components before the
    // request reaches validation. Preserve the user's raw traversal spelling.
    bad.destination = format!("{}/../escape", root.display());
    assert!(export_revision(&store, &ctx, session, &bad, None).is_err());
    let mut png = request;
    png.destination = root.join("png").to_string_lossy().into();
    png.format = ExportFormat::Png;
    assert!(export_revision(&store, &ctx, session, &png, None).is_err());
    assert!(!root.join("png").exists());
    agent.set_permission_mode(PermissionMode::ReadOnly);
    assert!(export_revision(&store, &ctx, agent.session.as_ref().unwrap(), &png, None).is_err());
}
