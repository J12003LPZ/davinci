# Sandboxed Execution Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a real, capability-reporting execution boundary below DaVinci's existing permission/task policy while preserving supervisor lifecycle, receipts, scheduler semantics, sessions, Context VM, MCP/LSP/browser compatibility, and cross-platform honesty.

**Architecture:** Evolve the existing supervised helper process in `davinci-agent` into the first execution-plane executor. Put serializable sandbox policy/result types in `davinci-protocol`, deterministic broker/backend logic in `davinci-agent::sandbox`, and attach a sandbox envelope to `ProcessConfig` so enforcement occurs inside the helper before the requested child is spawned. Add optional backend implementations incrementally and fail closed when mandatory capabilities are unavailable.

**Tech Stack:** Rust 1.83, serde, existing davinci-protocol framing conventions, existing supervisor process IPC, libc on Unix, existing telemetry/evidence/runtime contracts, optional external bubblewrap/Docker/Podman executables (no mandatory container dependency).

**Spec:** `docs/superpowers/specs/2026-09-29-sandboxed-execution-plane-design.md`

## Global Constraints

- Do not copy proprietary code or assume private Codex internals.
- Current production code/tests at `main@ef4222df4b177277fc16d2106cd37c715106932f` are authoritative.
- Model output, permission mode and approval are not security boundaries.
- Security-sensitive policy resolution is deterministic Rust code.
- Required unsupported controls fail closed.
- Project settings can narrow but never widen user/global sandbox authority.
- Do not mount container-engine sockets, SSH agent sockets or credential directories into restricted sandboxes.
- Preserve Rust 1.83 and existing exact-dependency convention.
- Do not push to `main`; changes live on `chatgpt/sandbox-execution-plane-20260929`.
- No security claim is complete until the corresponding enforcement test passes.
- Benchmark numbers must be measured, never fabricated.

## Review Focus

- A backend advertises a capability it does not actually enforce → tests assert required capability mismatch returns `CapabilityUnavailable` before spawn.
- Parent environment contains plausible secrets → foreground and managed child environment tests assert they are absent unless explicitly injected.
- Workspace path is reachable through a symlink/reparse alias → mount/path validation tests assert escape denial.
- A child/subagent requests more privilege than its parent → capability intersection tests assert monotonic reduction.
- Cancellation/teardown while descendants keep stdio/network handles open → supervisor security tests assert the entire owned lifetime ends within the existing bounded cleanup window.

---

### Task 1: Typed sandbox protocol contract

**Files:**
- Create: `crates/davinci-protocol/src/sandbox.rs`
- Modify: `crates/davinci-protocol/src/lib.rs`
- Test: `crates/davinci-protocol/src/sandbox.rs`

**Interfaces:**
- Produces: `SandboxMode`, `SandboxBackendKind`, `SandboxLifecycle`, `SandboxId`, `MountAccess`, `MountRule`, `FilesystemPolicy`, `NetworkPolicy`, `EnvironmentPolicy`, `ResourcePolicy`, `ProcessPolicy`, `SandboxCapabilities`, `SandboxSpec`, `ExecutionRequest`, `ExecutionResult`, `SandboxReceipt`, `SandboxErrorCode`, `SandboxFailure`.
- Produces: `SandboxCapabilities::satisfies(&self, required: &Self) -> bool`; `SandboxSpec::validate() -> Result<(), SandboxFailure>`; capability subset/intersection helpers for delegation.

- [ ] **Step 1:** Add serialization/validation tests for all four sandbox modes, denied/allowlist/unrestricted network, invalid mount traversal, unknown fields, capability subset and child intersection.
- [ ] **Step 2:** Run `cargo test -p davinci-protocol sandbox --offline --locked`; expected RED before types exist.
- [ ] **Step 3:** Implement the typed contract with `#[serde(deny_unknown_fields)]` on security-sensitive structures and no model-authority fields.
- [ ] **Step 4:** Run the focused protocol tests; expected PASS.
- [ ] **Step 5:** Commit `feat(protocol): add typed sandbox contract`.

### Task 2: Sandbox broker and capability resolution

**Files:**
- Create: `crates/davinci-agent/src/sandbox.rs`
- Modify: `crates/davinci-agent/src/lib.rs`
- Test: `crates/davinci-agent/src/sandbox.rs`

**Interfaces:**
- Consumes: Task 1 protocol types.
- Produces: `SandboxBackend` trait, `PreparedExecution`, `SandboxBroker`, `SandboxStatus`, `resolve_effective_spec(parent, requested)`, `sanitize_environment(policy, parent)`.
- Produces backends: `NoExecutionBackend`, `HostBackend`, `LinuxBubblewrapBackend` preparation, `ContainerBackend` preparation.
- Does not spawn the requested child itself.

- [ ] **Step 1:** Add tests proving full-access host is explicit, no-execution denies, missing required capability fails closed, child policy cannot widen parent, secret ambient environment is absent, and domain allowlist is rejected by a backend without secure allowlist support.
- [ ] **Step 2:** Run focused tests; expected RED.
- [ ] **Step 3:** Implement broker/backend preparation and PATH-based trusted executable discovery without executing project code.
- [ ] **Step 4:** Run focused tests; expected PASS.
- [ ] **Step 5:** Commit `feat(agent): add sandbox broker and capability negotiation`.

### Task 3: Upgrade supervisor wire contract and enforce in executor helper

**Files:**
- Modify: `crates/davinci-agent/src/jobs/supervisor/mod.rs`
- Modify: `crates/davinci-agent/src/jobs/supervisor/wire.rs`
- Modify: `crates/davinci-agent/src/jobs/supervisor/client.rs`
- Modify: `crates/davinci-agent/src/jobs/supervisor/helper.rs`
- Modify: `crates/davinci-agent/src/jobs/supervisor/platform.rs`
- Test: supervisor module tests

**Interfaces:**
- `ProcessConfig` gains `sandbox: SandboxSpec`.
- Helper validates requested spec, selects/prepares backend, and only then constructs the final `Command`.
- `ProcessExecutionEvidence` gains sandbox ID/spec digest/backend/capability fields.
- Bootstrap protocol gains explicit version/token binding without exposing the token to the requested child.

- [ ] **Step 1:** Add tests that a mismatched/invalid sandbox policy fails before child start and that evidence reports effective backend/capabilities.
- [ ] **Step 2:** Run supervisor tests; expected RED.
- [ ] **Step 3:** Thread typed sandbox config through wire/client/helper and call `SandboxBroker::prepare` in the helper.
- [ ] **Step 4:** Preserve current Unix session ownership and Windows Job Object behavior; add resource-limit hooks only where proven.
- [ ] **Step 5:** Run supervisor + foreground tests; expected PASS.
- [ ] **Step 6:** Commit `feat(executor): enforce sandbox policy in process helper`.

### Task 4: Shell and managed-process migration + secret isolation

**Files:**
- Modify: `crates/davinci-agent/src/tools.rs`
- Modify: `crates/davinci-agent/src/tools/foreground.rs`
- Modify: `crates/davinci-agent/src/process_manager.rs`
- Modify: `crates/davinci-agent/src/process_manager/command.rs`
- Modify: `crates/davinci-agent/src/jobs/managed.rs`
- Test: foreground/process-manager tests

**Interfaces:**
- `ToolContext` carries host-resolved sandbox policy/status, not model JSON.
- Foreground config uses `sanitize_environment`; it no longer copies `std::env::vars_os()`.
- Managed `ProcessConfig` receives the same task/session sandbox spec.

- [ ] **Step 1:** Add regression test with `GITHUB_TOKEN`, `AWS_SECRET_ACCESS_KEY`, `SSH_AUTH_SOCK` and arbitrary private env; assert sandbox child cannot enumerate them.
- [ ] **Step 2:** Add restricted/workspace-write tests for allowed workspace behavior and denied unsupported backend behavior.
- [ ] **Step 3:** Implement migration with compatibility full-access spec only when sandbox compatibility mode explicitly selects it.
- [ ] **Step 4:** Run `cargo test -p davinci-agent foreground process_manager --offline --locked`.
- [ ] **Step 5:** Commit `feat(agent): route command execution through sandbox policy`.

### Task 5: Settings/trust/status surface

**Files:**
- Modify: `crates/davinci-coding-agent/src/settings.rs`
- Create: `crates/davinci-coding-agent/src/sandbox_config.rs`
- Modify: `crates/davinci-coding-agent/src/trust.rs`
- Modify: `crates/davinci-coding-agent/src/args.rs`
- Modify: host/session construction paths that build `Agent` / `ToolContext`
- Modify: slash-command/status UI paths
- Test: settings/trust/args/TUI unit tests
- Docs: `docs/sandbox.md`, `README.md`, `docs/permission-modes.md`

**Interfaces:**
- User JSON key `sandbox`.
- Project policy may narrow only; `resolve_sandbox_settings(user, trusted_project, cli)` returns host-only effective policy.
- Add `--execution-sandbox`; do not reinterpret the current permission `--sandbox` alias silently.
- Add `/sandbox-status` with actual backend capabilities/state.

- [ ] **Step 1:** Tests for project widening rejection, untrusted project ignore, CLI/user precedence, and status wording.
- [ ] **Step 2:** Implement settings resolver and host wiring.
- [ ] **Step 3:** Add status output that distinguishes disabled/available/active/degraded/unavailable.
- [ ] **Step 4:** Run coding-agent/TUI focused tests.
- [ ] **Step 5:** Commit `feat(cli): configure and report execution sandbox`.

### Task 6: Verification/evidence/Context VM integration

**Files:**
- Modify: `crates/davinci-agent/src/runtime/evidence_store.rs`
- Modify: `crates/davinci-agent/src/command_receipt.rs`
- Modify: `crates/davinci-agent/src/jobs.rs`
- Modify: `crates/davinci-agent/src/transaction_verification.rs`
- Modify: verification planning/summary paths
- Modify: telemetry structures in `davinci-telemetry`
- Test: receipt/verification/evidence tests

**Interfaces:**
- Execution receipts carry sandbox identity/policy digest/backend/effective capabilities and workspace generation.
- Raw logs remain artifacts; model-facing result remains bounded summary.
- Verification request reuses exact sandbox/workspace identity from mutation generation.

- [ ] **Step 1:** Add receipt roundtrip and same-workspace verification tests.
- [ ] **Step 2:** Add test that model text alone cannot satisfy verification.
- [ ] **Step 3:** Implement receipt + telemetry extensions without logging secret values.
- [ ] **Step 4:** Run agent/telemetry verification suites.
- [ ] **Step 5:** Commit `feat(verification): bind evidence to sandbox execution`.

### Task 7: Scheduler, subagents and agent teams

**Files:**
- Modify: `crates/davinci-agent/src/scheduler.rs`
- Modify: `crates/davinci-agent/src/subagent.rs`
- Modify: child execution context/runtime registry modules
- Modify: graph worker construction in coding-agent
- Test: scheduler/subagent/runtime integration tests

**Interfaces:**
- Add `SandboxSharingPolicy::{Inherit, Isolated, SharedReadOnly, SharedWorkspace}`.
- Child effective sandbox policy is intersection/subset of parent.
- Scheduler key includes sandbox identity/sharing policy for mutable execution.

- [ ] **Step 1:** Add child privilege-escalation tests for filesystem/network/full-access/backend weakening.
- [ ] **Step 2:** Add parallel isolation test for two writable workers and shared read-only test.
- [ ] **Step 3:** Implement deterministic sandbox assignment without changing existing mutation barriers.
- [ ] **Step 4:** Run scheduler/subagent/graph tests.
- [ ] **Step 5:** Commit `feat(runtime): bind workers to sandbox authority`.

### Task 8: Local MCP, LSP, browser, extension/hook, package and graph migration

**Files:**
- Modify: `crates/davinci-mcp/src/config.rs`, `stdio.rs`
- Modify: `crates/davinci-agent/src/mcp.rs`
- Modify: `semantic/manager.rs`, `native_extensions/language_intelligence/transport.rs`
- Modify: `interaction_testing/browser_process.rs`
- Modify: `js_host.rs`, `hooks.rs`
- Modify: `packages.rs`
- Modify: `native_extensions/graph/process.rs`, Git/build/package intelligence runners
- Test: each subsystem's existing unit tests plus sandbox policy tests

**Interfaces:**
- MCP execution policy `remote|host|sandboxed|disabled`; project local executable defaults sandboxed.
- LSP execution policy `host|sandboxed|disabled`; project-selected defaults sandboxed.
- Browser gets dedicated sandbox spec/network policy.
- Package/install scripts and graph shell paths use brokered execution.

- [ ] **Step 1:** Add MCP/LSP privilege-escalation RED tests.
- [ ] **Step 2:** Migrate each subprocess family one at a time, retaining existing application authorization.
- [ ] **Step 3:** After each family, run its focused tests before moving on.
- [ ] **Step 4:** Search production code for remaining unclassified `Command::new` / `.spawn()` and update the inventory with disposition.
- [ ] **Step 5:** Commit `feat(sandbox): route project-controlled subprocesses through executor`.

### Task 9: Security evals and cross-platform fail-closed tests

**Files:**
- Create: `crates/davinci-evals/src/security_eval/sandbox.rs`
- Modify security-eval module registration
- Add fixtures under `crates/davinci-evals/fixtures/repos/security-sandbox/`
- Add platform-specific agent tests

**Interfaces:**
- Deterministic enforcement fixtures independent of model compliance.
- Backend capability tests gate claims per OS/runtime.

- [ ] **Step 1:** Add read/write/symlink/env/network/localhost/metadata/timeout/process-tree/background survival cases.
- [ ] **Step 2:** Add child-agent and local-MCP privilege escalation cases.
- [ ] **Step 3:** Add cancellation + 4/8 parallel workers.
- [ ] **Step 4:** Run supported security evals; unsupported backend cases must assert explicit capability failure rather than skip silently.
- [ ] **Step 5:** Commit `test(security): prove sandbox enforcement boundaries`.

### Task 10: Benchmarks, recovery, telemetry and final documentation

**Files:**
- Create reproducible sandbox benchmark under existing perf/eval conventions
- Modify execution recovery docs/code for sandbox crash classification
- Create: `docs/sandbox.md`
- Modify: `docs/ARCHITECTURE.md`, `docs/permission-modes.md`, runtime verification/recovery docs, README
- Update inventory with remaining trusted host spawns

**Interfaces:**
- Benchmark reports native compatibility vs sandbox startup/execution/build/test/4-worker/8-worker numbers from actual runs only.
- Recovery distinguishes `SandboxCrashed` / `CleanupFailure` and follows existing replay policy.

- [ ] **Step 1:** Add bounded sandbox-crash recovery test with no blind retry of unknown side effects.
- [ ] **Step 2:** Add benchmark harness and run it only on a capable machine; record environment + actual numbers.
- [ ] **Step 3:** Document platform capability matrix and caveats; include control/execution-plane diagrams.
- [ ] **Step 4:** Run `cargo fmt --all --check`, `cargo check --workspace --all-targets --offline --locked`, affected tests, clippy where available.
- [ ] **Step 5:** Search for remaining execution paths and reconcile every one with the inventory.
- [ ] **Step 6:** Commit `docs(security): document sandbox guarantees and limits`.

## Final acceptance gate

Do not label the feature complete until:

- ordinary coding build/test commands run inside an active sandbox on at least one supported backend;
- outside writes and sensitive host reads fail by enforcement;
- ambient credentials are absent;
- network denial is independent of workspace write access;
- descendants die on teardown;
- supported resource limits are proven;
- child agents/MCP cannot widen capability;
- verification uses the same workspace/sandbox identity;
- receipts/evidence include sandbox facts;
- capability mismatch fails before spawn;
- existing permission, Context VM, scheduler, session and persistence tests pass;
- security evals pass on each claimed backend;
- benchmark overhead is measured and documented.
