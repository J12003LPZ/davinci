# Verification evidence

The classifier recognizes a bounded subset of Python syntax and literal shell
commands. It does not execute inspected source, grant tool permissions, or prove
that a test suite is correct. Terminal process evidence is recorded separately
and must belong to the generation captured immediately before execution.

## Scope and failure recovery

Fresh targeted checks accumulate coverage of outstanding changed paths. A failed
scope stays failed when another scope passes; only an applicable fresh pass
clears it. Another mutation invalidates accumulated passes. An earlier failure
remains unresolved after an edit, but the harness may rerun that stale check once
for the new generation and report its actual output. A successful rerun of another
scope is never described as a failed command. Incomplete inventories are coalesced
once earlier evidence is stale, and the rerun's resulting generation bounds its
reminder so uncertainty cannot cause an automatic verification loop.

An unfiltered suite at the canonical workspace root can recover unknown mutation
paths. A targeted check, filtered suite, or suite in a subdirectory cannot claim
that recovery. Normalized paths resolve existing ancestors so deleted inputs,
symlinked working directories, and Windows path aliases keep their identity.
Source under `docs`, `tests`, or `fixtures` still needs coverage; documentation
exemptions apply only to document types inside the workspace.

Shell commands receive before/after content fingerprints of workspace inputs.
The inventory is limited to 16,384 entries, 64 MiB read, and 100 ms per snapshot.
Explicit outstanding inputs are read first, including inputs inside excluded
top-level build/runtime directories. Nested directories with the same names are
still scanned. File symlinks retain their link identity and fingerprint the
resolved target bytes; unreadable targets and special files make the observation
incomplete. Python bytecode caches are excluded.

Observed changes during a command advance the generation and invalidate that
command's result. An incomplete inventory invalidates previous evidence before
dispatch. A fresh unfiltered suite may then cover unknown scope if every known
input was observed unchanged; a targeted script cannot make that claim. This
conservative inventory is not continuous filesystem monitoring: a complete suite
remains an explicit project-wide coverage claim, and unobserved arbitrary writes
outside the workspace cannot be inferred from it. Host-managed background shells
keep evidence pending until their terminal job state, followed by a fresh check.

## Supported syntax

Literal `python -c` source (including quoted newlines), versioned interpreter
names, and quoted Bash heredocs are supported. The Bash tool retains heredoc
semantics on Windows. PowerShell has separate literal quoting and backslash
handling; Bash heredocs and `&&` remain inconclusive because PowerShell 5.1
does not support them.

Python support includes concrete comparisons, checked return values, literal
file reads/existence checks, imported package submodules, nonempty case tables,
bounded local check functions, selected unittest assertions, and concrete
expected-exception checks including bare `raise AssertionError`.

Obvious no-ops remain inconclusive: imported-symbol truthiness, structural
existence/type checks, constant container truthiness, identical comparisons,
disjunctions that can bypass a check, uncaught successful exits before an
assertion, dynamic execution, swallowed assertions, and unsupported control
flow. Unsupported syntax can be exercised by a normal suite instead.

The isolated helper receives JSON source on stdin under `-I -S`. Source is capped
at 64 KiB, output at 16 KiB, and cached syntax facts at 128 entries keyed by source
and interpreter identity. A timed-out helper is killed and reaped after a
250 ms budget on Unix or a 1 s budget on Windows, where process startup is
materially slower; timeout results are not cached. Path resolution and successful
verification evidence are never cached with syntax facts.

Regression cases live in `test_python_ast.py`, `inline_verification.rs`, the
`verification` Rust unit modules, and agent lifecycle tests in `lib_tests.rs`.
`recorded_checks.json` supplies the real behavioral-check acceptance corpus.
