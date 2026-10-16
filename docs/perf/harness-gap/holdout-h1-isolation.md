# H1 access isolation: agents cannot read the private suite

The transcript scan and the location check detect a leak after the fact. They are not access controls.
This adds one: the two agents run under a separate, unprivileged Windows account, so the operating
system refuses their reads of the suite, the reference solutions and every finished run directory.

Scope: this changes who the agent processes run as. It does not change the tasks, the schedule, the
thresholds, the evaluator or the arms. Graders and checks still run as the operator.

## The boundary

| Account | Runs | Can read |
|---|---|---|
| operator (your account) | harness, grading, evaluator | vault, results root, everything |
| agent account (`h1agent`) | DaVinci and Codex processes only | its own run directory while the agent runs, the campaign agent directory, granted tool and binary directories |

Enforced by file permissions:

1. The suite vault and the results base have inheritance removed and grant only the operator and
   SYSTEM. The vault also carries an explicit deny for the agent account.
2. Before each agent starts, the harness grants the agent account modify rights on that run
   directory only. The grant is removed when the agent exits, before the hidden tests are copied in.
   If the entry survives, the run fails. Finished run directories, which hold the hidden tests,
   are therefore unreadable to every later agent.
3. The agent gets its own profile variables and, if `BENCH_AGENT_PATH` is set, a PATH limited to the
   granted tool directories. Nothing points into the operator's profile.

Not covered: the account is not a sandbox against a kernel or privilege escalation bug, the agent can
use the network, and a task prompt that quotes a hidden test is the author's responsibility
(the brief forbids it). Anything the agent writes into its own run directory is visible to the operator.

## Setup (operator, once; steps 1 and 2 need an administrator prompt)

```powershell
# 1. create the account, no admin rights; choose a long random password
net user h1agent * /add /expires:never /passwordchg:no
# 2. keep it out of the sign-in screen
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList" /v h1agent /t REG_DWORD /d 0 /f
# 3. store the password for this operator account only (DPAPI), outside any repository
$env:BENCH_AGENT_USER = "h1agent"
python scripts\bench\agent_account.py store-credential --file "$env:LOCALAPPDATA\h1\agent.cred"
# 4. lock the vault and the results base; the vault also gets an explicit deny
python scripts\bench\agent_account.py lock --deny-agent C:\h1vault
python scripts\bench\agent_account.py lock C:\h1results
# 5. let the agent account use the toolchain and the pinned binaries, read and execute only
python scripts\bench\agent_account.py grant-read <python dir> <python Scripts dir> <git dir> <davinci checkpoint dir> <codex dir>
$env:BENCH_AGENT_CREDENTIAL = "$env:LOCALAPPDATA\h1\agent.cred"
$env:BENCH_AGENT_PATH = "<system32>;<python dir>;<python Scripts dir>;<git cmd dir>"
```

The agent account needs its own Codex sign-in (Codex reads the profile it runs in). DaVinci needs none:
the harness copies the operator's login into the per-campaign agent directory and grants only that
directory to the agent account.

## Proof before any model run

```powershell
python scripts\bench\agent_account.py selftest --vault C:\h1vault --results C:\h1results `
  --binary <davinci.exe> --binary <codex.exe> --tool python --tool pytest --tool git
```

This starts stub commands under the agent account, inside the same job object the harness uses, and
requires every one of these to hold:

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
campaign manifest records `agent_account`. Run the self-test before each stage and keep its output
with the stage logs; a failing check blocks launch.
