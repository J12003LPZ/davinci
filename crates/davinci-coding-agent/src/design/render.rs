//! Design capture adapts the existing supervised browser and artifact tracker.
use super::{admission::*, compile, error::*, records::*, runtime::*, store::*, types::*};
use crate::interaction_testing::{
    artifacts::ArtifactBudgetTracker,
    browser_process::{BrowserProcess, BrowserProcessConfig},
};
use davinci_session::JsonlSession;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

pub fn policy_hash() -> String {
    let mut hash = Sha256::new();
    hash.update(b"davinci-design-confinement-v1:os-network-denied:private-workspace:virtual-origin:all-websockets-denied:service-workers-blocked");
    hash.update(include_bytes!("../interaction_testing/browser_backend.js"));
    hash.update(include_bytes!("confinement.rs"));
    format!("{:x}", hash.finalize())
}

/// Reports only checks actually measured by the capture; it does not imply WCAG conformance.
pub fn geometry_checks(geometry: &Value) -> DesignResult<BTreeMap<String, CheckResult>> {
    let nodes = geometry["nodes"]
        .as_array()
        .filter(|nodes| nodes.len() <= 200)
        .ok_or_else(|| DesignError::CorruptArtifact("invalid geometry".into()))?;
    let mut accessibility = Vec::new();
    let mut rendering = Vec::new();
    if geometry["overflow"] == true {
        rendering.push("Horizontal viewport overflow".into());
    }
    if geometry["omitted"] != false || geometry["eventsOmitted"] != false {
        rendering.push("Capture coverage exceeded bounded inspection limits".into());
    }
    if geometry["title"]
        .as_str()
        .is_none_or(|s| s.trim().is_empty())
    {
        accessibility.push("Document title missing".into());
    }
    if geometry["lang"]
        .as_str()
        .is_none_or(|s| s.trim().is_empty())
    {
        accessibility.push("Document language missing".into());
    }
    for node in nodes {
        if node["visible"] != true {
            continue;
        }
        let width = node["width"]
            .as_i64()
            .ok_or_else(|| DesignError::CorruptArtifact("invalid node bounds".into()))?;
        let height = node["height"]
            .as_i64()
            .ok_or_else(|| DesignError::CorruptArtifact("invalid node bounds".into()))?;
        if node["interactive"] == true {
            if node["name"].as_str().is_none_or(|s| s.is_empty()) {
                accessibility.push("Interactive element lacks a name".into());
            }
            if node["focusable"] != true {
                accessibility.push("Interactive element is not keyboard focusable".into());
            }
            if width < 24 || height < 24 {
                accessibility.push("Interactive target is smaller than 24 CSS pixels".into());
            }
        } else if node["tag"] == "img" && node["name"].as_str().is_none_or(|s| s.is_empty()) {
            accessibility
                .push("Image lacks an accessible name; review whether it is decorative".into());
        }
    }
    let events = geometry["events"]
        .as_array()
        .filter(|items| items.len() <= 128)
        .ok_or_else(|| DesignError::CorruptArtifact("invalid capture diagnostics".into()))?;
    if !events.is_empty() {
        rendering
            .push("Browser reported console errors, blocked requests, downloads, or popups".into());
    }
    let check = |coverage: &[&str], failures: Vec<String>| CheckResult {
        state: if failures.is_empty() {
            CheckState::Current
        } else {
            CheckState::Failed
        },
        coverage: coverage.iter().map(|s| (*s).into()).collect(),
        failures,
    };
    Ok(BTreeMap::from([
        (
            "render".into(),
            check(
                &[
                    "viewport overflow",
                    "browser diagnostics",
                    "bounded node geometry",
                ],
                rendering,
            ),
        ),
        (
            "accessibility".into(),
            check(
                &[
                    "document title and language",
                    "interactive names",
                    "keyboard focusability",
                    "24px target bounds",
                ],
                accessibility,
            ),
        ),
        (
            "interaction".into(),
            CheckResult {
                state: CheckState::Pending,
                coverage: vec![],
                failures: vec![],
            },
        ),
    ]))
}

pub fn capture(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    runtime: &TrustedDesignRuntime,
    request: RenderRequest,
    operation_id: OperationId,
) -> DesignResult<RenderReceipt> {
    capture_with_budget(
        store,
        ctx,
        session,
        runtime,
        request,
        operation_id,
        &mut DesignLimits::default().max_render_bytes,
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_with_budget(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    runtime: &TrustedDesignRuntime,
    request: RenderRequest,
    operation_id: OperationId,
    budget: &mut u64,
) -> DesignResult<RenderReceipt> {
    let receipt = capture_inner(store, ctx, session, runtime, request, &[], budget)?;
    store.record_render(ctx, session, receipt.clone(), operation_id)?;
    Ok(receipt)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_inner(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    runtime: &TrustedDesignRuntime,
    request: RenderRequest,
    actions: &[super::interaction::PrototypeAction],
    budget: &mut u64,
) -> DesignResult<RenderReceipt> {
    ctx.check("design_render", &serde_json::to_value(&request)?)?;
    request.viewport.validate()?;
    if request.fixture != "default" {
        return Err(DesignError::InvalidInput("unknown render fixture".into()));
    }
    let revision = store.read_revision(ctx, session, request.artifact_id, request.revision)?;
    let board = revision
        .variants
        .iter()
        .flat_map(|v| &v.artboards)
        .find(|b| b.id == request.artboard_id)
        .ok_or_else(|| DesignError::NotFound("artboard missing".into()))?;
    let files = store.read_sources(ctx, session, request.artifact_id, request.revision)?;
    let compiled = compile::compile(ctx, runtime, &board.entry_point, &files)?;
    let supervisor = ctx.tools().foreground_supervisor.as_ref().ok_or_else(|| {
        DesignError::MissingCapability("browser process supervisor unavailable".into())
    })?;
    runtime.verify()?;
    let mut assets = BTreeMap::new();
    let mut asset_bytes = 0u64;
    for asset in &revision.assets {
        use base64::{engine::general_purpose::STANDARD, Engine};
        asset_bytes = asset_bytes
            .checked_add(asset.source.size)
            .filter(|total| *total <= DesignLimits::default().max_artifact_bytes)
            .ok_or_else(|| DesignError::BudgetExceeded("render asset limit".into()))?;
        if compiled
            .files
            .keys()
            .any(|name| name.eq_ignore_ascii_case(&asset.path))
        {
            return Err(DesignError::InvalidInput(
                "asset collides with compiled output".into(),
            ));
        }
        let bytes = store.read_blob(ctx, &asset.source)?;
        super::assets::validate_image(&bytes, &asset.source.media_type, asset.width, asset.height)?;
        assets.insert(
            &asset.path,
            json!({"mediaType":asset.source.media_type,"base64":STANDARD.encode(bytes)}),
        );
    }
    let package = runtime.root.join("node_modules/playwright-core");
    let browser = BrowserProcess::start_design(supervisor, BrowserProcessConfig {
        node:&runtime.node, package:&package, version:"1.62.1", workspace:ctx.workspace(),
        environment:BTreeMap::from([("PLAYWRIGHT_BROWSERS_PATH".into(),runtime.browser().cache.to_string_lossy().into_owned())]),
    }, &json!({"files":compiled.files,"assets":assets,"theme":request.theme,"reducedMotion":request.reduced_motion,"executable":runtime.browser().executable}))
        .map_err(DesignError::MissingCapability)?;
    let send = |command: Value| {
        ctx.check("design_render", &serde_json::to_value(&request)?)?;
        browser
            .request_with_abort(
                command,
                Duration::from_secs(30),
                ctx.tools().abort.as_deref(),
            )
            .map_err(DesignError::MissingCapability)
    };
    let opened = send(
        json!({"op":"open","options":{"origins":["https://design.invalid"],"viewport":request.viewport}}),
    )?;
    let resource = opened["resource"]
        .as_u64()
        .ok_or_else(|| DesignError::CorruptArtifact("browser lease missing".into()))?;
    let entry = if board.entry_point.ends_with(".html") {
        &board.entry_point
    } else {
        "index.html"
    };
    send(
        json!({"op":"execute","resource":resource,"command":{"action":"navigate","url":format!("https://design.invalid/{entry}")}}),
    )?;
    let mut interaction = CheckResult {
        state: CheckState::Pending,
        coverage: vec![],
        failures: vec![],
    };
    for (index, action) in actions.iter().enumerate() {
        match send(json!({"op":"execute","resource":resource,"command":action})) {
            Ok(_) => interaction.coverage.push(format!(
                "Prototype action {}: {}",
                index + 1,
                serde_json::to_value(action)?["action"]
                    .as_str()
                    .unwrap_or("unknown")
            )),
            Err(error) => {
                interaction.state = CheckState::Failed;
                interaction
                    .failures
                    .push(format!("Prototype action {} failed: {error}", index + 1));
                break;
            }
        }
    }
    if interaction.failures.is_empty()
        && actions.iter().any(|action| {
            matches!(
                action,
                super::interaction::PrototypeAction::ExpectText { .. }
            )
        })
    {
        interaction.state = CheckState::Current;
        interaction.coverage.push("Only the explicitly requested prototype sequence and text assertion; production behavior remains unverified".into());
    }
    let geometry =
        send(json!({"op":"execute","resource":resource,"command":{"action":"design_geometry"}}))?;
    let mut checks = geometry_checks(&geometry)?;
    if !actions.is_empty() {
        checks.insert("interaction".into(), interaction);
    }
    let screenshot =
        send(json!({"op":"execute","resource":resource,"command":{"action":"screenshot"}}))?;
    let mut tracker = ArtifactBudgetTracker::new(
        usize::try_from(*budget)
            .map_err(|_| DesignError::BudgetExceeded("capture address space".into()))?,
    );
    let retained = browser
        .retain_screenshot(&screenshot, &mut tracker)
        .map_err(DesignError::IoFailure)?;
    let label = retained["artifact"]
        .as_str()
        .ok_or_else(|| DesignError::CorruptArtifact("capture label missing".into()))?;
    let png = tracker
        .items
        .get(label)
        .ok_or_else(|| DesignError::CorruptArtifact("retained capture missing".into()))?;
    // These durable blobs and their session reference precede browser lease cleanup.
    let geometry_bytes = serde_json::to_vec(&geometry)?;
    let cost = (png.len() as u64)
        .checked_add(geometry_bytes.len() as u64)
        .ok_or_else(|| DesignError::BudgetExceeded("capture size overflow".into()))?;
    *budget = budget
        .checked_sub(cost)
        .ok_or_else(|| DesignError::BudgetExceeded("50 MiB render-run limit".into()))?;
    let screenshot = store.retain(ctx, "image/png", png)?;
    let geometry = store.retain(ctx, "application/json", &geometry_bytes)?;
    let receipt = RenderReceipt {
        request,
        source_hash: revision.source_hash,
        runtime_hash: runtime.fingerprint().into(),
        policy_hash: policy_hash(),
        screenshot,
        geometry,
        checks,
    };
    Ok(receipt)
}
