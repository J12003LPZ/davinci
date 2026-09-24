# Phase 8: Native Extensions Implementation Plan (`crates/davinci-coding-agent/src/native_extensions/`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Graph runs stay small on disk and in memory, can be cancelled, get pruned, and never leak processes; LSP, security scan and learning stop degrading over a session.

**Architecture:** File contents move out of graph checkpoints into one content-addressed blob store per project (`.davinci/graph/blobs/<sha256>`), shared by runs and garbage-collected when runs are pruned. Child processes use `davinci_sys::process` (Task 1.3). Shared state that the graph reads becomes `Arc` handles.

**Tech Stack:** Rust 1.83.

Depends on Phase 1. Task 2.13 (graph results strip `continuation`) should land first; this phase removes the contents at the source.

All paths below are relative to `crates/davinci-coding-agent/src/native_extensions/`.

---

## File map

| File | Tasks |
|---|---|
| `graph/mutation.rs:20-26, 98-135, 171-270, 500-510` | 8.1, 8.6 |
| `graph/blobs.rs` (create) | 8.1 |
| `graph/store.rs:136-150, 150-272, 346-375` | 8.1, 8.2, 8.3 |
| `graph/controller.rs:153-198, 245, 259, 338-373, 1120-1135` | 8.2, 8.6 |
| `graph/mod.rs:834-846, 1125-1145, 1425-1440` | 8.4, 8.9, 8.10 |
| `graph/types.rs:436-447` | 8.4 |
| `graph/process.rs:52-97, 236-262`, `graph/worker.rs:526, 611-646`, `graph/verify.rs:251-256` | 8.5, 8.6 |
| `mod.rs:340-484, 371-373, 575-582`, `vector_memory.rs:794-806`, `token_governor.rs:932-951` | 8.7, 8.11 |
| `language_intelligence/transport.rs:240-256`, `language_intelligence/manager.rs:135-142, 265-280` | 8.8 |
| `security_scan.rs:239-285, 393`, `security_scan/controller.rs:245-270`, `security_scan/analyzers.rs:265` | 8.11, 8.12 |
| `learning/store.rs:114-180, 263` | 8.13 |
| `graph/definitions.rs:1055-1165`, `graph/store.rs:357-362, 579-587` | 8.14 |
| `git_intelligence/runner.rs:296-317` | 8.15 |
| `mod.rs:133-288`, `workspace_snapshot/mod.rs:288, 325, 345`, `vector_memory.rs:441-457` | 8.16 |

---

### Task 8.1: Graph checkpoints hold hashes; file contents live once in a content-addressed blob store

**Finding fixed:** native-extensions 1 (checked in source; today's runs in this repo are small, 824 KB for 12 runs, so this is HIGH, not CRITICAL). `MutationBaseline.contents` (`graph/mutation.rs:22-25`: every tracked and non-ignored untracked file up to 512 KB, as `Vec<u8>`) is stored in `DeliveryCheckpoint.baseline`, `attempt_baseline`, `GraphContinuation.saved_baseline` and `saved_attempt_baselines`, all serialized into `state.json` with `to_vec_pretty`, where a byte array becomes one number per line (10-20× the source size). A 20 MB repo gives a state.json of hundreds of MB, rewritten with fsync on every checkpoint and cloned on every `snapshot()`. `capture_baseline` holds the whole repo in memory, and `capture_graph_delta` does it again (`mutation.rs:171-225`).

**Design:**
- Blob store: `<project graph dir>/blobs/<sha256>`, written with `davinci_sys::fs::atomic_write` only if absent. Content-addressed, so identical files across runs and attempts are stored once.
- `MutationBaseline.contents` becomes `#[serde(default, skip_serializing)]`: new checkpoints never contain bytes; old `state.json` files that do still load (their bytes are moved into the blob store on first save, see Step 5).
- `capture_baseline` streams: read one file, hash it, write the blob if ≤ 512 KB and absent, drop the bytes. Peak memory is one file.
- `capture_graph_delta` reads old bytes through `MutationBaseline::old_bytes(path, blob_dir)`, and reads current file contents only for files whose hash changed.
- `state.json` is written compact (`to_vec`), not pretty.

**Files:**
- Create: `graph/blobs.rs` (declare `mod blobs;` in `graph/mod.rs`)
- Modify: `graph/mutation.rs:20-26, 171-270, 500-510`
- Modify: `graph/store.rs:346-375` (`save_run`, `load_run`), `graph/controller.rs:245, 259` (callers of `capture_baseline`)

**Interfaces:**
- Produces:
  - `blobs::dir(cwd: &Path) -> PathBuf`
  - `blobs::put(dir: &Path, hash: &str, bytes: &[u8]) -> std::io::Result<()>`
  - `blobs::get(dir: &Path, hash: &str) -> Option<Vec<u8>>`
  - `MutationBaseline::old_bytes(&self, path: &str, blob_dir: &Path) -> Vec<u8>`
  - `capture_baseline(cwd: &Path) -> Result<MutationBaseline, String>` (same signature; now writes blobs)

- [ ] **Step 1: Write the failing tests**

`graph/mutation.rs` tests:

```rust
    #[test]
    fn baseline_serializes_without_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "secret-ish content").unwrap();
        let baseline = capture_baseline(dir.path()).unwrap();
        let json = serde_json::to_string(&baseline).unwrap();
        assert!(!json.contains("contents"), "{json}");
        assert!(json.contains("a.txt"));
        let hash = &baseline.files["a.txt"].hash;
        assert_eq!(
            blobs::get(&blobs::dir(dir.path()), hash).unwrap(),
            b"secret-ish content"
        );
    }

    #[test]
    fn delta_reads_old_bytes_from_the_blob_store() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let baseline = capture_baseline(dir.path()).unwrap();
        let reloaded: MutationBaseline =
            serde_json::from_str(&serde_json::to_string(&baseline).unwrap()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        let delta = capture_graph_delta(dir.path(), &reloaded).unwrap();
        let patch = format!("{delta:?}");
        assert!(patch.contains("-one") && patch.contains("+two"), "{patch}");
    }

    #[test]
    fn legacy_inline_contents_still_deserialize() {
        let legacy = r#"{"files":{"a.txt":{"hash":"h","len":3}},"contents":{"a.txt":[97,98,99]}}"#;
        let baseline: MutationBaseline = serde_json::from_str(legacy).unwrap();
        assert_eq!(baseline.contents["a.txt"], b"abc");
    }
```

`graph/store.rs` tests (next to `atomic_write_restores_old_content_when_publish_is_interrupted`, using the existing `sample_run(cwd, run_id, goal)` at `store.rs:740`):

```rust
    #[test]
    fn saved_state_is_compact_and_has_no_byte_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = sample_run(dir.path(), "r1", "goal");
        create_run_dir(dir.path(), "r1").unwrap();
        save_run(&mut run).unwrap();
        let raw = std::fs::read_to_string(run_dir(dir.path(), "r1").join("state.json")).unwrap();
        assert!(!raw.contains("\n  "), "state.json must be compact");
        assert!(!raw.contains("\"contents\""));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph::mutation native_extensions::graph::store`
Expected: FAIL (`contents` present; `blobs` missing).

- [ ] **Step 3: Implement `blobs.rs`**

```rust
//! Content-addressed file snapshots for graph runs. A checkpoint records
//! each file's hash; the bytes live here once, shared by every run and
//! attempt that saw the same content.

use std::path::{Path, PathBuf};

pub fn dir(cwd: &Path) -> PathBuf {
    super::store::graph_root(cwd).join("blobs")
}

fn path(dir: &Path, hash: &str) -> Option<PathBuf> {
    // Hashes are lowercase SHA-256 hex; anything else is refused so a
    // corrupt checkpoint cannot name a path outside the store.
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| dir.join(&hash[..2]).join(hash))
}

pub fn put(dir: &Path, hash: &str, bytes: &[u8]) -> std::io::Result<()> {
    let Some(target) = path(dir, hash) else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid blob hash"));
    };
    if target.exists() {
        return Ok(());
    }
    davinci_sys::fs::atomic_write(&target, bytes)
}

pub fn get(dir: &Path, hash: &str) -> Option<Vec<u8>> {
    std::fs::read(path(dir, hash)?).ok()
}
```

`graph_root(cwd)` is whatever `store.rs` uses for `.davinci/graph` (with its `.pi` fallback); expose it as `pub(super)` if it is private.

- [ ] **Step 4: Implement baseline and delta**

`mutation.rs` struct:

```rust
pub struct MutationBaseline {
    pub files: BTreeMap<String, FileFingerprint>,
    /// Legacy only: checkpoints written before the blob store kept bytes
    /// inline. Read so old runs resume; never written.
    #[serde(default, skip_serializing)]
    pub contents: BTreeMap<String, Vec<u8>>,
}

impl MutationBaseline {
    pub fn old_bytes(&self, path: &str, blob_dir: &Path) -> Vec<u8> {
        if let Some(bytes) = self.contents.get(path) {
            return bytes.clone();
        }
        self.files
            .get(path)
            .and_then(|fingerprint| super::blobs::get(blob_dir, &fingerprint.hash))
            .unwrap_or_default()
    }
}
```

`capture_baseline`:

```rust
pub fn capture_baseline(cwd: &Path) -> Result<MutationBaseline, String> {
    let blob_dir = super::blobs::dir(cwd);
    let mut files = BTreeMap::new();
    for rel_path in list_workspace_files(cwd) {
        if is_transaction_journal(&rel_path) {
            continue;
        }
        let Ok(bytes) = std::fs::read(cwd.join(&rel_path)) else {
            continue;
        };
        let hash = sha256_hex(&bytes);
        if bytes.len() <= 512 * 1024 {
            super::blobs::put(&blob_dir, &hash, &bytes).map_err(|err| err.to_string())?;
        }
        files.insert(rel_path, FileFingerprint { hash, len: bytes.len() as u64 });
    }
    Ok(MutationBaseline { files, contents: BTreeMap::new() })
}
```

`capture_graph_delta`: build `current_map` of fingerprints only (hash and length; drop bytes after hashing). For each path whose hash differs from the baseline, read the current file again to build its patch, and get old bytes with `baseline.old_bytes(path, &blob_dir)` (replacing `baseline.contents.get(path).cloned().unwrap_or_default()` at lines 245 and 265). Line 506 (`baseline.contents.insert(`) is a test or fixture path: switch it to `blobs::put` plus a `files` entry.

- [ ] **Step 5: Migrate legacy contents and write compact state**

In `save_run`, before serializing, move any legacy inline bytes into the store (so an old run shrinks the first time it is saved):

```rust
    let blob_dir = super::blobs::dir(&cwd);
    for baseline in snapshot.baselines_mut() {
        for (path, bytes) in std::mem::take(&mut baseline.contents) {
            if let Some(fingerprint) = baseline.files.get(&path) {
                super::blobs::put(&blob_dir, &fingerprint.hash, &bytes)?;
            }
        }
    }
    let content = serde_json::to_vec(&snapshot)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
```

`GraphRun::baselines_mut(&mut self) -> impl Iterator<Item = &mut MutationBaseline>` walks `continuation.{delivery, completed_delivery}.{baseline, attempt_baseline}`, `saved_baseline` and `saved_attempt_baselines` values; add it to `types.rs` / `continuation.rs`.

Also in `save_run`, write `graph.json` only when missing or different (the review found it rewritten on every save when `saved_definition` is `None`):

```rust
        let bytes = serde_json::to_vec_pretty(definition)?;
        if std::fs::read(&graph_path).ok().as_deref() != Some(bytes.as_slice()) {
            atomic_write(&graph_path, &bytes)?;
        }
```

- [ ] **Step 6: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph`
Expected: PASS. Rollback/resume tests that construct baselines with inline `contents` still pass because `old_bytes` reads legacy contents first.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "fix(graph): store baseline bytes once in a content-addressed blob store, compact checkpoints"
```

---

### Task 8.2: Progress checkpoints are debounced and written outside the run lock

**Finding fixed:** native-extensions 3. `on_progress` (`graph/controller.rs:1128`) calls `checkpoint`, which fsyncs and renames `state.json` for every worker progress line while holding the run mutex (`controller.rs:338-373`). Parallel workers serialize behind disk I/O; on Windows a transient rename failure sets a terminal `persistence_error` (Task 3.9 adds the rename retry; this task removes the write storm).

**Behavior:** progress lines update in-memory state immediately; the disk checkpoint for *progress* happens at most once every 2 s per run. Checkpoints for state transitions (task started, finished, failed, phase change) are still written immediately. Serialization happens on a clone taken under the lock; the write happens after the lock is released.

**Files:**
- Modify: `graph/controller.rs:338-373, 1120-1135`

**Interfaces:**
- Produces: `fn checkpoint_progress(&self, note: &str)` (debounced) next to the existing `checkpoint`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_burst_of_progress_lines_writes_state_once() {
        let (controller, cwd) = fixture_controller();
        let path = super::store::run_dir(&cwd, controller.run_id()).join("state.json");
        controller.checkpoint(Some("start"));
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        for i in 0..200 {
            controller.checkpoint_progress(&format!("line {i}"));
        }
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(first, after, "progress within 2s must not rewrite state.json");
    }
```

(`fixture_controller` builds a controller over a temp dir the way the controller tests do.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib a_burst_of_progress_lines`
Expected: FAIL to compile, then FAIL on the assertion if `checkpoint_progress` just calls `checkpoint`.

- [ ] **Step 3: Implement**

```rust
const PROGRESS_CHECKPOINT_EVERY: Duration = Duration::from_secs(2);

    fn checkpoint_progress(&self, note: &str) {
        let due = {
            let mut last = self.last_progress_checkpoint.lock().unwrap_or_else(|e| e.into_inner());
            let due = last.map_or(true, |at| at.elapsed() >= PROGRESS_CHECKPOINT_EVERY);
            if due {
                *last = Some(Instant::now());
            }
            due
        };
        if due {
            self.checkpoint(Some(note));
        }
    }
```

Add `last_progress_checkpoint: Mutex<Option<Instant>>` to the controller. Replace the `self.checkpoint(Some(&format!("{task_id}: {line}")))` in the progress callback (`controller.rs:1128`) with `self.checkpoint_progress(...)`.

In `checkpoint` itself (`:338-373`): take the lock, clone the `GraphRun` (after Task 8.1 it is small), release the lock, then call `store::save_run` on the clone, then briefly re-lock to copy back `updated_at` and any `persistence_error`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/controller.rs
git commit -m "perf(graph): debounce progress checkpoints and write them outside the run lock"
```

---

### Task 8.3: Finished graph runs are pruned, and unreferenced blobs are collected

**Finding fixed:** native-extensions 4. `prune_finished_runs` keeps any run whose `state.json` or artifact is over 256 KB (`graph/store.rs:140, 249-265`: "cannot scan, so assume it has operation references"). With inline contents almost every real run exceeded that, so `.davinci/graph/runs` grew without limit and `list_runs` (which loads every `state.json`, twice per `status()`) got slower. After Task 8.1 `state.json` is small, so the existing rule works; this task adds the blob garbage collection and a regression test.

**Files:**
- Modify: `graph/store.rs:150-272` (`prune_finished_runs`)

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn old_finished_runs_are_pruned_and_their_blobs_collected() {
        let dir = tempfile::tempdir().unwrap();
        let blob_dir = super::blobs::dir(dir.path());
        let mut orphan_hash = String::new();
        for i in 0..(RETAINED_RUNS + 3) {
            let run_id = format!("r{i:03}");
            create_run_dir(dir.path(), &run_id).unwrap();
            let mut run = sample_finished_run(dir.path(), &run_id);
            let bytes = format!("file for {run_id}").into_bytes();
            let hash = sha256_hex(&bytes);
            super::blobs::put(&blob_dir, &hash, &bytes).unwrap();
            run.set_baseline_file("a.txt", &hash, bytes.len() as u64);
            if i == 0 {
                orphan_hash = hash;
            }
            save_run(&mut run).unwrap();
        }
        prune_finished_runs(dir.path());
        assert_eq!(list_runs(dir.path()).len(), RETAINED_RUNS);
        assert!(super::blobs::get(&blob_dir, &orphan_hash).is_none());
    }
```

(`sample_finished_run` = the existing `sample_run` with `phase = Done`; `set_baseline_file` is a `#[cfg(test)]` helper that puts one fingerprint into the run's `saved_baseline`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib old_finished_runs_are_pruned`
Expected: FAIL on the blob assertion (runs may already prune after Task 8.1).

- [ ] **Step 3: Implement**

At the end of `prune_finished_runs`, after deleting runs:

```rust
    // Blobs are shared by runs; keep exactly the ones a remaining run names.
    let referenced: std::collections::HashSet<String> = list_runs(cwd)
        .iter()
        .filter_map(|summary| load_run(cwd, &summary.run_id))
        .flat_map(|run| run.baseline_hashes())
        .collect();
    let blob_dir = super::blobs::dir(cwd);
    if let Ok(shards) = fs::read_dir(&blob_dir) {
        for shard in shards.flatten() {
            if let Ok(entries) = fs::read_dir(shard.path()) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !referenced.contains(&name) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
```

`GraphRun::baseline_hashes()` collects `fingerprint.hash` from every baseline (`baselines_mut` from Task 8.1, in a read-only form). A run being created concurrently could reference a blob written a moment ago but not yet in its saved `state.json`; `create_run_dir` calls `prune_finished_runs` before the new run writes anything, and graph runs are exclusive per workspace (`WorkspaceLease`), so no other run writes blobs during pruning.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph::store` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "fix(graph): garbage-collect blobs of pruned runs"
```

---

### Task 8.4: The `graph_run` tool can be cancelled and has finite default limits

**Finding fixed:** native-extensions 5. `execute_tool` for `graph_run` (`graph/mod.rs:834-846`) has no `ToolContext` abort; `options.abort` is set only by `/graph-abort` or `abort_all_runs`. Every default is unlimited (`graph/types.rs:436-447`: no worker timeout, no verification timeout, no deadline, no cost cap). A plan-supplied `npm test` that starts a watcher blocks the agent turn until session shutdown, with unbounded spend.

**Defaults for runs started by the model through the tool** (the `/graph` command keeps the user's settings): worker timeout 20 min, verification timeout 10 min, run deadline 2 h, cost cap from settings `graph.maxCostUsd` or 5 USD. Settings override each (`graph.workerTimeoutMs`, `graph.verifyTimeoutMs`, `graph.runDeadlineMs`).

**Files:**
- Modify: `graph/mod.rs:834-846` (and the native tool dispatch signature if it does not pass `ToolContext`)
- Modify: `graph/types.rs:436-447` (add `RunOptions::for_tool(settings) -> RunOptions`)

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn tool_runs_have_finite_limits() {
        let options = types::RunOptions::for_tool(&GraphSettings::default());
        assert!(options.worker_timeout_ms.is_some());
        assert!(options.verify_timeout_ms.is_some());
        assert!(options.run_deadline_ms.is_some());
        assert!(options.max_cost_usd.is_some());
    }

    #[test]
    fn tool_abort_cancels_a_running_graph() {
        let host = fixture_graph_host_with_slow_worker();
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = abort.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let result = host.execute_tool_with_abort("graph_run", &serde_json::json!({"goal": "x"}), abort);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(result.unwrap().is_error);
    }
```

(Field names must match `RunOptions`; the slow worker fixture uses the existing graph fixture worker with a sleep.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib tool_runs_have_finite_limits tool_abort_cancels`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `RunOptions::for_tool(settings)` fills the four limits from settings or the defaults above.
- Thread the tool call's abort (`ToolContext.abort`, an `Arc<AtomicBool>`) into `options.abort`. If the native tool entry point does not receive a `ToolContext`, extend `NativeExtensionHost::execute_tool` to take `Option<Arc<AtomicBool>>` and pass it from the agent's dispatch (search the call site for `execute_tool(` in `native_tools.rs` / `runtime_host.rs`). Simplest wiring: spawn a watcher that forwards `tool_abort` to the run's `active.abort` every 50 ms until the run returns.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph crates/davinci-coding-agent/src/native_tools.rs
git commit -m "fix(graph): tool-started runs obey the turn's abort and have finite limits"
```

---

### Task 8.5: Graph workers and verification shells run in their own process group; Windows commands keep their quoting

**Findings fixed:** native-extensions 6 and 22. On Unix, workers (`graph/worker.rs:526`) and verification shells (`graph/process.rs:250-262`) are not group leaders, so `kill(-pid)` (`process.rs:90-97`) hits nothing; timed-out `cargo test`/`npm test` children keep running and keep the pipes open, so the `pump` threads (`process.rs:52-69`) never exit. On Windows `shell_command` uses `cmd /C` + `.arg(command)`, which escapes inner quotes MSVC-style (`\"`); cmd.exe does not parse that, so `cargo test -- "a b"` arrives mangled (SUSPECTED; the test below settles it).

**Files:**
- Modify: `graph/process.rs:236-262` (`shell_command`), `graph/worker.rs:~526` (worker spawn), `graph/process.rs:90-97` (kill)

- [ ] **Step 1: Write the failing tests**

```rust
    #[cfg(windows)]
    #[test]
    fn cmd_receives_quoted_arguments_intact() {
        let dir = tempfile::tempdir().unwrap();
        let output = shell_command(r#"echo "a b""#, dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), r#""a b""#);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_the_shells_children() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-running");
        let script = format!("(sleep 2; touch {}) & wait", marker.display());
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _ = run_child(shell_command(&script, dir.path()), &abort, 300, |_| {}, |_| {});
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(!marker.exists(), "the backgrounded child survived the timeout");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib graph::process::tests`
Expected: the platform's test FAILS.

- [ ] **Step 3: Implement**

```rust
pub fn shell_command(command: &str, cwd: &std::path::Path) -> Command {
    let mut process = if cfg!(windows) {
        let mut process = Command::new("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // cmd.exe does its own parsing; pass the line untouched.
            process.raw_arg("/C").raw_arg(command);
        }
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    process.current_dir(cwd);
    davinci_sys::process::set_own_process_group(&mut process);
    process
}
```

(`CommandExt::raw_arg` is stable since Rust 1.62.) Call `davinci_sys::process::set_own_process_group(&mut command)` at the worker spawn in `worker.rs` too, and replace the kill at `process.rs:90-97` with `davinci_sys::process::kill_tree(pid)`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "fix(graph): process groups for workers and shells, raw cmd.exe arguments on Windows"
```

---

### Task 8.6: Graph output and diff reading stay bounded; git calls time out

**Findings fixed:** native-extensions 10, 11 (memory half is Task 8.1), 21.
- `default_get_diff` (`graph/controller.rs:153-198`) reads each untracked file fully (`fs::read` at `:173`) before checking its size (`:182`); a multi-GB dataset is loaded to print "contents not shown". Its git call, and those in `graph/mutation.rs:101, 115`, `graph/replay.rs:184, 191`, use `.output()` with no timeout (a git lock, credential prompt or fsmonitor hangs the review).
- Verification and worker stderr are buffered without limit (`graph/verify.rs:251-256`, `graph/worker.rs:611, 644-646`) though only a 4000-character tail is used.

**Files:**
- Modify: `graph/controller.rs:153-198`, `graph/mutation.rs:98-135`, `graph/replay.rs:184-191`, `graph/verify.rs:251-256`, `graph/worker.rs:611-646`

**Interfaces:**
- Produces: `graph::git::run(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String>` (30 s timeout, 32 MB cap, sanitized env; wraps `git_intelligence::runner::run_status_interruptible` or `davinci_sys::process::run_bounded`)
- Produces: `struct TailBuffer { cap: usize, buf: VecDeque<u8> }` with `push(&mut self, bytes: &[u8])` and `text(&self) -> String`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn large_untracked_files_are_not_read_for_the_diff() {
        let dir = git_fixture_repo();
        let big = dir.path().join("dataset.bin");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(64 * 1024 * 1024).unwrap(); // sparse, instant
        let diff = default_get_diff(dir.path()).unwrap();
        assert!(diff.contains("dataset.bin"));
        assert!(diff.contains("not shown"));
    }

    #[test]
    fn tail_buffer_keeps_only_the_end() {
        let mut tail = TailBuffer::new(8);
        tail.push(b"0123456789");
        tail.push(b"ab");
        assert_eq!(tail.text(), "456789ab");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib large_untracked_files tail_buffer_keeps`
Expected: FAIL (first: slow and memory-heavy; second: missing type).

- [ ] **Step 3: Implement**

- In `default_get_diff`, check `fs::metadata(&path)?.len()` first; read only files under the size limit, with `File::open(..)?.take(limit + 1)`.
- `graph/git.rs` (new): `run(cwd, args)` building `git` with the same env sanitation as `git_intelligence/runner.rs:185-212` and `run_bounded(.., RunLimits { timeout: 30 s, output_cap: 32 MB })`; return an error on timeout or non-zero status. Replace every `Command::new("git")...output()` in `graph/` with it.
- `TailBuffer`: a `VecDeque<u8>` that drops from the front beyond `cap`; use it for verification and worker stderr with `cap = 64 * 1024` (the consumer uses the last 4000 characters; 64 KB leaves room for multi-byte text and the diagnostic-line extractor).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "fix(graph): size-check before reading, time-limited git, bounded stderr tails"
```

---

### Task 8.7: The graph sees live memory and governor state, not a copy from startup

**Finding fixed:** native-extensions 8. The graph receives copies of `VectorMemory` and `TokenGovernor` taken at host construction (`mod.rs:371-373`; `vector_memory.rs:794-806` keeps records in a plain `Vec`; `token_governor.rs:932-951` holds counters by value). Memories indexed after startup never reach graph context packets; `run.ecosystem_stats.governor_bytes_omitted` and `prunings` stay 0; `/governor-reset` resets only the host's copy.

**Files:**
- Modify: `vector_memory.rs:~790-810`, `token_governor.rs:~925-955`, `mod.rs:340-484` (construction), graph code that reads them

**Interfaces:**
- Produces: `pub type SharedVectorMemory = Arc<Mutex<VectorMemory>>`, `pub type SharedTokenGovernor = Arc<Mutex<TokenGovernor>>`; the host and the graph hold clones of the same `Arc`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn memory_indexed_after_startup_reaches_the_graph() {
        let host = fixture_native_host();
        host.vector_memory().lock().unwrap().index_text("late fact", MemoryKind::Fact).unwrap();
        let packet = host.graph_context_packet("anything");
        assert!(packet.contains("late fact"));
    }
```

(Use the real accessors; `graph_context_packet` stands for whatever function the graph calls to build context from memory.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib memory_indexed_after_startup`
Expected: FAIL.

- [ ] **Step 3: Implement**

Wrap each in `Arc<Mutex<_>>` once at construction; replace the `.clone()` handed to the graph with `Arc::clone`. Every read site locks briefly (`lock().unwrap_or_else(|e| e.into_inner())`). Keep locks short: copy out what is needed, then release, so a graph worker cannot hold the host's memory lock during a model call.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "fix(native): share memory and governor state between host and graph"
```

---

### Task 8.8: One slow LSP request does not kill the language server; the restart budget recovers

**Finding fixed:** native-extensions 9. On a request timeout, `transport.rs:244-254` sends `$/cancelRequest`, then `self.shared.fail(error)` and `self.terminate()`: the whole server dies. `manager.rs:273` refuses after `starts >= 2`, and `starts` is never reset, so after two timeouts all `lsp_*` tools fail until davinci restarts. The timeout is capped at 30 s (`manager.rs:139`); tsserver's first cold `references` call on a large monorepo takes longer.

**Behavior:** a timed-out request is cancelled and returns `request_timeout`; the server keeps running. `terminate` happens only on transport failure (pipe closed, protocol error). `starts` resets to 0 after the server has been healthy for 5 minutes. The first request after a start gets a separate 120 s budget.

**Files:**
- Modify: `language_intelligence/transport.rs:240-256`, `language_intelligence/manager.rs:135-142, 265-280`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_request_timeout_leaves_the_server_running() {
        let transport = fixture_transport_with_slow_method("textDocument/references", Duration::from_secs(2));
        let err = transport.request("textDocument/references", json!({}), Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.code, "request_timeout");
        assert!(transport.is_alive());
        assert!(transport.request("initialize", json!({}), Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn restart_budget_recovers_after_a_healthy_period() {
        let mut slot = ServerSlot { starts: 2, healthy_since: Some(Instant::now() - Duration::from_secs(301)), ..Default::default() };
        slot.refresh_budget();
        assert_eq!(slot.starts, 0);
    }
```

The fixture transport is an in-process fake LSP server (a thread speaking JSON-RPC over pipes); build it from the existing transport tests.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib language_intelligence`
Expected: FAIL.

- [ ] **Step 3: Implement**

In the timeout arm: keep the `$/cancelRequest` notify and the `pending.remove(&id)`, return the `request_timeout` error, and delete `self.shared.fail(error.clone()); self.terminate();`. Late replies for the removed id must be ignored by the reader (check it drops responses with unknown ids).

`ServerSlot` gains `healthy_since: Option<Instant>` (set when a session starts), and `refresh_budget()` resets `starts` when `healthy_since` is older than 5 minutes; call it before the `starts >= 2` check. Pass `Duration::from_secs(120)` for the first request after start, the configured value (still clamped to 30 s) afterwards.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib language_intelligence` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/language_intelligence
git commit -m "fix(lsp): cancel slow requests without killing the server; restart budget recovers"
```

---

### Task 8.9: A bare `/graph` shows a cancelled or blocked run instead of restarting it

**Finding fixed:** native-extensions 15. `/graph` with no goal resumes the latest run unless it is Done or Paused (`graph/mod.rs:1432-1438`); `resume()` rejects only Done. Typing `/graph` to look at an aborted run spawns workers again and spends money.

**Files:**
- Modify: `graph/mod.rs:1432-1438`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn bare_graph_command_shows_a_cancelled_run() {
        let host = fixture_graph_host_with_latest_run(types::Phase::Cancelled);
        let reply = host.handle_command("graph", "").unwrap();
        assert!(reply.contains("cancelled") || reply.contains("Cancelled"));
        assert_eq!(host.workers_spawned(), 0);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib bare_graph_command_shows`
Expected: FAIL (workers spawned).

- [ ] **Step 3: Implement**

Resume only a run that is running-but-detached (the state `resume` exists for); show status for everything else:

```rust
                            let resumable = matches!(run.current_lifecycle(), types::GraphLifecycle::Running)
                                && !is_running(&self.cwd);
                            if resumable {
                                self.resume(&run.run_id)?
                            } else {
                                // Done, Paused, Cancelled, Blocked: look, don't restart.
                                // `/graph-resume <id>` restarts on purpose.
                                self.status(Some(&run.run_id))
                            }
```

(Use the lifecycle variant that means "was running when the process ended"; if the enum has no such variant, use `run.phase` values that are not terminal and not Cancelled/Blocked.) Add `/graph-resume <id>` to `command_specs` if it does not exist, routed to `self.resume`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/mod.rs
git commit -m "fix(graph): bare /graph never restarts a cancelled or blocked run"
```

---

### Task 8.10: `/graph diff` compares revisions of the same run

**Finding fixed:** native-extensions 16. `graph/mod.rs:1130-1142` searches **all runs** for one whose `revision == current.revision - 1`; `revision` is a per-run counter, so the diff is computed against some other goal's run.

**Files:**
- Modify: `graph/mod.rs:1125-1145`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn graph_diff_uses_the_same_runs_history() {
        let host = fixture_graph_host();
        let other = host.fixture_run("other goal", /*revision*/ 1);
        let current = host.fixture_run_with_history("this goal", /*revisions*/ 2);
        let report = host.graph_diff(None).unwrap();
        assert_eq!(report.previous_run_id.as_deref(), Some(current.run_id.as_str()));
        assert_ne!(report.previous_run_id.as_deref(), Some(other.run_id.as_str()));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib graph_diff_uses_the_same_runs_history`
Expected: FAIL.

- [ ] **Step 3: Implement**

Load the prior revision from the run's own history (`graph/history.rs`; it stores per-revision snapshots of one run, see `history.rs:169` tests) instead of scanning other runs:

```rust
        let prior = match revision {
            Some(rev) => history::load_revision(&self.cwd, &current.run_id, rev),
            None if current.revision > 1 => history::load_revision(&self.cwd, &current.run_id, current.revision - 1),
            None => None,
        };
```

Use `history.rs`'s real loader name; if it cannot load a specific revision, add `load_revision` there.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "fix(graph): diff against the same run's previous revision"
```

---

### Task 8.11: Security scans stop at session shutdown, and waiting for one is bounded

**Finding fixed:** native-extensions 14. `session_shutdown` (`mod.rs:575-582`) aborts graph, LSP and learning but not `self.security`; scan threads keep calling the model after the session ends in RPC or long-lived hosts. `wait()` (`security_scan/controller.rs:249-268`) is two unbounded condvar waits; `wait_for_review` hangs if the worker is stuck in a provider call.

**Files:**
- Modify: `mod.rs:575-582`, `security_scan/controller.rs:249-268`

**Interfaces:**
- Produces: `ReviewController::wait_timeout(&self, timeout: Duration) -> bool` (true if finished)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn waiting_for_a_stuck_review_gives_up() {
        let controller = fixture_review_that_never_finishes();
        let started = std::time::Instant::now();
        assert!(!controller.wait_timeout(std::time::Duration::from_millis(200)));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib waiting_for_a_stuck_review`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`wait_timeout` uses `Condvar::wait_timeout_while` for both waits with the remaining time; `wait()` becomes `wait_timeout(Duration::MAX)` only where an unbounded wait is intended (none should remain; use 30 s in `wait_for_review` and report "scan still running" on timeout).

`session_shutdown`: add `self.security.review.abort(None);` (use the real abort entry on the review controller) before `self.learning.cancel_active_review();`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib security_scan` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "fix(security-scan): abort on session shutdown and bound waits"
```

---

### Task 8.12: Security-scan reports are private, atomic and cleaned up; cargo-audit cannot block on stderr

**Findings fixed:** native-extensions 13 and 23. Reports go to a shared, predictable `temp_dir()/pi-security-scans/<repo>/<scan>` with plain `fs::write` and no retention (`security_scan.rs:239-285, 393`): other users on a multi-user Unix host can read findings or plant a symlink; a crash leaves a torn `scan-manifest.json`; `/tmp` fills up. `security_scan/analyzers.rs:265` pipes cargo-audit's stderr and never reads it (SUSPECTED hang on verbose stderr).

**Files:**
- Modify: `security_scan.rs:239-285, 393`, `security_scan/analyzers.rs:265`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn scan_reports_live_under_the_agent_dir_and_are_private() {
        let dir = scan_output_dir(&fixture_agent_dir(), "repo-id", "scan-1");
        assert!(dir.starts_with(fixture_agent_dir()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(&dir).unwrap();
            ensure_private_dir(&dir).unwrap();
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn old_scan_reports_are_pruned() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..15 {
            std::fs::create_dir_all(root.path().join(format!("scan-{i:02}"))).unwrap();
        }
        prune_scan_reports(root.path(), 10);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 10);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib scan_reports_live_under old_scan_reports_are_pruned`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

- `scan_output_dir(agent_dir, repo, scan)` = `agent_dir.join("security-scans").join(repo).join(scan)`; `ensure_private_dir` sets 0700 on Unix (no-op elsewhere; the agent dir is already per-user on Windows).
- Every report file is written with `davinci_sys::fs::atomic_write`.
- `prune_scan_reports(root, keep)` deletes all but the newest `keep` scan directories (by modification time) after each scan finishes; `keep = 10`.
- `analyzers.rs:265`: `.stderr(Stdio::null())` for cargo-audit, or run it through `run_bounded` if its stderr is used for error messages.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib security_scan` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "fix(security-scan): private atomic reports under the agent dir with retention"
```

---

### Task 8.13: Learning ledgers are compacted

**Finding fixed:** native-extensions 19. `skills.jsonl` and `candidates.jsonl` gain a full record per outcome (`learning/store.rs:114-180`); `compact()` (`:263`) is only called from tests (`#[allow(dead_code)]`), so files and startup replay grow forever.

**Files:**
- Modify: `learning/store.rs:114-180, 263`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn opening_a_bloated_ledger_compacts_it() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = LearningStore::open(dir.path()).unwrap();
            for i in 0..100 {
                store.record_skill_outcome("skill-a", i % 2 == 0).unwrap();
            }
        }
        let lines_before = std::fs::read_to_string(dir.path().join("skills.jsonl")).unwrap().lines().count();
        let _store = LearningStore::open(dir.path()).unwrap();
        let lines_after = std::fs::read_to_string(dir.path().join("skills.jsonl")).unwrap().lines().count();
        assert!(lines_before >= 100);
        assert!(lines_after <= 2, "{lines_after}");
    }
```

(Use the store's real constructor and outcome method.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib opening_a_bloated_ledger`
Expected: FAIL.

- [ ] **Step 3: Implement**

At the end of `open` (after replay), if the number of physical lines in either file exceeds `2 × live entries + 32`, call `self.compact()`. Remove `#[allow(dead_code)]` from `compact`. `compact` already uses `atomic_write` after Task 3.9.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib learning` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/learning/store.rs
git commit -m "fix(learning): compact ledgers on open when they have grown"
```

---

### Task 8.14: Saved graph definitions round-trip, and a failed reload is reported

**Finding fixed:** native-extensions 20. `definitions.rs:1055-1165` writes YAML with `"{}"` around raw strings (no escaping); a description, parameter default or allowed value containing `"` or `\` becomes `saved_definition.yaml` that does not parse, and `\n` round-trips as a newline. On resume, the load failure is ignored (`store.rs:357-362, 579-587`) and the saved definition silently disappears.

**Files:**
- Modify: `graph/definitions.rs:1055-1165`, `graph/store.rs:357-362, 579-587`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn saved_definition_round_trips_quotes_backslashes_and_newlines() {
        let mut def = sample_definition();
        def.description = "say \"hi\" at C:\\temp\nsecond line".into();
        let yaml = to_yaml_string(&def);
        let parsed = parse_definition_yaml(&yaml).unwrap();
        assert_eq!(parsed.description, def.description);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib saved_definition_round_trips`
Expected: FAIL.

- [ ] **Step 3: Implement**

Add one quoting function and use it for every scalar the writer emits:

```rust
/// A YAML double-quoted scalar: escapes backslash, quote and control
/// characters, so any string round-trips.
fn yaml_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
```

Make sure the graph's YAML *reader* un-escapes these sequences (if it is a hand-written parser, add the matching unescape and test it; if it uses a YAML crate, it already does).

In `store.rs`, when `saved_definition.yaml` exists but fails to parse, set the run's `persistence_error` (or return an error from load) with the file path and parse error, instead of dropping it silently.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "fix(graph): escape YAML scalars and report saved-definition load failures"
```

---

### Task 8.15: The git runner cannot deadlock on stderr

**Finding (SUSPECTED):** native-extensions 12. `bounded_read_combined` (`git_intelligence/runner.rs:296-317`) reads stdout to the end before touching stderr; if git writes more than the pipe buffer to stderr while stdout is open, git blocks and every call waits out the full timeout.

**Files:**
- Modify: `git_intelligence/runner.rs:168-317`

- [ ] **Step 1: Write the failing test (Unix, uses a fake git)**

```rust
    #[cfg(unix)]
    #[test]
    fn a_git_that_floods_stderr_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("git");
        std::fs::write(&fake, "#!/bin/sh\nhead -c 200000 /dev/zero >&2\necho ok\n").unwrap();
        std::os::unix::fs::PermissionsExt::set_mode(&mut std::fs::metadata(&fake).unwrap().permissions(), 0o755);
        let started = std::time::Instant::now();
        let out = run_with_executable(&fake, dir.path(), &["status"], std::time::Duration::from_secs(5)).unwrap();
        assert!(String::from_utf8_lossy(&out.0).contains("ok"));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }
```

(Add a `#[cfg(test)] run_with_executable` seam if `executable()` cannot be overridden; set the mode with `fs::set_permissions`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib a_git_that_floods_stderr`
Expected: FAIL (times out) if the finding is real. If it passes, keep the test as a guard and skip Step 3.

- [ ] **Step 3: Implement**

Replace the spawn/read/wait block in `run_status_interruptible` with `davinci_sys::process::run_bounded(command, None, RunLimits { timeout, output_cap: 16 * 1024 * 1024 }, cancelled)`, keeping the env sanitation and the `repository-owned Git executable denied` check. Map `stdout_truncated` to the existing "exceeded byte limit" error and, when stdout is empty, return the stderr tail as today.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib git_intelligence` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions/git_intelligence/runner.rs
git commit -m "fix(git): drain stdout and stderr concurrently in the bounded git runner"
```

---

### Task 8.16: Command lists agree with dispatch; dead status branch removed; repo id lookup times out

**Findings fixed:** native-extensions 25.
- `mod.rs:133-169` dispatches `git-status`, `impact-status` and `sec-*`, but `command_specs` (`mod.rs:174-288`) omits them, so they are missing from autocomplete and RPC discovery.
- `workspace_snapshot/mod.rs:288, 325, 345` check for an `"already_restored"` status that `compare_entry` never produces.
- `vector_memory.rs:441-457` `resolve_repo_id` runs git twice with no timeout during host construction.

**Files:**
- Modify: `mod.rs:133-288`, `workspace_snapshot/mod.rs:288-345`, `vector_memory.rs:441-457`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn every_dispatched_command_is_listed() {
        let host = fixture_native_host();
        let listed: std::collections::BTreeSet<String> =
            host.command_specs().into_iter().map(|spec| spec.name).collect();
        for name in host.dispatched_command_names() {
            assert!(listed.contains(name), "{name} is dispatched but not listed");
        }
    }
```

Add `dispatched_command_names()` as a `const` list next to the dispatch `match` and assert the `match` covers exactly that list (so the list cannot drift from the match).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib every_dispatched_command_is_listed`
Expected: FAIL naming `git-status`.

- [ ] **Step 3: Implement**

Add the missing specs with one-line descriptions. Delete the three `"already_restored"` branches (or, if `compare_entry` *should* produce it, make it do so and test it; decide by reading `compare_entry`). Route `resolve_repo_id`'s two git calls through `graph::git::run` (Task 8.6).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --lib native_extensions` → PASS.

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "fix(native): list every dispatched command, drop dead restore branch, time-limit repo id"
```

---

## Phase 8 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-coding-agent --lib native_extensions
```

Then `00-index.md` → "Manual verification: graph".
