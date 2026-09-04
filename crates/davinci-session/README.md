# davinci-session

`davinci-session` manages conversation history, persistence, session discovery, and message branching.

---

## Key Capabilities

- **JSONL Session Storage (`jsonl_repo.rs`)**:
  - Encodes turns, messages, tool inputs/outputs, and compaction summaries as line-delimited JSON.
  - Streaming append-only writes ensure session safety against sudden process crashes.
- **Session Discovery & Path Resolution (`discovery.rs`)**:
  - Resolves agent directories (`~/.pi/agent/sessions/` or `$PI_CODING_AGENT_DIR`).
  - Path encoding: Directory paths are encoded using standard hyphen-delimited paths (e.g. `--Users--username--repo--`), fully compatible with TypeScript `pi`.
  - Scans for active sessions and handles cross-platform path transformations (Windows `USERPROFILE` vs Unix `HOME`).
- **Turn & Branch State**:
  - Tracks the turn lineage, allowing users to fork, rewind, or compact conversation trees.

---

## Directory Structure

```
davinci-session/
├── src/
│   ├── lib.rs              # Crate root and re-exports
│   ├── jsonl_repo.rs       # JSONL stream reader/writer
│   ├── repo.rs             # SessionRepository trait and interface
│   ├── discovery.rs        # Path resolution and session catalog scanning
│   └── types.rs            # Session, Turn, and Message data structures
└── Cargo.toml
```

---

## Testing

```bash
cargo test -p davinci-session
```
