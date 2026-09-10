# Davinci Feature Expansion — Shared Runtime Contracts

**Baseline:** `main@9c42820` · **Contract status:** proposed integration design.

These contracts prevent the fourteen plans from independently inventing incompatible authority, storage, lifecycle, and evidence systems. They are part of every feature plan. Names below are proposed unless explicitly identified as existing; implementation must adapt actual signatures rather than assume a new type already exists.

## 1. Identity and authority

Reuse existing `RunId`, `AgentId`, and `TaskId` in `davinci-agent::runtime::ids`. Add explicit newtype identifiers for request, attempt, decision, checkpoint, evidence, and scope revision where a string could otherwise be confused with another identity. Durable IDs do not themselves authorize an operation.

A command envelope contains schema version, request ID, runtime run/session lineage, target task or graph run, expected revision, and the actor identity supplied by the host. The actor and its capabilities come from the executing `RuntimeHandle`/host transport. The model may request an action and reference a target; it may not fill in an authoritative actor, owner generation, user-authenticated flag, completed evidence receipt, or budget balance.

A mutating task command compares expected revision and current owner generation atomically. Claim, release, handoff, status update, dependency change, completion, and cancellation all use that service. Reusing a request ID with identical canonical input returns the original result; using the same ID with different input is a conflict. A reassigned or restarted worker cannot continue writing by presenting an old task ID.

Control identity includes run, task, worker, attempt, and owner generation. Control requests return requested/acknowledged/terminal receipts, not a Boolean that falsely proves a process exited. A host-only authority path may manage a task on behalf of the user; it remains auditable and cannot be synthesized from an agent message.

## 2. Storage and committed truth

Existing `TaskRegistry` is in-memory state; existing runtime observer logging is not an authoritative transactional store. Introduce a task command coordinator and `TaskStore` whose successful acknowledgment means a validated transaction is durably recorded according to a documented platform contract.

The mutation sequence is: load current revision → construct and validate a candidate → evaluate required decision hooks without holding a storage/state mutex → reacquire/serialize the commit boundary → revalidate revision/owner and required evidence → append durable committed record → update the in-memory projection → publish the committed observation. If a hook changes the world or the expected revision is stale, restart/refuse the candidate, not overwrite the concurrent state. A bounded prepared record may support recovery, but it is not a committed success event.

Do not emit `TaskCompleted` as a replayable committed fact before the completion transaction succeeds. Separate requested and committed event kinds or add an unambiguous commit envelope. Observer subscribers may fail without blocking optional telemetry; mandatory authority and persistence failures propagate. No callback executes while holding a mutex that it could reenter.

Journal records carry version, monotonic store sequence, command ID, previous/new revisions, bounded payload, and integrity metadata. A corrupt final incomplete record can be recovered only under a defined tail policy; interior corruption or unknown authoritative schema fails closed into a visible recovery state. Preserve the original bytes for diagnosis. A snapshot plus journal replay must produce the same state as committed transactions, including results, cancellations, dependencies, owner generations, and consumed resources.

Native temporary-file replacement and flushing semantics must be tested on Windows as well as Unix. Do not claim a rename is a universal multi-file transaction or that `flush()` alone means durable storage. Sessionless mode reports ephemeral persistence explicitly. Rehydration selects the saved run/session lineage deliberately; it cannot create a fresh RunId and then hide all restored tasks behind a current-run filter.

## 3. State separation and migration

Keep the existing task states Pending, Ready, Running, Completed, Failed, Blocked, Cancelled in the core compatibility adapter. The board's presentation may map Ready/Pending to pending and Running to in_progress without discarding why a task is not ready. Add orthogonal reconciliation or lifecycle metadata rather than inventing success after restart. Historical Running work is not still alive just because its record exists.

`LivingPlan` and its Boolean per-step decisions remain separate from execution tasks. Existing `/plan approve` does not change the execution mode; `/plan accept [mode]` is an explicit host-controlled handoff. Question answers do not invoke either operation. Resume preserves legitimate historical answers and step decisions but clears blanket execution approval as the current host does.

Question Decision state is Open, AnsweredByUser, Deferred, or Cancelled, with question/option IDs, source revision, authenticated answer provenance, and optional bounded free text. A stale answer does not overwrite a newer plan. A deferred/cancelled required decision stays unresolved; it is never converted to a recommended answer.

Graph lifecycle is orthogonal to persisted Phase/TaskStatus: running, pause_requested, paused, stop_requested, draining, and terminal/reconciliation states need explicit definitions. Paused means no new work is admitted and the required safe boundary is acknowledged. It does not mean operating-system suspension. Existing bare `/graph` continuation remains compatible except that an explicitly paused run is not silently resumed.

## 4. One effect-admission path

The effective authority is the intersection of explicit denies, project trust, permission mode, tool capability, worker role, parent restrictions, task scope, current ownership, platform containment, and available budget. A UI response can resolve a legal question but cannot remove an intersection term. Always Approve suppresses eligible prompts; it is not a scope expansion or sandbox escape.

`prepare_tool_call` runs existing pre-tool processing, then checks the finalized tool/arguments. `run_prepared_call` revalidates ownership, scope revision, cancellation, and reservation immediately before effects. A prepared grant cannot authorize different arguments or a path redirected since preview. Protect filesystem mutation endpoints again at the actual open/write/rename boundary.

Every inner batch operation uses the same checks and its existing stable `{parent}#{index}` identity. Current batch code calls prepare/run/post_tool and bypasses the normal finalizer; refactor a shared admission/receipt boundary or call it explicitly in both paths with exactly-once guards. An outer batch permit never grants all hidden inner effects. Permission and evidence tests must cover bash, powershell, exec_command, write_stdin, apply_patch, notebooks, MCP/custom tools, and child agents, not only the obvious `write` tool.

Tool ledger replay is retrieval of a previous outcome, not a newly executed effect. It must not create a fresh verification receipt, reset a loop detector's actual-progress state, or charge another executed-tool attempt. Replayed text still obeys read/privacy boundaries.

## 5. Scope and process limits

`ScopeContract` is a revisioned accepted boundary containing canonical project/root identity, permitted file paths/patterns, prohibited paths, allowed effect classes, command/network constraints, verification requirements, and budget linkage. A child contract only narrows its parent. Dependencies and manifest files count as files; adding a package or external service is not automatically covered by permission to edit source.

Path checks consider traversal, case aliases, drive-relative paths, UNC paths, Windows reserved names, symlinks/reparse points, destination parents that do not yet exist, and both endpoints of rename. Re-resolve at mutation time or use suitable handle-relative operations. A process-local queue does not exclude unrelated external editors or other processes.

A shell command's allowlisted spelling does not prove its filesystem/network effects. If a hard scoped contract cannot be enforced for an opaque executable, deny or require an explicit contract adjustment that honestly states the available boundary. Do not describe a worktree, approval mode, command parser, LSP setting, or ConPTY as an OS sandbox. Unknown custom/MCP effects are not read-only merely because a tool describes itself that way.

Scope widening is a typed user decision showing old/new scope and reason, tied to the exact task/contract revision. It requires explicit host authentication even in Always Approve. The approved new contract must be durably committed before execution resumes. Denied/deferred expansion does not let the model continue by choosing a different alias.

## 6. Source manifests and evidence

F06 owns a content-based `SourceManifest`: canonical root identity, relevant file contents/hashes, absence/deletion markers, dependency/config/toolchain inputs, and coverage/completeness metadata. Git HEAD and porcelain status are useful metadata, not content fingerprints. A same-size edit to an already dirty file must change the manifest. Relevant nested non-Git files must not be reduced to shallow names and lengths.

F04, F08, F10, and graph replay consume this contract rather than defining incompatible notions of freshness. An incomplete or unreadable relevant source set is explicitly unknown; do not hash missing data as an empty valid file. A broad verifier may require a whole-project manifest while a narrow read can use a documented smaller dependency closure.

A host-produced verification receipt binds task, attempt, owner generation, task/contract revision, source manifest, actual execution ID, command or assertion identity, start/end, exit/outcome, output/artifact digests, runtime/tool versions, and provenance. Outcomes distinguish executed, skipped, simulated, aborted, unavailable, unknown, failed, and passed. Model prose and model-provided receipt fields cannot mint this authority.

Capture receipt metadata before post-hook rewriting and output compression. Retain a separate hook veto. The existing `davinci-agent/src/evidence.rs::EvidenceStore` retains overflow text; the proposed `VerificationEvidenceStore` is the typed receipt facade and may reference those lossless artifacts. Do not overwrite or confuse the two stores. Ensure large output remains retrievable after pruning.

Completion has distinct dimensions: source changed, required tests, required build, installed binary identity, automated live interaction, and manual checks where applicable. A task completes only when its declared required dimensions have fresh real proof and the owner/revision CAS still succeeds. Zero checks, all-skipped checks, simulated exit zero, missing exit information, an unavailable mandatory scanner, or a storage failure cannot satisfy a mandatory dimension. Legacy optional security policy is not evidence that a required scanner ran.

## 7. Owned mutation ledger and rewind

Record admitted native file mutations with task/attempt/owner identity, canonical target, before/after byte hashes, lossless before/after storage or reversible edit representation, and completed/unknown effect state. Capture preexisting dirty content as the baseline, not Git HEAD. A graph's observed baseline delta alone cannot attribute later manual edits to a worker.

Rewind first builds a bounded preview. Exact current post-images can be inverted. A proven nonoverlapping three-way inverse may preserve later manual edits. Overlap, missing content, changed ownership, external effects, or uncertain shell mutations produce conflicts/irreversible warnings; preserve current files rather than guessing. Added, deleted, renamed, binary, large, CRLF, and permission-sensitive files require explicit coverage or an unsupported result.

After approval, stop/drain affected owned work and revalidate the full preview digest. Journal the inverse transaction and apply only if all required preconditions hold. Recovery is itself guarded against new manual changes. Existing patch-crash recovery does not automatically provide this contract. Never implement task rewind as whole-tree git reset/restore/clean.

Code restoration, task-state restoration, and transcript branching are separate explicit selections. Old checkpoints remain immutable; new branches invalidate dependent evidence. Irreversible external effects are not erased by rewinding a transcript. Spend and actual execution history never rewind.

## 8. Whole-task accounting and control

A single root `BudgetLedger` covers foreground models, child workers, graph nodes, retries, review, verification, optional backend processes, and managed tool work. Maintain committed actual usage, reserved usage, and unknown/pending usage by immutable attempt IDs. Child grants refer to the same root; they do not start a new free budget.

Admission uses atomic remaining-capacity checks. Preserve a verification/recovery reserve so exploratory work cannot spend every remaining unit and then claim unverified completion. A final unknown provider cost is not zero and is not automatically refunded. Reconcile estimates when trustworthy usage arrives, exactly once. Canceled backoff is not a new provider attempt; an actual started retry is. Scope denial and ledger replay do not count as executed tool effects.

Use checked integer units for bounded token/count/time accounting; define money precision and reject NaN, infinity, negatives, overflow, and unknown units. Preserve legacy graph zero-as-unlimited at its configuration adapter, then use an explicit Unlimited variant internally. An exhausted finite zero balance is not Unlimited. Retry, resume, fork, and rewind retain consumed spend. Graph summaries are projections of receipts and cannot double count a reused artifact or lose failed-attempt cost.

The progress watchdog observes bounded fingerprints: task/attempt, canonical tool and arguments, result digest, meaningful state/evidence/source change, and elapsed/iteration history. Repeated reads, repeated errors, no-change edit/test cycles, excessive retries, and idle polling without progress are distinct signals. Whitespace or a new call ID is not progress; new content or a legitimate advancing job may be. A trigger warns/yields/pauses according to policy and offers the spec's bounded user choices without inventing permission or unbounded resets.

A real control registry maps workers to cancellation tokens, process handles/groups, jobs, and transports. Stop changes desired state, reaches provider/backoff/tool children, waits for owned exit, escalates only within owned descendants, and reports failures. It must not kill a user's unrelated shell, sibling, or parent. Steering is user data delivered at a documented safe boundary, never new authority; queued, delivered, applied, and rejected receipts remain distinct.

## 9. Context and memory inspection

Capture an immutable manifest of the actual provider view after history projection, Living Plan/ephemeral context, system and tool-schema composition, and graph-specific packet construction. Runtime and ecosystem ContextPacket types remain adapters, not interchangeable structures. Record candidate inclusion/exclusion reasons, source/version, role, estimated size, protection status, and cache-affinity metadata without duplicating secrets.

The inspector reads snapshots. Opening, refreshing, or scrolling it must not recollect memory, invoke a provider, load a new skill, or change a worker's already frozen input. Label heuristic token estimates separately from provider-reported usage; estimates are not automatically an exact tokenizer or an upper bound.

Pin/exclude applies only to optional items within authorized session/project/profile scope. Mandatory safety, ownership, permission, task contract, and protected prompt context cannot be removed. Retrieved text claiming to be a safety instruction does not acquire that status. Preserve provenance distinctions between a user fact, an inference, and tool evidence. Correction/forget actions use explicit scoped decisions and tombstones; they invalidate affected future injection/cache state without pretending a previously sent request can be unsent.

Pruning retains lossless retrievability, preserves structured image blocks, and resets stale governor deduplication. The inspector is not a second context assembly implementation.

## 10. Interaction transport and UI

Extend the existing blocking approval callback and native channel/Ask overlay. Policy produces the legal scopes; the view only renders them. Approval challenges bind final tool arguments, target identity, current mode/policy/scope revision, and an expiry/cancellation state. Reject unknown, stale, duplicate, unoffered, or differently scoped replies. A project save failure cannot silently broaden or substitute a grant.

Clarification uses a distinct question/reply payload and record type, not ToolApprovalDecision. It is explicitly serial in the scheduler because the existing RPC UI waiter is thread-local. Native extension UI currently drained after a turn is not a synchronous mid-turn bridge. Print and noninteractive children return an explicit unavailable/cancelled result; they never wait for nonexistent user input or invent a default answer.

Preserve existing modal-first `input_owner`, autocomplete/composer behavior, SheetChrome, Unicode wrapping, scroll bounds, voice safety, and draft/caret state. During approval/decision/scope modals, navigation keys belong to the modal and Shift+Tab must not cycle modes. At idle, the exact five-mode cycle remains Manual → Accept Edits → Plan Mode → Auto Mode → Always Approve → Manual. Ordinary Tab keeps its existing completion behavior.

RPC additions use capability negotiation or existing legal select/input adapters. Matching request IDs and explicit terminal outcomes are mandatory. Distinguish timeout from EOF/disconnection; release waiting calls exactly once. Keep stdout JSON clean and retain legacy TUI cancellation semantics.

## 11. Reusable graph and graph operations

F12 owns a strict versioned saved-definition DTO under `.davinci/graphs/<safe-name>.yaml`, distinct from existing run history under `.davinci/graph/runs` or legacy `.pi/graph/runs`. The DTO compiles into a frozen native execution definition with bounded role/artifact/input bindings. Existing roles and artifact contracts remain finite: no embedded code, arbitrary prompt mutation, dynamic topology expressions, shell interpolation, or permission grants.

Validate duplicate/unsafe IDs and YAML keys, unknown fields/version, node/edge bounds, role/artifact/mutation coherence, references, DAG/reachability, writer ordering, and every successful completion path's applicable gates. Existing GraphDefinition serialization and topology validation alone are not sufficient to execute an imported definition. The controller must follow compiled bindings rather than regenerate a different graph from classification or skip readiness checks for unknown node IDs.

Retain one mutation-capable graph writer at a time, even across disjoint scopes. Real verification and applicable security remain controller-enforced gates. Standard/Complex review coverage remains mandatory; existing Simple mode does not include a Reviewer, and this bundle does not silently change that product policy. Graph retries invalidate affected descendant artifacts and verification/review proofs. Unknown partial Writer outcomes require reconciliation before retry.

F13 owns typed lifecycle/attempt controls and selected-run/node identity. F14 adds immutable checkpoints, parent lineage, source/fingerprint binding, and retention pinning. Retry/fork/rewind never overwrite the source checkpoint or reset spend. Explain and diff are bounded read-only projections of owned execution state, not model-generated rationalizations or private chain-of-thought.

Distinguish the existing simulated `--dry-run` pipeline from the proposed `/graph dry-run` preflight. Existing simulation can persist fake artifacts; preflight performs zero model/worker/verification executions and no run-state mutation. Never present either as real verification. Verify-only runs actual authorized checks against the current source manifest and regenerates required proof without re-running writer work.

Exports are versioned, bounded, declarative snapshots. Default exports exclude raw context/transcripts/credentials/private local paths. Redaction is defense in depth, not proof that arbitrary raw logs are safe; inclusion is allowlisted. Exported YAML/metadata cannot carry executable authority on later import. Legacy bare/goal command routing and internal unadvertised lifecycle names remain compatible; reserved verbs have an explicit goal escape.

## 12. Optional backend admission and references

No native YAML DAG parser, LSP client, or active native PTY test dependency was found in the inspected manifests. New integrations must use exact dependency pins, frozen lockfiles, offline provisioned test environments, license/security review, and the pinned Rust 1.83.0 toolchain. A candidate's published version does not prove that its full dependency closure compiles on this repository or operating system.

| Area | Proposed decision | Admission evidence required |
|---|---|---|
| LSP | Bounded stdio JSON-RPC over existing std/serde_json; no new asynchronous runtime required. | Framing/cancellation/version/Unicode fixtures; trusted server process containment; actual per-server capabilities. |
| YAML | Evaluate exact `yaml-rust2 = 0.10.4` parser candidate with unnecessary features disabled; do not use the flat frontmatter parser. | MSRV/platform/lockfile build, duplicate-key/token visibility, bounded parsing and rejected alias/tag/multidocument fixtures; choose a different exact pin if it fails. |
| PTY | Evaluate exact `portable-pty = 0.9.0` with exact `vt100 = 0.15.2` screen parser candidate. | Rust 1.83 dependency closure, Windows ConPTY and Unix lifecycle/resize/cleanup, VT behavior needed by fixtures. |
| Browser | Managed separately provisioned exact-version Playwright runtime/bridge, frozen package lock; no implicit downloads. | Selected version recorded at implementation, supported host/browser binaries, loopback fixture policy, real DOM assertions, console/network artifacts, and process cleanup. |

Candidate versions are intentional evaluation inputs, **not claims of the latest release or verified compatibility**. Browser version is selected and pinned in the admission task rather than invented in this documentation. Server/browser runtime configuration cannot be supplied by an untrusted repository without policy review.

Primary technical references consulted during planning:

- Microsoft LSP 3.17 specification: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/ — protocol framing, capabilities, position encoding, and cancellation contracts.
- Microsoft protocol source: https://github.com/Microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/specification.md — normative protocol source; implementations still need fixtures.
- Microsoft ConPTY session lifecycle: https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session — create/configure before launch, independent I/O handling, resize, and shutdown concerns.
- Microsoft Job Objects: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects — Windows process-group ownership/management primitives; not proof of general filesystem isolation.
- Playwright tracing: https://playwright.dev/docs/api/class-tracing — context tracing does not replace explicitly recorded test assertions.
- Playwright network and Trace Viewer: https://playwright.dev/docs/network and https://playwright.dev/docs/trace-viewer — supporting network and trace artifacts, not permission boundaries.
- YAML 1.2.2: https://yaml.org/spec/1.2.2/ — YAML syntax baseline; the saved-graph format intentionally supports a stricter bounded subset.
- yaml-rust2 candidate: https://docs.rs/crate/yaml-rust2/0.10.4 and https://docs.rs/crate/yaml-rust2/0.10.4/features — published candidate and feature information.
- portable-pty manifest: https://github.com/wezterm/wezterm/blob/main/pty/Cargo.toml — inspected 0.9.0 manifest; repository main can move, so pin and review the exact release during implementation.
- vt100 candidate manifest: https://docs.rs/crate/vt100/0.15.2/source/Cargo.toml — published version/dependency manifest.

These references support protocol/backend considerations; all task contracts, safety decisions, and proposed limits above are engineering design choices for this repository. The local audit is the source for claims about existing Davinci code.
