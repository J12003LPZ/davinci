# Davinci harness reliability assessment and design

Date: 2026-09-04. Scope: the active Rust workspace, with `davinci-coding-agent`
as the product entry point. Legacy TypeScript and `vendor/pi` are behavioral
references, not implementation targets. Existing untracked `plugins/` is out of scope.

## Objective and acceptance

Improve real coding-task reliability and context efficiency with small, reviewable
changes to existing mechanisms. This audit does not establish competitive parity:
that requires controlled end-to-end runs with the same model, tasks, permissions,
and evaluation criteria. No paid provider runs, dependency additions, commits,
installation, deployment, or credential changes are part of this work.

Completion requires source-backed findings, a prioritized implementation plan,
targeted regression checks, a final diff review, and an explicit account of
unverified behavior. Only affected tests and compilation checks are required.

## Architecture and inspection map

| Surface | Active implementation | Assessment focus |
| --- | --- | --- |
| Product setup and provider requests | `crates/davinci-coding-agent/src/main.rs`, `settings.rs`, `model_resolver.rs` | Actual prompt/tool assembly, trust, runtime wiring |
| Agent loop and recovery | `crates/davinci-agent/src/{lib,turn,events,stats}.rs` | Retry classification, cancellation, completion evidence |
| Context and instructions | `crates/davinci-agent/src/{context,compaction,pruning,skills,templates}.rs` | Discovery, retained state, accurate budgets |
| Tools and edits | `crates/davinci-agent/src/{tools,apply_patch,edit_diff,file_mutation_queue,jobs,evidence}.rs` | Target integrity, output bounds, terminal recovery |
| Permissions and delegation | `crates/davinci-agent/src/{permission,scheduler,batch,subagent,tool_ledger}.rs` | Authority, mutation barriers, replay identity |
| Native orchestration | `crates/davinci-coding-agent/src/native_extensions/{graph,ecosystem,learning}/` | Graph verification, recovery bounds, learning provenance |
| Retrieval and token governor | `crates/davinci-coding-agent/src/native_extensions/{token_governor,vector_memory}.rs`, `extension_host.rs` | Stale results, repeated reads, compaction lifecycle |
| Provider portability and caching | `crates/davinci-ai/src/` | Provider request construction, cache/retry behavior |
| Persistence and measurements | `crates/davinci-session*/`, `crates/davinci-{telemetry,evals}/` | Durable state, metric provenance, task success |

The inspection is architectural and targeted, not a claim of exhaustive review of
every line or a full security audit. Supporting crate interfaces and callers are
traced where they cross these boundaries.

## Strengths already present

- Rust-native tools, multiple providers, range reads, targeted search, precise
  edits, background jobs with retained stdin, and retrievable output evidence.
- Parallel read lanes with mutation barriers, per-file edit serialization, and a
  tool-call ledger that validates tool name and canonical arguments before replay.
- Provider-view pruning keeps original session evidence; structured compaction
  preserves goal, constraints, progress, decisions, and next steps.
- Explicit permissions, bounded subagent fan-out, graph artifacts, verification
  and review gates, and runtime counters already exist. Replacing these wholesale
  would add migration risk without demonstrating a benefit.

## Selected approach

Three options were considered: prompt-only tuning, replacing the agent architecture,
and repairing concrete integration failures. Repairing the existing mechanisms is
the recommended approach: it directly protects correctness and keeps changes small.
Prompt improvements are supplementary; they cannot enforce filesystem safety or
make a stale cache correct.

### Patch authority and recovery

The pre-change `execute_apply_patch` replays `.pi_patch_journal.json` before validating
the requested patch. That repository-controlled file can name unrelated files to
overwrite or delete. Permissions inspect the new patch, not journal actions.
Restoration failures are ignored and the journal is removed anyway.

Treat a pre-existing journal as unresolved data, never implicit authorization.
Ordinary patch execution must refuse it without changing files. Retain the explicit
recovery API for callers that deliberately authorize recovery; validate its full
target set first and retain the journal on recovery failure. Reserve the journal
path from patch targets and exclusively create new journals. Report incomplete
rollback and cleanup honestly. A full cross-process transaction manager is outside
this bounded repair.

### Context accounting

The pre-change `Agent::estimated_context_tokens` counts messages and ephemeral context but omits
the system prompt and tool schemas. Include stable request overhead in the budget,
using the actual product tool catalog where available. Keep estimates explicitly
approximate; no new tokenizer dependency or claim of exact billing tokens.

### Recovery and policy

The provider retry loop classifies transient failures, but sleeps through the full
backoff without observing cancellation. Make backoff interruptible and count actual
retry attempts. Keep retry bounds and existing public settings. Do not automatically
retry tool mutations whose outcome is unknown.

Keep stable engineering instructions cache-friendly. Prompt rewriting is deferred:
runtime integration failures have demonstrable impact, while more prompt prose
has no demonstrated task-success benefit in this audit.

## Ranked findings and delivered changes

Severity here ranks engineering/data-integrity impact; it is not a CVE rating or
a claim of remote exploitability. Cost/risk are relative to this repository.

| Priority | Observed problem and root cause | Delivered change / expected benefit | Cost and tradeoff |
| --- | --- | --- | --- |
| Critical | Patch permission names requested files, but automatic repository journal replay can delete/overwrite unrelated files. Restore errors were ignored. | Refuse existing journals; reserve journal targets; exclusive creation and flush; validate all recovery targets; preserve failed recovery state and report rollback/cleanup errors. Prevent hidden mutation authority. | Small/medium. Interrupted patches now require explicit, target-aware recovery; intentionally less automatic. |
| High | Global graph extras become allowed tools for every role; shell alias `exec_command` bypasses shell-policy checking. | Controller advertises extras only to writers; worker enforces baseline tools for nonwriters even if its set is altered. Shell aliases require command validation. | Small. Custom extras for nonwriters are intentionally refused until trusted effect classification exists. Regex shell policy is not OS isolation. |
| High | Native pre-tool mutex poisoning becomes `None`, interpreted as permission to run. | Recover the locked value and still invoke native guards, consistent with the outer host lock policy. | Small. Does not claim recovery of arbitrary extension state; only prevents silent guard bypass. |
| High | Context pressure excludes the system prompt and actual tool catalog. | Include system bytes plus cached per-request catalog/identity overhead; library fallback counts active builtin/MCP specs. Earlier pruning/compaction and more useful peak-context telemetry. | Small. Byte/4 remains approximate, not an upper bound; an oversized fixed prefix still needs catalog reduction. |
| High | Pruning/automatic compaction can remove the read body while the governor claims it remains visible. | At provider boundary, detect visibility-counter changes and clear governor ledgers; host hook records pruning passes. Original evidence/store remains intact. | Small. Additional reads after compaction are intentional; other embedders must notify their own visibility-based caches. |
| High | HEAD plus porcelain status is not a content fingerprint. Re-editing dirty files or changing ignored/outside paths can leave the key unchanged. | Live host supplies unknown freshness, so repeated searches execute; remove now-unused Git-state helper. Existing dedupe still accepts a complete authoritative key supplied by another caller. | Small. Two Git subprocesses per fingerprinted search are removed; repeated searches may consume more output, but cannot be suppressed by false freshness. No end-to-end token reduction is claimed. |
| High | Retry backoff ignores abort until its whole sleep ends; tests skip the real wait. Aborted responses can count as retry success. | Poll abort in 25 ms sleep slices in both test/runtime paths. Count actual extra attempts in `providerRetries`; record failed-call wall time too; do not mark aborted retries successful. | Small. Cancellation interrupts backoff, not necessarily a provider transport already in flight. Existing retry limits/classifier preserved. |
| Medium | Release gate reports tool-call deltas but ignores them when allowing regressions. Corpus references a removed package/file name. | Include median tool-call delta in the no-more-than-10% worsening condition; refresh JSONL path/package. | Small. Imported/synthetic measurements are still caller-supplied, and median gates can hide per-task outliers. |

### Intentional remaining opportunities

1. **High: a real end-to-end evaluation adapter and measurement provenance.**
   `davinci-evals/src/lib.rs` is fixture-driven or uses a caller-injected completion;
   `codex_eval.rs` consumes caller-populated metrics. The richer Codex JSONL telemetry
   writer is not wired into live completion. Build one runner that captures task
   success, changed paths, verification, provider usage, retries and interventions
   from real runtime events, explicitly labeling synthetic vs live rows. This is
   the next prerequisite for credible competitive claims, not another prompt rewrite.
   It requires fixture design and authorization for provider costs/external CLIs.
2. **High: compaction of the provider-visible projection.** `Agent::compact` still
   summarizes raw messages, including pruned tool output. Change it only with
   session-entry/first-kept-ID and resume tests: silently breaking durable replay
   costs more than the possible savings. Summarization and long-task continuity
   were source-inspected, not live model-tested in this pass.
3. **Medium: incremental memory indexing.** `agent_memory_messages` and
   `VectorMemory::index_messages` scan/chunk full history each settled turn before
   deduplication. Introduce a session-entry cursor with resume/branch/compaction
   reset contracts. This needs lifecycle tests and measured CPU/latency benefit.
4. **High for hostile environments: stronger execution/transaction isolation.**
   Journal exclusive creation is not a complete cross-process lock or race-proof
   filesystem sandbox; external processes can still change files/symlinks after
   validation, and patch writes are not power-loss-atomic. Explicit recovery may
   partially restore before an I/O failure, retaining the journal. Harden with
   protected state, file identities/stale-write checks and OS sandboxing as a
   separate design. Keep tool-wide session approvals explicit in the UI; path-scoped
   approvals would change existing consent semantics and need a separate UX contract.
5. **Medium: measure caching before changing transport identity.** Responses body
   cache keys and Codex websocket continuation use different identity paths. A
   role-stable prompt-cache key is not automatically a safe session-continuation key;
   do not merge them without cross-session isolation tests. Static capability
   profiles are configured claims, not observed provider feature support.

**Optional, intentionally not added:** another planner/router, a global repository
index, a new tokenizer dependency, more prompt narration, or an autonomous outer
loop. Existing facilities should first be evaluated on real tasks. No implementation
was justified merely because a competitor advertises a similar feature.

## Coverage and limits

- Inspected the active Rust execution path and supporting prompts, context,
  tools/search/editing/jobs, permissions, scheduling/ledger, native graph/governor/
  memory, provider/cache/usage, session/compaction, and eval/telemetry boundaries.
  This was not an exhaustive line-by-line or dependency vulnerability audit.
- Earlier read-only investigators contributed evidence; after the user's
  continuation instruction all implementation, verification and final review were
  solo. The separate security reviewer did not complete; no independent final
  review is claimed.
- No authenticated provider task runs, competitor runs, whole-workspace suite,
  coverage campaign, crash/power-loss test or cross-platform runtime test was run.
  Testing occurred on Windows. Existing untracked `plugins/` remains untouched.
- The existing canonical-argument tool ledger and retained child stdin were
  rechecked against source; historical findings about them were not assumed current.
- Public function signatures and existing retry configuration remain. Additive
  context-overhead configuration and `providerRetries` are new; older JSON stats
  deserialize with retry count zero. Rust consumers using exhaustive `RunStats`
  literals must account for the new field (no such local callers were found).

## External architectural evidence

These sources inform principles, not claims that Davinci matches competitors:

- [Anthropic context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents): focused context, compaction, structured notes, and bounded delegation.
- [Codex tool orchestration source](https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/orchestrator.rs): explicit tool approval and sandbox orchestration.
- [Hermes context compression and caching](https://hermes-agent.nousresearch.com/docs/developer-guide/context-compression-and-caching/): compression and caching must operate on the context actually owned by the runtime.

## Verification strategy

Use deterministic temporary-directory regressions for patch authority and failure
paths, scripted provider completions for recovery, and provider-view assertions for
context budgeting. Run affected module tests and check affected consumers compile.
Do not run the whole workspace, coverage campaigns, or paid benchmarks without an
actual need. Record results and residual findings in the implementation plan.
