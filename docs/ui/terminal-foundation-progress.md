# Terminal rebuild: implementation evidence

## Scope of this batch

Branch: `J12003LPZ/claude-terminal-implementation-20260921`.
Created from `778c719a41bc2c64ddfae902c02a1ac4c03ef90b` on the earlier rebuild branch. Neither the earlier branch nor `main` was modified by this implementation session.

Status: the shared terminal foundation is implemented and its TUI tests pass. **The approved whole-terminal Claude Code parity project is not complete.** This report is not a substitute for its specification or implementation plan.

Implemented source changes:

- Neutral built-in dark/light surfaces with separate selection, success, warning and error colors. Native and regular renderer built-in palettes now share one source of truth. Explicit Vox and custom theme loading remain supported; existing users who explicitly selected Vox will still see that optional palette. No settings, credentials, permissions or telemetry preferences are rewritten.
- Shared headings preserve their supplied case and no longer add opaque paper backgrounds or decorative edges. Shared prompt separators use a straight rule.
- The welcome banner now uses three compact rows containing the actual product/version, model/effort and workspace rather than the old block-letter masthead. Other welcome content is retained for the later layout pass.
- Ctrl+U edits to the beginning of the current input line through the existing Editor. The usage-view default moves to Alt+U; explicit user keybinding overrides remain respected. Ctrl+Y can restore the removed text.
- Clipping and long-word handling retain complete Unicode graphemes. Styled-run width accounting saturates safely instead of wrapping a 16-bit counter; truncation still enforces its actual cell budget for very long output.
- Seven new regressions exercise editing/yank, view dismissal and draft/caret retention, custom bindings, oversized styled runs, joined emoji, renderer palette consistency and compact banner bounds.

## Verification performed

The baseline was exercised before product changes. Its six rebuild contracts had five failures (the old palette, headings, rules, joined-emoji clipping and Ctrl+U ownership) and one pass. The baseline's existing TUI unit suite passed.

Candidate revision: `f6c56bcae74a53a7f33ec7971823dc449e964cef`.

Verified GitHub Actions run:
https://github.com/J12003LPZ/davinci/actions/runs/35562565619

All of the following completed successfully on Ubuntu with repository-pinned Rust 1.83.0 and `PI_OFFLINE=1`:

```sh
cargo test --locked -p davinci-tui --test terminal_rebuild -- --nocapture
cargo test --locked -p davinci-tui
cargo clippy --locked -p davinci-tui --all-targets -- -D warnings
```

The first command passes all six rebuild contracts. The full suite includes the seven new `terminal_foundation` regressions. Candidate Rust files were formatted with rustfmt 1.83.0 before testing, and `git diff --check` passed. Existing tests that explicitly required uppercase headings or textured rules were updated to the requested ordinary-case/plain-rule presentation; their focus, visibility, selection and layout checks were retained. No tests were disabled to make this batch pass.

The workflow published tested Git objects without changing any branch reference. Its resulting source tree was `8b989337b68f65e43645990c9780d8feaca76dc4`; each of the ten source/test file SHA-256 digests and Git blob IDs was independently checked against the inspected local files. The final implementation commit stores those ordinary source files and removes the temporary candidate patches. This evidence document is added alongside the tested source without changing it.

A local library build and direct execution of all thirteen integration regressions also succeeded using the Rust toolchain and dependency cache obtained through the authenticated GitHub artifact connector. The offline preview was rendered from actual Ratatui buffers with fixture data; it was not generated as an image of a hypothetical application. No model calls or microphone capture were used.

## Gates still open

The current colors and layouts are implementation work, **not measured proof of Claude Code v2.1.278 parity**. Pinned-version cell/style captures, full action-trace comparison and native Windows Terminal/ConPTY review have not been completed. The complete workspace/host regression suite and an independent reviewer have not been certified by this report; the verified scope here is `davinci-tui`.

Settings, model selection, tools/transcript details, authentication/permissions, sessions/diffs, utility screens and nested dialogs still require their full reference-layout/interaction migration. Their use of the new shared presentation does not make those screens finished. Background-task target isolation and the approved optional graph with usable input remain subsequent implementation work. There is no new claim of concurrent-task safety from this batch.

No application was installed, no running agent was interrupted, and no service was restarted. This is an isolated development branch, not a release or an instruction to replace a running harness.
