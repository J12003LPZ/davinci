# Ecosystem verification, 2026-09-23

Live end-to-end check of the graph, token governor, vector memory, and
language intelligence on Windows 11, against `origin/main` at `a9887dbe`.
Model: `openai-codex/gpt-5.6-luna`. Embeddings: local Ollama with
`embeddinggemma` (768 dimensions). Language server: typescript-language-server
5.3.0. Each live check ran in a throwaway repository, not in a user project.

## Results before fixes

| Area | Check | Result |
| --- | --- | --- |
| Offline suites | vector memory, token governor, language intelligence, graph, ecosystem, learning, security scan (881 tests) | Pass |
| Token governor | 4,000-line shell output, then ask for one line | Pass. Output cut to a 1.1 KB digest; the model called `retrieve_output` and got the exact line. |
| Vector memory | State a fact, then ask in a new session with no history | Pass. The model answered from injected memory. |
| Vector memory | Store contents after that turn | **Defect.** The promoted copy of a chunk had the same id as its source and never got an embedding. |
| Vector memory | `/memory-search` output | **Defect.** Each hit carried its full 768-float vector; one search printed about 50 KB. |
| Parallel reads | Three `read` calls in one message, in a session with a session file | **Defect.** Two of three failed with `journal writer is busy`. |
| Graph, simple route | Add a function and a test | **Fail.** Writer reads failed (as above); one rejected edit then blocked every later edit; the retry exited at start; run blocked after 40 s. |
| Workspace after that run | Any edit, from a new session | **Fail.** `unresolved effect ... holds resource claim *` on every mutation. |
| Graph checkpoint size | `state.json` of that run | **Defect.** 14.5 MB: the baseline inlined `.davinci/operations/operations.sqlite3` and its WAL, twice. |
| Language intelligence | Real TypeScript server test, 10 runs | **Flaky.** 1 of 10 runs reported no diagnostics for a file with a type error. |

## Causes and fixes

1. **Journal writer contention** (`davinci-agent/src/runtime/operations/store.rs`).
   The journal took its lock with `try_lock` and failed at once, while the
   scheduler runs up to eight read-class calls on parallel threads. The lock
   now waits up to 2 s before `WriterBusy`. The tool itself never runs under
   the lock, so the wait covers only short journal transactions.
2. **Workspace lock after a tool error** (same file). A tool that returns an
   error is recorded as `Failed` with effect `Unknown`, and only `Succeeded`,
   known-no-effect and cancelled attempts released their resource claims. The
   claim on `*` stayed forever. A returned error now releases the claim.
   `Unknown` still blocks automatic replay. An attempt that never completed
   (a crash mid-effect) keeps its claim for the recovery path.
3. **Graph baseline captured harness state**
   (`native_extensions/graph/mutation.rs`). `.davinci/operations/`,
   `.pi/operations/` and both vector-memory stores are now excluded, like
   `.davinci/graph/`.
4. **Memory id collision** (`native_extensions/vector_memory.rs`). The record
   kind is now part of the id seed. `/memory-reindex` gives existing
   duplicates their own id and embeds up to 256 records without a vector
   (previously it only reloaded the file, and `rebuild_local_embeddings` was
   unused).
5. **Search output** now omits embedding vectors.
6. **Diagnostics race** (`native_extensions/language_intelligence/session.rs`).
   typescript-language-server publishes syntax diagnostics before type
   errors. The tool now keeps the newest publication for a document version
   until the server has been quiet for 400 ms.
7. **Clippy on Windows**: a needless `return` in Windows-only code in
   `graph/worker_sessions.rs` failed `make clippy` on Windows.

Each fix has a regression test that failed before the change.

## Results after fixes

| Check | Result |
| --- | --- |
| Graph, simple route | Done in 60 s. Correct code, 2 of 2 tests pass, `state.json` 28 KB. |
| Graph, `--complex` | Done in 130 s: classifier, 2 researchers, test analyzer, planner, writer, 3 reviewer passes. Review `approve` with every diff chunk covered. 4 of 4 tests pass. |
| Real TypeScript server test | 5 of 5 pass. First definition about 730 ms, warm hover about 7 ms. |
| `/memory-reindex` on the store written by the old build | 1 id repaired, 1 record embedded, lag 0. |
| `/memory-page` | Verdict `connected`, retrieval check used vector similarity. |

## New: `/memory-page`

Writes `.davinci/vector-memory/memory-page.html` and opens it. See
[`vector-memory.md`](vector-memory.md).

## Open items (not fixed here)

- **Workspaces locked by a build from before this fix stay locked.** The fix
  prevents new locks; it does not release claims already written. Releasing
  them automatically needs a recovery decision per claim. Workaround, with no
  davinci session running in that workspace: move `.davinci/operations/`
  aside.
- **Graph retry after a mid-turn exit.** The second writer attempt printed
  `rejected observe event: ... InvalidTransition { state: InTurn, event:
  "turn_started" }` and exited in 2.6 s with no model call. It did not recur
  once the causes above were fixed, and it was not reproduced on its own.
- **`.davinci-transactions/`** is created at the repository root by graph
  runs and left with `active.lock` after the run ends. It is not under
  `.davinci/` and shows in `git status`.
- **Memory quality.** Every assistant reply is stored as a `decision` with
  importance 0.8, including replies like `OK` or `4`.
- **Legacy store shadowing.** When `.davinci/vector-memory/` exists, records
  in `.pi/vector-memory/` are not read. In this repository that is 89 records.
  `/memory-page` reports it; nothing migrates them.
- **`/memory-status`** reports `denseAvailable: true` without contacting
  Ollama, and lists a Qdrant URL although remote projection is disabled.
  `/memory-page` probes the server instead.
- **Language intelligence covers TypeScript and JavaScript only.** There is no
  Rust, Python or Go server support.
