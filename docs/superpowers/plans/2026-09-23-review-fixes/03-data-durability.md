# Phase 3: Data Durability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** No crash, full disk, second process or parse error can make davinci destroy or lock the user out of their sessions, settings, credentials or memories.

**Architecture:** Every durable file is written through `davinci_sys::fs::atomic_write*` (Task 1.1); every append-only JSONL file is repaired with `truncate_torn_tail` (Task 1.6) before its first append; long-lived writers hold an `ExclusiveFileLock` (Task 1.5); short settings/trust writes use `LockFile` with stale takeover (Task 1.2). Read-modify-write happens entirely under the lock.

**Tech Stack:** Rust 1.83, `davinci-sys`, `rusqlite` (already pinned).

Depends on Phase 1. Independent of Phase 2 except that Task 3.4 changes the lock Task 2.2's held-lock test waits on (the test stays correct).

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-session/Cargo.toml` | 3.1 (add `davinci-sys`) |
| `crates/davinci-session/src/lib.rs:59-66, 129-199, 394-431` | 3.1, 3.2, 3.3 |
| `crates/davinci-session/src/codec.rs:354-427` | 3.2 |
| `crates/davinci-session/src/runtime_log.rs:47-65, 108-114` | 3.8 |
| `crates/davinci-coding-agent/src/settings.rs:1355-1420` | 3.4 |
| `crates/davinci-coding-agent/src/trust.rs:266-281` | 3.4 |
| `crates/davinci-coding-agent/src/packages.rs:40-55, 522-548, 742-779` | 3.4, 3.7 |
| `crates/davinci-ai/src/auth.rs:101-129, 185-280` | 3.5 |
| `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs:872-905` | 3.6 |
| `crates/davinci-coding-agent/src/native_extensions/token_governor.rs:509-522` | 3.9 |
| `crates/davinci-coding-agent/src/native_extensions/learning/store.rs:242-302` | 3.9 |
| `crates/davinci-coding-agent/src/native_extensions/workspace_snapshot/mod.rs:912-928` | 3.9 |
| `crates/davinci-coding-agent/src/native_extensions/graph/store.rs:292-325` | 3.9 |
| `crates/davinci-coding-agent/src/permissions.rs:218-226` | 3.9 |
| `crates/davinci-session-sqlite/src/lib.rs:118-175, 405-415, 697-704` | 3.10 |

---

### Task 3.1: A torn last line never makes a session unopenable

**Finding fixed:** session 1. `JsonlSession::open` (`davinci-session/src/lib.rs:189-195`) rejects the whole file when any line fails to parse, including a last line cut off by a crash, kill or full disk. `write_line` (`:394-405`) then appends onto that partial line, gluing two records. `davinci -c` fails forever with `Invalid JSONL v4 session … line N is not valid JSON`. The other loader, `JsonlStoredSession::load` (`jsonl_repo.rs:421-436`), already repairs this; the product uses `JsonlSession`.

**Rule implemented:** only an **unterminated** last line (no trailing `\n`) that fails to parse is dropped. A newline-terminated line that fails to parse is real corruption and still errors, so a bad middle line never disappears silently.

**Files:**
- Modify: `crates/davinci-session/Cargo.toml` (add `davinci-sys = { path = "../davinci-sys" }`)
- Modify: `crates/davinci-session/src/lib.rs:59-66` (struct), `:129-199` (`open`), `:394-405` (`write_line`), every `JsonlSession { .. }` literal (`lib.rs:~120`, `lib.rs:~162`, `codec.rs:419`)

**Interfaces:**
- Produces: private `struct WriterState { tail_checked: bool, rewrite_as_v4: bool, lock: Option<std::sync::Arc<davinci_sys::lock::ExclusiveFileLock>> }` with `Default`, stored as `writer: WriterState` on `JsonlSession`. Tasks 3.2 and 3.3 use `rewrite_as_v4` and `lock`.

- [ ] **Step 1: Write the failing tests**

Add to `lib.rs` tests:

```rust
    #[test]
    fn open_skips_an_unterminated_torn_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/work", None).unwrap();
        session.append_entry(test_entry("one")).unwrap();
        let path = session.path.clone();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{\"kind\":\"entry\",\"seq\":9,\"entry\":{\"id\":\"to");
        std::fs::write(&path, bytes).unwrap();
        let reopened = JsonlSession::open(&path).unwrap();
        assert_eq!(reopened.entries.len(), 1);
    }

    #[test]
    fn append_after_torn_tail_does_not_glue_lines() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/work", None).unwrap();
        session.append_entry(test_entry("one")).unwrap();
        let path = session.path.clone();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{\"kind\":\"ent");
        std::fs::write(&path, bytes).unwrap();
        let mut reopened = JsonlSession::open(&path).unwrap();
        reopened.append_entry(test_entry("two")).unwrap();
        let again = JsonlSession::open(&path).unwrap();
        assert_eq!(again.entries.len(), 2);
    }

    #[test]
    fn terminated_corrupt_line_is_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/work", None).unwrap();
        session.append_entry(test_entry("one")).unwrap();
        let path = session.path.clone();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"not json\n");
        std::fs::write(&path, bytes).unwrap();
        assert!(JsonlSession::open(&path).is_err());
    }
```

`test_entry(id)` must build a minimal `SessionEntry`; reuse the helper the existing `lib.rs` tests use for `append_entry` (search the test module for `append_entry(`), and match `JsonlSession::create`'s real argument list at `lib.rs:69`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-session --lib tests::open_skips tests::append_after_torn`
Expected: both FAIL with `Invalid JSONL v4 session`.

- [ ] **Step 3: Implement**

Add the writer state beside the struct:

```rust
/// Per-handle bookkeeping for the first write. Clones share the lock.
#[derive(Debug, Clone, Default)]
struct WriterState {
    /// `truncate_torn_tail` has run for this handle.
    tail_checked: bool,
    /// The file on disk is TS v3; the first write converts it (Task 3.2).
    rewrite_as_v4: bool,
    /// Held from the first write until the last clone drops (Task 3.3).
    lock: Option<std::sync::Arc<davinci_sys::lock::ExclusiveFileLock>>,
}
```

Add `writer: WriterState,` to `JsonlSession` and `writer: WriterState::default(),` to every struct literal (`codec.rs` too; make `WriterState` `pub(crate)`).

In `open`, replace the `BufReader`/`lines` reading with whole-file reading so the loop knows whether the file ends in a newline:

```rust
        let content = fs::read_to_string(path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SessionError::not_found(format!("Session file not found: {}", path.display()))
            } else {
                SessionError::storage(format!("Unable to open session file: {err}"))
            }
        })?;
        let terminated = content.ends_with('\n');
        let physical: Vec<&str> = content.lines().collect();
        let last_index = physical.len().saturating_sub(1);
```

`first` becomes `physical.first()` (empty file keeps the "line 1 is empty" error). `migrate_v3_to_v4` takes an iterator over the remaining lines: change its signature to `rest: impl Iterator<Item = (usize, &'a str)>` (line number, text) and pass `physical.iter().copied().enumerate().skip(1).map(|(i, l)| (i + 1, l))`; inside it, apply the same torn-tail rule. In the v4 loop:

```rust
        for (index, line) in physical.iter().enumerate().skip(1) {
            let line_no = index + 1;
            if line.trim().is_empty() {
                continue;
            }
            match parse_mutation(line) {
                // ... existing Ok arms unchanged ...
                Err(_) if index == last_index && !terminated => {
                    // Torn by a crash mid-append: never acknowledged, drop it.
                    // The first write truncates it on disk (write_line).
                    break;
                }
                Err(err) => {
                    return Err(SessionError::invalid_entry(format!(
                        "Invalid JSONL v4 session {}: line {line_no} {}",
                        path.display(),
                        err
                    )));
                }
            }
        }
```

In `write_line`, before opening for append:

```rust
    fn write_line(&mut self, line: &str) -> Result<(), SessionError> {
        self.prepare_first_write()?;
        self.persist(|path| { /* existing append body unchanged */ })
    }

    /// Runs once per handle before the first append.
    fn prepare_first_write(&mut self) -> Result<(), SessionError> {
        if !self.writer.tail_checked {
            davinci_sys::fs::truncate_torn_tail(&self.path).map_err(|err| {
                SessionError::storage(format!("Unable to repair session tail: {err}"))
            })?;
            self.writer.tail_checked = true;
        }
        Ok(())
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-session`
Expected: PASS (new and existing tests).

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-session Cargo.lock
git commit -m "fix(session): open sessions with a torn tail and truncate it before appending"
```

---

### Task 3.2: A migrated v3 session is rewritten as v4 before its first write

**Finding fixed:** session 2, and a wider case found while planning. `migrate_v3_to_v4` (`codec.rs:354-427`) converts only the in-memory copy. Afterwards:
- `set_name` → `rewrite_header` (`lib.rs:407-431`) drops physical line 1 (`let _ = lines.next()`), which for a v3 file without a session header is the first message, and keeps the v3 lines as they are; the next open parses them as v4 and fails on ISO timestamps.
- **every** `append_entry` appends a v4 mutation line onto the v3 file; the next open goes back through `migrate_v3_to_v4`, whose `parse_v3_line` cannot read that v4 line.

So any write to a resumed TS session corrupts it.

**Behavior:** the first write to a migrated session publishes the whole file as v4 (header + one mutation per entry) with an atomic rename, after copying the original to `<name>.v3.bak` so the TS-readable copy survives. This keeps the documented one-way v3 → v4 upgrade (see memory "Session format": do not write v3).

**Files:**
- Modify: `crates/davinci-session/src/codec.rs:419-426` (set `writer.rewrite_as_v4 = true`)
- Modify: `crates/davinci-session/src/lib.rs` (`prepare_first_write`, `rewrite_header`)

- [ ] **Step 1: Write the failing tests**

```rust
    const V3_NO_HEADER: &str = concat!(
        r#"{"type":"message","id":"a1","parentId":null,"timestamp":"2026-01-01T00:00:00.000Z","message":{"role":"user","content":"hi"}}"#,
        "\n",
        r#"{"type":"message","id":"a2","parentId":"a1","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","content":"hello"}}"#,
        "\n"
    );

    #[test]
    fn renaming_a_migrated_v3_session_keeps_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-01-01_abc.jsonl");
        std::fs::write(&path, V3_NO_HEADER).unwrap();
        let mut session = JsonlSession::open(&path).unwrap();
        assert_eq!(session.entries.len(), 2);
        session.set_name("renamed").unwrap();
        let reopened = JsonlSession::open(&path).unwrap();
        assert_eq!(reopened.entries.len(), 2);
        assert_eq!(reopened.display_name().as_deref(), Some("renamed"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("2026-01-01_abc.jsonl.v3.bak")).unwrap(),
            V3_NO_HEADER
        );
    }

    #[test]
    fn appending_to_a_migrated_v3_session_reopens_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-01-01_abc.jsonl");
        std::fs::write(&path, V3_NO_HEADER).unwrap();
        let mut session = JsonlSession::open(&path).unwrap();
        session.append_entry(test_entry("a3")).unwrap();
        let reopened = JsonlSession::open(&path).unwrap();
        assert_eq!(reopened.entries.len(), 3);
        assert_eq!(reopened.leaf_id.as_deref(), Some("a3"));
    }
```

Check the v3 line shape against the existing v3 fixtures at `lib.rs:~530-565` (the tests that assert `source_format_hint() == 3`) and copy their exact field names if they differ from the above.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-session --lib migrated_v3`
Expected: both FAIL (entry count 1 after rename; reopen error after append).

- [ ] **Step 3: Implement**

`codec.rs`, in the returned struct: `writer: WriterState { rewrite_as_v4: true, ..WriterState::default() },`.

`lib.rs`, extend `prepare_first_write` (after the torn-tail block):

```rust
        if self.writer.rewrite_as_v4 {
            self.publish_as_v4()?;
            self.writer.rewrite_as_v4 = false;
        }
```

and add:

```rust
    /// Replace an on-disk TS v3 file with the v4 image of what we loaded.
    /// The v3 original is kept beside it as `<name>.v3.bak`.
    fn publish_as_v4(&mut self) -> Result<(), SessionError> {
        let mut backup = self.path.as_os_str().to_owned();
        backup.push(".v3.bak");
        if !Path::new(&backup).exists() {
            fs::copy(&self.path, &backup).map_err(|err| {
                SessionError::storage(format!("Unable to back up v3 session: {err}"))
            })?;
        }
        let mut body = encode_header(&self.header);
        for entry in &self.entries {
            body.push_str(&encode_mutation(&SessionMutation::Entry {
                lane: None,
                entry: entry.clone(),
            }));
        }
        let path = self.path.clone();
        self.persist(|_| {
            davinci_sys::fs::atomic_write(&path, body.as_bytes()).map_err(|err| {
                SessionError::storage(format!("Unable to convert session to v4: {err}"))
            })
        })
    }
```

`set_name` calls `rewrite_header`, which does not go through `write_line`; add `self.prepare_first_write()?;` as the first line of `rewrite_header` so a rename also converts first. After conversion, `rewrite_header`'s "drop line 1, keep the rest" is correct because line 1 is now the v4 header.

Migrated v3 entries have `seq == 0` if `parse_v3_line` does not number them. Check: if `next_seq()` would then restart at 1 for every entry, number them in `migrate_v3_to_v4` (`entry.seq = index as u64 + 1`) so the v4 image has unique sequence numbers.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-session
git commit -m "fix(session): convert migrated v3 sessions to v4 before the first write"
```

---

### Task 3.3: One writer per session file

**Finding fixed:** session 3. Nothing stops two processes (`davinci -c` in two terminals) from appending to one session. Both compute `seq` and `leaf_id` from their own memory, producing duplicate `seq` values and silent forks, and `rewrite_header`'s read → temp → rename loses the other side's appends.

**Behavior:** the first write takes `ExclusiveFileLock` on `<session>.jsonl.lock` and holds it for the life of the handle (clones share it). A second process's first write fails with a clear error instead of corrupting the file. Reading (discovery, `/resume` listing, export) never locks.

**Files:**
- Modify: `crates/davinci-session/src/lib.rs` (`prepare_first_write`)
- Modify: the coding-agent code that turns a session write error into a status line, only if the message needs to name the other process (search `persistence_error()` in `crates/davinci-coding-agent/src`)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_second_writer_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = JsonlSession::create(dir.path(), "/work", None).unwrap();
        first.append_entry(test_entry("one")).unwrap();
        let mut second = JsonlSession::open(&first.path).unwrap();
        let err = second.append_entry(test_entry("two")).unwrap_err();
        assert!(err.to_string().contains("open in another davinci process"), "{err}");
        first.append_entry(test_entry("three")).unwrap();
        drop(first);
        let mut third = JsonlSession::open(&second.path).unwrap();
        third.append_entry(test_entry("four")).unwrap();
    }
```

(`second.path` stays valid after the failed write because `persist` records the error on `second` only.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-session --lib a_second_writer_is_refused`
Expected: FAIL, the second append succeeds.

- [ ] **Step 3: Implement**

At the top of `prepare_first_write`:

```rust
        if self.writer.lock.is_none() {
            let mut lock_path = self.path.as_os_str().to_owned();
            lock_path.push(".lock");
            let lock = davinci_sys::lock::ExclusiveFileLock::try_acquire(Path::new(&lock_path))
                .map_err(|err| {
                    if err.kind() == std::io::ErrorKind::WouldBlock {
                        SessionError::storage(format!(
                            "Session {} is open in another davinci process; close it there or start a new session",
                            self.path.display()
                        ))
                    } else {
                        SessionError::storage(format!("Unable to lock session: {err}"))
                    }
                })?;
            self.writer.lock = Some(std::sync::Arc::new(lock));
        }
```

The `<session>.jsonl.lock` file stays on disk after exit (Task 1.5 explains why). Session discovery must skip it: in `discovery.rs`, where it filters `*.jsonl`, confirm `.lock` files are excluded (they end in `.lock`, not `.jsonl`, so an extension check already excludes them; add a test if discovery matches by `contains(".jsonl")`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-session` then `cargo test -p davinci-coding-agent` (sessions are opened by many coding-agent tests; a test that opens the same session twice and writes through both handles now fails and must write through one handle).
Expected: PASS after any such test is fixed.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-session crates/davinci-coding-agent
git commit -m "fix(session): hold an exclusive lock while a session is written"
```

---

### Task 3.4: Settings and trust files: stale locks recover, unparseable files are never overwritten, writes are atomic

**Findings fixed:** coding-agent 6 and 7.
- `load_settings_file` (`settings.rs:957-979`) returns `Settings::default()` for a file that fails to parse; callers such as `handle_package_command` (`packages.rs:41-52`) and `install_and_persist` (`packages.rs:522-548`) load, change and save, so one comment or trailing comma plus `davinci install x` replaces the user's whole settings file with defaults.
- `save_settings` (`settings.rs:1355-1367`) and `write_trust_file` (`trust.rs:266-281`) use plain `fs::write`.
- `acquire_settings_lock` (`settings.rs:1393-1420`) never treats a lock as stale, so one crash leaves `settings.json.lock` or `trust.json.lock` behind and every later save fails.

**Files:**
- Modify: `crates/davinci-coding-agent/src/settings.rs:1355-1420`
- Modify: `crates/davinci-coding-agent/src/trust.rs:266-281`
- Modify: `crates/davinci-coding-agent/src/packages.rs:40-55, 522-548`

**Interfaces:**
- Produces: `pub fn update_settings(agent_dir: &Path, change: impl FnOnce(&mut Settings)) -> Result<Settings, String>` (load, change and save under one lock; refuses when the file on disk does not parse)

- [ ] **Step 1: Write the failing tests**

`settings.rs` tests:

```rust
    #[test]
    fn saving_never_overwrites_a_file_that_does_not_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        std::fs::write(&path, "{ \"theme\": \"dark\", // my comment\n }").unwrap();
        let err = save_settings(dir.path(), &Settings::default()).unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("my comment"));
        assert!(update_settings(dir.path(), |s| s.packages.push("npm:x".into())).is_err());
    }

    #[test]
    fn stale_settings_lock_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let lock = settings_lock_path(&settings_path(dir.path()));
        std::fs::write(&lock, "999999\n").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        save_settings(dir.path(), &Settings::default()).unwrap();
    }

    #[test]
    fn update_settings_changes_one_field_and_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            settings_path(dir.path()),
            r#"{"subagents":{"x":1},"packages":[]}"#,
        )
        .unwrap();
        update_settings(dir.path(), |s| s.packages.push("npm:x".into())).unwrap();
        let raw = std::fs::read_to_string(settings_path(dir.path())).unwrap();
        assert!(raw.contains("subagents"));
        assert!(raw.contains("npm:x"));
    }
```

`File::set_modified` is stable since Rust 1.75, fine on 1.83.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib settings::tests::saving_never settings::tests::stale_settings settings::tests::update_settings`
Expected: FAIL (first overwrites; second errors with "Failed to acquire settings lock"; third does not compile).

- [ ] **Step 3: Implement**

Replace `with_settings_lock`, `SettingsLock` and `acquire_settings_lock` with:

```rust
/// How long a writer waits for another davinci (or TS pi) process to finish
/// its settings write. Writes take milliseconds.
const SETTINGS_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

pub fn with_settings_lock<T>(
    path: &Path,
    write: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guard = davinci_sys::lock::LockFile::acquire(
        &settings_lock_path(path),
        SETTINGS_LOCK_WAIT,
        davinci_sys::lock::DEFAULT_STALE_AFTER,
    )
    .map_err(|err| format!("Failed to acquire settings lock: {err}"))?;
    write()
}
```

Replace `save_settings`:

```rust
pub fn save_settings(agent_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    let path = settings_path(agent_dir);
    with_settings_lock(&path, || write_settings_locked(&path, settings))
}

/// Load, change and save under one lock, so two writers cannot lose each
/// other's change and a file that does not parse is never replaced.
pub fn update_settings(
    agent_dir: &Path,
    change: impl FnOnce(&mut Settings),
) -> Result<Settings, String> {
    fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    let path = settings_path(agent_dir);
    with_settings_lock(&path, || {
        refuse_unparseable(&path)?;
        let mut settings = load_settings_file(&path);
        change(&mut settings);
        write_settings_locked(&path, &settings)?;
        Ok(settings)
    })
}

fn refuse_unparseable(path: &Path) -> Result<(), String> {
    match fs::read_to_string(path) {
        Ok(raw) if !raw.trim().is_empty() && parse_settings_value(&raw).is_none() => Err(format!(
            "{} is not valid JSON; fix or remove it before davinci changes it (nothing was written)",
            path.display()
        )),
        _ => Ok(()),
    }
}

fn write_settings_locked(path: &Path, settings: &Settings) -> Result<(), String> {
    refuse_unparseable(path)?;
    let mut value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    prune_nulls(&mut value);
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    davinci_sys::fs::atomic_write(path, text.as_bytes()).map_err(|err| err.to_string())
}
```

Delete the now-unused `SettingsLock` struct and its `Drop`. Add `davinci-sys` to coding-agent `Cargo.toml` if Task 2.3 did not.

`trust.rs` `write_trust_file`: replace the final `fs::write(path, body)` with `davinci_sys::fs::atomic_write(path, body.as_bytes()).map_err(|err| err.to_string())`.

`packages.rs`: the `"remove" | "uninstall"` arm becomes

```rust
            update_settings(agent_dir, |settings| {
                settings.extensions.retain(|item| item != &source);
                settings.packages.retain(|item| item.source() != source);
            })?;
```

and `install_and_persist` replaces its `load_settings` / push / `save_settings` with one `update_settings(agent_dir, |settings| { ... })` after the install succeeded. (Task 6.7 changes *where* local installs are recorded; keep this task to the persistence mechanics.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib settings:: trust::` and `cargo test -p davinci-coding-agent --bin davinci packages::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent Cargo.lock
git commit -m "fix(settings): atomic writes, stale-lock recovery, never overwrite unparseable settings"
```

---

### Task 3.5: `auth.json`: atomic owner-only writes, a cross-process lock, refresh re-reads under the lock

**Finding fixed:** davinci-ai 6. `AuthStorage::persist` (`auth.rs:271-278`) is a plain `fs::write` with default permissions (0644 on Unix), no temp-and-rename and no lock; `maybe_refresh` (`auth.rs:185-269`) works on the snapshot taken at `open`.
- Two processes refresh a rotating refresh token at once: the loser sends a used refresh token, gets `invalid_grant`, and the user is logged out.
- The last writer overwrites a login another process just saved.
- A crash mid-write leaves invalid JSON; `open` returns `Invalid` and every stored credential is unusable.

**Files:**
- Modify: `crates/davinci-ai/Cargo.toml` (`davinci-sys`, if Task 2.14 did not add it)
- Modify: `crates/davinci-ai/src/auth.rs:121-129, 185-280`

**Interfaces:**
- Produces: private `fn lock_path(&self) -> PathBuf` (`auth.json.lock`) and `fn write_change(&mut self, provider: &str, credential: Option<Credential>) -> Result<(), AuthStorageError>`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn two_handles_do_not_lose_each_others_logins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut a = AuthStorage::open(&path).unwrap();
        let mut b = AuthStorage::open(&path).unwrap();
        a.login_api_key("openai", "sk-a").unwrap();
        b.login_api_key("anthropic", "sk-b").unwrap();
        let fresh = AuthStorage::open(&path).unwrap();
        assert!(fresh.get("openai").is_some());
        assert!(fresh.get("anthropic").is_some());
    }

    #[test]
    fn refresh_adopts_a_token_another_process_already_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let now = 1_000_000;
        let mut stale = AuthStorage::open(&path).unwrap();
        stale
            .login_oauth("openai-codex", "old-access".into(), Some("pi-fixture-r1".into()), Some(now))
            .unwrap();
        // Another process refreshed and saved a token that is valid for an hour.
        let mut other = AuthStorage::open(&path).unwrap();
        other
            .login_oauth("openai-codex", "new-access".into(), Some("pi-fixture-r2".into()), Some(now + 3_600_000))
            .unwrap();
        assert!(stale.maybe_refresh("openai-codex", now, 60_000, false).unwrap());
        assert_eq!(stale.get("openai-codex").unwrap().access.as_deref(), Some("new-access"));
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        AuthStorage::open(&path).unwrap().login_api_key("openai", "sk").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
```

Use the real names of the API-key setter (`login_api_key` is used by `main.rs:6693`) and `login_oauth`'s argument order at `auth.rs:158`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib auth::tests::two_handles auth::tests::refresh_adopts auth::tests::auth_file_is_owner_only`
Expected: FAIL (first loses `openai`; second refreshes again instead of adopting).

- [ ] **Step 3: Implement**

```rust
const AUTH_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

impl AuthStorage {
    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    fn lock(&self) -> Result<davinci_sys::lock::ExclusiveFileLock, AuthStorageError> {
        davinci_sys::lock::ExclusiveFileLock::acquire(&self.lock_path(), AUTH_LOCK_WAIT)
            .map_err(|err| AuthStorageError::Write(format!("auth.json is busy: {err}")))
    }

    fn read_disk(&self) -> Result<HashMap<String, Credential>, AuthStorageError> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|err| AuthStorageError::Invalid(err.to_string())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(err) => Err(AuthStorageError::Read(err.to_string())),
        }
    }

    /// Apply one provider's change to what is on disk *now*, under the lock,
    /// so a concurrent process's other changes survive.
    fn write_change(&mut self, provider: &str, credential: Option<Credential>) -> Result<(), AuthStorageError> {
        let _lock = self.lock()?;
        let mut data = self.read_disk()?;
        match credential {
            Some(credential) => data.insert(provider.to_string(), credential),
            None => data.remove(provider),
        };
        let raw = serde_json::to_string_pretty(&data).map_err(|err| AuthStorageError::Write(err.to_string()))?;
        davinci_sys::fs::atomic_write_private(&self.path, raw.as_bytes())
            .map_err(|err| AuthStorageError::Write(err.to_string()))?;
        self.data = data;
        Ok(())
    }
}
```

`set` becomes `self.write_change(provider, Some(credential))`; `remove` becomes `self.write_change(provider, None)`; every other mutator that called `persist()` after changing `self.data` (search `self.persist()`) calls `write_change` with that provider's new value instead. Delete `persist`.

In `maybe_refresh`, after the early returns for `no_refresh` and non-OAuth, take the lock for the whole refresh and re-read:

```rust
        let _lock = self.lock()?;
        self.data = self.read_disk()?;
        let Some(cred) = self.get(provider).cloned() else {
            return Ok(false);
        };
        if !credential_expires_by(&cred, now_ms.saturating_add(min_expiry_ms)) {
            // Another process refreshed while we waited: use its token.
            return Ok(true);
        }
```

then the existing refresh logic. The inner `login_oauth` call would try to take the same lock again; add a private `store_locked(&mut self, provider, credential)` that does the read-modify-write body of `write_change` **without** locking, and use it inside `maybe_refresh` (the `_lock` guard is already held).

Returning `Ok(true)` when another process refreshed keeps the caller's "credential changed, re-resolve" behavior.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai Cargo.lock
git commit -m "fix(auth): lock and atomically write auth.json, adopt concurrent refreshes"
```

---

### Task 3.6: Vector memory keeps other repositories' records and writes atomically

**Finding fixed:** native-extensions 7. `load_local` (`vector_memory.rs:872-891`) drops records whose `repo_id` differs; `persist_local` (`:893-905`) writes back only what it kept, with a plain `fs::write`. `git remote add origin …` changes `repo_id` (remote first, toplevel path as fallback), and the next index silently deletes every earlier memory. A crash mid-write empties the store.

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs:~800 (struct), 872-905`

**Interfaces:**
- Produces: a new private field `foreign_lines: Vec<String>` on the memory store (raw lines for other `repo_id`s, written back unchanged)

- [ ] **Step 1: Write the failing test**

Add to the vector-memory tests (use the constructor the existing tests use, e.g. `VectorMemory::open_for_test(cwd, repo_id)`; search the test module for how a store is built with a fixed `repo_id`):

```rust
    #[test]
    fn records_of_another_repo_id_survive_a_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = memory_for(dir.path(), "repo-a");
        first.index_text("remember the rust edition", MemoryKind::Fact).unwrap();
        let mut second = memory_for(dir.path(), "repo-b");
        second.index_text("unrelated", MemoryKind::Fact).unwrap();
        let again = memory_for(dir.path(), "repo-a");
        assert!(again.records.iter().any(|r| r.repo_id == "repo-a"));
        let raw = std::fs::read_to_string(again.local_path()).unwrap();
        assert!(raw.contains("repo-a") && raw.contains("repo-b"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib vector_memory::tests::records_of_another_repo_id`
Expected: FAIL (`repo-a` record gone).

- [ ] **Step 3: Implement**

`load_local`:

```rust
    fn load_local(&mut self) {
        let Ok(content) = fs::read_to_string(self.local_path()) else {
            return;
        };
        self.records.clear();
        self.foreign_lines.clear();
        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str::<MemoryRecord>(line) {
                Ok(record) if record.repo_id == self.repo_id || record.repo_id == "*" => {
                    self.records.push(record)
                }
                // Another repo identity (for example before `git remote add`):
                // not ours to use, not ours to delete.
                Ok(_) => self.foreign_lines.push(line.to_string()),
                // Unknown future format: keep it byte-for-byte.
                Err(_) => self.foreign_lines.push(line.to_string()),
            }
        }
        // ... existing `self.known = ...` rebuild unchanged ...
    }
```

`persist_local`:

```rust
    fn persist_local(&self) -> Result<(), ToolError> {
        let mut content = self.foreign_lines.join("\n");
        for record in &self.records {
            if let Ok(line) = serde_json::to_string(record) {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&line);
            }
        }
        content.push('\n');
        davinci_sys::fs::atomic_write(&self.local_path(), content.as_bytes())
            .map_err(|err| ToolError::Failed(err.to_string()))
    }
```

Record cap and retention: add `const MAX_LOCAL_RECORDS: usize = 20_000;` and, in `persist_local`, when `self.records.len()` exceeds it, drop the oldest by `created_at` (use the record's timestamp field name) before writing. Foreign lines are not capped (they are someone else's).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib vector_memory`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/vector_memory.rs
git commit -m "fix(memory): keep other repo records and write the store atomically"
```

---

### Task 3.7: Updating a git package never deletes the working install; git arguments cannot become options

**Finding fixed:** coding-agent 14. `install_git_live` (`packages.rs:742-779`) runs `remove_dir_all(&dest)` before `git clone`, so an offline or auth failure during `update --extensions` destroys a working extension. The URL and ref are passed without `--`, so `git:--upload-pack=…` is parsed as an option.

**Files:**
- Modify: `crates/davinci-coding-agent/src/packages.rs:742-779`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn failed_git_update_keeps_the_existing_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let dest = git_checkout_path(&agent, false, dir.path(), "git:example.invalid/x/y").unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("marker"), "keep me").unwrap();
        let _git = EnvRestore::set("PI_GIT_CMD", "davinci-no-such-git-binary");
        assert!(install_git_live(&agent, false, dir.path(), "git:example.invalid/x/y").is_err());
        assert_eq!(std::fs::read_to_string(dest.join("marker")).unwrap(), "keep me");
    }

    #[test]
    fn option_shaped_git_sources_and_refs_are_refused() {
        assert!(validate_git_arg("--upload-pack=touch /tmp/x").is_err());
        assert!(validate_git_arg("-c").is_err());
        assert!(validate_git_arg("https://github.com/a/b.git").is_ok());
        assert!(validate_git_arg("v1.2.3").is_ok());
    }
```

Use the `EnvRestore`/env-lock helper that `packages.rs` or `main.rs` tests already use.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci packages::tests::failed_git_update packages::tests::option_shaped`
Expected: FAIL (marker deleted; `validate_git_arg` missing).

- [ ] **Step 3: Implement**

```rust
/// A URL or ref that starts with `-` would be read by git as an option.
fn validate_git_arg(value: &str) -> Result<(), String> {
    if value.starts_with('-') {
        Err(format!("refusing git argument that looks like an option: {value}"))
    } else {
        Ok(())
    }
}

fn install_git_live(agent_dir: &Path, local: bool, cwd: &Path, spec: &str) -> Result<(), String> {
    let (url, git_ref) = parse_git_source(spec);
    validate_git_arg(&url)?;
    if let Some(git_ref) = &git_ref {
        validate_git_arg(git_ref)?;
    }
    let dest = git_checkout_path(agent_dir, local, cwd, spec)?;
    let parent = dest.parent().ok_or("git checkout path has no parent")?;
    fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    // Build the new checkout beside the old one; swap only when complete.
    let staging = parent.join(format!(
        ".{}.staging-{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("pkg"),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let git = std::env::var("PI_GIT_CMD").unwrap_or_else(|_| "git".into());
        run_install_command(
            &git,
            &["clone".into(), "--".into(), url.clone(), staging.display().to_string()],
            None,
        )?;
        if let Some(git_ref) = &git_ref {
            run_install_command(&git, &["checkout".into(), git_ref.clone(), "--".into()], Some(&staging))?;
        }
        if staging.join("package.json").exists() {
            let command = npm_command(agent_dir)?;
            let mut args = command[1..].to_vec();
            args.push("install".into());
            if command.last().map(String::as_str) == Some("npm") && command.len() == 1 {
                args.push("--omit=dev".into());
            }
            run_install_command(&command[0], &args, Some(&staging))?;
        }
        Ok::<(), String>(())
    })();
    if let Err(err) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }
    let previous = parent.join(format!(
        ".{}.previous-{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("pkg"),
        uuid::Uuid::new_v4()
    ));
    if dest.exists() {
        fs::rename(&dest, &previous).map_err(|err| err.to_string())?;
    }
    if let Err(err) = fs::rename(&staging, &dest) {
        // Put the old checkout back so the user keeps a working package.
        let _ = fs::rename(&previous, &dest);
        let _ = fs::remove_dir_all(&staging);
        return Err(err.to_string());
    }
    let _ = fs::remove_dir_all(&previous);
    Ok(())
}
```

`run_install_command` resolves the program by name; wrap its `Command::new(program)` in `davinci_sys::process::resolve_program(program)` so `npm` works on Windows (this also covers coding-agent finding 9 for the install path; Task 6.6 covers the remaining npm call sites).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --bin davinci packages::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/packages.rs
git commit -m "fix(packages): stage git updates and refuse option-shaped git arguments"
```

---

### Task 3.8: Runtime log writer repairs its tail, syncs, and rejects out-of-range schema versions

**Finding fixed:** session 19. The reader tolerates a torn last line, but `RuntimeLogWriter::open` (`runtime_log.rs:47-54`) appends straight onto it, after which that line is no longer last and `read_runtime_log` returns `CorruptRecord` for the whole log. `append` flushes but never syncs. `ver as u16` (`:108`) silently truncates a large version.

**Files:**
- Modify: `crates/davinci-session/src/runtime_log.rs:47-65, 106-114`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn writer_repairs_a_torn_tail_before_appending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.runtime.jsonl");
        std::fs::write(&path, "{\"schema_version\":1,\"a\":1}\n{\"schema_version\":1,\"a\":").unwrap();
        let mut writer = RuntimeLogWriter::open(&path).unwrap();
        writer.append(&serde_json::json!({"schema_version": 1, "a": 2})).unwrap();
        let rows: Vec<serde_json::Value> = read_runtime_log(&path).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn huge_schema_version_is_unsupported_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.runtime.jsonl");
        std::fs::write(&path, "{\"schema_version\":65537}\n").unwrap();
        let err = read_runtime_log::<serde_json::Value>(&path).unwrap_err();
        assert!(matches!(err, RuntimeLogError::UnsupportedSchemaVersion { .. }));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-session --lib runtime_log`
Expected: both new tests FAIL.

- [ ] **Step 3: Implement**

In `open`, before opening for append: `davinci_sys::fs::truncate_torn_tail(&path)?;` (the `?` works if `RuntimeLogError` has `From<std::io::Error>`; it does, since `open` already uses `?` on io calls).

In `append`, replace `self.file.flush()?;` with `self.file.sync_data()?;`.

In the version check:

```rust
        if let Some(ver) = value.get("schema_version").and_then(|v| v.as_u64()) {
            if ver > u64::from(CURRENT_RUNTIME_SCHEMA_VERSION) {
                return Err(RuntimeLogError::UnsupportedSchemaVersion {
                    found: u16::try_from(ver).unwrap_or(u16::MAX),
                    supported: CURRENT_RUNTIME_SCHEMA_VERSION,
                });
            }
        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-session/src/runtime_log.rs
git commit -m "fix(session): repair runtime log tail, sync appends, reject oversized schema versions"
```

---

### Task 3.9: Every ad-hoc "atomic" write uses `davinci_sys::fs::atomic_write`

**Findings fixed:** native-extensions 17 (governor output store writes non-atomically and never repairs a torn `out-*.txt`) and 18 (learning store and workspace snapshot delete the destination before renaming, so a crash between the two loses the file), plus the Windows transient-rename failure that becomes a terminal `persistence_error` in the graph store (native-extensions 3).

**Files and exact replacements:**

| Site | Replace with |
|---|---|
| `token_governor.rs:514-516` (`if !path.exists() { fs::write(..) }`) | `if !stored_output_is_intact(&path, &digest) { davinci_sys::fs::atomic_write(&path, content.as_bytes())?; }` where `stored_output_is_intact` reads the file and compares `file_content_hash` to `digest` |
| `learning/store.rs:250-259` (`save_state`) | `davinci_sys::fs::atomic_write(&state_path, json_str.as_bytes()).map_err(|e| e.to_string())` |
| `learning/store.rs:268-302` (`compact`, both files) | build each file's content in a `String`, then `davinci_sys::fs::atomic_write` |
| `workspace_snapshot/mod.rs:912-928` (`atomic_write`) | body becomes `davinci_sys::fs::atomic_write(path, bytes).map_err(|error| format!("publish atomic file: {error}"))` |
| `graph/store.rs:292-294` (`atomic_write`) | keep the function and its test seam; make the production path `davinci_sys::fs::atomic_write(path, content)` so it gets the Windows rename retry. Keep `atomic_write_with` for the existing interrupted-publish test |
| `permissions.rs:218-226` (`write_atomically`) | `davinci_sys::fs::atomic_write(path, text.as_bytes()).map_err(|err| err.to_string())` |
| `learning/skill_manager.rs:118` (`atomic_write_file`) | same delegation |
| `davinci-agent/src/tool_ledger.rs:147` (`atomic_write_json`) | same delegation |

- [ ] **Step 1: Write the failing test (governor, the one with a behavior change)**

```rust
    #[test]
    fn torn_stored_output_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutputStore::new(dir.path().to_path_buf());
        let saved = store.save("full content").unwrap();
        let path = dir.path().join(format!("{}.txt", saved.id));
        std::fs::write(&path, "full con").unwrap(); // torn by a crash
        store.save("full content").unwrap();
        assert_eq!(store.load(&saved.id).unwrap(), "full content");
    }
```

Use the store type and constructor name at `token_governor.rs:~495-503`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib token_governor::tests::torn_stored_output`
Expected: FAIL (returns `"full con"`).

- [ ] **Step 3: Make every replacement in the table**

Keep each function's signature; only bodies change. Run `rg -n "remove_file\(&(state_path|candidates_path|skills_path)|let _ = fs::remove_file\(path\);" crates/davinci-coding-agent/src/native_extensions` afterwards: expected no output.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent -p davinci-coding-agent --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent crates/davinci-coding-agent
git commit -m "fix(storage): route ad-hoc atomic writes through davinci-sys"
```

---

### Task 3.10: SQLite store: safe concurrent open, correct FTS maintenance, no swallowed errors

**Findings fixed:** session 7, 8, 9. (Whether the SQLite backend stays at all is decision D4 in `00-index.md`; if Julien chooses removal, skip this task and do Task 10.2 instead.)
- `open` sets `busy_timeout` after `journal_mode=WAL` (`lib.rs:126-131`), so the WAL switch fails at once with `SQLITE_BUSY` under contention.
- `apply_migrations` (`lib.rs:138-174`) checks, applies and records in separate steps with no transaction; two processes opening a fresh DB both apply, and the second fails with `UNIQUE constraint failed`.
- `insert_entry_row` uses `INSERT OR REPLACE` (`lib.rs:~408`); with `recursive_triggers` off, REPLACE's implicit delete does not fire `session_search_fts_ad`, so each re-import leaves orphan FTS rows. The FTS table is created outside migrations and never rebuilt.
- `persist_session` (`lib.rs:697-704`) turns any error from the `next_seq` query into `1` (`unwrap_or(1)`), replaying the whole log.

**Files:**
- Modify: `crates/davinci-session-sqlite/src/lib.rs:118-175, 405-415, 697-704`
- Create: `crates/davinci-session-sqlite/migrations/002_fts_rebuild.sql`

- [ ] **Step 1: Write the failing tests**

In `crates/davinci-session-sqlite/tests/` (new file `concurrency.rs`):

```rust
use davinci_session_sqlite::SqliteSessionStore;

#[test]
fn two_threads_opening_a_fresh_database_both_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || SqliteSessionStore::open(&path).map(|_| ()))
        })
        .collect();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
}
```

In `lib.rs` tests:

```rust
    #[test]
    fn reimporting_a_session_keeps_the_fts_index_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteSessionStore::open(&dir.path().join("s.db")).unwrap();
        let session = sample_session_with_entries(dir.path(), 3);
        store.upsert_session(&session).unwrap();
        store.upsert_session(&session).unwrap();
        store
            .conn
            .execute_batch("INSERT INTO session_search_fts(session_search_fts, rank) VALUES('integrity-check', 1);")
            .unwrap();
    }
```

Use the store's real type name and a session builder from the existing tests (search for `upsert_session(` in the test module).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-session-sqlite`
Expected: the concurrency test fails intermittently with `UNIQUE constraint failed` or `database is locked` (run it 5 times: `for i in 1 2 3 4 5; do cargo test -p davinci-session-sqlite --test concurrency || break; done`); the FTS integrity check fails with `database disk image is malformed`.

- [ ] **Step 3: Implement**

`open`: move `conn.busy_timeout(...)` to directly after `Connection::open`, before any pragma.

`apply_migrations`:

```rust
    pub fn apply_migrations(&self) -> Result<(), SessionError> {
        fn storage(what: &'static str) -> impl Fn(rusqlite::Error) -> SessionError {
            move |err| SessionError::storage(format!("{what}: {err}"))
        }
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage("Unable to start migration"))?;
        let result = (|| {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS migrations (id TEXT PRIMARY KEY, applied_at TEXT NOT NULL);",
                )
                .map_err(storage("Unable to create migrations table"))?;
            for (id, sql) in [
                ("001_initial.sql", INITIAL_MIGRATION_SQL),
                ("002_fts_rebuild.sql", FTS_REBUILD_MIGRATION_SQL),
            ] {
                let applied: Option<String> = self
                    .conn
                    .query_row("SELECT id FROM migrations WHERE id = ?1", [id], |row| row.get(0))
                    .optional()
                    .map_err(storage("Unable to read migrations"))?;
                if applied.is_none() {
                    self.conn.execute_batch(sql).map_err(storage(id))?;
                    self.conn
                        .execute(
                            "INSERT INTO migrations (id, applied_at) VALUES (?1, ?2)",
                            params![id, chrono_now()],
                        )
                        .map_err(storage("Unable to record migration"))?;
                }
            }
            Ok(())
        })();
        let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        self.conn.execute_batch(end).map_err(storage("Unable to finish migration"))?;
        result
    }
```

`ensure_search_schema` must run **before** `002` (the rebuild needs the FTS table): call `ensure_search_schema()` first in `open`, then `apply_migrations()`. Since `ensure_search_schema` only uses `IF NOT EXISTS`, it is safe to run concurrently.

`migrations/002_fts_rebuild.sql`:

```sql
-- Repopulate the external-content FTS index from `entries`. Rows written
-- before the index existed, and rows orphaned by INSERT OR REPLACE, are
-- fixed by one rebuild.
INSERT INTO session_search_fts(session_search_fts) VALUES('rebuild');
```

with `const FTS_REBUILD_MIGRATION_SQL: &str = include_str!("../migrations/002_fts_rebuild.sql");` beside `INITIAL_MIGRATION_SQL`.

`insert_entry_row`:

```sql
INSERT INTO entries (session_id, seq, id, parent_id, type, timestamp, payload)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(session_id, id) DO UPDATE SET
    seq = excluded.seq,
    parent_id = excluded.parent_id,
    type = excluded.type,
    timestamp = excluded.timestamp,
    payload = excluded.payload
```

(An UPDATE fires `session_search_fts_au`, which maintains the index correctly.)

`persist_session`:

```rust
            let next: i64 = self
                .conn
                .query_row(
                    "SELECT next_seq FROM session_sequences WHERE session_id = ?1",
                    [&session.id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|err| SessionError::storage(format!("Unable to read next sequence: {err}")))?
                .unwrap_or(1);
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-session-sqlite` (the concurrency test 5 times, as above).
Expected: PASS every time.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-session-sqlite
git commit -m "fix(sqlite): transactional migrations, busy timeout first, FTS-safe upserts, surface sequence errors"
```

---

## Phase 3 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-sys -p davinci-session -p davinci-session-sqlite -p davinci-ai -p davinci-agent -p davinci-coding-agent
```

Then the manual check in `00-index.md` → "Manual verification: durability".
