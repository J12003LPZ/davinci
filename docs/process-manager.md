# Managed processes

DaVinci can keep an authorized development server alive across tool calls and
reuse it within a session. The process manager extends the existing `JobBook`;
it does not create a second registry or persist live processes in the cache.
Normal interactive sessions and authorized Graph workers use the same service.

## Tools and settings

The CLI enables managed processes by default. Construction and tool discovery
do not start a supervisor or a development server. The tools are deferred until
discovered through the normal capability system.

| Tool | Behavior |
| --- | --- |
| `process_start` | Start or reuse a direct executable with literal `argv`, optional workspace `cwd`, explicit environment overrides, restart policy and declared ports. |
| `process_status` | Read the owned record, lifecycle identity, state, exit, provenance, restart attempts and ports. |
| `process_output` | Read a bounded output page using a byte cursor. |
| `process_write` | Write at most 16 KiB of literal UTF-8 to an active owned process. |
| `process_stop` | Release this owner's lease; the final release requests descendant cleanup. |
| `process_list` | List this owner's retained records and startup/reuse/restart counters. |

Set `processManager.enabled` to `false` in the normal settings file to disable
these tools. Invalid fields in this settings block disable the capability while
preserving unrelated valid settings. Existing authorized background shell jobs
continue to work. If the host cannot construct the manager, it reports the
failure and removes the managed tools from discovery.

`process_start` returning successfully means the OS process was created. It
does not certify application readiness or that a port is listening. Declared
ports are metadata, explicitly returned as unverified. Use application output
and the relevant verification capability to determine readiness.

## Ownership and reuse

Equivalent starts within an explicit ownership scope share one live process.
The key binds the canonical workspace/cwd, resolved executable, exact argv,
effective environment, permission revision and restart/port options. Every
request still passes current permission checks. Separate sessions cannot reuse
each other's processes merely by providing the same numeric ID or command.

The parent can give a Graph worker a child lease. Each worker retains its own
lease and role restrictions; one worker's release does not stop another owner's
server. Final release, session switch, host shutdown or cancellation requests
cleanup. Reloading the same active session retains its scope. A session switch
revokes its old child leases. Session files never reconnect to a stored PID.

Status includes an opaque owner and session-scope identity, the first start's
host-provided runtime session/agent/task/Graph-node provenance, executable/argv,
cwd, PID, lifetime UUID, start timestamp, state and exit information. Provenance
describes the initiating request, not every subsequent lease holder. PID is
diagnostic only: cleanup targets an owned OS lifetime, never a later process
found by looking up an old PID.

Restart is disabled by default. An explicit policy permits at most three retries
after nonzero exit, with a 50–5,000 ms backoff. Successful exit and explicit stop
never retry. Every attempt rechecks the current permission revision and workspace
identity. One-call approval is consumed by the original call and cannot authorize
an automatic retry. Status reports attempts, backoff/restarting state and errors;
each new OS lifetime receives a new UUID. Failed startup ends that restart cycle.

## Permissions and environment

Start and stdin are shell-class actions. Status, output and list are reads;
stop releases a lease. Explicit named denies apply to every operation. Existing
shell deny rules also constrain managed starts, while a shell allow does not
automatically grant a persistent-process operation. One-call approval binds the
exact request and current policy and is rechecked at dispatch/startup.

Graph exposes start/status/output/stop/list only to its Writer and TestAnalyzer
roles through the authenticated parent transport. Their existing command policy
still applies; Graph roles cannot inject arbitrary stdin. Hard task contracts
reject unconfined process execution. This manager controls ownership and
lifecycle; it is not a filesystem or network sandbox.

Executables and arguments are passed separately; no command string is assembled
for a shell. On Windows, known npm/pnpm/yarn shims resolve to an installed Node
executable and an adjacent known JavaScript entry point. Missing or unsupported
shims fail explicitly. Canonical entry-point identity is checked before converting
the Windows path to a form Node can load. No dependency is installed automatically.

Children start with a cleared environment. The inherited allowlist is `PATH`,
`SystemRoot`, `WINDIR`, `TEMP`, `TMP`, `TMPDIR`, `LANG`, `LC_ALL` and `LC_CTYPE`,
when present. Explicit overrides are part of the authorized start and reuse key.
Overrides cannot change executable discovery, platform roots, interpreter injection
variables or DaVinci's internal supervisor variables. Environment values are not
included in status; status exposes variable names and a digest. A launched program
can still read files or print sensitive data allowed by its own OS authority.

## Bounds and output

| Resource | Bound |
| --- | --- |
| Running processes plus startup reservations | 16 per shared `JobBook` |
| Retained managed records | 64 |
| Lease owners per record | 64 |
| Waiters on one startup | 32, with a 10-second deadline |
| Start request | 64 KiB serialized; 128 arguments and 32 KiB combined argv |
| Explicit environment | 32 entries, 8 KiB combined keys/values |
| Declared ports | 8 unique nonzero ports |
| Retained output per job | 4 MiB cap, trimmed toward 3 MiB at a line boundary |
| Output page | 8 KiB by default; maximum 64 KiB raw bytes |
| Stdin call | 16 KiB; 500 ms acknowledgement deadline |

Output cursors count raw bytes. With no cursor, reading starts at the oldest
retained byte. `truncated` is true when the requested cursor precedes retained
output; request cursor zero to detect any discarded prefix. `next_cursor`,
`earliest_cursor`, `total_bytes` and `remaining_bytes` make pagination explicit.
Invalid UTF-8 is replaced for display, so displayed UTF-8 can be larger than the
raw byte page. An oversized single line can be dropped entirely by line trimming.

The existing token governor bounds model-visible tool output and provides
`retrieve_output` references for retained tool results. Those references recover
the saved result page, not bytes already discarded from the process ring. A
stdin timeout can occur after partial delivery; inspect output before retrying.
Stdout and stderr are collected together without a promised cross-stream order.

## Cleanup and platform limits

A private supervisor establishes ownership before starting the requested child.
On Windows it enters an unnamed, non-inheritable Job Object with kill-on-close
and no breakaway flag. On Unix it creates a session/process group. Closing the
host lifeline or losing the final lease terminates that owned group/job, including
ordinary descendants. The host keeps exclusive ownership of the unreaped helper
until cleanup, preventing a reused numeric PID from becoming a cleanup target.

Stop requests are asynchronous and repeatable. Inspect status until termination
when subsequent work depends on resources being gone. Root exit uses a bounded
pipe-drain window; `output_complete: false` reports incomplete output when a
descendant holds pipes or delivery cannot finish in time. Startup, pipe queues,
input acknowledgement and reconciliation are bounded; blocking I/O runs outside
the shared job table lock.

Unix process groups do not confine deliberately escaping programs that create a
new session or move to another group. This is lifecycle supervision of ordinary
development tools, not containment of hostile executables. Abrupt termination of
both the host and supervisor, OS shutdown, and arbitrary escaped descendants are
outside the cleanup guarantees. Native platform tests exercise normal descendants,
held pipes, cancellation, host loss, concurrency and lease release.

## Evidence

The frozen [Windows baseline](superpowers/plans/2026-09-17-engineering-program/evidence/p2-process-baseline.json)
started two jobs and six fixture processes for two equivalent requests. The
[managed sample](superpowers/plans/2026-09-17-engineering-program/evidence/p2-process-after-windows.json)
started one job and three fixture processes; the repeated request reused it in
1.4052 ms. Cold readiness was 190.1967 ms versus 129.4012 ms for the old first
start, including the managed supervisor and descendant readiness. This is a
reuse/ownership improvement, not a claim that cold startup became faster.

The managed sample observed zero surviving fixture listeners after shutdown and
200 ms after abrupt host exit, versus three at that observation point in the
old host-loss baseline. Both fixtures use real loopback Node servers and an
explicit cleanup deadline. Timings are single debug-profile measurements, not
portable performance budgets or provider-token measurements. The
[program ledger](superpowers/plans/2026-09-17-engineering-program/README.md)
records actual validation and exact-head platform CI; pending gates remain open.
