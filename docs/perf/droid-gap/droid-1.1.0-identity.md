# Droid 1.1.0 — identity pin and fact re-check (Stage 0, P0.1)

Offline, $0. No model call was made and nothing was spent. Every number below comes from a
command run in this session (see [Commands run](#commands-run)); the 1.0.0 reference
(`C:\Users\sergi\Desktop\lean-droid-1.0\droid-re-research\DROID-1.0.0-COMPLETE-RUNTIME-REFERENCE.md`,
build `0.244.0`) is quoted only where a row is labelled `not re-checked`.

## 1. Identity table

### Droid

| Field | Value |
|---|---|
| Path | `C:\Users\sergi\bin\droid.exe` |
| Product version (`--version`) | `1.1.0` |
| Build tag (log `tags.version`) | `1.1.0` — changed from `0.244.0` (1.0.0) |
| SHA-256 | `7F23DA2D6D271EDD45F198B946F51A0EA7A779D7C48C7DCE3DDA2C81C52AA775` |
| Size (bytes) | `327,162,080` (was 326,704,352 in 1.0.0) |
| Authenticode signer | `Valid` — CN=**The San Francisco AI Factory, Inc.**, O=The San Francisco AI Factory, Inc., L=San Francisco, S=California, C=US |
| PE `FileVersion` | `1.4.4` (the Bun standalone runtime version, matches 1.0.0's Bun 1.4.4) |
| File write time | `2026-10-31T13:16:32.35-04:00` |

Build tag evidence — one fresh log line emitted by the 1.1.0 binary, from
`C:\Users\sergi\.factory\logs\droid-log-single.log` (newest log file, 761,268 B, written
2026-10-31 14:41:05):

```
[2026-10-31T17:16:23.913Z] INFO: Using cached system certificates | Context:
{"value":"124 certificates, cached 2157 minutes ago","tags":{...,"version":"1.1.0",...},
"factory.process.instance_id":"c40bd834-48f1-4a64-80c5-358841d58fee","process.pid":65356}
```

Note: for 1.0.0 the product version (`1.0.0`) and build tag (`0.244.0`) were two different
channels. For 1.1.0 the log build tag now reads `1.1.0` — the two channels coincide.

### CLIProxyAPI

| Field | Value |
|---|---|
| Binary path | `C:\Users\sergi\AppData\Local\CLIProxyAPI\cli-proxy-api.exe` |
| SHA-256 | `11DC49F746325F951096152B6B6499422E26E84D85CB5991023F2AAAC4225029` |
| Size (bytes) | `70,583,808` |
| Config file | `C:\Users\sergi\AppData\Local\CLIProxyAPI\config.yaml` |

Non-secret config keys (read from `config.yaml`; nothing secret is present there — the only
`api-keys` entry is the literal placeholder `sk-dummy`):

| Key | Value in `config.yaml` |
|---|---|
| `port` | `8317` (set) |
| `debug` | **not present** in `config.yaml` → defaults to `false` (per `config.example.yaml:45`) |
| `request-retry` | **not present** in `config.yaml` → defaults to `3` (per `config.example.yaml:154`) |

Also set: `host: "127.0.0.1"`, `auth-dir: "C:/Users/sergi/.cli-proxy-api"`,
`routing.strategy: "round-robin"`, `routing.session-affinity: true`,
`codex.disable-codex-cloaking: false`, `codex.optimize-multi-agent-v2: true`,
`claude-code.disable-cloaking-model-list: true`, `xai.inject-x-search: false`.

## 2. CLI surface table (1.1.0, from `droid exec --help`)

| Flag | Status in 1.1.0 | Evidence (exact help line) |
|---|---|---|
| `-m, --model <id>` | present | `-m, --model <id>    Model ID to use (default: gpt-6.1-sol)` |
| `--auto <level>` | present | `--auto <level>    Autonomy level: low|medium|high` |
| `--tool-execution-mode` | **present but hidden** (not in help) | no help line; accepted and validated at runtime (see below) |
| `-o, --output-format <format>` | present | `-o, --output-format <format>  Output format: text|json|stream-json|stream-jsonrpc|acp (default: "text")` |
| prompt (positional) | present | `Arguments: prompt   The prompt to execute` |
| `-f, --file <path>` | present | `-f, --file <path>   Read prompt from file` |
| `--list-tools` | present | `--list-tools   List available tools for the selected model and exit` |
| `--settings <path>` | present | `--settings <path>   Path to runtime settings file merged for this process only` |

`--tool-execution-mode` is absent from `droid exec --help` in 1.1.0 exactly as it was in 1.0.0
(a hidden flag, reference lines 514–516). It still works and still validates its argument:

```
error: option '--tool-execution-mode <mode>' argument 'bogus' is invalid. Expected direct-only or script-only
```

**Real way to force the tool-execution mode in 1.1.0:** the hidden
`--tool-execution-mode direct-only|script-only` flag (verified functional this session). It is
not necessary to use a `--settings` file, though `--settings <path>` also exists for a
per-process merge.

**Exact command shapes:**

```
droid exec [prompt]                                        # prompt as positional argument
droid exec -f task.md                                      # prompt from file
droid exec --model custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0 "do X"     # model + prompt
droid exec --auto high --model <alias> "do X"              # autonomy + model + prompt
droid exec --auto medium -o json --model <alias> "do X"    # output format
droid exec --tool-execution-mode direct-only --model <alias> "do X"    # force direct
droid exec --tool-execution-mode script-only --model <alias> "do X"    # force Script
```

## 3. Tool surface table

Chosen alias: `custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0` ("Codex via ChatGPT (gpt-6.1-sol)").

| Mode | How forced | `--list-tools` count | Tool names reported |
|---|---|---|---|
| default | no flag | **30** | base catalog (below) |
| direct-only | `--tool-execution-mode direct-only` | **30** | byte-identical to default |
| script-only | `--tool-execution-mode script-only` | **30** | byte-identical to default |

The 30 names (category headers shown as in the tool's own output):

- **Read** (17): DismissHandoffItems, EndFeatureRun, ExitSpecMode, FetchUrl, GenerateImage,
  Glob, Grep, ListAutomations, ListAutomationTemplates, LS, ProposeMission, Read,
  ReadAutomation, StageSettingsChanges, StartMissionRun, TodoWrite, WebSearch
- **Edit** (1): ApplyPatch
- **Execute** (12): CreateAutomation, DeleteAutomation, EditAutomation, Execute, GenerateDroid,
  Loop, RunAutomation, Skill, Task, TaskOutput, TaskStop, ToolSearch

`Script` and `WaitForScript` are **not** enumerated by `--list-tools` in any mode, consistent
with the 1.0.0 finding that they are injected at session scope only when the resolved mode is
`script_only` (reference line 3748). `--list-tools` therefore cannot distinguish the two modes;
it always prints the base catalog.

Byte size: `--list-tools` does **not** report a request-visible tool-schema byte size. The
`-o json` dump of the 30 tool definitions is **79,558 bytes** of JSON (measured file size), which
is a serialization of the `--list-tools` output, not the request tool-schema bytes. The request
tool-schema bytes (Script 29,728 / direct 69,689) require the mock capture and are `not
re-checked` here.

## 4. Account-scoped config (redacted `node effective-config.js`)

Run from `C:\Users\sergi\Desktop\lean-droid-1.0\droid-re-research\scripts`; writes
`evidence\effective-config.snapshot.json` and prints a curated subset. No identifiers are
retained by the script.

```
lastStartupVer  : 1.1.0

script_tool_models.models (n=12):
   1. gpt-6.1-sol        2. gpt-6-sol       3. gpt-6-astra      4. gpt-5.6-sol
   5. gpt-5.6-sol-fast   6. claude-opus-5-5 7. claude-opus-5-5-fast  8. claude-opus-5
   9. claude-opus-5-fast 10. claude-fable-5 11. claude-fable-5.1 12. kimi-k3

claim-relevant flags:
  mid_conversation_tool_changes        = true
  one_hour_anthropic_caching           = false
  adaptive_compaction_prompt           = true
  nested_task_delegation               = false
  execute_background_completion_wake   = true
  prompt_cache_key_handoff             = false
  mission_ledger                       = true
  shared_process_agents                = true
  daemon_cgroup_isolation              = false
  cli_rules_engine                     = true
  gpt_6_1_terra                        = false
  gpt_6_astra_ultrafast                = false
  droid_shield_2.0                     = false
  local_signals_inference_enabled      = true
  communication_prompt_v2              = true
  output_style_setting                 = true
  recall_tool                          = false
  session_index_search                 = false
  risk_assessment                      = false

nested_task_delegation_limits:
  {"max_depth":1,"max_active_descendants_per_root":8,"max_active_task_subagents_per_daemon":15}
```

The snapshot file this run wrote has `fileSize 13461`, `fileMtime 2026-10-31T18:45:06.824Z`,
`lastStartupVersion 1.1.0`, 19 flags, 12 Script models.

The 1.0.0 reference records the byte-for-byte identical allow-list, identical 19 flag values and
identical `{1, 8, 15}` delegation limits (reference lines 4989–5031). Nothing account-scoped
changed between 1.0.0 and 1.1.0 except `lastStartupVersion`.

## 5. Fact re-check table

Labels: `same as 1.0.0` / `changed` / `not re-checked`.

| # | Fact the plan relies on | Label | Evidence |
|---|---|---|---|
| 1 | `--tool-execution-mode` hidden flag exists | **same as 1.0.0** | accepted + validated: `--tool-execution-mode bogus` → `error: ... Expected direct-only or script-only`; `direct-only`/`script-only` run exit 0. Absent from help in both versions. |
| 2 | `--auto` levels | **same as 1.0.0** | 1.1.0 help: `--auto <level>  Autonomy level: low|medium|high`; DEFAULT (no flag) = read-only (1.0.0 `autonomyLevel off` ≈ no-flag read-only). |
| 3 | `--list-tools` availability | **same as 1.0.0** | present in help; exits before any model call; returns 30 tools for the GPT-family alias (1.0.0: 30 GPT / 31 Claude). |
| 4 | Script-mode tool count / bytes (5 / 29,728 B) | **not re-checked** | needs the mock capture (ch. 19 §2.1); mock suite forbidden (R3 — binds 8317 where CLIProxyAPI runs). |
| 5 | Direct-mode tool count / bytes (23 / 69,689 B) | **not re-checked** | needs the mock capture (ch. 19 §2.1); same R3 reason. |
| 6 | 3/6 repeat thresholds (nudge 3, stop 6) | **not re-checked** | runtime-proven only via mock (ch. 19 §5.1, X27); static constants not re-extracted from the 1.1.0 binary. R3. |
| 7 | 3 silent turns stop | **not re-checked** | runtime-proven only via mock (ch. 19 §5.2, X28). R3. |
| 8 | Background wait and kill | **not re-checked** | runtime-proven only via mock (ch. 19 §5.4, X25/X26/X30). R3. |
| 9 | Script allow-list (12 models) | **same as 1.0.0** | `effective-config.js` 1.1.0 list is byte-identical to 1.0.0's (reference lines 5013–5024). |
| 10 | Subagent limits (1 / 8 / 15) | **same as 1.0.0** | `nested_task_delegation_limits {max_depth:1, max_active_descendants_per_root:8, max_active_task_subagents_per_daemon:15}` identical (reference 5027–5031). |
| 11 | Compaction flags | **same as 1.0.0** | `adaptive_compaction_prompt=true`, `mid_conversation_tool_changes=true`; all 19 flags identical to 1.0.0 snapshot (reference 4989–5008). |
| 12 | Model alias resolution (`custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0`) | **same as 1.0.0** | alias resolves to "Codex via ChatGPT (gpt-6.1-sol)"; `--list-tools` works; settings.json still maps `baseUrl http://localhost:8317/v1`, `provider openai`. |
| 13 | Proxy endpoint as part of the system under test | **changed** | endpoint URL config unchanged (`http://localhost:8317/v1`), but liveness changed: 1.0.0 `connection refused` (ch. 16 §2.1) → 1.1.0 **listening** (`Test-NetConnection 127.0.0.1:8317` → `TcpTestSucceeded=True`). |

Additional identity deltas (not part of the 13 relied-on facts above, but recorded because
finding 1 flags them): product version `1.0.0 → 1.1.0`, build tag `0.244.0 → 1.1.0`, binary
size `326,704,352 → 327,162,080` B, SHA-256 changed.

## 6. Commands run

No provider request was made: only `--help`, `--version`, `--list-tools` (which exits before any
model call), file hashing, config reads, and one TCP port check. No prompt was ever passed to
`droid exec`, and the mock suite (`run-live-script-experiments.ps1`) and
`mock-responses-server.js` were **not** run.

```
git rev-parse --abbrev-ref HEAD                                   # J12003LPZ/droid-gap-stage0
C:\Users\sergi\bin\droid.exe --version                            # 1.1.0
C:\Users\sergi\bin\droid.exe exec --help
Get-FileHash C:\Users\sergi\bin\droid.exe -Algorithm SHA256       # 7F23DA2D...AA775
Get-AuthenticodeSignature C:\Users\sergi\bin\droid.exe            # Valid, SF AI Factory
Get-FileHash ...\CLIProxyAPI\cli-proxy-api.exe -Algorithm SHA256  # 11DC49F7...5029
# read-only: C:\Users\sergi\AppData\Local\CLIProxyAPI\config.yaml
# read-only: C:\Users\sergi\.factory\settings.json
# grep build tag: C:\Users\sergi\.factory\logs\droid-log-single.log
droid exec --model custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0 --list-tools                       # 30 tools
droid exec --model <alias> --tool-execution-mode direct-only --list-tools                       # 30 tools, exit 0
droid exec --model <alias> --tool-execution-mode script-only --list-tools                       # 30 tools, exit 0
droid exec --model <alias> --tool-execution-mode bogus --list-tools                             # error, exit 1
droid exec --model <alias> --list-tools -o json                                                 # 79,558-byte JSON
cd C:\Users\sergi\Desktop\lean-droid-1.0\droid-re-research\scripts; node effective-config.js
Test-NetConnection 127.0.0.1 -Port 8317                                                          # True
```

No settings file, CLIProxyAPI config, or `~\.factory` file was written.
