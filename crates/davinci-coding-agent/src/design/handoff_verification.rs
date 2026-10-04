//! Collect target checks through existing command, transaction and browser owners.
//! These receipts describe measured scope; they never certify backend integration.
use super::{admission::*, error::*, events::*, handoff::*, store::*, types::*};
use crate::native_extensions::browser::{BrowserAssertionSpec, BrowserConfig, BrowserController};
use davinci_agent::{runtime::transactions::coordinator_for_context, Agent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetBrowserCheck {
    pub process_id: u32,
    pub port: u32,
    pub path: String,
    pub actions: Vec<TargetBrowserAction>,
    pub dom_contains: String,
    pub accessibility_contains: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetBrowserAction {
    pub tool: String,
    pub args: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyHandoff {
    pub proposal_id: OperationId,
    pub proposal_hash: String,
    pub build_command: String,
    pub test_command: String,
    pub timeout_ms: u32,
    pub browser: Option<TargetBrowserCheck>,
    pub operation_id: OperationId,
}
impl VerifyHandoff {
    pub fn validate(&self) -> DesignResult<()> {
        validate_hash(&self.proposal_hash)?;
        for command in [&self.build_command, &self.test_command] {
            validate_text(command, 2048, "target command")?;
        }
        if self.build_command == self.test_command || !(1..=300_000).contains(&self.timeout_ms) {
            return Err(DesignError::InvalidInput(
                "distinct build/test commands and a bounded timeout are required".into(),
            ));
        }
        if let Some(browser) = &self.browser {
            if browser.process_id == 0
                || !(1..=65535).contains(&browser.port)
                || browser.actions.is_empty()
                || browser.actions.len() > 16
                || !browser.path.starts_with('/')
                || browser.path.starts_with("//")
            {
                return Err(DesignError::InvalidInput(
                    "invalid target browser flow".into(),
                ));
            }
            validate_text(&browser.path, 8192, "target path")?;
            validate_text(&browser.dom_contains, 4096, "DOM assertion")?;
            validate_text(
                &browser.accessibility_contains,
                4096,
                "accessibility assertion",
            )?;
            for action in &browser.actions {
                if !matches!(
                    action.tool.as_str(),
                    "browser_click" | "browser_type" | "browser_select"
                ) || !action.args.is_object()
                    || action.args.get("browser_id").is_some()
                    || serde_json::to_vec(&action.args)?.len() > 8192
                {
                    return Err(DesignError::InvalidInput(
                        "invalid target browser action".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn session(agent: &mut Agent) -> DesignResult<&mut davinci_session::JsonlSession> {
    agent
        .session
        .as_mut()
        .ok_or_else(|| DesignError::MissingCapability("persistent session required".into()))
}
fn target_fingerprint(ctx: &AuthorizedDesignContext) -> DesignResult<String> {
    let snapshot = super::sync::extract_system(ctx)?;
    let mut locks = BTreeMap::new();
    for name in [
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
    ] {
        let path = ctx.workspace().join(name);
        if !path.try_exists()? {
            continue;
        }
        super::runtime::no_links(&path)?;
        ctx.check_static_read(&path)?;
        if std::fs::metadata(&path)?.len() > 8 * 1024 * 1024 {
            return Err(DesignError::BudgetExceeded("target lockfile size".into()));
        }
        locks.insert(name, super::runtime::file_hash(&path)?);
    }
    digest(&(snapshot.fingerprint, locks))
}
fn publish(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    agent: &mut Agent,
    artifact_id: ArtifactId,
    request: &VerifyHandoff,
    operation_id: OperationId,
    report: &Value,
) -> DesignResult<()> {
    let bytes = serde_json::to_vec(report)?;
    if bytes.len() > 220 * 1024 {
        return Err(DesignError::BudgetExceeded("target receipt size".into()));
    }
    let reference = store.retain(ctx, "application/json", &bytes)?;
    store.publish(
        ctx,
        session(agent)?,
        operation_id,
        digest(request)?,
        DesignChange::HandoffChecked {
            artifact_id,
            run_id: request.operation_id,
            report: reference,
        },
    )?;
    Ok(())
}

/// No receipts, source claims or completion flags are accepted from the caller.
/// Browser configuration is supplied by the trusted host, never by a request DTO.
pub fn verify_handoff(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: VerifyHandoff,
    browser_config: BrowserConfig,
    cancel: Option<Arc<AtomicBool>>,
) -> DesignResult<Value> {
    request.validate()?;
    if agent.is_plan_mode() {
        return Err(DesignError::Denied(
            "leave Plan Mode before running target checks".into(),
        ));
    }
    let args = serde_json::to_value(&request)?;
    let ctx = AuthorizedDesignContext::for_operation(agent, workspace, "design_check", &args)?;
    let ctx = if let Some(flag) = cancel {
        ctx.with_cancellation(flag)
    } else {
        ctx
    };
    store.prepare(&ctx, session(agent)?, "design_check", &request)?;
    let prepared = load_proposal(store, &ctx, session(agent)?, request.proposal_id)?;
    if prepared.proposal_hash != request.proposal_hash {
        return Err(DesignError::Conflict(
            "target checks require the exact reviewed proposal".into(),
        ));
    }
    if [&request.build_command, &request.test_command]
        .iter()
        .any(|command| !prepared.proposal.request.verification.contains(command))
    {
        return Err(DesignError::Denied(
            "target commands must appear verbatim in the reviewed proposal".into(),
        ));
    }
    super::acceptance::accepted_revision(
        store,
        &ctx,
        session(agent)?,
        prepared.proposal.request.artifact_id,
        prepared.proposal.request.revision,
    )?;
    let transaction = store
        .events(&ctx, session(agent)?)?
        .into_iter()
        .rev()
        .find_map(|event| match event.change {
            DesignChange::HandoffApplied {
                proposal_hash,
                transaction_id,
            } if proposal_hash == request.proposal_hash => Some(transaction_id),
            _ => None,
        })
        .ok_or_else(|| {
            DesignError::Denied("apply this exact proposal before checking the target".into())
        })?;
    let coordinator = coordinator_for_context(workspace, &agent.tool_context)
        .map_err(|e| DesignError::Denied(e.to_string()))?;
    let authority = |path: &Path| ctx.check_static_read(path).map_err(|e| e.to_string());
    let observation = coordinator
        .observe_source(&transaction, &authority)
        .map_err(DesignError::StaleSource)?;
    let fingerprint = target_fingerprint(&ctx)?;
    if store
        .retry(
            &ctx,
            session(agent)?,
            request.operation_id,
            &digest(&request)?,
        )?
        .is_some()
    {
        let reference = store
            .events(&ctx, session(agent)?)?
            .into_iter()
            .rev()
            .find_map(|event| match event.change {
                DesignChange::HandoffChecked { run_id, report, .. }
                    if run_id == request.operation_id =>
                {
                    Some(report)
                }
                _ => None,
            })
            .ok_or_else(|| {
                DesignError::CorruptArtifact("target check checkpoint missing".into())
            })?;
        let report: Value = serde_json::from_slice(&store.read_blob(&ctx, &reference)?)?;
        if report["state"] != "complete" {
            return Err(DesignError::Conflict("target check outcome is unknown; inspect its command operations before explicitly starting a new check".into()));
        }
        if report["target_fingerprint"] != fingerprint
            || report["source_digest"] != observation.source_digest()
        {
            return Err(DesignError::StaleSource(
                "target changed since these checks".into(),
            ));
        }
        return Ok(report);
    }
    let mut report = json!({
        "request":request,"state":"running","transaction_id":transaction,
        "source_digest":observation.source_digest(),"target_fingerprint":fingerprint,
        "affected_paths":observation.affected_files(),"commands":[],"browser":null,
        "implementation":"pending_verification",
        "incomplete_coverage":["Command execution and browser assertions do not certify production API integration, mock removal, RSC/client boundaries or transitive source coverage; review the actual target and its tests."]
    });
    publish(
        store,
        &ctx,
        agent,
        prepared.proposal.request.artifact_id,
        &request,
        request.operation_id,
        &report,
    )?;
    let checks = (|| -> DesignResult<()> {
        for (kind, command) in [
            ("build", &request.build_command),
            ("test", &request.test_command),
        ] {
            ctx.check("design_check", &args)?;
            coordinator
                .check_source_observation(&observation, &authority)
                .map_err(DesignError::StaleSource)?;
            let receipt = command_receipt(agent, workspace, command, request.timeout_ms, &ctx)?;
            let passed = receipt.is_passed();
            let discovered = kind != "test"
                || (receipt.assertion_counts.is_some() && receipt.is_verified_check());
            report["commands"]
                .as_array_mut()
                .unwrap()
                .push(json!({"kind":kind,"receipt":receipt,"checks_passed":passed && discovered}));
            if !passed || !discovered {
                return Err(DesignError::Denied(format!(
                    "target {kind} failed or lacks actual test discovery"
                )));
            }
            if target_fingerprint(&ctx)? != fingerprint {
                return Err(DesignError::StaleSource(
                    "target inputs changed during checks".into(),
                ));
            }
        }
        if let Some(flow) = &request.browser {
            // A dedicated existing browser owner drops its process tree even if
            // cancellation/revocation prevents the normal close operation.
            let browser = BrowserController::new(workspace, browser_config);
            let receipt = browser_flow(&browser, store, workspace, &ctx, &transaction, flow)?;
            let passed = receipt["interaction"]["assertions_passed"] == true;
            report["browser"] = receipt;
            if !passed {
                return Err(DesignError::Denied(
                    "target browser assertions failed".into(),
                ));
            }
        } else {
            return Err(DesignError::MissingCapability(
                "actual target browser flow was not supplied".into(),
            ));
        }
        Ok(())
    })();
    ctx.check("design_check", &args)?;
    coordinator
        .check_source_observation(&observation, &authority)
        .map_err(DesignError::StaleSource)?;
    if target_fingerprint(&ctx)? != fingerprint {
        return Err(DesignError::StaleSource(
            "target inputs changed during checks".into(),
        ));
    }
    report["state"] = json!("complete");
    report["checks_passed"] = json!(checks.is_ok());
    report["failure"] = json!(checks.err().map(|error| error.to_string()));
    publish(
        store,
        &ctx,
        agent,
        prepared.proposal.request.artifact_id,
        &request,
        OperationId::new(),
        &report,
    )?;
    Ok(report)
}

fn browser_flow(
    browser: &BrowserController,
    store: &DesignStore,
    workspace: &Path,
    ctx: &AuthorizedDesignContext,
    transaction: &str,
    flow: &TargetBrowserCheck,
) -> DesignResult<Value> {
    let execute = |name: &str, args: Value| -> DesignResult<Value> {
        ctx.check("design_check", &json!({}))?;
        let result = browser
            .execute(workspace, name, &args, ctx.tools())
            .map_err(|e| DesignError::Denied(e.to_string()))?;
        if result.is_error {
            return Err(DesignError::Denied(result.content));
        }
        result.details.ok_or_else(|| {
            DesignError::MissingCapability("native browser evidence unavailable".into())
        })
    };
    let opened = execute(
        "browser_open",
        json!({"process_id":flow.process_id,"port":flow.port,"path":flow.path,"transaction_id":transaction}),
    )?;
    let id: uuid::Uuid = serde_json::from_value(opened["browser_id"].clone())?;
    let result = (|| {
        for action in &flow.actions {
            let mut args = action.args.clone();
            args["browser_id"] = json!(id);
            execute(&action.tool, args)?;
        }
        let receipt = browser
            .verify_host(
                workspace,
                id,
                BrowserAssertionSpec {
                    dom_contains: flow.dom_contains.clone(),
                    accessibility_contains: Some(flow.accessibility_contains.clone()),
                },
                ctx.tools(),
            )
            .map_err(DesignError::Denied)?;
        let artifact = receipt
            .screenshot_artifact
            .as_deref()
            .ok_or_else(|| DesignError::MissingCapability("target screenshot missing".into()))?;
        let screenshot = retain_screenshot(browser, store, workspace, ctx, id, artifact)?;
        let mut report = serde_json::to_value(receipt)?;
        report["screenshot"] = serde_json::to_value(screenshot)?;
        Ok(report)
    })();
    let closed = execute("browser_close", json!({"browser_id":id}));
    match (result, closed) {
        (Ok(receipt), Ok(_)) => Ok(receipt),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn command_receipt(
    agent: &mut Agent,
    workspace: &Path,
    command: &str,
    timeout: u32,
    ctx: &AuthorizedDesignContext,
) -> DesignResult<davinci_agent::runtime::evidence_store::ExecutionReceipt> {
    let abort = Arc::new(AtomicBool::new(false));
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stopped.load(Ordering::Acquire) {
                if ctx.check("design_check", &json!({})).is_err() {
                    abort.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        struct Stop<'a>(&'a AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _stop = Stop(&stopped);
        agent
            .execute_host_verification_command(
                workspace,
                command,
                u64::from(timeout),
                abort.clone(),
            )
            .map_err(DesignError::Denied)
    })
}

fn retain_screenshot(
    browser: &BrowserController,
    store: &DesignStore,
    workspace: &Path,
    ctx: &AuthorizedDesignContext,
    id: uuid::Uuid,
    artifact: &str,
) -> DesignResult<super::blob::ArtifactRef> {
    use base64::Engine;
    let mut bytes = Vec::new();
    let mut expected: Option<(String, usize)> = None;
    loop {
        ctx.check("design_check", &json!({}))?;
        let chunk = browser
            .retrieve_artifact(
                workspace,
                &json!({"browser_id":id,"artifact":artifact,
            "offset":bytes.len(),"limit":65536}),
                ctx.tools(),
            )
            .map_err(DesignError::Denied)?;
        let size = chunk["size"]
            .as_u64()
            .filter(|size| *size <= 16 * 1024 * 1024)
            .ok_or_else(|| DesignError::BudgetExceeded("target screenshot size".into()))?
            as usize;
        let hash = chunk["sha256"]
            .as_str()
            .ok_or_else(|| DesignError::CorruptArtifact("screenshot hash missing".into()))?;
        if let Some((old_hash, old_size)) = &expected {
            if hash != old_hash || size != *old_size {
                return Err(DesignError::StaleSource(
                    "screenshot changed during retrieval".into(),
                ));
            }
        } else {
            validate_hash(hash)?;
            expected = Some((hash.to_owned(), size));
        }
        let part = base64::engine::general_purpose::STANDARD
            .decode(chunk["base64"].as_str().unwrap_or(""))
            .map_err(|_| DesignError::CorruptArtifact("screenshot encoding".into()))?;
        if part.is_empty()
            || part.len() > 65536
            || bytes.len() + part.len() > size
            || chunk["offset"].as_u64() != Some(bytes.len() as u64)
        {
            return Err(DesignError::CorruptArtifact(
                "screenshot chunk bounds".into(),
            ));
        }
        bytes.extend_from_slice(&part);
        if bytes.len() == size {
            if chunk["eof"] != true || super::skills::byte_hash(&bytes) != hash {
                return Err(DesignError::CorruptArtifact(
                    "screenshot content hash".into(),
                ));
            }
            return store.retain(ctx, "image/png", &bytes);
        }
    }
}
