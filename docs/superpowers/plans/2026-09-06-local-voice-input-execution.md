# Local voice implementation checkpoint — 2026-09-06

Plan: [local voice input](2026-09-06-local-voice-input.md).
Status: **experimental implementation; release definition of done is not satisfied**.
Work is uncommitted on `main`. Existing unrelated dirty work was preserved.
No subagents were used. Executable commands used RTK; Headroom was used for
focused source/diff inspection. No token-savings percentage is claimed.

## Implemented artifacts

- `crates/davinci-voice`: lightweight reducer, bounded protocol, immutable model
  catalog and normalization; optional CPAL/rubato/ringbuf native feature; pinned
  whisper.cpp source, optimized CPU C ABI adapter, isolated worker, cancellation
  watchdog, fresh decoding state and five-minute model retention.
- `voice_input.rs`, `voice_models.rs`, `davinci_interactive.rs`, `main.rs`: shared
  idle/running-loop controller, input-before-completion preflight, process reaping,
  modal/session/draft invalidation, global-only settings, early CLI, explicit
  download/import setup, byte progress, cancellation, verified atomic publication.
- TUI model/app/runtime/chrome/keybindings: live-caret one-undo insertion, no-send
  guard, capture-state mic label, shared frame/hit rectangle, mouse cleanup,
  explicit binding preservation, Ctrl+T / Alt+T migration, effective hotkeys.
- CI native matrix for Windows/macOS/Linux; two-executable installer wiring and
  license copies; `docs/voice-input.md`, UI specification, fixture provenance and
  Windows native dependency notice inventory.

## Phase audit

| Phase | Implemented / verified | Remaining acceptance work |
|---|---|---|
| 0: supply chain | Exact native commit/archive hash, three model hashes/sizes verified against real downloads, native Windows Rust 1.83 build, 48-package Windows license inventory | Other target dependency/license audit; realfft standalone notice gap; native sanitizer/security regression breadth |
| 1: deterministic UX | Reducer and fake host tests; idle/busy no-send, pending Enter, stale/duplicate completion, cancellation, epoch invalidation, insertion/undo, keymap and frame geometry | Interactive visual acceptance and complete adversarial input matrix |
| 2: native slice | Native capture/resampling adapter, bounded worker, actual approved tiny/base/small fixture recognition, fresh state/cancellation, Unicode helper-path handshake/EOF smoke checks | Real capture, permissions, disconnect and force-kill-during-capture proof |
| 3: setup/config | Global settings validation, CLI status/list/import, explicit setup UI, progress, size/hash checks, cancellation, lock/publication tests, temporary offline CLI import | Fake HTTP redirect/disk-full matrix, live setup UI and terminal selection checks |
| 4: hardening | Deadline/identity guards, no-send fake regressions, pipe bounds, parent EOF/version/oversize process checks, session and handoff invalidation | Full protocol fuzzing/saturation, actual permission/modal capture races, repeated-process handle/RSS stability, UI p95 and stop latency |
| 5: release | CI and installers authored, local fixture benchmark recorded, notices/docs present | CI matrix execution, installed/release artifact tests, macOS permission attribution, real mic on each platform, network-blocked recognition, additional-language/noise/technical-name corpus |

## Validation evidence

All ordinary tests used offline settings; none opened a microphone or downloaded
models. Native model tests were explicitly provisioned with temporary approved
downloads, never the normal user voice cache.

- Final `cargo test --workspace --locked`: 2,141 passed / one ignored (31 suites).
- Affected coding-agent/TUI suites after lifecycle/layout edits: 1,534 passed /
  one ignored. Focused final host/setup tests: seven passed.
- `cargo test -p davinci-voice --features native --locked`: 14 passed / one
  opt-in model test ignored. Lightweight default build remains CMake-free.
- Opt-in native engine test passed separately for tiny, base and small. Each
  verified model identity, English and automatic-language decoding of the same
  11-second fixture, fresh decoding state and pre-cancelled inference.
- Workspace formatting and Clippy with `--all-targets --locked -- -D warnings`
  passed. Native-feature Clippy also passed.
- Native helper and coding-agent debug builds passed on pinned Rust 1.83.0.
- Unicode-directory helper smoke: handshake exit 0 (0.138 s), version mismatch
  exit 1 (0.008 s), oversize frame exit 1 (0.008 s), parent EOF exit 0 (0.009 s).
  Every tested child was reaped; no capture was requested.
- `davinci --offline voice status`, `voice model list`, and offline base import
  into `%TEMP%/davinci-voice-import-check` passed. Normal cache remained empty.
- Final diff whitespace check and PowerShell installer syntax validation passed.
  Installer scripts and remote CI were not executed.

Native compilation used a temporary verified Kitware CMake 3.31.8 distribution
and local MSVC; CMake was not installed globally. The first model benchmark was
explicitly stopped after discovering that Cargo debug flags had disabled native
optimization despite a Release configuration. The build was corrected and all
reported successful model measurements use optimized native code. That stopped
run is not counted as a pass.

See [voice documentation](../../voice-input.md) for reproducible fixture inputs,
hardware, timings and peak working sets. Measurements do not establish the
four-core reference-machine or interactive latency targets.

## Delivery boundary

No commit, push, deployment, production installation, credential changes or
microphone activation occurred. Source and fixtures are ready for continued
review, but none of the outstanding gates above is silently marked complete.
The original plan's release checklist remains unchecked pending that evidence.
