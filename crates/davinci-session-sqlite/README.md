# davinci-session-sqlite

`davinci-session-sqlite` provides an embedded SQLite indexing and cache layer on top of raw session files.

---

## Key Capabilities

- **Branch Snapshot Cache (`branch_cache.rs`)**:
  - Maintains indexed SQLite tables indexing session IDs, turn sequences, message IDs, and branch pointers.
  - Enables instant `/resume` and `/tree` UI lookups without sequentially parsing large multi-megabyte JSONL files from disk.
- **Embedded Engine**:
  - Leverages bundled `rusqlite` with zero external database dependencies.
  - Safe concurrent access with busy-timeout and transaction boundaries.

---

## Testing

```bash
cargo test -p davinci-session-sqlite
```
