# Named-file context

When a user request names a file, Davinci reads it at the start of the turn
and puts its contents in that turn's runtime state. The model can then edit
or reason about the file in its first response instead of spending a model
round trip on `ls`, `find` and `read`.

This has no TypeScript `pi` counterpart. It is on by default. Turn it off with
the `namedFileContext` setting or `DAVINCI_NAMED_FILES=0` (`false` and `off`
work too).

## Why

In the 2026-09-25 head-to-head against Codex CLI (8 tasks, 3 repetitions),
21 of 24 prompts named the file to change. No Davinci run read that file in
its first request. Request 1 was always discovery (`ls`, `find` for the named
file, `grep`), and the file was read in request 2. Each request costs about
5 seconds, and the median run made 7.

Prompt guidance does not fix this. The G1 preview profile told the model to
"read named files directly", and the first request still read a named file in
0 of 24 runs ([G1](../perf/request-efficiency/G1.md)). So the harness does the
read itself.

The expected effect is about one request fewer in runs whose prompt names a
file. That estimate has not been measured yet. The benchmark gate for it is:
pass rate at least the parent's, total requests and requests after the first
edit not above the parent's, and the share of runs whose first request edits
or greps instead of discovering.

## What is attached

`prompt::named_files::candidate_paths` takes path-like words from the message:
a word with a directory separator or a file extension. It strips quotes,
backticks, brackets, trailing punctuation, a leading `./` or `@`, and a
`:line` or `:line:column` suffix. It skips URLs, flags, `~` paths and any
word containing `..`. It considers at most 64 words.

Each candidate resolves against the working directory. A bare name such as
`pricing.py` that is not at the root is looked up among at most 2,000
workspace files (same ignore rules as the native `find`), and it is attached
only when exactly one file has that name.

A file is attached only if all of these hold:

- It is a regular file inside the working directory after resolving symlinks.
- It is not a protected or credential path (`permission::is_sensitive_file_path`,
  which covers `.env*`, `.ssh`, `auth.json`, `.git` internals and similar).
- A `read` of it is allowed outright by the current permission policy, for
  both its workspace-relative and absolute spelling. A path that would ask,
  or that a deny rule, Plan Mode boundary or filesystem boundary refuses, is
  left out. The model can still request it through the normal gate.
- It is text (no NUL byte in the first 8 KB).

At most 3 files are named. A file over 8 KB, or one that would push the
attached total over 12 KB, is listed with its line and byte counts but not
attached. A file containing `</runtime_state>` or `</named_files>` is listed
but not attached, so it cannot close the enclosing sections.

## Format

```text
<named_files>
Files named in the user's request, read by the harness when this turn started. ...
----- BEGIN intervals.py (37 lines) [a1b2c3d4] -----
<file contents, verbatim>
----- END intervals.py [a1b2c3d4] -----
big.py (900 lines, 40000 bytes): not attached, read it when needed
</named_files>
```

Contents are verbatim so an `edit` can quote them exactly. The bracketed tag
is the first 4 bytes of the SHA-256 of the contents, so no line in the file
can forge its end marker.

## Lifetime and caching

The block is captured once per real user turn, next to the environment
snapshot, and frozen. Model and tool continuations reuse it, so the provider
input stays an append-only prefix and the block appears once per turn. A new
user turn recaptures it: a turn that names no file has no block. A session
switch or a custom replacement prompt clears it.

The contents are a snapshot from the start of the turn. The block says so and
tells the model to read a file again after it changes.

## Tests

- `crates/davinci-agent/src/prompt/named_files.rs`: candidate extraction,
  verbatim attachment, unique bare-name lookup, refusal of secrets, binaries,
  paths outside the workspace and denied paths, size budgets, and forged end
  markers.
- `crates/davinci-agent/tests/named_files.rs`: the block appears exactly once
  on both turn-context placements and across continuations, disappears on a
  turn that names no file, respects a `read(...)` deny rule, and is removed by
  the setting.
- `crates/davinci-coding-agent/src/settings.rs`: `namedFileContext` parsing and
  the environment override.
