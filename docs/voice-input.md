# Local voice input (experimental)

Local voice is available in the default Davinci terminal composer, including
while an agent turn runs. It inserts a single editable result at the **live
cursor**. It never sends, queues or executes that result. Review it and press
Enter separately. One editor undo removes the insertion.

Use **Ctrl+T** or the mic on the upper composer rule to start, and either again
to stop. Escape cancels dictation first; Ctrl+C also retains agent interruption.
Typing, cursor movement and paste remain available. Recording stops at 120
seconds. Enter is blocked during capture/processing and briefly after insertion
until the draft has been drawn. Opening a modal or replacing a session/draft
invalidates a pending result.

## Install and setup

Build/install the matching `davinci` and `davinci-voice-worker` from the same
checkout using `scripts/install.sh` or `scripts/install-davinci.ps1`. Native
builds need CMake and a C++17 compiler; Linux also needs ALSA development headers
and pkg-config. Ordinary workspace builds/tests do not build the native helper.
The helper is resolved beside the running executable, never searched on PATH.

For a faster local CPU build, set `DAVINCI_VOICE_NATIVE_CPU=1` before building
the native worker or running the install script. In PowerShell:

```powershell
$env:DAVINCI_VOICE_NATIVE_CPU = '1'
cargo build --release -p davinci-voice --features native --bin davinci-voice-worker --locked
```

This enables whisper.cpp's host CPU instruction detection (including SIMD).
Use that helper only on the machine where it was built; it is not a portable
release artifact. Unset the variable or set it to `0` for portable builds.
Cross compilation with this option is rejected. The model, language selection,
thread limit and editable-text-only behavior are unchanged.

The first activation with no model opens a setup screen. Choose Download to
consent to Internet access, Import to copy an approved existing file, or Cancel.
The command palette also has **Local voice setup**. Setup never starts recording.
Provider credentials are unnecessary for these early CLI commands:

```text
davinci voice status
davinci voice devices
davinci voice model list
davinci voice model install base
davinci voice model import base /path/to/ggml-base.bin
```

Download is an explicit network operation. `--offline` disables it; local import
still works. Imports and downloads verify exact size and SHA-256 before atomic
publication. Models live under the resolved user agent directory at
`voice/models/<id>/<sha256>.bin` (normally `~/.davinci/agent`, with legacy resolution
retained). A stale `install.lock` requires inspection before removal. A corrupt
published model must be explicitly removed before a replacement is installed.

## User configuration and shortcuts

Only the `voice` object in **user** `settings.json` is read. Project settings
cannot select devices or models. Other settings are preserved. Defaults:

```json
{"voice":{"enabled":true,"mouse":true,"model":"base","language":"auto","inputDevice":null,"backend":"cpu"}}
```

Models are multilingual `tiny`, `base` (default), or `small`. Language accepts
`auto` or a Whisper language code such as `en` or `es`. Device names come from
`voice devices`; a missing or ambiguous configured name fails without switching
to another microphone. Restart after changing settings.

The composer defaults to `davinci.voice.toggle = ctrl+t` and moves
`davinci.tools.expand` to `alt+t`. Explicit mappings are preserved. A conflicting
voice mapping is disabled with a diagnostic; select a unique binding or use the
mic. Legacy thinking and session-tree mappings retain their meaning. Disabling
voice restores the old tools default. Mic labels use the effective voice key.

Mouse reporting can affect native terminal selection; use Shift-selection where
your terminal supports it or set `voice.mouse=false`. Keyboard voice still works.
The mic is hidden on modal or too-small surfaces; active capture is cancelled if
its control cannot be displayed.

## Privacy and limitations

Recognition is local CPU inference with at most four threads, leaving one
logical core when available. Successful sessions retain model weights for up to
five idle minutes; decoding state is fresh for every utterance. The helper
receives a minimal OS/audio environment, not provider credentials. PCM remains
in bounded worker memory, never in audio files or host IPC. Text is not logged
by this path. Zeroization is best effort: OS/native allocations cannot provide a
complete memory-erasure guarantee. Normal harness/provider behavior begins only
when you explicitly submit the edited draft.

The worker protocol has 64 KiB frames and bounded channels. Capture uses a bounded
ring, 16 kHz mono PCM and a 120-second cap. Cancellation requests have a 750 ms
kill escalation; errors preserve typed text. Model setup cancellation may wait
for the current network read timeout, while the terminal stays responsive.

Print, JSON, RPC and legacy TUI modes do not provide interactive dictation.
Windows native compilation and deterministic tests have been exercised locally;
real microphone behavior, network-blocked recognition, installed artifacts,
macOS/Linux audio permissions and performance budgets remain acceptance gates.
All platforms are **experimental** until those artifact-specific checks pass.
The CI matrix adds native compilation/tests but does not prove microphone access.

## Immutable dependencies and models

whisper.cpp `v1.9.3`, peeled commit
`371b5a7561823ab2bb32142d2751e35e7534727b`, is vendored under
`crates/davinci-voice/native/upstream` with its MIT license. The codeload archive
SHA-256 is `89051d8fca516a3ad1f5c2f8f9d2fccb089afbaec338fca3f8731999babc6f81`.
The adapter enables generic CPU inference and disables GPU, BLAS, OpenMP,
native-ISA tuning, examples, server and curl integration. See `native/CMakeLists.txt`.

| Direct native dependency | Pin | License | Declared Rust minimum |
|---|---|---|---|
| CPAL | 0.16.0 | Apache-2.0 | 1.70 |
| rubato | 0.16.2 | MIT | 1.61 |
| ringbuf | 0.4.8 | MIT OR Apache-2.0 | not declared |
| cmake build helper | 0.1.58 | MIT OR Apache-2.0 | 1.65 |

Model metadata is pinned to ggerganov/whisper.cpp Hugging Face revision
`5359861c739e955e79d9a303bcbc70fb988958b1`. Original Whisper weights are MIT licensed.
Exact converted artifact identities from that revision's LFS metadata:

| Model | Bytes | SHA-256 |
|---|---:|---|
| tiny | 77,691,713 | `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21` |
| base | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |
| small | 487,601,967 | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` |

Upstream licenses are retained with vendored sources. A complete transitive and
platform/system redistribution audit, sanitizer run and model accuracy benchmark
are still required before a supported release. No universal speed/accuracy claim
is made. The opt-in engine fixture test documents explicit model provisioning in
`crates/davinci-voice/fixtures/README.md`.

## Local fixture measurements, 2026-09-06

Windows x86-64, AMD Ryzen 7 9800X3D (8 cores / 16 logical processors), four
inference threads, optimized generic CPU native build. Each model used the same
11-second English JFK fixture and greedy settings. Load includes SHA-256
verification; decoding excludes load. English runs first, then automatic
language selection, with fresh decoding state and retained model weights.

| Model | Cold load | English decode | Auto-language decode | Peak process working set |
|---|---:|---:|---:|---:|
| tiny | 2.03 s | 2.45 s | 5.25 s | 165 MiB |
| base | 3.94 s | 7.26 s | 11.96 s | 297 MiB |
| small | 12.69 s | 25.59 s | 48.34 s | 945 MiB |

All three passed the expected phrase check and pre-cancelled inference check.
These are single-run fixture measurements, not microphone, multilingual accuracy,
UI p95 latency or repeated-session leak measurements. The automatic base result
does not establish the plan's ten-second target for a 15-second utterance on a
four-core reference machine. Explicitly choose tiny if latency matters more than
model size. No automatic downgrade occurs.

### Host CPU tuning comparison, 2026-09-07

On the same Ryzen 7 9800X3D, the base model and 11-second fixture were run
before and after enabling `DAVINCI_VOICE_NATIVE_CPU=1`, with four threads:

| Build | Cold load | English decode | Auto-language decode |
|---|---:|---:|---:|
| Portable CPU | 3.97 s | 5.44 s | 10.84 s |
| Host-tuned CPU | 3.55 s | 0.57 s | 1.06 s |

Both passed the expected phrase and cancellation checks; all 15 native voice
tests passed with tuning enabled. These are single-run offline measurements,
excluding microphone capture and terminal delivery. Cold model loading still
adds latency on the first use or after the idle worker expires.
