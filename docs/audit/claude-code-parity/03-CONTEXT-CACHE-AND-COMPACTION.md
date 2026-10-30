# 03 — Context assembly, prompt cache and compaction

Status: evidence record. No code changed for this subsystem in this audit.
Research basis: F-07-01…05, F-08-01…05, F-09-01…04, F-35-01…05, F-41-01…07.
Research reports: `07-context-assembly.md`, `08-prompt-cache-and-tools.md`,
`09-session-persistence.md`, `35-auto-compact.md`, `41-token-cache-effectiveness.md`.

## 1. Request path (verified)

```
main.rs  provider_messages = current.messages_for_provider()
         request_tools    = provider_tools(current)
         system           = current.provider_system_prompt()
         cache_key        = live_cache_key(model, auth, &system, current, &request_tools)
         live_complete_streaming_with_sink_envelope(...)
```

`crates/davinci-coding-agent/src/main.rs:3535-3578`; the loop is
`Agent::run_loop` in `crates/davinci-agent/src/turn.rs:212-215`.

## 2. Prefix stability

| Mechanism | Evidence | Test |
|---|---|---|
| A composed turn prompt is split; only stable text goes into `instructions` | `prompt/turn.rs:38-61`, `lib.rs:2423-2437` | `cache_stability.rs:138-274` |
| Runtime state (permission mode, plan revision, environment, extra dirs, codemode guidance) is `Dynamic` and rendered in `<runtime_state>` | `prompt/runtime_state.rs:37-113` | `cache_stability.rs:152-191` |
| The split is applied only to OpenAI reasoning families; other routes keep turn state in the system prompt by design | `turn_context.rs:29-41`; override `DAVINCI_TURN_CONTEXT=system\|appended` | `cache_stability.rs:277-296` |
| Nested instruction files are appended as a `davinci.nested_instructions` message, not folded into the prefix | `instruction_files.rs:60-76` | `instruction_scope.rs` |
| Project context uses cwd-relative paths, so the checkout root never enters the prefix | `context.rs:48-108` | — |

Remaining prefix-break sources, each verified:

- cwd/shell/tool change → the environment snapshot re-renders (`prompt/environment.rs:352-409`);
  the date is frozen per turn (`:366-385`).
- tool set change → the tools array changes (`cache_stability.rs:22-24`);
- an edit to `AGENTS.md` → `provider_system_prompt` changes (`lib.rs:3542-3552`);
- compaction, by design.

## 3. Tool schema determinism and exposure

| Property | Evidence | Test |
|---|---|---|
| Responses tool array sorted by name, with a schema digest | `davinci-ai/src/responses_tools.rs:98-120`, `:161-166` | `request_shape.rs:90-152` |
| Agent-side order: builtins → MCP → capability registry sorted by name | `lib.rs:4255-4280`, `:4468-4507`, `runtime/capabilities.rs:781-790` | `lean_tool_surface.rs` |
| Non-Responses builders do **not** sort (`openai_body`, `anthropic_body`) | `davinci-ai/src/stream.rs:2692-2709`, `:3047-3058` | — (determinism then rests on caller order) |
| No `preserve_order` feature in the workspace, so JSON objects serialise alphabetically | `Cargo.toml`/`Cargo.lock` | — |
| The tool array can still grow at a user-turn boundary on the frozen route (MCP registration, relevance re-application) | `lib.rs:4343-4393` | `lean_tool_surface.rs:74-110` |

**Default surface is `Full`** (`lib.rs:437-441`; `settings.rs:1464-1469`): out of the
box every authorized schema is sent. `lean` is opt-in.

## 4. Prompt-cache wire fields

- `prompt_cache_key` (and, on Codex, `prompt_cache_options {mode, ttl}`) unless
  retention is `none` (`davinci-ai/src/stream.rs:2098-2110`). Azure gets the key only
  (`:2092-2096`). Legacy retention fields are capability-gated (`:2114-2143`);
  `apply_chatgpt_plan_preview_constraints` strips retention for public plan models
  (`:1742`).
- Anthropic breakpoints are placed on the last user block and the last tool
  (`:3011-3014`, `:3060-3068`; tests `:5296-5427`).
- Cached input is read from `/input_tokens_details/cached_tokens` and kept as a
  disjoint bucket from cache writes; an unreported count is a flag, never a zero
  (`stream_decoder.rs:927-975`; tests `:1841`, `codex_ws_public_tests.rs:629-638`).
- Usage is persisted with the assistant entry (`turn.rs:5373-5439`).
- Accounting: `cache_stats.rs:26-200` (`cache_observation`, `detect_miss`,
  `compute_cache_waste`), tests `:602-720`. Missing usage is `Unknown`.

## 5. Compaction

### 5.1 Trigger and fit

`compaction_threshold` clamps the requested threshold to
`[keep_recent + reserve, context_window - reserve]` (`compaction.rs:216-236`).
`find_cut_point` never splits a tool result (`:239-241`, `:258-307`). The result is
validated against `max_tokens` and an assistant-reply rule; if nothing fits, the
transcript is returned unchanged with a `NO_ASSISTANT_REPLY_SUMMARY` (`:469-559`).

Active mode decides with `ContextFoldPolicy`; other modes use `should_compact` then
`compact_automatic` (`turn.rs:779-861`). Hybrid measures with the provider's own
count (`:764-773`) and retries once after a provider overflow (`:967-985`).
Failed automatic attempts back off (`lib.rs:5226-5276`).

### 5.2 Commit ordering

The session entry is appended **before** live history is replaced; a failed append
restores the previous messages and reports "Compaction not saved" (`lib.rs:5441-5461`).
Sessionless hybrid rebase failure rolls back (`:5465-5480`). Each Context VM fold
chunk is checkpointed before the next summariser call, with `undo_fold` if the
checkpoint cannot be written (`:4823-4871`); a fold that exceeds `MAX_FOLD_CHUNKS`
fails closed keeping completed chunks (`:4763-4768`; test `context_fold_chunks.rs:76`).

### 5.3 Preserving user constraints

Strong on the Context VM path only:

- the fold prompt requires verbatim user grounding and states that only a user may
  change a user constraint (`runtime/context_vm/fold.rs:64`);
- the reducer rejects ungrounded user/policy values (`reducer.rs:333-338`), restricts
  supersession to newer user/policy evidence (`:635-663`), and keeps exact user words
  in goals (`:154-179`, `:788-817`);
- tests `wor62_*` (8 tests, `context_vm_integrity.rs:46-330`) and
  `requirements_survive_repeated_compaction_*` (`context_vm_hybrid.rs:581-633`).

The **default** mode is `Off` (`context_vm/types.rs:40`). In `Off`, compaction is the
plain summariser whose only constraint handling is a prompt section; nothing validates
the produced summary. The repository's own fixture records 394/883 requirement checks
kept in `Off` mode (`docs/context-vm-measurements.md:59`).

## 6. What is measured and what is not

| Claim in repo | Kind | Honest reading |
|---|---|---|
| 300-turn fixture table | offline structural, fixture summariser | plumbing, not model fold quality (`context-vm-measurements.md:52-62`) |
| 60-turn table, "non-prefix bytes" | structural upper bound | explicitly "not a measured cache miss" (`:121-123`) |
| requirement-retention table | offline plumbing | "measures the plumbing, not model quality" (`:10-12`) |
| graph/TUI benchmark tables | local release run, offline | owned-layout estimate, not RSS; p95 fixture-specific (`:178-213`) |
| cached-token usage matrix | offline fake usage blocks | "does not measure billed cache hits" (`openai-efficiency.md:140`) |
| "cache-sharing compaction with `tool_choice: none`" | shape accepted | "zero observed cache reuse" (`:229`) |

No live before/after cache measurement exists (`openai-efficiency.md:198`, `:233`;
`context-vm-measurements.md:243-247`). This audit made none.

## 7. Gaps worth acting on (priority assigned in `07-GAP-MATRIX.md`)

| # | Gap | Why it matters | Evidence |
|---|---|---|---|
| CTX-1 | Default `Off` leaves constraint preservation to an unvalidated summary | the most user-visible regression after a long session | `types.rs:40`, `compaction.rs:13-15` |
| CTX-2 | `Full` tool surface by default | prefix size and cache-miss blast radius grow with every capability | `lib.rs:437-441` |
| CTX-3 | Skill bodies all read at startup | context cost is deferred, I/O and memory are not | `skills.rs:131-158` |
| CTX-4 | No test for mid-session MCP catalog mutation and the resulting prefix break | the documented "additive registration at a turn boundary" is unexercised | search found none |
| CTX-5 | No live cache measurement | cannot claim a saving; cannot detect a regression on the provider side | `openai-efficiency.md:233` |

## 8. Explicit non-claims

- A smaller prompt, a prefix hit or a cache read is **not** evidence of task success
  or of subscription savings.
- None of the numbers above were produced by this audit. They are either absent or
  labelled offline/structural by their own authors.
- `Off` mode's 394/883 figure is a fixture measurement of plumbing, not a model
  quality result, and is not comparable to anything Claude Code reports.
