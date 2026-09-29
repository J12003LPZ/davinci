# Execution sandbox

DaVinci separates **approval policy** from **operating-system execution isolation**.

```text
TRUSTED CONTROL PLANE
Agent / Context VM / Memory / Scheduler / Permissions / Task Contracts
                              |
                              v
                        Sandbox Broker
                              |
                   typed authenticated protocol
==============================|================================
                              v
UNTRUSTED EXECUTION PLANE
shell / builds / tests / package code / local executable services
                              |
                              v
                     backend enforcement
```

The model is not a security boundary. Approving a command does not grant it access to host credentials or paths that the execution sandbox does not expose.

## Configuration

User settings use the existing JSON settings format:

```json
{
  "sandbox": {
    "mode": "workspace_write",
    "backend": "auto",
    "network": {
      "mode": "deny"
    },
    "resources": {
      "timeoutSeconds": 300,
      "maxMemoryMb": 4096,
      "maxProcesses": 128
    }
  }
}
```

The command line can override only the execution mode for the run:

```text
--execution-sandbox none
--execution-sandbox restricted
--execution-sandbox workspace-write
--execution-sandbox full-access
```

The older `--sandbox` option remains a compatibility alias for **permission mode**. It does not select OS isolation.

Project settings are considered only for trusted projects and may narrow, never widen, the user/global sandbox policy.

## Modes

### `none` / `no_execution`

No subprocess execution is permitted by the sandbox policy.

### `restricted`

- workspace read-only;
- explicit runtime/toolchain mounts read-only;
- private temporary storage;
- sanitized environment;
- network denied by default;
- required unsupported capabilities fail closed.

### `workspace_write`

- workspace read/write;
- host filesystem outside explicit mounts unavailable when the selected backend enforces filesystem isolation;
- network denied by default;
- sanitized environment;
- process ownership, timeout and output controls remain enabled;
- configured hard memory/PID limits are mandatory and fail closed if the backend cannot enforce them.

### `full_access`

Explicit host escape hatch. It does **not** claim filesystem or network isolation and is never selected from model tool arguments or trusted project settings alone.

## Backend capability truth

### Linux bubblewrap

The current native isolated backend is bubblewrap when a trusted host `bwrap` executable is available outside the workspace.

It prepares:

- a constructed mount namespace;
- explicit read-only runtime/toolchain mounts;
- workspace RO or RW according to mode;
- private `/tmp`;
- PID/IPC/UTS/session isolation;
- network namespace isolation for denied networking;
- cleared environment followed by explicit variables;
- die-with-parent semantics.

Domain allowlisting is **not** claimed. A requested allowlist fails closed until a backend that securely controls DNS, redirects and post-resolution egress is implemented.

### Host backend

Used only for explicit `full_access`. It retains DaVinci's supervised process-tree lifecycle and bounded output/timeout handling, but reports no filesystem or network isolation.

### Container backend

The typed backend exists in policy, but a container runtime implementation is not yet complete on this branch. Selecting it returns `SandboxUnavailable` rather than silently falling back.

### Windows and macOS

DaVinci does not equate process ownership with filesystem/network isolation.

- Windows retains Job Object process-tree ownership, but restricted/workspace-write must use a backend that can truthfully provide the required filesystem/network capabilities.
- macOS similarly fails closed for required properties that no selected backend can enforce.

No platform silently reports an isolation capability that its backend does not advertise.

## Environment and secrets

Sandboxed foreground execution starts from a cleared/minimal environment instead of copying the parent process environment.

Baseline runtime variables may be forwarded. Additional values require explicit policy injection.

Ambient variables such as tokens, cloud credentials and SSH-agent sockets are not automatically inherited. Secret-name filtering is defense in depth only; the primary rule is an allowlisted environment.

Execution evidence stores environment variable names and a digest, not raw values.

## Filesystem

Filesystem policy is mount-based for isolated backends, not a string-prefix-only check.

DaVinci also retains its existing application-level canonicalization, symlink/reparse checks and task-contract path policy above the sandbox.

Sensitive host locations such as the user's home credential directories are not mounted by default. Runtime mounts are explicit and read-only.

## Network

Network policy is independent from filesystem write authority.

`workspace_write` does not imply network access. The default is denied.

A backend must actually enforce a requested network property. URL pre-checks alone are not treated as secure domain isolation.

## Process lifecycle

The existing supervisor is the execution-plane seed:

- Unix establishes a fresh session/process group before untrusted child spawn.
- Windows establishes a kill-on-close Job Object before child spawn.
- helper stdin is a parent lifeline;
- cancellation/timeout terminates the owned process lifetime;
- sandbox policy and a per-helper authentication token cross the private control channel;
- the requested child never receives that control token.

Background lifetimes remain bounded by sandbox/process policy.

## Evidence and verification

A sandboxed process receipt binds:

- sandbox ID;
- redacted sandbox-spec digest;
- requested/effective backend;
- effective capabilities;
- executable and argv;
- cwd;
- environment references/digest;
- launch/exit state;
- output hashes/artifact references.

Verification only treats sandbox enforcement as established when the executor returned a matching effective sandbox receipt. Model text that says a test passed is not execution evidence.

Use:

```text
/sandbox-status
```

to inspect the configured policy. The status intentionally says enforcement is verified **per execution receipt** rather than claiming that configuration alone means a sandbox is active.

## Delegation

Subagent/workflow workers receive sandbox authority from the trusted parent runtime.

A child may narrow but not widen:

- sandbox mode;
- network access;
- writable mounts;
- environment injection;
- resource ceilings;
- backend authority.

Writable worktree workers receive a sandbox rebound to their own canonical worktree rather than implicitly sharing the parent's mutable workspace.

## Local MCP, LSP, extensions, hooks, packages, graph workers and browser processes

Local MCP has a sandbox-aware transport. Other persistent/specialized transports remain intentionally fail closed while they are migrated.

When execution sandboxing is active:

- project/plugin local MCP commands marked `sandboxed` run through the authenticated process supervisor and active sandbox backend; their server-level environment is intersected with the trusted sandbox environment policy;
- explicit trusted-user MCP commands marked `host` remain a separate escape hatch;
- remote MCP HTTP remains control-plane networking;
- language-server raw host spawn is refused;
- JavaScript extension raw host spawn is refused;
- executable hooks are refused;
- graph-worker raw child spawn is refused;
- package-manager install execution is refused;
- browser raw/supervisor launch without a sandbox-aware transport is refused.

This is a compatibility limitation, not a sandboxing claim. These paths must gain their own executor transport and explicit mounts/network policy before they can run under restricted modes.

Trusted control-plane network activity such as provider requests and explicitly remote MCP HTTP services is separate from process-sandbox networking.

## Errors

Security failures remain distinct from command exit codes. Typed categories include:

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

A non-zero exit from a successfully sandboxed command is an execution result, not a sandbox setup failure.

## Current limitations

This branch does not yet satisfy the entire long-term acceptance target:

- container backend execution is not implemented;
- hard memory/PID/CPU/disk controls are capability-gated but not all available in the native bubblewrap backend;
- LSP, extension/hook, graph, package-manager and browser specialized transports are fail-closed rather than fully migrated;
- domain network allowlisting is intentionally unsupported;
- benchmark numbers are not published until they can be measured on a capable runner.

These gaps must not be hidden by documentation, status output or permission labels.
