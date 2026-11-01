# First-request anatomy — DaVinci vs Droid (Stage 0, P0.2)

Offline, $0. No provider request was made and nothing was spent. Every DaVinci byte below is
produced by an offline Rust test that builds the first provider request for the prompt `hi` and
reads its `WireManifest`; every Droid cell is either `not re-checked` (the mock capture it
needs is forbidden by R3 — the live CLIProxyAPI holds port 8317) or `no byte source` (Droid's
offline surface does not expose the value).

## 1. The table

| Configuration | Tool count | Tool-schema bytes | Instruction bytes | Project-instruction bytes | Turn-context bytes | Total request bytes | Source |
|---|---|---|---|---|---|---|---|
| DaVinci Full | 40 | 25,906 | 9,747 | 5,650 | 426 | 36,447 | this test (`first_request_anatomy`) |
| DaVinci Lean | 16 | 12,830 | 9,747 | 5,650 | 426 | 23,371 | this test (`first_request_anatomy`) |
| DaVinci Full+Codemode | 41 | 27,444 | 9,747 | 5,650 | 1,176 | 38,736 | this test (`first_request_anatomy`) |
| Droid direct | not re-checked | not re-checked | no byte source | no byte source | no byte source | no byte source | `droid-1.1.0-identity.md` fact 5 |
| Droid script | not re-checked | not re-checked | no byte source | no byte source | no byte source | no byte source | `droid-1.1.0-identity.md` fact 4 |

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

### Droid `not re-checked` — why

Task 3 (`droid-1.1.0-identity.md`) re-checked what it could offline, but the only source for
Droid's *request* tool count and tool-schema bytes is the scripted mock capture, which binds port
8317 where the live CLIProxyAPI runs (rule R3 forbids stopping it). The 1.0.0 reference recorded
**direct: 23 tools / 69,689 B** and **script: 5 tools / 29,728 B** (reference ch. 19 §2.1); those
are 1.0.0 figures and are **not** restated as current. The only re-checked Droid number is
`--list-tools` = **30** base-catalog tools for every mode, which is the available-tool listing,
not the request tool-schema count, and cannot distinguish direct from script.

Droid has no offline byte source for instruction, project-instruction, turn-context or total
request bytes: its `--list-tools` JSON dump (79,558 B) is a serialization of the listing output,
not a request capture, and no request is ever built without a live provider or the mock.

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
