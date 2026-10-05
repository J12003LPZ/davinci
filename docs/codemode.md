# Codemode

Codemode lets the model run a short, sandboxed JavaScript program that calls
your tools (`tools.<name>({...})`) and returns one small result, instead of
making many separate tool calls. It is **read-only**: scripts can only use
read-class tools, and every call still goes through the normal permission
checks and is recorded in the session's operation journal.

It is off by default.

## Turn it on

- **Interactive:** open `/config` and switch **Codemode** on. It applies from
  the next prompt. Switching it off removes the tool from later turns.
- **One run:** `davinci --codemode read-only ...` enables it for that run only.

`/config` stores `"codemode": {"mode": "read-only"}` (or `"off"`) in your user
settings (`<agent dir>/settings.json`). Project settings cannot turn Codemode
on or change its runtime.

## Requirements

- **A session.** Codemode records its operations in the session journal, so it
  is unavailable with `--no-session`.
- **The admitted runtime:** Node.js **24.21.0** exactly, plus the Codemode host
  bundle whose files match the manifest built into DaVinci
  (`crates/davinci-coding-agent/src/codemode_host/assets-manifest.json`).

## Runtime layout

When settings name no paths, DaVinci looks in your agent directory:

| Path | Contents |
| --- | --- |
| `<agent dir>/codemode/node/node.exe` (Windows) or `<agent dir>/codemode/node/bin/node` | Node.js 24.21.0 |
| `<agent dir>/codemode/host` | The Codemode host bundle |

The agent directory is `~/.davinci/agent` (or the legacy `~/.pi/agent` when that
is the one in use, or `DAVINCI_CODING_AGENT_DIR`).

To use other locations, add absolute paths next to the mode:

```json
{ "codemode": { "mode": "read-only", "nodePath": "D:/runtimes/node-24.21.0/node.exe", "hostPath": "D:/runtimes/codemode-host" } }
```

The runtime must live outside the workspace you work in.

## Install the runtime (Windows, Git Bash)

Run from a DaVinci source checkout at the same version as your installed binary,
so the bundle matches its manifest. Replace `AGENT` with your agent directory.

```bash
AGENT="$HOME/.pi/agent"            # or ~/.davinci/agent
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

## Messages

| Message | Meaning |
| --- | --- |
| `Codemode on: available from the next prompt` | `/config` switched it on and the runtime was admitted. |
| `Codemode stays off: Codemode Node runtime unavailable` | No Node at the configured or managed path. Install the runtime. |
| `Codemode stays off: Codemode runtime admission failed: Codemode requires the admitted Node 24.21.0 runtime` | Another Node version is configured. |
| `Codemode stays off: Codemode host admission failed: ...` | The bundle is missing or does not match this DaVinci's manifest. Rebuild it. |
| `Codemode is on in /config but stays off: ...` | Printed at startup when the switch is on but the runtime or session is unavailable; DaVinci starts without Codemode. |
| `Codemode records its operations in the session journal; remove --no-session to use it` | Codemode with `--no-session`. With `--codemode` this stops the run. |
