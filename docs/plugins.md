# Plugins

DaVinci loads plugins in the Claude Code and Codex format. A plugin can add
skills, slash commands, agent profiles, hooks and MCP servers. You can use
plugins that Claude Code or Codex already installed, or install new ones from
any Claude Code or Codex marketplace.

Design and exact behavior: `docs/superpowers/specs/2026-09-26-plugin-marketplace-design.md`.

## Quick start

Use what you already have:

```text
davinci plugin import            # list plugins installed by Claude Code / Codex
davinci plugin import --all      # adopt every plugin enabled there
```

Adopted plugins load from their original location and follow updates made by
Claude Code or Codex. `plugin uninstall` only stops DaVinci from loading them.

Install from a marketplace:

```text
davinci plugin marketplace add anthropics/claude-plugins-official
davinci plugin browse review
davinci plugin install code-review@claude-plugins-official
```

Marketplaces that Claude Code or Codex already cloned are browsable without
adding them. Installs are copied to `~/.davinci/agent/plugins/cache/`.

Every command also works inside a session as `/plugin …`. Changes take effect
in a new session or after `/reload`.

## Managing what is installed

In a session, `/plugin` with no arguments opens a manager with three tabs:
Plugins, Skills and MCP servers. `←`/`→` (or Tab) switch tabs, `↑`/`↓`
select, and the hint row names only the keys the selected row allows:

| Key | Plugins | Skills | MCP servers |
|---|---|---|---|
| enter | details | details | configuration |
| `u` | update from its marketplace | – | – |
| `e` | enable / disable | – | enable / disable (`"disabled": true`) |
| `a` then `y` | show the hook commands, then approve them | – | – |
| `r` | revoke hooks | – | – |
| `d` then `y` | uninstall | move to trash | remove from its `mcp.json` |

Delete and hook approval wait for `y`; any other key cancels. A held `d` or
`a` never confirms.

- Plugin skills, commands and agents change at once. A plugin's MCP servers
  and `SessionStart` hooks, and every MCP server change, apply in the next
  session. `u` (update) runs git in the foreground, so the screen waits
  for it.
- Plugins adopted from Claude Code or Codex are updated there. Deleting one
  only stops DaVinci loading it.
- Only skills in `~/.davinci/agent/skills/` and a trusted project's skill
  directory can be deleted. They are moved to
  `~/.davinci/agent/trash/skills/`, so you can move them back. The move is
  a rename: if it fails (a file in use, a project on another drive) nothing
  is removed. A skill folder that holds other skills cannot be deleted from
  here. Skills from plugins are managed through their plugin.
- An MCP server is edited in the file that defines it: the user `mcp.json` or
  a trusted project's. Servers from plugins are managed through their
  plugin. Editing rewrites the file as formatted JSON, through a symlink to
  its target, keeping its permissions.

## Hooks need your approval

Hooks are shell commands that run on your machine. A plugin's hooks stay off
until you approve them:

```text
davinci plugin info superpowers      # shows every hook command
davinci plugin approve superpowers
davinci plugin test superpowers      # runs its SessionStart hooks once
```

The approval covers the exact hook definitions and every file in the plugin.
If an update changes any of them, the hooks turn off and
`plugin list` shows `changed, approval needed`. `plugin revoke <plugin>` turns
them off again.

Supported events: `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PostToolUse`, `Stop` and `SessionEnd`. Other events are listed by
`plugin info` as not supported and never run. A hook can block a tool call
but can never approve one: DaVinci's permission mode still decides. Hooks do
not run in graph workers.

Hook commands run in Git Bash on Windows (set `DAVINCI_HOOK_BASH` to choose a
bash; without one, hooks do not run) and receive Claude Code's stdin JSON and
`CLAUDE_PLUGIN_ROOT`. Environment variables that look like credentials
(names containing `KEY`, `TOKEN`, `SECRET`, `PASSWORD`, `CREDENTIAL`, `AUTH`
and similar) are not passed to hooks.

## Names and precedence

- Skills, commands and agents keep their own names. A user or project
  definition with the same name wins over a plugin's.
- MCP servers are named `plugin_<plugin>_<server>`. An entry with the same
  name in your `mcp.json` wins.
- `/agents` lists plugin agents with the source `plugin`.

## Commands

```text
plugin [list]                          installed plugins
plugin browse [query]                  plugins in every known marketplace
plugin install <name>[@marketplace]    install and enable
plugin import [claude|codex] [<key>|--all]
plugin info <plugin>                   components, hooks and warnings
plugin approve | revoke <plugin>       hooks on or off
plugin test <plugin>                   run approved SessionStart hooks once
plugin enable | disable <plugin>
plugin update <plugin>                 reinstall from its marketplace
plugin uninstall <plugin>
plugin marketplace add <owner/repo | git-url | directory>
plugin marketplace list | update [name] | remove <name>
```

## Turning plugins off

`DAVINCI_PLUGINS=off` loads no plugin for that process. `plugin disable
<plugin>` turns one off persistently.

## Limits

- `UserPromptSubmit` hooks add context but cannot block a prompt.
- `SessionStart` hooks receive an empty `session_id`: the session file is
  created after resource discovery. Later events carry the real id.
- DaVinci does not list skills in the system prompt yet (neither plugin nor
  user skills), so the model does not pick one on its own. Invoke a plugin
  skill with `/skill:<name>`.
- `/plugin` runs in the foreground: `marketplace add` and `install` from git
  wait for the clone.
- Claude Code's `npm` plugin sources and non-command hook types (`prompt`)
  are not supported.
