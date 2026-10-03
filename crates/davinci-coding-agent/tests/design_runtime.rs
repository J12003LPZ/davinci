use davinci_agent::{jobs::supervisor::SupervisorCommand, Agent, PermissionMode};
use davinci_coding_agent::design::{admission::*, compile, host::*, runtime::*};
use davinci_session::JsonlSession;
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires explicit pinned runtime installation outside the test workspace"]
fn supervised_compiler_and_host_use_pinned_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.session = Some(
        JsonlSession::create_in_directory(
            &workspace.join("sessions"),
            &workspace.to_string_lossy(),
            None,
        )
        .unwrap(),
    );
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.tool_context.foreground_supervisor = Some(SupervisorCommand {
        executable: env!("CARGO_BIN_EXE_davinci").into(),
        argv: vec!["--internal-process-supervisor".into()],
    });
    let ctx = AuthorizedDesignContext::from_agent(&agent, &workspace).unwrap();
    let runtime = TrustedDesignRuntime::configured(&workspace).unwrap();
    let files = BTreeMap::from([(
        "app.tsx".into(),
        "export default function App(){return <button>Test</button>}".into(),
    )]);
    let result = compile::compile(&ctx, &runtime, "app.tsx", &files).unwrap();
    assert!(result.files["bundle.js"].contains("Test"));
    assert_eq!(
        result.source_hash,
        davinci_coding_agent::design::store::digest(&files).unwrap()
    );
    let hostile = BTreeMap::from([("app.tsx".into(), "import fs from 'node:fs'; export default function App(){return <p>{fs.readFileSync('/private')}</p>}".into())]);
    assert!(compile::compile(&ctx, &runtime, "app.tsx", &hostile).is_err());
    let mut host = DesignHostLease::start(&ctx, &runtime).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let message = loop {
        assert!(Instant::now() < deadline);
        if let Some(message) = host.receive(Duration::from_millis(50)).unwrap() {
            break message;
        }
    };
    assert_eq!(message["id"], "ready");
    let url = message["result"]["url"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"));
    assert!(url.contains('#'));
    host.close();
}

#[test]
fn runtime_rejects_repository_tooling_before_launch() {
    let temp = tempfile::tempdir().unwrap();
    let node = temp.path().join("node");
    std::fs::write(&node, "fixture").unwrap();
    assert!(TrustedDesignRuntime::load(&node, temp.path(), temp.path()).is_err());
    assert!(TrustedDesignRuntime::load(Path::new("missing"), temp.path(), temp.path()).is_err());
}

#[test]
#[ignore = "requires explicitly installed runtime and native network-denied browser sandbox"]
fn native_confined_capture_and_prototype_actions() {
    use davinci_coding_agent::design::{interaction::*, records::*, store::*, types::*};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("native fixture");
    agent.cwd = root.clone();
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    agent.tool_context.foreground_supervisor = Some(SupervisorCommand {
        executable: env!("CARGO_BIN_EXE_davinci").into(),
        argv: vec!["--internal-process-supervisor".into()],
    });
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let runtime = TrustedDesignRuntime::configured(&root).unwrap();
    let store = DesignStore::new(root.join("design"));
    let session = agent.session.as_mut().unwrap();
    let artifact = store
        .create(
            &ctx,
            session,
            CreateDesign {
                title: "Native capture".into(),
                brief: "Check a local button".into(),
                kind: DesignKind::Product,
                variants: 1,
                operation_id: OperationId::new(),
            },
        )
        .unwrap();
    let sources=store.store_sources(&ctx,session,&BTreeMap::from([("app.tsx".into(),"import {useState} from 'react'; export default function App(){const [done,setDone]=useState(false);return <main><button style={{minWidth:80,minHeight:44}} onClick={()=>setDone(true)}>Continue</button>{done&&<p>Saved locally</p>}</main>}".into())]),vec!["app.tsx".into()]).unwrap();
    let board = ArtboardId::new();
    let revision = store
        .commit_revision(
            &ctx,
            session,
            RevisionWrite {
                artifact_id: artifact.id,
                expected_revision: RevisionId(0),
                operation_id: OperationId::new(),
                sources,
                variants: vec![Variant {
                    id: VariantId::new(),
                    title: "One".into(),
                    artboards: vec![Artboard {
                        id: board,
                        title: "Home".into(),
                        entry_point: "app.tsx".into(),
                    }],
                }],
                bindings: vec![],
                assets: vec![],
                profile_refs: vec![],
                system_snapshot: None,
            },
        )
        .unwrap();
    let request = InteractionRequest {
        render: RenderRequest {
            artifact_id: artifact.id,
            revision: revision.revision,
            artboard_id: board,
            viewport: Viewport {
                width: 390,
                height: 844,
            },
            theme: Theme::Light,
            fixture: "default".into(),
            reduced_motion: true,
        },
        actions: vec![
            PrototypeAction::Click {
                selector: PrototypeSelector::Role {
                    role: "button".into(),
                    name: "Continue".into(),
                },
            },
            PrototypeAction::ExpectText {
                text: "Saved locally".into(),
            },
        ],
        operation_id: OperationId::new(),
    };
    let result = interact(&store, &ctx, session, &runtime, request).unwrap();
    assert_eq!(
        result.capture.checks["interaction"].state,
        CheckState::Current
    );
    assert!(result.capture.screenshot.size > 100);
    println!(
        "Native runtime {}; PNG {}",
        runtime.fingerprint(),
        result.capture.screenshot.sha256
    );
}
