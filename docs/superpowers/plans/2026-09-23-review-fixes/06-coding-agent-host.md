# Phase 6: Coding-Agent Host Implementation Plan (`davinci-coding-agent`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** RPC clients, hooks, JS extensions, installed packages and Windows users get a host that does not hang, does not drop input, and finds the programs and files it is told to use.

**Architecture:** Child processes go through `davinci_sys::process::run_bounded` / `resolve_program` (Task 1.3). Package source parsing moves into a library module so the resource loader (library) and the installer (binary) share one definition of where a package lives. RPC gets a stdin watcher that keeps reading during a turn.

**Tech Stack:** Rust 1.83, Node.js for the extension runner (unchanged).

Depends on Phase 1. Independent of Phases 2-5 except where noted.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-coding-agent/src/main.rs:3205-3212, 3425-3440, 3658-3684` | 6.1, 6.2 |
| `crates/davinci-agent/src/lib.rs`, `turn.rs:608-620` (`inject_queued`) | 6.2 |
| `crates/davinci-coding-agent/src/hooks.rs:~680-800` | 6.3 |
| `crates/davinci-coding-agent/src/js_host.rs:164-175, 226-320, 520-570` | 6.4, 6.13 |
| `crates/davinci-coding-agent/src/extension_runner.js` | 6.4 |
| `crates/davinci-coding-agent/src/package_source.rs` (create) | 6.5 |
| `crates/davinci-coding-agent/src/packages.rs:422-520, 550-561, 612-650, 680-730, 960-980` | 6.5, 6.6, 6.7 |
| `crates/davinci-coding-agent/src/settings.rs:758-860` | 6.5 |
| `crates/davinci-coding-agent/src/main.rs:863, 6142, 7560, 7574, 7791` | 6.5 |
| `crates/davinci-coding-agent/src/trust.rs:126-131` | 6.8 |
| `crates/davinci-coding-agent/src/llama.rs:478-495` | 6.9 |
| `crates/davinci-coding-agent/src/davinci_surfaces.rs:612-625` | 6.10 |
| `crates/davinci-coding-agent/src/extension_host.rs:235-292` | 6.11 |
| `crates/davinci-coding-agent/src/extension_host.rs:1193-1204`, `js_host.rs:879-899` | 6.12 |
| `crates/davinci-coding-agent/src/external_editor.rs:52-63` | 6.13 |
| `crates/davinci-coding-agent/src/main.rs:335, 835-853`, `settings.rs:1124-1136` | 6.14 |

---

### Task 6.1: One malformed RPC line is answered with an error; the session continues

**Finding fixed:** coding-agent 10. `main.rs:3212` does `serde_json::from_str(&line).map_err(...)?`, which returns from `run_rpc_with_host` and ends the session on the first bad line.

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs:3212`

- [ ] **Step 1: Write the failing test**

An RPC e2e test in `crates/davinci-coding-agent/tests/` (next to the existing RPC tests; search `--mode rpc` or `"get_state"` in `tests/`):

```rust
#[test]
fn a_malformed_rpc_line_does_not_end_the_session() {
    let mut rpc = RpcChild::spawn_offline();
    rpc.send_raw("{ this is not json");
    let error = rpc.read_response();
    assert_eq!(error["success"], false);
    assert!(error["error"].as_str().unwrap().contains("parse"));
    rpc.send(serde_json::json!({"id": "2", "type": "get_state"}));
    let state = rpc.read_response();
    assert_eq!(state["id"], "2");
    assert_eq!(state["success"], true);
}
```

Reuse the RPC child helper the existing RPC tests use.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --test <rpc test file> a_malformed_rpc_line`
Expected: FAIL (process exits after the first line).

- [ ] **Step 3: Implement**

```rust
        let mut command: RpcCommand = match serde_json::from_str(&line) {
            Ok(command) => command,
            Err(err) => {
                let response = rpc::fail_response(None, "parse", format!("parse error: {err}"));
                output::write_raw_stdout_line(
                    &serde_json::to_string(&response).map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?;
                continue;
            }
        };
```

- [ ] **Step 4: Run to verify pass, then commit**

Run: the Step 2 command → PASS; `cargo test -p davinci-coding-agent --tests rpc` → PASS.

```bash
git add crates/davinci-coding-agent
git commit -m "fix(rpc): answer malformed lines with an error instead of ending the session"
```

---

### Task 6.2: RPC `abort`, steer and follow-up work while a turn is running

**Finding fixed:** coding-agent 11. The RPC loop runs `complete_prompt_with_host` synchronously (`main.rs:3438-3440`) and reads no input until the turn finishes (except inside a UI dialog, `rpc_wait_ui_response`). An `abort` sent during generation is handled after the turn; the `is_streaming` steer/followUp branch in `handle_rpc` (`rpc.rs:281-305`) can never run.

**Behavior:** while a turn runs, a watcher thread takes lines from the same channel (`rx`). It handles `abort` (sets the turn's abort signal), and `prompt` with `streamingBehavior: "steer" | "followUp"` (queues the text on a thread-safe inbox the agent drains at its next `inject_queued`), and replies to each immediately. Any other command is pushed back to `leftover` in order and handled after the turn, as today.

**Files:**
- Modify: `crates/davinci-agent/src/lib.rs` (add `remote_queue`), `crates/davinci-agent/src/turn.rs:608-620`
- Modify: `crates/davinci-coding-agent/src/main.rs:3425-3440, 3672-3684`

**Interfaces:**
- Produces:
  - `pub struct RemoteQueue(Arc<Mutex<Vec<(QueueKind, QueuedMessage)>>>)` in `davinci-agent`, `Clone`, with `push_steer(text, images)`, `push_follow_up(text, images)`
  - `Agent::remote_queue(&self) -> RemoteQueue`
  - `inject_queued` first moves everything from the remote queue into `self.queues`

- [ ] **Step 1: Write the failing tests**

`davinci-agent` unit test:

```rust
    #[test]
    fn remote_steers_are_injected_at_the_next_turn_boundary() {
        let mut agent = fixture_agent_that_calls_a_tool_once();
        let remote = agent.remote_queue();
        remote.push_steer("use the other file".into(), Vec::new());
        agent.run_prompt("go").unwrap();
        assert!(agent.messages.iter().any(|m| m.role == "user" && m.text().contains("use the other file")));
    }
```

RPC e2e test:

```rust
#[test]
fn abort_during_a_turn_stops_it_promptly() {
    let mut rpc = RpcChild::spawn_with_slow_fixture_model(std::time::Duration::from_secs(30));
    rpc.send(serde_json::json!({"id": "1", "type": "prompt", "message": "go"}));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let started = std::time::Instant::now();
    rpc.send(serde_json::json!({"id": "2", "type": "abort"}));
    rpc.read_until_id("1");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}
```

The slow fixture model is whatever the existing tests use for a provider that streams slowly (a `PI_*_REPLY` fixture with a delay, or the loopback server helper in `davinci-ai` tests); if none exists, add a fixture provider that sleeps between deltas and checks the abort flag.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib remote_steers` and the e2e test.
Expected: FAIL.

- [ ] **Step 3: Implement `RemoteQueue`**

`davinci-agent/src/queues.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Steer,
    FollowUp,
}

/// Messages a host receives on another thread while a turn runs. The loop
/// moves them into `SteerFollowUpQueues` at its next injection point.
#[derive(Debug, Clone, Default)]
pub struct RemoteQueue(std::sync::Arc<std::sync::Mutex<Vec<(QueueKind, QueuedMessage)>>>);

impl RemoteQueue {
    pub fn push_steer(&self, text: String, images: Vec<davinci_ai::MessageContent>) {
        self.push(QueueKind::Steer, text, images);
    }
    pub fn push_follow_up(&self, text: String, images: Vec<davinci_ai::MessageContent>) {
        self.push(QueueKind::FollowUp, text, images);
    }
    fn push(&self, kind: QueueKind, text: String, images: Vec<davinci_ai::MessageContent>) {
        let mut queue = self.0.lock().unwrap_or_else(|err| err.into_inner());
        queue.push((kind, QueuedMessage::new(text, images)));
    }
    pub(crate) fn drain(&self) -> Vec<(QueueKind, QueuedMessage)> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|err| err.into_inner()))
    }
}
```

Match `QueuedMessage`'s real constructor and image type (see `enqueue_steer_with(&text, images)` in `rpc.rs:296`). Add `remote: RemoteQueue` to `Agent` (default), `pub fn remote_queue(&self) -> RemoteQueue { self.remote.clone() }`, and at the top of `inject_queued`:

```rust
        for (kind, message) in self.remote.drain() {
            match kind {
                QueueKind::Steer => self.queues.steer.push(message),
                QueueKind::FollowUp => self.queues.follow_up.push(message),
            }
        }
```

- [ ] **Step 4: Implement the RPC watcher**

In `main.rs`, wrap the turn call at `:3438-3440`:

```rust
            let remote = runtime.agent.remote_queue();
            let (reply, events) = std::thread::scope(|scope| {
                let stop = std::sync::atomic::AtomicBool::new(false);
                let watcher = scope.spawn(|| rpc_watch_during_turn(&leftover, &rx, &ui_abort, &remote, &stop));
                let result = rpc_with_ui_abort(&mut runtime.agent, &ui_abort, |agent| {
                    complete_prompt_with_host(&prompt_args, agent, Some(host.clone()), false)
                });
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = watcher.join();
                result
            });
```

```rust
/// Read RPC lines while a turn runs. `abort` and streaming prompts are
/// handled now; everything else waits in `leftover`, in order.
fn rpc_watch_during_turn(
    leftover: &Mutex<std::collections::VecDeque<String>>,
    rx: &Mutex<std::sync::mpsc::Receiver<String>>,
    active: &RpcUiAbort,
    remote: &davinci_agent::RemoteQueue,
    stop: &std::sync::atomic::AtomicBool,
) {
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        let line = match rx.lock() {
            Ok(rx) => match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            },
            Err(_) => return,
        };
        let parsed: Option<RpcCommand> = serde_json::from_str(&line).ok();
        let handled = match &parsed {
            Some(command) if command.kind == "abort" => {
                if let Some(signal) = active.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    signal.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                rpc_write_ok(command.id.clone(), "abort");
                true
            }
            Some(command) if command.kind == "prompt" => match (command.streaming_behavior.as_deref(), command.message.clone()) {
                (Some("steer"), Some(text)) => {
                    remote.push_steer(text, Vec::new());
                    rpc_write_ok(command.id.clone(), "prompt");
                    true
                }
                (Some("followUp"), Some(text)) => {
                    remote.push_follow_up(text, Vec::new());
                    rpc_write_ok(command.id.clone(), "prompt");
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if !handled {
            leftover.lock().unwrap_or_else(|e| e.into_inner()).push_back(line);
        }
    }
}
```

`rpc_write_ok` writes `rpc::ok_response(id, kind, None)` as one stdout line through `output::write_raw_stdout_line` (use the real `ok` helper name from `rpc.rs`). Steer text is not template-expanded here (the agent is borrowed by the turn); expand it with a clone of `runtime.agent.skills` / `templates` taken before the scope, passed into the watcher. Images in streaming prompts: parse `command.images` with `davinci_agent::parse_rpc_images` as `rpc.rs:276` does.

`rpc_emit_and_wait_ui` also reads `rx` during the turn (for UI dialogs). Both run on different threads now; the watcher must not steal a UI reply. Route by content: in the watcher, a line whose `type` is the UI-response type (`extension_ui_response`, see `rpc_wait_ui_response`) is pushed to a second queue `ui_replies` that `rpc_emit_and_wait_ui` reads first. Add that queue next to `leftover`.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p davinci-agent` and `cargo test -p davinci-coding-agent --tests rpc`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-agent crates/davinci-coding-agent
git commit -m "fix(rpc): handle abort, steer and follow-up while a turn is running"
```

---

### Task 6.3: Hooks run through `run_bounded`

**Finding fixed:** coding-agent 15. `hooks.rs:715-719` writes the payload to stdin before the timeout loop starts (a hook that does not read stdin, with a payload larger than the pipe, blocks until it exits); the readers stop reading at `MAX_HOOK_STREAM_BYTES` (a chatty hook then blocks writing and is killed as a timeout); after exit, `join()` waits for EOF, so a backgrounded grandchild holding stdout hangs the agent; on Unix the child has no process group, so `kill(-pid)` in `kill_process_tree` hits nothing.

**Files:**
- Modify: `crates/davinci-coding-agent/src/hooks.rs:~680-826`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_hook_that_ignores_a_large_stdin_payload_still_times_out_on_schedule() {
        let dir = tempfile::tempdir().unwrap();
        let script = if cfg!(windows) { "ping -n 30 127.0.0.1 >NUL" } else { "sleep 30" };
        let payload = serde_json::json!({ "output": "x".repeat(4 * 1024 * 1024) });
        let started = std::time::Instant::now();
        let result = run_hook_command(dir.path(), script, &payload, Some(500), "read");
        assert!(result.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn a_hook_that_backgrounds_a_child_returns_when_it_exits() {
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let result = run_hook_command(dir.path(), "sleep 30 & exit 0", &serde_json::json!({}), Some(10_000), "read");
        assert!(result.is_ok());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
```

`run_hook_command` stands for the function that contains lines 700-826 (use its real name and argument order).

- [ ] **Step 2: Run to verify failure**

Run: `timeout 120 cargo test -p davinci-coding-agent --lib hooks::tests::a_hook_that`
Expected: FAIL (the first takes 30 s; the second hangs).

- [ ] **Step 3: Implement**

Keep the part of the function that builds `cmd` (program, args, env, `creation_flags(0x08000000)` on Windows). Replace everything from `let mut child = match cmd.spawn()` to the final `Err(format!("hook ... blocked ..."))` with:

```rust
    let timeout = timeout_ms.map(Duration::from_millis).unwrap_or(HOOK_TIMEOUT);
    let output = match davinci_sys::process::run_bounded(
        cmd,
        Some(payload_str.into_bytes()),
        davinci_sys::process::RunLimits { timeout, output_cap: MAX_HOOK_STREAM_BYTES },
        &|| false,
    ) {
        Ok(output) => output,
        Err(err) => {
            GLOBAL_HOOK_TELEMETRY.blocked.fetch_add(1, Ordering::Relaxed);
            return Err(format!("hook `{program}` failed: {err}"));
        }
    };
    if output.timed_out {
        GLOBAL_HOOK_TELEMETRY.timed_out.fetch_add(1, Ordering::Relaxed);
        GLOBAL_HOOK_TELEMETRY.blocked.fetch_add(1, Ordering::Relaxed);
        return Err(format!("hook `{program}` timed out after {}s", timeout.as_secs()));
    }
    if output.status.is_some_and(|status| status.success()) {
        return Ok(());
    }
    GLOBAL_HOOK_TELEMETRY.blocked.fetch_add(1, Ordering::Relaxed);
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stdout).into_owned();
    }
    Err(format!("hook `{program}` blocked {tool}: {}", text.trim()))
```

Delete `kill_process_tree` if nothing else uses it. If the hook function's callers need the hook's stdout on success (for example `sessionStart` hooks that inject context), return `output.stdout` in that path instead of `()`; follow the current function's return type.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib hooks`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/hooks.rs crates/davinci-coding-agent/Cargo.toml
git commit -m "fix(hooks): run hooks through run_bounded so stdin, grandchildren and big output cannot hang"
```

---

### Task 6.4: JS extensions: the UI-wait path cannot hang, stderr is always drained, `console.log` cannot break the protocol, provider streams are not killed at 120 s

**Findings fixed:** coding-agent 12, 13 and the `streamSimple` half of 22.
- UI-wait path (`js_host.rs:552-562`): polls `try_wait` and the UI channel, but drains stdout/stderr only after the child exits, with no timeout; an extension reply larger than the pipe buffer blocks the child forever while `NODE_LOCK` is held.
- Persistent runners (`js_host.rs:243-251`): stderr is piped and never read; when its buffer fills, the runner blocks and is killed after 120 s as "hung".
- `extension_runner.js` has no `console` redirection; any `console.log` in a handler writes a non-JSON line to stdout; `read_line` fails, and the pooled path respawns and retries, which fails again (`js_host.rs:460-467`).
- `streamSimple` (a JS provider's generation) is subject to the 120 s reply timeout, so long generations are killed.

**Files:**
- Modify: `crates/davinci-coding-agent/src/extension_runner.js` (top of file)
- Modify: `crates/davinci-coding-agent/src/js_host.rs:236-320, 520-570`

- [ ] **Step 1: Write the failing tests**

Add fixture extensions under `crates/davinci-coding-agent/tests/fixtures/extensions/` (the directory the JS host tests already use):
- `noisy/index.js`: a command handler that does `console.log("hello from handler")` and `console.error("x".repeat(200000))` and then returns a result.
- `big-reply/index.js`: a UI-wait handler whose reply is 1 MB of JSON.

```rust
    #[test]
    fn console_output_in_a_handler_does_not_break_the_reply() {
        let module = fixture_extension("noisy");
        let result = run_js_extension(&module, "command", &serde_json::json!({"name": "noisy"})).unwrap();
        assert!(result.ok);
    }

    #[test]
    fn a_large_reply_on_the_ui_wait_path_completes() {
        let module = fixture_extension("big-reply");
        let started = std::time::Instant::now();
        let result = run_js_extension_with_ui_channel(&module, "command", &serde_json::json!({})).unwrap();
        assert!(result.ok);
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }
```

Mark both `#[ignore]` when Node.js is absent the way existing JS tests do (`if !node_available() { return; }`). Use the real names of the two entry points.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci js_host::tests::console_output js_host::tests::a_large_reply`
Expected: FAIL (parse error; hang past 20 s).

- [ ] **Step 3: Redirect console in the runner**

At the top of `extension_runner.js`, before any extension module is imported:

```js
// stdout carries one JSON reply per line to davinci. Anything an extension
// prints goes to stderr so it can never be mistaken for a reply.
const util = require("node:util");
for (const name of ["log", "info", "debug", "warn", "error", "trace", "dir"]) {
  console[name] = (...args) => process.stderr.write(util.format(...args) + "\n");
}
```

If the runner is an ES module (`import` syntax), use `import util from "node:util";` instead of `require`.

- [ ] **Step 4: Drain stderr and skip non-JSON lines in Rust**

In `PersistentJsSession::start`, after taking stdout:

```rust
        if let Some(stderr) = child.stderr.take() {
            let module_name = module.display().to_string();
            std::thread::spawn(move || {
                // Keep the pipe empty; surface the last lines in traces only.
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if davinci_ai::trace::enabled() {
                        davinci_ai::trace::log(&format!("extension {module_name}: {}", line.chars().take(500).collect::<String>()));
                    }
                }
            });
        }
```

In the stdout reader thread, forward only lines that start with `{` (after trimming); log others through the same trace call. That makes an extension that writes to `process.stdout` directly harmless as well.

- [ ] **Step 5: Fix the UI-wait path**

Before the `while child.try_wait()...` loop, start reader threads for stdout and stderr that accumulate into shared buffers (same pattern as `davinci_sys::process::drain`, capped at 16 MB for stdout and 64 KB for stderr), and add a deadline:

```rust
        let deadline = std::time::Instant::now() + extension_reply_timeout();
        while child.try_wait().map_err(|err| err.to_string())?.is_none() {
            poll_ui_channel(dir);
            if std::time::Instant::now() >= deadline {
                davinci_sys::process::kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(dir);
                return Err(format!("{EXTENSION_TIMEOUT_MARK} after {}s", extension_reply_timeout().as_secs()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
```

After exit, join the readers (they end at EOF; if a grandchild keeps the pipe, wait at most 500 ms, then take what was read) and parse stdout as before.

The UI-wait deadline applies to *extension computation*; time the user spends answering a dialog must not count. `poll_ui_channel` dispatches the dialog synchronously; measure it and extend `deadline` by the time spent inside it.

- [ ] **Step 6: No reply timeout for provider streams**

Find the `streamSimple` call path (`rg -n "streamSimple" crates/davinci-coding-agent/src/js_host.rs`). Give `send`/`read_line` a `timeout: Option<Duration>` parameter; pass `Some(extension_reply_timeout())` everywhere except `streamSimple`, which passes an idle timeout of 300 s *between lines* (the stream sends deltas as lines), not a total timeout.

- [ ] **Step 7: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --bin davinci js_host`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/davinci-coding-agent/src/js_host.rs crates/davinci-coding-agent/src/extension_runner.js crates/davinci-coding-agent/tests/fixtures/extensions
git commit -m "fix(extensions): drain runner stderr, keep console output off the protocol, bound UI waits, no 120s cap on provider streams"
```

---

### Task 6.5: Installed npm and git packages contribute their resources

**Finding fixed:** coding-agent 8. `collect_package_resources` (`settings.rs:758-771`) uses `Path::new(pkg.source())` literally, so it looks for a directory named `npm:foo` or `git:github.com/x/y`; installed packages never contribute extensions, skills, prompts or themes. Also, local sources without a manifest are walked recursively through `read_dir` following symlinks with no depth limit, which can overflow the stack on a symlink loop (pnpm `node_modules`).

**Files:**
- Create: `crates/davinci-coding-agent/src/package_source.rs` (library module; declare `pub mod package_source;` in `lib.rs` and `mod package_source;` in `main.rs`)
- Modify: `crates/davinci-coding-agent/src/packages.rs:422-520, 612-650` (move definitions out; `pub use crate::package_source::{...};` so existing call sites compile)
- Modify: `crates/davinci-coding-agent/src/settings.rs:758-860`
- Modify: the five callers in `main.rs` (`:863, :6142, :7560, :7574, :7791`) and two in `settings.rs` tests (`:1880, :1884`)

**Interfaces:**
- Produces:
  - `package_source::{ParsedSource, parse_package_source, parse_git_source, git_checkout_path, npm_install_root, git_install_root}` (moved verbatim)
  - `package_source::installed_root(source: &str, agent_dir: &Path, cwd: &Path) -> Option<PathBuf>`
  - `settings::collect_package_resources(pkg: &PackageSource, kind: &str, agent_dir: &Path, cwd: &Path) -> Vec<PathBuf>`

- [ ] **Step 1: Write the failing tests**

`package_source.rs` tests:

```rust
    #[test]
    fn installed_root_finds_global_then_project_installs() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("project");
        let global = agent.join("npm").join("node_modules").join("@scope").join("tool");
        std::fs::create_dir_all(&global).unwrap();
        assert_eq!(installed_root("npm:@scope/tool@1.2.3", &agent, &cwd), Some(global));
        let local = cwd.join(".pi").join("npm").join("node_modules").join("other");
        std::fs::create_dir_all(&local).unwrap();
        assert_eq!(installed_root("npm:other", &agent, &cwd), Some(local));
        assert_eq!(installed_root("npm:missing", &agent, &cwd), None);
    }

    #[test]
    fn local_sources_resolve_to_their_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            installed_root(&dir.path().display().to_string(), dir.path(), dir.path()),
            Some(dir.path().to_path_buf())
        );
    }
```

`settings.rs` tests:

```rust
    #[test]
    fn npm_package_skills_are_collected_from_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let root = agent.join("npm").join("node_modules").join("skills-pack");
        std::fs::create_dir_all(root.join("skills").join("demo")).unwrap();
        std::fs::write(root.join("skills").join("demo").join("SKILL.md"), "---\nname: demo\n---\n").unwrap();
        let pkg: PackageSource = "npm:skills-pack".into();
        let found = collect_package_resources(&pkg, "skills", &agent, dir.path());
        assert!(found.iter().any(|path| path.ends_with("SKILL.md")), "{found:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_do_not_recurse_forever() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("a").join("loop")).unwrap();
        let pkg: PackageSource = dir.path().display().to_string().as_str().into();
        let _ = collect_package_resources(&pkg, "skills", dir.path(), dir.path());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib package_source settings::tests::npm_package_skills settings::tests::symlink_loops`
Expected: FAIL to compile.

- [ ] **Step 3: Move and add**

Move `ParsedSource`, `parse_package_source`, `parse_git_source`, `git_checkout_path` (make it `pub`), `npm_install_root`, `git_install_root` from `packages.rs` into `package_source.rs` unchanged. In `packages.rs`, add `pub use crate::package_source::{git_checkout_path, git_install_root, npm_install_root, parse_git_source, parse_package_source, ParsedSource};`.

Add:

```rust
/// Where an installed package's files are. Global installs first, then the
/// project-local ones (`install -l`), matching the installer's layout.
pub fn installed_root(source: &str, agent_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match parse_package_source(source) {
        ParsedSource::Local(path) => vec![crate::package_source::expand_local(&path, cwd)],
        ParsedSource::Npm { name, .. } => [false, true]
            .iter()
            .map(|local| npm_install_root(agent_dir, *local, cwd).join("node_modules").join(&name))
            .collect(),
        ParsedSource::Git(_) => [false, true]
            .iter()
            .filter_map(|local| git_checkout_path(agent_dir, *local, cwd, source).ok())
            .collect(),
    };
    candidates.into_iter().find(|path| path.is_dir())
}

/// `~/x` and relative local sources, resolved the way the installer stores them (Task 6.6).
pub fn expand_local(path: &str, cwd: &Path) -> PathBuf {
    let expanded = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => davinci_session::home_dir().map(|home| home.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    };
    if expanded.is_absolute() { expanded } else { cwd.join(expanded) }
}
```

`settings.rs`:

```rust
pub fn collect_package_resources(
    pkg: &PackageSource,
    kind: &str,
    agent_dir: &Path,
    cwd: &Path,
) -> Vec<PathBuf> {
    let Some(root) = crate::package_source::installed_root(pkg.source(), agent_dir, cwd) else {
        return Vec::new();
    };
    // ... existing body, with `root` a PathBuf ...
}
```

Replace `collect_dir_resources` and `collect_glob_files_from` recursion with `walkdir` (already a dependency), never following symlinks and capped at depth 16:

```rust
fn collect_dir_resources(root: &Path, dir: &Path, pkg: &PackageSource, kind: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .max_depth(16)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        push_if_allowed(root, entry.into_path(), pkg, kind, &mut out);
    }
    out
}
```

and the same shape for the glob walk (filter with `glob_match(pattern, relative)`).

Update the five `main.rs` callers to pass `&default_agent_dir()` and the relevant cwd (`&agent.cwd` or `cwd`), and the `settings.rs:1880/1884` test calls.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent
git commit -m "fix(packages): load resources from installed npm and git packages; never follow symlink loops"
```

---

### Task 6.6: Windows: npm, editors and `~` paths work

**Finding fixed:** coding-agent 9 and 17. `Command::new("npm")` (`packages.rs:695, 716, 973`) cannot start `npm.cmd` on Windows, so `install npm:…`, `update --extensions`, the npm step for git packages and update checks all fail unless `npmCommand`/`PI_NPM_CMD` is set. `dirs_home()` (`packages.rs:559-561`) reads only `HOME`, usually unset on Windows, so `~/` never expands; relative local sources are stored unresolved and later resolved against whatever the cwd is.

**Files:**
- Modify: `crates/davinci-coding-agent/src/packages.rs:550-561, 680-730, 960-980` (every `Command::new(` in the file)

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn local_sources_are_stored_absolute() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ext")).unwrap();
        assert_eq!(
            normalize_local_source("./ext", dir.path()).unwrap(),
            dir.path().join("ext").canonicalize().map(|p| crate::trust::display_path(&p)).unwrap()
        );
    }

    #[test]
    fn every_command_in_packages_goes_through_resolve_program() {
        let source = include_str!("packages.rs");
        for (number, line) in source.lines().enumerate() {
            if line.contains("Command::new(") && !line.contains("resolve_program") && !line.trim_start().starts_with("//") {
                panic!("packages.rs:{} spawns without resolve_program: {line}", number + 1);
            }
        }
    }
```

(The second test is a cheap guard against regressions; keep it in the test module of `packages.rs` itself. `display_path` comes from Task 6.8.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci packages::tests::local_sources packages::tests::every_command`
Expected: FAIL.

- [ ] **Step 3: Implement**

- Wrap every program name: `std::process::Command::new(davinci_sys::process::resolve_program(program))`.
- Delete `dirs_home`; `resolve_local_path` becomes `crate::package_source::expand_local(path, &cwd)`.
- Add `fn normalize_local_source(path: &str, cwd: &Path) -> Result<String, String>` that expands, canonicalizes, strips `\\?\` (Task 6.8's `display_path`) and returns the string to persist; use it in `install_and_persist` for `ParsedSource::Local` so settings store an absolute path.

- [ ] **Step 4: Run to verify pass, then commit**

Run: `cargo test -p davinci-coding-agent --bin davinci packages` → PASS. On a Windows machine also run `cargo run -p davinci-coding-agent -- install npm:<small public package>` once and confirm it installs (manual step, record the result in the PR).

```bash
git add crates/davinci-coding-agent/src/packages.rs
git commit -m "fix(packages): resolve npm through PATHEXT, expand ~ with the real home, store local sources absolute"
```

---

### Task 6.7: `install -l` / `remove -l` write the project settings and `remove` deletes the install

**Finding fixed:** coding-agent 16. `--local` changes only where files go; the source is always saved to the agent-dir settings (`packages.rs:522-548`), and `remove` never deletes files (`packages.rs:48-53`).

**Files:**
- Modify: `crates/davinci-coding-agent/src/packages.rs:40-55, 522-548`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn local_install_is_recorded_in_the_project_and_removed_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("project");
        let ext = dir.path().join("ext");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let _cwd = CurrentDirRestore::set(&cwd);
        handle_package_command("install", &[ext.display().to_string(), "-l".into()], &agent).unwrap();
        let project = crate::settings::load_settings(&cwd.join(".pi"));
        assert!(project.packages.iter().any(|p| p.source().ends_with("ext")));
        assert!(crate::settings::load_settings(&agent).packages.is_empty());
        handle_package_command("remove", &[ext.display().to_string(), "-l".into()], &agent).unwrap();
        assert!(crate::settings::load_settings(&cwd.join(".pi")).packages.is_empty());
    }
```

(`CurrentDirRestore`: whatever cwd guard the tests use; `install_and_persist` reads `current_dir()`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci local_install_is_recorded`
Expected: FAIL.

- [ ] **Step 3: Implement**

`settings_dir_for(local, agent_dir, cwd)`: `cwd.join(".pi")` when local (or `project_config::resolve(cwd, "")`'s dir if a `.davinci` exists), else `agent_dir`. Use it for `update_settings` (Task 3.4) in install and remove. In `remove`, after updating settings, delete the install: for npm run `npm uninstall <name> --prefix <root>` through `run_install_command`; for git, `remove_dir_all(git_checkout_path(...))`; local sources are never deleted (they are the user's directory).

- [ ] **Step 4: Run to verify pass, then commit**

Run: `cargo test -p davinci-coding-agent --bin davinci packages` → PASS.

```bash
git add crates/davinci-coding-agent/src/packages.rs
git commit -m "fix(packages): -l records in project settings; remove uninstalls what it installed"
```

---

### Task 6.8: Trust keys on Windows match TS pi and show plain paths

**Finding fixed:** coding-agent 18. `canonicalize_trust_path` (`trust.rs:126-131`) uses `fs::canonicalize`, which returns `\\?\C:\…` on Windows. Entries written by TS pi in `~/.pi/agent/trust.json`, and `trustedProjects` in settings, never match; trust labels show `\\?\`.

**Files:**
- Modify: `crates/davinci-coding-agent/src/trust.rs:126-131`

**Interfaces:**
- Produces: `pub fn display_path(path: &Path) -> String` (canonical string without the verbatim prefix)

- [ ] **Step 1: Write the failing test**

```rust
    #[cfg(windows)]
    #[test]
    fn trust_paths_have_no_verbatim_prefix() {
        let dir = tempdir().unwrap();
        let key = canonicalize_trust_path(dir.path());
        assert!(!key.starts_with(r"\\?\"), "{key}");
    }

    #[cfg(windows)]
    #[test]
    fn a_trust_entry_written_by_ts_pi_is_found() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        fs::create_dir_all(&agent).unwrap();
        let plain = project.canonicalize().unwrap().display().to_string().trim_start_matches(r"\\?\").to_string();
        fs::write(agent.join("trust.json"), serde_json::json!({ plain: true }).to_string()).unwrap();
        assert!(resolve_project_trusted(&agent, &project, None, Some("ask"), &[]));
    }
```

- [ ] **Step 2: Run to verify failure (Windows)**

Run: `cargo test -p davinci-coding-agent --lib trust::tests::trust_paths_have_no trust::tests::a_trust_entry_written`
Expected: FAIL on Windows.

- [ ] **Step 3: Implement**

```rust
pub fn canonicalize_trust_path(path: &Path) -> String {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    display_path(&canonical)
}

/// A canonical path as TS pi writes it: Node's `fs.realpathSync` never adds
/// the `\\?\` verbatim prefix that `std::fs::canonicalize` returns on Windows.
pub fn display_path(path: &Path) -> String {
    davinci_agent::strip_verbatim_prefix(path)
        .to_string_lossy()
        .into_owned()
}
```

(`strip_verbatim_prefix` is `pub` at `davinci-agent/src/permission.rs:1510`; re-export it from `davinci_agent` if it is not already.)

Existing stored keys that *do* carry `\\?\` (written by current davinci) must still match: in `find_nearest_trust_entry`, look up both `key` and `format!(r"\\?\{key}")`.

- [ ] **Step 4: Run to verify pass, then commit**

Run: `cargo test -p davinci-coding-agent --lib trust` → PASS.

```bash
git add crates/davinci-coding-agent/src/trust.rs crates/davinci-agent/src/lib.rs
git commit -m "fix(trust): store and compare Windows trust paths without the verbatim prefix"
```

---

### Task 6.9: llama.cpp streaming decodes multi-byte characters split across reads

**Finding fixed:** coding-agent 19. `llama.rs:486` runs `String::from_utf8_lossy` on each 2048-byte chunk, so a character split between chunks becomes U+FFFD (CJK, emoji).

**Files:**
- Modify: `crates/davinci-coding-agent/src/llama.rs:478-495`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn utf8_split_across_chunks_is_preserved() {
        let body = "data: {\"content\":\"日本\"}\n\n".as_bytes().to_vec();
        // Cut inside the first CJK character.
        let split = body.iter().position(|b| *b >= 0x80).unwrap() + 1;
        let reader = ChunkedReader::new(vec![body[..split].to_vec(), body[split..].to_vec()]);
        let mut texts = Vec::new();
        read_llama_sse(reader, None, |event| texts.push(event)).unwrap();
        assert!(format!("{texts:?}").contains("日本"));
    }
```

`ChunkedReader` is a small test `Read` impl that returns one given chunk per `read` call; `read_llama_sse` stands for the function containing lines 478-495.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci llama::tests::utf8_split`
Expected: FAIL (`\u{fffd}`).

- [ ] **Step 3: Implement**

Keep undecoded bytes in a `Vec<u8>`:

```rust
    let mut pending: Vec<u8> = Vec::new();
    // ...
            Ok(n) => {
                pending.extend_from_slice(&chunk[..n]);
                let valid_up_to = match std::str::from_utf8(&pending) {
                    Ok(_) => pending.len(),
                    Err(err) if err.error_len().is_none() => err.valid_up_to(), // incomplete tail
                    Err(err) => err.valid_up_to() + err.error_len().unwrap_or(1), // invalid byte: lossy
                };
                let text = String::from_utf8_lossy(&pending[..valid_up_to]).replace("\r\n", "\n");
                pending.drain(..valid_up_to);
                buffer.push_str(&text);
                // ... existing frame loop unchanged ...
            }
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --bin davinci llama` → PASS.

```bash
git add crates/davinci-coding-agent/src/llama.rs
git commit -m "fix(llama): keep multi-byte characters split across stream reads"
```

---

### Task 6.10: Truncating a string for display never cuts inside a character

**Finding fixed:** coding-agent 20. `davinci_surfaces.rs:620-623` calls `String::truncate(max_len)`, which panics when `max_len` is inside a character; reached from the context inspector preview (`:686`, 4096 bytes) with non-ASCII source or reason text.

**Files:**
- Modify: `crates/davinci-coding-agent/src/davinci_surfaces.rs:620-623`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn sanitize_truncation_respects_char_boundaries() {
        let text = "é".repeat(3000); // 6000 bytes, 2 per char
        let out = sanitize_for_preview(&text, 4095);
        assert!(out.contains("[truncated"));
    }
```

(use the real function name around line 612.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci sanitize_truncation`
Expected: FAIL with `assertion failed: self.is_char_boundary(new_len)`.

- [ ] **Step 3: Implement**

```rust
    if sanitized.len() > max_len {
        let mut cut = max_len;
        while !sanitized.is_char_boundary(cut) {
            cut -= 1;
        }
        let truncated_bytes = sanitized.len() - cut;
        sanitized.truncate(cut);
        sanitized.push_str(&format!("\n... [truncated {truncated_bytes} bytes]"));
    }
```

Run `rg -n "\.truncate\(" crates/davinci-coding-agent/src crates/davinci-tui/src` and apply the same guard at any other `String::truncate` whose length is not already a char boundary.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-coding-agent crates/davinci-tui
git commit -m "fix(ui): truncate display strings on char boundaries"
```

---

### Task 6.11: Extension load failures are reported

**Finding fixed:** coding-agent 21. `extension_host.rs:235-292` keeps a module only `if let Ok(loaded) … if loaded.ok`; errors and `ok: false` results vanish, so users see missing commands and tools with no explanation.

**Files:**
- Modify: `crates/davinci-coding-agent/src/extension_host.rs:235-292` (and the `ExtensionHost` struct)
- Modify: the `/extensions` sheet and startup notices (`rg -n "fn render_extensions|/extensions" crates/davinci-coding-agent/src`)

**Interfaces:**
- Produces: `ExtensionHost.load_errors: Vec<(String /* module */, String /* error */)>`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_failing_extension_is_listed_with_its_error() {
        let host = host_with_fixture_extensions(&["throws-on-load"]);
        assert_eq!(host.load_errors.len(), 1);
        assert!(host.load_errors[0].1.contains("boom"));
    }
```

Fixture `tests/fixtures/extensions/throws-on-load/index.js`: `throw new Error("boom");`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci a_failing_extension_is_listed`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
                match run_js_extension(&module, "load", &load_payload) {
                    Ok(loaded) if loaded.ok => { /* existing body */ }
                    Ok(loaded) => host.load_errors.push((
                        module.display().to_string(),
                        loaded.error.unwrap_or_else(|| "extension reported ok: false".into()),
                    )),
                    Err(error) => host.load_errors.push((module.display().to_string(), error)),
                }
```

(`loaded.error`: use the result struct's error field name.) Show each entry once at startup as a notice (`Extension <module> failed to load: <error>`) and as rows in `/extensions`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-coding-agent
git commit -m "fix(extensions): report extension load failures at startup and in /extensions"
```

---

### Task 6.12: Manifest `command` tools receive the model's arguments and have a timeout

**Finding fixed:** coding-agent 23. Manifest command tools (`extension_host.rs:1193-1204`, `js_host.rs:879-899`) ignore the model's arguments and run with no timeout or output cap.

**Behavior:** the arguments are passed as JSON on stdin and in `DAVINCI_TOOL_ARGS` (TS pi convention: check the vendor extension docs for the variable name it uses and use the same), through `run_bounded` with a 120 s timeout and 1 MiB output cap (manifest may override `timeoutMs`).

**Files:**
- Modify: `crates/davinci-coding-agent/src/js_host.rs:879-899`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn manifest_command_tool_gets_its_arguments() {
        let script = if cfg!(windows) { "more" } else { "cat" }; // echoes stdin
        let result = run_manifest_command_tool(script, &serde_json::json!({"path": "a.txt"}), None).unwrap();
        assert!(result.content.contains("\"path\""), "{}", result.content);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci manifest_command_tool_gets`
Expected: FAIL.

- [ ] **Step 3: Implement**

Build the `Command` as today (shell wrapper on each OS), then:

```rust
    let args_json = serde_json::to_string(args).map_err(|err| err.to_string())?;
    command.env("DAVINCI_TOOL_ARGS", &args_json);
    let output = davinci_sys::process::run_bounded(
        command,
        Some(args_json.into_bytes()),
        davinci_sys::process::RunLimits {
            timeout: Duration::from_millis(timeout_ms.unwrap_or(120_000)),
            output_cap: 1024 * 1024,
        },
        &|| false,
    )
    .map_err(|err| err.to_string())?;
```

and map `timed_out` / non-zero status to an error result.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --bin davinci js_host` → PASS.

```bash
git add crates/davinci-coding-agent/src/js_host.rs
git commit -m "fix(extensions): pass arguments to manifest command tools and bound their run"
```

---

### Task 6.13: The external editor launches reliably and respects a cancelled edit

**Finding fixed:** coding-agent 24. `external_editor.rs:52-63` splits `$EDITOR` on whitespace (quoted paths with spaces break), runs `code --wait` without PATHEXT (fails on Windows), ignores the exit status (`:cq` in vim still submits), and does not normalize CRLF (check `normalize`; if it already strips `\r`, only the first three remain).

**Files:**
- Modify: `crates/davinci-coding-agent/src/external_editor.rs:52-63`

**Interfaces:**
- Produces: `fn split_command_line(line: &str) -> Vec<String>` (POSIX-shell-like: whitespace separates, `'…'` and `"…"` group, `\` escapes outside single quotes)

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn editor_command_lines_split_like_a_shell() {
        assert_eq!(split_command_line("code --wait"), ["code", "--wait"]);
        assert_eq!(
            split_command_line(r#""C:\Program Files\Sublime\subl.exe" -w"#),
            [r"C:\Program Files\Sublime\subl.exe", "-w"]
        );
        assert_eq!(split_command_line("vim -c 'set tw=72'"), ["vim", "-c", "set tw=72"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_zero_editor_exit_cancels_the_edit() {
        let editor = ExternalEditor::new("false", "draft").unwrap();
        assert!(editor.edit().is_err());
    }
```

On Windows keep backslashes literal inside double quotes (they are path separators); only treat `\"` as an escape.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci external_editor`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
        let parts = split_command_line(&self.command);
        let (program, args) = parts.split_first().ok_or_else(|| "empty editor command".to_string())?;
        let mut cmd = Command::new(davinci_sys::process::resolve_program(program));
        cmd.args(args).arg(&self.file_path);
        let status = cmd.status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("editor exited with {status}; the edit was discarded"));
        }
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent --bin davinci external_editor` → PASS.

```bash
git add crates/davinci-coding-agent/src/external_editor.rs
git commit -m "fix(editor): shell-style command parsing, PATHEXT lookup, honour a cancelled edit"
```

---

### Task 6.14: Startup order and process-global state

**Findings fixed:** coding-agent "also noted":
- `js_host.rs:165-175` writes the runner to a predictable, never-cleaned `%TEMP%/pi-extension-runner/extension_runner-<pid>.js`; on multi-user Unix another user can pre-create that directory and swap the file (SUSPECTED).
- `main.rs:835-853` and `settings.rs:1124-1136` call `std::env::set_var` after MCP and background threads have started, which is unsound on Unix (glibc `setenv` races `getenv`).
- `main.rs:335` applies the project `httpProxy` before `--trust` / `--no-trust` is parsed, so a trusted project's proxy is applied even with `--no-trust`.

**Files:**
- Modify: `crates/davinci-coding-agent/src/js_host.rs:164-175`
- Modify: `crates/davinci-coding-agent/src/main.rs:335, 835-853`, `settings.rs:1124-1136`

- [ ] **Step 1: Runner file in the agent directory, content-addressed, owner-only**

```rust
fn runner_path() -> Result<PathBuf, String> {
    static PATH: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    PATH.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(RUNNER_JS.as_bytes()));
        let path = davinci_session::default_agent_dir()
            .join("runtime")
            .join(format!("extension_runner-{}.js", &digest[..16]));
        let current = std::fs::read(&path).ok();
        if current.as_deref() != Some(RUNNER_JS.as_bytes()) {
            davinci_sys::fs::atomic_write_private(&path, RUNNER_JS.as_bytes())
                .map_err(|err| err.to_string())?;
        }
        Ok(path)
    })
    .clone()
}
```

Test: `runner_path()` returns a path under the agent dir whose content equals `RUNNER_JS`; on Unix its mode is 0600.

- [ ] **Step 2: Move every `set_var` before the first thread**

List them: `rg -n "set_var\(" crates/davinci-coding-agent/src/main.rs crates/davinci-coding-agent/src/settings.rs`. For each that runs after `McpRegistry::connect`, the update-check thread, or any `thread::spawn`, move the computation earlier in `main` (after argument parsing, before any thread starts), or pass the value explicitly instead of through the environment. Add a comment at the new location: `// set_var is only sound before other threads exist.`

- [ ] **Step 3: Apply `httpProxy` after trust is known**

Move the `httpProxy` application from `main.rs:335` to after `parsed.project_trust_override` is available, and read the project value only when `is_trusted(&settings, cwd, parsed.project_trust_override)` is true. Test: a project with `.pi/settings.json` `{"httpProxy":"http://127.0.0.1:9"}` run with `--no-trust` leaves `HTTP_PROXY` unset (unit-test the function that computes the proxy, not the global env).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-coding-agent
git commit -m "fix(startup): owner-only runner file, set_var before threads, proxy only for trusted projects"
```

---

## Phase 6 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-coding-agent
```

Then `00-index.md` → "Manual verification: host" (Windows `install npm:…`, RPC abort, a noisy extension).
