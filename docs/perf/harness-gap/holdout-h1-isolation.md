# H1 access isolation: agents cannot read the private suite

The transcript scan and the location check detect a leak after the fact. They are not access controls.
This adds one: the two agents run with a restricted copy of the operator's own token, so the operating
system refuses their reads of the suite, the reference solutions and every finished run directory.
No second account and no administrator rights are involved.

Scope: this changes how the agent processes are started. It does not change the tasks, the schedule,
the thresholds, the evaluator or the arms. Graders and checks keep the ordinary token.

## How it works

Windows checks every file access twice for a restricted token: once as usual, and once against the
token's restricting SIDs only. The agent token restricts to Users, Everyone and the logon session.

| Location | Grants Users or Everyone | Agent can read it |
|---|---|---|
| suite vault, results base | no (inheritance removed, operator and SYSTEM only) | no |
| operator profile (home, `.codex`, `.pi`, temp) | no | no |
| a finished run directory | no (grant revoked before the hidden tests are copied in) | no |
| the live run directory | yes, only while the agent runs | yes |
| the campaign agent directory (login copy, stand-in home, temp) | yes | yes |
| toolchain and pinned binaries | yes, read and execute, granted once | yes |

The logon-session SID is not for files. Without it the process cannot open its own desktop and a
program such as git dies with `0xC0000142`.

Steps the harness takes when `BENCH_AGENT_BOUNDARY=restricted-token`:

1. Before each agent starts, grant Users modify rights on that run directory only.
2. Start the agent suspended under the restricted token, inside the same job object as before.
3. When it exits, remove the grant and read the access list back. If a Users or Everyone entry
   survives, for example because the results base was not locked, the run fails.
4. Only then copy the hidden tests in and grade, as the operator.

The agent gets a stand-in home under the campaign agent directory (`home\AppData\Local\Temp`,
`home\.codex`) and, if `BENCH_AGENT_PATH` is set, only that PATH. The command is looked up on that
PATH, never on the operator's. The Codex arm reads its login from the stand-in `.codex`.

Not covered: this is not a defence against a Windows privilege escalation bug, the agent can use the
network, and a prompt that quotes a hidden test is the author's responsibility (the brief forbids
it). The agent still runs as the operator, so anything the ordinary `Users` or `Everyone` entries
expose elsewhere on the machine stays readable.

## Setup (operator, once; no administrator prompt)

```powershell
$b = "scripts\bench\agent_boundary.py"
# 1. lock the vault and the results base: operator and SYSTEM only
python $b lock C:\h1vault C:\h1results
# 2. let the agent run the toolchain and the pinned binaries (read and execute only)
python $b grant-read <python dir> <davinci checkpoint dir> <codex dir>
# 3. select the boundary and the agent PATH
$env:BENCH_AGENT_BOUNDARY = "restricted-token"
$env:BENCH_AGENT_PATH = "C:\Windows\System32;C:\Windows;<python dir>;<python Scripts dir>;C:\Program Files\Git\cmd"
```

Create the vault and results directories first, at a location outside your profile. A directory
created at the drive root inherits permissions for Users, which is why step 1 removes inheritance.

## Proof before any model run

```powershell
python $b selftest --vault C:\h1vault --results C:\h1results `
  --binary <davinci.exe> --binary <codex.exe> --tool python --tool pytest --tool git
```

This starts stub commands under the restricted token, inside the harness job object. Every check
must hold:

| Check | Required |
|---|---|
| read a file in the vault | denied |
| read a file in a finished run directory | denied |
| read a file in the operator profile | denied |
| list the results root | denied |
| write into a granted run directory | allowed |
| run python, pytest, git and each pinned binary with `--version` | allowed |
| read a run directory after its grant is revoked | denied |

A command that fails to launch never counts as a denial: it must start, run and exit nonzero. The
campaign manifest records `agent_boundary`. Run the self-test before each stage and keep its output
with the stage logs; a failing check blocks launch.

## Still unproven offline

Network and certificate loading in the DaVinci binary under this token. `--version` starts the binary
but does not open a TLS connection, and that needs a model call. If the first approved stage shows
a connection failure in the first run, it stops under the existing stop rules and nothing is
re-run to compensate.
