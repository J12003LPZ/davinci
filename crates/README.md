# Rust workspace

The active product consists of **14 workspace crates**. Start with the
[architecture guide](../docs/ARCHITECTURE.md) for entry points, data flow,
configuration, and the boundaries between them.

| Area | Crates |
| --- | --- |
| Product assembly and UI | [davinci-coding-agent](davinci-coding-agent/), [davinci-tui](davinci-tui/), [davinci-voice](davinci-voice/) |
| Execution and provider access | [davinci-agent](davinci-agent/), [davinci-ai](davinci-ai/), [davinci-mcp](davinci-mcp/) |
| Persistence | [davinci-session](davinci-session/), [davinci-session-sqlite](davinci-session-sqlite/) |
| Typed IPC | [davinci-protocol](davinci-protocol/), [davinci-client](davinci-client/), [davinci-server](davinci-server/) |
| Measurement and compatibility | [davinci-telemetry](davinci-telemetry/), [davinci-evals](davinci-evals/), [davinci-parity](davinci-parity/) |

`davinci-core/` is an archived early port, excluded from the workspace.
The pinned TypeScript reference lives in [vendor/davinci](../vendor/davinci/);
the directories in [packages](../packages/) are legacy migration stubs.

Use the root `Cargo.toml` and each crate's `src/lib.rs` / `src/main.rs` to
establish what is compiled. A Rust source file's presence alone does not make
it part of a crate.

Keep exact dependency pins and Rust 1.83 compatibility. Scope tests to changed
behavior and its callers; most unit tests are inline, while larger integration
and evaluation fixtures live in each crate's `tests/` directory. Some tests
start local subprocesses or loopback servers. Platform-specific unsafe code
exists for filesystem handles, process management, and voice bindings, so
platform validation is a separate requirement from compiling on one host.
