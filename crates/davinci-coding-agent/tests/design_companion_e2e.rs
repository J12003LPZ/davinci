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
