# Codemode

Codemode lets the model run a short, sandboxed JavaScript program that calls
your tools (`tools.<name>({...})`) and returns one small result, instead of
making many separate tool calls. It has two enabled modes: **read-only** (default when on) and **controlled**
(opt-in, currently behind its recovery release gate). A read-only script can
only use read-class tools; a controlled script may also edit files and run the
guarded shell. Every call still goes through the normal permission checks and
is recorded in the session's operation journal.

It is off by default.

## Turn it on

- **Interactive:** open `/config` and switch **Codemode** on. It applies from
  the next prompt. Switching it off removes the tool from later turns.
- **One run:** `davinci --codemode read-only ...` enables it for that run only.

`/config` stores `"codemode": {"mode": "read-only"}` (or `"off"`) in your user
settings (`<agent dir>/settings.json`). Project settings cannot turn Codemode
on or change its runtime.

## Controlled mode

`"codemode": {"mode": "controlled"}` selects controlled mode from trusted user
settings or `davinci --codemode controlled`. It is opt-in: nothing enables it
implicitly, a script header can never select it, and project settings cannot
turn Codemode on. The recovery-release gate is accepted
(`config::CONTROLLED_RECOVERY_ACCEPTED`); if the crash-recovery matrix ever
regresses, set it back to `false` and controlled mode fails closed with the
gate message instead of degrading to read-only.

Controlled scripts get the Droid spellings `tools.Read`, `tools.Grep`,
`tools.Glob`, `tools.LS`, `tools.Execute`, `tools.ApplyPatch`, `tools.Edit` and
`tools.Write`, each mapped onto an already-authorized tool. Raw multiline inputs
arrive as `inputs.name` from leading `// @input: name <<'TAG'` blocks;
`text()` and `await notify()` are the explicit output channels. Image output is
not yet supported (an OPEN parity gap) and fails with `UNSUPPORTED_OUTPUT`.

While controlled mode is on, the prompt carries a versioned Script reference
built from the aliases actually bound, and `wait_for_script` is offered so a run
can be inspected without re-running it. Both disappear when the mode is off.

## Background Scripts

With controlled mode on, a script that opens with
`// @script: {"startInBackground": true}` runs detached and returns
`{"state": "running", "toolCallId": "<handle>"}` immediately. The run is owned
by the session: its cancellation token is a child of the session token, and
session replacement, RPC `create_session` and exit paths drain open runs
(a launch during a drain is refused). `wait_for_script` accepts
`status`, a bounded `wait`, and `kill`. A finished detached run stores a bounded
summary (status, at most 16 scrubbed notifications of 4096 bytes, and the
retained Script log reference) in the same write that publishes its terminal
state. The next turn that starts after the run finishes is told once, as
persisted context appended after the existing history and labeled as untrusted
Script data that grants no authority; it is acknowledged only after it is
persisted, and a wakeup already in the session is not added again. An idle
session does not start a turn by itself, so a run that finishes while nothing is
running is reported only when the next turn begins. Polling with `status` or
`wait` is not counted as a stalled loop by the progress watchdog; `kill` is. A run that cannot be stopped
is recorded as `recoveryRequired`, never as completed or cancelled, and guest
JavaScript is never resumed after a crash.

`kill` cancels the run and waits a bounded grace; the worker records its own
terminal state, and a timeout gives `recoveryRequired`. A run owned by another
process returns `NOT_OWNER`. Liveness is a lock each process holds at
`<agent dir>/script-owners/<owner>.lock`, outside any run directory. A dead owner's open runs are reconciled at the
first registry open in a process, and again whenever a run in scope is looked up
or checked for a wakeup. A worker that panics leaves its run `recoveryRequired`
with a fixed summary, and a worker result that arrives after `recoveryRequired`
keeps that state and only adds its summary if none is stored. Handles are scoped to a session
branch: a run records the leaf it started from, and resolves only while that
entry is on the path to the current leaf. A run from another branch does not
resolve and its wakeup is not delivered there; navigating back to the branch
restores access, including after a restart.

## Function fallback (deferred gap)

The design asks to preserve a function-typed script fallback when the custom
freeform shape is rejected. Automatic retry is not implemented. Today the
fallback is the opt-in rollout flag (`PI_CODEX_APPLY_PATCH`, shared with
`apply_patch`): with it off, `codemode` is a function tool.
`script_wire_mode` and `script_wire_fallback` exist with no production caller,
because automatic fallback touches the request retry core and no real
rejection of the custom shape has been observed (the route accepted it,
`docs/cache/codex-backend-probe.md`). A captured real rejection would unblock
it.

## Rollback

Set the mode to `off` (or remove the `codemode` key). Later turns no longer
offer or run Codemode. Durable run records under
`<evidence dir>/script-runs` are left in place and are never replayed: an
unresolved mutation stays `recoveryRequired` and must be reconciled from the
operation journal. Evaluating controlled mode's performance is tracked in
`docs/superpowers/plans/2026-10-31-droid-parity-benchmark-implementation.md`.

## Requirements

- **A session.** Codemode records its operations in the session journal, so it
  is unavailable with `--no-session`.
- **The admitted runtime:** Node.js **24.21.0** exactly, plus the Codemode host
  bundle whose files match the manifest built into DaVinci
  (`crates/davinci-coding-agent/src/codemode_host/assets-manifest.json`).
  Rebuilding the bundle requires regenerating that manifest:
  `node crates/davinci-coding-agent/codemode-host/package-bundle.mjs NEW_DIR MANIFEST`,
  then copying `MANIFEST` over `assets-manifest.json` and rebuilding DaVinci.

## Runtime layout

When settings name no paths, DaVinci looks in your agent directory:

| Path | Contents |
| --- | --- |
| `<agent dir>/codemode/node/node.exe` (Windows) or `<agent dir>/codemode/node/bin/node` | Node.js 24.21.0 |
| `<agent dir>/codemode/host` | The Codemode host bundle |

The agent directory is `DAVINCI_CODING_AGENT_DIR` when set, otherwise
`~/.davinci/agent` when it exists, otherwise the legacy `~/.pi/agent`. A stored
login may still be read from `~/.pi/agent`, but settings and the Codemode
runtime use the agent directory. Every Codemode error names the path it checked.

To use other locations, add absolute paths next to the mode:

```json
{ "codemode": { "mode": "read-only", "nodePath": "D:/runtimes/node-24.21.0/node.exe", "hostPath": "D:/runtimes/codemode-host" } }
```

A runtime given with `nodePath`/`hostPath` must live outside the workspace you work in. The managed runtime in `<agent dir>/codemode` is DaVinci's own and is admitted even when the workspace contains it, as when you start DaVinci from your home directory.

## Install the runtime (Windows, Git Bash)

Run from a DaVinci source checkout at the same version as your installed binary,
so the bundle matches its manifest. Replace `AGENT` with your agent directory.

```bash
AGENT="$HOME/.davinci/agent"; [ -d "$AGENT" ] || AGENT="$HOME/.pi/agent"
mkdir -p "$AGENT/codemode" && cd "$AGENT/codemode"
curl -fsSLO https://nodejs.org/dist/v24.21.0/node-v24.21.0-win-x64.zip
curl -fsSLO https://nodejs.org/dist/v24.21.0/SHASUMS256.txt
grep ' node-v24.21.0-win-x64.zip$' SHASUMS256.txt | sha256sum -c -
unzip -q node-v24.21.0-win-x64.zip && mv node-v24.21.0-win-x64 node
rm node-v24.21.0-win-x64.zip SHASUMS256.txt
node/node.exe --version            # v24.21.0

cd <davinci checkout>/crates/davinci-coding-agent/codemode-host
npm ci --ignore-scripts --no-audit --fund=false
"$AGENT/codemode/node/node.exe" package-bundle.mjs "$(cygpath -w "$AGENT/codemode/host")" "$(cygpath -w "$AGENT/codemode/host-manifest.json")"
cmp "$AGENT/codemode/host-manifest.json" ../src/codemode_host/assets-manifest.json && echo "bundle matches"
```

On Linux and macOS use the matching `node-v24.21.0-<os>-<arch>.tar.xz` from
`https://nodejs.org/dist/v24.21.0/`, extract it to `<agent dir>/codemode/node`
(so `node/bin/node` exists), and drop the `cygpath -w` wrappers.

Rebuild the bundle after upgrading DaVinci if its manifest changed.

## How the model learns to use it

When Codemode is on, two things tell the model about it: the `codemode` tool
description (what it does, a worked example, when not to use it) and a short
paragraph in the per-turn `<runtime_state>` block saying to prefer one script
over several read/grep/find/ls calls. Neither appears when Codemode is off, so
the prompt is unchanged for sessions without it.

Without that text the model never chose the tool: 0 calls in 72 unsteered
benchmark sessions. `scripts/bench/codemode_adoption.py` measures it:
`run` asks six multi-lookup repo questions with Codemode on and off (no
appended prompt), and `report` grades every answer against facts read from the
repository and exits 1 when the Codemode-on arm falls below 50% adoption or 80%
correct. `report` and its tests are offline; `run` uses model usage. Every
`harness_stats` also carries `codemodeRuns`, `codemodeUnfinished` and
`codemodeEmptyResults`, and a top-level `codemode` block:
`{"mode": "off" | "read-only", "available": bool, "calls": n}`. `available` is
whether the model was offered the tool this run (a missing Node runtime leaves
it `false`), and `calls` counts every `codemode` dispatch, so `available: true`
with `calls: 0` means it was offered and never chosen. Compare pass rate, tokens and wall time against the
same arm with Codemode off before treating more calls as better.

## Messages

| Message | Meaning |
| --- | --- |
| `Codemode on: available from the next prompt` | `/config` switched it on and the runtime was admitted. |
| `Codemode stays off: Codemode Node runtime unavailable: no Node 24.21.0 at <path>` | No Node at that path. Install the runtime there, or set `nodePath`. |
| `Codemode stays off: Codemode runtime admission failed at <path>: Codemode requires the admitted Node 24.21.0 runtime` | Another Node version is at that path. |
| `Codemode stays off: Codemode host admission failed at <path>: ...` | The bundle there is missing or does not match this DaVinci's manifest. Rebuild it. |
| `Codemode is on in /config but stays off: ...` | Printed at startup when the switch is on but the runtime or session is unavailable; DaVinci starts without Codemode. |
| `Codemode records its operations in the session journal; remove --no-session to use it` | Codemode with `--no-session`. With `--codemode` this stops the run. |
