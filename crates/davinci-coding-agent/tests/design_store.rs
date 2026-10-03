use davinci_agent::{Agent, PermissionMode};
use davinci_coding_agent::design::{
    admission::*, error::DesignError, records::*, store::*, types::*,
};
use davinci_session::JsonlSession;
use std::collections::BTreeMap;

fn setup() -> (
    tempfile::TempDir,
    Agent,
    AuthorizedDesignContext,
    DesignStore,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    let context = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let store = DesignStore::new(root.join("design"));
    (temp, agent, context, store)
}
fn input() -> CreateDesign {
    CreateDesign {
        title: "Fixture".into(),
        brief: "A useful product page".into(),
        kind: DesignKind::Landing,
        operation_id: OperationId::new(),
        variants: 2,
    }
}
fn write(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    id: ArtifactId,
) -> RevisionWrite {
    let sources = store
        .store_sources(
            ctx,
            session,
            &BTreeMap::from([("index.html".into(), "<h1>Hello</h1>".into())]),
            vec!["index.html".into()],
        )
        .unwrap();
    RevisionWrite {
        artifact_id: id,
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
    }
}
#[test]
fn committed_revision_reopens_and_retries_exactly() {
    let (_tmp, mut agent, ctx, store) = setup();
    let session = agent.session.as_mut().unwrap();
    let input = input();
    let created = store.create(&ctx, session, input.clone()).unwrap();
    assert_eq!(store.create(&ctx, session, input.clone()).unwrap(), created);
    let mut altered = input;
    altered.brief.push('!');
    assert!(matches!(
        store.create(&ctx, session, altered),
        Err(DesignError::Conflict(_))
    ));
    let req = write(&store, &ctx, session, created.id);
    let revision = store.commit_revision(&ctx, session, req.clone()).unwrap();
    assert_eq!(store.commit_revision(&ctx, session, req).unwrap(), revision);
    let path = session.path.clone();
    agent.session = None;
    let reopened = JsonlSession::open(&path).unwrap();
    assert_eq!(
        store
            .read_revision(&ctx, &reopened, created.id, RevisionId(1))
            .unwrap(),
        revision
    );
    assert_eq!(
        store.list(&ctx, &reopened).unwrap()[0].revision,
        RevisionId(1)
    );
}
#[test]
fn stale_writer_and_wrong_owner_cannot_publish_or_read() {
    let (_tmp, mut agent, ctx, store) = setup();
    let s = agent.session.as_mut().unwrap();
    let artifact = store.create(&ctx, s, input()).unwrap();
    let req = write(&store, &ctx, s, artifact.id);
    let mut stale = JsonlSession::open(&s.path).unwrap();
    store.commit_revision(&ctx, s, req.clone()).unwrap();
    let mut loser = req;
    loser.operation_id = OperationId::new();
    assert!(store.commit_revision(&ctx, &mut stale, loser).is_err());
    let (_other, other_agent, other_ctx, _) = setup();
    assert!(store
        .read_revision(&other_ctx, s, artifact.id, RevisionId(1))
        .is_err());
    assert!(store
        .read_revision(
            &ctx,
            other_agent.session.as_ref().unwrap(),
            artifact.id,
            RevisionId(1)
        )
        .is_err());
}
#[test]
fn uncommitted_blobs_are_invisible_and_corruption_is_detected() {
    let (_tmp, mut agent, ctx, store) = setup();
    let s = agent.session.as_mut().unwrap();
    let artifact = store.create(&ctx, s, input()).unwrap();
    let req = write(&store, &ctx, s, artifact.id);
    assert_eq!(store.list(&ctx, s).unwrap()[0].revision, RevisionId(0));
    assert!(store
        .read_revision(&ctx, s, artifact.id, RevisionId(1))
        .is_err());
    let revision = store.commit_revision(&ctx, s, req).unwrap();
    let path = store
        .blob_directory(&ctx)
        .join(&revision.sources.files["index.html"].relative_store_path);
    std::fs::write(path, "corrupt").unwrap();
    assert!(matches!(
        store.read_revision(&ctx, s, artifact.id, RevisionId(1)),
        Err(DesignError::CorruptArtifact(_))
    ));
}
#[test]
fn revoked_policy_prevents_mutation() {
    let (_tmp, mut agent, ctx, store) = setup();
    agent.set_permission_mode(PermissionMode::ReadOnly);
    assert!(matches!(
        store.create(&ctx, agent.session.as_mut().unwrap(), input()),
        Err(DesignError::Denied(_))
    ));
}

#[test]
fn retained_raster_is_decoded_and_part_of_revision_identity() {
    use davinci_coding_agent::design::assets::ImageInput;
    let (_tmp, mut agent, ctx, store) = setup();
    let s = agent.session.as_mut().unwrap();
    let artifact = store.create(&ctx, s, input()).unwrap();
    let mut req = write(&store, &ctx, s, artifact.id);
    let original = store.commit_revision(&ctx, s, req.clone()).unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 3))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let input = ImageInput {
        path: "assets/photo.png".into(),
        media_type: "image/png".into(),
        width: 2,
        height: 3,
        provenance: "Locally authored test image".into(),
        rights: "CC0-1.0".into(),
    };
    assert!(store.store_image(&ctx, s, &input, b"invalid").is_err());
    let asset = store.store_image(&ctx, s, &input, bytes.get_ref()).unwrap();
    req.assets = vec![asset];
    req.expected_revision = RevisionId(1);
    req.operation_id = OperationId::new();
    let updated = store.commit_revision(&ctx, s, req).unwrap();
    assert_ne!(original.source_hash, updated.source_hash);
    assert_eq!(
        store
            .read_revision(&ctx, s, artifact.id, RevisionId(2))
            .unwrap(),
        updated
    );
}
