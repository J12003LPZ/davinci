# Execution sandbox

DaVinci separates **approval policy** from **operating-system execution isolation**.

```text
TRUSTED CONTROL PLANE
Agent / Context VM / Memory / Scheduler / Permissions / Task Contracts
                              |
                              v
                        Sandbox Broker
                              |
              typed protocol over a private pipe
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
    "environment": {
      "allow": ["NPM_TOKEN"]
    },
    "resources": {
      "timeoutSeconds": 300,
      "maxMemoryMb": 4096,
      "maxProcesses": 128
    }
  }
}
```

`environment.allow` names host variables passed through to sandboxed processes; nothing else from the host environment is inherited. `timeoutSeconds` is optional: without it a command's own tool-call timeout governs, as it does without a sandbox. With it, it is a ceiling for every command.

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

- workspace read/write, except `.git`, `.davinci` and `.pi`, which stay read-only (see [Workspace control paths](#workspace-control-paths));
- host filesystem outside explicit mounts unavailable when the selected backend enforces filesystem isolation;
- network denied by default;
- sanitized environment;
- process ownership, timeout and output controls remain enabled;
- configured hard memory/PID limits are mandatory and fail closed if the backend cannot enforce them.

### Workspace control paths

The host acts on some workspace files outside the sandbox: Davinci runs `git status`, `git ls-files` and `git diff` in the workspace, the user commits from it, and project configuration defines hooks, MCP servers and extensions. If sandboxed code could write `.git/config` (`core.fsmonitor`, filters, hooks) or `.davinci/`, those commands would run on the host. So in `workspace_write`, existing `.git`, `.davinci` and `.pi` entries are bound read-only on top of the writable workspace at every launch. A repository an earlier command created is protected from then on.

Consequences and limits:

- `git commit`, `git add` and other commands that write `.git` fail inside the sandbox; run them from the host.
- A single command that creates `.git` where none existed can still write its config in that same command, before the next launch protects it.
- A symlinked `.git` cannot be protected by a bind mount and is left as is.

### `full_access`

Explicit host escape hatch. It does **not** claim filesystem or network isolation and is never selected from model tool arguments or trusted project settings alone.

## Backend capability truth

### Linux bubblewrap

The current native isolated backend is bubblewrap when a trusted host `bwrap` executable is available outside the workspace.

It prepares:

- a constructed mount namespace;
- explicit read-only runtime/toolchain mounts. System paths that are symlinks on the host (`/bin`, `/lib`, `/lib64` on usr-merged distributions, a systemd-managed `/etc/resolv.conf`) are also mounted at their own spelling, since every dynamic executable names `/lib64/ld-linux-*.so` as its interpreter. `/etc/alternatives`, name-resolution files and CA stores are included;
- Rust toolchains: `RUSTUP_HOME` points at the read-only host rustup directory, and `CARGO_HOME` is a writable directory in the ephemeral home with the host's downloaded registry and git checkouts mounted read-only, so offline builds of locked projects work;
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

Experimental. With `backend: "container"` (or `auto` without bubblewrap) and a user-configured `container.image`, commands run through Docker or Podman with a read-only root, `--network none` for denied networking, and `--memory`/`--pids-limit`/ulimits. Each launch gets its own container name, so concurrent launches under one policy never collide or tear each other down. The same host runtime mounts as bubblewrap are bound into the container, which replaces the image's own `/usr`; treat this backend as unfinished.

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
- helper stdin is a parent lifeline and the private control channel: only the parent holds it, so the channel itself authenticates the frames;
- cancellation/timeout terminates the owned process lifetime.

Background commands (`bash`/`powershell` with `background: true`) run through the same supervisor with the sandboxed configuration and are registered as supervised jobs; `job_output`, `job_kill` and stdin work as usual. Their lifetime is bounded to 30 minutes outside `full_access`.

Output beyond the output ceiling (4 MiB) is dropped, not fatal: the command keeps running, and its capture is reported incomplete.

### Resource limits

On Unix, `maxMemoryMb` sets `RLIMIT_DATA` (not `RLIMIT_AS`, under which Node, the JVM and Go fail while reserving address space), and CPU time and file size use `RLIMIT_CPU`/`RLIMIT_FSIZE`. These apply **per process**, not to the process tree. The container backend enforces its limits inside the container instead.

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

Without an execution sandbox (the default), local MCP servers from every source run on the host as before; only a server that explicitly asks for `"execution": "sandboxed"` is refused, never silently run unsandboxed.

When execution sandboxing is active:

- project/plugin local MCP commands always run sandboxed, through the process supervisor and the active sandbox backend, as session-long services: the per-command output ceiling and background lifetime do not apply to them (their own 16 MiB line limit does);
- a sandboxed server's `env` values are passed as written; `${NAME}` references resolve only for host variables in `sandbox.environment.allow`, and a reference to any other variable refuses the server rather than leaking or silently dropping it;
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
- hard memory/PID/CPU/disk controls are capability-gated but not all available in the native bubblewrap backend, and its memory/CPU limits are per-process rlimits;
- LSP, extension/hook, graph, package-manager and browser specialized transports are fail-closed rather than fully migrated;
- domain network allowlisting is intentionally unsupported;
- benchmark numbers are not published until they can be measured on a capable runner.

These gaps must not be hidden by documentation, status output or permission labels.
