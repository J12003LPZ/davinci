//! Fault hooks exist only in unit-test builds, never in the shipped runtime.
use super::{admission::*, error::*, records::*, store::*, types::*};
use davinci_agent::{Agent, PermissionMode};
use davinci_session::JsonlSession;
use std::{cell::Cell, collections::BTreeMap};

thread_local! {
    static INTERRUPT: Cell<Option<&'static str>> = const { Cell::new(None) };
}

pub(super) fn checkpoint(boundary: &'static str) -> DesignResult<()> {
    INTERRUPT.with(|pending| {
        if pending.get() == Some(boundary) {
            pending.set(None);
            Err(DesignError::IoFailure(format!(
                "test interruption: {boundary}"
            )))
        } else {
            Ok(())
        }
    })
}

#[test]
fn publication_interruptions_reopen_as_old_or_complete_revision_and_retry_once() {
    for boundary in ["before_blob", "after_blob", "before_event", "after_event"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut agent = Agent::new("publication fixture");
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.session = Some(
            JsonlSession::create_in_directory(
                &root.join("sessions"),
                &root.to_string_lossy(),
                None,
            )
            .unwrap(),
        );
        let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
        let store = DesignStore::new(root.join("design"));
        let session = agent.session.as_mut().unwrap();
        let artifact = store
            .create(
                &ctx,
                session,
                CreateDesign {
                    title: "Atomic revision".into(),
                    brief: "Keep committed source intact".into(),
                    kind: DesignKind::Product,
                    variants: 1,
                    operation_id: OperationId::new(),
                },
            )
            .unwrap();
        let files = BTreeMap::from([("index.html".into(), "<h1>Complete source</h1>".into())]);
        // Source publication has the same before/after durable-blob boundaries.
        if boundary.ends_with("blob") {
            INTERRUPT.with(|pending| pending.set(Some(boundary)));
            assert!(
                store
                    .store_sources(&ctx, session, &files, vec!["index.html".into()])
                    .is_err(),
                "source: {boundary}"
            );
            assert_eq!(
                store.manifest(&ctx, session, artifact.id).unwrap().revision,
                RevisionId(0)
            );
        }
        let sources = store
            .store_sources(&ctx, session, &files, vec!["index.html".into()])
            .unwrap();
        let write = RevisionWrite {
            artifact_id: artifact.id,
            expected_revision: RevisionId(0),
            operation_id: OperationId::new(),
            sources,
            variants: vec![Variant {
                id: VariantId::new(),
                title: "One".into(),
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
        };
        INTERRUPT.with(|pending| pending.set(Some(boundary)));
        assert!(
            store.commit_revision(&ctx, session, write.clone()).is_err(),
            "revision: {boundary}"
        );
        let path = session.path.clone();
        // Drop the session/writer as a terminating caller would, and reconstruct
        // solely from durable records. No in-memory operation result is reused.
        agent.session = None;
        let mut recovered = JsonlSession::open(&path).unwrap();
        let expected = if boundary == "after_event" {
            RevisionId(1)
        } else {
            RevisionId(0)
        };
        assert_eq!(
            store
                .manifest(&ctx, &recovered, artifact.id)
                .unwrap()
                .revision,
            expected,
            "{boundary}"
        );
        let revision = store
            .commit_revision(&ctx, &mut recovered, write.clone())
            .unwrap();
        assert_eq!(revision.revision, RevisionId(1));
        let entries = recovered.entries.len();
        assert_eq!(
            store.commit_revision(&ctx, &mut recovered, write).unwrap(),
            revision
        );
        assert_eq!(
            recovered.entries.len(),
            entries,
            "retry must not append twice"
        );
        assert_eq!(
            store
                .read_sources(&ctx, &recovered, artifact.id, RevisionId(1))
                .unwrap(),
            files
        );
        assert_eq!(store.prune_unreferenced(&ctx, &mut recovered).unwrap(), 0);
    }
}
