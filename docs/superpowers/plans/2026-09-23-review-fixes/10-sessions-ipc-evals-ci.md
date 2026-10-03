# Phase 10: Sessions, IPC, Evals and CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Quality gates can fail when quality drops, CI runs the storage and IPC tests on Windows, stub subsystems stop pretending to work, and session listing scales with the number of sessions.

**Architecture:** Mostly deletions and wiring. Three tasks depend on decisions D4, D6 and D7 in `00-index.md`; each gives the recommended option in full and the alternative's scope.

**Tech Stack:** Rust 1.83, GitHub Actions.

Independent of Phases 2-9, except Task 10.9 (CI) which should land after Task 2.15 so the release-strings check has something to check.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-evals/src/main.rs:781-797`, `src/behavior/gate.rs`, `src/behavior/runner.rs:49-60, 149-171` | 10.1 |
| `.github/workflows/behavior-live.yml:79-94` | 10.2 |
| `crates/davinci-parity/` | 10.3 |
| `crates/davinci-session-sqlite/`, `crates/davinci-coding-agent/src/main.rs:1173-1187`, settings `session_backend` | 10.4 |
| `crates/davinci-server/`, `crates/davinci-client/`, `crates/davinci-coding-agent/src/experimental.rs`, `main.rs:13-86` | 10.5 |
| `crates/davinci-session/src/discovery.rs:137-172, 278-285` | 10.6 |
| `crates/davinci-session/src/lib.rs:380-388`, `jsonl_repo.rs:591` | 10.7 |
| `crates/davinci-protocol/src/framing.rs:167`, `src/{messages,validate,tests}.rs` | 10.8 |
| `.github/workflows/ci.yml:263-300` | 10.9 |
| `.gitignore`, `crates/davinci-core/`, `packages/`, `package.json`, `bun.lock`, `crates/davinci-evals/README.md`, `fixtures/repos/*/Cargo.toml`, `.harness-opt-stage` | 10.10 |

---

### Task 10.1: The behavior gate fails on behavior regressions and on too few scored runs

**Finding fixed:** session/evals 4. `validate_behavior_gate` (`davinci-evals/src/main.rs:781-797`) runs only `scheduled_infrastructure_gate`. `evaluate_gate` and `RegressionBudget` (`behavior/gate.rs:7-60`: 85% minimum pass rate, turn and tool budgets) are only called from tests. Configuration failures and timeouts are excluded from both the pass-rate denominator and the infrastructure rate (`behavior/runner.rs:49-60, 149-171`); `minimum_behavioral_runs_met` is never called. A candidate prompt profile at 0% pass rate passes `behavior-live.yml:130`, and a run where every case times out passes with zero scored runs.

**Files:**
- Modify: `crates/davinci-evals/src/main.rs:781-797`
- Modify: `crates/davinci-evals/src/behavior/runner.rs:49-60, 149-171`

- [ ] **Step 1: Write the failing tests**

In `crates/davinci-evals/src/main.rs` tests (or a new `tests/gate.rs` that calls the binary's gate function through the library):

```rust
    #[test]
    fn a_collapsed_candidate_fails_the_gate() {
        let artifacts = fixture_artifacts(summary(0.92, 40), Some(summary(0.0, 40)));
        let err = validate_behavior_gate(artifacts.path().to_path_buf()).unwrap_err();
        assert!(err.contains("pass rate"), "{err}");
    }

    #[test]
    fn a_run_where_everything_timed_out_fails_the_gate() {
        let artifacts = fixture_artifacts(summary_all_timeouts(40), None);
        let err = validate_behavior_gate(artifacts.path().to_path_buf()).unwrap_err();
        assert!(err.contains("scored"), "{err}");
    }

    #[test]
    fn a_healthy_candidate_passes() {
        let artifacts = fixture_artifacts(summary(0.90, 40), Some(summary(0.91, 40)));
        validate_behavior_gate(artifacts.path().to_path_buf()).unwrap();
    }
```

`fixture_artifacts` writes a complete `ArtifactRoot` (manifest plus `disposition-summary.json`) into a temp dir; build it from the types `validate_behavior_gate` reads. `summary(pass_rate, scored_runs)` builds a `BehaviorSuiteSummary`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-evals a_collapsed_candidate a_run_where_everything_timed_out`
Expected: both FAIL (the gate passes).

- [ ] **Step 3: Implement**

```rust
/// Fewer scored runs than this is not evidence either way.
const MINIMUM_SCORED_RUNS: usize = 10;
/// Timeouts above this share of all runs fail the gate on their own.
const MAXIMUM_TIMEOUT_RATE: f64 = 0.20;

fn validate_behavior_gate(artifacts: PathBuf) -> Result<String, String> {
    let root = ArtifactRoot::new(&artifacts);
    let manifest = root.validate_complete()?;
    let bytes = fs::read(artifacts.join("disposition-summary.json"))
        .map_err(|error| format!("failed to read disposition summary: {error}"))?;
    let summary: DispositionSummaryArtifact = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode disposition summary: {error}"))?;
    for side in std::iter::once(&summary.baseline).chain(summary.candidate.as_ref()) {
        davinci_evals::behavior::scheduled_infrastructure_gate(side)?;
        davinci_evals::behavior::require_scored_runs(side, MINIMUM_SCORED_RUNS, MAXIMUM_TIMEOUT_RATE)?;
    }
    if let Some(candidate) = &summary.candidate {
        let result = davinci_evals::behavior::evaluate_gate(
            &summary.baseline,
            candidate,
            &davinci_evals::behavior::RegressionBudget::default(),
        );
        if !result.passed {
            return Err(format!("behavior regression: {}", result.violations.join("; ")));
        }
    }
    Ok(format!("behavior gate passed for run {} ({})", manifest.run_id, artifacts.display()))
}
```

`require_scored_runs` (new, in `behavior/runner.rs` or `gate.rs`): error when `scored_runs < minimum` ("only N scored runs; need at least M") or when `timeouts / total_runs > max_timeout_rate`. Wire `minimum_behavioral_runs_met` into it if it computes the same thing. Use `GateResult`'s real field names.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-evals` → PASS.

```bash
git add crates/davinci-evals
git commit -m "fix(evals): behavior gate enforces the regression budget and a minimum of scored runs"
```

---

### Task 10.2: The scheduled live eval fails when it is not configured

**Finding fixed:** session/evals 5. `behavior-live.yml:79-94` writes `CONFIGURATION_FAILURE` and exits 0 when the model variable or API key is missing, so the scheduled run shows green without running anything.

**Files:**
- Modify: `.github/workflows/behavior-live.yml:79-94`

- [ ] **Step 1: Implement**

In both `exit 0` branches, fail on scheduled runs and stay informational on manual ones:

```bash
            if [[ "${GITHUB_EVENT_NAME}" == "schedule" ]]; then
              exit 1
            fi
            exit 0
```

- [ ] **Step 2: Verify**

Run `actionlint .github/workflows/behavior-live.yml` if available (`go install github.com/rhysd/actionlint/cmd/actionlint@latest`), otherwise check YAML syntax with `python -c "import yaml,sys; yaml.safe_load(open('.github/workflows/behavior-live.yml'))"`. After merge, trigger the workflow once with `workflow_dispatch` and confirm the unconfigured provider is reported and not failed; the next scheduled run fails loudly if secrets are missing.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/behavior-live.yml
git commit -m "ci(evals): unconfigured scheduled live evals fail instead of passing"
```

---

### Task 10.3 (DECISION D6, recommended: delete): `davinci-parity` stops reporting fake parity

**Finding:** session/evals 6. The RPC check uses `unwrap_or(RpcCommand{kind:"get_state"})` against its own copy of the type, so it always passes; `diff_jsonl(&left, &left)` compares a file with itself; `maybe_parallel_run` never runs the TypeScript reference; the hello check only tests `frame.len() > 4`; the temp directory is never cleaned (`crates/davinci-parity/src/lib.rs:116-127`, `main.rs:32, 44-48`). A green CI shard says nothing about parity.

**Recommended (delete):**
- [ ] Remove `"crates/davinci-parity"` from workspace `members`, delete the directory, remove its CI job/matrix entry (`rg -n "davinci-parity" .github`), and remove references in docs (`rg -n "davinci-parity" docs README.md`).
- [ ] Run `cargo build --workspace` and `cargo test --workspace --no-run`; expected: success.
- [ ] Commit: `git commit -m "chore: remove davinci-parity, whose checks could not fail"`.

**Alternative (keep):** replace the crate with golden files: record real outputs from `vendor/davinci` for N RPC commands and session files, commit them under `crates/davinci-parity/golden/`, and make each check compare davinci's output to its golden file with a failing test per mismatch. Scope: a feature project; list it as its own plan.

---

### Task 10.4 (DECISION D4, recommended: remove the setting): the SQLite session backend

**Finding:** session 10. The only production caller (`main.rs:1173-1187`) upserts a session once at creation, with no entries, and ignores errors (`let _`). Nothing is written after that; writer leases are used only in tests and parity. A user who sets `session_backend = "sqlite"` gets a database of empty sessions.

**Recommended (remove the setting, keep the crate for later):**
- [ ] Delete the `session_backend` branch in `main.rs:1173-1187` and the setting field; if a user's settings still contain `sessionBackend: "sqlite"`, print one warning at startup ("the SQLite session backend was removed; sessions are stored as JSONL") and continue.
- [ ] Remove `davinci-session-sqlite` from `davinci-coding-agent`'s dependencies if nothing else uses it; keep the crate in the workspace with its tests (Task 3.10 fixes still apply to it) so it can be revived behind a storage interface later.
- [ ] Test: settings with `sessionBackend: "sqlite"` load, produce the warning once, and sessions are written as JSONL.
- [ ] Commit: `git commit -m "fix(sessions): drop the non-functional sqlite backend setting"`.

**Alternative (make it real):** define a `SessionStore` trait covering `append_entry`, `set_name`, `open`, `list`; implement it for JSONL and SQLite; route every write through it; enforce the writer lease on writes. Scope: a feature project; its own plan.

---

### Task 10.5 (DECISION D7, recommended: gate behind a cargo feature): the experimental server and client

**Findings:** session 11-15, 17. `davinci-server` answers prompts with `reply:{text}` (or `PI_SERVER_PROMPT_REPLY`) and never calls a model (`davinci-server/src/lib.rs:579-603`); `run_server` handles one connection and exits; only the first `--listen` address is bound; requests are handled before Hello under a shared "memory" connection (`:259`); `serve_stream` never calls `disconnect()` or `decoder.end()` (`:907-943`); `handshake_timeout` is unused; `updated_at` always equals `created_at` (`:651-652`). The transport is Unix-only; the client's handshake relies on a 2 s / 20 ms timing guess (`client/src/unix.rs:159-200`); `read_messages` decodes the wrong direction and spins on EOF (`client/src/lib.rs:142-153`); binding sets an existing parent directory to 0700 (`server/src/unix.rs:140-160`). Reachable today with `PI_EXPERIMENTAL=1`.

**Recommended (feature gate):**
- [ ] Add `[features] experimental-ipc = ["dep:davinci-server", "dep:davinci-client"]` to `davinci-coding-agent`, make both dependencies `optional = true`, and put `experimental.rs`'s server/client commands and `main.rs:13-86` behind `#[cfg(feature = "experimental-ipc")]`. Without the feature, `davinci server` / `davinci client` print "not available in this build".
- [ ] Fix the two items that are dangerous even as an experiment, in `davinci-server`: never `chmod` a directory the server did not create (create a private `0700` subdirectory for the socket instead), and reject any request before Hello.
- [ ] Delete `davinci-client`'s `read_messages` and the never-drained `pending` map (`client/src/lib.rs:91-101, 142-153`).
- [ ] Tests: `cargo build -p davinci-coding-agent` (feature off) does not compile `davinci-server`; `cargo test -p davinci-server` covers "request before Hello is refused" and "binding in an existing directory leaves its mode unchanged" (Unix).
- [ ] Commit: `git commit -m "chore(ipc): gate experimental server/client behind a feature; fix socket dir permissions"`.

**Alternative (make it real):** per-connection state, a real model call path, Windows named pipes, blocking reads with the configured handshake timeout. Scope: its own plan.

---

### Task 10.6: Session listing reads only headers and skips runtime sidecar logs

**Finding fixed:** session 20. `summarize_file` (`discovery.rs:137-172`) reads every `*.jsonl` file in full, including `*.runtime.jsonl` sidecar logs, and falls back to a full `JsonlSession::open` when the first line is not a v4 header (a sidecar log's first line is not, so it is treated as a v3 session). The server's `snapshot()` runs this on every hello, list, create and detach.

**Files:**
- Modify: `crates/davinci-session/src/discovery.rs:137-172, 278-285`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn runtime_sidecar_logs_are_not_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/w", None).unwrap();
        session.append_entry(test_entry("a")).unwrap();
        let sidecar = session.path.with_extension("runtime.jsonl");
        std::fs::write(&sidecar, "{\"type\":\"runtime\",\"schema_version\":1}\n").unwrap();
        let found = discover_sessions(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, session.path);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-session --lib runtime_sidecar_logs_are_not_sessions`
Expected: FAIL (2 sessions).

- [ ] **Step 3: Implement**

Where directory entries are filtered (`discovery.rs:278-285`), skip names ending in `.runtime.jsonl` (and `.lock`, `.v3.bak` from Phase 3). `summarize_file` keeps reading the whole file for `all_messages_text` because search needs it; add a cheaper `list_sessions_headers(dir)` that reads only the first line (via `BufReader::read_line`) for callers that only list (the server snapshot, `/resume` list), and switch those callers to it.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-session` → PASS.

```bash
git add crates/davinci-session/src/discovery.rs crates/davinci-server
git commit -m "fix(sessions): skip sidecar logs in discovery and list by header only"
```

---

### Task 10.7: Appending to a session costs O(1), not O(entries)

**Finding (performance):** session 21. `next_seq` (`davinci-session/src/lib.rs:380-388`) scans all entries and records on every append, and `commit_change` (`jsonl_repo.rs:591`) clones the whole `Session` per change; total cost is quadratic in session length.

**Budget:** appending 20,000 entries to a new session takes under 2 s in release (today: measure first).

**Files:**
- Modify: `crates/davinci-session/src/lib.rs:59-66, 380-388` (cache `max_seq`), `jsonl_repo.rs:~591`

- [ ] **Step 1: Measure**

```rust
    #[test]
    #[ignore = "performance budget; run with --release -- --ignored"]
    fn twenty_thousand_appends_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/w", None).unwrap();
        let started = std::time::Instant::now();
        for i in 0..20_000 {
            session.append_entry(test_entry(&format!("e{i}"))).unwrap();
        }
        let elapsed = started.elapsed();
        eprintln!("{elapsed:?}");
        assert!(elapsed < std::time::Duration::from_secs(2));
    }
```

Note that each append `sync_all`s (durability); on a slow disk the fsyncs alone may exceed the budget. If so, measure with the fsync count held constant and set the budget on CPU time (exclude fsync by measuring with a `tmpfs`/ramdisk path on Linux CI).

- [ ] **Step 2: Implement**

Add `max_seq: u64` to `JsonlSession` (private), set while loading (max over entries and records) and updated on every append; `next_seq` returns `self.max_seq + 1`. In `jsonl_repo.rs`, replace the per-change `Session` clone with an undo record of the single change (or apply the change after the durable write succeeds, so no rollback copy is needed).

- [ ] **Step 3: Run the budget and the suite, commit**

```bash
git add crates/davinci-session
git commit -m "perf(sessions): O(1) sequence numbers and no whole-session clone per change"
```

---

### Task 10.8: Protocol crate: no per-block copy in the frame decoder; uncompiled files removed

**Findings fixed:** session 16 and 18. `framing.rs:167` does `current_payload_block = payload_blocks.last().cloned()`, copying each 64 KiB block just to track its length (bounded by the 16 MiB frame cap, so not a DoS). `crates/davinci-protocol/src/{messages.rs,validate.rs,tests.rs}` (~1,300 lines) are not declared in `lib.rs:3-6`, never compile, and use an older API.

**Files:**
- Modify: `crates/davinci-protocol/src/framing.rs:~160-175`
- Delete: `crates/davinci-protocol/src/messages.rs`, `validate.rs`, `tests.rs`

- [ ] **Step 1: Delete the uncompiled files**

Confirm they are uncompiled: `rg -n "mod (messages|validate|tests)" crates/davinci-protocol/src/lib.rs` returns nothing. Before deleting `tests.rs`, read it; if a test there covers behavior the compiled `codec.rs` tests do not (for example a specific CBOR edge case), port that one test into `codec.rs` against the current API first.

- [ ] **Step 2: Track the block length instead of cloning**

Replace the `Option<Vec<u8>>` `current_payload_block` with `current_block_len: usize`, updated where bytes are appended to the last block. Keep the existing framing tests; add one that decodes a 16 MiB frame and asserts the result is byte-identical.

- [ ] **Step 3: Run and commit**

Run: `cargo test -p davinci-protocol` → PASS.

```bash
git add -A crates/davinci-protocol
git commit -m "chore(protocol): remove uncompiled modules; track block length without copying"
```

---

### Task 10.9: CI runs the storage and IPC tests on Windows, with `--locked`

**Finding fixed:** session 23. The full per-crate test jobs run only on Ubuntu and without `--locked` (`ci.yml:263-300`); Windows runs hand-picked subsets, so session, SQLite, protocol, client and server tests never run on the platform development happens on. `sync_parent` is a no-op on Windows and untested there.

**Files:**
- Modify: `.github/workflows/ci.yml:263-300`

- [ ] **Step 1: Implement**

Turn the per-package job into a two-OS matrix and lock it:

```yaml
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest]
        package:
          # ... existing list, plus davinci-sys ...
    runs-on: ${{ matrix.os }}
```

and use `cargo test -p "${{ matrix.package }}" --locked` / `cargo build ... --locked`. `umask 077` is bash-only; keep `shell: bash` (Git Bash exists on windows-latest). Exclude `davinci-voice` on Windows if it needs native toolchains the runner lacks (it has its own job at `ci.yml:225-236`).

Add a release-build step on ubuntu that runs the Task 2.15 check:

```yaml
      - name: Release build contains no credential fixtures
        if: matrix.os == 'ubuntu-latest' && matrix.package == 'davinci-coding-agent'
        shell: bash
        run: |
          cargo build --release -p davinci-coding-agent --bin davinci --locked
          ! strings target/release/davinci | grep -E "PI_OAUTH_FIXTURE|PI_OAUTH_REFRESH_URL|PI_OAUTH_TOKEN_URL|PI_MCP_FIXTURE|PI_SECURITY_SCAN_FIXTURE"
```

- [ ] **Step 2: Verify**

Push the branch and check that every matrix cell runs. A Windows failure that appears here is a real bug the old CI hid: fix it in its own commit (with a test) or, if it needs investigation, mark only that test `#[cfg_attr(windows, ignore = "tracked in <issue>")]` with an issue link. Do not skip whole packages.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: run every package's tests on Windows and Linux with --locked"
```

---

### Task 10.10: Repository hygiene

**Findings:** session 22, 24. Items, each its own small commit:

- [ ] **`.gitconfig-gh`** (untracked at the repo root, not ignored; contains token/helper/oauth keys by keyword, contents not printed): add `.gitconfig-gh` to `.gitignore`. Tell Julien to move the file out of the repository and rotate any token it contains if it was ever committed (`git log --all -- .gitconfig-gh` shows whether it was; run that and report the answer, do not print the file).
- [ ] **Untracked build clutter at the root** (`.expansion-clone/`, `.expansion-objects/`, `final-git/`, `final-worktree/`, `program-git/`, `program-worktree/`, `.codex-p9/`, `*.patch`): add ignore entries so they cannot be committed by accident. Deleting them is Julien's call (the index lists the cleanup command).
- [ ] **(DECISION D8) `crates/davinci-core` and `packages/`** (19 and 8 tracked files, referenced by no `Cargo.toml` or workflow; plus the root `package.json` workspaces entry and `bun.lock`): recommended delete; alternative keep with a README saying what they are for. Before deleting, `rg -n "davinci-core|packages/" --glob '!target'` must show no live reference.
- [ ] **`crates/davinci-evals/README.md`** points to a `harness.rs` that does not exist: fix the path to the real module.
- [ ] **`fixtures/repos/*/Cargo.toml`** have no `[workspace]` table, so `cargo` run inside them may resolve to the parent workspace (SUSPECTED): add an empty `[workspace]` table to each and confirm `cargo metadata --manifest-path fixtures/repos/<x>/Cargo.toml` reports that fixture as its own workspace root.
- [ ] **`.harness-opt-stage`** is tracked: check what writes it (`rg -n "harness-opt-stage"`); if it is generated state, `git rm --cached` it and ignore it.

Commit each item separately with a `chore:` message.

---

## Phase 10 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

CI green on both operating systems.
