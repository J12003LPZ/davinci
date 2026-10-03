# davinci-agent

`davinci-agent` is the core agentic runtime engine. It manages the assistant reasoning loop, tool registry, concurrent tool execution scheduling, context pruning, permission boundaries, and subagent orchestration.

---

## Key Capabilities

- **Agent Reasoning Loop (`agent.rs`, `turn.rs`)**:
  - Three-stage turn architecture: Prepare (permission checks), Run (parallel tool execution), Finalize (in-order history recording).
  - Integration with `davinci-ai` streaming client for real-time token reception.
- **Built-in Tool Suite (`src/tools/` and `src/` modules)**:
  - File operations: `read`, `write`, `edit`, `apply_patch`.
  - Terminal execution: `bash`, `powershell`, background jobs (`job_output`, `job_kill`).
  - Search & Discovery: `grep`, `find`, `ls`.
  - Network & Web: `web_fetch`, `web_search`.
  - Task & Subagent: `todo`, `agent` (scoped child workers), `batch` (multi-operation execution).
  - Notebooks & MCP: `notebook_edit`, `mcp_read`.
- **Multi-Lane Tool Scheduler (`scheduler.rs`)**:
  - Lanes: Read-only operations, safe MCP calls, and subagent tasks execute concurrently on up to 8 worker threads.
  - Barriers: Write, edit, shell, and side-effect tools act as execution barriers, preserving strict source order.
- **Tool Ledger & Concurrency Safety**:
  - Deduplication ledger hashing canonical arguments to prevent race conditions and duplicate concurrent mutations.
  - Active reservation tracking with condition-variable synchronization.
- **Context Management (`compaction.rs`, `pruning.rs`)**:
  - Verbatim prompt compaction preserving parity token contracts.
  - Large tool output pruning replacing old verbose results with concise markers when approaching window limits.
- **Permission Boundaries (`permission.rs`)**:
  - Modes: `read-only`, `ask`, `edits`, and `auto`.
  - Glob matching on tool names and file targets with fallback to interactive user approval.

---

## Directory Structure

```
davinci-agent/
├── src/
│   ├── agent.rs             # Core Agent struct and conversation cycle
│   ├── turn.rs              # Turn preparation, execution, and finalization
│   ├── scheduler.rs         # Multi-lane concurrent tool scheduler
│   ├── permission.rs        # Tool permission checks and rule evaluation
│   ├── subagent.rs          # Subagent spawning, scoping, and task fanout
│   ├── compaction.rs        # Context compaction and summary generation
│   ├── pruning.rs           # Context budget and tool result pruning
│   ├── evidence.rs          # Offload storage for oversized tool outputs
│   ├── jobs.rs              # Background process tracking and stdin/stdout management
│   ├── tools/               # Tool trait implementations
│   └── lib.rs               # Library entry and strategy declarations
└── Cargo.toml
```

---

## Testing

```bash
# Run all tests in the agent crate
cargo test -p davinci-agent

# Run specific tests (e.g. compaction or scheduler)
cargo test -p davinci-agent compaction
cargo test -p davinci-agent scheduler
```
