# First-request anatomy — DaVinci vs Droid (Stage 0, P0.2)

Offline, $0. No provider request was made and nothing was spent. Every DaVinci byte below is
produced by an offline Rust test that builds the first provider request for the prompt `hi` and
reads its `WireManifest`; every Droid cell comes from the Task 11 mock capture (a local mock on
port 18317, no provider request) or is `no byte source` (the value is not split out of Droid's
request).

## 1. The table

| Configuration | Tool count | Tool-schema bytes | Instruction bytes | Project-instruction bytes | Turn-context bytes | Total request bytes | Source |
|---|---|---|---|---|---|---|---|
| DaVinci Full | 40 | 25,906 | 9,747 | 5,650 | 426 | 36,447 | this test (`first_request_anatomy`) |
| DaVinci Lean | 16 | 12,830 | 9,747 | 5,650 | 426 | 23,371 | this test (`first_request_anatomy`) |
| DaVinci Full+Codemode | 41 | 27,444 | 9,747 | 5,650 | 1,176 | 38,736 | this test (`first_request_anatomy`) |
| Droid direct | 23 | 69,711 | 6,150 | no byte source | no byte source | 94,606 | mock capture, `direct_reads` request 1 |
| Droid script | 5 | 29,746 | 6,075 | no byte source | no byte source | 54,780 | mock capture, `script_reads` request 1 |

Units, so the columns are not misread:

- **Tool count** — the number of entries in the request body's `tools` array.
- **Tool-schema bytes, instruction bytes, total request bytes** — JSON-encoded wire bytes, taken
  from `WireManifest` (`crates/davinci-ai/src/responses_request.rs`). `instruction bytes` is the
  `instructions` field, a JSON string, so its newlines are `\n`-escaped and each counts two bytes.
- **Project-instruction bytes, turn-context bytes** — raw text bytes of the
  `<project_context>` and `<turn_context>` blocks respectively (the two sub-strings the
  `WireManifest` does not split out). They are the content size, not the JSON-encoded size, so
  they are not directly additive with the JSON-encoded columns. They are also **contained in**
  other columns, not disjoint from them: the `<project_context>` block is a substring of the
  `instructions` value, so its bytes are already inside `Instruction bytes`; the `<turn_context>`
  block is the text of the turn-context message, so its bytes are already inside the
  `conversation B` figure (section 2). Do not add either column to anything.

### 1b. Droid capture (Task 11)

Source: the first `POST /v1/responses` the 1.1.0 binary sent to a local mock on `127.0.0.1:18317`
(per-process `--settings`, alias `custom:mock-probe-0`, dead-proxy env; CLIProxyAPI untouched),
prompt `read three files`, empty temp cwd, `--auto high`. Request body keys: `include`, `input`,
`instructions`, `model`, `parallel_tool_calls`, `prompt_cache_key`, `reasoning`, `safety_identifier`,
`store`, `stream`, `text`, `tool_choice`, `tools`. Bytes are the UTF-8 length of each field's compact
JSON substring of the raw body (found verbatim in it), so they are wire bytes like the DaVinci columns.

| Droid request | Tools | `tools` B | `instructions` B | `input` B (2 items) | Total body B |
|---|---:|---:|---:|---:|---:|
| script-only | 5 (`Script`, `WaitForScript`, `TaskOutput`, `TaskStop`, `Task`) | 29,746 | 6,075 | 18,593 | 54,780 |
| direct-only | 23 | 69,711 | 6,150 | 18,379 | 94,606 |

- The 1.0.0 reference gave 5 / 29,728 B and 23 / 69,689 B, so the tool sets are unchanged and the
  schema bytes grew by 18 B and 22 B.
- `input` is one `<system-reminder>` item (headed "Unified tool catalog ...") plus the 52 B user message. Project instructions and turn context
  are not split out of it, so those two columns stay `no byte source`. This probe cwd held no
  `AGENTS.md`, so a real workspace adds its project text on top.
- The prompt differs from DaVinci's `hi` (`read three files` adds a few bytes), and DaVinci's
  figures use `gpt-5.6-luna` while the Droid request used `model: gpt-6.1-sol`. The columns are
  therefore comparable by category, not byte for byte.
- `--list-tools` (30 tools, 79,558 B as JSON) is still only the available-tool listing, not the
  request tool array.

## 2. DaVinci numbers (test output, `--nocapture`)

```
configuration             tools    tool-schema B    instruction B project-instr B (in instr)   turn-context B (in conv)   conversation B          total B     structural B
DaVinci Full                 40            25906             9747                       5650                        426              588            36447               36
DaVinci Lean                 16            12830             9747                       5650                        426              588            23371               36
DaVinci Full+Codemode        41            27444             9747                       5650                       1176             1339            38736               36
note: project-instr B is a raw-text substring contained in instruction B;
note: turn-context B is raw text contained in conversation B. Do not add columns.
```

`conversation B` (the `ConversationItem` segments — the user `hi` message plus the turn-context
message wrapper) and `structural B` (the top-level JSON key/brace/comma bytes that no
`WireManifest` segment carries) are printed as cross-checks, not as table columns. The
`project-instr B` and `turn-context B` columns are raw text contained in `instruction B` and
`conversation B` respectively, so none of the printed columns should be summed.

What the test asserts for every row, and nothing beyond this:

- the `tools` array length equals `provider_tool_specs().len()` and is non-zero;
- `ToolContract` bytes equal the serialized `body["tools"]`; `TrustedInstructions` bytes equal the
  serialized `instructions` (plus any `system` value and developer/system input items);
  `ConversationItem` bytes equal the serialized remaining `input` items — each recomputed from the
  body, not from the manifest;
- the manifest's total segment bytes equal the independently computed tools + instructions +
  conversation + remaining-config object + routing object bytes (dropping any one segment from the
  manifest makes this fail; verified by skipping the first segment, which failed 36,270 vs 36,411);
- the structural remainder (`total - sum(segments)`) is the same in all three configurations.

These checks catch a dropped or mis-sized manifest segment, but they reuse the same top-level key
grouping as `build_segments`, so they do not independently prove that the 36-byte structural
remainder is the exact JSON punctuation; it is only shown to be constant across configurations.

## 3. Reproduction

```
cargo test -p davinci-agent --test first_request_anatomy -- --nocapture
```

The test is offline and deterministic except for two inputs it deliberately reads from the
current checkout: the `AGENTS.md` at the repository root (project-instruction bytes) and the
built-in tool catalog (tool count and tool-schema bytes). Re-running it after those change
reproduces the new numbers.

## 4. Per-column sources (DaVinci)

| Column | Symbol / line that produced it |
|---|---|
| Tool count | `body["tools"].len()`, body from `cache_stability::wire_body_for_next_request` (`crates/davinci-agent/src/cache_stability.rs:45`); cross-checked against `Agent::provider_tool_specs` (`lib.rs:4705`) |
| Tool-schema bytes | `WireManifest.segments` filtered to `WireSegmentCategory::ToolContract`, summed `byte_len` (`crates/davinci-ai/src/responses_request.rs:189-197`) |
| Instruction bytes | `WireSegmentCategory::TrustedInstructions` sum (`responses_request.rs:199-216`); the `instructions` field, whose value is `Agent::provider_system_prompt` (`lib.rs:3779`) |
| Project-instruction bytes | raw length of the `<project_context>` block appended by `context::append_repository_context` (`context.rs:48`) from `agent.context_files`, loaded by `Agent::load_instruction_files` (`instruction_files.rs:57`) |
| Turn-context bytes | raw length of the `<turn_context>` block rendered by `turn_context::render_turn_context` (`turn_context.rs:92`) and appended by `Agent::commit_turn_context` (`lib.rs:2576`) |
| Total request bytes | `WireManifest.request_bytes_before_compression` (`responses_request.rs:85`) |

### Configuration seams used

| Configuration | Seam |
|---|---|
| DaVinci Full | `Agent::new_builtin(PromptProfile::Stable)`, `provider="openai-codex"`, `model_id="gpt-5.6-luna"`, `TurnContextPlacement::Appended`, `ToolSurface::Full` + `freeze_tools_for_cache()` |
| DaVinci Lean | same, with `ToolSurface::Lean` + `freeze_tools_for_cache()` (exposes `tools::LEAN_TOOLS`) |
| DaVinci Full+Codemode | same as Full, plus a journal-backed runtime and `Agent::enable_read_only_codemode` (`codemode/execution.rs:85`), which registers the real `codemode` schema |

All three use the same model as the existing offline backend-route seam
(`cache_stability::codex_test_model`, `openai-codex/gpt-5.6-luna`). The tool count and
tool-schema bytes are model-independent; the instruction bytes can shift a few bytes per model
via the provider adapter (`prompt::provider`), and are reported for `gpt-5.6-luna`.

### Why "summed segment bytes ≠ total" (one note on the plan's wording)

The plan says to assert "the summed segment bytes equal `request_bytes_before_compression` when
every segment is counted". `build_segments` (`responses_request.rs:148`) places the body's
*values* into segments but no segment carries the top-level JSON structure (key names, braces,
commas), and it re-wraps the model-visible config and routing keys in their own objects. The sum
is therefore the total **minus a small structural remainder** — 36 bytes for every configuration
here. The test therefore does not assert segment sum equals total; it asserts the checks listed in
section 2.
