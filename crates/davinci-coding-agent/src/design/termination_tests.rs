//! OS termination checks. No environment fault switch exists in production.
use super::{admission::*, error::*, records::*, store::*, types::*};
use davinci_agent::{Agent, PermissionMode};
use davinci_session::JsonlSession;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

thread_local! { static WAIT_AT: RefCell<Option<(String, PathBuf)>> = const { RefCell::new(None) }; }
pub(super) fn checkpoint(boundary: &str) -> DesignResult<()> {
    WAIT_AT.with(|cell| {
        let state = cell.borrow();
        if let Some((expected, root)) = state.as_ref().filter(|(expected, _)| expected == boundary)
        {
            std::fs::write(root.join("ready"), expected)?;
            // The parent terminates this process. No stack unwinding, session
            // Drop or graceful writer release can participate in recovery.
            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        Ok(())
    })
}
fn files() -> BTreeMap<String, String> {
    BTreeMap::from([("index.html".into(), "<h1>Durable design</h1>".into())])
}
fn write(id: ArtifactId, op: OperationId, sources: SourceBundle) -> RevisionWrite {
    RevisionWrite {
        artifact_id: id,
        expected_revision: RevisionId(0),
        operation_id: op,
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
    }
}
#[test]
fn publication_child() {
    let Some(root) = std::env::var_os("DAVINCI_TEST_PUBLICATION_ROOT") else {
        return;
    };
    let root = PathBuf::from(root).canonicalize().unwrap();
    let boundary = std::env::var("DAVINCI_TEST_PUBLICATION_BOUNDARY").unwrap();
    let phase = std::env::var("DAVINCI_TEST_PUBLICATION_PHASE").unwrap();
    let mut agent = Agent::new("termination fixture");
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
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
                title: "Atomic source".into(),
                brief: "Retain exact committed source".into(),
                kind: DesignKind::Product,
                variants: 1,
                operation_id: OperationId::new(),
            },
        )
        .unwrap();
    let op = OperationId::new();
    std::fs::write(
        root.join("recovery.json"),
        serde_json::to_vec(&serde_json::json!({
            "session":session.path,"artifact_id":artifact.id,"operation_id":op
        }))
        .unwrap(),
    )
    .unwrap();
    if phase == "source" {
        WAIT_AT.with(|state| *state.borrow_mut() = Some((boundary.clone(), root.clone())));
    }
    let sources = store
        .store_sources(&ctx, session, &files(), vec!["index.html".into()])
        .unwrap();
    let write = write(artifact.id, op, sources);
    std::fs::write(root.join("write.json"), serde_json::to_vec(&write).unwrap()).unwrap();
    WAIT_AT.with(|state| *state.borrow_mut() = Some((boundary, root)));
    store.commit_revision(&ctx, session, write).unwrap();
    panic!("publication checkpoint was not reached");
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn physical_termination_at_publication_boundaries_preserves_source_and_idempotency() {
    for (phase, boundary) in [
        ("source", "before_blob"),
        ("source", "after_blob"),
        ("revision", "before_blob"),
        ("revision", "after_blob"),
        ("revision", "before_event"),
        ("revision", "after_event"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "design::termination_tests::publication_child",
                    "--nocapture",
                ])
                .env("DAVINCI_TEST_PUBLICATION_ROOT", &root)
                .env("DAVINCI_TEST_PUBLICATION_PHASE", phase)
                .env("DAVINCI_TEST_PUBLICATION_BOUNDARY", boundary)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !root.join("ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "{phase}/{boundary}: child exited before checkpoint"
            );
            assert!(
                Instant::now() < deadline,
                "{phase}/{boundary}: checkpoint timeout"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        let recovery: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("recovery.json")).unwrap()).unwrap();
        let mut agent = Agent::new("recovered termination fixture");
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.session = Some(
            JsonlSession::open(std::path::Path::new(recovery["session"].as_str().unwrap()))
                .unwrap(),
        );
        let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
        let store = DesignStore::new(root.join("design"));
        let session = agent.session.as_mut().unwrap();
        let id: ArtifactId = serde_json::from_value(recovery["artifact_id"].clone()).unwrap();
        let expected = if boundary == "after_event" {
            RevisionId(1)
        } else {
            RevisionId(0)
        };
        assert_eq!(
            store.manifest(&ctx, session, id).unwrap().revision,
            expected,
            "{phase}/{boundary}"
        );
        let request: RevisionWrite = if root.join("write.json").exists() {
            serde_json::from_slice(&std::fs::read(root.join("write.json")).unwrap()).unwrap()
        } else {
            let sources = store
                .store_sources(&ctx, session, &files(), vec!["index.html".into()])
                .unwrap();
            write(
                id,
                serde_json::from_value(recovery["operation_id"].clone()).unwrap(),
                sources,
            )
        };
        let revision = store
            .commit_revision(&ctx, session, request.clone())
            .unwrap();
        let count = session.entries.len();
        assert_eq!(
            store.commit_revision(&ctx, session, request).unwrap(),
            revision
        );
        assert_eq!(session.entries.len(), count);
        assert_eq!(
            store
                .read_sources(&ctx, session, id, RevisionId(1))
                .unwrap(),
            files()
        );
    }
}
