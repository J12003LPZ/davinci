//! Explicit transaction tools use the same coordinator and dispatch authority as ordinary edits.
use super::tools::ToolTransaction;
use crate::tools::{AgentTool, ToolContext, ToolError, ToolResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

pub fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "patch_preview" | "patch_apply" | "patch_status" | "patch_rollback"
    )
}

pub fn tool_specs() -> Vec<AgentTool> {
    let mut tools = vec![AgentTool {
        name: "patch_preview".into(),
        description: "Preview a Codex patch without changing source files. Records bounded durable preimages and returns a transaction id and affected files. Apply only while those sources are unchanged.".into(),
        parameters: json!({"type":"object","additionalProperties":false,"properties":{"input":{"type":"string"}},"required":["input"]}),
    }];
    for (name, description) in [
        ("patch_apply", "Apply an owned preview after fresh permission and source checks. Pass the complete affected_files array from preview as paths. A transaction cannot be applied twice."),
        ("patch_status", "Inspect an owned durable transaction. Pass its complete affected_files array as paths; every path requires current read authorization. Set observe_commit to true to inspect local Git objects and record an exact matching HEAD commit; this never creates a commit or fetches objects."),
        ("patch_rollback", "Recover an owned transaction only when current bytes and identity are still transaction-owned. Pass its complete affected_files array as paths. Conflicts preserve later user changes."),
    ] {
        tools.push(AgentTool { name:name.into(), description:description.into(), parameters:json!({"type":"object","additionalProperties":false,"properties":{"id":{"type":"string"},"paths":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"string"}}},"required":["id","paths"]}) });
        if name == "patch_status" {
            tools.last_mut().expect("tool just inserted").parameters["properties"]["observe_commit"] = json!({"type":"boolean","default":false});
        }
    }
    tools
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {
    input: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Existing {
    id: String,
    paths: Vec<String>,
    #[serde(default)]
    observe_commit: bool,
}

pub(crate) fn execute(
    cwd: &Path,
    name: &str,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.transactions_disabled {
        return Err(ToolError::Failed(
            "Explicit editing transactions are disabled; ordinary edit safety remains enabled"
                .into(),
        ));
    }
    let manager = ToolTransaction::new(cwd, context)?;
    let summary = if name == "patch_preview" {
        let args: Preview =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::Failed(e.to_string()))?;
        if args.input.len() > super::model::MAX_TRANSACTION_BYTES {
            return Err(ToolError::Failed(
                "patch exceeds transaction byte limit".into(),
            ));
        }
        let (changes, _) = crate::apply_patch::prepare_patch(cwd, &args.input, |path| {
            manager.snapshot(path).map_err(|e| e.to_string())
        })
        .map_err(ToolError::Failed)?;
        manager.preview(changes)?
    } else {
        let args: Existing =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::Failed(e.to_string()))?;
        if args.paths.is_empty() || args.paths.len() > super::model::MAX_FILES {
            return Err(ToolError::Failed("transaction requires 1..64 paths".into()));
        }
        if input.get("observe_commit").is_some() && name != "patch_status" {
            return Err(ToolError::Failed(
                "commit observation is only supported by patch_status".into(),
            ));
        }
        let status = manager.status(&args.id, &args.paths)?;
        match name {
            "patch_status" if args.observe_commit => manager.observe_commit(&args.id)?,
            "patch_status" => status,
            "patch_apply" => manager.mutate(&args.id, false)?,
            "patch_rollback" => manager.mutate(&args.id, true)?,
            _ => return Err(ToolError::Unknown(name.into())),
        }
    };
    let content = serde_json::to_string(&summary).map_err(|e| ToolError::Failed(e.to_string()))?;
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(json!({"transaction":summary})),
    })
}
