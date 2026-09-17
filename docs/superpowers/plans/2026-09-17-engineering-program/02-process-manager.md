# P2: Persistent Process Manager

Status: design approved by the user on 2026-09-17; implementation and local gates passed; exact-head platform CI pending.
Execution sequence: 2 of 12.
Dependencies: P1 green; existing JobBook, process leases, command policy and RuntimeBus.
Requirements authority: project section 7 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Repeated equivalent dev-server requests reuse one currently authorized supervised process, with bounded output and reliable cleanup.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-agent/src/jobs.rs and runtime/ process ownership helpers
- crates/davinci-agent/src/tools.rs, permission.rs, permission_risk.rs and shell_policy.rs
- crates/davinci-coding-agent/src/{main,extension_host,settings,shutdown,runtime_host}.rs
- crates/davinci-coding-agent/src/native_extensions/graph/{roles,process,worker_hooks}.rs
- crates/davinci-agent/tests/process_manager.rs and coding-agent integration fixtures (new)

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Start a local fixture server through current background-job APIs twice; measure startups, jobs, output bounds, cancellation and descendant cleanup. Record existing Windows and Unix behavior rather than assume it.

## Ordered implementation and RED/GREEN work

1. RED: same-authority equivalent argv requests start once; distinct workspace/environment/authorization does not reuse. Verify start/write/status/stop denial after permission revocation and cross-owner access refusal.

2. Extend JobBook records/leases with stable host owner/session/workspace, executable/argv/cwd, safe environment policy digest, PID plus OS lifetime identity, timestamps/state/exit, output cursor, restart policy and known ports. Preserve bash job/stdin compatibility.

3. Add process_start/status/output/write/stop/list native or builtin adapters using existing registry, not a second process table. Resolve a direct executable and argv; on Windows resolve package-manager JS shims through a trusted Node path where necessary without concatenating shell commands.

4. Extend current command policy with argv-aware analysis so quoting cannot create an authorization gap. Use env_clear plus approved platform/runtime variables and explicitly authorized overrides; never automatically inherit secret variables.

5. Reserve a startup key under a short lock, spawn outside global locks, publish a lease or failure, and wake bounded waiters. Default session ownership; workspace reuse only within explicit parent-authorized leases. No indefinite detached persistent mode by default.

6. Use OS-owned descendant cleanup (Windows Job Object, Unix group/session and parent-loss reconciliation) with identity checks. Reconcile process exit/crash; default no restart, optional bounded attempts/backoff. Stop/cancel/shutdown must reap descendants and close pipes.

7. Retain bounded ring output with cursors/truncation and existing retrieve_output overflow evidence. Bound stdin and output reads; do not hold JobBook/global/native host locks while blocking on I/O.

8. Let authorized Graph workers request parent-owned equivalent server leases through existing worker transport patterns. A worker exit releases its lease without terminating another live owner; final owner/session shutdown cleans resources.

9. GREEN: lifecycle, security, concurrency and native normal-session tests, affected package gates, startup/reuse eval and cross-platform exact-head CI; document cleanup/reconciliation limits.

## Required targeted validation

- Long-lived process survives individual calls; stdin round trip, output overflow, nonzero exit, malformed argv/cwd/environment, port provenance and interrupted startup.
- Child/grandchild exit and abrupt parent termination fixtures on Windows/Linux/macOS; reused numeric PID never causes unrelated kill.
- Two normal calls and multiple Graph consumers share only identical authorized resources. Permission modes and tool restrictions remain effective.
- No browser/server starts merely from tool schema/status discovery; shutdown and repeated stop are bounded/idempotent.

## Settings and fallback

processManager.enabled gates the new start/reuse interface. Existing authorized bash jobs retain behavior. Maximum processes/output/restarts are bounded implementation defaults with only critical host configuration exposed.

## Specific acceptance guard

A direct child disappearing is insufficient proof: descendant cleanup, held pipes and host-loss ownership require platform evidence.

## Completion gate and handoff

The approved [program design](../../specs/2026-09-17-engineering-program.md)
and original [requirements](../../specs/2026-09-17-engineering-program-requirements.md)
are the reference. Shared authorization, bounded-output, cache, lifecycle, telemetry,
settings and security invariants apply in addition to the project-specific steps.

Record all fourteen section-34 gates in this project's ledger row: design approval,
plan, observed RED/GREEN, affected package tests, fmt, Clippy, integration, security,
normal path, applicable Graph path, benchmark/eval, docs, diff review, and exact-head
CI. Planned fixtures/test names above are not claims of existing or executed tests.

Validation sequence: focused failing acceptance test; focused passing checks after
implementation; affected crate tests with `--offline --locked`; `cargo fmt --check`;
affected Clippy during development and required workspace Clippy at the milestone;
matching integration/security/eval cases; feature-branch push and actual CI monitoring.
Run broader suites only for affected shared contracts or final acceptance. Record
exact executed selectors, exits and test counts; zero-case runs do not count.

Use a separate coherent commit/PR for the subsystem. With earlier PRs unmerged,
stack on the preceding verified branch and target that branch for review. Preserve
dependency commits without force pushes. Do not begin the next subsystem with
known failures or incomplete required gates. Approval to implement this program
does not authorize merging new PRs into main.

The handoff in README records files/APIs changed, validated commands and results,
metric/artifact paths, head SHA/CI URLs, limitations, and the next dependency input.
No production capability is called done from this plan alone.
