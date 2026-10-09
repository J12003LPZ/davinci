//! WOR-83: replaying a runtime sidecar must not hold the whole file in memory.
//! A counting allocator measures the peak heap while a ~25 MB log is read into
//! a small typed record.
use davinci_session::{read_runtime_log, runtime_log_path};
use serde::Deserialize;
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

#[derive(Deserialize)]
struct Row {
    #[allow(dead_code)]
    sequence: u32,
}

#[test]
fn wor83_runtime_log_replay_does_not_buffer_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = runtime_log_path(&dir.path().join("s.jsonl"));
    let pad = "p".repeat(220);
    {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        for sequence in 0..100_000u32 {
            writeln!(
                out,
                "{{\"schema_version\":1,\"sequence\":{sequence},\"pad\":\"{pad}\"}}"
            )
            .unwrap();
        }
        // A torn final line is still tolerated.
        write!(out, "{{\"schema_version\":1,\"sequence\":").unwrap();
    }
    let file_bytes = std::fs::metadata(&path).unwrap().len() as usize;
    assert!(file_bytes > 20_000_000);

    PEAK.store(CURRENT.load(Ordering::SeqCst), Ordering::SeqCst);
    let base = CURRENT.load(Ordering::SeqCst);
    let rows: Vec<Row> = read_runtime_log(&path).unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - base;

    assert_eq!(rows.len(), 100_000);
    assert!(
        peak < file_bytes / 4,
        "peak heap {peak} bytes for a {file_bytes} byte log"
    );
}
