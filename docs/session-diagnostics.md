# Session status and diagnostics

Use `/status` for the model, permission mode, jobs, foreground and background
usage, sandbox enforcement, and the native services. Its sections include the
repository index, cache, test impact, Git, change impact, verification planner,
workspace checkpoints, package/build intelligence, LSP, memory, governor,
learning, and hooks. Their former `*-status` names are hidden from slash
discovery; internal handlers remain available to RPC and sheets.

Use `/doctor` to check settings, model and MCP configuration, credential
presence, sandbox enforcement, observed MCP/LSP health, and install identity.
Invalid configuration is reported without echoing its values. The credential
check reports presence only: it neither executes helpers nor refreshes or tests
credentials. MCP/LSP checks use existing runtime state and do not start servers
or connections. A service that has not started has an empty observed session list; configuration alone does not establish health.

The install check reads the running executable's sibling
`davinci.identity.json` (`davinci.exe.identity.json` on Windows). Schema 3 records
source/release provenance and the binary SHA-256. The doctor computes the
running file's hash and compares it with that record; absent metadata leaves
source provenance unknown, and a mismatch makes the identity invalid. Matching
bytes still leave release provenance unverified: the doctor does not validate
the recorded release tag or CI proof independently.

`/cost` and RPC `get_session_stats.background` separate learning review and
security watch from foreground usage. Their counters cover receipts observed
for the originating session in the current process. Running requests and
missing usage or prices remain unknown, with partial measurements available.
They do not restore historical background spend from session files. See
[background cost visibility](learning.md#background-cost-visibility) and the
[security watch](security-scan.md#session-security-watch).
