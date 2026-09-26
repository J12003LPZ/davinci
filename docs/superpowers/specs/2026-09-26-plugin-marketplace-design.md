# Plugin marketplace (Claude Code and Codex compatible)

Status: implemented in `crates/davinci-coding-agent/src/plugins/`.
No TypeScript counterpart. This is a DaVinci-specific divergence from `pi`.

## Goal

Let DaVinci use the plugin ecosystems of Claude Code and Codex rather than
building a new one. A plugin is a directory that may contain skills, slash
commands, agents, hooks and MCP servers. DaVinci must:

1. Install plugins from marketplaces itself (no dependency on Claude Code or
   Codex being installed).
2. Adopt plugins that Claude Code or Codex already installed, loading them in
   place, so existing setups work immediately.
3. Never run a plugin's shell hooks until the user has approved exactly those
   hooks.

## Decisions

| Question | Decision |
|---|---|
| Components | Everything: skills, commands, agents, hooks, MCP servers. |
| Source | Native marketplace core plus import of Claude/Codex installs. |
| Hook trust | Ask once per plugin. Approval records a SHA-256 digest of the hook definitions and the whole plugin tree (except `.git`; `node_modules` and files over 8 MiB by size and mtime). A hook runs through a shell and can reach any plugin file, so no subset is safe to leave out. Any change disables the hooks until re-approved. The digest is computed once per process per plugin root; `plugin approve` always hashes afresh. |
| Plugin names | Skills, commands and agents keep their own names. A user or project definition with the same name wins. |
| MCP names | `plugin_<plugin>_<server>`, so tool names match Claude Code (`mcp__plugin_<plugin>_<server>__<tool>`). User and project `mcp.json` entries win. |
| Kill switch | `DAVINCI_PLUGINS=off` loads no plugins. Graph workers (`PI_GRAPH_ROLE`) never run plugin hooks. |

## Storage

All state lives under `<agent_dir>/plugins/` (`~/.davinci/agent/plugins/`):

```text
installed.json            enabled plugins, origin, approval digests
marketplaces.json         marketplaces added with `plugin marketplace add`
marketplaces/<name>/      marketplace clones or copies
cache/<mkt>/<plugin>/     plugins installed by DaVinci
data/<plugin>/            CLAUDE_PLUGIN_DATA for each plugin
```

An `installed.json` entry has an `origin`:

- `davinci`: installed by DaVinci; `installPath` points into `cache/`.
- `claude`: adopted; the path is resolved on every load from
  `$CLAUDE_CONFIG_DIR/plugins/installed_plugins.json` (default `~/.claude`),
  so Claude Code updates are followed.
- `codex`: adopted; resolved from `$CODEX_HOME/plugins/cache/<mkt>/<plugin>/`
  (newest version directory; default `~/.codex`).

Marketplace catalogs are read from DaVinci's own registry, from Claude Code's
`known_marketplaces.json` install locations, and from Codex local marketplaces
in `config.toml`. DaVinci's registry wins on a name conflict. Catalogs are read
only; installing always copies into DaVinci's cache.

## Plugin layout

The manifest is `.claude-plugin/plugin.json`, else `.codex-plugin/plugin.json`,
else `plugin.json`. A plugin without a manifest is named after its directory.
Default component locations are `skills/<name>/SKILL.md`, `commands/*.md`,
`agents/*.md`, `hooks/hooks.json` and `.mcp.json`. Manifest fields `skills`,
`commands`, `agents`, `hooks` and `mcpServers` add paths (string or array) or
inline objects. Every path must resolve inside the plugin root.

Marketplace catalogs are `.claude-plugin/marketplace.json`,
`.agents/plugins/marketplace.json` or `marketplace.json`. Supported entry
sources: relative path, `{source: "local", path}`, `{source: "github", repo,
ref?}`, `{source: "url" | "git", url, ref?}` and `{source: "git-subdir", url,
path, ref?}`. When the plugin directory has no manifest (`strict: false`
entries), the marketplace entry is written as its manifest.

## Runtime integration

- **Skills**: each `SKILL.md` is passed to `discover_skills` as a file root,
  so reference documents beside it are not mistaken for skills. Respects
  `--no-skills`.
- **Commands**: command files are prompt-template roots (`$ARGUMENTS` is
  already supported). Respects `--no-prompt-templates`.
- **Agents**: parsed with `AgentProfile::parse`. Claude tool names map to
  DaVinci names (`Read`→`read`, `Glob`→`find`, `Bash`→`bash`…). Short model
  aliases (`sonnet`, `opus`) become `inherit`. A profile that lists write or
  edit tools gets `edits`, otherwise `read-only`; containment against the
  parent still applies.
- **MCP**: plugin servers are the base layer under user and project
  `mcp.json`. `${CLAUDE_PLUGIN_ROOT}`, `${DAVINCI_PLUGIN_ROOT}` and
  `${CLAUDE_PLUGIN_DATA}` are expanded.
- **Hooks** (approved plugins only):

| Claude event | DaVinci point | Effect |
|---|---|---|
| `SessionStart` | resource discovery, once per process and cwd | stdout or `additionalContext` becomes a context file |
| `UserPromptSubmit` | before each prompt, next to memory injection | stdout or `additionalContext` becomes turn context |
| `PreToolUse` | `Agent.pre_tool` | exit 2, `decision: block` or `permissionDecision: deny` blocks the call |
| `PostToolUse` | `Agent.post_tool` | `additionalContext` or a block reason is appended to the tool result |
| `Stop` | end of each prompt | observe only |
| `SessionEnd` | where user `stop` hooks run | observe only |

Other events (`Notification`, `SubagentStop`, `PreCompact`, …) and non-command
hook types are listed as unsupported by `plugin info` and never run. A hook
`permissionDecision: allow` never bypasses DaVinci's permission gate.
`UserPromptSubmit` blocking is not supported; the output is used as context
only.

Hook commands run through `bash -c` (Git Bash on Windows, never the WSL
`System32\bash.exe`) or PowerShell when `shell: "powershell"`. When no bash
exists the hook does not run and a warning names `DAVINCI_HOOK_BASH`; `cmd`
would parse bash syntax differently. Hooks get Claude's stdin JSON
(`session_id`, `cwd`, `hook_event_name`, `tool_name`, `tool_input`,
`tool_response`, `prompt`, `source`) and `CLAUDE_PLUGIN_ROOT`,
`CLAUDE_PLUGIN_DATA`, `CLAUDE_PROJECT_DIR` and `DAVINCI_PLUGIN_ROOT`.
Environment variables whose names contain `KEY`, `TOKEN`, `SECRET`,
`PASSWORD`, `PASSWD`, `CREDENTIAL`, `_PAT`, `PRIVATE`, `COOKIE`, `NETRC` or
`AUTH` are removed (except `SSH_AUTH_SOCK`).

Git sources from catalogs accept only `https`, `http`, `ssh` and `git`
transports (`GIT_ALLOW_PROTOCOL` is set for every git call), so a catalog
cannot use `ext::`, `fd::` or `file://` remotes. Default timeout 60 s,
output capped at 64 KiB; a timeout or crash is a non-blocking warning, as in
Claude Code. `async: true` hooks run in the background.

## Commands

`davinci plugin …` on the command line and `/plugin …` in a session:

```text
plugin [list]                       installed plugins and their state
plugin browse [query]               catalog across all marketplaces
plugin install <name>[@mkt]         copy into DaVinci's cache and enable
plugin import [claude|codex] [key|--all]
                                    list or adopt Claude Code / Codex installs
plugin info <key>                   components, hooks, warnings
plugin approve <key> | revoke <key> approve or revoke the listed hooks
plugin enable <key> | disable <key>
plugin update <key>                 reinstall from the marketplace
plugin uninstall <key>
plugin marketplace add <owner/repo | git-url | dir>
plugin marketplace list | update [name] | remove <name>
```

Changes apply to new sessions or after `/reload`.

## Testing

Inline unit tests use temporary directories with `CLAUDE_CONFIG_DIR`,
`CODEX_HOME` and a temporary agent directory. They cover manifest discovery,
path escape rejection, marketplace source parsing, install from a local
marketplace, Claude/Codex adoption, hook digest invalidation, hook output
parsing, matcher and tool-name mapping. No test touches the network.
