//! The companion studio after a generation: pairing, opening, and what is left
//! when the browser cannot be opened. Offline: a scripted host stands in for
//! the Node process and the opener is injected. No browser, no network and no
//! provider run here, so this is orchestration evidence only; the real runtime
//! is covered by the ignored native tests in `design_runtime.rs`.

mod design_support;
use design_support::*;

use davinci_coding_agent::design::{
    admission::AuthorizedDesignContext, controller::*, error::*, generation::*, types::*,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};

const URL: &str = "http://127.0.0.1:41234/#pair=SECRET-PAIRING-TOKEN";

/// The host process, as a script of what the browser asks.
struct ScriptedStudio {
    inbox: VecDeque<Value>,
    answers: Vec<(String, DesignResult<Value>)>,
}
impl ScriptedStudio {
    fn new(messages: Vec<Value>) -> Self {
        Self {
            inbox: messages.into(),
            answers: vec![],
        }
    }
    fn answer(&self, id: &str) -> &Value {
        match &self
            .answers
            .iter()
            .find(|(answered, _)| answered == id)
            .unwrap_or_else(|| panic!("{id} was never answered"))
            .1
        {
            Ok(value) => value,
            Err(error) => panic!("{id} failed: {error}"),
        }
    }
}
impl StudioChannel for ScriptedStudio {
    fn receive(&mut self, _: Duration) -> DesignResult<Option<Value>> {
        Ok(self.inbox.pop_front())
    }
    fn respond(&mut self, id: &str, result: DesignResult<Value>) -> DesignResult<()> {
        self.answers.push((id.to_string(), result));
        Ok(())
    }
    fn cancellation(&self, _: &str) -> DesignResult<Arc<AtomicBool>> {
        Ok(Arc::new(AtomicBool::new(false)))
    }
}

fn ready(url: &str) -> Value {
    json!({"id":"ready","result":{"url":url}})
}
fn ask(id: &str, operation: &str, payload: Value) -> Value {
    json!({"id":id,"operation":operation,"payload":payload})
}
fn close() -> Value {
    json!({"id":"c","operation":"close"})
}

/// A design with three committed variants, as a finished generation leaves it.
fn generated() -> (
    tempfile::TempDir,
    davinci_agent::Agent,
    DesignController,
    ArtifactId,
) {
    let (dir, mut agent, store, request) = setup_with_variants(3);
    let artifact = request.artifact_id;
    let mut model = model(vec![
        three_concepts(),
        source_with("design_create", json!([variant(0), variant(1), variant(2)])),
        review(),
    ]);
    let state = run_it(
        &store,
        &mut agent,
        dir.path(),
        &request,
        &mut model,
        &mut renderer(),
    );
    assert_eq!(state.progress.phase, RunPhase::Complete);
    (dir, agent, DesignController::new(store, true), artifact)
}

fn serve(
    dir: &std::path::Path,
    agent: &mut davinci_agent::Agent,
    controller: &DesignController,
    artifact: ArtifactId,
    studio: &mut ScriptedStudio,
    opener: &dyn Fn(&str) -> StudioOpen,
) -> DesignResult<Value> {
    let ctx = AuthorizedDesignContext::from_agent(agent, dir)?;
    controller.serve_channel(agent, dir, artifact, &ctx, studio, opener)
}

#[test]
fn new_design_opens_paired_browser_studio_and_variants() {
    let (dir, mut agent, controller, artifact) = generated();
    let opened = Mutex::new(vec![]);
    let mut studio = ScriptedStudio::new(vec![
        ready(URL),
        ask("s", "status", json!({"artifact_id": artifact})),
        ask(
            "r",
            "read",
            json!({"artifact_id": artifact, "revision": "1"}),
        ),
        close(),
    ]);
    let result = serve(
        dir.path(),
        &mut agent,
        &controller,
        artifact,
        &mut studio,
        &|url| {
            opened.lock().unwrap().push(url.to_string());
            StudioOpen {
                opened: true,
                reason: None,
                clipboard: None,
            }
        },
    )
    .unwrap();

    // The pairing address went to the opener once, and only the opener.
    assert_eq!(*opened.lock().unwrap(), [URL]);
    assert_eq!(result["studio"]["opened"], true);
    assert!(!result.to_string().contains("SECRET-PAIRING-TOKEN"));
    // The browser's requests reach the controller: all three concepts are there.
    assert_eq!(studio.answer("s")["manifest"]["requested_variants"], 3);
    assert_eq!(
        studio.answer("r")["revision"]["variants"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn a_browser_that_will_not_open_keeps_the_studio_and_names_the_way_back() {
    let (dir, mut agent, controller, artifact) = generated();
    let mut studio = ScriptedStudio::new(vec![
        ready(URL),
        ask("s", "status", json!({"artifact_id": artifact})),
        close(),
    ]);
    let result = serve(
        dir.path(),
        &mut agent,
        &controller,
        artifact,
        &mut studio,
        &|url| {
            StudioOpen::failure(
                url,
                &format!("could not run xdg-open for {url}"),
                Some("copied to the clipboard"),
            )
        },
    )
    .unwrap();
    assert_eq!(result["studio"]["opened"], false);
    assert!(result["studio"]["reason"]
        .as_str()
        .unwrap()
        .contains("xdg-open"));
    assert_eq!(result["studio"]["clipboard"], "copied to the clipboard");
    assert_eq!(
        result["studio"]["recovery"],
        format!("/design open {artifact}")
    );
    // The error carries the reason, never the capability.
    assert!(!result.to_string().contains("SECRET-PAIRING-TOKEN"));
    assert!(result["studio"]["reason"]
        .as_str()
        .unwrap()
        .contains("[studio address]"));
    // Still served: the person can paste the address while the lease lives.
    assert_eq!(studio.answer("s")["manifest"]["requested_variants"], 3);
}

#[test]
fn an_unreachable_studio_stops_at_once_and_loses_no_artifact() {
    let (dir, mut agent, controller, artifact) = generated();
    let mut studio = ScriptedStudio::new(vec![
        ready(URL),
        ask("never", "status", json!({"artifact_id": artifact})),
    ]);
    let result = serve(
        dir.path(),
        &mut agent,
        &controller,
        artifact,
        &mut studio,
        &|url| StudioOpen::failure(url, "no browser and no clipboard", None),
    )
    .unwrap();
    assert_eq!(result["closed"], true);
    assert_eq!(result["studio"]["opened"], false);
    assert!(studio.answers.is_empty(), "nothing was served");
    assert!(!result.to_string().contains("SECRET-PAIRING-TOKEN"));
    // The revisions are still there for `/design open` later.
    let status = controller
        .execute(
            &mut agent,
            dir.path(),
            DesignRequest::Status {
                artifact_id: artifact,
            },
        )
        .unwrap();
    assert_eq!(status["manifest"]["requested_variants"], 3);
}

#[test]
fn only_a_loopback_pairing_address_is_ever_opened() {
    for bad in [
        "https://example.com/#pair=x",
        "http://127.0.0.1:1/",
        "http://localhost:1/#pair=x",
        "file:///etc/passwd#pair=x",
    ] {
        let (dir, mut agent, controller, artifact) = generated();
        let opened = Mutex::new(0);
        let mut studio = ScriptedStudio::new(vec![ready(bad)]);
        let error = serve(
            dir.path(),
            &mut agent,
            &controller,
            artifact,
            &mut studio,
            &|_| {
                *opened.lock().unwrap() += 1;
                StudioOpen {
                    opened: true,
                    reason: None,
                    clipboard: None,
                }
            },
        )
        .unwrap_err();
        assert!(matches!(error, DesignError::Denied(_)), "{bad}: {error:?}");
        assert_eq!(*opened.lock().unwrap(), 0, "{bad} reached the opener");
    }
}

#[test]
fn a_finished_run_says_what_to_do_next() {
    // The generation result carries the follow-up; the interactive shell runs it.
    let (_dir, _agent, _controller, artifact) = generated();
    let complete = with_next_step(json!({"progress":{"phase":"complete"}}), artifact);
    assert_eq!(
        complete["next"]["action"],
        format!("/design open {artifact}")
    );
    let checkpoint = OperationId::new();
    let blocked = with_next_step(
        json!({"progress":{"phase":"blocked"},"failure":{"retryable":true,"checkpoint":checkpoint}}),
        artifact,
    );
    assert_eq!(
        blocked["next"]["action"],
        format!("/design resume {artifact} {checkpoint}")
    );
    let dead = with_next_step(
        json!({"progress":{"phase":"blocked"},"failure":{"retryable":false,"checkpoint":checkpoint}}),
        artifact,
    );
    assert!(dead["next"]["action"]
        .as_str()
        .unwrap()
        .starts_with("/design revise"));
}

// ---- selection and targeted revision

use davinci_ai::ContentBlock;
use davinci_coding_agent::design::selection::*;

fn status(
    controller: &DesignController,
    agent: &mut davinci_agent::Agent,
    dir: &std::path::Path,
    artifact: ArtifactId,
) -> Value {
    controller
        .execute(
            agent,
            dir,
            DesignRequest::Status {
                artifact_id: artifact,
            },
        )
        .unwrap()
}
fn select(
    controller: &DesignController,
    agent: &mut davinci_agent::Agent,
    dir: &std::path::Path,
    request: SelectVariant,
) -> DesignResult<Value> {
    controller.execute(agent, dir, DesignRequest::Select { request })
}
fn read_files(
    controller: &DesignController,
    agent: &mut davinci_agent::Agent,
    dir: &std::path::Path,
    artifact: ArtifactId,
    revision: &str,
) -> Value {
    controller
        .execute(
            agent,
            dir,
            DesignRequest::Read {
                artifact_id: artifact,
                revision: serde_json::from_value(json!(revision)).unwrap(),
            },
        )
        .unwrap()["files"]
        .clone()
}
/// Every file under the workspace that is not the design store itself.
fn repository(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut found = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            let name = path
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if ["sessions", "design", "ledger.json"]
                .iter()
                .any(|skip| name.starts_with(skip))
            {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push((name, std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}
/// A `design_patch` call that returns the current revision with edits applied.
fn patch(current: &Value, files: &Value, edit: impl Fn(&mut Value, &mut Value)) -> ContentBlock {
    let mut variants = current["revision"]["variants"].clone();
    let mut files = files.clone();
    edit(&mut variants, &mut files);
    ContentBlock::ToolCall {
        id: OperationId::new().to_string(),
        name: "design_patch".into(),
        arguments: json!({
            "files": files,
            "entry_points": current["revision"]["sources"]["entry_points"],
            "variants": variants,
            "bindings": [],
        }),
    }
}
fn page(title: &str, body: &str) -> Value {
    json!(format!(
        "<!doctype html><html lang=\"en\"><title>{title}</title><main>{body}</main></html>"
    ))
}

#[test]
fn select_variant_revise_reload_and_review_without_apply() {
    let (dir, mut agent, controller, artifact) = generated();
    std::fs::write(dir.path().join("app.txt"), "the repository").unwrap();
    let before = repository(dir.path());

    let shown = status(&controller, &mut agent, dir.path(), artifact);
    assert!(shown["selection"].is_null(), "nothing chosen yet");
    let ids: Vec<String> = shown["revision"]["variants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| variant["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 3);
    let revision = shown["manifest"]["revision"].clone();
    let choose = |id: &str, operation: OperationId, revision: &Value| SelectVariant {
        artifact_id: artifact,
        revision: serde_json::from_value(revision.clone()).unwrap(),
        variant_id: Some(id.parse().unwrap()),
        operation_id: operation,
    };

    // Stale or foreign choices are refused and leave no mark.
    assert!(matches!(
        select(
            &controller,
            &mut agent,
            dir.path(),
            choose(&ids[1], OperationId::new(), &json!("9"))
        ),
        Err(DesignError::Conflict(_))
    ));
    assert!(matches!(
        select(
            &controller,
            &mut agent,
            dir.path(),
            choose(&VariantId::new().to_string(), OperationId::new(), &revision)
        ),
        Err(DesignError::NotFound(_))
    ));
    assert!(status(&controller, &mut agent, dir.path(), artifact)["selection"].is_null());

    // Choose B. The same operation twice is one choice; another payload under
    // the same operation id is a conflict.
    let operation = OperationId::new();
    let receipt = select(
        &controller,
        &mut agent,
        dir.path(),
        choose(&ids[1], operation, &revision),
    )
    .unwrap();
    assert_eq!(receipt["variant_id"], ids[1]);
    assert_eq!(
        select(
            &controller,
            &mut agent,
            dir.path(),
            choose(&ids[1], operation, &revision)
        )
        .unwrap(),
        receipt
    );
    assert!(matches!(
        select(
            &controller,
            &mut agent,
            dir.path(),
            choose(&ids[2], operation, &revision)
        ),
        Err(DesignError::Conflict(_))
    ));

    // A restart keeps the choice: the session is read back from disk.
    let path = agent.session.as_ref().unwrap().path.clone();
    agent.session = Some(davinci_session::JsonlSession::open(&path).unwrap());
    let reloaded = status(&controller, &mut agent, dir.path(), artifact);
    assert_eq!(reloaded["selection"]["variant_id"], ids[1]);

    // "Make B more minimal": the model rewrites B and tries to touch A first.
    let files = read_files(&controller, &mut agent, dir.path(), artifact, "1");
    let minimal = |variants: &mut Value, files: &mut Value| {
        variants[1]["title"] = json!("Concept 1, more minimal");
        files["v1.html"] = page("B", "Minimal");
    };
    let touches_a = patch(&reloaded, &files, |variants, files| {
        minimal(variants, files);
        files["v0.html"] = page("A", "Changed A");
    });
    let good = patch(&reloaded, &files, minimal);
    let store = controller.store.clone();
    let request = GenerationRequest {
        artifact_id: artifact,
        expected_revision: serde_json::from_value(revision.clone()).unwrap(),
        brief: "make B more minimal".into(),
        operation_id: OperationId::new(),
        target_variant: Some(ids[1].parse().unwrap()),
    };
    let mut m = model(vec![touches_a, good, review()]);
    m.calls = 3;
    let state = run_it(
        &store,
        &mut agent,
        dir.path(),
        &request,
        &mut m,
        &mut renderer(),
    );
    assert_eq!(
        state.progress.phase,
        RunPhase::Complete,
        "{:?}",
        state.failure
    );
    assert_eq!(
        state.progress.repairs, 1,
        "the edit to A was sent back once"
    );
    assert_eq!(m.calls, 6, "no concept plan was asked for");

    let after = status(&controller, &mut agent, dir.path(), artifact);
    assert_eq!(after["manifest"]["revision"], "2");
    let new_files = read_files(&controller, &mut agent, dir.path(), artifact, "2");
    assert_ne!(new_files["v1.html"], files["v1.html"], "B changed");
    assert_eq!(new_files["v0.html"], files["v0.html"], "A is untouched");
    assert_eq!(new_files["v2.html"], files["v2.html"], "C is untouched");
    assert_eq!(
        after["revision"]["variants"][0],
        reloaded["revision"]["variants"][0]
    );
    assert_eq!(
        after["revision"]["variants"][2],
        reloaded["revision"]["variants"][2]
    );
    assert_eq!(
        after["selection"]["variant_id"], ids[1],
        "the choice outlives the revision"
    );

    // Choosing and revising changed nothing in the repository.
    assert_eq!(repository(dir.path()), before);

    // A model that will not leave C alone is stopped at the exact reason, and
    // the last revision stays the head.
    let stubborn = || {
        patch(&after, &new_files, |variants, files| {
            variants[1]["title"] = json!("B again");
            files["v2.html"] = page("C", "Changed C");
        })
    };
    let mut request = request;
    request.expected_revision = serde_json::from_value(json!("2")).unwrap();
    request.operation_id = OperationId::new();
    let mut m = model(vec![stubborn(), stubborn(), stubborn()]);
    m.calls = 10;
    let state = run_it(
        &store,
        &mut agent,
        dir.path(),
        &request,
        &mut m,
        &mut renderer(),
    );
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    let failure = state.failure.unwrap();
    assert_eq!(failure.phase, RunPhase::Generate);
    assert!(
        failure.message.contains("only the selected concept"),
        "{}",
        failure.message
    );
    assert_eq!(
        status(&controller, &mut agent, dir.path(), artifact)["manifest"]["revision"],
        "2"
    );
    assert_eq!(repository(dir.path()), before);
}

#[test]
fn the_browser_asks_the_host_what_a_brief_will_create() {
    let (dir, mut agent, controller, _) = generated();
    let mut resolve = |brief: &str| {
        controller
            .execute(
                &mut agent,
                dir.path(),
                DesignRequest::Resolve {
                    brief: brief.into(),
                },
            )
            .unwrap()
    };
    let dashboard = resolve("Create a modern dashboard for AI agents");
    assert_eq!(dashboard["kind"], "product");
    assert_eq!(dashboard["variants"], 3);
    assert_eq!(dashboard["source"], "inferred");
    let vague = resolve("something fresh");
    assert_eq!(vague["kind"], "landing");
    assert_eq!(vague["source"], "default");
}
