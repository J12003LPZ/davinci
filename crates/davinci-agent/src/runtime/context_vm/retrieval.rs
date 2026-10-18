use super::{ContextVmRuntime, EventImage};
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
    /// Images the source carried, handed to the model as image content of the
    /// tool result (never serialized into `content`).
    #[serde(skip)]
    pub images: Vec<EventImage>,
    /// What came back or could not: image references, how many were attached,
    /// or why they are gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_note: Option<String>,
}

/// Images one `retrieve_context` call returns at most.
pub const MAX_RETRIEVED_IMAGES: usize = 4;
/// Base64 bytes of image data one call returns at most.
pub const MAX_RETRIEVED_IMAGE_BYTES: usize = 4 * 1024 * 1024;

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
    let mut material = None;
    let (source, content) = if let Some(page) = request.page.as_deref() {
        retrieve_page(runtime, page)?
    } else if let Some(source_ref) = request.source_ref.as_deref() {
        let found = runtime.source_material(source_ref)?;
        let text = found.text.clone();
        material = Some(found);
        (source_ref.to_string(), text)
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
    // Images ride on the first unfiltered page only, so paging through a
    // long source does not repeat them.
    let (images, image_note) = match material {
        Some(material) => attach_images(
            &source,
            material.images,
            material.images_not_retained,
            request.offset == 0 && query.is_none(),
        ),
        None => (Vec::new(), None),
    };
    Ok(RetrieveContextResult {
        source,
        content,
        truncated: next_offset.is_some(),
        next_offset,
        images,
        image_note,
    })
}

/// The images to return within [`MAX_RETRIEVED_IMAGES`] and
/// [`MAX_RETRIEVED_IMAGE_BYTES`], and a note naming what was and was not
/// returned. Silence about missing images would read as "text was all there
/// was".
fn attach_images(
    source: &str,
    images: Vec<EventImage>,
    not_retained: bool,
    first_page: bool,
) -> (Vec<EventImage>, Option<String>) {
    if images.is_empty() {
        let note = not_retained.then(|| {
            "This source carried image(s) that are not recoverable: no session file is bound and the in-memory image bound was exceeded. Only the text is available."
                .to_string()
        });
        return (Vec::new(), note);
    }
    if !first_page {
        return (
            Vec::new(),
            Some(format!(
                "This source carries {} image(s); they are attached to the first unfiltered page (offset 0, no query).",
                images.len()
            )),
        );
    }
    let total = images.len();
    let mut bytes = 0usize;
    let mut kept = Vec::new();
    for image in images {
        if kept.len() >= MAX_RETRIEVED_IMAGES
            || bytes.saturating_add(image.data.len()) > MAX_RETRIEVED_IMAGE_BYTES
        {
            break;
        }
        bytes += image.data.len();
        kept.push(image);
    }
    let lines: Vec<String> = kept.iter().map(|image| image.reference(source)).collect();
    let mut note = format!(
        "{} of {total} image(s) attached: {}",
        kept.len(),
        lines.join(" ")
    );
    if kept.len() < total {
        note.push_str(&format!(
            ". {} omitted past the {MAX_RETRIEVED_IMAGES}-image / {MAX_RETRIEVED_IMAGE_BYTES}-byte bound",
            total - kept.len()
        ));
    }
    (kept, Some(note))
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
    // Images travel as `details.images`, which the turn loop turns into
    // image content of the tool result.
    let details = image_details(&result.images);
    Ok(ToolResult {
        content,
        is_error: false,
        details,
    })
}

/// `details.images` for a tool result: the shape `tool_result_message` turns
/// into image content.
fn image_details(images: &[EventImage]) -> Option<Value> {
    (!images.is_empty()).then(|| {
        serde_json::json!({
            "images": images.iter().map(|image| serde_json::json!({
                "data": image.data,
                "mimeType": image.mime_type,
            })).collect::<Vec<_>>()
        })
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

    use super::super::{events_from_messages, ContextVmConfig};
    use davinci_ai::{ChatMessage, MessageContent};

    fn runtime() -> ContextVmRuntime {
        ContextVmRuntime::new(ContextVmConfig::default(), Default::default())
    }

    fn image_message(text: &str, data: &str) -> ChatMessage {
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(MessageContent::Text { text: text.into() });
        }
        content.push(MessageContent::Image {
            data: data.into(),
            mime_type: "image/png".into(),
        });
        ChatMessage {
            role: "user".into(),
            content,
            ..ChatMessage::default()
        }
    }

    fn by_source(source_ref: &str) -> RetrieveContextRequest {
        RetrieveContextRequest {
            source_ref: Some(source_ref.into()),
            ..RetrieveContextRequest::default()
        }
    }

    #[test]
    fn sessionless_retrieve_returns_the_image_of_an_image_only_message() {
        let vm = runtime();
        let events = events_from_messages(&[image_message("", "QUJD")]);
        assert_eq!(events.len(), 1, "an image-only message is an event");
        vm.record_events(&events);
        let result = retrieve(&vm, &by_source(&events[0].source_ref)).unwrap();
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].data, "QUJD");
        assert_eq!(result.images[0].mime_type, "image/png");
        let note = result.image_note.clone().unwrap();
        assert!(note.contains("1 of 1 image(s) attached"), "{note}");
        assert!(note.contains(&events[0].images[0].digest()[..16]), "{note}");
        // Bytes never enter the text the model reads.
        assert!(!serde_json::to_string(&result).unwrap().contains("QUJD"));
    }

    #[test]
    fn later_pages_and_queries_do_not_repeat_images() {
        let vm = runtime();
        let events = events_from_messages(&[image_message("line one\nline two", "QUJD")]);
        vm.record_events(&events);
        let mut request = by_source(&events[0].source_ref);
        request.query = Some("two".into());
        let result = retrieve(&vm, &request).unwrap();
        assert!(result.images.is_empty());
        assert!(result.image_note.unwrap().contains("first unfiltered page"));
    }

    #[test]
    fn sessionless_images_past_the_memory_bound_are_reported_not_implied() {
        let vm = runtime();
        let big = "A".repeat(super::super::sources::MAX_SESSIONLESS_IMAGE_BYTES / 3 + 1);
        let events = events_from_messages(&[
            image_message("first", &big),
            image_message("second", &big),
            image_message("third", &big),
        ]);
        vm.record_events(&events);
        for kept in &events[..2] {
            let result = retrieve(&vm, &by_source(&kept.source_ref)).unwrap();
            assert_eq!(result.images.len(), 1);
        }
        let third = retrieve(&vm, &by_source(&events[2].source_ref)).unwrap();
        assert!(third.images.is_empty());
        assert_eq!(third.content, "third");
        let note = third.image_note.unwrap();
        assert!(note.contains("not recoverable"), "{note}");
        assert!(
            !note.contains("  "),
            "the note reads as one sentence: {note}"
        );
    }

    #[test]
    fn a_rebase_keeps_images_of_sources_the_state_still_cites() {
        let vm = runtime();
        let events = events_from_messages(&[image_message("", "QUJD")]);
        vm.append_delta(&events).unwrap();
        let cited = events[0].source_ref.clone();
        let compacted = events_from_messages(&[ChatMessage::text("user", "a short summary")]);
        vm.rebase(&compacted).unwrap();
        let result = retrieve(&vm, &by_source(&cited)).unwrap();
        assert_eq!(result.images.len(), 1, "{result:?}");
        // A later append sees the retained source outside the events; it
        // must not read that as a rewrite and wipe the retained images.
        vm.record_events(&compacted);
        let result = retrieve(&vm, &by_source(&cited)).unwrap();
        assert_eq!(result.images.len(), 1, "{result:?}");
    }

    #[test]
    fn retrieved_images_are_bounded_per_call() {
        let image = |n: usize| EventImage::new("image/png", format!("img{n}"));
        let many: Vec<EventImage> = (0..MAX_RETRIEVED_IMAGES + 2).map(image).collect();
        let (kept, note) = attach_images("transient:0:x", many, false, true);
        assert_eq!(kept.len(), MAX_RETRIEVED_IMAGES);
        assert!(note.unwrap().contains("2 omitted"));
    }

    #[test]
    fn bound_session_retrieve_returns_the_image_from_the_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let repo = davinci_session::JsonlSessionRepo::new(dir.path());
        let mut session = repo
            .create(davinci_session::JsonlCreateOptions {
                id: Some("img-session".into()),
                cwd: "/fixture".into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        let content = serde_json::to_value(image_message("see this", "QUJD").content).unwrap();
        let entry = session
            .append_entry(
                davinci_session::SessionEntry::message("user", content),
                "main",
            )
            .unwrap();
        let events = super::super::events_from_session_branch(&[entry.clone()], Some(&entry.id));
        assert_eq!(events[0].images.len(), 1);
        let vm = runtime();
        vm.bind_session_source(session.info.path.clone(), "img-session".into());
        vm.record_events(&events);
        // The stored copy keeps no image bytes; the JSONL is the source.
        assert!(vm.events()[0].images.is_empty());
        let result = retrieve(&vm, &by_source(&events[0].source_ref)).unwrap();
        assert_eq!(result.content, "see this");
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].data, "QUJD");
    }

    #[test]
    fn tool_details_carry_images_in_the_shape_the_turn_loop_reads() {
        let details = image_details(&[EventImage::new("image/png", "QUJD")]).unwrap();
        let parsed = crate::parse_rpc_images(details["images"].as_array().unwrap());
        assert_eq!(
            parsed,
            vec![MessageContent::Image {
                data: "QUJD".into(),
                mime_type: "image/png".into()
            }]
        );
        assert!(image_details(&[]).is_none());
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
