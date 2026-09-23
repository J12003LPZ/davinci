# Optional operation fuzzing

This repository did not contain a fuzz workspace when the unified execution
recovery work began, and the stable workspace has no `libfuzzer-sys` or
property-testing dependency. Task 21 therefore keeps the required gate
deterministic and offline: `operation_properties.rs` mutates bounded,
serialized `RecoveryInput` bytes and reduces every input that still parses.
The test records the seed and mutation sequence in the system temporary
directory if a panic or invariant failure occurs.

An optional nightly campaign can be added here without changing the product
workspace. Keep its manifest and toolchain file under `fuzz/`, pin every
dependency after checking the repository MSRV, and keep corpus entries under
`fuzz/corpus/`. Suggested targets are a bounded operation-record parser and
the recovery reducer. A short campaign is exploratory evidence only; stable
gates remain the deterministic property test and the process-kill crash
matrix.
