//! Replayable, bounded prototype actions in the same confined capture owner.
use super::{admission::*, error::*, events::*, records::*, runtime::*, store::*, types::*};
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrototypeSelector {
    Role { role: String, name: String },
    Label { value: String },
    TestId { value: String },
}
impl PrototypeSelector {
    fn validate(&self) -> DesignResult<()> {
        match self {
            Self::Role { role, name } => {
                validate_text(role, 64, "role")?;
                validate_text(name, 120, "accessible name")
            }
            Self::Label { value } | Self::TestId { value } => validate_text(value, 120, "selector"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrototypeAction {
    Click {
        selector: PrototypeSelector,
    },
    Type {
        selector: PrototypeSelector,
        text: String,
    },
    Select {
        selector: PrototypeSelector,
        value: String,
    },
    Key {
        key: String,
    },
    ExpectText {
        text: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionRequest {
    pub render: RenderRequest,
    pub actions: Vec<PrototypeAction>,
    pub operation_id: OperationId,
}
impl InteractionRequest {
    pub fn validate(&self) -> DesignResult<()> {
        self.render.viewport.validate()?;
        if self.actions.is_empty() || self.actions.len() > 16 {
            return Err(DesignError::BudgetExceeded(
                "prototype sequence needs 1..16 actions".into(),
            ));
        }
        for action in &self.actions {
            match action {
                PrototypeAction::Click { selector } => selector.validate()?,
                PrototypeAction::Type { selector, text } => {
                    selector.validate()?;
                    if text.len() > 4096 || text.contains('\0') {
                        return Err(DesignError::InvalidInput("prototype input length".into()));
                    }
                }
                PrototypeAction::Select { selector, value } => {
                    selector.validate()?;
                    validate_text(value, 120, "selected value")?;
                }
                PrototypeAction::Key { key }
                    if [
                        "Tab",
                        "Shift+Tab",
                        "Enter",
                        "Escape",
                        "Space",
                        "ArrowUp",
                        "ArrowDown",
                        "ArrowLeft",
                        "ArrowRight",
                    ]
                    .contains(&key.as_str()) => {}
                PrototypeAction::Key { .. } => {
                    return Err(DesignError::InvalidInput(
                        "unsupported prototype key".into(),
                    ))
                }
                PrototypeAction::ExpectText { text } => {
                    validate_text(text, 512, "expected outcome")?
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionReceipt {
    pub request: InteractionRequest,
    pub capture: RenderReceipt,
}

pub fn load(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    id: OperationId,
) -> DesignResult<InteractionReceipt> {
    let reference = store
        .events(ctx, session)?
        .into_iter()
        .find_map(|event| match event.change {
            DesignChange::Interacted { receipt } if event.operation_id == id => Some(receipt),
            _ => None,
        })
        .ok_or_else(|| DesignError::NotFound("prototype interaction missing".into()))?;
    Ok(serde_json::from_slice(&store.read_blob(ctx, &reference)?)?)
}
pub fn interact(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    runtime: &TrustedDesignRuntime,
    request: InteractionRequest,
) -> DesignResult<InteractionReceipt> {
    request.validate()?;
    store.prepare(ctx, session, "design_render", &request)?;
    let hash = digest(&request)?;
    if store
        .retry(ctx, session, request.operation_id, &hash)?
        .is_some()
    {
        return load(store, ctx, session, request.operation_id);
    }
    let mut budget = DesignLimits::default().max_render_bytes;
    let capture = super::render::capture_inner(
        store,
        ctx,
        session,
        runtime,
        request.render.clone(),
        &request.actions,
        &mut budget,
    )?;
    let head = store.manifest(ctx, session, request.render.artifact_id)?;
    if head.revision != request.render.revision {
        return Err(DesignError::Conflict(
            "design changed during interaction".into(),
        ));
    }
    let receipt = InteractionReceipt { request, capture };
    let reference = store.retain(ctx, "application/json", &serde_json::to_vec(&receipt)?)?;
    ctx.check("design_render", &serde_json::to_value(&receipt.request)?)?;
    store.publish(
        ctx,
        session,
        receipt.request.operation_id,
        hash,
        DesignChange::Interacted { receipt: reference },
    )?;
    Ok(receipt)
}
