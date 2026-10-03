# DaVinci execution-boundary inventory — 2026-09-29

This inventory is based on production code at `main@ef4222df4b177277fc16d2106cd37c715106932f`. Current code and tests are authoritative; older plans are background only.

## Boundary definition

The **control plane** is trusted DaVinci code that plans, schedules, applies application policy, owns sessions/evidence, and brokers execution.

The **execution plane** is any process that may execute repository-controlled or model-generated code, configuration, scripts, package hooks, compiler plugins, tests, language servers, local MCP executables, browser automation dependencies, or extension code.

Application permission checks and task contracts remain necessary, but they are not OS isolation.

## Existing execution-plane seed

DaVinci already has a useful process boundary in `crates/davinci-agent/src/jobs/supervisor/`:

- `client.rs` launches a dedicated helper with a minimal helper environment and private stdin/stdout protocol.
- `wire.rs` carries typed `ProcessConfig` / lifecycle events with bounded frames.
- `helper.rs` is the only process in that path that calls `Command::new(config.executable)` for the requested child.
- `platform.rs` establishes process-tree ownership before the child is spawned: `setsid()` on Unix and a kill-on-close Job Object on Windows.
- `tools/foreground.rs` routes foreground shell execution through the supervisor and records process evidence.
- `jobs/managed.rs` reuses the same supervisor for owned long-lived processes.

This gives DaVinci a process/lifecycle boundary, but **not** filesystem, network, environment, or resource isolation for the requested child.

## Process-spawn inventory

| Area | Production path | Current behavior | Sandbox disposition |
|---|---|---|---|
| Foreground shell / `exec_command` | `crates/davinci-agent/src/tools/foreground.rs` → `jobs/supervisor` | Supervised helper, but foreground config copies the entire parent environment | Route through sandbox policy; sanitize environment; enforce backend below helper |
| Background jobs / managed processes | `crates/davinci-agent/src/jobs.rs`, `jobs/managed.rs`, `process_manager/command.rs` | Process-tree ownership and bounded managed-process count; managed command resolver already builds a small environment | Attach sandbox spec to every managed process; keep ownership/reuse logic |
| Supervisor helper | `crates/davinci-agent/src/jobs/supervisor/client.rs` | Trusted host helper launch | Remains trusted broker-side spawn; never receives repository credentials |
| Requested child | `crates/davinci-agent/src/jobs/supervisor/helper.rs` | Direct host `Command::new` after helper handshake | Becomes the enforcement point that prepares a backend-specific sandbox launch |
| Verification Python inspector | `crates/davinci-agent/src/verification.rs` | Host Python `-I -S` process | Classify as trusted parser helper only for bounded source inspection; project builds/tests must use sandbox |
| Worktree/Git runtime | `crates/davinci-agent/src/runtime/worktree.rs` | Host Git | Trusted control-plane Git for DaVinci-owned worktree bookkeeping; repository hooks/config must be disabled or isolated |
| Local MCP stdio | `crates/davinci-mcp/src/stdio.rs`, `davinci-agent/src/mcp.rs` | Direct child process, explicit environment helper | Project/local executable MCP defaults to sandboxed; trusted user host MCP must be explicit |
| MCP HTTP | `crates/davinci-mcp/src/http.rs` | Control-plane `ureq` networking | Remote/trusted service path; do not confuse with sandbox process network |
| JS extensions | `crates/davinci-coding-agent/src/js_host.rs` | Direct Node child for extension module | Project/plugin-controlled JS runs sandboxed by default; trusted user extension may opt into host policy |
| Hooks | `crates/davinci-coding-agent/src/hooks.rs` | Direct child with `env_clear` plus allowlisted baseline | Project-controlled executable hooks run sandboxed; approval alone never grants host access |
| Language intelligence | `native_extensions/language_intelligence/transport.rs`, `semantic/manager.rs` | Direct language-server child; newer manager sanitizes environment and has execution policy | Add execution location policy: host only for explicitly trusted server; project-resolved server sandboxed |
| Browser automation bridge | `interaction_testing/browser_process.rs`, `native_extensions/browser.rs` | Existing supervisor, private temp host, pinned host Node/package validation | Attach explicit browser sandbox spec; browser network is a separate capability and must not inherit shell policy |
| Graph workers / shell | `native_extensions/graph/process.rs`, `graph/git.rs` | Direct shell/child process and process-group cleanup | Route command execution through sandbox broker; worktree identity maps to sandbox identity |
| Git intelligence | `native_extensions/git_intelligence/runner.rs` | Direct host Git; strips `GIT_*`; rejects repo-owned git executable | Prefer brokered read-only sandbox; trusted plumbing Git can remain control-plane only when hooks/config are disabled |
| Package installation | `crates/davinci-coding-agent/src/packages.rs` | Direct Git/npm execution; npm scripts can execute package code | Sandbox package managers; network capability explicit; package scripts never gain host filesystem/secrets |
| Plugin marketplace/update | `plugins/*`, `packages.rs` | Host filesystem + Git/npm paths | Download/install staging is control-plane; any package/plugin install script executes in sandbox |
| External editor / UI browser opener | `external_editor.rs`, `davinci-tui/src/open_browser.rs` | User-invoked host process | Host-only user action, not model execution; label outside sandbox guarantee |
| Provider/auth/network stack | `davinci-ai/src/http.rs`, `codex*.rs`, auth modules | Trusted control-plane network | Stays outside execution sandbox; secrets must never be copied into sandbox env |
| Web tools | `crates/davinci-agent/src/web.rs` | Explicit application network tool | Control-plane capability governed by tool policy, not process-sandbox network |
| Evals/tests | `crates/davinci-evals/*`, test fixtures | Spawn harnesses/competitors/fixtures | Test infrastructure; security evals must exercise real backend enforcement, not model refusal |

## Filesystem mutation inventory

Model-visible and repository-affecting mutations enter through:

- `tools.rs` built-in `write` / `edit`, `apply_patch.rs`, `notebook.rs`.
- `runtime/transactions/*`, `transaction_verification.rs`, `file_mutation_queue.rs`, checkpoints/rewind.
- Shell, package manager, build/test/compiler, LSP, hook, extension, MCP, browser and graph child processes, whose filesystem effects are not fully knowable at dispatch time.
- Coding-agent package/plugin installation and configuration persistence.

The first group remains governed by existing path/task-contract logic. The second group needs OS-enforced mounts because pre-dispatch path checks cannot constrain arbitrary child syscalls.

## Network-capable inventory

- Trusted control-plane: provider HTTP/WebSocket, OAuth callback, web tools, remote MCP HTTP, update/model-download paths.
- Untrusted/process-derived: shell commands, package managers, test binaries, compilers/plugins, local MCP executables, LSP servers, extension/hook Node processes, browser automation, graph workers.

Only the latter category is subject to sandbox network policy. A domain allowlist is not considered enforced merely by checking the original URL; redirects, DNS rebinding/resolution, IPv4/IPv6, localhost/private/link-local/metadata targets must be controlled below the process.

## Project-controlled executable sources

Treat as untrusted unless explicitly elevated by user/global policy:

- repository binaries/scripts and test executables;
- package-manager lifecycle scripts;
- compiler/build plugins and build scripts;
- project language-server settings and `node_modules/.bin` tools;
- project MCP stdio commands;
- project hooks;
- project/plugin JS extensions;
- browser dependencies selected from project state;
- project configuration that changes commands or executable paths.

## Current security properties worth preserving

- Permission mode and task-contract checks are already separate from OS claims.
- `runtime/contracts.rs` fails closed where its application-level contract cannot prove process/network containment.
- Foreground and managed process paths have deterministic process-tree ownership and cancellation.
- Command receipts/evidence already bind executable, argv, cwd, environment references/digest, exit state, and output hashes.
- Scheduler mutation barriers and subagent scope reduction already exist.
- Browser bridge has private temporary files and executable provenance checks.
- LSP manager already has sanitized-environment helpers and trust/hash policy.
- Managed-process command resolution rejects common interpreter/loader injection variables.

These are retained and moved above or into the sandbox boundary rather than replaced.

## Architectural gaps to close

1. No first-class `SandboxSpec` / capability contract shared across the process boundary.
2. Supervisor protocol has process correlation but no explicit sandbox protocol version/capability negotiation.
3. Foreground shell currently copies the complete parent environment.
4. The helper launches requested children on the host with no filesystem/network sandbox.
5. Resource policy is incomplete: timeout/output bounds exist, but hard CPU/memory/PID/disk limits vary by platform and backend.
6. Several direct process-spawn paths bypass the supervisor.
7. Local MCP, LSP, extensions, hooks, package managers, graph workers and browser processes lack one common execution policy.
8. Sandbox identity is not represented in scheduler/subagent/verification evidence.
9. The current CLI `--sandbox` spelling is only a permission-mode alias and must not be presented as OS sandboxing.
10. There is no backend capability report that can fail closed when a requested property is unavailable.

## Initial backend capability truth

The first implementation must report capabilities, not promises:

- **Linux native**: support a bubblewrap-style backend when a trusted `bwrap` executable is available. Filesystem mount isolation, process namespace isolation and network-denied mode can be enforced. Domain allowlists remain unsupported unless a real egress proxy/firewall backend is added.
- **Optional Docker/Podman**: support disabled network, read-only root, workspace mounts and runtime resource flags without making a container runtime mandatory. Never mount the container-engine socket.
- **Windows native**: retain Job Object process-tree ownership. Until restricted token/AppContainer filesystem+network confinement is implemented and tested, restricted/workspace-write modes that require those properties fail closed. Container backend may satisfy them when available.
- **macOS native**: do not claim equivalent kernel isolation from deprecated/insufficient mechanisms. Restricted/workspace-write must use a backend that truthfully provides the requested capabilities or fail closed.
- **Full access host**: explicit escape hatch only; it reports process lifecycle/timeout/output controls but no filesystem or network isolation.

