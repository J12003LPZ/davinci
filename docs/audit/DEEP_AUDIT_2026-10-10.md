# DaVinci deep audit, 2026-10-10

Revision audited: `7b92153a` on `fix/audit-final-report` ("fix: resolve final audit findings and transport suspicions").
Findings ledger origin: written by an earlier session at `ae974c6c`. Every entry was re-checked at `7b92153a`; line numbers below are current.
Mode: report only. No production code, tests, or repository config were changed. The only file written in the repo is this report.
Method: solo manual review, Clippy, the workspace test suite, and disposable probes compiled against the real crates in a scratch directory outside the repo (`dvprobe` crate plus a Python MCP HTTP fixture). No worktree, no subagents.

## What changed since the ledger was written

The ledger was written at `ae974c6c`. Commit `7b92153a` landed afterwards and touched `davinci-mcp/src/http.rs`, `security_scan/redaction.rs`, a new `native_extensions/credential_redaction.rs`, `davinci-session/src/{jsonl_repo.rs,lib.rs}`, `rpc.rs`, and `inspector.rs`. Every probe was rerun with a binary built after that commit (probe binary 05:17, HEAD sources 02:17). Result:

- All prior-audit findings that the ledger listed as "still open" (DVA-001 to DVA-007, DVA-009) are **resolved at HEAD**.
- Two ledger findings (MCP-02, MCP-03) are **resolved at HEAD**.
- SEC-03 is **partly resolved**: quoted `auth_token = "..."` is now masked; unquoted `*_TOKEN=` values and `npm_` / `glpat-` tokens still leak.

## Summary

| Bucket | Count |
|---|---|
| Confirmed and open at HEAD | 10 (3 High, 5 Medium, 2 Low) |
| Resolved at HEAD, re-verified by probe or code read | 10 (DVA-001 to DVA-007, DVA-009, MCP-02, MCP-03) |
| Unverified candidates | 8 |
| Rejected after checking | 8 |

| ID | Severity | Status | Location | One line |
|---|---|---|---|---|
| SHELL-01 | High | Confirmed (probe `perm`) | `crates/davinci-agent/src/permission_risk.rs:369-382`, `shell_policy.rs:349`, `shell_policy.rs:452` | Auto mode approves a PowerShell command whose statements are separated by a lone CR |
| SHELL-02 | High | Confirmed (probe `perm`) | `crates/davinci-agent/src/shell_policy.rs:186`, `:537`, `:616` | ReadOnly/ReadAndTest shell profile admits `cargo tree/metadata` with any arguments, including a compiler override |
| SEC-01 | High | Confirmed (probe `secread`) | `crates/davinci-coding-agent/src/native_extensions/security_scan.rs:1964-1967`; `security_scan/snapshot.rs:177-195` | PEM private-key body lines pass through Security Scan redaction |
| SEC-02 | Medium | Confirmed (probe `secread`) | `security_scan/redaction.rs:13` | Credentials in non-HTTP URLs (`postgres://user:pass@`) are not masked |
| SEC-03 | Medium | Confirmed, narrowed (probe `secread`) | `security_scan/redaction.rs:8`, `:11` | Unquoted `*_TOKEN=` values and `npm_` / `glpat-` tokens are not masked |
| MCP-01 | Medium | Confirmed (probe `mcp-stdio-utf8`) | `crates/davinci-mcp/src/stdio.rs:141-147`, `:490-492` | One non-UTF-8 byte on server stdout permanently kills the stdio transport |
| MCP-04 | Medium | Confirmed (static) | `crates/davinci-coding-agent/src/mcp.rs:19-37`; `crates/davinci-agent/src/mcp.rs:467-469`; `permission.rs:1590-1595` | Plugin and project `mcp.json` can set `trustReadOnlyHints` for their own servers |
| SESS-01 | Medium | Confirmed (probe `unc`) | `crates/davinci-session/src/jsonl_repo.rs:46-49` vs `discovery.rs:115-119`; caller `crates/davinci-coding-agent/src/main.rs:1660-1676` | `--session-id` sessions under a UNC cwd are never rediscovered; the next run fails |
| EXT-01 | Low | Confirmed (static) | `crates/davinci-coding-agent/src/main.rs:11203`, `:11222`, `:11231` | Extension host drops `appendEntry` / `setLabel` / `setSessionName` persistence errors (DVA-004 sibling) |
| CM-01 | Low | Confirmed (static) | `crates/davinci-agent/src/codemode/dispatch.rs:339-345`; `runtime/capacity.rs:240-249` | A `SerialBarrier` child can run alongside parallel-safe children in Code Mode ReadOnly |

## Confirmed findings

### SHELL-01: Auto mode approves CR-separated PowerShell statements

- **Severity:** High. **Status:** Confirmed.
- **Location:** `crates/davinci-agent/src/permission_risk.rs:369-376` (chain check), `:377` (segment split), `:382` (word split); `crates/davinci-agent/src/shell_policy.rs:349` (splitter), `:452` (lexer). Caller: `crates/davinci-agent/src/permission.rs:202` (`ToolClass::Shell` in Auto).
- **What is wrong:** `routine_local_shell` normalizes `\r` to `;` only for its empty-segment sanity check (line 373), then hands the original string to `split_shell_segments_with_diagnostic`, which splits on `;` and `\n` but not `\r` (line 349). The resulting single segment is word-split with `split_whitespace`, which treats `\r` as whitespace, so the second statement becomes extra "arguments" of the first, read-only program.
- **Impact:** In Auto mode, a model-proposed PowerShell command made of an allowed read command plus a second arbitrary statement runs without a prompt. PowerShell treats a lone CR as a statement terminator (verified with `pwsh` by the earlier session). The same splitter and lexer back `shell_policy::evaluate` (line 887), so graph worker ReadOnly profiles inherit the gap (static; not separately probed).
- **Evidence:** probe `perm` at HEAD:
  ```
  powershell Get-Content README.md                                 -> ALLOW (no prompt)
  powershell Get-Content README.md; Start-Process notepad.exe      -> ASK
  powershell Get-Content README.md<CR>Start-Process notepad.exe    -> ALLOW (no prompt)
  powershell Get-Content README.md<CR>Invoke-WebRequest -Uri ...   -> ASK   (rejected only because the URL contains ':')
  ```
  The `;` control asks; the CR variant does not. The `Invoke-WebRequest` row is refused by an unrelated colon check, not by statement separation.
- **Suggested fix:** Treat `\r` as a statement separator in `split_shell_segments_with_diagnostic` (`';' | '\n' | '\r'`), and have `literal_shell_words` return `None` on any unquoted `\r`, `\n`, or other vertical whitespace instead of treating it as a word break. Fail closed: any control character outside quotes means "not routine".
- **Regression test:** in `permission_risk` tests, assert `routine_local_shell` is false and `PermissionPolicy::decide` in Auto returns `Ask` for `"<read cmd>\r<second cmd>"`, `"\r\n"` and `"\u{0b}"`/`"\u{0c}"` variants; in `shell_policy` tests, assert `split_shell_segments_with_diagnostic("a\rb")` yields two segments and `evaluate(ReadOnly, "cat x\rStart-Process y")` is not `Allowed`.

### SHELL-02: ReadOnly shell profile admits cargo with arbitrary arguments

- **Severity:** High (graph workers that are meant to be read-only can execute code). **Status:** Confirmed.
- **Location:** `crates/davinci-agent/src/shell_policy.rs:186` (`cargo (tree|metadata)` in `READ_PATTERNS`), `:537` (`read_arguments_are_safe`), `:616` (`"cargo" | "npm" => true`). Profile mapping: `crates/davinci-coding-agent/src/native_extensions/graph/roles.rs:225-226`, `:265-267` (Researcher, Historian get ReadOnly; TestAnalyzer, Reviewer get ReadAndTest).
- **What is wrong:** The prefix pattern admits `cargo tree` and `cargo metadata`, and `read_arguments_are_safe` then returns `true` for `cargo` regardless of arguments. Cargo accepts `--config` overrides that replace the compiler or compiler wrapper, and both subcommands invoke the compiler to probe target info, so a "read-only" command executes a project-controlled program.
- **Impact:** A worker restricted to the ReadOnly or ReadAndTest profile can run arbitrary code. The earlier session verified that cargo runs the configured wrapper for these subcommands.
- **Evidence:** probe `perm` at HEAD (override value elided):
  ```
  ReadOnly profile: cargo tree --config '<compiler override>'          -> is_read_only=true decision=Allowed
  ReadOnly profile: cargo metadata --config '<compiler wrapper override>' -> is_read_only=true decision=Allowed
  ```
- **Suggested fix:** Give `cargo` an argument allowlist in `read_arguments_are_safe` like `find` and `uniq` already have: reject `--config`, `-Z`, `--manifest-path` outside the boundary, and `+toolchain`; or drop `cargo tree/metadata` from the ReadOnly set and expose them through a native tool. Review `npm` in the same arm (see rejected list: `npm ls/view/info/explain` found no exec path, but the arm still accepts any arguments).
- **Regression test:** `shell_policy` table test asserting `evaluate(ReadOnly, ..)` and `evaluate(ReadAndTest, ..)` deny `cargo tree --config ...`, `cargo metadata --config=...`, `cargo -Zunstable-options tree`, while plain `cargo tree` and `cargo metadata --format-version 1` stay allowed.

### SEC-01: PEM private-key bodies are not redacted

- **Severity:** High (confidentiality). **Status:** Confirmed.
- **Location:** `crates/davinci-coding-agent/src/native_extensions/security_scan.rs:1964-1967` (`redact_evidence` masks only a line that contains `PRIVATE KEY`); `security_scan/redaction.rs:5-29` (line-by-line, no body pattern); `security_scan/snapshot.rs:177-195` (`denied` lists `id_rsa`, `id_ed25519` but not `id_ecdsa`, `id_dsa`).
- **What is wrong:** Redaction is per line. The BEGIN and END lines are replaced, but the base64 body lines between them match no pattern and pass through unchanged. Separately, the snapshot deny list misses two common OpenSSH key filenames, so those files enter the snapshot at all.
- **Impact:** Private-key material in source (embedded PEM constants, test fixtures, committed keys) reaches the Security Scan worker model through `sec_source_read` (`security_scan/tools.rs:74`) and survives `report::sanitize` into exported reports.
- **Evidence:** probe `secread` at HEAD (synthetic, non-key material):
  ```
  == embedded_pem.rs
  const KEY: &str = "\
  [REDACTED PRIVATE KEY MATERIAL]
  MIIEfixtureFIXTUREfixtureFIXTURE...abcd
  MIIEfixtureFIXTUREfixtureFIXTURE...abcd
  [REDACTED PRIVATE KEY MATERIAL]
  == report::sanitize(embedded pem)
  (same: both body lines survive)
  ```
- **Suggested fix:** Redact on the whole text before splitting into lines: replace every `-----BEGIN [A-Z ]*PRIVATE KEY-----` ... `-----END [A-Z ]*PRIVATE KEY-----` span (multiline, non-greedy, also when the END marker is missing: redact to end of input or end of file). Add `id_ecdsa`, `id_dsa`, `id_ecdsa_sk`, `id_ed25519_sk` and `*.ppk` to `denied`.
- **Regression test:** `redaction::text` and `report::sanitize` tests with a multi-line PEM (RSA, EC, OPENSSH, PKCS#8), one inside a string literal with escaped newlines, and one with no END line; assert no body line survives. `snapshot::denied` test for each added filename.

### SEC-02: credentials in non-HTTP URLs are not redacted

- **Severity:** Medium. **Status:** Confirmed.
- **Location:** `crates/davinci-coding-agent/src/native_extensions/security_scan/redaction.rs:13`.
- **What is wrong:** The userinfo pattern is anchored to `https?://`. Database and broker URLs (`postgres://`, `mysql://`, `mongodb+srv://`, `redis://`, `amqp://`) carry the same `user:password@` form and are not masked.
- **Impact:** Connection-string passwords in `.env.example`, config, or docs reach the scan worker and reports.
- **Evidence:** probe `secread`: `DATABASE_URL=postgres://admin:fixturepw@db.internal/app` is returned unchanged.
- **Suggested fix:** Generalize to `[a-z][a-z0-9+.-]*://[^\s/:@]+:[^\s/@]+@` and replace only the password part.
- **Regression test:** table test over the schemes above plus an `https` control, asserting the password is gone and the host stays visible.

### SEC-03: unquoted token assignments and npm / GitLab tokens are not redacted

- **Severity:** Medium. **Status:** Confirmed, narrowed by HEAD.
- **Location:** `security_scan/redaction.rs:8` (unquoted key list lacks `token`), `:11` (prefix list lacks `npm_`, `glpat-`). For contrast, `native_extensions/credential_redaction.rs:10` (new at HEAD) includes `token`, which is why quoted values are now masked.
- **What is wrong:** HEAD added `token` to the quoted-assignment pass only. The unquoted assignment regex still recognises `access_token` but not a bare `TOKEN` key, and no prefix rule covers npm or GitLab personal tokens.
- **Impact:** `export NPM_TOKEN=...`, `GITLAB_TOKEN=...`, CI files and shell scripts leak tokens to the scan worker and reports.
- **Evidence:** probe `secread` at HEAD:
  ```
  export NPM_TOKEN=npm_fixtureTOKENfixtureTOKEN    (unchanged)
  export GITLAB_TOKEN=glpat-fixtureTOKENfixture    (unchanged)
  [REDACTED]                                       (quoted auth_token: fixed at HEAD)
  ```
- **Suggested fix:** Add `token` to the key alternation at line 8 (keeping the `[a-z0-9_.-]*` affixes so `NPM_TOKEN`, `auth_token` match) and add `\bnpm_[A-Za-z0-9]{20,}` and `\bglpat-[A-Za-z0-9_-]{20,}` prefix rules.
- **Regression test:** `redaction::text` cases for `NPM_TOKEN=`, `GITLAB_TOKEN=`, `token: value`, bare `npm_...`, bare `glpat-...`, plus a prose control (`the token expires`) that must stay visible.

### MCP-01: one non-UTF-8 stdout byte kills the stdio transport

- **Severity:** Medium. **Status:** Confirmed.
- **Location:** `crates/davinci-mcp/src/stdio.rs:141-147` (reader thread sends the error and `break`s), `:490-492` (`String::from_utf8` turns a bad line into `InvalidData`). Contradicted by the doc comment at `:271-272` ("skipping notifications and stray log lines").
- **What is wrong:** A stray log line in a legacy code page is a decode error, and the reader thread treats any decode error as fatal: it reports it to the waiting call and exits, closing the pipe.
- **Impact:** The in-flight call fails and every later call fails with "server closed stdout". Common on Windows, where servers print through cp1252 consoles.
- **Evidence:** probe `mcp-stdio-utf8` at HEAD (server prints `log: caf\xe9 ready` before its reply):
  ```
  first call:  Err("mcp stdout: invalid utf-8 sequence of 1 bytes from index 8")
  second call: Err("mcp server closed stdout (stderr: ... OSError: [Errno 22] Invalid argument ...)")
  ```
- **Suggested fix:** In `read_stdout_line`, return the line via `String::from_utf8_lossy` (or skip the line) when it is not valid UTF-8; JSON-RPC frames are UTF-8 by spec, so a non-UTF-8 line is never a frame and can be skipped like any other non-JSON line. Keep the oversize-line error fatal.
- **Regression test:** stdio transport test with a fixture server that writes `b"log: caf\xe9\n"` before each reply; assert two consecutive `call_tool` calls succeed.

### MCP-04: plugin and project config can self-grant `trustReadOnlyHints`

- **Severity:** Medium. **Status:** Confirmed (static).
- **Location:** `crates/davinci-mcp/src/config.rs:45-46` (field), `crates/davinci-coding-agent/src/mcp.rs:19-37` (`resolve_origin` rewrites `execution` for project/plugin layers but never clears the trust flag), `crates/davinci-agent/src/mcp.rs:467-469` (trusted servers recorded), `crates/davinci-agent/src/permission.rs:1590-1595` (`class_of` returns `ToolClass::Read`) and `:201` (Read auto-allowed). Code Mode ReadOnly admission reads the same set (`codemode/authority.rs:47`).
- **What is wrong:** The flag that turns a server's own `readOnlyHint` into auto-approval is accepted from the same configuration layer that defines the server. Plugin layers are loaded for every project; project layers only for trusted projects (`mcp.rs:64`).
- **Impact:** An installed plugin, or a trusted project's `mcp.json`, can make any of its tools auto-approved in Auto mode and callable from Code Mode ReadOnly by declaring them read-only. Most relevant for remote (`url`) and sandboxed servers, where the host had not otherwise granted code execution.
- **Evidence:** code path above; no layer between `load_layer` and `merge` clears `trust_read_only_hints` for `project_or_plugin`. Existing tests (`turn.rs:6572`, `contract_executor.rs:576`) cover contract authority, not this config origin.
- **Suggested fix:** In `resolve_origin`, set `server.trust_read_only_hints = false` when `project_or_plugin` is true, so only the user agent-directory file (or `DAVINCI_MCP_CONFIG`) can grant it.
- **Regression test:** `mcp::load` test with a plugin layer and a trusted project layer both setting `trustReadOnlyHints: true`; assert the merged config has it false for both, and true for an equivalent user-layer server.

### SESS-01: `--session-id` sessions under a UNC working directory are never rediscovered

- **Severity:** Medium (hard failure on second run; Windows network shares only). **Status:** Confirmed.
- **Location:** `crates/davinci-session/src/jsonl_repo.rs:46-49` (`jsonl_session_directory_name` uses `trim_start_matches`, stripping every leading separator) vs `crates/davinci-session/src/discovery.rs:115-119` (`encode_cwd_component` uses `strip_prefix`, stripping one). Caller: `crates/davinci-coding-agent/src/main.rs:1660-1676` (`resolve_session_ref` then `JsonlSessionRepo::create`); lookup at `discovery.rs:545-557` is scoped to the cwd directory.
- **What is wrong:** For a cwd with two or more leading separators the two encoders disagree: `\\server\share\proj` becomes `--server-share-proj--` on create and `---server-share-proj--` on discovery. The session file is written where discovery never looks.
- **Impact:** First `--session-id X` run creates the session. Every later `--session-id X` run misses it, falls through to create, and exits with `Session already exists: X`. `--continue` and `--resume` from that cwd also never list it. Plain sessions are unaffected because `JsonlSession::create` (`lib.rs:88`) uses `encode_cwd_component`; `JsonlSessionRepo::create` has one production caller, this one. Verbatim paths (`\\?\C:\...`) hit the same mismatch (static).
- **Evidence:** probe `unc` at HEAD:
  ```
  cwd=/home/u/proj         create_dir=--home-u-proj--      discover_dir=--home-u-proj--       discovered=true
  cwd=C:\Users\u\proj      create_dir=--C--Users-u-proj--  discover_dir=--C--Users-u-proj--   discovered=true
  cwd=\\server\share\proj  create_dir=--server-share-proj-- discover_dir=---server-share-proj-- discovered=false second_create=Session already exists: probe19
  ```
  (The probe calls `create` twice directly, so `second_create` errors for every row; the defect is `discovered=false`, which makes the real caller take that path.)
- **Suggested fix:** Make `jsonl_session_directory_name` call `encode_cwd_component` (one encoder), and have discovery also scan the old multi-strip name so sessions already written under it stay reachable.
- **Regression test:** `davinci-session` test that creates through `JsonlSessionRepo::create` with cwd `\\server\share\proj` and `//server/share/proj`, then asserts `resolve_session_ref(root, Some(cwd), id)` finds it; plus an equality test `jsonl_session_directory_name(c) == encode_cwd_component(c)` over a cwd table.

### EXT-01: extension host drops session persistence errors

- **Severity:** Low. **Status:** Confirmed (static). Same defect class as DVA-004, which HEAD fixed in RPC only.
- **Location:** `crates/davinci-coding-agent/src/main.rs:11203` (`appendEntry`), `:11222` (`setLabel`), `:11231` (`setSessionName`), all `let _ = ...`. A failure channel exists in the same function (`failures`, declared `:11158`, used `:11301`) and is surfaced by the caller at `:3737-3753`.
- **What is wrong:** The three ops discard `append_entry` / `set_name` errors instead of pushing to `failures`. Missing fields also default to empty strings: a call without `name` clears the session name, and a call without `entryId` writes a label targeting `""`.
- **Impact:** Extension-written entries, labels, and names can be silently lost on a storage failure, while `session_call_note` (`:11146`) still reports "an extension named the session ...".
- **Evidence:** code read; the sibling `switchSession` arm in the same `match` reports failures, these three do not.
- **Suggested fix:** Push `(index, error.to_string())` to `failures` on error; reject `setLabel` without `entryId` and `setSessionName` without a `name` field (distinguish absent from explicit `""`).
- **Regression test:** `apply_session_calls` test with a session whose backing path is replaced by a directory (the technique used by `rpc.rs` test `rename_reports_storage_failure_without_event_or_metadata_change`); assert each op returns a failure entry.

### CM-01: Code Mode lets parallel-safe children run beside a serial-barrier child

- **Severity:** Low (ReadOnly mode only; all admitted capabilities are read-only). **Status:** Confirmed (static).
- **Location:** `crates/davinci-agent/src/codemode/dispatch.rs:339-345` (per-call `max_concurrency`: `parallelism` for `ParallelSafe`, `1` otherwise), `:382-388` (`acquire`); `crates/davinci-agent/src/runtime/capacity.rs:240-249` (admit if `active < max_concurrency`); policy enum `runtime/capabilities.rs:40-43`.
- **What is wrong:** The pool compares the current count against the caller's own limit. A `SerialBarrier` child waits for `active == 0`, but once it runs, a `ParallelSafe` child with limit N sees `1 < N` and starts. The barrier holds only in one direction.
- **Impact:** Tools that declared themselves serial (shared state, rate-limited servers) can see overlapping calls in Code Mode ReadOnly.
- **Evidence:** code path above; mutating mode always uses limit 1, so it is unaffected.
- **Suggested fix:** Track the active barrier in `WorkerCounts` (e.g. `exclusive: bool`) and admit nothing while it is set; or acquire barrier children with limit 1 and require `active == 0` for every caller while a barrier holds a slot.
- **Regression test:** `capacity.rs` test: hold a permit acquired with limit 1 (barrier), then assert `acquire(4, ..)` blocks until it drops; plus a Code Mode dispatch test with one serial and two parallel-safe fake capabilities recording overlap.

## Resolved at HEAD (re-verified)

| ID | Prior status | Evidence at `7b92153a` |
|---|---|---|
| DVA-001, DVA-002 | Open at `ae974c6c` | `clone_session` now copies only the selected ancestry and keeps entry IDs (`crates/davinci-session/src/lib.rs:409-426`). `session_integrity` suite 9/9 pass in this session. |
| DVA-003 | Open | `bound_value` truncates at a char boundary (`inspector.rs:1103-1109`) with a 2/3/4-byte boundary test. |
| DVA-004, DVA-005 | Open | `set_session_name` returns `fail` on persistence error and emits the event for every successful rename including `null` (`rpc.rs:477-491`); two new RPC tests. |
| DVA-006 | Open | probe `secread`: `quoted_space.py` now prints `[REDACTED]` (was `[REDACTED] first second third"`). |
| DVA-007 | Open | probe `jsonl`: `load ok=false before=324 after=324 final_line_survives=true`; file untouched, no rewrite (was 324 to 308). |
| DVA-008 | Historical | Fixed by PR #167, already in this lineage. |
| DVA-009 | Open | fixture `sse-after`: `CLIENT REPLIED id=srv-ping`. |
| MCP-02 | Confirmed at `ae974c6c` | fixture `json-batch`: `CLIENT REPLIED id=srv-ping` (was no reply). `answer_batch_requests` at `http.rs:150-164`. |
| MCP-03 | Confirmed at `ae974c6c` | fixture `sse-unterminated`: `CLIENT REPLIED id=srv-ping` (was no reply). |

Control: fixture `sse-before` also answered. `davinci-mcp` library suite 46/46 pass in this session.

## Unverified candidates

Not counted as confirmed. Each has a code location verified at HEAD and a stated reason it is not confirmed.

| ID | Likely severity | Location | Suspicion | What would settle it |
|---|---|---|---|---|
| CAND-01 | Medium | `security_scan/retention.rs:60-63` | On Windows the lease is dropped before `remove_dir_all`; a scan that takes the lease in that window can have part of its directory deleted (`remove_dir_all` is not atomic). | Two-process race test with a barrier between `drop(lease)` and removal. |
| CAND-02 | Medium | `security_scan/tools.rs:90-92` | `sec_source_search` matches the raw, unredacted text and reports match locations, so match/no-match answers reveal content that `sec_source_read` redacts. Code path is certain; whether the scan worker is in the threat model is a policy call. | Decide whether worker output is untrusted; if so, search over `redaction::text(line)`. |
| CAND-03 | Medium | `native_extensions/vector_memory.rs:476` | `redact_secrets` has no PEM-body or URL-userinfo handling (same gaps as SEC-01/SEC-02); memory extraction from messages (`:573`) persists the result. | Probe: message containing a PEM block and a `postgres://u:p@` URL through memory extraction. |
| CAND-04 | Low | `runtime/context_vm/retrieval.rs:102-115` | A single line longer than `MAX_PAGE_BYTES` is cut and the next offset skips the remainder; the tail is unretrievable. | Decide whether this is a design limit; if not, page within a line. |
| CAND-05 | Low | `runtime/context_vm/mod.rs:676-681` | `record_events` clears `source_contents` and repopulates only from events, dropping sources registered by broker packets. | Probe a broker packet source, then `record_events`, then retrieve. |
| CAND-06 | Low | `runtime/context_vm/sources.rs:179-182` | Index reset triggers only when the file shrank; a rewrite that ends up the same length or longer keeps stale offsets. | Rewrite a session with a shorter header plus appended entries, then retrieve by id. |
| CAND-07 | Low | `crates/davinci-session/src/lib.rs:515-524` | `prepare_first_write` adopts the file's `leaf_id` when another process appended, discarding a `set_leaf` branch choice made before the first write. | Two-handle test: open, `set_leaf(old)`, append from a second handle, write from the first. |
| CAND-08 | Low | `crates/davinci-coding-agent/src/main.rs:11215-11222` | `setLabel` without `entryId` writes a label targeting `""` (also listed under EXT-01); unclear whether the tree loader tolerates it. | Load a session containing such a label entry. |

## Rejected candidates

| Candidate | Why rejected |
|---|---|
| `bash` row of probe `perm` (`cat README.md<CR>...` allowed) | Bash does not treat CR as a separator or as IFS whitespace; the command is one `cat` with an odd filename. Not exploitable through the bash tool. PowerShell is the affected shell (SHELL-01). |
| `npm` in `read_arguments_are_safe` accepting any arguments | `READ_PATTERNS` (`shell_policy.rs:188`) first limits npm to `ls/list/view/info/explain`; no script-execution path was found for those subcommands. Kept as a hardening note in SHELL-02, not a finding. |
| DVA-007 fix makes a session with a completed malformed record fail to load | Intended by the remediation ("reject completed malformed records without rewriting"); the file is preserved and the error is explicit. Availability trade-off, not a defect. |
| MCP-04 for untrusted projects | Project `mcp.json` is loaded only when the project is trusted (`coding-agent/src/mcp.rs:64-66`); the finding is limited to plugin layers and trusted projects. |
| SESS-01 affecting ordinary sessions | `JsonlSession::create` uses the discovery encoder (`lib.rs:88`); `JsonlSessionRepo`'s own list and find (`jsonl_repo.rs:237`, `:292`, `:343`) are internally consistent. Only the `--session-id` create path is affected. |
| `.env.example` not covered by `denied` | `denied` rejects any component starting with `.env.`; the probe file name (`dburl.env.example`) was chosen to bypass the deny list on purpose so redaction could be tested. Not a deny-list gap. |
| `Invoke-WebRequest` after CR being blocked means SHELL-01 is limited | It is blocked only because its URL argument contains `:`; any second statement without a colon or path escape is approved. Rejected as a mitigation. |
| Prior-audit rejections (SigV4 date slicing, memory UUID formatter, graph diff slicing, OSC color, protocol bounded errors, Kitty chunks) | Re-read in `BUG_TRACKER.md`; their reasoning still holds at HEAD. Not re-probed. |

Note: the truncated earlier draft of this file claimed "14 confirmed" and "9 rejected". The ledger it was built from records 10 confirmed (of which 2 are now resolved) and does not record the 9 rejected items, so those numbers cannot be reproduced and are superseded by the counts above.

## Coverage ledger

"Read" means the implementation and its direct callers were reviewed for this audit. "Mapped" means entry points only. Nothing here is claimed as exhaustively line-reviewed.

| Crate (src `.rs` files) | Reviewed | Not reviewed |
|---|---|---|
| `davinci-mcp` (8) | Read: `lib`, `stdio`, `http`, `config`, `types` | `bin/mcp_fixture` (test support) |
| `davinci-session` (10) | Read: `jsonl_repo` load/append/naming, `lib.rs` `JsonlSession` (create, clone, prepare_first_write), `discovery` naming and `resolve_session_ref` | `tree.rs` beyond clone context, `codec`, migration paths |
| `davinci-agent` (219) | Read: `shell_policy` analyze/evaluate path, `permission_risk::routine_local_shell`, `permission.rs` class/decide, `mcp.rs` registry trust, `codemode/{mod,deadline,authority,dispatch,policy}`, `runtime/capacity`, `runtime/context_vm/{mod,sources,retrieval}`, `runtime/operations/inspector` (diff only) | Providers glue, turn loop beyond permission, journal writer/recovery, notebook, browser/external adapters, most of `runtime/` |
| `davinci-coding-agent` (325) | Read: `security_scan` (`redaction`, `tools`, `snapshot::denied`, `retention`, `report::sanitize`, `redact_evidence`), `credential_redaction`, `vector_memory::redact_secrets`, `mcp.rs` config layering, `main.rs` `resolve_or_create_session` and `apply_session_calls`, `rpc.rs` rename (diff), `graph/roles` profile mapping | TUI glue in `main.rs`, `semantic/*` (changed at HEAD, not re-reviewed), plugins, learning, interaction testing, repo intelligence |
| `davinci-ai` (52) | Not reviewed | All |
| `davinci-tui` (128) | Not reviewed | All |
| `davinci-evals` (77) | Not reviewed | All |
| `davinci-client` (6), `davinci-protocol` (6), `davinci-server` (2) | Not reviewed | All |
| `davinci-session-sqlite` (3), `davinci-sys` (5), `davinci-telemetry` (1) | `davinci-sys::lock` used by retention and session lock, read at call sites only | Rest |

## Verification results

| Check | Result | Source |
|---|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | Clean, exit 0 | Earlier session `clippy.log` at `ae974c6c`; HEAD's remediation also reports all-target/all-feature Clippy clean |
| Workspace tests | 6,222 passed, 0 failed, 57 ignored | As supplied and as recorded in `REMEDIATION_RESULTS.md`. The pass count includes nested fixture subprocess summaries, so it is not a count of unique tests. |
| Windows file locks | 4 targets first failed with `os error 32` and passed on rerun: `davinci-coding-agent` tests `repo_intelligence_edges`, `retired_decision_settings`, `runtime_recovery_e2e`, `security_scan_incremental_telemetry` | Earlier session `tests.log` (lines 5714-5787) |
| `cargo test -p davinci-session -p davinci-mcp` at HEAD (this session, scratch target dir) | All pass: `davinci-mcp` lib 46, `davinci-session` lib 73, `session_integrity` 9, `wor178_181_session_recovery` 5, plus 7 smaller targets; exit 0 | This session |
| Probes at HEAD | `perm`, `secread`, `jsonl`, `unc`, `mcp-stdio-utf8`, and `mcp-http` in four fixture modes; outputs quoted per finding | This session |

Note on the workspace number: the earlier session's `tests.log` summaries add up to 6,204 passed; the difference from 6,222 is consistent with the four locked targets being counted only in the rerun. The workspace suite was not rerun by this session.

## Limits

- Windows host only. No Linux or macOS build or test runs, so SHELL-01's PowerShell behaviour and SESS-01's UNC paths were checked only on Windows, and POSIX `//host/share` paths (same encoder mismatch, static) were not exercised.
- No fuzzing, property testing, mutation testing, or concurrency stress. CAND-01, CAND-07 and CM-01 are race-shaped and are static only.
- No live providers, OAuth, or internet MCP servers. MCP evidence is loopback fixtures only.
- Large crates (`davinci-ai`, `davinci-tui`, `davinci-evals`, most of `davinci-agent/runtime`) were not reviewed; absence of findings there means nothing.
- `semantic/transport.rs` and `semantic/backend.rs` were changed at HEAD and were not re-reviewed.
- All secrets in probes are synthetic.

## Recommended fix order

1. **SHELL-01**: silent code execution in the default Auto path; small, local fix in the splitter and lexer.
2. **SHELL-02**: code execution from profiles documented as read-only; argument allowlist for `cargo`.
3. **SEC-01**: private-key bodies reaching the model and exported reports; multiline PEM mask plus deny-list names. Apply the same mask to `vector_memory::redact_secrets` (CAND-03) in the same change.
4. **SEC-03, SEC-02**: one change to `redaction.rs` patterns; ship together with SEC-01's tests.
5. **MCP-04**: one-line clear in `resolve_origin`; closes a permission escalation from plugin config.
6. **MCP-01**: transport death from a single byte; skip non-UTF-8 lines.
7. **SESS-01**: unify the two cwd encoders and keep scanning the old name.
8. **EXT-01**: route the three ops through `failures`, matching the DVA-004 fix.
9. **CM-01**: barrier-aware capacity pool.
10. Then settle the candidates, starting with CAND-02 (policy decision) and CAND-01 (race test).
