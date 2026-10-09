//! WOR-84: listing sessions must not hold each transcript in memory. A counting
//! allocator measures the peak heap while a ~25 MB session is summarized; its
//! message text is small, the bulk is non-message entries.
use davinci_session::{discover_sessions, JsonlSession, SessionEntry};
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
        PEAK.fetch_max(now, Ordering::SeqCst);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn wor84_session_listing_streams_transcripts() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "/tmp/work", Some("big")).unwrap();
    session
        .append_entry(SessionEntry::message(
            "user",
            serde_json::json!([{"type":"text","text":"first question"}]),
        ))
        .unwrap();
    let path = session.path.clone();
    drop(session);
    {
        let mut out = std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap(),
        );
        let pad = "p".repeat(220);
        for index in 0..100_000 {
            writeln!(
                out,
                "{{\"kind\":\"entry\",\"type\":\"custom\",\"id\":\"c{index}\",\"pad\":\"{pad}\"}}"
            )
            .unwrap();
        }
    }
    let file_bytes = std::fs::metadata(&path).unwrap().len() as usize;
    assert!(file_bytes > 20_000_000);

    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let found = discover_sessions(dir.path(), None).unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - base;

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].all_messages_text, "first question");
    assert_eq!(found[0].message_count, 1);
    assert!(
        peak < file_bytes / 4,
        "peak heap {peak} bytes for a {file_bytes} byte transcript"
    );
}
