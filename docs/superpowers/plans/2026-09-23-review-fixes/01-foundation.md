# Phase 1: Shared Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the shared helpers that later phases depend on: crash-safe file publication, torn-tail repair for JSONL, two cross-process locks (short-hold with stale takeover, long-hold released by the OS), a supervised child-process runner, and one resolver for project config paths.

**Architecture:** A new leaf crate `crates/davinci-sys` (no davinci dependencies) holds the OS-facing helpers so `davinci-ai`, `davinci-session`, `davinci-agent`, `davinci-mcp` and `davinci-coding-agent` can all use them without a dependency cycle. The project-config resolver is coding-agent specific, so it lives in `crates/davinci-coding-agent/src/project_config.rs`.

**Tech Stack:** Rust 1.83 (edition 2021), std only plus `uuid` (already a workspace dependency). Tests use `tempfile`.

Read `00-index.md` first for the global constraints and phase order.

---

## File map

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` (workspace) | modify | add `crates/davinci-sys` to `members` |
| `crates/davinci-sys/Cargo.toml` | create | crate manifest |
| `crates/davinci-sys/src/lib.rs` | create | module list |
| `crates/davinci-sys/src/fs.rs` | create | `atomic_write`, `atomic_write_private`, `sync_parent`, `truncate_torn_tail` |
| `crates/davinci-sys/src/lock.rs` | create | `LockFile` (short holds, stale takeover), `ExclusiveFileLock` (long holds, OS-released) |
| `crates/davinci-agent/src/runtime/session.rs:275-337` | modify | `WorkerSessionLease` wraps `ExclusiveFileLock` |
| `crates/davinci-sys/src/process.rs` | create | `resolve_program`, `kill_tree`, `set_own_process_group`, `run_bounded` |
| `crates/davinci-agent/src/jobs.rs:260-275` | modify | `kill_tree` delegates to `davinci_sys::process::kill_tree` |
| `crates/davinci-coding-agent/src/project_config.rs` | create | `.davinci` / `.pi` resolution for one config name |
| `crates/davinci-coding-agent/src/lib.rs`, `src/main.rs` | modify | declare `project_config` in both crate roots |

`main.rs` compiles its own copy of many modules (`main.rs:1-113`). Until Task 11.1 removes that duplication, every new module used by `trust.rs`, `settings.rs` or `hooks.rs` must be declared in **both** `lib.rs` and `main.rs`.

---

### Task 1.1: `davinci-sys` crate with crash-safe writes

**Files:**
- Modify: `Cargo.toml` (workspace `members`)
- Create: `crates/davinci-sys/Cargo.toml`
- Create: `crates/davinci-sys/src/lib.rs`
- Create: `crates/davinci-sys/src/fs.rs`

**Interfaces:**
- Produces:
  - `davinci_sys::fs::atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()>`
  - `davinci_sys::fs::atomic_write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()>` (0600 on Unix)
  - `davinci_sys::fs::sync_parent(dir: &Path) -> std::io::Result<()>`

- [ ] **Step 1: Create the crate skeleton**

`crates/davinci-sys/Cargo.toml`:

```toml
[package]
name = "davinci-sys"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[dependencies]
uuid.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/davinci-sys/src/lib.rs`:

```rust
//! OS-facing helpers shared by every davinci crate: crash-safe file
//! publication, cross-process lock files, and supervised child processes.
//! This crate depends on no other davinci crate, so any crate can use it.

pub mod fs;
pub mod lock;
pub mod process;
```

Add `"crates/davinci-sys",` to `[workspace] members` in the root `Cargo.toml`, directly after `"crates/davinci-protocol",`.

Create empty `crates/davinci-sys/src/lock.rs` and `crates/davinci-sys/src/process.rs` (each containing only `//! Filled in by Task 1.2 / 1.3.`) so the crate builds.

- [ ] **Step 2: Write the failing tests**

Append to `crates/davinci-sys/src/fs.rs` (the file starts with only this test module; Step 4 adds the code above it):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_creates_parent_and_replaces_content() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");
        atomic_write(&path, b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, b"x").unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["state.json".to_string()]);
    }

    #[test]
    fn failed_write_keeps_the_previous_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, b"old").unwrap();
        // A directory at the temp location's parent cannot take a rename
        // target that is itself a non-empty directory.
        let blocked = dir.path().join("blocked");
        std::fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(atomic_write(&blocked, b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert!(blocked.join("child").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn private_write_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json");
        atomic_write_private(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p davinci-sys`
Expected: FAIL to compile with `cannot find function 'atomic_write' in this scope`.

- [ ] **Step 4: Implement**

Put this at the top of `crates/davinci-sys/src/fs.rs`, above the test module:

```rust
//! Crash-safe file publication. A reader sees the old file or the new one,
//! never a torn one: the bytes go to a temp file in the same directory, are
//! flushed, and replace the target with one rename.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

/// Publish `bytes` at `path`. Creates the parent directory if needed.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, bytes, false)
}

/// Same as [`atomic_write`], and on Unix the file is created 0600 so only
/// its owner can read it. Use it for credentials and tokens.
pub fn atomic_write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, bytes, true)
}

/// Make a completed rename durable. Windows has no directory fsync; NTFS
/// journals the rename itself, so this does nothing there.
pub fn sync_parent(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

fn write_with(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = private;
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rename_with_retry(&temp, path)?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// On Windows an antivirus scanner or the indexer can hold the target open
/// for a few milliseconds, and the rename fails with a sharing violation
/// (`PermissionDenied`). Retry briefly before giving up.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 5;
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err)
                if cfg!(windows)
                    && err.kind() == io::ErrorKind::PermissionDenied
                    && attempt + 1 < ATTEMPTS =>
            {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20 * u64::from(attempt)));
            }
            Err(err) => return Err(err),
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p davinci-sys`
Expected: PASS (3 tests on Windows, 4 on Unix).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/davinci-sys
git commit -m "feat(sys): add davinci-sys crate with crash-safe atomic_write"
```

---

### Task 1.2: Cross-process `LockFile` with stale takeover

**Files:**
- Modify: `crates/davinci-sys/src/lock.rs`

**Interfaces:**
- Produces:
  - `pub struct LockFile` (dropping it releases the lock)
  - `LockFile::acquire(path: &Path, wait: Duration, stale_after: Duration) -> std::io::Result<LockFile>`
  - `pub const DEFAULT_STALE_AFTER: Duration` (10 s, same as proper-lockfile in TS pi)
  - `pub fn lock_path_for(target: &Path) -> PathBuf` (`settings.json` → `settings.json.lock`)
  - Error kind `std::io::ErrorKind::WouldBlock` when `wait` runs out.

**Why:** `settings.rs:1393-1420` uses `create_new` with no staleness check. A crash (or `std::process::exit`, which skips destructors on other threads) leaves `settings.json.lock` behind forever, and every later save fails. `trust.rs:59-64` then turns the lock error into "no decision", which skips a stored "Do not trust".

**Accepted race (document it in the code):** two waiters that both observe the same stale lock inside the same ~20 ms window can both remove it. Holders keep the lock for milliseconds and the stale threshold is 10 s, so this needs a crashed holder plus two simultaneous waiters. proper-lockfile has the same window.

- [ ] **Step 1: Write the failing tests**

`crates/davinci-sys/src/lock.rs` (test module only for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn second_acquire_waits_then_fails_while_held() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json.lock");
        let _held = LockFile::acquire(&path, Duration::ZERO, DEFAULT_STALE_AFTER).unwrap();
        let started = Instant::now();
        let err = LockFile::acquire(&path, Duration::from_millis(100), DEFAULT_STALE_AFTER)
            .err()
            .expect("lock is held");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn drop_releases_the_lock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.lock");
        drop(LockFile::acquire(&path, Duration::ZERO, DEFAULT_STALE_AFTER).unwrap());
        assert!(!path.exists());
        LockFile::acquire(&path, Duration::ZERO, DEFAULT_STALE_AFTER).unwrap();
    }

    #[test]
    fn stale_lock_left_by_a_crash_is_taken_over() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.lock");
        std::fs::write(&path, "12345\n").unwrap();
        // Zero stale_after: any existing lock counts as abandoned.
        let lock = LockFile::acquire(&path, Duration::ZERO, Duration::ZERO).unwrap();
        drop(lock);
        assert!(!path.exists());
    }

    #[test]
    fn lock_path_appends_dot_lock() {
        assert_eq!(
            lock_path_for(std::path::Path::new("/a/settings.json")),
            std::path::PathBuf::from("/a/settings.json.lock")
        );
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-sys lock`
Expected: FAIL to compile, `cannot find type 'LockFile'`.

- [ ] **Step 3: Implement**

Put above the test module in `lock.rs`:

```rust
//! A cross-process lock held by owning a `<target>.lock` file.
//!
//! `create_new` is atomic on every platform we ship. A holder that crashed
//! leaves the file behind, so a lock whose mtime is older than
//! `stale_after` is treated as abandoned and taken over.
//!
//! Known window: two waiters that see the same stale lock within ~20 ms can
//! both remove it. Holders keep locks for milliseconds and the default stale
//! threshold is 10 s, so this needs a crashed holder plus two simultaneous
//! waiters. proper-lockfile (used by TypeScript pi) has the same window.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// proper-lockfile's default, so TS pi and davinci agree on staleness.
pub const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(10);

const RETRY_EVERY: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub struct LockFile {
    path: PathBuf,
}

impl LockFile {
    pub fn acquire(path: &Path, wait: Duration, stale_after: Duration) -> io::Result<Self> {
        let started = Instant::now();
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if is_stale(path, stale_after) {
                        let _ = fs::remove_file(path);
                        continue;
                    }
                    if started.elapsed() >= wait {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            format!("{} is held by another process", path.display()),
                        ));
                    }
                    std::thread::sleep(RETRY_EVERY);
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn lock_path_for(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    target.with_file_name(name)
}

fn is_stale(path: &Path, stale_after: Duration) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age >= stale_after)
        .unwrap_or(false)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-sys lock`
Expected: PASS, 4 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-sys/src/lock.rs
git commit -m "feat(sys): add LockFile with stale-lock takeover"
```

---

### Task 1.3: Supervised child processes (`run_bounded`, `resolve_program`, `kill_tree`)

**Files:**
- Modify: `crates/davinci-sys/src/process.rs`
- Modify: `crates/davinci-agent/Cargo.toml` (add `davinci-sys = { path = "../davinci-sys" }`)
- Modify: `crates/davinci-agent/src/jobs.rs:260-275`

**Interfaces:**
- Produces:
  - `pub fn resolve_program(name: &str) -> std::path::PathBuf`
  - `pub fn resolve_program_in(name: &str, path_var: &std::ffi::OsStr, pathext: Option<&str>) -> Option<std::path::PathBuf>`
  - `pub fn set_own_process_group(command: &mut std::process::Command)`
  - `pub fn kill_tree(pid: u32)`
  - `pub struct RunLimits { pub timeout: Duration, pub output_cap: usize }`
  - `pub struct BoundedOutput { pub status: Option<ExitStatus>, pub stdout: Vec<u8>, pub stderr: Vec<u8>, pub stdout_truncated: bool, pub stderr_truncated: bool, pub timed_out: bool, pub cancelled: bool }`
  - `pub fn run_bounded(command: Command, stdin: Option<Vec<u8>>, limits: RunLimits, cancelled: &dyn Fn() -> bool) -> std::io::Result<BoundedOutput>`

**Why:** the review found the same four bugs copied across hooks, packages, js_host, graph, security scan and the shell tool:
1. stdin written synchronously before the timeout starts (`hooks.rs:715-719`), so a hook that never reads stdin blocks forever on a large payload;
2. readers joined without a deadline, so a backgrounded grandchild holding stdout hangs the agent (`hooks.rs:792-797`, `tools.rs:2011-2041`);
3. no process group on Unix, so `kill -TERM -pid` hits nothing (`graph/worker.rs:526`, `graph/process.rs:250-262`, hooks);
4. `Command::new("npm")` cannot find `npm.cmd` on Windows (`packages.rs:695`).

`run_bounded` fixes all four in one place. Later tasks switch callers to it.

- [ ] **Step 1: Write the failing tests**

Test module for `process.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn shell(script: &str) -> Command {
        if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", script]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        }
    }

    fn limits(ms: u64) -> RunLimits {
        RunLimits {
            timeout: Duration::from_millis(ms),
            output_cap: 64 * 1024,
        }
    }

    #[test]
    fn captures_stdout_and_exit_status() {
        let out = run_bounded(shell("echo hello"), None, limits(10_000), &|| false).unwrap();
        assert!(out.status.unwrap().success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
        assert!(!out.timed_out);
    }

    #[test]
    fn timeout_kills_the_child() {
        let script = if cfg!(windows) { "ping -n 30 127.0.0.1 >NUL" } else { "sleep 30" };
        let started = Instant::now();
        let out = run_bounded(shell(script), None, limits(300), &|| false).unwrap();
        assert!(out.timed_out);
        assert!(out.status.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn cancel_stops_the_child() {
        let script = if cfg!(windows) { "ping -n 30 127.0.0.1 >NUL" } else { "sleep 30" };
        let out = run_bounded(shell(script), None, limits(30_000), &|| true).unwrap();
        assert!(out.cancelled);
        assert!(!out.timed_out);
    }

    #[test]
    fn output_beyond_the_cap_is_truncated_not_blocking() {
        let script = if cfg!(windows) {
            "for /L %i in (1,1,300) do @echo 0123456789"
        } else {
            "i=0; while [ $i -lt 300 ]; do echo 0123456789; i=$((i+1)); done"
        };
        let limits = RunLimits {
            timeout: Duration::from_secs(20),
            output_cap: 100,
        };
        let out = run_bounded(shell(script), None, limits, &|| false).unwrap();
        assert_eq!(out.stdout.len(), 100);
        assert!(out.stdout_truncated);
        assert!(out.status.unwrap().success());
    }

    #[test]
    fn unread_large_stdin_does_not_block() {
        let payload = vec![b'x'; 4 * 1024 * 1024];
        let started = Instant::now();
        let out = run_bounded(shell("exit 0"), Some(payload), limits(10_000), &|| false).unwrap();
        assert!(out.status.unwrap().success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn grandchild_holding_stdout_does_not_hang() {
        let started = Instant::now();
        let out = run_bounded(shell("sleep 30 & echo done"), None, limits(10_000), &|| false)
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("done"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn resolve_program_in_uses_pathext_on_windows_style_lookup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("npm.cmd"), "@echo off").unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        let found = resolve_program_in("npm", &path_var, Some(".COM;.EXE;.BAT;.CMD"));
        assert_eq!(found, Some(dir.path().join("npm.cmd")));
    }

    #[test]
    fn resolve_program_in_without_pathext_needs_exact_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool"), "#!/bin/sh").unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(
            resolve_program_in("tool", &path_var, None),
            Some(dir.path().join("tool"))
        );
        assert_eq!(resolve_program_in("missing", &path_var, None), None);
    }

    #[test]
    fn names_with_a_separator_are_not_searched() {
        let path_var = std::ffi::OsString::new();
        assert_eq!(resolve_program_in("./npm", &path_var, Some(".CMD")), None);
        assert_eq!(resolve_program("./npm"), std::path::PathBuf::from("./npm"));
    }
}
```

`resolve_program_in` checks `is_file()` only, so the Unix test does not need an executable bit.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-sys process`
Expected: FAIL to compile, `cannot find function 'run_bounded'`.

- [ ] **Step 3: Implement**

Top of `process.rs`:

```rust
//! Supervised child processes.
//!
//! `run_bounded` is the one way davinci runs a helper program (hooks, git,
//! npm, verification shells, analyzers). It guarantees:
//! - stdin is written on its own thread, so a child that never reads it
//!   cannot block us;
//! - stdout and stderr are drained concurrently and capped, so a chatty
//!   child can neither fill a pipe and stall nor exhaust memory;
//! - the timeout and the cancel flag are checked while the child runs;
//! - on Unix the child leads its own process group, so a kill reaches its
//!   grandchildren; on Windows `taskkill /T` walks the tree;
//! - after the child exits we wait at most `READER_GRACE` for the pipes to
//!   close, so a backgrounded grandchild that inherited them cannot hang us.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);
const READER_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub timeout: Duration,
    /// Per stream. Bytes beyond it are read and discarded.
    pub output_cap: usize,
}

#[derive(Debug, Default)]
pub struct BoundedOutput {
    /// `None` when the child was killed for timeout or cancel.
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Find `name` on PATH the way a shell would. On Windows this tries each
/// PATHEXT extension, because `Command::new("npm")` only appends `.exe`
/// and npm ships as `npm.cmd`. Returns `name` unchanged when nothing is
/// found, so the spawn error still names what the user asked for.
pub fn resolve_program(name: &str) -> PathBuf {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = if cfg!(windows) {
        Some(std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()))
    } else {
        None
    };
    resolve_program_in(name, &path_var, pathext.as_deref()).unwrap_or_else(|| PathBuf::from(name))
}

pub fn resolve_program_in(name: &str, path_var: &OsStr, pathext: Option<&str>) -> Option<PathBuf> {
    // A path (`./npm`, `C:\tools\npm`) is used as given, never searched.
    if name.contains('/') || name.contains('\\') || name.is_empty() {
        return None;
    }
    let has_extension = Path::new(name).extension().is_some();
    for dir in std::env::split_paths(path_var) {
        match pathext {
            Some(exts) if !has_extension => {
                for ext in exts.split(';').filter(|ext| !ext.is_empty()) {
                    let mut file = OsString::from(name);
                    file.push(ext.to_ascii_lowercase());
                    let candidate = dir.join(&file);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
            _ => {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Unix: make the child the leader of a new process group so `kill_tree`
/// reaches everything it starts. Windows: nothing to do, `taskkill /T`
/// walks the process tree.
pub fn set_own_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// Kill `pid` and everything it started. On Unix `pid` must lead its own
/// group (see [`set_own_process_group`]).
pub fn kill_tree(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let _ = Command::new("kill")
            // A negative PID is a process group, not another signal option.
            .args(["-TERM", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[derive(Default)]
struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
}

fn drain<R: Read + Send + 'static>(
    mut pipe: R,
    cap: usize,
    done: mpsc::Sender<()>,
) -> Arc<Mutex<Captured>> {
    let shared = Arc::new(Mutex::new(Captured::default()));
    let sink = Arc::clone(&shared);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let mut captured = sink.lock().unwrap_or_else(|err| err.into_inner());
                    let room = cap.saturating_sub(captured.bytes.len());
                    let keep = room.min(read);
                    captured.bytes.extend_from_slice(&chunk[..keep]);
                    if keep < read {
                        captured.truncated = true;
                    }
                }
            }
        }
        let _ = done.send(());
    });
    shared
}

fn take(shared: &Arc<Mutex<Captured>>) -> (Vec<u8>, bool) {
    let mut captured = shared.lock().unwrap_or_else(|err| err.into_inner());
    (std::mem::take(&mut captured.bytes), captured.truncated)
}

pub fn run_bounded(
    mut command: Command,
    stdin: Option<Vec<u8>>,
    limits: RunLimits,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<BoundedOutput> {
    command
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    set_own_process_group(&mut command);
    let mut child = command.spawn()?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A child that exits without reading gives us BrokenPipe; ignore it.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let (done_tx, done_rx) = mpsc::channel();
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out = drain(stdout, limits.output_cap, done_tx.clone());
    let err = drain(stderr, limits.output_cap, done_tx);

    let deadline = Instant::now() + limits.timeout;
    let mut result = BoundedOutput::default();
    loop {
        if let Some(status) = child.try_wait()? {
            result.status = Some(status);
            break;
        }
        let was_cancelled = cancelled();
        if was_cancelled || Instant::now() >= deadline {
            result.cancelled = was_cancelled;
            result.timed_out = !was_cancelled;
            kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(POLL);
    }

    let grace_end = Instant::now() + READER_GRACE;
    let mut closed = 0;
    while closed < 2 {
        let left = grace_end.saturating_duration_since(Instant::now());
        match done_rx.recv_timeout(left) {
            Ok(()) => closed += 1,
            Err(_) => {
                // A grandchild still holds a pipe. Kill the group and stop
                // waiting; the reader threads end when the pipe closes.
                kill_tree(child.id());
                break;
            }
        }
    }
    (result.stdout, result.stdout_truncated) = take(&out);
    (result.stderr, result.stderr_truncated) = take(&err);
    Ok(result)
}
```

Destructuring assignment to fields (`(result.stdout, result.stdout_truncated) = ...`) is stable since Rust 1.59, so it is fine on 1.83.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-sys process`
Expected: PASS (8 tests on Windows, 9 on Unix).

- [ ] **Step 5: Point `davinci_agent::jobs::kill_tree` at the shared one**

In `crates/davinci-agent/Cargo.toml` `[dependencies]` add:

```toml
davinci-sys = { path = "../davinci-sys" }
```

Replace the body of `pub fn kill_tree(pid: u32)` at `crates/davinci-agent/src/jobs.rs:260-275` with:

```rust
pub fn kill_tree(pid: u32) {
    davinci_sys::process::kill_tree(pid);
}
```

Run: `cargo test -p davinci-agent jobs`
Expected: PASS (existing jobs tests unchanged).

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-sys/src/process.rs crates/davinci-agent/Cargo.toml crates/davinci-agent/src/jobs.rs Cargo.lock
git commit -m "feat(sys): add run_bounded supervised child runner and PATHEXT program lookup"
```

---

### Task 1.4: One resolver for project config paths

**Files:**
- Create: `crates/davinci-coding-agent/src/project_config.rs`
- Modify: `crates/davinci-coding-agent/src/lib.rs` (add `pub mod project_config;` after `pub mod prompt_host;`)
- Modify: `crates/davinci-coding-agent/src/main.rs` (add `mod project_config;` after `mod permissions;` at line 103)

**Interfaces:**
- Produces:
  - `project_config::candidates(cwd: &Path, name: &str) -> [PathBuf; 2]` (`.davinci/<name>`, then `.pi/<name>`)
  - `project_config::resolve(cwd: &Path, name: &str) -> Option<PathBuf>` (first that exists)
  - `project_config::all(cwd: &Path, name: &str) -> Vec<PathBuf>` (every one that exists, `.davinci` first)
  - `project_config::any_exists(cwd: &Path, names: &[&str]) -> bool` (checks both dirs for every name)

**Why:** `trust.rs:210-215` picks `.davinci/` **or** `.pi/` for the whole directory, but `hooks.rs:405-413`, `mcp.rs:20-25`, `settings.rs:1002-1007` pick per file. A repo with an empty `.davinci/` and a `.pi/hooks.json` passes the trust check and still gets its hook loaded. One resolver used by the check and the loaders makes that impossible.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn prefers_davinci_then_falls_back_per_file() {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        fs::create_dir_all(cwd.join(".davinci")).unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("hooks.json"), "{}").unwrap();
        // .davinci exists but has no hooks.json: the .pi file is still found.
        assert_eq!(resolve(cwd, "hooks.json"), Some(cwd.join(".pi").join("hooks.json")));
        fs::write(cwd.join(".davinci").join("hooks.json"), "{}").unwrap();
        assert_eq!(
            resolve(cwd, "hooks.json"),
            Some(cwd.join(".davinci").join("hooks.json"))
        );
    }

    #[test]
    fn all_returns_both_directories_in_order() {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        fs::create_dir_all(cwd.join(".davinci").join("skills")).unwrap();
        fs::create_dir_all(cwd.join(".pi").join("skills")).unwrap();
        assert_eq!(
            all(cwd, "skills"),
            vec![cwd.join(".davinci").join("skills"), cwd.join(".pi").join("skills")]
        );
    }

    #[test]
    fn any_exists_checks_both_directories() {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        fs::create_dir_all(cwd.join(".davinci")).unwrap();
        fs::write(cwd.join(".davinci").join("README"), "").unwrap();
        assert!(!any_exists(cwd, &["mcp.json", "hooks.json"]));
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("mcp.json"), "{}").unwrap();
        assert!(any_exists(cwd, &["mcp.json", "hooks.json"]));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib project_config`
Expected: FAIL to compile, `cannot find function 'resolve'`.

- [ ] **Step 3: Implement**

```rust
//! Where project configuration lives: `.davinci/<name>` first, then the
//! legacy `.pi/<name>`, decided per file. The trust check and every loader
//! use this module so they cannot disagree about what a project ships.

use std::path::{Path, PathBuf};

use crate::settings::{CONFIG_DIR_NAME, LEGACY_CONFIG_DIR_NAME};

pub fn candidates(cwd: &Path, name: &str) -> [PathBuf; 2] {
    [
        cwd.join(CONFIG_DIR_NAME).join(name),
        cwd.join(LEGACY_CONFIG_DIR_NAME).join(name),
    ]
}

/// The one file a single-file loader (settings, hooks, mcp) should read.
pub fn resolve(cwd: &Path, name: &str) -> Option<PathBuf> {
    candidates(cwd, name).into_iter().find(|path| path.exists())
}

/// Every existing location. Directory resources (skills, prompts, agents,
/// extensions) are merged from both, `.davinci` first.
pub fn all(cwd: &Path, name: &str) -> Vec<PathBuf> {
    candidates(cwd, name)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

pub fn any_exists(cwd: &Path, names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| candidates(cwd, name).iter().any(|path| path.exists()))
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib project_config`
Expected: PASS, 3 tests. Also run `cargo build -p davinci-coding-agent --bin davinci` to confirm the `main.rs` declaration compiles (an unused-module warning is expected until Phase 2 uses it from `mcp.rs`/`hooks.rs`; add `#[allow(dead_code)]` on the `mod project_config;` line in `main.rs` only if clippy fails on it).

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/project_config.rs crates/davinci-coding-agent/src/lib.rs crates/davinci-coding-agent/src/main.rs
git commit -m "feat(config): add per-file .davinci/.pi project config resolver"
```

---

### Task 1.5: `ExclusiveFileLock`, an OS-released lock for long holds

**Files:**
- Modify: `crates/davinci-sys/Cargo.toml` (add `[target.'cfg(unix)'.dependencies] libc.workspace = true`)
- Modify: `crates/davinci-sys/src/lock.rs`
- Modify: `crates/davinci-agent/src/runtime/session.rs:275-337` (`WorkerSessionLease` becomes a thin wrapper)

**Interfaces:**
- Produces:
  - `pub struct ExclusiveFileLock` (dropping it releases the lock; the file is left in place)
  - `ExclusiveFileLock::try_acquire(path: &Path) -> std::io::Result<ExclusiveFileLock>` (contention returns `ErrorKind::WouldBlock`)
  - `ExclusiveFileLock::acquire(path: &Path, wait: Duration) -> std::io::Result<ExclusiveFileLock>`

**Why a second lock type:** `LockFile` (Task 1.2) decides staleness by age. That is right for locks held for milliseconds (settings, trust) and it matches TS pi's `settings.json.lock`. It is wrong for a lock held for a whole session or across a network refresh: a live holder's lock would look stale after 10 s. The operating system already knows whether the holder is alive: `flock` on Unix and an unshared open on Windows are released when the process exits, crash included. `WorkerSessionLease` (`davinci-agent/src/runtime/session.rs:275-337`) already does exactly this; this task moves it into `davinci-sys` so the session store (Task 3.3) and `auth.json` (Task 3.5) can use it.

- [ ] **Step 1: Write the failing tests**

Append to the `lock.rs` test module:

```rust
    #[test]
    fn exclusive_lock_blocks_a_second_holder_until_dropped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl.lock");
        let first = ExclusiveFileLock::try_acquire(&path).unwrap();
        let err = ExclusiveFileLock::try_acquire(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        drop(first);
        ExclusiveFileLock::try_acquire(&path).unwrap();
    }

    #[test]
    fn exclusive_lock_acquire_waits_then_gives_up() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("auth.json.lock");
        let _held = ExclusiveFileLock::try_acquire(&path).unwrap();
        let started = Instant::now();
        assert!(ExclusiveFileLock::acquire(&path, Duration::from_millis(100)).is_err());
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_lock_refuses_a_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "").unwrap();
        let link = dir.path().join("x.lock");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(ExclusiveFileLock::try_acquire(&link).is_err());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-sys lock::tests::exclusive`
Expected: FAIL to compile, `cannot find type 'ExclusiveFileLock'`.

- [ ] **Step 3: Implement**

Add to `lock.rs` (above the tests; the code is `WorkerSessionLease::acquire` with contention mapped to `WouldBlock`):

```rust
/// A lock the operating system releases when its holder exits, crash
/// included: `flock(LOCK_EX)` on Unix, an open with no sharing on Windows.
/// Use it for locks held for a long time (an open session, a credential
/// refresh over the network), where age cannot tell slow from dead.
#[derive(Debug)]
pub struct ExclusiveFileLock {
    #[allow(dead_code)] // held for its lifetime; Drop does the unlock
    file: fs::File,
}

impl ExclusiveFileLock {
    pub fn try_acquire(path: &Path) -> io::Result<Self> {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not an ordinary file", path.display()),
                ));
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // share_mode(0): a second open anywhere fails while we hold it.
            options.share_mode(0).custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let file = match options.open(path) {
            Ok(file) => file,
            // ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33).
            Err(err) if cfg!(windows) && matches!(err.raw_os_error(), Some(32 | 33)) => {
                return Err(held(path));
            }
            Err(err) => return Err(err),
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if file.metadata()?.file_attributes() & 0x400 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is a reparse point", path.display()),
                ));
            }
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: `file` owns this live descriptor for the whole call.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let err = io::Error::last_os_error();
                return Err(if err.kind() == io::ErrorKind::WouldBlock { held(path) } else { err });
            }
        }
        Ok(Self { file })
    }

    pub fn acquire(path: &Path, wait: Duration) -> io::Result<Self> {
        let started = Instant::now();
        loop {
            match Self::try_acquire(path) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock && started.elapsed() < wait => {
                    std::thread::sleep(RETRY_EVERY);
                }
                other => return other,
            }
        }
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Closing alone may leave a forked child holding the description.
            // SAFETY: `self.file` still owns the descriptor here.
            unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn held(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        format!("{} is held by another process", path.display()),
    )
}
```

The lock file is never deleted: deleting a flock'd path lets a second process lock a new inode at the same name while the first still holds the old one.

- [ ] **Step 4: Make `WorkerSessionLease` use it**

Replace `struct WorkerSessionLease(std::fs::File);`, its `impl` and its `Drop` in `davinci-agent/src/runtime/session.rs` with:

```rust
struct WorkerSessionLease(#[allow(dead_code)] davinci_sys::lock::ExclusiveFileLock);

impl WorkerSessionLease {
    fn acquire(path: &std::path::Path) -> Result<Self, String> {
        davinci_sys::lock::ExclusiveFileLock::try_acquire(path)
            .map(Self)
            .map_err(|error| format!("worker conversation ownership unavailable: {error}"))
    }
}
```

If an existing test asserts the exact text "worker conversation lease is not an ordinary file", keep that text by matching `ErrorKind::InvalidInput` and returning the old message.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p davinci-sys lock` and `cargo test -p davinci-agent runtime::session`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-sys crates/davinci-agent/src/runtime/session.rs Cargo.lock
git commit -m "feat(sys): move OS-released exclusive file lock into davinci-sys"
```

---

### Task 1.6: `truncate_torn_tail` for append-only JSONL files

**Files:**
- Modify: `crates/davinci-sys/src/fs.rs`

**Interfaces:**
- Produces: `davinci_sys::fs::truncate_torn_tail(path: &Path) -> std::io::Result<u64>` (bytes removed)

**Why:** every JSONL writer here appends `line + "\n"` and then syncs. A crash or a full disk mid-write leaves a last line with no newline. The next append is glued onto it, which turns one lost record into a corrupt *middle* line, and every reader rejects a corrupt middle line (`davinci-session/src/lib.rs:189-195`, `runtime_log.rs:99-107`). Cutting the unterminated tail before the first append keeps the damage to the one record that was never acknowledged.

- [ ] **Step 1: Write the failing tests**

Append to the `fs.rs` test module:

```rust
    #[test]
    fn torn_tail_is_cut_back_to_the_last_newline() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n{\"c\":").unwrap();
        assert_eq!(truncate_torn_tail(&path).unwrap(), 5);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n{\"b\":2}\n");
    }

    #[test]
    fn complete_file_is_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        assert_eq!(truncate_torn_tail(&path).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");
    }

    #[test]
    fn single_torn_line_empties_the_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        std::fs::write(&path, "{\"a\":").unwrap();
        assert_eq!(truncate_torn_tail(&path).unwrap(), 5);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn torn_tail_longer_than_one_chunk_is_found() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut body = b"{\"a\":1}\n".to_vec();
        body.extend(std::iter::repeat(b'x').take(200_000));
        std::fs::write(&path, &body).unwrap();
        assert_eq!(truncate_torn_tail(&path).unwrap(), 200_000);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}\n");
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = tempdir().unwrap();
        assert_eq!(truncate_torn_tail(&dir.path().join("none.jsonl")).unwrap(), 0);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-sys fs::tests::torn`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Add to `fs.rs` (with `use std::io::{Read, Seek, SeekFrom};` added to the imports):

```rust
/// If `path` does not end with `\n`, cut it back to just after its last
/// `\n` (or to empty). Returns the number of bytes removed. JSONL writers
/// call this once before their first append.
pub fn truncate_torn_tail(path: &Path) -> io::Result<u64> {
    let mut file = match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(0);
    }
    const CHUNK: u64 = 64 * 1024;
    let mut end = len;
    let mut buffer = vec![0u8; CHUNK as usize];
    let mut first = true;
    while end > 0 {
        let start = end.saturating_sub(CHUNK);
        let size = (end - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..size])?;
        if first && buffer[size - 1] == b'\n' {
            return Ok(0);
        }
        first = false;
        if let Some(offset) = buffer[..size].iter().rposition(|byte| *byte == b'\n') {
            let keep = start + offset as u64 + 1;
            file.set_len(keep)?;
            file.sync_all()?;
            return Ok(len - keep);
        }
        end = start;
    }
    file.set_len(0)?;
    file.sync_all()?;
    Ok(len)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-sys`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-sys/src/fs.rs
git commit -m "feat(sys): add truncate_torn_tail for append-only JSONL files"
```

---

## Phase 1 exit check

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p davinci-sys -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings
cargo test -p davinci-sys -p davinci-agent -p davinci-coding-agent
```

Expected: all three succeed. Phase 2 starts only after this.
