# Sandboxed execution plane design

**Date:** 2026-09-29  
**Repository baseline:** `main@ef4222df4b177277fc16d2106cd37c715106932f`  
**Inventory:** `docs/security/sandbox-execution-boundary-inventory-2026-09-29.md`

## Goal

Turn DaVinci from an agent runtime whose dangerous child processes are governed primarily by application policy into a runtime with a genuine, independently enforced execution boundary:

```text
TRUSTED CONTROL PLANE
Agent / Context VM / Memory / Scheduler / Permission + Task Policy
                    |
                    v
              Sandbox Broker
                    |
            typed authenticated IPC
====================|====================
                    v
UNTRUSTED EXECUTION PLANE
shell / code / tests / packages / local MCP / LSP / browser workers
                    |
                    v
          backend OS enforcement
```

The model is never a security boundary. User approval is never a replacement for the sandbox.

## Threat model

Assume all of the following may be malicious:

- model-generated commands and code;
- repository files, symlinks, junctions/reparse points and project configuration;
- package lifecycle scripts and dependency build scripts;
- compiler plugins, build scripts and test binaries;
- local MCP servers;
- project-selected language servers;
- extension/hook JavaScript or executable hooks;
- browser automation dependencies and pages;
- Git configuration/hooks reachable from repository operations.

Protect the host, to the extent reported by the active backend, against:

- reads/writes outside authorized mounts, including symlink/path traversal escapes;
- deletion outside the workspace;
- access to SSH/cloud/browser credentials and ambient environment secrets;
- package post-install code escaping the workspace;
- destructive shell commands affecting the host;
- uncontrolled descendants, fork/process bombs and process persistence;
- runaway CPU/memory/output/disk/time consumption;
- unrestricted outbound network, localhost/private/link-local/metadata probing;
- privilege escalation through subagent delegation, MCP or LSP;
- execution continuing after cancellation/session teardown.

The threat model does **not** claim protection from a hostile host administrator/kernel/hypervisor, a compromised container runtime, or a same-privilege host attacker that already controls DaVinci itself.

## Security layers

The layers remain distinct and execute in this order:

1. **User approval** — consent to attempt an operation.
2. **Application policy** — permission mode, task contract, role/capability/delegation rules.
3. **Sandbox policy resolution** — deterministic conversion of task/user/global settings to a typed `SandboxSpec`; requested mandatory controls must be supported.
4. **Backend enforcement** — namespace/container/OS primitives applied by the execution-plane helper.
5. **VM/container isolation** — optional stronger backend where available; not equivalent to an application policy.

Approval may make an operation eligible. It never weakens the resolved sandbox unless the user explicitly selects the separate `full_access` sandbox mode.

## Chosen architecture

### Why evolve the existing supervisor

Three approaches were considered:

1. **New executor stack immediately.** Clean separation, but duplicates DaVinci's existing process ownership, output framing, cancellation and evidence plumbing before delivering security.
2. **Evolve the existing supervisor helper into the execution plane.** Reuses a real process boundary already used by shell/browser/managed processes, preserves receipts and cleanup, and keeps the change incremental. **Chosen.**
3. **Container-only execution.** Stronger when a runtime exists, but makes Docker/Podman a mandatory product dependency and breaks current installations. Rejected as the only backend; retained as optional.

The existing helper becomes the first `davinci-executor` implementation even when it is packaged in the same binary initially. Its protocol and backend interface are independent of the agent and intentionally serializable so a dedicated binary/remote executor can be substituted later without moving policy reasoning into the executor.

### Core ownership

```text
Agent / scheduler / verification
        |
        | resolved application authority
        v
SandboxBroker (control plane)
        |
        | SandboxSpec + ExecutionRequest
        v
process supervisor client
        |
        | private authenticated, versioned protocol
================ SECURITY BOUNDARY ================
        |
        v
executor helper
        |
        | validate spec against backend capabilities
        v
SandboxBackend
   | linux-bwrap
   | container
   | host-full-access
   | unavailable/deny
        |
        v
requested child process tree
```

The helper must never ask the LLM whether a syscall/path/host is allowed. It only applies typed deterministic policy.

## Typed sandbox contract

Types live in `davinci-protocol` so control-plane and execution-plane code share one representation without ad-hoc JSON:

- `SandboxMode::{NoExecution, Restricted, WorkspaceWrite, FullAccess}`
- `SandboxBackendKind::{Auto, LinuxBubblewrap, MacosSeatbelt, Container, Host}`
- `SandboxLifecycle::{Created, Ready, Running, Stopping, Stopped, Failed}`
- `SandboxId` / `SandboxReceipt`
- `SandboxSpec`
- `ExecutionRequest` / `ExecutionResult`
- `FilesystemPolicy`, `MountRule`, `MountAccess`
- `NetworkPolicy::{Denied, AllowList { domains, ports }, Unrestricted}`
- `EnvironmentPolicy`
- `ResourcePolicy`
- `ProcessPolicy`
- `SandboxCapabilities`
- `SandboxErrorCode` / typed `SandboxFailure`

Unknown fields on security-sensitive wire types are rejected.

A `SandboxSpec` contains only policy and host-resolved paths. Model tool JSON never deserializes directly into one. Project configuration can request fewer capabilities but cannot grant itself broader global/user authority.

## Sandbox modes

### `no_execution`

- no child process starts;
- useful for planning/read-only analysis;
- direct file-reading tools remain controlled by existing application policy.

### `restricted`

- workspace mounted read-only;
- private ephemeral writable temp;
- network denied;
- sanitized environment;
- process/lifecycle limits;
- host credential paths absent because they are not mounted.

### `workspace_write`

- workspace mounted read/write;
- other host paths absent except explicit read-only runtime/toolchain mounts;
- network denied by default;
- sanitized environment;
- process/resource/lifecycle controls remain.

### `full_access`

- separate, explicit user/global selection;
- never selected by model-generated arguments or project settings;
- may execute directly on the host;
- capability report explicitly says filesystem/network isolation are false.

Permission modes remain independent. Existing `--sandbox` permission alias must be deprecated/renamed in UX before the product claims it controls this subsystem.

## Filesystem policy

Mount policy is an allowlist. The sandbox root is assembled from explicit mounts; it is never based solely on string-prefix checks.

Mount forms:

- `ReadOnly { source, target }`
- `ReadWrite { source, target }`
- `Temporary { target, size_limit }`
- `Hidden { target }` for backends that start from a broader root; an empty-root backend simply omits the path.

Before policy crosses the boundary:

- workspace and host mount sources are canonicalized;
- relative targets reject `..`, NUL and platform-invalid forms;
- Windows drive-relative/UNC/ADS and reparse/junction behavior use existing hardened path utilities;
- symlink/reparse escape checks reuse `permission.rs` and `runtime/contracts.rs`;
- writable mount sources may not be nested under a protected mount;
- backend mount construction must operate on canonical sources and preserve mount boundaries.

Default restricted/workspace-write policy does not mount `$HOME`, `~/.ssh`, `~/.aws`, browser profiles, Docker sockets, cloud config, or arbitrary `~/.config`.

Runtime/toolchain mounts are explicit read-only mounts discovered by the trusted host. A backend must not solve tool availability by bind-mounting the entire host root.

## Network policy

Network is a first-class required capability.

- `Denied`: backend must create a network-isolated environment or refuse to run.
- `AllowList`: requires an enforcement backend that controls DNS resolution and post-resolution egress/redirect destinations. Merely validating the original hostname is insufficient.
- `Unrestricted`: explicit policy only.

Denied mode covers IPv4, IPv6, DNS, localhost, private ranges, link-local, metadata endpoints and inherited Unix sockets to the extent the backend namespace/mount design can enforce them.

Initial Linux bubblewrap and container backends support `Denied` and `Unrestricted`. They report domain allowlisting unavailable until a real egress proxy/firewall implementation exists; mandatory allowlist policy therefore fails closed.

## Environment and secret policy

The sandbox starts from `env_clear()`.

A small baseline may be injected by the trusted broker (platform/runtime variables such as `PATH`, `SystemRoot`, locale and the sandbox temp/home paths). All additional values require explicit allowlist/injection policy.

The default must not copy the parent environment. Current foreground shell behavior that copies `std::env::vars_os()` is removed.

Secret injection is represented separately from ordinary environment values. Long-lived credentials are not automatically injected. Future brokered credentials can expose short-lived handles/tokens without changing the child-process contract.

Name-based secret matching remains defense-in-depth only, not the security boundary.

## Resource and process policy

Typed policy includes:

- wall timeout;
- maximum stdout/stderr bytes;
- memory limit;
- CPU time/quota where backend supports it;
- process/PID limit;
- file/output size limit;
- optional disk/temp limit;
- maximum background lifetime.

DaVinci already enforces wall timeout/output bounds in foreground paths and deterministic group/job teardown. Backends advertise which additional hard limits they actually enforce.

A property marked mandatory in `SandboxSpec.required_capabilities` must cause `CapabilityUnavailable` rather than degrade.

## Process lifecycle

`SandboxHandle` is owned by a session/task/worker identity and follows:

`Created -> Ready -> Running -> Stopping -> Stopped`, with `Failed` terminal on unrecoverable setup/runtime errors.

Existing supervisor process ownership remains the mechanical lifecycle primitive:

- Unix helper owns a fresh session/process group before child spawn.
- Windows helper is placed in a kill-on-close Job Object before child spawn.
- helper stdin is the parent lifeline;
- cancellation/timeout requests terminate the owned tree;
- session shutdown terminates all owned sandboxes;
- background jobs carry an explicit maximum lifetime and owner.

No PID-only kill API is treated as proof of ownership.

## Authenticated typed boundary

The current supervisor wire protocol remains private but is upgraded:

- protocol version is explicit;
- broker creates a random per-helper authentication token;
- helper receives it only in its trusted bootstrap environment, which is cleared before requested child launch;
- every configuration/control frame binds to that token and process lifetime;
- unknown frame fields and oversized frames fail closed;
- helper reports selected backend + effective `SandboxCapabilities` before it can claim `Ready`;
- receipt includes sandbox ID/spec digest/backend/capabilities.

This local token prevents accidental/cross-protocol control and binds requests to the helper DaVinci created. It is not a defense against a host attacker that already owns DaVinci's user account.

## Backend interface

Keep the interface small and synchronous to match DaVinci's existing threaded supervisor:

```rust
pub trait SandboxBackend {
    fn kind(&self) -> SandboxBackendKind;
    fn capabilities(&self) -> SandboxCapabilities;
    fn prepare(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
    ) -> Result<PreparedExecution, SandboxFailure>;
}
```

`PreparedExecution` is trusted-executor-only and contains the final executable/argv/cwd/environment plus backend cleanup metadata. It is not model-visible.

### Linux bubblewrap backend

When a trusted `bwrap` binary is available:

- empty/constructed root;
- explicit read-only runtime/toolchain mounts;
- workspace RO/RW according to mode;
- private tmpfs/temp mount;
- new PID/IPC/UTS/session namespaces;
- denied network via network namespace;
- die-with-parent behavior;
- no host home or engine socket mounts.

The backend reports only tested controls. Domain allowlisting is unavailable.

### Container backend

Optional Docker/Podman backend:

- `--rm`;
- network `none` for denied policy;
- read-only root when possible;
- explicit workspace/runtime mounts;
- tmpfs temp;
- CPU/memory/PID flags when configured and supported;
- explicit environment and working directory;
- deterministic stop/removal;
- explicit image setting controlled by trusted user/global config.

Never mount `/var/run/docker.sock`, a Podman control socket, SSH agent socket, or equivalent host-control endpoint.

### Windows

Existing Job Object behavior remains useful for process-tree ownership. It is not advertised as filesystem/network isolation. Restricted/workspace-write fail closed unless a backend with those capabilities (future restricted token/AppContainer implementation or container runtime) is selected.

### macOS

The partial Seatbelt backend converts typed policy into a deny-default kernel profile through `/usr/bin/sandbox-exec`. Host paths remain at their canonical locations: relocated mounts and unsupported capabilities fail closed. Seatbelt cannot mediate `setsid`/`setpgid`; the current Unix supervisor cannot own detached descendants. Process-tree isolation, deterministic teardown, tree timeouts, and lifetime-private temp are not advertised. Session policies require these properties and cannot launch through Seatbelt; native Auto remains disabled. Required macOS CI fixtures test reduced-capability filesystem/network/environment policies and this ownership gap. Memory/PID/CPU/temp-size budgets, network allowlists and ephemeral roots remain unsupported. See `docs/sandbox.md` for the limits and unfinished Phase 4 acceptance requirements.

## Capability negotiation

`SandboxCapabilities` is factual and granular, including at least:

- filesystem isolation;
- network denied;
- network allowlist;
- environment isolation;
- process-tree isolation;
- PID limit;
- memory limit;
- CPU limit;
- ephemeral temp/root;
- output limit;
- timeout;
- deterministic teardown.

Resolution computes `required - available`. Any missing mandatory capability returns `CapabilityUnavailable` before requested child spawn.

## Scheduler, subagents and teams

Sandbox identity is part of execution scheduling.

Default worker policy:

- read-only subagent: isolated sandbox or shared immutable workspace snapshot;
- writable subagent/worktree: isolated sandbox bound to that worktree;
- deliberately shared task: shared sandbox only when host/task configuration says so.

New delegation policy enum:

- `Inherit`
- `Isolated`
- `SharedReadOnly`
- `SharedWorkspace`

Capability reduction is deterministic:

```text
child_capabilities = parent_capabilities ∩ requested_child_capabilities
```

A child cannot turn restricted execution into full access, enable network the parent lacks, widen writable mounts, inject new secrets, or switch to a weaker backend. Only a separately recorded user/global authorization can widen policy.

## Verification, receipts and Context VM

Build/lint/test commands use the same sandbox/workspace identity as the mutation they verify.

Each execution receipt records:

- sandbox ID;
- sandbox spec digest;
- backend and effective capabilities;
- executable + argv;
- cwd;
- start/end timestamps;
- exit code;
- timeout/cancel/resource-kill status;
- environment variable names/digest, never secret values;
- stdout/stderr content-addressed artifact references/hashes;
- workspace/task/generation/revision identity;
- tool version when known.

Raw logs stay in evidence/artifact storage. Context VM receives bounded semantic summaries such as failing test, error code and affected path. Existing Context VM and immutable evidence contracts remain authoritative.

## LSP policy

Language servers gain explicit execution location:

- `host`: only trusted user/global server configuration whose executable provenance is accepted;
- `sandboxed`: default for project-controlled/project-resolved server;
- `disabled`.

Sandboxed LSP is long-lived and can reuse a sandbox tied to the engineering snapshot/worktree, preserving current shared snapshot/cache optimizations. Its environment allowlist is intersected with sandbox environment policy.

## MCP policy

MCP server definition gains execution location:

- `remote`: HTTP service, control-plane network policy;
- `host`: explicit trusted user/global local server;
- `sandboxed`: default local/project executable;
- `disabled`.

Project configuration cannot self-promote to `host`. MCP tool annotations remain application hints and do not grant sandbox capabilities.

## Browser policy

Browser automation processes execute under a dedicated sandbox spec because they may need network even while shell network is denied. Browser profile/session credentials are never mounted implicitly. Any browser network widening is a separate host policy decision.

## Configuration and trust

DaVinci uses JSON settings. Add a top-level user setting:

```json
{
  "sandbox": {
    "mode": "workspace_write",
    "backend": "auto",
    "network": { "mode": "deny" },
    "resources": {
      "timeoutSeconds": 300,
      "maxMemoryMb": 4096,
      "maxProcesses": 128
    }
  }
}
```

Project settings may request a **narrower** mode/limits only when project trust allows the config file to participate. Effective policy uses protected-wins/minimum-capability semantics; project config cannot request `full_access`, add credential mounts, widen network, or increase user/global resource ceilings.

Default compatibility rollout:

1. unconfigured existing installations report `sandbox: unavailable/not configured` and preserve current behavior only under an explicit compatibility switch during migration;
2. once a capable backend is detected, new/default safe execution uses `workspace_write` + denied network;
3. UI always displays whether execution is actually sandboxed/degraded/unavailable.

No permission preset is relabeled as an OS sandbox.

## CLI/TUI status

Expose status independently from permissions:

```text
Sandbox: workspace_write
Backend: linux_bubblewrap
State: active
Network: denied
Filesystem: workspace rw + runtime ro + temp
Memory: 4 GiB (enforced)
Processes: 128 (enforced)
```

A `/sandbox-status` command is appropriate. `--sandbox` currently aliases permission mode and therefore cannot simultaneously mean OS sandbox without a migration; introduce a distinct flag such as `--execution-sandbox` first, deprecate the misleading alias, then migrate deliberately.

## Error model

Security failures are typed and do not collapse into shell exit codes:

- `SandboxUnavailable`
- `CapabilityUnavailable`
- `PolicyDenied`
- `FilesystemDenied`
- `NetworkDenied`
- `ResourceLimitExceeded`
- `ExecutionTimedOut`
- `SandboxCrashed`
- `ProtocolFailure`
- `CleanupFailure`

Command non-zero exit remains an execution result, distinct from sandbox failure.

## Recovery

Reuse execution recovery rules:

- record exact sandbox failure;
- recreate sandbox only when backend failure is recoverable;
- restore/reuse the exact workspace state/checkpoint required by the task;
- retry only operations whose replay/idempotency policy allows it;
- never blindly retry an unknown side effect;
- retries remain bounded.

## Observability

Extend existing telemetry with non-secret fields:

- sandbox create/start latency;
- selected backend/capabilities;
- execution count/denial count;
- timeout/OOM/resource-kill count;
- filesystem/network denial count when backend exposes it;
- cleanup failures;
- wall time and backend overhead;
- sandbox reuse count.

No raw environment values or credential material is logged.

## Security evaluation

Tests must exercise enforcement, not prompts. Required cases include:

- read `~/.ssh` and `~/.aws`;
- write `../outside-project` and absolute outside path;
- symlink/junction/reparse escape;
- enumerate ambient secret env;
- external/DNS/localhost/private/link-local/metadata network access;
- infinite loop timeout;
- process/fork explosion within a bounded fixture;
- child/background survival after teardown;
- child-agent privilege escalation;
- local MCP privilege escalation;
- LSP/project executable privilege escalation;
- parallel isolated workers.

Every control gets: allowed succeeds, forbidden fails, obvious alternate bypass fails, teardown works, parallel execution works where relevant, cancellation works.

## Performance and reuse

Measure native compatibility execution versus sandboxed execution for trivial command, filesystem-heavy command, compiler, tests, 4 workers and 8 workers.

Safe reuse key includes at least backend, sandbox spec digest, canonical workspace/worktree identity, task/worker sharing policy and credential-injection identity. Unrelated writable workers never share mutable sandbox state merely to reduce startup latency.

Do not publish benchmark numbers until measured.

## Incremental delivery invariants

1. Add typed contracts and inventory before behavior changes.
2. Upgrade supervisor/executor protocol before routing more callers.
3. Environment sanitization is mandatory before restricted modes claim safety.
4. A backend cannot advertise a property until enforcement tests exist.
5. Core shell/build/test/managed-process routes move before LSP/MCP/browser.
6. Delegation and verification receipts carry sandbox identity before claiming agent-team isolation.
7. Cross-platform gaps fail closed; no silent downgrade.
8. Existing permission modes, Context VM, scheduler ordering, sessions and evidence remain behaviorally compatible except where execution is intentionally denied by the new sandbox policy.
