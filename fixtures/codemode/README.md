# Codemode fixture boundary

Baseline: `08bf0038a40c469e57a5f12c9d753e580b198e0b`. No local checkout, dirty-state, or local Rust execution is claimed: repository inspection and writes use GitHub, and tests use GitHub Actions. Rust is pinned to 1.83.0. Default coding-agent features are empty; `test-fixtures`, `interaction-testing`, and `experimental-ipc` remain explicit.

## Current production ownership

- Provider ingress: `Agent::execute_tool_batch` in `crates/davinci-agent/src/turn.rs`
- Preparation: `prepare_tool_call_with_origin` canonicalizes names, persists journal intent before authority hooks, runs runtime decisions and pre-hooks, checks active tools/contracts/effects/current permission, selects the lane, and authorizes/queues the journal operation
- Execution: `run_prepared_call` rechecks contracts/effects/dispatch approval, observes cancellation and journal authority, dispatches through the normal adapter, and commits execution evidence
- Direct publication: `finalize_tool_call`; batch publication: `run_batch_with_parent_operation` in `batch.rs`. Both run post-hooks, mutation/verification observation, receipt handling and journal-presentation caching. These are the seams to consolidate, not bypass
- Scheduling: `scheduler::run_lanes_with_cancel`, normal root/lane controls, and supervisor ownership. Read parallelism does not authorize parallel writes
- Operation identities: `ToolOperationOrigin` in `lib.rs`; `ToolOperationPlanner`, `ToolOperationRuntime`, and `ToolOperationDispatcher` under `runtime/operations/`. The existing journal remains the sole authority
- MCP projection: `McpRegistry::call` currently projects `result.text()` and `is_error`, dropping structured data. `davinci-mcp/src/types.rs` does not retain `structuredContent` or `outputSchema`. Supervised MCP ownership is `davinci-agent/src/mcp/sandboxed.rs`
- Governor: `TokenGovernor::after_tool`, `retrieve`, and `retrieve_artifact` in `native_extensions/token_governor.rs`; host integration through `extension_host::native_before_tool/native_after_tool`
- Durable entry construction: `runtime_host::attach_operation_runtime` and `configure_session_workflow`. Sessionless use stays ephemeral
- Packaging owners: `scripts/install.sh`, `scripts/install-davinci.ps1`, `scripts/release_identity.py`, and the existing CI/release workflows. Cargo-only installation must stay valid

## Fixtures and actual gates

`mcp-results.json` contains only invented data: structured, legacy text, explicit tool failure, mixed/binary placeholders, hostile optional metadata, output schema and large-identifier cases. No real credentials, accounts, home-directory reads, or external MCP services are involved. Lost-response and delay cases describe required later disposable adapters, not measurements.

`crates/davinci-agent/tests/common/codemode.rs` is included as a test-only child of `turn.rs` to exercise its private production dispatcher without expanding the public SDK. It characterizes read hooks and durable results, denied writes with zero effects, and unchanged MCP text/error decoding.

Existing Governor tests `compression_preserves_notable_lines_and_is_reversible`, `retrieval_is_paged_and_says_where_to_continue`, and the permission-revision retrieval test establish recovery behavior. Existing `operation_dispatch` integration tests cover durable admission and duplicate identities.

Focused CI explicitly runs nonempty batch, tool-ledger, MCP, Codemode baseline, operation-dispatch and Governor suites after `cargo fetch --locked`, then uses `--offline --locked`. RTK is not installed by this fixture job; raw Cargo execution is the reported fallback. Full existing CI remains in place.

See the [execution record](../../docs/superpowers/plans/2026-10-04-codemode-progress.md) for actual run identities, failures and remaining gates. Pending tests are not passing evidence.
