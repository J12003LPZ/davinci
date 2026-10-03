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
            PrototypeAction::Key { key: "Tab".into() },
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
    let mut captures = Vec::new();
    for viewport in Viewport::defaults() {
        let mut current = request.clone();
        current.render.viewport = viewport;
        current.operation_id = OperationId::new();
        let result = interact(&store, &ctx, session, &runtime, current).unwrap();
        assert_eq!(
            result.capture.checks["interaction"].state,
            CheckState::Current
        );
        assert_eq!(result.capture.checks["render"].state, CheckState::Current);
        assert_eq!(result.capture.source_hash, revision.source_hash);
        assert!(result.capture.screenshot.size > 100);
        captures.push(result);
    }
    // interact closes its browser before returning. Drop the session writer and
    // retrieve each adopted screenshot through its persisted, owner-bound receipt.
    let session_path = session.path.clone();
    drop(agent.session.take());
    agent.session = Some(JsonlSession::open(&session_path).unwrap());
    let reopened_ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let reopened = agent.session.as_ref().unwrap();
    if let Some(directory) = std::env::var_os("DAVINCI_DESIGN_EVIDENCE") {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut binary = std::fs::File::open(env!("CARGO_BIN_EXE_davinci")).unwrap();
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let count = binary.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        std::fs::write(
            directory.join("native-environment.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "platform":std::env::consts::OS, "arch":std::env::consts::ARCH,
                "binary_sha256":format!("{:x}", hash.finalize()),
                "runtime_hash":runtime.fingerprint(),
                "ci_source_commit":std::env::var("GITHUB_SHA").ok(),
                "ci_run_id":std::env::var("GITHUB_RUN_ID").ok(),
                "installed_launch_path_verified":false
            }))
            .unwrap(),
        )
        .unwrap();
    }
    for result in captures {
        let retained = load(&store, &reopened_ctx, reopened, result.request.operation_id).unwrap();
        assert_eq!(retained.capture, result.capture);
        let png = davinci_agent::runtime::evidence_store::VerificationEvidenceStore::new(
            store.blob_directory(&reopened_ctx),
        )
        .get_artifact(&(&retained.capture.screenshot).into())
        .unwrap();
        let viewport = &retained.request.render.viewport;
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            viewport.width
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
            viewport.height
        );
        if let Some(directory) = std::env::var_os("DAVINCI_DESIGN_EVIDENCE") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let name = format!("native-capture-{}x{}", viewport.width, viewport.height);
            std::fs::write(directory.join(format!("{name}.png")), png).unwrap();
            std::fs::write(
                directory.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&retained).unwrap(),
            )
            .unwrap();
        }
        println!(
            "Native runtime {}; {}x{} retained PNG {}",
            runtime.fingerprint(),
            viewport.width,
            viewport.height,
            retained.capture.screenshot.sha256
        );
    }
}

#[test]
#[ignore = "requires the pinned native Linux sandbox and browser; run native tests serially"]
fn native_hostile_page_and_process_cleanup() {
    use davinci_coding_agent::interaction_testing::browser_process::{
        BrowserProcess, BrowserProcessConfig,
    };
    use serde_json::json;
    assert!(
        cfg!(target_os = "linux"),
        "native process inspection requires Linux"
    );
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().canonicalize().unwrap();
    let runtime = TrustedDesignRuntime::configured(&workspace).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let html = format!(
        r#"<!doctype html><html lang="en"><title>Hostile fixture</title><body><main>Trying requests</main><script>
        const denied = [
          fetch('http://{address}/fetch').then(() => false, () => true),
          new Promise(resolve => {{
            try {{ const ws = new WebSocket('ws://{address}/socket');
              ws.onmessage = () => {{ ws.close(); resolve(false); }};
              ws.onclose = event => resolve([1006, 1008].includes(event.code));
              ws.onerror = () => resolve(true);
            }} catch {{ resolve(true); }}
          }}),
          new Promise(resolve => {{ const image = new Image();
            image.onload = () => resolve(false); image.onerror = () => resolve(true);
            image.src = 'http://{address}/image'; document.body.append(image);
          }})
        ];
        Promise.all(denied).then(results => {{ document.querySelector('main').textContent =
          results.every(Boolean) ? 'Blocked all three requests' : 'NETWORK ESCAPE'; }});
        </script></body></html>"#
    );
    let runtime_directory =
        std::path::PathBuf::from(std::env::var_os("DAVINCI_DESIGN_RUNTIME").unwrap());
    let node = std::path::PathBuf::from(std::env::var_os("DAVINCI_DESIGN_NODE").unwrap());
    // configured() above validates the whole installation before its paths are
    // used by this direct transport fixture. Do not widen the runtime's API.
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(runtime_directory.join("runtime-manifest.json")).unwrap(),
    )
    .unwrap();
    let package = runtime_directory.join("node_modules/playwright-core");
    let environment = BTreeMap::from([
        (
            "PLAYWRIGHT_BROWSERS_PATH".into(),
            manifest["browser"]["cache"].as_str().unwrap().into(),
        ),
        (
            "FONTCONFIG_FILE".into(),
            runtime_directory
                .join("fonts.conf")
                .to_string_lossy()
                .into_owned(),
        ),
    ]);
    let browser = BrowserProcess::start_design(
        &SupervisorCommand {
            executable: env!("CARGO_BIN_EXE_davinci").into(),
            argv: vec!["--internal-process-supervisor".into()],
        },
        BrowserProcessConfig {
            node: &node,
            package: &package,
            version: "1.62.1",
            workspace: &workspace,
            environment,
        },
        &json!({"files":{"index.html":html},"assets":{},"theme":"light",
            "reducedMotion":true,"executable":manifest["browser"]["executable"]}),
    )
    .unwrap();
    let send = |value: serde_json::Value| {
        browser
            .request(value.clone(), Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("native fixture request {value} failed: {error}"))
    };
    let opened = send(json!({"op":"open","options":{"origins":["https://design.invalid"]}}));
    let resource = opened["resource"].as_u64().unwrap();
    send(
        json!({"op":"execute","resource":resource,"command":{"action":"navigate","url":"https://design.invalid/index.html"}}),
    );
    send(
        json!({"op":"execute","resource":resource,"command":{"action":"expect_text","text":"Blocked all three requests"}}),
    );
    let geometry =
        send(json!({"op":"execute","resource":resource,"command":{"action":"design_geometry"}}));
    assert!(!geometry["events"].as_array().unwrap().is_empty());
    assert_eq!(
        davinci_coding_agent::design::render::geometry_checks(&geometry).unwrap()["render"].state,
        davinci_coding_agent::design::records::CheckState::Failed
    );
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );

    let processes = linux_processes();
    let mut descendants = std::collections::BTreeSet::from([std::process::id()]);
    loop {
        let before = descendants.len();
        for (&pid, (parent, _, _)) in &processes {
            if descendants.contains(parent) {
                descendants.insert(pid);
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    descendants.remove(&std::process::id());
    assert!(
        descendants.iter().any(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|name| name.contains("chrome"))
        }),
        "must observe real Chromium descendants before testing cleanup"
    );
    drop(browser);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let current = linux_processes();
        let alive: Vec<_> = descendants
            .iter()
            .filter(|pid| {
                current.get(pid).is_some_and(|(_, state, started)| {
                    *state != 'Z' && *started == processes[pid].2
                })
            })
            .collect();
        if alive.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "browser descendants survived close: {alive:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Some(directory) = std::env::var_os("DAVINCI_DESIGN_EVIDENCE") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            Path::new(&directory).join("native-security.json"),
            serde_json::to_vec_pretty(&json!({"runtime_hash":runtime.fingerprint(),"blocked_requests":3,"host_connections":0,
                "observed_descendants":descendants,"all_observed_descendants_stopped":true,
                "geometry":geometry}))
            .unwrap(),
        )
        .unwrap();
    }
}

fn linux_processes() -> BTreeMap<u32, (u32, char, u64)> {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let (_, fields) = stat.rsplit_once(") ")?;
            let fields: Vec<_> = fields.split_whitespace().collect();
            Some((
                pid,
                (
                    fields.get(1)?.parse().ok()?,
                    fields.first()?.chars().next()?,
                    fields.get(19)?.parse().ok()?,
                ),
            ))
        })
        .collect()
}
