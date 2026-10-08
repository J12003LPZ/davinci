use super::ContextVmRuntime;
use crate::tools::{ToolContext, ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrieveContextRequest {
    pub page: Option<String>,
    #[serde(rename = "sourceRef")]
    pub source_ref: Option<String>,
    pub query: Option<String>,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrieveContextResult {
    pub source: String,
    pub content: String,
    pub truncated: bool,
    pub next_offset: Option<usize>,
}

pub fn retrieve(
    runtime: &ContextVmRuntime,
    request: &RetrieveContextRequest,
) -> Result<RetrieveContextResult, String> {
    if request.page.is_some() == request.source_ref.is_some() {
        return Err("exactly one of page or sourceRef is required".into());
    }
    if request.limit > 400 {
        return Err("limit must be <= 400".into());
    }
    let result = retrieve_inner(runtime, request);
    runtime.note_page_fault(result.is_ok());
    result
}

fn retrieve_inner(
    runtime: &ContextVmRuntime,
    request: &RetrieveContextRequest,
) -> Result<RetrieveContextResult, String> {
    let (source, content) = if let Some(page) = request.page.as_deref() {
        retrieve_page(runtime, page)?
    } else if let Some(source_ref) = request.source_ref.as_deref() {
        let content = runtime.source_content(source_ref)?;
        (source_ref.to_string(), content)
    } else {
        return Err("exactly one of page or sourceRef is required".into());
    };

    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    if let Some(query) = &request.query {
        lines.retain(|line| line.contains(query));
    }
    let offset = request.offset.min(lines.len());
    let limit = if request.limit == 0 {
        120
    } else {
        request.limit
    };
    let end = offset.saturating_add(limit).min(lines.len());
    let truncated = end < lines.len();
    Ok(RetrieveContextResult {
        source,
        content: lines[offset..end].join("\n"),
        truncated,
        next_offset: truncated.then_some(end),
    })
}

pub fn retrieve_context_tool(
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let request: RetrieveContextRequest = serde_json::from_value(input.clone())
        .map_err(|error| ToolError::Failed(format!("invalid retrieve_context request: {error}")))?;
    let runtime = context
        .runtime
        .as_ref()
        .ok_or_else(|| ToolError::Failed("context VM runtime is unavailable".into()))?;
    let result = retrieve(&runtime.context_vm, &request).map_err(ToolError::Failed)?;
    let content = serde_json::to_string(&result)
        .map_err(|error| ToolError::Failed(format!("retrieve_context encode failed: {error}")))?;
    Ok(ToolResult {
        content,
        is_error: false,
        details: None,
    })
}

fn retrieve_page(runtime: &ContextVmRuntime, requested: &str) -> Result<(String, String), String> {
    let page_id = requested
        .strip_prefix("ctx://page/")
        .or_else(|| requested.strip_prefix("context_vm:"))
        .unwrap_or(requested);
    // Only pages the active root references are this conversation's. Page
    // IDs are content-addressed and the cache is shared, so a well-formed ID
    // from another session would otherwise load that session's state.
    let root = runtime.root();
    let Some(page) = root
        .checkpoint
        .iter()
        .chain(&root.deltas)
        .chain(&root.episodes)
        .find(|page| page.id == page_id)
        .cloned()
    else {
        return Err("context page unavailable; replay/rebuild required".into());
    };
    let object = runtime
        .store
        .load(&page)
        .map_err(|_| "context page unavailable; replay/rebuild required".to_string())?;
    let content = serde_json::to_string_pretty(&object)
        .map_err(|error| format!("context page render failed: {error}"))?;
    Ok((format!("ctx://page/{}", page.id), content))
}
