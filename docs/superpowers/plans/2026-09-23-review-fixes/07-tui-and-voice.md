# Phase 7: TUI and Voice Implementation Plan (`davinci-tui`, `davinci-voice`, the davinci render loop)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The terminal is always left in the state the UI needs (raw, alternate screen) while davinci runs and always restored when it exits; untrusted text cannot drive the terminal; the transcript and the composer behave on long sessions and wide screens; voice keeps the speech it hears.

**Architecture:** A generation counter makes a stale `Session` unable to tear down a newer one. One sanitizer runs where transcript entries are built. Each event loop reacquires the terminal at the top of every iteration. Voice changes stay inside the worker and the C++ shim.

**Tech Stack:** Rust 1.83, ratatui 0.29, crossterm 0.28, whisper.cpp (vendored).

Independent of Phases 2-6.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-tui/src/davinci/runtime.rs:85-130, 322-385, 440-460, 612-620` | 7.1, 7.2, 7.4 |
| `crates/davinci-coding-agent/src/davinci_interactive.rs:7575-7576, 1461-1520, 3055-3087, 4204-4260, 4306-4740` | 7.1, 7.2, 7.9, 7.10 |
| `crates/davinci-tui/src/davinci/sanitize.rs` (create), `model.rs:567-582`, `transcript.rs:98-127, 217`, `app.rs:333-341`, `runtime.rs:439-441` | 7.3 |
| `crates/davinci-tui/src/davinci/model.rs:2588-2602` | 7.5 |
| `crates/davinci-tui/src/editor.rs:63-98, 201-233, 390-425, 1002-1010` | 7.6, 7.7 |
| `crates/davinci-tui/src/davinci/ui.rs:120-135, 533` | 7.8 |
| `crates/davinci-tui/src/davinci/app.rs:122-126, 151-154, 274-279, 620-625`, `cogitator.rs:211-218` | 7.11 |
| `crates/davinci-coding-agent/src/main.rs:4761-4777` | 7.12 |
| `crates/davinci-voice/src/bin/davinci-voice-worker.rs:50-160`, `native/voice_shim.cpp:~54`, `src/engine.rs:~99`, `src/catalog.rs:53-65` | 7.13, 7.14, 7.15 |

---

### Task 7.1: A stale `Session` cannot restore the terminal under a newer one; a failed `open` leaves raw mode off

**Findings fixed:** TUI 1 (checked in source) and 7. `davinci_interactive.rs:7575-7576` does `*self.terminal = session` with a freshly opened `Session`; assigning drops the **old** `Session` after the new one set the global `HELD = true`, and the old one's `Drop` → `close()` → `restore()` tears down the new session (raw mode off, alternate screen left). After `/login` the UI draws over the shell with echo on, and Ctrl+C kills pi. Separately, `Session::open` (`runtime.rs:325-347`) stores `HELD = true` then returns early with `?` if `EnterAlternateScreen` or `PushKeyboardEnhancementFlags` fails; no `Session` exists, so nothing restores raw mode.

**Files:**
- Modify: `crates/davinci-tui/src/davinci/runtime.rs:85-130, 322-350, 368-385, 612-620`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs:7575-7576`

- [ ] **Step 1: Write the failing tests**

The terminal side effects are global, so test the ownership logic through a seam: move the "may this session restore?" decision into a pure function.

```rust
    #[test]
    fn only_the_newest_session_may_restore() {
        let first = Generation::begin();
        let second = Generation::begin();
        assert!(!first.is_current());
        assert!(second.is_current());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib davinci::runtime::tests::only_the_newest_session`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the generation**

```rust
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Which `Session` owns the terminal. Opening a session starts a new
/// generation; a session whose generation is no longer current (it was
/// replaced) must not restore the terminal when it drops.
#[derive(Debug, Clone, Copy)]
struct Generation(u64);

impl Generation {
    fn begin() -> Self {
        Self(GENERATION.fetch_add(1, Ordering::SeqCst) + 1)
    }
    fn is_current(self) -> bool {
        GENERATION.load(Ordering::SeqCst) == self.0
    }
}
```

Add `generation: Generation` to `Session`, set by `open`. `close` becomes:

```rust
    pub fn close(&mut self) -> io::Result<()> {
        if self.generation.is_current() {
            restore()
        } else {
            Ok(())
        }
    }
```

`restore()` (the free function the panic hook calls) is unchanged: on a panic, whoever owns the terminal, it must be given back.

- [ ] **Step 4: Make `open` restore on partial failure**

```rust
    pub fn open() -> io::Result<Self> {
        install_panic_hook();
        let generation = Generation::begin();
        let opened = (|| {
            enable_raw_mode()?;
            HELD.store(true, Ordering::SeqCst);
            let mut out = io::stdout();
            execute!(out, EnterAlternateScreen)?;
            let _ = execute!(out, EnableBracketedPaste);
            let disambiguated = supports_keyboard_enhancement();
            if disambiguated {
                execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
                DISAMBIGUATED.store(true, Ordering::SeqCst);
            }
            Ok((Terminal::new(CrosstermBackend::new(out))?, disambiguated))
        })();
        let (terminal, disambiguated) = match opened {
            Ok(parts) => parts,
            Err(err) => {
                let _ = restore();
                return Err(err);
            }
        };
        Ok(Self { terminal, generation, /* existing fields, using `disambiguated` */ })
    }
```

- [ ] **Step 5: Use `reacquire` after `/login`**

`davinci_interactive.rs:7575-7576` becomes:

```rust
        if let Err(err) = self.terminal.reacquire() {
            return Next::Fail(err.to_string());
        }
```

(The login flow released the terminal before printing the URL; `reacquire` takes it back without creating a second `Session`.) Check the code just above 7575 for how the terminal was released for login; if it called `restore()` or `close()`, `reacquire` is the exact inverse.

- [ ] **Step 6: Run and verify by hand**

Run: `cargo test -p davinci-tui` → PASS. Manual (Windows and one Unix terminal): start `davinci`, run `/login openai-codex`, cancel the browser step, return: typing must not echo to the shell and Ctrl+C must not exit davinci. Record the result in the PR.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-tui crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "fix(tui): stale sessions cannot restore the terminal; failed open restores raw mode; /login reacquires"
```

---

### Task 7.2: Every event loop reacquires the terminal; the panic hook restores only for main-thread panics

**Finding fixed:** TUI 2. The panic hook (`runtime.rs:116-125`) calls `restore()` for every panic on every thread, including panics later caught by `catch_unwind` (`extension_host.rs:1675`, `runtime/bus.rs:100/150/175`, `security_scan/controller.rs:350`, `cache/singleflight.rs:51`). Only the turn loop calls `reacquire()` (`davinci_interactive.rs:1702`); the idle loop (`:4306-4739`) and the scope-expansion modal (`:3055-3087`) keep painting onto the primary screen. `draw()` (`runtime.rs:452-458`) then re-enables mouse capture while in cooked mode.

**Files:**
- Modify: `crates/davinci-tui/src/davinci/runtime.rs:116-125, 448-458`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs` (top of the idle loop, the turn loop, the scope-expansion modal loop)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_caught_panic_on_another_thread_does_not_restore() {
        let restored = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = restored.clone();
        let decide = panic_restores_terminal;
        let handle = std::thread::spawn(move || {
            flag.store(decide(), std::sync::atomic::Ordering::SeqCst);
        });
        handle.join().unwrap();
        assert!(!restored.load(std::sync::atomic::Ordering::SeqCst));
        assert!(std::thread::current().name() != Some("main") || panic_restores_terminal());
    }
```

(`panic_restores_terminal()` is the pure predicate the hook will call.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib a_caught_panic_on_another_thread`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
static MAIN_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Only a panic on the thread that owns the UI ends the UI. A worker
/// thread's panic is usually caught (`catch_unwind`) and the UI keeps
/// running; restoring the terminal for it breaks the screen.
fn panic_restores_terminal() -> bool {
    MAIN_THREAD
        .get()
        .is_some_and(|main| *main == std::thread::current().id())
}
```

`install_panic_hook` records `MAIN_THREAD.get_or_init(|| std::thread::current().id())` (it is called from `Session::open`, on the UI thread), and the hook body becomes:

```rust
        std::panic::set_hook(Box::new(move |info| {
            if panic_restores_terminal() {
                let _ = restore();
            }
            previous(info);
        }));
```

An uncaught panic on a worker thread that brings the process down still leaves the terminal restored, because the main thread's `Session` drops during process exit (or, for `abort`, the OS resets on process exit; document this edge in a comment).

In `draw`, do nothing if the terminal is not held:

```rust
        if !HELD.load(Ordering::SeqCst) {
            return Ok(());
        }
```

At the top of every iteration of the idle loop, the turn loop and the scope-expansion modal loop in `davinci_interactive.rs`, add `let _ = terminal.reacquire();` (it is a single atomic load when already held).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-tui -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-tui crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "fix(tui): restore on main-thread panics only; every loop reacquires the terminal"
```

---

### Task 7.3: Untrusted text cannot write control sequences to the terminal

**Finding fixed:** TUI 3. Nothing strips control characters from tool output (`model.rs:567-582`), prose/tool rows (`transcript.rs:98-127, 217`), extension rows (`app.rs:333-341`) or the window title (`runtime.rs:439-441`, from the session name). unicode-width 0.2 counts control characters as width 1 inside a string, and ratatui 0.29's `Paragraph` only skips width-0 symbols, so ESC, CR and BEL reach the terminal. A tool that prints a file containing `\x1b]52;c;…\x07` writes the clipboard; `\x1b[2J` clears the screen; mid-line `\r` from cargo/npm progress output moves the cursor and corrupts the row permanently (ratatui's diff thinks it is fine).

**Files:**
- Create: `crates/davinci-tui/src/davinci/sanitize.rs` (declare in `davinci/mod.rs`)
- Modify: `model.rs:567-582`, `transcript.rs` (where prose and tool rows become `Span`s), `app.rs:333-341`, `runtime.rs:439-441`

**Interfaces:**
- Produces: `pub fn terminal_safe(text: &str) -> std::borrow::Cow<'_, str>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_and_controls_are_removed() {
        assert_eq!(terminal_safe("a\x1b]52;c;SGVsbG8=\x07b"), "ab");
        assert_eq!(terminal_safe("x\x1b[2Jy"), "xy");
        assert_eq!(terminal_safe("bell\x07"), "bell");
        assert_eq!(terminal_safe("tab\tkept"), "tab\tkept");
    }

    #[test]
    fn carriage_return_keeps_what_the_terminal_would_show() {
        assert_eq!(terminal_safe("10%\r50%\r100%"), "100%");
        assert_eq!(terminal_safe("line\r\n"), "line\n");
    }

    #[test]
    fn plain_text_is_not_copied() {
        assert!(matches!(terminal_safe("plain text"), std::borrow::Cow::Borrowed(_)));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib davinci::sanitize`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
//! Text from tools, models, extensions and session names is shown, never
//! executed: escape sequences are removed and other control characters
//! dropped, except `\n` and `\t`. A bare `\r` keeps what a terminal would
//! leave visible: the text after the last `\r` on that line.

use std::borrow::Cow;

pub fn terminal_safe(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Cow::Borrowed(text);
    }
    let stripped = crate::ansi::strip_terminal_sequences(text);
    let mut out = String::with_capacity(stripped.len());
    for (index, line) in stripped.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let visible = line.rsplit('\r').next().unwrap_or(line);
        out.extend(visible.chars().filter(|c| !c.is_control() || *c == '\t'));
    }
    Cow::Owned(out)
}
```

(`strip_terminal_sequences` is exported from `ansi.rs:509`; confirm it handles OSC terminated by BEL and by `ESC \`, and CSI; extend it if not, with its own test.)

Apply it once where text enters the model: in `tool_output_rows` (`let lines: Vec<String> = terminal_safe(text).lines()...`), in the constructors that create prose and extension `Entry` values, and to the title before `SetTitle`. Grep for other entry points: `rg -n "Entry::(Prose|Tool|Extension|Note)" crates/davinci-tui/src crates/davinci-coding-agent/src/davinci_interactive.rs`.

Markdown rendering of assistant prose uses its own styled spans; sanitize the raw text before it is parsed, so styling still works.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-tui -p davinci-coding-agent` → PASS. Manual: `!printf 'a\033]52;c;SGVsbG8=\007b\n'` (Unix) in davinci; the clipboard is unchanged and the row shows `ab`.

```bash
git add crates/davinci-tui crates/davinci-coding-agent
git commit -m "fix(tui): strip escape sequences and control characters from displayed text"
```

---

### Task 7.4: Mouse motion does not redraw; frames are drawn only when something changed

**Finding (performance):** TUI 4, redraw half. The turn loop polls every 40 ms and composes a full frame each time (`davinci_interactive.rs:1504/1513`); every input event, including mouse *motion* (crossterm's `EnableMouseCapture` turns on any-motion tracking, mode 1003), composes a frame; on the GraphRun screen `route_graph_mouse` composes a second one per mouse event (`runtime.rs:628`).

**Budget:** with an idle UI and the mouse moving continuously over the window, davinci uses less than 5% of one core (measure with the OS process monitor for 10 s before and after; record both numbers in the PR).

**Files:**
- Modify: `crates/davinci-tui/src/davinci/runtime.rs` (mouse enable, `poll_event`)
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs` (the loops' draw calls)

- [ ] **Step 1: Enable only click and drag reporting**

Replace `event::EnableMouseCapture` with explicit modes that exclude any-motion (1003):

```rust
/// Clicks (1000), drags (1002) and SGR coordinates (1006). Not 1003:
/// reporting every motion event makes the UI redraw while the mouse just
/// crosses the window.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";
```

Write them with `io::stdout().write_all(..)`. On Windows, crossterm's `EnableMouseCapture` uses the console API, not escape codes; keep crossterm's call on Windows and, in `poll_event`, drop `MouseEventKind::Moved` events there.

- [ ] **Step 2: Dirty flag**

Add `dirty: bool` to the davinci `Model` (or the loop state). Set it on every input event other than `Moved`, on resize, on agent events, and when the tick advances an animation that is visible (spinner, caret blink). Each loop draws only when `dirty`, then clears it.

- [ ] **Step 3: Test**

```rust
    #[test]
    fn mouse_motion_does_not_mark_the_frame_dirty() {
        let mut model = Model::default();
        model.dirty = false;
        apply_event(&mut model, Event::Mouse(mouse(MouseEventKind::Moved, 3, 3)));
        assert!(!model.dirty);
        apply_event(&mut model, Event::Key(key('a')));
        assert!(model.dirty);
    }
```

(`apply_event` is the model update function the loops call; use its real name.)

- [ ] **Step 4: Run, measure, commit**

Run: `cargo test -p davinci-tui` → PASS; measure the budget.

```bash
git add crates/davinci-tui crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "perf(tui): ignore mouse motion and redraw only dirty frames"
```

---

### Task 7.5: `trim_transcript` never erases the whole transcript

**Finding fixed:** TUI 6 (checked in source). `trim_transcript` (`model.rs:2592-2602`) scans forward from `len - 4000` for a `Gap`; if none of the last 4000 entries is a `Gap`, `cut` reaches `len` and `drain(..len)` removes everything, including the final answer. `start_tool` (`davinci_interactive.rs:600-609`) only inserts a `Gap` after a Δ block, so one long autonomous turn with more than 4000 tool entries triggers it.

**Files:**
- Modify: `crates/davinci-tui/src/davinci/model.rs:2592-2602`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn trimming_without_a_gap_keeps_the_newest_entries() {
        let mut model = Model::default();
        for i in 0..5000 {
            model.transcript.push(Entry::note(format!("tool {i}")));
        }
        model.trim_transcript();
        assert_eq!(model.transcript.len(), 4000);
        assert!(format!("{:?}", model.transcript.last().unwrap()).contains("tool 4999"));
    }
```

(Use whichever non-`Gap` `Entry` constructor exists.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib trimming_without_a_gap`
Expected: FAIL (length 0).

- [ ] **Step 3: Implement**

```rust
    pub fn trim_transcript(&mut self) {
        const TRANSCRIPT_CAP: usize = 4000;
        let len = self.transcript.len();
        if len <= TRANSCRIPT_CAP {
            return;
        }
        let minimum = len - TRANSCRIPT_CAP;
        // Prefer a block boundary; without one in range, cut at the cap so
        // the newest entries (and the final answer) always survive.
        let cut = (minimum..len)
            .find(|&index| matches!(self.transcript[index], Entry::Gap))
            .unwrap_or(minimum);
        self.transcript.drain(..cut);
    }
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-tui --lib model` → PASS.

```bash
git add crates/davinci-tui/src/davinci/model.rs
git commit -m "fix(tui): trimming the transcript never drops everything"
```

---

### Task 7.6: Up and Down in the composer use the real width

**Finding fixed:** TUI 5. `last_width` starts at 80 (`editor.rs:86`) and is only set inside `Component::render` (`editor.rs:1073`), which davinci never calls; `terminal_rows` stays 24. With a 200-column single-line prompt, Up jumps 80 columns left inside the same row instead of recalling history.

**Files:**
- Modify: `crates/davinci-tui/src/editor.rs:63-98`
- Modify: the davinci composer render (`rg -n "fn .*composer" crates/davinci-tui/src/davinci/views`)

**Interfaces:**
- Produces: `Editor::set_layout_width(&self, width: usize)` (sets `last_width`)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn up_on_a_single_wide_line_recalls_history() {
        let mut editor = Editor::default();
        editor.push_history("previous prompt");
        editor.set_layout_width(usize::MAX); // davinci's composer scrolls, never wraps
        editor.insert_str(&"x".repeat(200));
        editor.move_up();
        assert_eq!(editor.text(), "previous prompt");
    }
```

(Use the real history and insert method names; the test's point is the width.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib up_on_a_single_wide_line`
Expected: FAIL to compile (`set_layout_width`), then FAIL on the assertion if the method is added without calling it.

- [ ] **Step 3: Implement**

```rust
    /// The width the visual line map uses for Up/Down and paging. Hosts that
    /// render the editor themselves (davinci) must set it every frame.
    pub fn set_layout_width(&self, width: usize) {
        self.last_width.set(width.max(1));
    }
```

In the davinci composer view, call `editor.set_layout_width(usize::MAX)` (it scrolls horizontally, never wraps) and `editor.set_terminal_rows(area.height as usize)` each frame. If the composer does wrap multi-line prompts, pass the inner width instead.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-tui` → PASS.

```bash
git add crates/davinci-tui
git commit -m "fix(editor): davinci composer sets the real layout width and height"
```

---

### Task 7.7: Left and Right move by grapheme, like Backspace and Delete

**Finding fixed:** TUI 8. `move_left`/`move_right` (`editor.rs:390-422`) step one `char`; Backspace/Delete step one grapheme (`editor.rs:349/368`). Left across a skin-tone emoji or `e` + combining accent lands inside the cluster; `split_at_caret` (`chrome.rs:711-723`) then draws the pieces apart and typing splits the cluster.

**Files:**
- Modify: `crates/davinci-tui/src/editor.rs:390-422`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn arrows_step_over_whole_graphemes() {
        let mut editor = Editor::default();
        editor.insert_str("a👍🏽e\u{301}");
        editor.move_left();
        assert_eq!(editor.cursor(), "a👍🏽".len());
        editor.move_left();
        assert_eq!(editor.cursor(), "a".len());
        editor.move_right();
        assert_eq!(editor.cursor(), "a👍🏽".len());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib arrows_step_over_whole_graphemes`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `move_left`, replace the `chars().next_back()` step with `self.cursor -= prev_grapheme_len(&self.buffer[..self.cursor]);`; in `move_right`, `self.cursor += next_grapheme_len(&self.buffer[self.cursor..]);` (both helpers exist at `editor.rs:1002-1010`).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-tui --lib editor` → PASS.

```bash
git add crates/davinci-tui/src/editor.rs
git commit -m "fix(editor): arrow keys move by grapheme cluster"
```

---

### Task 7.8: Width arithmetic in `truncate_run` cannot overflow

**Finding fixed:** TUI 9. `ui.rs:127-128` casts `UnicodeWidthStr::width(..) as u16` and adds in `u16`; a span wider than 65535 cells panics in debug and clips wrongly in release. `ui.rs:533` (`as u16 + 6`) has the same pattern.

**Files:**
- Modify: `crates/davinci-tui/src/davinci/ui.rs:120-135, 533`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_huge_span_is_clipped_not_overflowed() {
        let spans = vec![Span::raw("x".repeat(70_000)), Span::raw("tail")];
        let out = truncate_run(spans, 80);
        assert!(run_width(&out) <= 80);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-tui --lib a_huge_span_is_clipped`
Expected: FAIL (debug overflow panic).

- [ ] **Step 3: Implement**

Compute in `usize` and convert once at the edge:

```rust
    let width = usize::from(width);
    let mut used = 0usize;
    for span in spans {
        let span_width = UnicodeWidthStr::width(span.content.as_ref());
        if used + span_width <= width {
            used += span_width;
            out.push(span);
            continue;
        }
        let room = width.saturating_sub(used);
        if room > 0 {
            let clipped = clip_ellipsis(span.content.as_ref(), u16::try_from(room).unwrap_or(u16::MAX));
            // ...
```

At `ui.rs:533` use `u16::try_from(len).unwrap_or(u16::MAX).saturating_add(6)`. Check `run_width` for the same `as u16` sum.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-tui` → PASS.

```bash
git add crates/davinci-tui/src/davinci/ui.rs
git commit -m "fix(tui): do width sums in usize"
```

---

### Task 7.9: The idle loop and the scope-expansion modal share the per-iteration housekeeping

**Findings fixed:** TUI 11, 12, 14.
- The scope-expansion modal loop (`davinci_interactive.rs:3055-3087`) never calls `voice.tick`, drops `Paste` events, never increments `model.tick`, and never reacquires the terminal (reacquire is fixed by 7.2).
- Several `continue`s in the idle loop (`:4382/4414/4419/4448/4476/4522/4526/4574`) skip the tick block at `:4732-4738`, so auto-repeat keys consumed by voice, shortcuts or Ctrl+V stall `model.tick` and `poll_jobs`.
- `set_hosted_tui_active(true)` (`:4207`) is never reset on the early `?` returns at `:4222` and `:4252` (for example a bad `--file`), so later `crate::println!` output goes to a transcript that will never be drawn.

**Files:**
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`

**Interfaces:**
- Produces: `fn housekeeping(shell: &mut Shell)` (tick, voice tick, job poll; private) and `struct HostedTuiGuard` (resets the flag on drop)

- [ ] **Step 1: Extract and call housekeeping first**

Move the tick block (`:4732-4738`) plus `voice.tick(...)` into `fn housekeeping(...)`, and call it at the **top** of each loop iteration (idle, turn, scope modal) instead of at the bottom. The `continue`s then cannot skip it. Handle `Event::Paste` in the modal the same way the idle loop does (append to the composer if the modal has one; otherwise ignore explicitly, with a comment).

- [ ] **Step 2: Guard the hosted flag**

```rust
struct HostedTuiGuard;
impl HostedTuiGuard {
    fn activate() -> Self {
        crate::output::set_hosted_tui_active(true);
        Self
    }
}
impl Drop for HostedTuiGuard {
    fn drop(&mut self) {
        crate::output::set_hosted_tui_active(false);
    }
}
```

Replace the `set_hosted_tui_active(true)` at `:4207` with `let _hosted = HostedTuiGuard::activate();` and delete the matching manual reset at the end of `run` (the guard does it on every exit path).

- [ ] **Step 3: Test**

```rust
    #[test]
    fn hosted_flag_is_reset_when_run_fails_early() {
        let parsed = Args { file: vec!["/definitely/not/a/file".into()], ..Args::default() };
        let _ = davinci_interactive::run(&parsed /* fixture agent */);
        assert!(!crate::output::hosted_tui_active());
    }
```

(Use the real `--file` field and a fixture that fails before a terminal is opened; if `run` always opens the terminal first, unit-test `HostedTuiGuard` directly instead.)

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "fix(tui): housekeeping runs every iteration of every loop; hosted flag reset on early exits"
```

The full single-event-pump refactor (three loops into one) is Task 11.6.

---

### Task 7.10: Verify, then fix, fast typing misread as a paste on Windows

**Finding (SUSPECTED):** TUI 10. `paste_burst.rs:39-47` classifies a batch of 8+ keys, or any Enter, arriving with the same timestamp as a paste. If the UI thread stalls, typed keys queue up and arrive together; `ok⏎` typed during a stall becomes a newline instead of a submit.

- [ ] **Step 1: Reproduce before changing code**

On Windows, add a temporary 300 ms sleep in the idle loop (behind an env var used only for this check), type `ok` and Enter quickly, and observe. Record the result in the PR. If it does not reproduce, add the test below as a guard and stop.

- [ ] **Step 2: Test and fix (if reproduced)**

```rust
    #[test]
    fn a_trailing_enter_after_a_short_typed_burst_submits() {
        let mut filter = PasteFilter::default();
        let now = std::time::Instant::now();
        let keys = [key('o'), key('k'), enter()];
        let events = filter.feed_batch(&keys, now);
        assert!(matches!(events.last(), Some(Event::Key(k)) if k.code == KeyCode::Enter));
    }
```

Fix: never convert a batch's **trailing** Enter into a newline when the batch has fewer than 8 printable keys before it; the paste heuristic still applies to bursts that contain Enter in the middle or are long.

- [ ] **Step 3: Commit**

```bash
git add crates/davinci-tui/src/davinci/paste_burst.rs
git commit -m "fix(tui): a short typed burst ending in Enter submits instead of pasting"
```

---

### Task 7.11: Overlays and sheets render without cloning the whole `Model`

**Finding (performance):** TUI 4, clone half. `app.rs:151-154` clones the whole `Model` whenever an overlay is open, `app.rs:620-625` clones it again in `overlay_body`, `app.rs:122-126` and `:274-279` clone for Models/Settings screens and command sheets, and `cogitator.rs:211-218` for the model picker. Each clone copies up to 4000 transcript entries, the workspace file list, the corpus, sessions and the editor with its undo history, 25 times a second while a permission prompt is open.

**Budget:** with 4000 transcript entries and a permission prompt open, one `compose_frame` allocates less than 1 MB (measure with a counting allocator in a `#[ignore]` test, or time 100 frames: under 50 ms total in release).

**Files:**
- Modify: `crates/davinci-tui/src/davinci/app.rs:122-126, 151-154, 274-279, 620-625`, `cogitator.rs:211-218`

- [ ] **Step 1: Measure**

```rust
    #[test]
    #[ignore = "performance budget; run with --release -- --ignored"]
    fn composing_with_an_overlay_is_cheap() {
        let mut model = model_with_transcript(4000);
        model.overlay = Some(sample_permission_overlay());
        let started = std::time::Instant::now();
        for _ in 0..100 {
            let _ = compose_frame(&model, 50);
        }
        let elapsed = started.elapsed();
        eprintln!("100 frames: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(50));
    }
```

Run it now in release and record the number.

- [ ] **Step 2: Replace each clone with a render context**

Each clone exists to render with a changed theme (dimmed) or a changed screen. Introduce

```rust
/// What a view needs beyond the model: the theme to paint with and the
/// screen to show, so overlays can dim the background without a copy.
pub struct RenderContext<'a> {
    pub theme: std::borrow::Cow<'a, Theme>,
    pub screen: Screen,
}
```

and change the affected view functions from `(&Model)` to `(&Model, &RenderContext)`. Where a clone was used to change `model.screen`, pass the screen in the context instead. `Theme` is small; `Cow::Owned(dimmed)` for the background is fine.

- [ ] **Step 3: Run the budget and the suite, commit**

Run: Step 1 command → PASS; `cargo test -p davinci-tui` → PASS.

```bash
git add crates/davinci-tui
git commit -m "perf(tui): render overlays and sheets through a context instead of cloning the model"
```

---

### Task 7.12: The legacy TUI gives the terminal back on early errors

**Finding fixed (legacy path only):** TUI 15. `main.rs:4761-4777` returns with `prepare_initial_message(...)?` after `tui.start()` without stopping the TUI; the legacy stack installs no panic hook.

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs:4761-4777`

- [ ] **Step 1: Implement**

Call `prepare_initial_message` **before** `tui.start()` if it does not need the TUI. If it does, wrap the started TUI in a guard whose `Drop` calls `tui.stop()`, and call `davinci_tui::davinci::runtime::install_panic_hook()` (or a legacy equivalent that calls the legacy restore) right after `start()`.

- [ ] **Step 2: Test and commit**

Manual: `davinci --legacy-tui --file /does/not/exist`; the shell must be usable afterwards (echo on, primary screen). Record in the PR. If Task 11.3 retires the legacy TUI, skip this task.

```bash
git add crates/davinci-coding-agent/src/main.rs
git commit -m "fix(legacy-tui): stop the TUI on early errors"
```

---

### Task 7.13: Voice keeps speech whisper would keep, and decodes invalid UTF-8 lossily

**Findings fixed:** TUI 17, now CONFIRMED against the vendored source. `voice_shim.cpp:~54` skips any segment with `no_speech_prob > no_speech_thold`. whisper.cpp already applies the combined rule inside `whisper_full` (`native/upstream/src/whisper.cpp:7622-7623`: `no_speech_prob > no_speech_thold && avg_logprobs < logprob_thold`). The shim's extra, weaker test drops real speech: whisper.cpp gives every segment in a 30 s window that window's `no_speech_prob`. Skipping a segment can also cut a multi-byte UTF-8 character across segments; `engine.rs:~99` then fails `from_utf8` and returns `InferenceFailed`.

**Files:**
- Modify: `crates/davinci-voice/native/voice_shim.cpp:~54`
- Modify: `crates/davinci-voice/src/engine.rs:~99`

- [ ] **Step 1: Write the failing test (Rust side)**

```rust
    #[test]
    fn invalid_utf8_from_the_decoder_is_decoded_lossily() {
        let bytes = b"caf\xc3 ok\0".to_vec(); // truncated 'é'
        assert_eq!(text_from_decoder(&bytes).unwrap(), "caf\u{fffd} ok");
    }
```

Extract `text_from_decoder(buffer: &[u8]) -> Result<String, VoiceError>` from the block at `engine.rs:~96-101` so it is testable without a model.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-voice --lib invalid_utf8_from_the_decoder`
Expected: FAIL to compile, then FAIL with `InferenceFailed` once extracted.

- [ ] **Step 3: Implement**

`voice_shim.cpp`: delete the line
`if (whisper_full_get_segment_no_speech_prob_from_state(state.get(), i) > params.no_speech_thold) continue;`
and add a comment: `// whisper_full already drops no-speech windows (no_speech_prob > thold AND avg_logprob < logprob_thold); filtering again on no_speech_prob alone drops real speech.`

`engine.rs`:

```rust
fn text_from_decoder(buffer: &[u8]) -> Result<String, VoiceError> {
    let end = buffer.iter().position(|b| *b == 0).ok_or(VoiceError::InferenceFailed)?;
    let raw = String::from_utf8_lossy(&buffer[..end]);
    normalize::normalize(&raw).map_err(|_| VoiceError::TextTooLong)
}
```

- [ ] **Step 4: Run and verify by hand**

Run: `cargo test -p davinci-voice` → PASS. Manual: dictate a 20-second sentence with a pause in the middle; the whole sentence is transcribed. Record in the PR.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-voice
git commit -m "fix(voice): rely on whisper's own no-speech rule and decode text lossily"
```

---

### Task 7.14: The voice worker exits when idle after a cancel, and does not wake every 10 ms

**Findings fixed:** TUI 16 and 19. After a `Cancel`, `cancel` stays `true` until the next `Prepare`, and the `continue` (`davinci-voice-worker.rs:~110`) skips the `warm_until` idle-exit check; if the host's `Shutdown` is lost, the worker keeps the model loaded until the parent exits. The owner loop wakes every 10 ms for the whole 300 s warm period (`:51`); the host supervisor polls `try_wait` every 10 ms (`voice_input.rs:112-128`); the shutdown watcher every 20 ms forever (`shutdown.rs:36-48`).

**Files:**
- Modify: `crates/davinci-voice/src/bin/davinci-voice-worker.rs:50-160`
- Modify: `crates/davinci-coding-agent/src/voice_input.rs:112-128`, `shutdown.rs:36-48`

- [ ] **Step 1: Implement the worker changes**

- After emitting `Cancelled`, reset: `cancel.store(false, Ordering::Release);` and fall through to the idle check instead of `continue`.
- Receive with a timeout that depends on state: while `capture.is_some()`, keep 10 ms (audio must be drained); otherwise `warm_until.saturating_duration_since(Instant::now())` (wake once when the warm period ends).

- [ ] **Step 2: Host side**

Replace the 10 ms `try_wait` poll with a thread that calls `child.wait()` and sends the exit status on a channel; replace the 20 ms shutdown poll with a `Condvar` or channel that the shutdown path signals. If `shutdown.rs` watches a signal flag set by a signal handler (which cannot use a channel), use `signal-hook`'s `iterator` (already a dependency) or keep polling at 250 ms, which is enough for a human.

- [ ] **Step 3: Test**

```rust
    #[test]
    fn worker_exits_after_the_warm_period_following_a_cancel() {
        let mut worker = spawn_test_worker_with_warm(std::time::Duration::from_millis(200));
        worker.send(Command::Prepare { /* fixture */ });
        worker.send(Command::Cancel { /* same session */ });
        assert!(worker.wait_exit(std::time::Duration::from_secs(3)).is_some());
    }
```

Make `warm` configurable for tests (an env var read only in test builds, or a constructor parameter if the loop is extracted into a function).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-voice -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-voice crates/davinci-coding-agent
git commit -m "fix(voice): idle exit after cancel; sleep until needed instead of polling"
```

---

### Task 7.15: Loading a voice model does not hold two copies in memory

**Finding:** TUI 18. `catalog.rs:53-65` reads the verified model file (488 MB for "small") into a `Vec`, then `engine.rs:41-49` hands it to `whisper_init_from_buffer*`, which copies it into its own tensors: about twice the model size at peak.

**Behavior:** compute the SHA-256 by streaming the file in 1 MiB chunks (no full copy in memory), record its length and modification time, and load it with whisper's from-file initializer. Immediately before loading, re-read length and mtime; if either changed, refuse and re-verify. Models live in the agent directory, which only the user can write, so this narrow check-then-load window is acceptable; state that in a code comment.

**Files:**
- Modify: `crates/davinci-voice/src/catalog.rs:53-65`, `src/engine.rs:41-49`, `native/voice_shim.cpp` (add a from-file init wrapper)

- [ ] **Step 1: Measure**

Record peak RSS of the worker while loading the "small" model today (Windows: Task Manager peak working set; Linux: `/usr/bin/time -v`). Budget: peak under 1.3 × model size.

- [ ] **Step 2: Implement**

Hash with a 1 MiB buffered read loop (`sha2::Sha256::update` per chunk), compare, keep the `File` metadata (len, modified). Before init, `fs::metadata(path)` must match those; then call a new shim function `voice_init_from_file(const char* path, ...)` that wraps `whisper_init_from_file_with_params_no_state`. Keep `init_from_buffer` for tests that embed tiny models.

- [ ] **Step 3: Measure again, test, commit**

Run: `cargo test -p davinci-voice` → PASS; peak RSS within budget (record both numbers).

```bash
git add crates/davinci-voice
git commit -m "perf(voice): verify by streaming and load the model from file"
```

---

## Phase 7 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-tui -p davinci-voice -p davinci-coding-agent
```

Then `00-index.md` → "Manual verification: terminal".
