# Security Review: pi-rust

## Scope

Whole current pi-rust working tree, standard offline source review.

- Scan mode: repository
- Target kind: git_worktree
- Target ID: target_sha256_1ec17e996dcf2225648f17fe6b5d3f9f0f9d84f2ac7bccbebe7430dc74767c4f
- Revision: 1dedd6536ce7fc1e8db40bedbcd1959e26675492
- Snapshot digest: codex-security-snapshot/v1:sha256:ce238497a921406e6d0375c5d7567bf2ffc396c5f80fcc0bfb18e71bbad62336
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none
- Runtime or test status: Static source inspection plus local Rust tests, Clippy, and locked dependency graph inspection; no external services exercised.
- Artifacts reviewed: README.md, Cargo.toml, Cargo.lock, crates/, vendor/pi/SECURITY.md
- Scan context: Find bugs and security issues. Do not fix them. Produce the Codex Security report.md only.

Limitations and exclusions:
- The repository contains 1,874 inventoried files; review prioritized the native Rust product and treated vendor/pi as behavioral and policy reference.
- Independent focused-review workers became unavailable at the account usage limit, so their review receipts could not be recovered.
- Rust dependency advisory scanners were not installed and were not added during this read-only audit.
- The single full-workspace test failure was intermittent and was not promoted to a finding without a reproducible root cause.
- Excluded local writable state and user credentials: Vendor security policy treats exposed third-party or user credentials and attacks requiring local writable state as out of scope.
- Excluded user-installed extensions, skills, untrusted repositories, and approved packages: Vendor security policy excludes these trusted-local or user-approved execution cases.
- Excluded services exposed directly to the public internet: Vendor security policy excludes public internet exposure; static review did not assume such deployment.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 2 |
| Severity mix | medium: 1, low: 1 |
| Confidence mix | high: 2 |
| Coverage | partial |
| Validation mode | Source-backed parent validation after architecture review and four locally completed focused review packets. |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

Pi Rust is a local, unsandboxed terminal coding agent. Sensitive boundaries include provider credentials and requests, workspace tools, session/config storage, trusted project resources, JavaScript extensions, MCP transports, and native graph/memory/security extensions.

### Assets

- Provider credentials and OAuth tokens
- Conversation and session transcripts
- Workspace files and shell authority
- User/project trust and configuration state
- Provider request payloads
- Extension and MCP authority

### Trust Boundaries

- Local user and operating-system boundary
- Trusted versus untrusted project resources
- Model provider and network boundary
- Tool permission boundary
- JavaScript extension subprocess boundary
- MCP stdio and HTTP boundary

### Attacker Capabilities

- Influence provider-controlled responses
- Influence configured MCP or extension outputs when Pi grants authority
- Supply inputs to exposed RPC, URL, parser, file, and package interfaces

### Security Objectives

- Do not disclose credentials beyond intended recipients
- Do not consume project-local control files before trust
- Enforce permission policy before side effects
- Contain paths, protocols, and artifacts to their intended roots
- Report tool side effects and replay semantics accurately

### Assumptions

- No root SECURITY.md applies; vendor/pi/SECURITY.md governs vendor/pi only.
- Permission modes are in-process product gates, not an OS sandbox.
- Static inspection does not establish runtime deployment exposure.

## Findings

| Finding | Severity | Confidence | Detailed write-up |
| --- | --- | --- | --- |
| [Tool ledger conflates distinct calls and can execute concurrent duplicates twice](#finding-1) | medium | high | inline below |
| [write_stdin reports success without writing to the process](#finding-2) | low | high | inline below |

### Confidence Scale

| Label | Meaning |
| --- | --- |
| high | Direct evidence supports the finding with no material unresolved blocker. |
| medium | Evidence supports a plausible issue, but material runtime or reachability proof remains. |
| low | Evidence is incomplete and the item is retained only for explicit follow-up. |

<a id="finding-1"></a>

### [1] Tool ledger conflates distinct calls and can execute concurrent duplicates twice

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | The lookup key, batch preparation order, and delayed record_start are explicit in source; stored tool_name and normalized_arguments are not consulted on replay. |
| Category | exactly-once-execution-integrity |
| CWE | none |
| Affected lines | crates/pi-agent/src/tool_ledger.rs:93-115, crates/pi-agent/src/turn.rs:528-550, crates/pi-agent/src/turn.rs:610-620, crates/pi-agent/src/turn.rs:684-694 |

#### Summary

The exactly-once ledger indexes completed results only by call_id and returns cached output without checking the recorded tool name or normalized arguments. In a batch, every call is prepared before any execution records a start, so duplicate IDs in the same assistant message can both pass the replay check and perform side effects. Across retries or malformed provider output, an ID reused for different content can instead receive a stale result from the earlier call.

#### Root Cause

Exactly-once identity is defined as call_id alone, and the ledger check and reservation are separated across the prepare and execute stages. The implementation therefore neither detects content collisions nor atomically excludes an in-flight duplicate.

**Completed lookup uses call_id alone** — `crates/pi-agent/src/tool_ledger.rs:93-115`

Although the record stores tool_name and normalized_arguments, replay lookup accepts only call_id and returns output without comparing call identity.

```rust
pub fn get_completed_result(&self, call_id: &str) -> Option<(String, bool)> {
    let rec = self.records.get(call_id)?;
    if rec.status == ToolExecutionStatus::Completed {
        rec.output.clone().map(|out| (out, rec.is_error))
    } else {
        None
    }
}

pub fn record_start(&mut self, call_id: &str, tool_name: &str, arguments: &Value) {
    let entry = self
        .records
        .entry(call_id.to_string())
        .or_insert_with(|| ToolCallRecord {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            normalized_arguments: normalize_arguments(arguments),
```

**Replay bypasses tool and argument identity** — `crates/pi-agent/src/turn.rs:610-620`

A reused ID returns prior output before hooks, permission checking, or comparison with the current tool name and arguments.

```rust
if let Some((cached, is_error)) = self
    .tool_ledger
    .lock()
    .ok()
    .and_then(|l| l.get_completed_result(id))
{
    return Preparation::Immediate(crate::ToolResult {
        content: cached,
        is_error,
        details: Some(serde_json::json!({ "replayed_from_ledger": true })),
    });
}
```

**All calls are prepared before execution begins** — `crates/pi-agent/src/turn.rs:528-550`

Every duplicate ID is checked and captured into a scheduled call before any scheduled closure invokes record_start.

```rust
let mut scheduled = Vec::with_capacity(width);
for (id, name, args) in &tool_calls {
    if agent.abort_requested() {
        break;
    }
    let preparation = agent.prepare_tool_call(cwd, id, name, args, 0);
    let lane = match &preparation {
        Preparation::Ready { lane } => *lane,
        Preparation::Immediate(_) => crate::scheduler::ToolLane::Parallel,
    };
    let (id, name, args) = (id.clone(), name.clone(), args.clone());
    scheduled.push(crate::scheduler::ScheduledCall {
        lane,
        run: Box::new(move || {
            let result = match preparation {
                Preparation::Immediate(result) => result,
                Preparation::Ready { .. } => {
                    agent.run_prepared_call(cwd, &id, &name, &args, 0)
                }
            };
            agent.finalize_tool_call(cwd, &id, &name, &args, result)
        }),
    });
}
```

**Reservation occurs only inside execution** — `crates/pi-agent/src/turn.rs:684-694`

The ledger is updated after batch preparation, immediately before tool execution, so it is not an atomic reservation gate.

```rust
if name == "batch" && depth == 0 {
    return self.run_batch(cwd, id, args);
}
if let Ok(mut ledger) = self.tool_ledger.lock() {
    ledger.record_start(id, name, args);
}
// The tool sees the turn's abort flag so a long shell command
// or a `job_output` wait ends when the user interrupts.
let mut context = self.tool_context.clone();
context.abort = self.abort_signal.clone();
let outcome = match execute_tool_with(cwd, name, args, &context) {
```

#### Validation

Confirmed: different content sharing a completed ID is replayed from cache, and same-batch duplicate IDs are all scheduled before the first ledger start.

Validation method: Direct static trace of ToolCallLedger lookup/record semantics and execute_tool_batch scheduling; focused existing ledger tests were executed.

- **Status:** confirmed
- **Disposition:** reportable

Assertions:
- get_completed_result receives only call_id.
- Replay does not compare tool_name or normalized_arguments.
- Batch preparation completes before scheduled calls record a start.
- Existing ledger tests cover only retrying the same call identity, not collisions or concurrent duplicates.

Counterevidence and remaining uncertainty:
- The ledger stores tool_name and normalized_arguments, but those fields are not used as a replay invariant.

Limitations:
- No new collision/concurrency regression test was added because the user requested a report-only audit.

#### Dataflow

Provider-supplied id, tool name, and arguments enter batch preparation; only id is queried for replay; multiple ready calls are scheduled; each records start only when its closure runs; the requested tool then performs its side effect.

- **Source:** Assistant tool-call ID, tool name, and arguments

- **Sink:** Cached ToolResult or execute_tool_with side effect

- **Outcome:** Wrong result is attributed to a call, or a mutating operation executes more than once.

Transformations:
- Batch preparation clones every call before execution.
- Ledger lookup discards tool name and normalized argument identity.

**Completed lookup uses call_id alone** — `crates/pi-agent/src/tool_ledger.rs:93-115`

Although the record stores tool_name and normalized_arguments, replay lookup accepts only call_id and returns output without comparing call identity.

```rust
pub fn get_completed_result(&self, call_id: &str) -> Option<(String, bool)> {
    let rec = self.records.get(call_id)?;
    if rec.status == ToolExecutionStatus::Completed {
        rec.output.clone().map(|out| (out, rec.is_error))
    } else {
        None
    }
}

pub fn record_start(&mut self, call_id: &str, tool_name: &str, arguments: &Value) {
    let entry = self
        .records
        .entry(call_id.to_string())
        .or_insert_with(|| ToolCallRecord {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            normalized_arguments: normalize_arguments(arguments),
```

**All calls are prepared before execution begins** — `crates/pi-agent/src/turn.rs:528-550`

Every duplicate ID is checked and captured into a scheduled call before any scheduled closure invokes record_start.

```rust
let mut scheduled = Vec::with_capacity(width);
for (id, name, args) in &tool_calls {
    if agent.abort_requested() {
        break;
    }
    let preparation = agent.prepare_tool_call(cwd, id, name, args, 0);
    let lane = match &preparation {
        Preparation::Ready { lane } => *lane,
        Preparation::Immediate(_) => crate::scheduler::ToolLane::Parallel,
    };
    let (id, name, args) = (id.clone(), name.clone(), args.clone());
    scheduled.push(crate::scheduler::ScheduledCall {
        lane,
        run: Box::new(move || {
            let result = match preparation {
                Preparation::Immediate(result) => result,
                Preparation::Ready { .. } => {
                    agent.run_prepared_call(cwd, &id, &name, &args, 0)
                }
            };
            agent.finalize_tool_call(cwd, &id, &name, &args, result)
        }),
    });
}
```

**Replay bypasses tool and argument identity** — `crates/pi-agent/src/turn.rs:610-620`

A reused ID returns prior output before hooks, permission checking, or comparison with the current tool name and arguments.

```rust
if let Some((cached, is_error)) = self
    .tool_ledger
    .lock()
    .ok()
    .and_then(|l| l.get_completed_result(id))
{
    return Preparation::Immediate(crate::ToolResult {
        content: cached,
        is_error,
        details: Some(serde_json::json!({ "replayed_from_ledger": true })),
    });
}
```

**Reservation occurs only inside execution** — `crates/pi-agent/src/turn.rs:684-694`

The ledger is updated after batch preparation, immediately before tool execution, so it is not an atomic reservation gate.

```rust
if name == "batch" && depth == 0 {
    return self.run_batch(cwd, id, args);
}
if let Ok(mut ledger) = self.tool_ledger.lock() {
    ledger.record_start(id, name, args);
}
// The tool sees the turn's abort flag so a long shell command
// or a `job_output` wait ends when the user interrupts.
let mut context = self.tool_context.clone();
context.abort = self.abort_signal.clone();
let outcome = match execute_tool_with(cwd, name, args, &context) {
```

#### Reachability

Reachable when a provider retry, recovery stream, adapter, or malformed assistant message reuses a call ID.

- **Attacker:** No attacker is required; a faulty or adversarial provider-side call producer can supply the collision.

- **Entry point:** AssistantMessage tool_calls consumed by execute_tool_batch

- **Source:** Duplicate or reused call_id

- **Sink:** Cached output replay or execute_tool_with

- **Outcome:** Stale result substitution, suppressed intended action, or duplicate shell/file side effect.

Preconditions:
- Two tool calls share a call_id within the ledger lineage.
- For stale replay, the earlier call completed; for duplicate execution, both calls are prepared before either reserves the ID.

Existing controls:
- Tool permissions still constrain each call's nominal authority, but they do not enforce exactly-once identity or prevent duplicated authorized effects.

Limitations:
- The review did not exercise a live provider that emitted duplicate IDs.

**Replay bypasses tool and argument identity** — `crates/pi-agent/src/turn.rs:610-620`

A reused ID returns prior output before hooks, permission checking, or comparison with the current tool name and arguments.

```rust
if let Some((cached, is_error)) = self
    .tool_ledger
    .lock()
    .ok()
    .and_then(|l| l.get_completed_result(id))
{
    return Preparation::Immediate(crate::ToolResult {
        content: cached,
        is_error,
        details: Some(serde_json::json!({ "replayed_from_ledger": true })),
    });
}
```

**All calls are prepared before execution begins** — `crates/pi-agent/src/turn.rs:528-550`

Every duplicate ID is checked and captured into a scheduled call before any scheduled closure invokes record_start.

```rust
let mut scheduled = Vec::with_capacity(width);
for (id, name, args) in &tool_calls {
    if agent.abort_requested() {
        break;
    }
    let preparation = agent.prepare_tool_call(cwd, id, name, args, 0);
    let lane = match &preparation {
        Preparation::Ready { lane } => *lane,
        Preparation::Immediate(_) => crate::scheduler::ToolLane::Parallel,
    };
    let (id, name, args) = (id.clone(), name.clone(), args.clone());
    scheduled.push(crate::scheduler::ScheduledCall {
        lane,
        run: Box::new(move || {
            let result = match preparation {
                Preparation::Immediate(result) => result,
                Preparation::Ready { .. } => {
                    agent.run_prepared_call(cwd, &id, &name, &args, 0)
                }
            };
            agent.finalize_tool_call(cwd, &id, &name, &args, result)
        }),
    });
}
```

**Reservation occurs only inside execution** — `crates/pi-agent/src/turn.rs:684-694`

The ledger is updated after batch preparation, immediately before tool execution, so it is not an atomic reservation gate.

```rust
if name == "batch" && depth == 0 {
    return self.run_batch(cwd, id, args);
}
if let Ok(mut ledger) = self.tool_ledger.lock() {
    ledger.record_start(id, name, args);
}
// The tool sees the turn's abort flag so a long shell command
// or a `job_output` wait ends when the user interrupts.
let mut context = self.tool_context.clone();
context.abort = self.abort_signal.clone();
let outcome = match execute_tool_with(cwd, name, args, &context) {
```

#### Severity

**Medium** — A duplicate or reused provider call ID can suppress an intended operation, replay an unrelated result, or execute a mutating shell/file operation twice. The trigger is constrained to abnormal provider/recovery output and does not independently expand the local agent's configured authority.

Severity would increase where provider output or RPC callers are attacker-controlled and mutating tools operate on high-value repositories, credentials, or deployment systems.

Impact assessment:
- **Level:** medium
- **Rationale:** Mutating tools can run twice or receive an unrelated cached result, corrupting workspace state or misleading subsequent automation.

Likelihood assessment:
- **Level:** low
- **Rationale:** Well-behaved providers normally generate unique call IDs; the defect manifests on retry/collision or malformed output.

#### Remediation

Define tool-call identity over the execution lineage, call_id, tool name, and canonical argument digest; reject a reused ID whose content differs; atomically reserve an unseen identity before scheduling; make later same-identity calls wait for or replay the first result; and preserve terminal results without allowing record_start to overwrite them.

Tests:
- Reuse a completed call ID with a different tool name and assert a collision error, not cached output.
- Reuse a completed call ID with different canonical arguments and assert a collision error.
- Submit two same-ID mutating calls in one batch and assert the side effect occurs exactly once.
- Submit concurrent identical calls and assert followers receive the first terminal result.

Preventive controls:
- Treat provider identifiers as untrusted routing metadata and bind them to canonical call content.
- Use an atomic pending/completed state transition as the scheduling gate for side-effecting operations.

<a id="finding-2"></a>

### [2] write_stdin reports success without writing to the process

| Field | Value |
| --- | --- |
| Severity | low |
| Confidence | high |
| Confidence rationale | Direct source inspection shows the handler never performs I/O, while both supported command transports leave no writable stdin handle for later use. |
| Category | tool-result-integrity |
| CWE | none |
| Affected lines | crates/pi-agent/src/tools.rs:356-386, crates/pi-agent/src/tools.rs:789-825, crates/pi-agent/src/jobs.rs:263-279 |

#### Summary

The write_stdin tool checks only whether an optional job exists and is running, then returns a successful byte count without opening, writing, or flushing the child process stdin. Background commands either start with null stdin or consume and drop the one piped stdin handle while sending the initial command, so a valid invocation delivers zero bytes while callers are told it succeeded.

#### Root Cause

The write_stdin API was exposed without a data path to a retained ChildStdin handle, and success is synthesized from requested input length instead of an I/O result.

**The spawn path closes or nulls stdin** — `crates/pi-agent/src/tools.rs:800-825`

Argv transport explicitly uses null stdin; Stdin transport takes the only handle to send the initial command and drops it before the child is registered.

```rust
match config.command_transport {
    pi_ai::CommandTransport::Argv => {
        process.arg(command).stdin(std::process::Stdio::null());
    }
    pi_ai::CommandTransport::Stdin => {
        process.stdin(std::process::Stdio::piped());
    }
}
let mut child = process
    .spawn()
    .map_err(|err| ToolError::Failed(err.to_string()))?;
if matches!(config.command_transport, pi_ai::CommandTransport::Stdin) {
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(command.as_bytes())
            .map_err(|err| ToolError::Failed(err.to_string()))?;
    }
}
Ok(child)
```

**The handler returns success without writing** — `crates/pi-agent/src/tools.rs:356-386`

The caller-controlled text is counted, but no ChildStdin is acquired and no write or flush occurs before success is returned.

```rust
fn write_stdin_tool(
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let text = input
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(id) = input.get("job_id").and_then(Value::as_u64) {
        let jobs = context.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = jobs.get(id as u32) {
            if !job.status().is_running() {
                return Ok(ToolResult {
                    content: format!("Job {id} has already exited."),
                    is_error: true,
                    details: None,
                });
            }
        } else {
            return Ok(ToolResult {
                content: format!("Job {id} not found."),
                is_error: true,
                details: None,
            });
        }
    }
    Ok(ToolResult {
        content: format!("Sent {} bytes to stdin", text.len()),
        is_error: false,
        details: None,
    })
}
```

#### Validation

Confirmed: every successful return from write_stdin is independent of an actual process write.

Validation method: Direct static trace of tool dispatch, handler, shell spawning, and background job storage; repository search for write_stdin tests.

- **Status:** confirmed
- **Disposition:** reportable

Assertions:
- A valid running job can reach the success response.
- The success response reports text.len() without a write operation.
- No retained stdin handle is exposed through JobBook for this handler.

Counterevidence and remaining uncertainty:
- The handler correctly rejects a missing or already-exited job when job_id is supplied, but that check does not deliver input.

Limitations:
- No regression test was added because the user requested a report-only audit.

#### Dataflow

Tool input text and optional job_id enter write_stdin_tool; the handler checks job status, counts the requested bytes, and returns success without an I/O transition.

- **Source:** write_stdin tool arguments

- **Sink:** ToolResult claiming bytes were sent

- **Outcome:** The caller proceeds while the target process receives no data.

**The handler returns success without writing** — `crates/pi-agent/src/tools.rs:356-386`

The caller-controlled text is counted, but no ChildStdin is acquired and no write or flush occurs before success is returned.

```rust
fn write_stdin_tool(
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let text = input
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(id) = input.get("job_id").and_then(Value::as_u64) {
        let jobs = context.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = jobs.get(id as u32) {
            if !job.status().is_running() {
                return Ok(ToolResult {
                    content: format!("Job {id} has already exited."),
                    is_error: true,
                    details: None,
                });
            }
        } else {
            return Ok(ToolResult {
                content: format!("Job {id} not found."),
                is_error: true,
                details: None,
            });
        }
    }
    Ok(ToolResult {
        content: format!("Sent {} bytes to stdin", text.len()),
        is_error: false,
        details: None,
    })
}
```

#### Reachability

Directly reachable whenever a background job expects follow-up stdin.

- **Attacker:** No attacker is required; an ordinary model or API caller triggers the defect.

- **Entry point:** write_stdin tool dispatch

- **Source:** input and job_id

- **Sink:** successful ToolResult

- **Outcome:** Interactive command stalls, times out, or behaves differently while orchestration records success.

Preconditions:
- A background process is running and expects later stdin.

Existing controls:
- Job existence and running-state checks reject some invalid targets but do not verify a write.

**The spawn path closes or nulls stdin** — `crates/pi-agent/src/tools.rs:800-825`

Argv transport explicitly uses null stdin; Stdin transport takes the only handle to send the initial command and drops it before the child is registered.

```rust
match config.command_transport {
    pi_ai::CommandTransport::Argv => {
        process.arg(command).stdin(std::process::Stdio::null());
    }
    pi_ai::CommandTransport::Stdin => {
        process.stdin(std::process::Stdio::piped());
    }
}
let mut child = process
    .spawn()
    .map_err(|err| ToolError::Failed(err.to_string()))?;
if matches!(config.command_transport, pi_ai::CommandTransport::Stdin) {
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(command.as_bytes())
            .map_err(|err| ToolError::Failed(err.to_string()))?;
    }
}
Ok(child)
```

**The handler returns success without writing** — `crates/pi-agent/src/tools.rs:356-386`

The caller-controlled text is counted, but no ChildStdin is acquired and no write or flush occurs before success is returned.

```rust
fn write_stdin_tool(
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let text = input
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(id) = input.get("job_id").and_then(Value::as_u64) {
        let jobs = context.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = jobs.get(id as u32) {
            if !job.status().is_running() {
                return Ok(ToolResult {
                    content: format!("Job {id} has already exited."),
                    is_error: true,
                    details: None,
                });
            }
        } else {
            return Ok(ToolResult {
                content: format!("Job {id} not found."),
                is_error: true,
                details: None,
            });
        }
    }
    Ok(ToolResult {
        content: format!("Sent {} bytes to stdin", text.len()),
        is_error: false,
        details: None,
    })
}
```

#### Severity

**Low** — The defect reliably breaks interactive background-process control and makes tool results untrustworthy, but the reviewed path does not by itself cross the local-agent security boundary or grant additional authority.

Impact would increase if higher-level automation treats this acknowledgement as proof that a privileged or safety-critical process received a control message.

Impact assessment:
- **Level:** low
- **Rationale:** Reliable loss of control input and false execution state, without an established privilege or confidentiality impact.

Likelihood assessment:
- **Level:** high
- **Rationale:** The handler always follows this behavior for a valid running job.

#### Remediation

Retain a piped ChildStdin handle for jobs that support interactive input; require and resolve job_id; write and flush the requested bytes under safe synchronization; return the actual I/O outcome; and explicitly reject jobs whose transport cannot accept stdin.

Tests:
- Start an interactive background process, send input, and assert the process observes the exact bytes.
- Assert write_stdin rejects jobs with null or closed stdin.
- Assert write and flush errors are returned as tool errors rather than success.

Preventive controls:
- Require tool acknowledgements for side effects to be derived from completed I/O.
- Maintain end-to-end tests for every advertised process-control tool.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| Architecture, resource, and trust-boundary mapping | System design | No issue found | Startup, provider, session, trust, tool, extension, MCP, and native-extension resource chains mapped. |
| Network, OAuth, provider transport, and credential storage | Credentials and SSRF | No issue found | Guarded DNS resolution, redirects, callback state checks, provider request construction, and local credential boundaries reviewed; no policy-valid vulnerability established. |
| RPC, server, protocol, session, and MCP | Local service authorization and persistence | No issue found | Local socket permissions, framing limits, session persistence, and MCP transports reviewed; no policy-valid vulnerability established. |
| Extensions, packages, export, and subprocesses | Code execution and content injection | No issue found | Project trust gates, package acquisition boundaries, extension subprocesses, and HTML export escaping reviewed; no policy-valid vulnerability established. |
| Permissions, trust, file tools, graph, memory, and native security scan | Authorization and execution integrity | Reported | Two source-validated correctness and execution-integrity bugs reported; neither independently crosses the stated security boundary. |

## Open Questions And Follow Up

- What caused stream::tests::a_live_stream_delivers_each_frame_before_the_next_is_sent to receive only the first frame in the full workspace run when both isolated reruns passed?
- Would a dedicated Rust advisory database scan identify a vulnerable locked dependency?
- cargo-audit, cargo-deny, osv-scanner, and trivy are unavailable; cargo tree inspected the locked dependency graph but could not evaluate advisories.
  - Follow-up prompt: Review deferred unit dependency-advisories and close its stated proof gap. Paths: Cargo.lock.
- Focused review workers became unavailable at the account usage limit; the parent completed the four review packets locally, so independent reviewer evidence is absent.
  - Follow-up prompt: Review deferred unit independent-review-receipts and close its stated proof gap. Paths: ..
- Vendor TypeScript was used as behavioral and policy reference but not exhaustively compared file-by-file with the native Rust rewrite.
  - Follow-up prompt: Review deferred unit vendor-typescript-parity and close its stated proof gap. Paths: vendor/pi/.
- The workspace test failed once because only the first of two SSE frames arrived; two isolated reruns passed, so root cause remains unproven.
  - Follow-up prompt: Review deferred unit sse-test-flake and close its stated proof gap. Paths: crates/pi-ai/src/stream.rs.
- Candidate awaits final source-evidence validation.
  - Follow-up prompt: Review deferred unit candidate-write-stdin-false-success and close its stated proof gap. Paths: crates/pi-agent/src/tools.rs, crates/pi-agent/src/jobs.rs.
- Candidate awaits final source-evidence validation.
  - Follow-up prompt: Review deferred unit candidate-tool-ledger-id-collision and close its stated proof gap. Paths: crates/pi-agent/src/tool_ledger.rs, crates/pi-agent/src/turn.rs.
- Focused review is still running.
  - Follow-up prompt: Review deferred unit candidate-network-auth and close its stated proof gap. Paths: crates/pi-agent/src/web.rs, crates/pi-ai/src/.
- Focused review is still running.
  - Follow-up prompt: Review deferred unit candidate-rpc-storage and close its stated proof gap. Paths: crates/pi-server/, crates/pi-protocol/, crates/pi-session/, crates/pi-mcp/.
- Focused review is still running.
  - Follow-up prompt: Review deferred unit candidate-extensions-export and close its stated proof gap. Paths: crates/pi-coding-agent/src/export.rs, crates/pi-coding-agent/src/js_host.rs, crates/pi-coding-agent/src/packages.rs.
- Focused review is still running.
  - Follow-up prompt: Review deferred unit candidate-permissions-tools and close its stated proof gap. Paths: crates/pi-agent/src/permission.rs, crates/pi-agent/src/tools.rs, crates/pi-agent/src/apply_patch.rs, crates/pi-coding-agent/src/native_extensions/.
