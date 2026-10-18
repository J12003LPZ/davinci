# Project trust

A project folder can ship configuration that runs code or changes the agent's
prompt and permissions: hooks, MCP servers, extensions, skills, prompts,
agents, packages and project settings (`.davinci/` or `.pi/`, plus
`.agents/skills`). DaVinci asks before loading any of it. Trusting a folder only
lets DaVinci load that configuration. It does not approve a tool call, start a
service, install software or relax the sandbox.

## Interactive launch

When a folder has such resources and no decision is stored, DaVinci asks before
the session directory, migrations, MCP connection, hooks, extensions and skills
are touched:

```
Do you trust the files in this folder?
  C:\work\repo

It has project configuration that can run code or change prompts: hooks.json, mcp.json.
Trusting only lets DaVinci load it. Tool approvals and the sandbox stay on.

> 1. No — Exit DaVinci
  2. Yes, I trust this folder
```

- `No` is the default (Enter, `1`, `n`, Esc, Ctrl+C, closed input). It stores
  "Do not trust" for the canonical folder path and exits with status 0.
- `Yes` stores trust and continues.
- A stored decision is never asked again. A stored denial keeps the project's
  resources unloaded on later launches (DaVinci still opens, without them);
  `/trust` and `/setup trust` change the decision.
- A nearer entry wins over a parent's: a saved "Do not trust" on a folder
  survives "Trust parent folder".
- Symlinks, junctions and the Windows drive-letter case resolve to one entry.
- An unreadable or malformed `trust.json` fails closed. DaVinci reports the file
  and the error and leaves it alone. `/setup` shows the same error.

## Headless launches (`--print`, `--mode json`, `--mode rpc`, the SDK)

There is nobody to ask, so nothing is prompted and nothing is stored. Project
resources load only when one of these host-owned inputs allows it:

1. a stored grant in the agent directory's `trust.json`;
2. `--approve` on the command line (`--no-approve` forces the opposite);
3. the user's **global** settings: `trustedProjects` lists the folder, or
   `defaultProjectTrust` is `"always"`. This is the trusted-host override. A
   project's own settings cannot set either, and a stored denial beats both.

Otherwise the project's resources stay unloaded and the caller is told, with the
stable code `UNTRUSTED_PROJECT`:

- print / json mode: a `Warning: UNTRUSTED_PROJECT: ...` line on stderr;
- RPC: an `extension_ui_request` `notify` event with `notifyType: "warning"`,
  `code: "UNTRUSTED_PROJECT"`, `path` and `diagnostic`, and `get_state` reports
  `projectTrusted` and `projectTrustCode`;
- SDK: `CreateAgentSessionResult::project_trust_diagnostic`.

## Packages and graph verification

`davinci update` skips an untrusted project's packages and says so with the same
code. `davinci install -l` refuses an untrusted project. A graph run in an
untrusted project gets no `verifyCommands` and no commands inferred from
`Cargo.toml` or `package.json`.

## Rollback and opt-out

Revert the phase 1 commits to return to the previous behaviour (silent
filtering, `/trust` only). To skip the question for a folder you control, run
`/setup trust` there once, pass `--approve`, or list it under `trustedProjects`
in the global settings.
