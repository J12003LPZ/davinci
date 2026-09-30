# Make the repository checks the finish line

User hooks live in `~/.davinci/agent/hooks.json` or, for a trusted project,
`.davinci/hooks.json` (`.pi/hooks.json` remains supported). The `completion`
event runs when the model tries to finish each prompt, inside the agent loop.
The legacy `stop` event keeps its run-end meaning.

To require this Rust repository's formatting and agent tests before finishing,
save this script as `.davinci/finish-line.sh`:

```sh
#!/bin/sh
check_output=$(mktemp) || exit 1
trap 'rm -f "$check_output"' EXIT HUP INT TERM
if cargo fmt --all -- --check >"$check_output" 2>&1 &&
   cargo test -p davinci-agent --offline --locked >>"$check_output" 2>&1; then
    exit 0
fi
printf 'Repository checks failed. Fix the reported failures and rerun them.\n' >&2
tail -c 60000 "$check_output" >&2
exit 2
```

Add this rule to `.davinci/hooks.json`, keeping any existing rules:

```json
{
  "rules": [
    {
      "event": "completion",
      "action": ["sh", ".davinci/finish-line.sh"],
      "timeoutMs": 600000,
      "onFailure": "block"
    }
  ]
}
```

Run `sh .davinci/finish-line.sh` from the repository to check the script before
starting a new prompt. The rule runs from the same workspace directory. Replace
the two Cargo commands with your own repository's lint and test commands. On
Windows, use an installed `sh` (for example Git Bash) or an equivalent script
and argv for your local shell. Project hooks require project trust; use the
existing `/setup trust` flow. If you change `hooks.json` during a turn, the
loaded project hooks refuse to execute until they are loaded again.

For short commands, the argv-list form is also supported:

```json
{"completion": [["sh", ".davinci/finish-line.sh"]]}
```

That form has a 60-second timeout. Rules allow `timeoutMs` and obey the existing
`hookPolicy.enabled`, default timeout, recursion depth, and `onFailure` settings.

Exit 2 sends stderr back to the model as the reason to continue. Exit 0 with
`{"decision":"block","reason":"..."}` on stdout does the same. Other exit
codes, spawn errors, and timeouts produce non-blocking warnings. Hook output is
bounded to 64 KiB per stream. Hook checks are bounded feedback, so a timeout does
not prove checks passed.

User completion stdin contains `kind: "completion"` and the boolean
`stop_hook_active`. It is false on the first attempt and true on an attempt
after a hook block. An approved plugin's Claude Code `Stop` hook runs at this
same point and receives `hook_event_name: "Stop"` and `stop_hook_active`.
After three blocks in a prompt, the agent finishes with a visible
`completion.hook_limit` notice. Design hooks to give actionable feedback and
allow the next attempt when their condition is satisfied.

User `postTool` / `postToolFailure` and plugin `PostToolUse` hooks can return
exit 2 to append their stderr to the tool result as feedback. The tool's
success or error status is preserved. Other failures produce warnings and
allow the tool result through. Denied calls and calls blocked before execution
do not run these hooks.

All completion and post-tool hooks stay off in graph workers. Project trust
and integrity checks still apply, and plugin hooks require the current
`hooksApproved` digest, including files changed during the prompt. Each dispatch
also rereads the user installation record; revoking approval, disabling or
removing the plugin, or setting `DAVINCI_PLUGINS=off` stops already loaded hooks.
When execution sandboxing is enabled, plugin hooks refuse direct host execution
with a warning until their runner uses the sandbox executor transport. Hooks
never approve a tool call or bypass its permission gate.

The host fixtures cover allow/block/warning outputs, timeouts, continuation
flags, legacy stop separation, trust and digest changes, result feedback, and
the shared print/RPC/interactive completion path. Live model acceptance and
native Windows shell execution require separate campaign/platform evidence.

Requirement review is enabled by default. For the evaluation's baseline arm,
set `DAVINCI_REQUIREMENT_REVIEW=0`; `1` enables it for the treatment arm. The
user or trusted project setting `"requirementReview": false` also disables it,
and the environment overrides that setting. This switch does not disable
completion hooks or change the Stable prompt or provider tool schema. It makes
the existing Stable/Preview profile switch usable in a controlled A/B campaign.
