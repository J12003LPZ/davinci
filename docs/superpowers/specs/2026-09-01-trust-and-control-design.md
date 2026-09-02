# Trust and control — phase 2 design

Sub-project 2 of the competitive-harness roadmap
(`2026-09-01-competitive-harness-roadmap.md`). Phase 1 made turns real; this
phase decides *whether a tool may run at all*, and asks the user when the
answer is not already known.

## Why

TypeScript `pi` has no tool approval: once a project is trusted every tool
runs, in every mode. Claude Code and Codex CLI both ship a permission model
by default — read-only tools run freely, edits and shell commands are
approved once, per session, or forever, and non-interactive runs fail closed.
A harness that runs `rm -rf` because the model asked to is not competitive
on trust, however good its transcript looks.

There is no TypeScript source to mirror here. This is a documented divergence
(CLAUDE.md, *Conventions*): the module doc comments say so instead of citing
a `.ts` file.

## What ships

- **Permission modes** — `ask` (default), `edits`, `auto`, `read-only` — set
  by `--permission-mode`, by the Codex-flavoured `--sandbox` alias, by
  `permissions.mode` in settings, or by `/permissions <mode>` for the rest of
  the session.
- **A gate in the agent loop** that every tool call passes through, after the
  extension `tool_call` hook and before execution. It answers allow, deny or
  ask; ask goes to an *approver* the host installs.
- **Approval in the davinci language** — the one-question panel (`ask.rs`)
  wears `LICENTIA · PERMISSION`, mid-turn, with the call summarised on its
  note line and four rows: once, this session, always in this project, deny.
- **Rules** — `permissions.allow` / `permissions.deny` lists in the user file
  (`~/.pi/agent/settings.json`) and, when the project is trusted, the project
  file (`.pi/settings.json`). "Always allow in this project" writes the
  project file.
- **Fail closed elsewhere** — `--print`, `--mode json` and the legacy chrome
  deny what they cannot ask about and tell the model why; `--mode rpc` asks
  the client through the `extension_ui_request` channel it already speaks.

## Modes

| Mode | `read` `grep` `find` `ls` | `write` `edit` | `bash` `powershell` | other tools |
|---|---|---|---|---|
| `read-only` | allow | **deny** | **deny** | **deny** |
| `ask` (default) | allow | ask | ask | ask |
| `edits` | allow | allow inside the project, ask outside | ask | ask |
| `auto` | allow | allow | allow | allow |

"Other tools" are extension, native and MCP tools the built-in table does
not know. They are asked about in `ask` and `edits` because the harness
cannot tell a memory lookup from a deploy; a rule (`vector_search`) makes
one quiet for good.

`--sandbox read-only|workspace-write|full-access` maps to
`read-only|edits|auto`. It is a policy preset, not an OS sandbox; the help
text says so.

Precedence, highest first: `--permission-mode` / `--sandbox`;
`PI_PERMISSION_MODE` (fixture hook, tests only); `/permissions <mode>` for the
session; project `permissions.mode` (trusted projects only); user
`permissions.mode`; `ask`.

## Rules

A rule is a string: the tool name alone (`bash`, `edit`), or the tool name
with a pattern in parentheses (`bash(git *)`, `write(src/**)`). The pattern
is matched against the call's *subject*:

| Tool | Subject |
|---|---|
| `bash`, `powershell` | the `command` string, trimmed |
| `read`, `write`, `edit`, `ls`, `find`, `grep` | the `path` argument, normalised to forward slashes and made relative to the project when it is inside it |
| anything else | the tool name only; a pattern never matches |

Patterns are globs with `*` (any run, including `/`), `**` (same), and `?`.
No dependency: a forty-line matcher in `pi-agent`. Matching is exact-case on
Unix, case-insensitive on Windows, like the file systems are.

Decision order for a call in mode *m*:

1. A matching `deny` rule → **deny**. Deny always wins.
2. `m == auto` → allow.
3. A matching `allow` rule → allow.
4. `m == read-only` and the tool is not a read tool → deny.
5. The mode table above.

Allow rules come from three places and are unioned: the user file, the
project file (only when the project is trusted — reading
`permissions.allow` from an untrusted checkout would let a repository grant
itself the shell), and rules added during the session by "allow for this
session". Deny rules likewise, minus the session source.

A session rule for a `bash` call is derived from the command: the program
and its first argument when the program is a bare name and the first
argument does not start with `-`, look like a path, or is a shell operator
(`git status --short` → `bash(git status *)`; `cargo test -p x` →
`bash(cargo test *)`; `rm -rf build` → `bash(rm *)`; `./run.sh now` →
`bash(./run.sh *)`; `ls && rm x` → `bash(ls *)`). A rule ending in ` *`
also matches the bare prefix, so `bash(git status *)` covers `git status`. For `write` and `edit` the rule is the bare tool name;
path rules are for hand-written settings. For every other tool it is the
bare tool name.

## The gate

`pi-agent/src/permission.rs` is currently dead code (no `mod` declares it)
and carries a wrong TS citation. It is rewritten and declared:

```rust
pub enum PermissionMode { ReadOnly, Ask, Edits, Auto }

pub struct PermissionPolicy {
    pub mode: PermissionMode,
    pub allow: Vec<PermissionRule>,   // user + project, in that order
    pub deny: Vec<PermissionRule>,
    pub session_allow: Vec<PermissionRule>,
}

pub enum PermissionVerdict {
    Allow,
    Deny { reason: String },
    Ask(ToolApprovalRequest),
}

pub struct ToolApprovalRequest {
    pub tool_call_id: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub subject: String,           // what the rule would match
    pub summary: String,           // one line for the note: `bash · git status`
    pub session_rule: String,      // what "this session" / "always" would add
    pub outside_project: bool,     // path rules: the target is not under cwd
}

pub enum ToolApprovalDecision {
    AllowOnce,
    AllowForSession,   // the gate appends `session_rule` to `session_allow`
    AllowAlways,       // the approver has already persisted the rule; the gate also appends it
    Deny,
}

pub struct ToolApprover(pub Arc<dyn Fn(&ToolApprovalRequest) -> ToolApprovalDecision + Send + Sync>);
```

`Agent` gains `permissions: Arc<Mutex<PermissionPolicy>>` (the worker thread
holds `&mut Agent` while the UI thread wants to read the mode for the status
line, and the gate runs from `&self`) and `approver: Option<ToolApprover>`.

`Agent::execute_one` becomes: emit `ToolExecutionStart` → extension
`pre_tool` hook (a block wins, as today) → **gate** → execute → `post_tool`.
`Ask` with no approver, or an approver answering `Deny`, produces an error
tool result the model can act on:

```
Permission denied: `bash` is not allowed to run `git push` in permission mode
`ask`. The user has to approve it; in a non-interactive run start pi with
--permission-mode auto or add an allow rule to ~/.pi/agent/settings.json.
```

(The second sentence is dropped when a user denied it interactively: "The
user declined." replaces it.) No new `AgentEvent` variants: the ledger row
already exists from `ToolExecutionStart`, and the davinci approver has the
model in hand to dress it while it waits. JSON and RPC consumers see the
existing `tool_execution_end` with `is_error: true`.

Parallel tool execution (`ToolExecutionMode::Parallel`) serialises through
the gate: one question at a time is the panel's contract, and the approver
is called under the policy lock so the answer to the first call can quiet
the second.

## Surfaces

### davinci (`davinci_session.rs`)

The approver installed for a turn sends `(request, reply_tx)` over a channel
to `run_turn`'s loop and blocks on `reply_rx.recv()`. The loop, on receiving
one:

- dresses the ledger row for the call as `◉ bash · git status · awaiting
  approval` (the Studio step already exists; only its detail text changes);
- builds the `Ask` (`Question::Permission`), opens `Overlay::Ask`, and keeps
  the working line running — the clock is still the turn's;
- routes keys through `pi_tui::davinci::app::handle_key` while the overlay
  is up, so ↑↓/enter/esc behave as in every other panel. `Flow::Choose(
  Choice::Ask(i))` answers; esc (`Flow::Continue` with the overlay gone)
  answers **deny**; ctrl+c / a second esc raises the abort flag and answers
  deny, so the worker returns promptly;
- persists "always" through `crate::permissions::remember_project_rule`
  before replying, and notes `remembered bash(git status *) in
  .pi/settings.json` in the transcript.

Panel copy (design.md §5: Latin name beside the plain term, body plain):

```
╭─ LICENTIA · PERMISSION ───────────────────────────── /permissions ─╮
│ ◉ allow once              runs this call only                      │
│ ○ allow for this session  bash(git status *) until pi exits        │
│ ○ always allow here       bash(git status *) saved to .pi/settings │
│ ○ deny                    the model is told no                     │
│                                                                    │
│ bash · git status                                                  │
│ ↑↓ move · enter select · esc close                                 │
╰────────────────────────────────────────────────────────────────────╯
```

The "always allow here" row is omitted when the project is not trusted
(the file would never be read back) and a note says why on first omission.
For `write`/`edit` the note carries the path and, for a target outside the
project, `· outside the project`. For a long shell command the note is
clipped with `…` at the panel width; the full command is in the ledger.

`/permissions` with no argument says the mode and lists the rules in force,
by source. `/permissions <mode>` switches the session mode and says so.
Both are davinci-only commands like `/diff` (Instrumenta lists them). The
status bar's mode word is unchanged: it says which screen is up, not the
permission mode — the header's right run gains ` · auto` only when the mode
is `auto`, the one state worth a permanent reminder.

### `--print`, `--mode json`

No approver. `Ask` → deny with the non-interactive message. Nothing else
changes: the exit code follows the reply as before, and a denied tool is not
a provider error.

### `--mode rpc`

The approver asks the client with the `select` request the JS UI waiter
already emits (`rpc_emit_and_wait_ui`): title `Allow bash?`, message = the
summary, options `["allow once", "allow for this session", "always allow in
this project", "deny"]`. The client's `extension_ui_response.value` is
matched by text; anything else, a timeout, or a closed pipe is deny. A
client that predates this sees an `extension_ui_request` it can already
render.

### Legacy chrome (`--legacy-tui`)

The approver uses the existing `open_extension_confirm` dialog:
`Allow bash?` / the summary, yes = allow once, no = deny. No session or
always rows; the legacy chrome is the fallback surface, not the product.

## Settings and flags

`settings.rs`:

```rust
#[serde(default)]
pub permissions: Option<PermissionSettings>,

pub struct PermissionSettings {
    #[serde(default)] pub mode: Option<String>,
    #[serde(default)] pub allow: Vec<String>,
    #[serde(default)] pub deny: Vec<String>,
}
```

`deep_merge_json` replaces arrays, so the project file's `allow` would hide
the user's. A new `crate::permissions::policy_for(agent_dir, cwd, override)`
loads the two files separately and unions them; project rules are read only
when `trust::resolve_project_trusted` says so. `remember_project_rule`
appends to `.pi/settings.json` under the settings lock, creating the file
with only the `permissions` key when it does not exist, and keeps unknown
keys as `save_settings` does.

`args.rs`: `--permission-mode <read-only|ask|edits|auto>` and `--sandbox
<read-only|workspace-write|full-access>`; both set
`Args::permission_mode: Option<PermissionMode>`. `help.txt` documents both
under the tool flags, with a one-line note that `--sandbox` is a policy
preset. `PI_PERMISSION_MODE` is honoured only when neither flag is given,
so fixtures can pin a mode without touching the argument list.

`build_agent` sets `agent.permissions` from `policy_for` and the flags.

## Trust, not permissions

Project trust (`/trust`, `trust.json`) stays what it is: whether `.pi`
resources load. It now also decides whether `.pi/settings.json` may grant
permissions, which is the same question. Nothing in this phase changes the
trust prompt.

## Diagnostics

`PI_AI_TRACE` is a provider trace, and under the davinci panel it also logs
each key and the decision it produced. Permission decisions are visible in
the transcript. For `--print`, `--verbose` prints one stderr line per deny:
`pi: denied bash (no approver in a --print run)`.

`PI_OFFLINE_TOOL_CALL='{"name":"bash","arguments":{"command":"git status"}}'`
with `PI_OFFLINE=1` makes the first reply of a turn that tool call (the
reply after the tool result is the usual offline stub), so the whole tool
path — gate, panel, result — can be driven headlessly with no provider.

## Testing

Fixture-only, as everything else.

- `pi-agent/permission.rs`: matcher (glob cases, Windows case folding, path
  normalisation), decision order for every mode × tool class, deny-wins,
  session rule derivation (`git status`, `cargo test -p x`, `rm -rf`,
  `./run.sh`, a quoted program), `AllowForSession` appending to
  `session_allow` and quieting the next identical call.
- `pi-agent/turn.rs`: a run with an approver that counts calls — `ask` mode
  asks once for `bash`, never for `read`; `read-only` denies `write` without
  asking; a `Deny` answer produces the error result and the loop continues;
  no approver in `ask` mode denies with the non-interactive message.
- `pi-coding-agent/permissions.rs`: `policy_for` unions user and project
  rules; ignores project rules when untrusted; `remember_project_rule`
  creates and appends without disturbing other keys; mode precedence.
- `args.rs`: both flags parse, `--sandbox` maps, bad values are diagnostics.
- `davinci_session.rs`: the `Ask` built from a request has four rows (three
  when untrusted); esc denies; `Choice::Ask(1)` replies `AllowForSession`;
  the ledger row reads `awaiting approval` while pending and its final
  state after. The headless ConPTY harness (memory note) is used once by
  hand to see the panel mid-turn; not automated.
- `rpc.rs` / `main.rs`: the RPC approver emits a `select` and maps the
  four answers; an unmatched answer denies.
- `settings.rs`: `permissions` round-trips and unknown keys survive.

## Out of scope

OS-level sandboxing (seatbelt, landlock, job objects); a `/permissions`
sheet with its own mockup (the note is enough until there is a screen for
it); per-tool rules for extension arguments; hooks that answer permission
questions (phase 6); plan mode (phase 5); bash command parsing beyond the
first two words (no `&&`/`;` splitting — a compound command derives its rule
from its first program, and the user sees the whole command before allowing
it).
