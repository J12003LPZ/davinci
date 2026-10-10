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

    let limit = if request.limit == 0 {
        120
    } else {
        request.limit
    };
    // A query matches case-insensitively: the model searching for "error"
    // must find "Error" (WOR-81). The returned lines are exact.
    let query = request.query.as_deref().map(str::to_lowercase);
    let (content, next_offset) = page_lines(
        content.lines().filter(|line| {
            query
                .as_deref()
                .is_none_or(|query| line.to_lowercase().contains(query))
        }),
        request.offset,
        limit,
    );
    Ok(RetrieveContextResult {
        source,
        content,
        truncated: next_offset.is_some(),
        next_offset,
    })
}

/// The most text one `retrieve_context` page returns. `limit` counts lines,
/// and one line can be megabytes, so lines alone do not bound a page.
pub const MAX_PAGE_BYTES: usize = 64 * 1024;

/// Takes one page from a lazy line iterator. It pulls at most `limit + 1`
/// lines past `offset` (the extra one only proves another page exists), so a
/// huge source is never copied to serve the first page.
///
/// A page also stops before [`MAX_PAGE_BYTES`]; `next_offset` then points at
/// the first line left out. A single line longer than a whole page is cut
/// at the bound and marked, and the next page starts after it.
fn page_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
    offset: usize,
    limit: usize,
) -> (String, Option<usize>) {
    let mut lines = lines.skip(offset);
    let mut page = String::new();
    let mut taken = 0usize;
    while taken < limit {
        let Some(line) = lines.next() else {
            return (page, None);
        };
        let separator = usize::from(taken > 0);
        let room = MAX_PAGE_BYTES.saturating_sub(page.len() + separator);
        if line.len() > room {
            if taken == 0 {
                let mut cut = room.min(line.len());
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                page.push_str(&line[..cut]);
                page.push_str(&format!(
                    " … [line cut at {MAX_PAGE_BYTES} bytes of {}]",
                    line.len()
                ));
                taken = 1;
            }
            return (page, Some(offset.saturating_add(taken)));
        }
        if separator == 1 {
            page.push('\n');
        }
        page.push_str(line);
        taken += 1;
    }
    let more = lines.next().is_some();
    (page, more.then(|| offset.saturating_add(limit)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wor68_first_page_pulls_only_one_line_past_the_limit() {
        let pulled = std::cell::Cell::new(0usize);
        let lines =
            std::iter::repeat_n("line", 1_000_000).inspect(|_| pulled.set(pulled.get() + 1));
        let (content, next) = page_lines(lines, 0, 1);
        assert_eq!(content, "line");
        assert_eq!(next, Some(1));
        assert!(pulled.get() <= 2, "pulled {} lines", pulled.get());
    }

    #[test]
    fn a_page_is_bounded_in_bytes_not_only_lines() {
        // One multi-megabyte line with limit=1.
        let huge = "é".repeat(2 * 1024 * 1024);
        let text = format!("{huge}\nsmall");
        let (content, next) = page_lines(text.lines(), 0, 1);
        assert!(
            content.len() <= MAX_PAGE_BYTES + 64,
            "{} bytes",
            content.len()
        );
        assert!(content.contains("line cut at"));
        assert_eq!(next, Some(1));
        assert_eq!(page_lines(text.lines(), 1, 1), ("small".to_string(), None));
        // Many medium lines: the page stops at the byte bound, and the next
        // page resumes at the first line left out.
        let line = "y".repeat(10_000);
        let many = vec![line.as_str(); 100].join("\n");
        let (content, next) = page_lines(many.lines(), 0, 400);
        assert!(content.len() <= MAX_PAGE_BYTES);
        let taken = content.lines().count();
        assert_eq!(next, Some(taken));
        assert!((1..100).contains(&taken));
    }

    #[test]
    fn wor68_paging_matches_the_previous_offset_and_next_offset_semantics() {
        let text = "a\nb\nc\nd\ne";
        let page = |offset, limit| page_lines(text.lines(), offset, limit);
        assert_eq!(page(0, 2), ("a\nb".to_string(), Some(2)));
        assert_eq!(page(2, 2), ("c\nd".to_string(), Some(4)));
        assert_eq!(page(4, 2), ("e".to_string(), None));
        assert_eq!(page(9, 2), (String::new(), None));
        assert_eq!(page(0, 5), ("a\nb\nc\nd\ne".to_string(), None));
        let filtered = text.lines().filter(|line| *line != "b");
        assert_eq!(page_lines(filtered, 1, 2), ("c\nd".to_string(), Some(3)));
    }
}
