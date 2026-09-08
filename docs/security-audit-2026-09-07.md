# Security audit and repair record — 2026-09-07

Repository: `C:\Users\sergi\Desktop\pi-rust`.
Baseline: `main` at `b875592`. Changes are uncommitted.
This is a bounded audit and repair pass, not a certification that all bugs or security issues are fixed.

The pre-existing untracked `plugins/` directory was not edited. Active changes are confined to eleven Rust source files, Cargo.toml, Cargo.lock, and this new report. No vendor/davinci or stale packages/ sources were changed. No real credentials or sessions were inspected; fixtures use temporary or isolated audit directories. No commits, pushes, branch switches, system installers, or live-provider test calls were performed.

## Implemented fixes

### 1. Shell permission-policy bypasses and Unicode panic

File: `crates/davinci-agent/src/shell_policy.rs`.

The redirection scanner used a character index as a UTF-8 byte offset. A pure-policy test with `echo 🙂 | cat` reproduced a non-character-boundary panic. It also mishandled quotes around literal/escaped backslashes, allowing a later output redirect to go undetected.

The scanner now uses byte-safe character offsets and explicit quote/escape state. Restricted-policy checks parse literal words, dequote concatenated option fragments, and reject ambiguous expansions/structures. Option-aware checks reject side effects hidden in find/fd/rg/sort/tree/uniq, arbitrary sed/awk programs, and non-read-only Git operations. Git mutation detection also recognizes an absolute executable path. ReadAndTest permits actual Python test modules rather than arbitrary scripts or `-c`; Node must use an allowed test mode or an actual Vitest script operand, not `-e` followed by a runner-looking argument.

Seven new regression groups were added. The first five failed before the first patch; independent review found the Node and absolute-Git follow-ups, which were separately reproduced and fixed. All 14 focused shell-policy tests passed before the final integrated run.

Compatibility: some formerly accepted shell forms are intentionally rejected. Native read/search tools remain available. Explicitly allowed project build/test commands still execute trusted project code: this command classifier is NOT an operating-system sandbox.

### 2. CBOR allocation amplification and inconsistent validation

File: `crates/davinci-protocol/src/cbor.rs`.

A three-byte malformed container advertising 4096 entries requested a 131072-byte allocation before being rejected. The decoder no longer reserves untrusted advertised array/map capacity up front; it grows fallibly after decoding actual entries. Negative integer arguments are range-checked while unsigned, before conversion; previously an out-of-range negative argument could wrap into accepted Integer(0). Duplicate map keys and integral floats outside the protocol safe-integer range are now rejected, matching the pinned TypeScript decoder's validation.

Four new regressions initially failed and then passed; all 14 protocol tests passed. The allocation recorder uses small, bounded fixtures, not a host-exhaustion stress test. No new aggregate allocator quota or process-wide memory sandbox is claimed.

### 3. MCP HTTP/stdio input bounds and response correlation

Files: `crates/davinci-mcp/src/http.rs`, `stdio.rs`.

Successful HTTP bodies and subprocess stdout were previously buffered without size limits; stdout notifications also entered an unbounded queue while idle. HTTP bodies and individual stdout lines are now capped at 16 MiB before decoding. The stdout queue holds at most four bounded lines and applies backpressure; reader errors reach the consumer, and dropping the consumer releases a blocked sender. SSE parsing materializes one event at a time from the already bounded HTTP body. This is not a claim of socket-level streaming.

HTTP JSON and selected SSE responses must have an exactly matching, non-null request ID, jsonrpc 2.0, no method field, and exactly one result/error field. A legitimate result:null remains valid. A stale or malformed reply can no longer be mistaken for the current request merely because its result has the right shape.

Four new regression tests cover body/UTF-8 limits, malformed/cross-request envelopes, bounded CRLF/EOF stdout framing, and queue saturation/drop behavior. The body, correlation, and line defects were observed failing before repair; the queue lifecycle test was added with the bounded implementation. All 27 MCP crate tests passed before the final integrated run. No destructive payload or live MCP server was used.

### 4. MCP remote hints no longer grant authorization by default

Files: `crates/davinci-mcp/src/config.rs`, `crates/davinci-agent/src/mcp.rs`.

A configured server's readOnlyHint previously became local read-class authorization and read-only scheduling metadata. Server discovery alone does not attest to its behavior. ServerConfig now has an explicit, default-false local `trustReadOnlyHints` setting. The registry uses remote hints for authorization only for servers with that local opt-in, and removes stale trust on reconnect/drop.

A fixture advertising a mutating-sounding tool as read-only verifies default ReadOnly/plan denial, Ask prompting, explicit local opt-in, and deny-rule precedence. This regression passed before the final integrated run. The existing positive autoallow fixture was deliberately updated to opt in.

Compatibility: integrations that intentionally rely on a server's read-only assertions must explicitly set `trustReadOnlyHints: true` in that server's local configuration. Do so only for a server trusted to honor those assertions. A previously configured or project-trusted endpoint does not silently imply this opt-in.

### 5. Untrusted graph configuration cannot authorize worker execution

Files under `crates/davinci-coding-agent/src/native_extensions/graph/`: `mod.rs`, `controller.rs`, `worker.rs`.

Untrusted repository graph.json settings could supply workerExtensions and verification commands. Explicit worker `-e` flags were honored even with automatic extension loading disabled, and extension initialization preceded tool guards. GraphController now ignores project graph configuration until project trust is granted. WorkerSpec construction and CLI argument assembly independently omit untrusted extra extensions.

A temporary, structurally valid graph.json fixture reproduced untrusted extension authorization before repair. Existing worker-spec and worker-argument tests were extended to assert that no extension reaches an untrusted worker. The fixture also preserves explicitly trusted project behavior. No JavaScript payload was executed.

The final integrated verification below determines the post-fix status. This fix is distinct from the still-open same-batch graph_submit issue listed later.

### 6. Startup and tool-manager test isolation without weakening offline mode

File: `crates/davinci-coding-agent/src/startup.rs`.

The package/tmux fixture expected a package-update reply while the production offline guard correctly returned no updates. Reply parsing is now a pure helper tested directly; the runtime offline guard remains intact. A neighboring npm fixture no longer deletes the caller's offline/agent-directory settings and restores its prior reply variable.

The integrated run also exposed a separate process-environment race: tests in `crates/davinci-coding-agent/src/tools_manager.rs` cleared PATH while concurrent security-scan tests needed Git. Tool discovery, fixture replies, and offline state are now explicit inputs to a private helper. The public function still obtains those inputs normally; inline missing-tool/install fixtures no longer mutate PATH or the caller's offline environment. No security-scan production guard or test expectation was weakened.

This repairs these fixture-isolation defects; it is not a production network-permission relaxation or a claim that every environment-mutating test is now race-free.

### 7. TLS dependency advisory remediation

Files: Cargo.toml, Cargo.lock.

Exact pins changed from rustls 0.23.19 to 0.23.43 and rustls-pki-types 1.10.1 to 1.15.1, preserving existing Rustls feature choices. The lockfile changes only three packages and now selects rustls-webpki 0.103.15 instead of 0.102.8.

Official RustSec RUSTSEC-2026-0099 and RUSTSEC-2026-0098 describe DNS-wildcard and URI name-constraint validation flaws; patched stable rustls-webpki is >=0.103.12. The application uses the ordinary Rustls verifier, so this is an active dependency path. These flaws require an otherwise correctly signed certificate chain and issuer misissuance; no arbitrary self-signed certificate bypass or malicious-certificate runtime reproduction is claimed.

Sources verified on 2026-09-07: https://rustsec.org/advisories/RUSTSEC-2026-0099.html and https://rustsec.org/advisories/RUSTSEC-2026-0098.html.

A targeted crates.io metadata update and locked Windows dependency fetch were needed because the active Cargo cache did not contain the new packages. These were dependency downloads, not live-provider tests or system installations. Subsequent tests/builds use --offline --locked on the repository's pinned Rust 1.83.0 toolchain.

## Verification and delivery

Integrated job `job_35_17888264865015` compiled the security patches and passed strict Clippy, but exposed an existing graph-budget fixture needing explicit project trust (in both library/binary targets) and the tool-manager PATH race described above. The budget fixture now explicitly trusts its temporary project; the production trust guard is unchanged.

The corrected final verification completed successfully in job `job_36_17888267992c65`. `target/security-audit/verification-final.json` records exit code 0 for workspace tests (2619 passed, 3 ignored across 31 suites), strict Clippy, formatting/whitespace checks, and the release build. A fresh rerun in job `job_37_17888276914e60` also exited 0 with 2619 passed and 3 ignored; no runtime source changed between these runs. Commands:

- `rtk cargo test --workspace --offline --locked --no-fail-fast`
- `rtk cargo clippy --workspace --all-targets --offline --locked -- -D warnings`
- `rtk cargo build -p davinci-coding-agent --release --offline --locked`

Already passed: `rtk cargo fmt --all -- --check` and `git diff --check` on the complete patch. Git's LF/CRLF conversion notices are warnings, not whitespace-check failures.

Test environment: DAVINCI_OFFLINE=1, PI_OFFLINE=1, PI_DISABLE_NETWORK=1, PI_LEARNING_DISABLE_BACKGROUND=0; agent/session roots under repository target/security-audit. The initial audit mistakenly used PI_LEARNING_DISABLE_BACKGROUND=1, causing the graph-learning fixture to skip its own review. Correcting the environment made that fixture pass; the production disable switch was not changed.

Jobs 32 and 33 stopped before compiling/testing because rustls 0.23.43 was not cached; they are not code-test failures. Locked fetch job `job_34_1788826449d892` downloaded the three TLS packages. Job 35 is the subsequent verification run.

PowerShell Get-Command davinci -All and where.exe both resolved the normal launcher to `C:\Users\sergi\.cargo\bin\davinci.exe`. After the successful release build, the existing executable was backed up and replaced. Installed `--version` and `--help` smoke checks passed. A fresh SHA-256 comparison confirmed the release build and installed executable both equal `e862c0570669b82e0f6286eb393c2d0ccfdad914291e12976be1441edeba60f7`; the preserved backup hashes to `5b7fae9a8a91673b5f93196d22fc0c50642cdcd963000633fba0349f4c2ea97c`. Paths and delivery evidence are recorded in `target/security-audit/installation-result.json`. No active process was terminated. Restart existing Davinci sessions to use the updated executable.

Optional native voice compilation remains blocked by missing CMake in the Windows environment (job `job_23_17888241867770`), not a confirmed Rust source/MSRV failure. Unix-only runtime paths were not exercised on Windows. No all-features or exhaustive cargo-audit attestation is made.

## Remaining findings — NOT FIXED by this patch

These are source-confirmed reviewer findings with proposed offline regression recipes, not runtime regressions reproduced by Prime. Their affected implementation files are unchanged.

### Client handshake version validation

`crates/davinci-client/src/connection.rs::handle_message` discards Hello.version, and direct/framed SessionClient connection paths also accept incompatible snapshot versions. Check both outer and snapshot versions before state installation or callbacks. Proposed loopback fixtures: (2,1), (1,2), (2,2) rejected; (1,1) retained. No authentication bypass is claimed.

### Graph terminal submission within one tool batch

`graph/worker_hooks.rs::submit` is not one-shot inside persistence, while `davinci-agent/src/turn.rs::execute_tool_batch` prepares all calls before any execution. A single response containing [submit,submit] can overwrite the first artifact; [submit,write] can execute after submission despite prehook checks. Repair needs synchronized one-shot persistence, retry after failed validation/write, and an execution-time terminal guard without blindly repeating side-effectful hooks. Test both repeated review artifacts and a canned-provider mixed tool batch. The project-trust fix above does not address this.

### Learning skill path and symlink confinement

`native_extensions/learning/skill_manager.rs::execute_patch` and `execute_write_file` trust persisted ledger paths and validate names only in create. Supporting-file checks also skip an earlier symlink when the immediate parent does not yet exist. Foreground authorization and background ownership/hash/project-trust protections exist, but do not confine these destinations.

Repair all relevant write/view/history paths through an authoritative configured-root resolver; reject invalid names/poisoned ledger paths, validate existing ancestors, and fail closed on canonicalization errors. Use wholly temporary ../, absolute-name, poisoned-ledger, and symlink-plus-missing-parent fixtures. This requires an authorized tool operation; no unconditional auto-execution or RCE is claimed.

### Governor retrieval first-line bound

`native_extensions/token_governor.rs::retrieve` exempts the first selected line from retrieve_max_bytes and may report truncated=false. A 96000-byte line can exceed the default 48000-byte cap. Repair needs UTF-8-safe intra-line continuation that is lossless and does not repeat the same prefix; avoid merely truncating without a recoverable cursor. The backing store currently also loads the full output before selection. No runtime context-exhaustion experiment was performed.

### URL/IDNA dependency advisory

The lockfile still contains url 2.5.0 -> idna 0.5.0, matching RUSTSEC-2024-0421. Official remediation recommends url >=2.5.4 / idna >=1.0.3. Update with an MSRV-compatible transitive lock rather than blindly selecting a newer ICU graph. Existing parsed-host and dial-time IP checks mean no application SSRF bypass was established. Proposed malformed-Punycode parser cases have not been run. Source: https://rustsec.org/advisories/RUSTSEC-2024-0421.html.

Also noted for maintenance: lru 0.12.5 matches informational RUSTSEC-2026-0002, but no active use of the affected IterMut API was established. Do not describe this as demonstrated application memory corruption. cargo-audit and a local advisory database were unavailable, so dependency review was targeted and manual.

## Agent and graph coverage

All 14 read-only assignments were submitted and accepted in coordinator run `run-670d12335e85b65f`. Five completed tool-backed reports: MCP; protocol/client/server; graph permissions; learning/extensions; dependencies/CI. Nine assignments stalled or detached without tool-backed final results. They are incomplete coverage, not nine clean audits. The coordinator run is now closed with state `finished` and zero workers to stop; no replacement worker wave ran. Prime applied source patches centrally to avoid concurrent edits.

The native baseline graph succeeded: `run-d0b7bfd29ea843138121a07718fe1bc7`.
Verified artifact: `artifact-fdd8a754bd7ea775fe0699d679b9eb227d2f9650c421b59454ae319936cce337` (`git-status-porcelain/v1`). This records tracked repository status only; empty baseline entries do not include the pre-existing untracked plugins/ directory. It is repository evidence, not proof that tests or a complete security review passed.

The final native status graph also succeeded: `run-c4f451831d3648c0b5e017f497469a6b`, with successful independent-verification receipt `receipt:801dca75c57842559a1bb398c891aca7`. Artifact `artifact-3dc90cda36f35f7ca158dc1ec59de5df81d0b2314fc17ff5a10167c5d214b64e` is 675 bytes with SHA-256 `51d8dcc66bec679b9a6190d266cb7df6ed98c987121b79f28ff73891825ee29f`. Like the baseline graph, this records tracked Git status, not test outcomes or an exhaustive security verdict.

A subsequent two-node evidence graph, `run-d4fc37adb2bf4324aefe13e52429c350`, completed successfully with receipt `receipt:2f3aa06b068d4827b925dfee1354594c`. Both read-only Git status and Git diff nodes passed their independent deterministic command-exit gates. Status artifact: `artifact-ccbf2127deb2c125cd746eb9d3799426de2f6d862d821f472b4b0d52987c6a89`. Diff artifact: `artifact-04303304f520d72276eacc43c1d92399109766de39bec3497a5cec9014767763`. The diff receipt records exit code 0, no timeout or output truncation, and a SHA-256 of `8b7f0a4e39dc6f50c161cda6369e6a99d9e39641a52a139f7f62b91d763fd051` for its 62187-byte stdout. These artifacts preserve command provenance and output hashes, not a model security verdict. Only this report was updated afterward.
