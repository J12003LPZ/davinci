# davinci-coding-agent

`davinci-coding-agent` is the primary product application crate that compiles into the `davinci` executable binary. It orchestrates user interaction, CLI parsing, terminal rendering, extensions, and lifecycle execution.

---

## Key Capabilities

- **CLI Dispatch & Modes (`main.rs`)**:
  - `run_interactive`: Starts the rich Ratatui TUI shell.
  - `run_print`: Headless single-turn output (`--print` / `-p`), pipe processing, and automated scripts.
  - `run_rpc`: Headless JSON-RPC over stdio (`--mode rpc`).
- **Interactive Shell (`davinci_interactive.rs`, `davinci_sources.rs`)**:
  - Full-featured terminal interface connecting the user to `davinci_tui`.
  - Slash command parser (`/help`, `/model`, `/thinking`, `/permissions`, `/diff`, `/graph`, etc.).
  - Real-time fact providers dressing command sheets with live git status, token meters, and context windows.
- **Native Extensions (`src/native_extensions/`)**:
  - **Vector Memory (`vector_memory.rs`)**: Indexes conversation messages using local embeddings (e.g. via Ollama) for semantic recall.
  - **Token Governor (`token_governor.rs`)**: Output digestion, large output offloading, and deduplication of repeated reads within a rolling window.
  - **Security Scanner (`security_scan.rs`)**: Static threat analysis scanning tool runs, paths, and repositories offline.
  - **Graph Engine (`graph/`)**: Multi-worker task graph executor running scoped `--print` subagent workers.
  - **Learning (`learning/`)**: Fail-open turn review, candidate artifact evaluation, safe procedural skill creation (`SKILL.md`), progressive disclosure notices, and `/learn` command.
- **JavaScript Extensions (`extension_host.rs`, `js_host.rs`)**:
  - Embedded runner (`extension_runner.js`) driving custom JS plugins in a subprocess when Node.js is installed.
- **Trust & Permissions (`trust.rs`, `permissions.rs`)**:
  - User and project trust boundaries, sandboxing levels, and interactive permission gates (`auto`, `ask`, `edits`, `read-only`).

---

## Directory Structure

```
davinci-coding-agent/
├── src/
│   ├── main.rs                   # Entry point and mode dispatch
│   ├── davinci_interactive.rs    # Interactive TUI session runner and slash commands
│   ├── davinci_sources.rs        # Live data bindings for TUI status sheets
│   ├── args.rs                   # Clap CLI argument definitions
│   ├── settings.rs               # ~/.pi/agent/settings.json reader/writer
│   ├── trust.rs                  # Project trust decision store
│   ├── permissions.rs            # Permission policy evaluator
│   ├── hooks.rs                  # Lifecycle hook execution engine
│   ├── extension_host.rs         # Native and JS extension manager
│   ├── js_host.rs                # Node subprocess bridge
│   ├── native_extensions/        # Vector memory, token governor, security scan, graph
│   └── sdk.rs                    # Public programmatic embedding SDK
└── Cargo.toml
```

---

## Building & Testing

```bash
# Build the binary
cargo build -p davinci-coding-agent

# Run all crate unit tests
cargo test -p davinci-coding-agent

# Run with stdout logging
cargo test -p davinci-coding-agent -- --nocapture
```
