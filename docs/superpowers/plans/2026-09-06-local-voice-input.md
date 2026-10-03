# Fully local voice input for the Da Vinci terminal

Engineering design and implementation plan — 2026-09-06

**Repository:** `C:\Users\sergi\Desktop\pi-rust`  
**Status:** Investigation and design only. No feature implementation, dependency installation, model download, microphone access, build, or test execution was performed for this plan.  
**Implementation target:** The default Rust Da Vinci interactive TUI, including dictation while an agent turn is running.  
**Non-negotiable invariant:** Speech recognition produces an editor insertion, never a message submission, queued prompt, command execution, or provider request.

## 1. Recommended solution

Add an optional, bundled Rust companion process named `davinci-voice-worker`. It owns microphone capture, in-memory audio, and local inference. The terminal owns a small voice controller and inserts a completed transcription into the existing `Composer` through its existing `Editor::insert_str` path.

Use **CPAL for capture, Rubato for sample-rate conversion, and whisper.cpp for CPU inference**, with the **multilingual Whisper `base` model** as the default. Use a small, project-owned C ABI shim around a pinned whisper.cpp source tree rather than introducing Python, an HTTP speech service, a system executable discovered through `PATH`, or a second chat editor. The native engine and official Whisper weights are MIT-licensed; CPAL is Apache-2.0. Distribution must also include notices for the pinned transitive dependencies. [S1], [S2], [S3], [S4]

The default interaction is **toggle-to-record**: Ctrl+T or a left click on the mic starts recording; the next activation stops recording and starts transcription. Escape cancels. The result appears once, at the current editor caret, as ordinary text with one undo step. The user must subsequently issue the existing send action. There is no spoken-command interpretation, automatic send, automatic queueing, or live partial-text replacement.

| Decision | Selected behavior |
|---|---|
| Scope | Default local interactive Da Vinci TUI, idle and during agent work |
| Speech engine | Pinned whisper.cpp source, isolated in a companion process |
| Default model | `base`, multilingual, unquantized GGML model file `ggml-base.bin` |
| Lightweight option | `tiny`; less resource demand, quality must be evaluated on the user's language |
| Higher-accuracy option | `small`; explicit opt-in to larger download and slower CPU processing |
| Compute | CPU by default; at most four inference threads, leaving a core available where possible |
| Capture | Explicit activation only; never keep an idle microphone stream open |
| End of recording | Explicit toggle; 120-second safety cap stops and transcribes, but does not send |
| Text delivery | Single insertion through the existing editor; no paste-marker compression |
| Mic placement | Right end of the upper composer rule, outside editable text |
| Model acquisition | Explicit download consent or verified local import; offline runtime afterward |
| Native safety | Separate process, bounded protocol, cooperative cancellation followed by forced termination if necessary |

**Why this architecture:** It preserves the repository's synchronous terminal loop and existing editor while containing native inference failures. It avoids a Python environment and keeps recorded audio out of files and IPC. A second executable is a deliberate packaging cost in exchange for bounded cancellation, independent model lifetime, and protection of the terminal from native crashes.

**Scope exclusions:** Do not add dictation to `--print`, RPC/server/client modes, fixture screens, or the legacy TUI in this initial implementation. Do not forward microphone audio over SSH or a remote-client connection. Leave those modes working as before and state their voice support accurately. Do not redesign the transcript, agent runtime, extension system, general mouse selection, or editor.

## 2. Repository findings and current input architecture

### 2.1 Inspected implementation, not assumed framework

The checkout is a Rust Cargo workspace. The initial inspected HEAD was `2edc963` on `main`; there were already many uncommitted changes. Some source hashes and line counts changed during inspection. The findings below describe the inspected working tree, not an immutable clean revision. Re-open these symbols before implementation and preserve unrelated work.

Repository references use paths relative to the root. Line numbers are navigation hints from the inspection; symbol names are authoritative anchors.

| Area | Verified finding | Evidence in repository |
|---|---|---|
| Language and toolchain | Rust, edition 2021; pinned Rust 1.83.0; workspace MIT license | `Cargo.toml`, `rust-toolchain.toml` |
| Packages | Active code is in `crates/*`; the executable package is `davinci-coding-agent`, with binary `davinci` | Workspace and package manifests |
| TUI | Ratatui `=0.29.0` with Crossterm `=0.28.1` | `crates/davinci-tui/Cargo.toml`, workspace dependencies |
| Default interface | `davinci_interactive::run`; legacy selected separately | `crates/davinci-coding-agent/src/main.rs::run`, `args.rs` |
| Editable input | `davinci::model::Composer` wraps the shared `Editor`; cursor is a UTF-8 byte offset | `davinci-tui/src/davinci/model.rs:26–119`, `editor.rs:47–109` |
| Input routing | Canonical key encoding and configurable actions, with modal/completion/surface/composer ownership | `interaction.rs`, `keybindings.rs`, `davinci/app.rs::handle_key` |
| Rendering | Flat vectors of Ratatui lines, ultimately rendered as a paragraph; not a tree of independent button widgets | `davinci/app.rs::compose`, `davinci/runtime.rs::Session::draw` |
| Concurrency | Synchronous UI loops; standard threads, `mpsc`, mutexes and atomic cancellation; a scoped provider worker | `davinci-coding-agent/src/davinci_interactive.rs::run_turn` |
| Mouse | Default Da Vinci runtime does not enable mouse capture and ignores mouse events | `davinci/runtime.rs::Session`, live event-loop matches |
| Platform work | Crossterm lifecycle, Windows ConPTY paste filtering, Unix keyboard-protocol negotiation | `davinci/runtime.rs`, `davinci/term.rs` |
| Dependency convention | Exact versions; existing JSON, hashing, HTTP and path utilities can be reused | Workspace `Cargo.toml` |
| Tests | Inline Rust test modules, deterministic fixtures, offline CI | `CLAUDE.md`, source test modules, `.github/workflows/ci.yml` |
| CI coverage | Current quality workflow runs on Ubuntu, not a verified three-platform audio matrix | `.github/workflows/ci.yml` |

The root `package.json` and older TypeScript material are not grounds for implementing this feature in JavaScript. Keep the new runtime feature in Rust. Do not alter the read-only upstream reference under `vendor/davinci`.

### 2.2 Existing keyboard-to-send path

```text
Terminal key / bracketed paste
    |
    v
Crossterm -> Session::poll_event -> existing paste filtering
    |
    +-- idle: extension routing -> app::handle_key
    |              -> ownership / completions / shared editor actions
    |
    +-- running turn: permission handler or mid_turn_key
                   -> existing composer edits
    |
    v
Model.composer -> shared Editor { buffer, byte cursor, undo, history }
    |
    | EXPLICIT USER SEND ONLY
    +-- idle: Model::submit -> Flow::Submit(text)
    |                        -> on_line -> submit_prompt
    |
    +-- busy: Model::queue -> run_turns later -> on_line
                                   |
                                   v
                    extension input / command classification
                    skill and template expansion
                    Agent::prompt_with -> provider worker
```

Relevant anchors:

- `davinci/app.rs::handle_key`, around lines 376–459, applies editor actions and calls `Model::submit` for the send binding. Completion handling can also consume Enter, including a special `/model` submission path.
- `davinci/model.rs::submit`, around 2086, calls `Editor::submit`, updates transcript entries, and marks the model running. `queue`, around 2105, also consumes the editor and records text for later execution.
- `davinci_interactive.rs::mid_turn_key`, around 1086, handles input during generation separately. Ordinary Enter invokes `model.queue()`.
- `davinci_interactive.rs::run_turns`, around 1134, automatically routes already queued lines through `on_line` after a turn.
- `on_line`, around 5978, handles shell commands, extension commands and normal prompts. `submit_prompt`, around 6079, calls extension input handlers, expands text, and ultimately invokes `agent.prompt_with`.

**Integration consequence:** Handling only `app::handle_key` is insufficient. Both the idle loop and the running-turn loop must process voice events and apply the same send/cancellation protections. Dictation completion must not call any function in the send half of the diagram.

### 2.3 Existing editor functionality to reuse

`Editor::insert_str`, around `editor.rs:319–325`, already exits history browsing, clears preferred-column state, pushes one undo snapshot, inserts at the current byte cursor, and clears the previous action. `Composer::push_str` delegates to it. Use that functionality directly; do not invent a second editor or replace the entire buffer.

Do **not** use `Editor::handle_paste` for dictated text: it can collapse more than ten lines or 1,000 characters into a paste marker. Dictation must remain directly visible and editable. Do not route a completion through `Model::type_char` without a focus guard, because that method can target the command-palette query rather than the composer. [Repository: `editor.rs::handle_paste`, `model.rs::type_char`.]

The existing undo binding is **Ctrl+-**, not Ctrl+Z. The latter is associated with terminal suspension in the shared keymap. Do not promise a conventional desktop shortcut that this editor does not use.

### 2.4 Existing collisions and layout constraints

Ctrl+T is already assigned to `davinci.tools.expand`; `mid_turn_key` also hardcodes it. The shared keymap separately assigns Ctrl+T to legacy thinking visibility and a tree filter. These are different input contexts, not three features to remove. [Repository: `keybindings.rs::default_pairs`, `davinci_interactive.rs::mid_turn_key`.]

The composer follows a short conversation and moves toward the bottom as content grows. Suggestions, extension rows, working status and notices affect its position. Therefore a mic hitbox cannot be calculated as “terminal bottom-right.” The current design document explicitly describes a keyboard-only interface; introducing one pointer control is a narrow, documented exception. [Repository: `davinci/app.rs::compose`, `davinci/views/chrome.rs::composer`, `docs/ui/design.md`.]

## 3. User experience and mic placement

### 3.1 Place the mic on the upper composer rule

Put the control at the right end of the upper horizontal rule surrounding the composer. This satisfies the desired right-side placement without taking width from editable text or interfering with the caret, multiline text, or existing horizontal scrolling.

```text
Assistant response ends here.

------------------------------------------------ [mic Ctrl+T]
  > Explain the retry logic in this function |
--------------------------------------------------------------
  Enter send / Shift+Enter newline

Listening:
-------------------------------------- [REC 00:12 / ^T stop]
  > Explain the retry logic in this function |
--------------------------------------------------------------
  Listening locally / Esc cancel / typing still works

After insertion:
------------------------------------------------ [mic Ctrl+T]
  > Explain the retry logic in this function and add tests.|
--------------------------------------------------------------
  Inserted / review, then Enter to send / Ctrl+- undo
```

Use a microphone glyph with the visible word `mic` where the terminal/font renders it reliably. The ASCII `[mic Ctrl+T]` form is a supported fallback, not a degraded feature. Do not require Nerd Fonts or an emoji font. Measure display cells through the existing Unicode-width utilities; do not use byte length for alignment.

Reuse `Theme` tokens. The recording state has a static marker plus `REC` and elapsed time, so color is not the only signal. Use the existing animation policy for a processing spinner; no continuous waveform, sound effect, flashing whole panel, or additional progress dashboard.

Preserve any existing hidden-line count on the left of the rule. Abbreviate labels before reducing typing space. Suggested label budgets are full `mic Ctrl+T` at ordinary widths, `mic ^T` on narrower screens, and `[mic]` where necessary. An active recording must always retain a visible `REC` or cancellation status; if resizing makes the composer/status impossible to display, cancel capture rather than recording invisibly.

### 3.2 Discoverability and focus

The mic is available only when the normal agent composer is the input destination: `Screen::Agent`, no overlay, no Codex split. An autocomplete list above that composer does not make voice unavailable. Start activation dismisses the list; subsequent normal edits may recompute it.

A click does not move the text caret or create a separate keyboard focus destination. Keyboard users retain Ctrl+T, a command-palette voice action, and `/hotkeys` documentation. The palette action closes the palette and then dispatches the same voice action; it must not rewrite or submit the draft. Add setup/status/device actions to the existing command surface without creating another chat input.

During transcription the editor remains usable. The result goes at the caret **where it is when the result is accepted**, not at the cursor position from the beginning of recording. State this once in voice help: “Dictation is inserted at your current cursor. It never sends.”

### 3.3 Mouse behavior and terminal selection

Enable Crossterm mouse reporting for the eligible voice-enabled composer, and disable it when voice mouse support is off or terminal ownership is released. Left-button down inside the current mic rectangle dispatches `VoiceAction::Toggle`; ignore release, drag, wheel, right-click and clicks outside the control. Keyboard and mouse have no separate recording implementations.

Mouse reporting can affect a terminal's native selection behavior. Explain that terminal-specific modifier selection, often Shift+drag, may be necessary; do not promise it works identically everywhere. Provide the advanced user setting `voice.mouse=false` to keep keyboard dictation and native terminal mouse behavior. Do not implement a general mouse-selection subsystem as part of this work.

Invalidate cached hitboxes after resize, screen change or layout invalidation. Render a fresh layout before accepting a click against changed geometry. On terminals without usable mouse reporting, the keyboard path remains complete.

## 4. Voice interaction and state machine

### 4.1 Authoritative state and actions

Create one pure reducer for voice state, driven by a monotonic clock and typed events. The host controller executes its effects; the TUI stores only its lightweight view projection. Do not store duplicate authoritative state in a button, worker thread and `Model`.

Public user actions are `Toggle`, `Cancel`, `OpenSetup` and `Disable`. Internal events include model readiness, capture-started/stopped acknowledgements, a final transcription, errors, timeout and context invalidation. Every session has a strictly increasing `session_id` and a captured `composer_epoch`.

| State | Visible presentation | Toggle | Other transitions |
|---|---|---|---|
| Disabled | No active mic; voice remains discoverable in settings | Explain disabled state; do not capture | Enable -> Idle or NeedsSetup |
| NeedsSetup | `mic setup` | Open download/import setup | Successful setup -> Idle; never auto-record after installation |
| Idle | `mic Ctrl+T` | Validate context, create ID, prepare worker/model | Missing helper/model -> actionable setup error |
| Preparing | `Preparing voice... Esc cancel` | Ignore; no second session | Ready + still-valid activation -> start capture; failure -> Error |
| Listening | `REC 00:12 / Ctrl+T stop` | Request stop -> Stopping | Escape -> Cancelling; 120 seconds -> Stopping |
| Stopping | `Stopping microphone...` | Ignore | CaptureStopped -> Transcribing; failure -> Error |
| Transcribing | `Transcribing locally... Esc cancel` | Ignore with current status | Valid final result -> Inserted; empty -> NoSpeech notice |
| Cancelling | `Cancelling voice...` | Ignore until resources stop | Capture closed / worker reaped -> Idle; late result discarded |
| Inserted | `Inserted / review before sending` for two seconds | Begin a new session when the activation debounce permits | Normal typing continues; notice expires to Idle |
| Error | Short typed reason and retry/setup guidance | Retry only the recoverable operation | No automatic capture retry or model download |

`MicrophoneUnavailable`, `PermissionDenied`, `UnsupportedBackend`, `ModelMissing`, `ModelCorrupt`, `DeviceDisconnected`, `NoSpeech`, `CaptureOverflow`, `InferenceFailed`, `WorkerCrashed`, and `ProtocolMismatch` are typed outcomes, not one ambiguous “voice failed” string.

A cancellation is not visibly complete until capture closure is acknowledged or the worker has exited. Immediately invalidate its result eligibility, but keep “Cancelling” visible while microphone shutdown is pending.

### 4.2 Precise keyboard semantics

Use toggle, not press-and-hold: the repository cannot rely on matching key-release events across its supported terminals. Ignore reported repeat/release events for the voice action and debounce repeated activations for 250 ms. Legacy terminals can encode held-key repetition as ordinary presses, so do not advertise hold-to-talk behavior.

- Ctrl+T in Idle/Inserted starts; Ctrl+T in Listening stops and transcribes. It never sends.
- Ctrl+T while loading, stopping, transcribing or cancelling does not enqueue another session.
- Escape while voice is active cancels voice and is consumed before completion dismissal, double-Escape navigation or the busy-loop agent-abort handler.
- Ctrl+C cancels voice **and** preserves the existing agent-interrupt behavior. It is not redefined as voice-only.
- Normal typing, deletion, cursor movement and existing newline bindings remain available. Voice does not clear the draft.
- A quit, suspend/terminal handoff, session switch, destructive draft replacement, or a new permission/modal surface cancels voice and invalidates its result.
- Silence alone does not stop a normal recording. This prevents a pause while thinking about code from ending dictation unexpectedly.

### 4.3 Send protection, including the busy queue

While Preparing, Listening, Stopping, Transcribing or Cancelling, consume every configured composer send/follow-up/queue action before it reaches extensions, autocomplete submission, `Model::submit`, or `Model::queue`. Show “Finish or cancel voice before sending.” Do not remember that key as a deferred submission. Existing newline actions still insert newlines.

When a final result arrives, process already pending terminal input under the active-state gate before applying the result, subject to a bounded per-loop event budget. After insertion, keep a submit inhibition until a frame containing the insertion has been drawn and at least 250 ms has passed. Consume submit repeats during this guard; do not replay them. This reduces the completion/queued-Enter race without claiming a terminal can perfectly reconstruct physical key timing. A subsequent deliberate send uses the unchanged send path.

No normal voice event is allowed to carry `Flow::Submit`, invoke `on_line`, call an extension's input handler, push a history entry, append a user transcript block, call `agent.prompt_with`, or add to `model.queued`. Assert these invariants with spies in both live-loop adapters.

### 4.4 Draft identity, insertion and undo

Add a `composer_epoch` to the TUI model. Increment it on session replacement, new/fork/resume, explicit whole-draft replacement, external-editor replacement, and any operation that changes the identity of the draft. Centralize these relevant replacements behind a small model helper. Ordinary edits and cursor movement do not increment it.

Accept a result only if its ID is the currently active ID, its epoch equals the current epoch, its state permits a result, and the normal composer is still eligible. Opening an overlay cancels instead of retaining a result to surprise the user later. Duplicate, cancelled or stale results are discarded without mutation.

Add `Model::insert_dictation(text, expected_epoch)` as a narrow guarded operation. After normalization, call `composer.push_str(&text)` once, mark the caret moved, and refresh suggestions. That delegates to the existing atomic `Editor::insert_str`. Do not use `set_text`, synthesize keyboard input, replace previous text, or restore a snapshot on cancellation. One Ctrl+- immediately after insertion undoes only the dictation transaction; later typed edits retain their normal undo ordering.

There is no observed shared selection API to extend. Initial behavior is insertion, not replacement of a selected range. Preserve existing paste markers and multiline content already present in the buffer.

### 4.5 Normalization contract

Treat engine output as untrusted plain text, not commands or terminal control sequences. Require valid UTF-8, normalize CRLF/CR to LF, remove ESC and C0/C1 controls except LF, convert tabs to spaces, and trim only engine-created outer whitespace. Preserve Unicode combining characters, zero-width joiners, language punctuation and internal line breaks. Do not run an LLM cleanup pass or translate unless translation becomes a separate future feature.

Join engine segment strings in their original order without injecting a space between every segment; that would damage languages without word spaces. At the final insertion boundary, add a separating space only when both neighboring characters are ASCII alphanumeric or underscore and neither side is already whitespace. Apply that rule independently at the left and right edges. All pre-existing bytes remain unchanged. Test punctuation, CJK, mixed scripts and insertion in the middle of a line.

A normalized empty result produces a NoSpeech notice and no undo entry. Cap final text at 32 KiB of UTF-8; an over-limit result is an error rather than a partial insertion. Spoken words such as “send,” “enter,” or “delete everything” are text only.

## 5. Speech-to-text technology comparison

The comparison below is specific to a Rust terminal with short dictated prompts. It is not a universal accuracy ranking. No latency or accuracy benchmark was executed on this checkout or the user's microphone.

| Candidate | Quality, languages and interaction | CPU, GPU and resources | Packaging, maturity and license | Decision |
|---|---|---|---|---|
| whisper.cpp + Whisper base | Multilingual transcription with punctuation; suitable for record-then-transcribe. Incremental demonstrations exist, but stable editable partials would require additional orchestration. | CPU inference and quantization supported; optional accelerator backends. Base has a modest model footprint compared with larger Whisper variants. Loading is nonzero and benefits from warm reuse. | Native C/C++, public C API, documented Windows/macOS/Linux support, active releases. Engine and original weights MIT. | Selected: best overall fit for multilingual prompt dictation without a Python runtime. [S1], [S2], [S3] |
| faster-whisper / CTranslate2 | Uses Whisper models; quality depends on model and decoding settings. A segment generator is not by itself a complete interactive streaming design. | CPU INT8 is available; GPU configurations add accelerator runtime requirements. Performance depends on beam size, batching and hardware. | Python and CTranslate2 deployment plus audio dependencies are a larger distribution surface. faster-whisper is MIT. | Credible server/batch option, but unnecessary Python packaging for this Rust feature. Do not infer universal speed superiority from differently configured benchmarks. [S5] |
| sherpa-onnx with an audited model, e.g. original Moonshine Tiny | Both streaming and offline recognizer families exist. Original Moonshine Tiny is English-focused; newer Moonshine families differ. Model choice determines languages and latency. | Small specialized models can be attractive for short CPU dictation; ONNX execution providers vary by build. No measured comparison here. | Native/Rust integration is available. sherpa-onnx is Apache-2.0; original Moonshine Tiny weights are MIT. Every other model needs its own audit. | Strong alternative if English-only or true streaming becomes the primary requirement; not the multilingual default in this plan. [S6], [S7] |
| Vosk with an explicitly licensed small model | Genuine incremental recognition and language-specific models; vocabulary controls can be useful. Punctuation and free-form developer prose require evaluation. | Small English model listed at 40 MB; well suited to constrained CPU deployments. Site guidance describes roughly 300 MB runtime memory for small models. | Mature Kaldi-based API; engine Apache-2.0. Model licenses vary, including restrictions that are unacceptable for this catalog. | Good low-resource/streaming alternative; not selected without task-specific quality results beating the proposed default. [S8], [S9] |

Do not ship several speech runtimes initially. Keep the adapter boundary so a benchmark-supported engine replacement does not alter capture, UI, insertion or the no-send invariant.

### 5.1 Models and resource expectations

Upstream whisper.cpp lists approximate model file/runtime-memory figures of tiny 75 MiB/~273 MB, base 142 MiB/~388 MB, and small 466 MiB/~852 MB. These are upstream guidance, not total application RSS, guaranteed startup time, or measured results on this machine. Audio buffers, per-inference state, executable code and the main harness add overhead. [S1]

Use multilingual `base` because the requested language was not restricted. Offer `tiny` for responsiveness on constrained CPUs and `small` for users accepting higher compute and memory demand. An English-only `base.en` can be an explicit catalog option after its artifact is audited; do not silently infer English from the interface language. Defer quantized variants until comparative fixtures show their accuracy/latency tradeoff; do not download or substitute them automatically.

## 6. Selected open-source stack and rationale

### 6.1 Native engine and version policy

Use whisper.cpp through an opaque project-owned C ABI shim. Keep the shim restricted to context creation/destruction, inference with cancellation, retrieving UTF-8 text and error/version reporting. Construct whisper parameter structs in C++; never duplicate their changing memory layout in Rust. Catch C++ exceptions at the shim boundary, use explicit ownership/free functions, and never unwind across FFI.

The native API provides PCM inference with a separate state and an abort callback. Keep one model context in the worker, create a fresh inference state per utterance, and serialize calls to that context. [S3]

**Inspection-time candidate pin:** whisper.cpp `v1.9.3`, shown by upstream as a **pre-release**, with displayed commit prefix `371b5a7`. The release page includes fixes for very-short-audio out-of-bounds access and malformed tensor dimensions. Do not label this tag a stable release or substitute a moving `master`/nightly URL. Resolve and record the full commit and source archive digest during Phase 0. A later stable release containing the same fixes is preferable only after the same compatibility and native-fixture gates pass. [S10]

This choice intentionally favors a source revision containing the inspected safety fixes over blindly choosing an older stable label. Its pre-release status is a release risk and a required test gate, not a claim that this build already works in the repository.

The `whisper-rs` option was investigated. Its archived GitHub repository points to continued development on Codeberg; archive status alone does not mean the project is abandoned. The inspected published crate is 0.16.0 and the GitHub license is Unlicense, not MIT. A binding release must be paired with its actual bundled native revision. This plan avoids coupling the safety-fix pin to that independently released binding by using the small shim. Do not add `whisper-rs` or `whisper-rs-sys` as a second implementation. [S11], [S12]

### 6.2 Capture and conversion

Pin **CPAL `=0.16.0`** as the initial compatibility target. Its manifest declares Rust 1.70 and Apache-2.0; the current 0.18 line raises the toolchain requirement beyond this repository's pinned Rust 1.83. Do not upgrade Rust as an incidental voice change. Older-line maintenance and full transitive resolution still require review. [S4], [S13]

Use **Rubato `=0.16.2`** as the proposed older-API resampler pin and **ringbuf `=0.4.8`** for bounded single-producer/single-consumer buffering. The projects use permissive licensing. The exact Rubato 0.16.2 manifest and the full Rust 1.83 transitive build were not verified in this investigation; Phase 0 must verify its packaged license/API/MSRV before landing the lockfile. Do not replace that gate with an unbounded dependency on the current Rubato release, whose toolchain requirements differ. [S14], [S15]

Use the existing workspace `serde`, `serde_json`, `thiserror`, `sha2`, `ureq`, `zeroize` and path helpers. Use **cmake `=0.1.58`** as the Rust build helper; its manifest declares Rust 1.65 and MIT OR Apache-2.0. Native builds additionally require a C/C++ toolchain and CMake. No runtime compiler is required for installed release binaries. [S16]

### 6.3 Free/open-source boundary

The shipped speech implementation and approved models must be free/open-source, redistributable, and usable without accounts, fees or network services. Audit both source and binary dependency graphs; permissive top-level licenses do not prove every transitive component has the same license.

Disable CUDA/cuDNN, CoreML, Metal, Accelerate, ASIO and other optional proprietary acceleration/SDK paths in the strict default speech build. Windows WASAPI and macOS CoreAudio are operating-system interfaces on the already requested platforms; this plan does not claim Windows or macOS itself is open source. No proprietary speech recognizer or accelerator service is required. Linux system audio libraries may have LGPL obligations; account for them instead of describing every system dependency as permissively licensed.

An optional future Vulkan build may be offered only with an audited open-source userspace/driver combination. A Vulkan API label alone does not prove the installed driver is open source. CPU remains the supported fallback and the only required backend for initial completion.

## 7. Proposed architecture and data flow

```text
MAIN DAVINCI PROCESS                         DAVINCI-VOICE-WORKER

Ctrl+T ----\
            > VoiceAction::Toggle
mic click -/          |
                      v
           VoiceController / pure reducer
           session ID + composer epoch
           pending send protection
                      |
            bounded private stdio protocol -----> control reader
                      |                               | cancel flag
                      |                         native owner thread
                      |                               |
                      |                         CPAL input stream
                      |                               |
                      |                     preallocated SPSC buffer
                      |                               |
                      |                      downmix + resample
                      |                         16 kHz mono f32
                      |                               |
                      |                       bounded RAM audio
                      |                               |
                      |                      whisper.cpp CPU decode
                      |                               |
             FinalText(id, epoch, text) <--------------+
                      |
             validate / normalize
                      |
           Model::insert_dictation
                      |
         Composer::push_str -> Editor::insert_str
                      |
          USER REVIEWS AND EDITS TEXT
                      |
       later explicit existing send / queue action
                      |
          existing on_line / submit_prompt path
```

### 7.1 Component boundaries

| Component | Responsibilities | Must not do |
|---|---|---|
| TUI voice view | State labels, mic geometry, help and theme integration | Own streams, model weights, process handles or audio |
| Pure reducer | Legal transitions, IDs, timeouts, cancellation eligibility | Perform I/O or submit messages |
| Host `VoiceController` | Execute reducer effects, supervise worker, coordinate context/epoch and insertion | Block the UI on process I/O, decoding or model loading |
| Worker protocol layer | Framed commands/events, version handshake, limits, EOF handling | Listen on a network port or execute shell commands |
| Capture adapter | Device negotiation, callback, shutdown and capture errors | Perform inference or logging in the audio callback |
| Audio pipeline | Downmix, band-limited resampling, bounded accumulation, coarse silence checks | Write recordings to disk |
| STT adapter | Load approved model, decode, cancel, return text and typed errors | Download models or retain previous dictation as prompt context |
| Model manager in host package | Catalog, consent, download/import, checksums, atomic installation | Open a microphone or run arbitrary model-provided code |

### 7.2 Crate and process organization

Add a workspace crate `crates/davinci-voice`. Its default library contains lightweight types, normalization, protocol and reducer logic. A `native` feature enables CPAL, resampling, buffering and the native engine. Declare the `davinci-voice-worker` binary with `required-features = ["native"]`.

The coding-agent and TUI depend only on the lightweight library. This creates no TUI-to-agent dependency cycle. The cloneable `Model` receives a small `VoiceView`, not native resources. Default workspace tests need not compile the speech engine; a separate native job does. The crate's build script must return without invoking CMake when the native feature is absent.

The host launches a trusted absolute sibling path derived from the installed `davinci` executable, never a bare name resolved through the current directory or `PATH`. Release archives install both executables together. Require a protocol-version and build-identity handshake before capture. Missing/incompatible helper is a recoverable feature-unavailable state.

Use private stdin/stdout pipes rather than HTTP, sockets or a long-lived daemon. Audio stays entirely in the worker. The host uses dedicated pipe-reader/writer threads and bounded channels so writes or reads never block terminal event processing. Drain native stderr continuously into a bounded, redacted diagnostic buffer or suppress it; never inherit raw native output into the TUI.

### 7.3 Thread ownership and lifecycle

Create and destroy CPAL streams and the model context on the worker's owner thread; do not assume these types can move across threads on every platform. CPAL's callback performs only bounded sample conversion/copy and nonblocking buffer operations. A control-reader thread updates cancellation state while native inference is running. A small writer thread handles framed status output; intermediate status may be coalesced.

The parent polls voice events in both `run` and `run_turn`. While voice is active, clamp terminal polling to approximately 40 ms; retain the current idle cadence otherwise. Keep bounded work per iteration so an event flood cannot starve keyboard handling. Do not add Tokio or move the existing agent provider loop to a different concurrency framework.

Warm model weights may be kept for five minutes after a successful session, then unloaded off the UI thread. Never retain an open input stream during that interval. A fresh inference state is used for each recording; disable previous-utterance conditioning. This prevents dictation text from leaking into the next recognition context.

Cancel immediately invalidates the active result. Request cooperative abort, allow 750 ms for normal shutdown, and terminate the worker if it has not stopped; reap it off the UI thread. Do not start a replacement capture until the old worker has exited or confirmed capture closure. Parent-pipe EOF triggers worker cancellation; a watchdog must exit a stuck worker if native code ignores cancellation after its parent disappears.

### 7.4 Protocol contract

Use length-prefixed UTF-8 JSON: a fixed-width little-endian length followed by one JSON payload. Enforce a 64 KiB frame maximum before allocation. Set bounded channel capacities, reject invalid UTF-8/unknown mandatory versions, and ensure a blocked status pipe cannot block cancellation or capture shutdown.

Host commands: `Hello`, `Prepare`, `Start(session_id, composer_epoch, config)`, `Stop(session_id)`, `Cancel(session_id)`, `Shutdown`. Worker events: `Ready`, `CaptureStarted`, `CaptureStopped`, `Transcribing`, `Completed`, `Cancelled`, `Failed`. Commands and session events carry identity; preparation and handshake use a separate request ID where no recording exists yet.

`Completed`, `Cancelled` and `Failed` are mutually exclusive terminal outcomes for an accepted recording ID. `CaptureStopped` is an intermediate acknowledgement, not a second terminal result. Ignore late lower-sequence events and duplicate terminal messages. Treat unexpected exit/EOF as `WorkerCrashed`, not an empty successful transcript.

Preparation may begin only after a direct user activation or explicit setup action. Opening a stream requires a still-valid activation and eligible context. If preparation or an OS permission interaction lasts more than 30 seconds, return to Idle and ask for a fresh activation rather than starting an unexpectedly delayed recording.

### 7.5 Audio capture and inference contract

Discover the current default input device at activation unless a device was explicitly selected. Enumerate supported capture configurations and choose a supported rate/channel/sample format; do not assume the hardware accepts 16 kHz. Handle at least F32, I16 and U16, and return a typed error for any unsupported format rather than panicking. Explicitly bound accepted configurations, for example 8–192 kHz and one to eight channels.

CPAL 0.16 must be used according to its own API, not the latest documentation's newer device-ID interfaces. Store a host/name/disambiguation descriptor for selected devices and validate it every activation. If that explicit device disappears or becomes ambiguous, ask the user to select again; do not silently switch to a different microphone.

Use a preallocated bounded SPSC ring sized for up to two seconds of the negotiated native format, with a hard 16 MiB cap. The callback does not allocate, wait on a mutex, perform I/O, resample or call STT. An overflow marks the recording failed and stops capture; silently dropping audio could turn a spoken command into different text.

Off the callback, downmix channels consistently, sanitize non-finite samples, clamp sample range, and use band-limited Rubato resampling to 16 kHz mono f32. Compensate resampler delay and flush its tail without dropping the final syllable. Maintain one accumulated normalized buffer capped at 120 seconds: `120 * 16,000 * 4 = 7,680,000` bytes, about 7.32 MiB, excluding ring and engine memory. Avoid repeated full-buffer clones.

On Stop, close the input stream, drain accepted samples and finish resampling before inference. Recordings shorter than 300 ms, empty buffers and digital silence produce NoSpeech without decoding. Use a conservative energy check plus the engine's no-speech handling; an energy meter is not a semantic VAD and cannot guarantee suppression of all hallucinations. Do not add a separate neural VAD model in the first release. Leave automatic silence endpointing disabled.

Start with greedy decoding, one candidate, temperature zero, no temperature-increase retry ladder, transcription rather than translation, configured language or autodetect, and no previous-session prompt. Suppress native progress/text printing. Return ordinary segment text without timestamp markup. Cap inference time at `min(360 seconds, max(60 seconds, 3 * recorded_duration))`, using the process cancellation mechanism. These are safety limits, not performance predictions.

Use `max(1, min(4, available_parallelism.saturating_sub(1)))` as the initial CPU-thread policy. Benchmark it while an agent turn is active. Do not give inference every core merely because it improves an isolated benchmark.

## 8. Component/file-level changes

All paths in this section are **proposed implementation changes**, not changes performed by creating this document.

| Path | Planned change |
|---|---|
| Root `Cargo.toml`, `Cargo.lock` | Add `davinci-voice`, exact dependency pins, native feature graph and audited transitive lockfile |
| `crates/davinci-voice/Cargo.toml` | Lightweight library; native-only worker binary; optional capture/engine dependencies |
| `crates/davinci-voice/src/lib.rs` | Export small public action/event/view/config contracts |
| `crates/davinci-voice/src/state.rs` | Pure session reducer and fake-clock transition tests |
| `crates/davinci-voice/src/protocol.rs` | Bounded framing, protocol version, IDs and serialization tests |
| `crates/davinci-voice/src/error.rs` | Typed capture/model/worker errors with safe user messages |
| `crates/davinci-voice/src/normalize.rs` | Text filtering and insertion-boundary policy with Unicode tests |
| `crates/davinci-voice/src/capture.rs` | CPAL device discovery, configuration selection, stream ownership and callback |
| `crates/davinci-voice/src/audio.rs` | Ring consumption, downmix, Rubato conversion, duration bounds and silence checks |
| `crates/davinci-voice/src/engine.rs` | STT trait, opaque native adapter, per-utterance state and cancellation |
| `crates/davinci-voice/src/bin/davinci-voice-worker.rs` | Process entry, control reader, owner loop, watchdog and clean shutdown |
| `crates/davinci-voice/build.rs`, `native/CMakeLists.txt`, `native/voice_shim.{h,cpp}` | Native-only build and minimal stable C ABI |
| `crates/davinci-voice/native/upstream/` | Reviewed, pinned whisper.cpp source including its required GGML source and licenses; do not modify `vendor/davinci` |
| `crates/davinci-voice/native/UPSTREAM.md` | Full source revision, archive digest, patches if any, license inventory and upgrade procedure |
| `crates/davinci-tui/Cargo.toml` | Depend on lightweight voice types, not the native feature |
| `crates/davinci-tui/src/davinci/model.rs` | Voice view, draft epoch, guarded insertion and draft-replacement helpers |
| `crates/davinci-tui/src/davinci/app.rs` | Voice flow/action, composer eligibility, early send guard and layout metadata |
| `crates/davinci-tui/src/davinci/views/chrome.rs` | Upper-rule mic rendering with exact local geometry and state labels |
| `crates/davinci-tui/src/davinci/runtime.rs` | Mouse lifecycle, cached frame hit targets, resize invalidation, exhaustive fixture Flow handling |
| `crates/davinci-tui/src/keybindings.rs`, `keys.rs` | New action, context-aware effective bindings, explicit-override tracking, Alt+T normalization/tests |
| `crates/davinci-tui/src/editor.rs` | Regression tests only unless an inspected integration defect requires a narrowly scoped fix; reuse `insert_str` |
| `crates/davinci-coding-agent/Cargo.toml`, `src/main.rs` | Lightweight dependency, module declarations, early `voice` CLI dispatch |
| `crates/davinci-coding-agent/src/voice_input.rs` | Host supervisor/controller, common event preflight, process I/O and insertion orchestration |
| `crates/davinci-coding-agent/src/voice_models.rs` | Catalog, explicit download/import, integrity validation and cache operations |
| `crates/davinci-coding-agent/src/davinci_interactive.rs` | Wire controller into Shell, idle and running loops, busy key path, approvals and shutdown |
| `crates/davinci-coding-agent/src/settings.rs` | User-level voice settings, validation, project-scope exclusion and preservation of unknown keys |
| `crates/davinci-coding-agent/src/extension_host.rs` | Reserve effective voice chord; preserve non-voice extension routing |
| `crates/davinci-coding-agent/src/slash.rs`, `help.txt` | Setup/status/device/model commands and help, without sending them to providers |
| `crates/davinci-parity/fixtures/voice/` | Small licensed PCM fixtures and provenance manifest, not captured user audio |
| `.github/workflows/ci.yml` and release packaging scripts | Separate lightweight/native jobs; Windows/macOS/Linux native validation; bundle helper |
| `docs/ui/design.md` | Document mic exception to keyboard-only design, new shortcut and selection implications |
| `docs/voice-input.md` and redistribution notices | Installation, offline import, permissions, CPU limits, support scope and licenses |

### 8.1 Shared layout result

Introduce an internal `ComposerRender { rows, mic_local_rect }` and a final `ComposedFrame { lines, mic_rect }`. Keep the existing `compose(...) -> Vec<Line>` wrapper where useful for compatibility with current snapshot tests.

Calculate the mic's global row from the actual number of rows already appended when the composer is inserted. Apply final truncation and viewport bounds to both text and rectangles. `Session::draw` stores the resulting hit target only after a successful draw. A hidden or clipped mic has no hit target. Reuse the same Unicode width calculations in rendering and hit testing.

### 8.2 Shortcut migration without stealing explicit bindings

Add the action `davinci.voice.toggle`. For the default Da Vinci composer with voice enabled, assign Ctrl+T to voice and move the default `davinci.tools.expand` to **Alt+T**. Add the missing Alt+T canonical encoding where required and remove the hardcoded busy-loop Ctrl+T toggle in favor of the effective action map.

Track which keybindings were explicitly supplied by the user when parsing `keybindings.json`; current loading loses that distinction. If an explicit user action already owns Ctrl+T in the same active context, preserve it and leave voice keyboard activation unbound with a clear diagnostic and working mic click. If the user explicitly gives voice a collision, report it and require an unambiguous mapping rather than silently shadowing another action.

Legacy thinking and tree-filter Ctrl+T remain unchanged in their own contexts. Disabled voice restores the ordinary default tool-output binding unless the user customized it. Do not rewrite the user's keybindings file automatically. Resolve extension reservations against the effective context map, not dormant legacy defaults.

Voice-owned activation, cancellation and active-session send protection run before raw extension interception. All unrelated keys retain their existing extension-first ordering. Reset the double-Escape timer when Escape is consumed by voice, so cancellation cannot become half of a later navigation gesture.

## 9. Dependencies, model distribution, and configuration

### 9.1 Minimal dependency set and build policy

| Addition | Purpose | Pin / license / qualification |
|---|---|---|
| whisper.cpp source | Native local STT | Candidate `v1.9.3`, MIT; exact full revision/digest and pre-release gates required |
| CPAL | Cross-platform microphone input | `=0.16.0`, Apache-2.0; declared MSRV compatible with Rust 1.83 |
| Rubato | Band-limited sample-rate conversion | Proposed `=0.16.2`, project MIT/Apache-2.0; packaged-version/MSRV verification required |
| ringbuf | Nonblocking bounded callback handoff | `=0.4.8`, MIT OR Apache-2.0; lockfile/toolchain validation required |
| cmake Rust crate | Build-time native compilation driver | `=0.1.58`, MIT OR Apache-2.0; declared Rust 1.65 |
| Existing workspace crates | JSON, errors, hashing, explicit setup HTTP, zeroization | Reuse current exact workspace versions; no duplicate HTTP/runtime stack |

No Python, PyTorch, ONNX Runtime, Node, SDL2, FFmpeg, browser speech API, cloud STT SDK, or paid dependency is needed for the selected path. These exclusions are design decisions, not statements that the alternative projects are non-open-source.

Compile only the required native library and shim, not whisper.cpp examples, server or download utilities. Set `BUILD_SHARED_LIBS=OFF`, `WHISPER_BUILD_TESTS=OFF`, `WHISPER_BUILD_EXAMPLES=OFF`, `WHISPER_BUILD_SERVER=OFF`, `WHISPER_CURL=OFF`, `WHISPER_SDL2=OFF`, and `WHISPER_COREML=OFF`. Set `GGML_CPU=ON`, `GGML_NATIVE=OFF`, `GGML_BACKEND_DL=OFF`, `GGML_METAL=OFF`, `GGML_ACCELERATE=OFF`, `GGML_BLAS=OFF`, `GGML_CUDA=OFF`, `GGML_VULKAN=OFF`, `GGML_OPENMP=OFF`, and `GGML_RPC=OFF` for the default artifact. Explicitly review all other backend defaults when the source pin changes. [S17], [S18]

`GGML_NATIVE=OFF` alone does not prove generic CPU compatibility: the inspected GGML configuration has separate ISA defaults. Set an explicit x86 baseline for the portable worker, turning off AVX/AVX2/FMA/F16C/BMI2/SSE4.2-specific options unless the artifact explicitly requires them. Optionally package an AVX2 worker later and select it only after CPU-feature detection. Do not let a build host silently determine release CPU requirements. [S18]

Use C++17 and a tested CMake version in native CI. Build native sources from the vendored pin without network access in `build.rs`; obtain/update source intentionally during development, not while an end user's Cargo build is running. Package platform-required runtime libraries deliberately and audit their notices. A normal user installing release binaries should not need a compiler, CMake, Git or model-conversion script.

### 9.2 First-run experience

On initial activation, check whether the compatible worker and selected verified model exist without opening a microphone. If the model is missing, show an in-TUI setup sheet explaining local runtime, model size, disk destination, license and the fact that initial acquisition requires Internet unless imported.

Offer **Download base**, **Import existing model**, and **Cancel**. Download/import runs off the UI thread with real byte progress. Completing setup returns to Idle with “Ready — Ctrl+T to listen.” It does not begin recording just because the user installed a model. Do not require an AI-provider account or initialize a provider to perform voice setup.

### 9.3 Catalog and cache

Use the existing `davinci_session::discovery::default_agent_dir()` behavior. It respects `DAVINCI_CODING_AGENT_DIR`, then `PI_CODING_AGENT_DIR`, and existing `.davinci/agent` or legacy `.pi/agent` locations. Do not hardcode `.pi`, the repository directory, or a new unrelated Windows path. [Repository: `crates/davinci-session/src/discovery.rs:40–68`.]

Store model files under:

```text
<resolved-agent-dir>/voice/models/<catalog-id>/<sha256>.bin
```

The approved catalog contains `tiny`, `base`, and `small`, their GGML filenames, exact byte counts, immutable upstream revisions, SHA-256 hashes, source/license notices and compatible engine identity. Obtain artifacts from the documented whisper.cpp model distribution, not a search-result mirror. Do not confuse GGML `.bin` model files with arbitrary GGUF or Python checkpoint files. [S1], [S19]

**Unresolved release metadata:** This investigation did not establish a complete immutable revision/hash/size tuple for every selected model. Phase 0 must populate and independently verify those tuples before the installer ships. Never use a placeholder checksum, moving `main` download as a trust anchor, or a pull-request model URL as the production catalog. This is artifact verification work, not an undecided architecture.

Download into a uniquely named `.part` file in the destination filesystem. Enforce expected length, streaming size bounds, TLS verification, bounded HTTPS redirects and timeouts. Hash while streaming; publish by atomic rename only after length and SHA-256 match the bundled catalog. Serialize concurrent installs of the same model and retain the previous valid file until replacement succeeds. On failure, remove the incomplete file or leave it explicitly marked incomplete; never load it.

Verify integrity before loading a new model identity, off the UI thread. Reuse verified identity while the same worker keeps the model open; metadata changes require revalidation. Reject symlink/path substitutions where practical, and pass an already opened/readable model stream into the native loader or ensure the worker verifies the exact bytes it loads. Host verification followed by an unrelated pathname reopen is not sufficient protection against substitution.

Local import copies and verifies an approved catalog file; do not execute model code, deserialize Python pickles, or automatically trust an arbitrary format. The initial import UI supports approved catalog models only. An unrestricted custom-model interface is not needed for this release.

### 9.4 User-level configuration

Use a small optional `voice` block in the existing global `settings.json`:

```json
{
  "voice": {
    "enabled": true,
    "model": "base",
    "language": "auto",
    "inputDevice": null,
    "backend": "cpu"
  }
}
```

Absent settings resolve to these defaults in a voice-capable distribution. `enabled` means the feature is available, not that it records in the background. `inputDevice: null` selects the current system default. `language` accepts `auto` or a validated engine language code. Model names are catalog IDs, not arbitrary download URLs. `backend` initially accepts only `cpu`; future compiled capabilities may add a specifically documented option.

Shortcut overrides remain in the existing `keybindings.json`:

```json
{
  "davinci.voice.toggle": ["ctrl+t"],
  "davinci.tools.expand": ["alt+t"]
}
```

Keep advanced `voice.mouse=false` available for terminal-selection compatibility, but do not expose sample rate, buffer size, decoder temperatures, beam settings, timeout ladders or thread counts in the ordinary settings UI.

Voice hardware/model/enablement settings are **global-user settings only**. Project settings, including trusted project settings, must not silently enable capture, switch microphones or redirect model loading. Add merge tests proving project `voice` blocks cannot override these choices. Preserve the settings loader's existing handling of unknown unrelated keys.

### 9.5 CLI and release behavior

Add early handling for `davinci voice status`, `davinci voice devices`, `davinci voice model list`, `davinci voice model install <id>`, and `davinci voice model import <id> <path>`. The repository uses a handwritten argument parser; ordinary positional tokens otherwise become chat messages. Dispatch these commands before normal prompt parsing and provider/extension startup, following the existing early package-command pattern. Unknown voice subcommands return usage errors and are never sent to an agent. [Repository: `main.rs::run`, `args.rs::parse_args`.]

Honor the existing offline-mode helper and `--offline` behavior: an explicit network install fails helpfully in offline mode; local import, device listing and transcription remain usable. Avoid changing the harness's global offline semantics as part of this feature.

Release archives bundle `davinci`, its matching `davinci-voice-worker`, and license/build manifests. Models remain an explicit separate acquisition by default. Source-only builds without the native worker continue to support typed input and show accurate voice-unavailable guidance. Ensure any existing installation command that copies only `davinci` is updated before advertising bundled voice support.

## 10. Privacy and offline behavior

The voice path processes microphone audio and recognition text locally. It does not invoke the harness's configured chat provider, a browser speech service, telemetry endpoint or remote model service. Runtime network access is not needed once the compatible binaries and selected model have been provisioned.

Do not describe the complete installation as “100% offline” when obtaining binaries, dependencies or the first model uses the Internet. Support air-gapped setup by transferring the release archive and an approved model to the machine and importing locally. Test actual recognition with network access blocked and with no STT/API credentials configured.

| Data | Default lifecycle |
|---|---|
| Native capture samples | Worker-owned bounded ring; consumed and discarded |
| Normalized PCM | Worker RAM only; released on completion, cancellation or failure |
| Inference state | Fresh per utterance; destroyed afterward, not reused as conversational context |
| Model weights | Local read-only cache; may remain warm for five minutes |
| Final transcription | Delivered to the host once and inserted into the ordinary editor |
| Unsubmitted text | Existing editor/undo semantics only; no new dictation history or transcript log |
| Submitted text | Existing harness session/history/provider behavior, only after an explicit send |
| Diagnostics | Error category, duration, model ID and technical context; never raw audio or transcript text by default |

Use the existing `zeroize` dependency for owned audio buffers and sensitive protocol buffers where practical. Do not claim forensic erasure: allocator behavior, native inference scratch memory, OS audio buffers, swap and crash dumps can retain data outside that guarantee. Avoid automatic audio dumps and redact native stderr. Model filenames and device names may also be sensitive; do not add them to telemetry.

Spawn the worker with a minimal environment needed for its platform audio session rather than inheriting provider credentials unnecessarily. It has no network service and no reason to initialize extensions or the agent. This separation is not an OS security sandbox; normal user permissions still apply.

After the user sends the reviewed text, the existing harness may send that prompt to its configured remote AI provider. Clearly distinguish **local voice recognition** from **an entirely offline AI harness**. Existing trusted extensions retain their normal access to editor or submitted-text behavior; this feature adds no new dictation-specific extension event or audio access.

## 11. Cross-platform considerations

| Platform | Planned capture/build approach | Required verification and recovery |
|---|---|---|
| Windows | CPAL 0.16 WASAPI; native CPU helper built and packaged for the supported Rust target | Test Windows Terminal/PowerShell and ConPTY paste behavior, Unicode paths, device disconnects and actual desktop-microphone privacy settings |
| macOS Intel/Apple Silicon | CPAL CoreAudio, CPU-only native engine, no required Metal/CoreML/Accelerate path | Test permission attribution to terminal/helper, fresh denied/granted states, executable metadata and packaging on each advertised architecture |
| Linux | CPAL 0.16 ALSA; system routing may lead to PipeWire/PulseAudio when configured | Test supported ALSA devices and desktop audio routing; missing audio libraries/devices produce typed errors, not startup failure |
| WSL, SSH, containers, remote clients | No microphone forwarding in this feature | Device unavailable is an expected possibility; explain that capture occurs where the helper runs |

The CPAL 0.16 target manifest, not the latest API documentation, is the reference for the selected capture backends. Do not advertise a native PipeWire backend merely because a Linux desktop uses PipeWire. [S4]

On Windows, denial guidance should point to **Settings -> Privacy & security -> Microphone**, including desktop-app microphone access. Do not assume each unpackaged desktop executable has an individually toggleable permission. Do not automatically change privacy settings or the default audio device. [S20]

For macOS, provide an appropriate microphone purpose string using `NSMicrophoneUsageDescription` in the helper's packaging metadata and the applicable audio-input entitlement for hardened/sandboxed packaging. Determine the exact terminal-versus-helper permission attribution by testing the actual distributed executable; do not assume that a terminal grant covers every launch form. This is microphone capture, not system-audio capture, so a system-audio permission key is not a substitute. [S21], [S22]

Use OS-returned errors where available. CPAL/platforms may not expose a portable, precise “permission denied” classification in every case. Preserve that uncertainty: show “Microphone could not be opened; check device access and permissions” rather than confidently mislabeling every unavailable device as denial.

Native compilation on Linux needs the selected audio development headers/libraries; release users need the corresponding runtime libraries. Prefer dynamic use of distribution audio libraries and include required license notices. Do not bundle optional ASIO, JACK, GPU or codec dependencies unless explicitly selected and audited.

Cross-platform capability in upstream projects is not proof that this exact crate/native-source/toolchain combination passes on all three platforms. The supported-platform list is gated by Phase 5 evidence. Do not raise the harness's minimum OS version incidentally without documenting and validating the change.

## 12. Failure handling

Every failure preserves all existing typed input. An error must not call `clear`, `submit`, `queue`, or restore an old full-editor snapshot. No partial recognition is inserted on an error.

| Failure | Detection / state | User-facing recovery and resource behavior |
|---|---|---|
| No microphone | No default device or empty input-device list | `No microphone found`; offer device list and retry; no stream opened |
| Permission denied | Specific OS error when available, otherwise access failure | Explain relevant OS settings; no repeated permission prompts or automatic retries |
| Device disconnect | CPAL error callback / stream failure | Stop session, discard partial audio, preserve draft; require fresh activation |
| Unsupported backend/format | Host unavailable or no accepted configuration | Explain unsupported capture configuration; normal typing still works |
| Explicit device missing | Descriptor cannot be resolved unambiguously | Ask to select a device; do not silently record another device |
| Model missing | Catalog path absent | Open setup; no microphone capture until ready and newly activated |
| Model corrupt/substituted | Size/hash/load validation fails | Quarantine or remove only the invalid model file; offer verified re-download/import |
| Download failed | Timeout, TLS/network failure, disk-full or checksum mismatch | Keep existing valid model; never publish partial file; retry only by user action |
| Helper absent/incompatible | Spawn/handshake failure | Explain binary installation mismatch; do not search random PATH executables |
| Native crash | EOF or non-success worker exit | Show transcription failure, discard audio/result, reap helper; next explicit activation may restart |
| Empty/no speech | Empty/short/digital-silence input or empty decode | Brief `No speech detected`; no editor or undo mutation |
| Background/noise hallucination | Not perfectly detectable | Keep explicit review requirement; evaluate no-speech fixtures and improve thresholds without claiming certainty |
| Too-long recording | 120-second monotonic cap | Stop capture, show `Recording limit reached; transcribing`; never auto-send |
| Ring overflow | Callback overflow flag | Stop and report interrupted recording rather than inserting incomplete speech |
| Inference timeout | Duration-based deadline | Cancel then terminate worker if needed; no fallback to cloud |
| User cancellation | Escape, disable, context switch or terminal handoff | Invalidate ID immediately; close microphone; discard queued/later result |
| Stale/duplicate result | ID/epoch/state mismatch | Ignore silently or count locally in test diagnostics; never touch another draft |
| Shortcut collision | Effective context keymap validation | Preserve explicit existing mapping, show conflict, keep mic/palette available |
| Terminal too small | Active status cannot be rendered | Cancel capture safely; do not continue invisible recording |

Persistent errors remain discoverable through voice status/help but should not pollute the conversation transcript with repeated tool-like entries. A successful retry clears the prior error. No error recovery silently switches model, language, device, engine or backend.

## 13. Phased implementation plan

Implement in the following order. For each phase, write the failing deterministic tests first, implement the smallest change, and then run the listed checks. Do not use real microphones or network downloads in default tests. The phases below are future engineering work, not work completed by this planning task.

### Phase 0 — Lock the integration baseline and native supply chain

**Objective:** Make the source, license and toolchain assumptions concrete before relying on them.

**Affected components:** Workspace manifests/lockfile, new `davinci-voice` skeleton, `native/UPSTREAM.md`, source/license manifest, model catalog metadata and fixture provenance.

**Concrete work:**

1. Re-read the current versions of the input-loop/editor symbols in Section 2 and record a baseline without reverting unrelated changes.
2. Create the lightweight crate and native feature boundary. Preserve Rust 1.83 and exact pins.
3. Resolve the full whisper.cpp revision and source digest; verify the inspected safety fixes are present. Vendor its complete required source without touching `vendor/davinci`.
4. Verify the packaged CPAL/Rubato/ringbuf/cmake licenses and full transitive Rust 1.83 build. If an older pin cannot build, choose a reviewed compatible patch and record the exact reason; do not silently raise the toolchain.
5. Populate immutable model revision/size/SHA-256/license tuples for tiny/base/small. Acquire only permitted test fixtures with recorded provenance.
6. Define the shim header, protocol version and CPU build flags. Ensure native build configuration performs no network fetches.

**Dependencies:** None beyond the existing repository/toolchain and deliberate development-time artifact acquisition.

**Tests/verification:** Core-only crate builds without CMake/audio libraries; native compile smoke test on the first target; malformed model/short-buffer native regression fixtures; license and dependency graph inspection.

**Completion criteria:** Reproducible source/model identities are recorded; no unknown license or MSRV blocker remains in the selected default stack; the core/native boundary is demonstrably real. Do not proceed to release packaging with placeholder checksums.

### Phase 1 — Deterministic editor vertical slice with fake capture/STT

**Objective:** Prove the UX and no-send behavior before audio complexity.

**Affected components:** `state.rs`, `protocol.rs`, `normalize.rs`, TUI model/app/keybindings/chrome/runtime, host `voice_input.rs`, both live loop adapters and their inline tests.

**Concrete work:**

1. Implement the pure reducer, IDs, epochs, fake clock and typed outcomes.
2. Add the guarded insertion method using existing `Composer::push_str` and test one-step undo.
3. Implement one common voice preflight for keyboard/mouse/palette actions, Escape and active-session send inhibition. Wire it into idle and busy input paths before autocomplete/queueing.
4. Replace the busy-loop hardcoded tool shortcut with effective action resolution; implement explicit-binding-aware Ctrl+T migration and Alt+T encoding.
5. Add the upper-rule mic and shared layout/hitbox result. Implement mouse acquisition/restoration and the keyboard-only fallback.
6. Connect a fake worker producing delayed completion/error/cancel events. Fixture/demo runtime handles new Flow variants without opening hardware.

**Dependencies:** Phase 0 lightweight contracts; no real model or microphone required.

**Tests/verification:** Keyboard and click produce identical action/effect sequences; live-caret multiline insertion; cancellation after typing; stale epoch results; duplicated completion; Enter while active; the special `/model` completion path; busy `queue()` spy remains untouched; mouse hitbox snapshots at changing widths/heights.

**Completion criteria:** The complete activation -> delayed text -> editable composer path works with fakes in idle and running states. No fake result can submit or enqueue text. Existing editor behavior passes its regression tests.

### Phase 2 — Smallest real local CPU slice

**Objective:** Make actual microphone dictation work using a pre-provisioned approved model.

**Affected components:** Worker binary, native shim/build, `capture.rs`, `audio.rs`, `engine.rs`, host supervision.

**Concrete work:**

1. Implement versioned private-pipe communication, bounded queues, trusted helper lookup and EOF/error behavior.
2. Implement CPAL device/configuration negotiation and preallocated callback buffering.
3. Implement downmix/resampling with delay/tail handling, sample sanitization and recording limits.
4. Load a verified local base model in the worker, decode through a fresh native state, and return only final text.
5. Implement Stop acknowledgement, cooperative cancellation, forced termination and off-thread reaping.
6. Keep the controller alive across entry/exit of `run_turn`; a running agent must not make voice stop responding.

**Dependencies:** Phase 1 acceptance tests; Phase 0 pinned native source and model artifact. This phase does not require a downloader to function.

**Tests/verification:** Deterministic PCM decoding with an explicitly supplied local model; format/resampling tests; native crash injection; stop/cancel during capture and inference; first manual Windows microphone path; no temporary WAV files; no provider calls.

**Completion criteria:** With local binaries/model installed, Ctrl+T and mic click can record, stop and insert editable text. UI input stays responsive, cancellation terminates capture, and no network service is used.

### Phase 3 — Model setup, configuration and polished interaction

**Objective:** Remove manual setup friction without introducing hidden network or capture behavior.

**Affected components:** `voice_models.rs`, `settings.rs`, `main.rs`, help/command registry, mic states/setup sheet and release installation plumbing.

**Concrete work:**

1. Add global-only validated voice settings and device selection. Ignore project voice overrides.
2. Implement early CLI dispatch, status/list/devices/import/install, and helpful unavailable-helper errors.
3. Add in-TUI first-run consent, actual byte progress, cancellation, checksum validation, `.part` handling and atomic model publication.
4. Add truthful model/permission/error states, two-second inserted notice, post-insertion submit guard and narrow/ASCII rendering.
5. Update `/hotkeys`, command-palette discovery, default tool-output shortcut and migration guidance.
6. Keep model acquisition separate from recording: install/import ends in Idle.

**Dependencies:** Working Phase 2 CPU path and approved catalog metadata.

**Tests/verification:** Fake HTTP transport for interrupted/download/redirect/size/hash/disk failures; offline install refusal; local import success; settings round-trip; project-scope exclusion; custom key collisions; terminal selection opt-out; setup completion does not open a microphone.

**Completion criteria:** A user can install the binary, explicitly acquire/import a model and use voice without touching internal files. All setup failures preserve the draft and the previous valid model.

### Phase 4 — Adversarial lifecycle and integration hardening

**Objective:** Make failure behavior and concurrent editor use dependable.

**Affected components:** Controller/reducer, both event loops, permissions/modal transitions, worker watchdog, protocol/normalization/native test suites.

**Concrete work:**

1. Audit every session/draft replacement, overlay opening, terminal handoff and exit path for cancellation and epoch invalidation.
2. Exercise completion-versus-Enter races, duplicate/out-of-order replies, pipe saturation and worker parent loss.
3. Harden microphone disconnect, capture overflow, no-speech, decode timeout and unsupported-format behavior.
4. Ensure native stderr cannot corrupt the terminal or leak dictated text; verify buffers do not grow across repeated sessions.
5. Measure UI latency with inference and agent work active; enforce bounded per-tick processing and thread limits.
6. Verify no speech path reaches send, queue, command dispatch, extension input or provider calls before explicit user submission.

**Dependencies:** Phases 1–3.

**Tests/verification:** Fake-clock exhaustive transition cases; repeated start/stop/cancel stress; killed helper; malformed protocol fuzzing; native sanitizers; deterministic modal/approval and session-switch regressions; memory/handle counts over repeated recordings.

**Completion criteria:** The failure matrix is covered by automated tests where feasible, with explicit manual gaps. UI and input integrity remain intact under cancellation, worker death and event races.

### Phase 5 — Cross-platform release verification and documentation

**Objective:** Ship a tested free/local feature rather than an upstream-support assumption.

**Affected components:** Native CI matrix, release archives/install scripts, third-party notices, `docs/voice-input.md`, `docs/ui/design.md`.

**Concrete work:**

1. Add Windows, macOS and Linux core/native build coverage while retaining deterministic, model-free default CI.
2. Validate the installed two-executable layout, Unicode paths, helper identity, CPU instruction baseline and missing-helper recovery.
3. Perform real microphone tests on each advertised OS/architecture, including granted/denied permission and disconnect recovery.
4. Benchmark base/tiny/small against the same short-prompt corpus with identical settings and recorded hardware; publish cold/warm timings and memory, not universal claims.
5. Test air-gapped runtime and manual model import. Audit release dependencies and notices, including native/system components.
6. Document supported modes, model sizes, setup Internet requirements, permissions, Ctrl+T migration, mouse selection and explicit-send behavior.

**Dependencies:** Phase 4 acceptance and complete model/source metadata.

**Tests/verification:** Commands in Section 14, installed-binary manual matrix, network-blocked recognition, license/dependency inspection, release archive smoke tests.

**Completion criteria:** Every platform advertised as supported has evidence for the actual artifact. Any untested target is labeled experimental rather than implicitly promised. Section 16 is satisfied.

## 14. Testing and verification plan

### 14.1 Automated test matrix

Keep Rust tests in the repository's inline `#[cfg(test)]` modules. Use trait-backed fake capture, engine, transport, process and clock implementations. Do not require a user's default microphone, account, model cache or Internet connection for `cargo test --workspace`.

| Test group | Required cases and assertions |
|---|---|
| Reducer | Every legal/illegal state transition, start while busy, double stop, max duration, cancel during prepare/capture/decode, timeout and terminal-outcome exclusivity |
| IDs and epochs | Old result after cancellation, duplicate completion, new-session result mismatch, modal transition, draft replacement, out-of-order acknowledgements |
| Editor | Empty draft; insertion at beginning/middle/end; live cursor moved during decode; text typed/deleted during decode; Unicode byte boundaries; multiline; existing paste marker; one dictation undo; empty result creates no undo |
| No-send contract | Completion does not call Model submit/queue, Flow Submit, on_line, extension emit_input or Agent prompt; assertions in idle and running loops |
| Send races | Enter during all active states, key repeats, custom send bindings, autocomplete Enter including `/model`, prequeued Enter at completion, post-insertion redraw guard |
| Keyboard | Ctrl+T canonical encodings, reported repeat/release filtering, debounce, Alt+T tool-output migration, explicit binding conflict, legacy/tree mappings unchanged |
| Mouse/layout | Same action as keyboard; wide/narrow/short terminal; short transcript not pinned to bottom; queued rows; suggestions/extensions/notices; clipped/hidden control; resize invalidation; outside/drag/release ignored |
| Focus and emergency control | Escape cancels voice before agent abort; Ctrl+C cancels voice and interrupts agent; double-Escape timer reset; permission overlay; exit/suspend restores terminal and closes mic |
| Audio | F32/I16/U16 conversion, channel downmix, 44.1/48 kHz to 16 kHz, resampler delay/tail, nonfinite values, silence/short clip, overflow, duration cap |
| Protocol/process | Frame bounds, truncated/invalid JSON, wrong protocol, pipe full, reader/writer failure, worker EOF/crash, forced termination, parent EOF, no zombies |
| Models/setup | Missing/corrupt/changed file, digest mismatch, interrupted download, TLS/redirect policy, disk-full, concurrent installs, atomic publication, offline import, no microphone after setup |
| Privacy | No audio files, no transcript logs, no runtime STT network calls, no provider credentials required, per-utterance state reset, environment minimization |
| Configuration | Absent defaults, invalid language/model/backend, explicit missing device, user-only merge, disabled voice and mouse-only opt-out |

### 14.2 Native recognition fixtures

Store only small, redistributable audio fixtures with license/source provenance. Prefer committed PCM fixtures so the selected runtime does not gain a codec dependency just for tests. Keep expected transcripts and normalization rules in the provenance/test data, including silence, background noise, very short input, English technical terms and at least one additional language.

Native model-backed tests are opt-in/ignored by default and accept an explicitly provisioned approved model path. They must not download a model when absent. Test no-speech handling and technical-name errors as well as clean speech; perfect transcription is not an acceptance claim. Use the same clips/decoding settings to compare tiny/base/small, and record accuracy and latency separately.

### 14.3 Proposed verification commands

Run these during implementation from the repository root, not as part of this document-only task. Use the exact pinned toolchain and committed lockfile once dependency resolution is complete.

```text
cargo +1.83.0 fmt --check
cargo +1.83.0 test -p davinci-voice --no-default-features --locked
cargo +1.83.0 test -p davinci-tui --locked
cargo +1.83.0 test -p davinci-coding-agent --locked
cargo +1.83.0 test --workspace --locked
cargo +1.83.0 clippy --workspace --all-targets --locked -- -D warnings
cargo +1.83.0 build -p davinci-voice --features native --bin davinci-voice-worker --locked
cargo +1.83.0 test -p davinci-voice --features native --locked
```

Retain the repository's existing ecosystem/runtime regression commands in CI. Run model-backed ignored tests only in an explicitly provisioned offline job. Run equivalent native build/install smoke tests on each advertised platform; a Linux pass does not establish Windows or macOS microphone correctness. Add native address/undefined-behavior sanitizer runs separately from ordinary Rust tests where supported.

### 14.4 Responsiveness and resource budgets

The following are **engineering targets to measure**, not verified performance figures:

- Keyboard/caret feedback remains below 100 ms at the 95th percentile during inference on the reference machine.
- An ordinary stop closes capture within 500 ms; cancellation has a one-second forced-termination bound, with the UI responsive throughout.
- For an initial reference CPU class of four logical cores and 8 GB RAM, measure a 15-second utterance with warm base. A useful initial target is stop-to-insert within ten seconds; report actual results and recommend explicit tiny selection on slower hardware.
- Capture PCM is capped at 7.32 MiB plus the bounded ring/resampler state; no unbounded status or sample queues.
- Repeated sessions return to a stable memory/handle baseline apart from the intentionally retained model weights.

Measure cold model load separately from warm inference. Include long pauses, Bluetooth/USB devices, background CPU work and simultaneous agent output. Do not silently downgrade the model to meet a benchmark.

### 14.5 Manual acceptance script

On each supported platform, begin with no speech model and a nonempty unsent draft. Open setup, cancel it, install/import the model, and confirm nothing recorded or sent. Start by keyboard, speak, stop by mic click, and confirm the words appear at the live cursor without a new chat turn. Edit and undo them. Repeat starting by click and stopping by keyboard.

Then dictate while an agent turn runs; press Enter during capture and processing and confirm no new queued message. Cancel after typing additional text and verify every typed character survives. Open a permission/modal surface during dictation, disconnect the microphone, deny permission, kill the helper, resize to a narrow terminal and repeat with mouse disabled. Finally block networking and repeat recognition, then explicitly send once to confirm the normal send path still works.

## 15. Risks and tradeoffs

| Risk / tradeoff | Mitigation / decision |
|---|---|
| Native pre-release pin | Explicitly identified; require safety-fix, sanitizer and platform gates. Prefer a reviewed stable successor through the same adapter, not a moving version. |
| Old capture/resampling pins for Rust 1.83 | Verify packaged metadata and full lockfile; track maintenance/security updates. Do not hide a toolchain upgrade in this feature. |
| Maintaining a C ABI shim | Keep it small and opaque; pin native source; test ownership, cancellation and invalid inputs; no broad autogenerated API surface. |
| Two executable distribution | Bundle/version together; trusted sibling lookup and handshake; test installed layouts rather than only Cargo runs. |
| CPU latency and model accuracy | Default base, explicit tiny/small choices, measured cold/warm benchmarks, no unsupported universal accuracy promise. |
| No true streaming text | Intentional first release: one atomic result avoids unstable partial edits and complicated reconciliation. |
| Live-caret insertion can surprise someone who moved the cursor | Document it clearly; keep one undo step. It is preferable here to freezing editing or implementing a text-anchor transformation engine. |
| Silence/noise hallucinations | Conservative no-speech handling, short-clip safeguards, fixture testing and mandatory user review; never auto-send. |
| Mouse reporting affects native selection | Small targeted control, lifecycle cleanup and `voice.mouse=false`; do not pretend there is no terminal tradeoff. |
| Ctrl+T already occupied | Context-aware default migration, explicit override preservation, effective-map help and busy-loop tests. |
| Modal/agent-loop complexity | One controller and common preflight, ID/epoch validation, tests in both loops and permission handlers. |
| Model supply chain and malformed native input | Approved immutable catalog, hash/length verification, no executable checkpoints, isolated worker and native regression tests. |
| Audio privacy cannot guarantee memory erasure | RAM-only policy, bounded buffers, best-effort zeroization, no logs/files; disclose OS/native memory limits. |
| GPU convenience versus strict openness | CPU default; do not bundle proprietary accelerator requirements. Optional audited open-driver backend is later work, not a completion dependency. |
| Concurrent repository edits | Use symbol anchors, re-inspect before implementation, preserve existing modifications and avoid unrelated refactors. |

Material assumptions that remain to validate are hardware performance, the exact native/crate build on all target OS versions, permission attribution for the packaged macOS helper, terminal-specific mouse selection, and model artifact metadata. None changes the core editor-insertion architecture. Do not ask the user questions that these build, repository or hardware checks can answer.

## 16. Definition of done

The feature is ready only when all required items below have evidence:

- [ ] Ctrl+T and mic click dispatch the same action and reducer; no separate microphone implementation exists.
- [ ] A visible mic sits beside the existing composer without reducing editable text width or using guessed viewport coordinates.
- [ ] CPU-only local dictation works with the approved default model and no STT/API credentials.
- [ ] Recognition still works with networking blocked after explicit setup/import.
- [ ] Final text is inserted once at the live cursor through the existing editor and is immediately editable, including multiline/Unicode cases.
- [ ] One undo removes the dictation transaction without losing previous typed text or paste state.
- [ ] Dictation completion, timeout, max-duration stop, cancellation and errors cannot submit, queue or execute any message/command.
- [ ] The no-send invariant holds during agent generation, autocomplete, extension routing and completion/Enter races.
- [ ] Cancellation, failure, modal transition, session replacement and stale results preserve the current typed draft.
- [ ] Recording indication reflects actual capture state, and capture closes on stop, cancel, disable, exit and parent loss.
- [ ] UI responsiveness, bounded buffers, worker reaping and repeated-session resource behavior are measured.
- [ ] The existing Ctrl+T tool-output behavior has a documented replacement, explicit user mappings are preserved, and legacy/tree contexts remain unchanged.
- [ ] First-run setup uses explicit consent, validates model identity, supports local import, and never auto-records after installation.
- [ ] No audio file, transcription telemetry, cloud speech call or paid dependency is introduced by the selected path.
- [ ] Exact native/crate/model licenses, source revisions, hashes and redistribution notices are recorded and audited.
- [ ] Release archives install the matching helper, and every advertised platform passes installed-binary and real-microphone verification.
- [ ] Existing workspace tests and relevant ecosystem/input regressions pass; default CI remains deterministic and microphone/model-free.
- [ ] Documentation distinguishes setup downloads, offline voice runtime and the normal harness's post-send provider behavior.
- [ ] Unsupported modes and unverified platforms are labeled accurately; unrelated harness behavior has not been redesigned.

### Evidence status at plan completion

**Verified by repository inspection:** Rust/TUI/editor architecture, the existing atomic insertion method, two live input loops, current submission and queue paths, shortcut collisions, absence of default mic/mouse infrastructure, model-directory resolution, and current test/CI organization.

**Verified from primary sources:** The selected engine's local CPU capability and MIT license, original Whisper weight license, native PCM/cancellation API, CPAL 0.16 license/MSRV/platform dependencies, alternative-project capabilities and model-specific license cautions, and relevant release/build metadata. Sources are listed below.

**Not claimed as verified:** A native build in this checkout, real microphone behavior, task-specific transcription accuracy, timing/RSS budgets, every transitive dependency's final resolved compatibility, or a complete immutable model catalog. These are explicit implementation/release gates. This document does not represent a passing build or an implemented feature.

### Primary-source ledger

All external sources were consulted for this plan on 2026-09-06. Links point to upstream projects, published crate sources/model cards or official platform documentation. A mutable upstream page is research evidence, not a substitute for the immutable pins required in the implementation.

[S1]: https://github.com/ggml-org/whisper.cpp "whisper.cpp: local implementation, supported platforms, models and resource guidance"
[S2]: https://github.com/openai/whisper "Original Whisper repository: model families and code/weight MIT statement"
[S3]: https://raw.githubusercontent.com/ggml-org/whisper.cpp/v1.9.3/include/whisper.h "Pinned candidate C API: PCM, state ownership, callbacks and decoding parameters"
[S4]: https://raw.githubusercontent.com/RustAudio/cpal/v0.16.0/Cargo.toml "CPAL 0.16.0 manifest: Apache-2.0, Rust 1.70, target dependencies"
[S5]: https://github.com/SYSTRAN/faster-whisper "faster-whisper: local inference, CPU INT8, Python/CTranslate2 and benchmark qualifications"
[S6]: https://github.com/k2-fsa/sherpa-onnx "sherpa-onnx: streaming/offline recognizers, bindings and Apache-2.0"
[S7]: https://huggingface.co/moonshine-ai/moonshine-tiny "Original Moonshine Tiny model card and MIT model license"
[S8]: https://github.com/alphacep/vosk-api "Vosk API: offline streaming recognition and engine license"
[S9]: https://alphacephei.com/vosk/models "Vosk model sizes, memory guidance and per-model licenses"
[S10]: https://github.com/ggml-org/whisper.cpp/releases "Release status, v1.9.3 candidate and short-audio/model-header fixes"
[S11]: https://github.com/tazz4843/whisper-rs "Archived GitHub mirror, migration notice and Unlicense"
[S12]: https://docs.rs/crate/whisper-rs/0.16.0 "Published whisper-rs crate version and native binding dependency"
[S13]: https://docs.rs/crate/cpal/0.18.2/source/Cargo.toml.orig "Newer CPAL toolchain requirements; do not substitute for 0.16 APIs"
[S14]: https://github.com/HEnquist/rubato "Rubato project licensing, resampling/callback guidance and version/API differences"
[S15]: https://docs.rs/crate/ringbuf/0.4.8 "ringbuf 0.4.8: SPSC queue and package license"
[S16]: https://docs.rs/crate/cmake/0.1.58/source/Cargo.toml "cmake Rust helper manifest: version, license and minimum Rust"
[S17]: https://raw.githubusercontent.com/ggml-org/whisper.cpp/v1.9.3/CMakeLists.txt "whisper.cpp build options and optional components"
[S18]: https://raw.githubusercontent.com/ggml-org/whisper.cpp/v1.9.3/ggml/CMakeLists.txt "GGML CPU/ISA/backend build defaults"
[S19]: https://huggingface.co/ggerganov/whisper.cpp "Upstream-distributed converted Whisper model repository; pin exact artifacts during implementation"
[S20]: https://support.microsoft.com/en-us/windows/privacy/turn-on-app-permissions-for-your-microphone-in-windows "Official Windows microphone and desktop-app permission guidance"
[S21]: https://developer.apple.com/documentation/bundleresources/information-property-list/nsmicrophoneusagedescription "Apple microphone purpose-string documentation"
[S22]: https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input "Apple audio-input entitlement documentation"

**End of engineering plan.**
