use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use davinci_ai::{complete_from_events, load_builtin_models, replay_sse_events, ContentBlock};
use serde_json::{json, Value};

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        record_allocation(layout.size(), !pointer.is_null());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record_allocation(layout.size(), !pointer.is_null());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let resized = unsafe { System.realloc(pointer, layout, new_size) };
        record_allocation(new_size, !resized.is_null());
        resized
    }
}

fn record_allocation(bytes: usize, succeeded: bool) {
    if succeeded && TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
        ALLOCATED_BYTES.fetch_add(bytes, Ordering::Relaxed);
    }
}

struct AllocationWindow;

impl AllocationWindow {
    fn start() -> Self {
        ALLOCATED_BYTES.store(0, Ordering::Relaxed);
        TRACK_ALLOCATIONS.store(true, Ordering::Relaxed);
        Self
    }

    fn finish(self) -> usize {
        TRACK_ALLOCATIONS.store(false, Ordering::Relaxed);
        ALLOCATED_BYTES.load(Ordering::Relaxed)
    }
}

impl Drop for AllocationWindow {
    fn drop(&mut self) {
        TRACK_ALLOCATIONS.store(false, Ordering::Relaxed);
    }
}

fn append_event(corpus: &mut String, event: Value) {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .expect("Anthropic fixture event has a type");
    corpus.push_str("event: ");
    corpus.push_str(kind);
    corpus.push_str("\ndata: ");
    corpus.push_str(&serde_json::to_string(&event).expect("serialize fixture event"));
    corpus.push_str("\n\n");
}

fn large_write_stream(content_bytes: usize, delta_count: usize) -> String {
    assert!(delta_count > 0);
    let arguments = json!({
        "path": "benchmark.txt",
        "content": "x".repeat(content_bytes),
    })
    .to_string();
    let mut corpus = String::with_capacity(arguments.len() + delta_count * 160 + 512);

    append_event(
        &mut corpus,
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_benchmark",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-5",
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {"input_tokens": 1, "output_tokens": 0}
            }
        }),
    );
    append_event(
        &mut corpus,
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_benchmark",
                "name": "write",
                "input": {}
            }
        }),
    );

    for index in 0..delta_count {
        let start = arguments.len() * index / delta_count;
        let end = arguments.len() * (index + 1) / delta_count;
        let partial_json = arguments
            .get(start..end)
            .expect("fixture chunks are ASCII and end on UTF-8 boundaries");
        append_event(
            &mut corpus,
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": partial_json}
            }),
        );
    }

    append_event(
        &mut corpus,
        json!({"type": "content_block_stop", "index": 0}),
    );
    append_event(
        &mut corpus,
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use", "stop_sequence": null},
            "usage": {"output_tokens": 1}
        }),
    );
    append_event(&mut corpus, json!({"type": "message_stop"}));
    corpus
}

#[test]
#[ignore = "performance budget; run with --release -- --ignored --nocapture"]
fn large_tool_call_decodes_within_allocation_and_time_budgets() {
    let model = load_builtin_models()
        .into_iter()
        .find(|model| model.api == "anthropic-messages")
        .expect("Anthropic model");
    let corpus = large_write_stream(200_000, 2_000);

    let started = Instant::now();
    let allocation_window = AllocationWindow::start();
    let events = replay_sse_events(&model, &corpus);
    let message = complete_from_events(&events).expect("completed stream message");
    let allocated_bytes = allocation_window.finish();
    let elapsed = started.elapsed();

    match message.content.last() {
        Some(ContentBlock::ToolCall {
            name, arguments, ..
        }) => {
            assert_eq!(name, "write");
            assert_eq!(arguments["content"].as_str().map(str::len), Some(200_000));
        }
        other => panic!("expected final write tool call, got {other:?}"),
    }

    eprintln!("decode: {elapsed:?}; allocated: {allocated_bytes} bytes");
    assert!(
        allocated_bytes < 20 * 1024 * 1024,
        "allocated {allocated_bytes} bytes, expected less than 20 MiB"
    );
    assert!(
        elapsed < Duration::from_millis(200),
        "decode took {elapsed:?}, expected less than 200 ms"
    );
}
