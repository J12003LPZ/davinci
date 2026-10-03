#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / "crates/davinci-coding-agent/src/native_extensions/vector_memory.rs"
text = path.read_text()

old = '''pub fn fuse_hits(mut hits: Vec<MemoryHit>, limit: usize) -> Vec<MemoryHit> {
    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    hits.truncate(limit);
    hits
}

'''
count = text.count(old)
if count != 1:
    raise SystemExit(f"expected one obsolete fuse_hits helper, found {count}")
path.write_text(text.replace(old, "", 1))
