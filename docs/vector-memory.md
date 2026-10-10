# Vector memory

Vector memory keeps short, typed claims the learning system verified in past
turns of a project, and brings a few of them back before a prompt only when
they clearly apply. It is a native Rust extension
(`crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`).

## Where it lives

- Records: `<project>/.davinci/vector-memory/records.jsonl` (the legacy
  `<project>/.pi/vector-memory/records.jsonl` is read only when no `.davinci`
  store exists).
- Configuration: `~/.davinci/agent/vector-memory.json`, overridden by
  `PI_MEMORY_*` environment variables.
- Embeddings: the Ollama server at `ollamaUrl` (default
  `http://127.0.0.1:11434`) with `embeddingModel` (default `embeddinggemma`,
  768 dimensions). Without Ollama, search falls back to keyword matching.

## What is stored

Settled turns are no longer indexed as transcript chunks. Durable records are
written when learning promotes a claim (`Constraint`, `Decision`, `Bug`, `Fix`,
`Discovery`, `Fact`, ...), compacted to at most 300 characters. When a claim
names a file inside the project (`src/auth.rs`), the file's path and content
hash are stored with it. Stores written by older versions keep their transcript
records; `memory_search` still finds them, but they are never injected
automatically.

## What is injected before a prompt

Automatic injection is precision-first and stays silent by default. It has two
parts, together at most 700 tokens (`maxInjectedTokens`):

- **Pinned constraints** (up to 15 records, 400 tokens): `Constraint` records
  learned with confidence of at least 0.80, and `Constraint` or `Decision`
  records the user confirmed. They are injected on every prompt, with no
  search.
- **Anchored recall** (up to 2 records, 300 tokens): a claim is injected only
  when all of these hold:
  - its hybrid score is at least 0.55;
  - the prompt and the claim share a code anchor: a path (`src/auth.rs`, or
    just `auth.rs`), a qualified or snake/camel-case name
    (`ContextVmRuntime::fold`, `graph_scheduler`), an error code (`E0382`) or
    a version (`1.83`). Edge punctuation never counts, so `startup.` at the end
    of a sentence is prose, not an anchor;
  - it leads the runner-up by at least 0.08, unless an anchor in the prompt
    matches the leader and not the runner-up.

These thresholds are fixed in code, not taken from `minimumScore`, which still
governs `memory_search` and graph worker context.

A claim whose source file has changed since it was learned is **stale**. It is
dropped from automatic injection and from graph worker context, but
`memory_search` still returns it. When learning derives the same claim again,
the claim is re-anchored to the file as it is now and becomes eligible again.

Graph workers receive a separate context packet: up to 4 records and 1,200
tokens by `minimumScore`, with no anchor requirement, because a worker goal is
a task description that rarely names code.

Each injected line is labelled with its kind and score, for example
`- [Constraint | score 0.95] Keep Rust 1.83 compatibility`.

## Setting it up

Run `/setup` in the project. It checks every feature that needs you to do
something before it works, fixes what it can, and lists the rest:

| Area | `/setup` does | You do |
| --- | --- | --- |
| Vector memory | Starts an installed local Ollama (`ollama serve`), pulls `embeddingModel` in the background, and embeds records that have no vector. | Install Ollama when it is missing. Fix `embeddingDimensions` when the model disagrees. |
| .gitignore | Adds `/.davinci/vector-memory/` and `/.davinci/graph/` to the project's `.gitignore` inside a Git work tree. | Nothing. |
| Project trust | Reports when project settings, hooks, skills or MCP servers are ignored because the project is not trusted. | Review them, then run `/setup trust`. Restart to load them. |
| Plugin hooks | Lists enabled plugins whose hooks wait for approval. | `/plugins approve <name>`. |
| Language servers | Detects Rust, TypeScript/JavaScript and Python projects and looks for their servers on `PATH` or in `node_modules/.bin`. | Install the server it names. Davinci never installs one. |
| Project instructions | Looks for `AGENTS.md` (`CLAUDE.md` is not read). | `/init`. |

`/setup check` reports without changing anything. The model pull continues
after the command returns, so run `/setup` again to follow it. Trust is never
granted by `/setup` alone, because a trusted project can run its own hooks,
extensions and MCP servers.

## Commands

| Command | What it does |
| --- | --- |
| `/setup [check\|trust]` | Sets up vector memory and the other features above. |
| `/memory-page [--no-open] [query]` | Writes `.davinci/vector-memory/memory-page.html` beside the records and opens it. |
| `/memory-status` | Opens the memory sheet with record counts and configuration. |
| `/memory-search <query>` | Runs the broad search (every kind, including stale claims) without the injection gate. |
| `/memory-reindex` | Reloads the store, gives records that share an id their own id, and embeds up to 256 records that have no vector. Run it again for more. |
| `/memory-clear` | Deletes the project's records. |

## The memory page

`/memory-page` probes the configured Ollama server when it runs and shows:

- **Connection:** whether Ollama answers, whether the embedding model is
  installed, a test embedding with its dimensions and latency, and whether the
  retrieval check used vector similarity or only keywords.
- **Store:** record count, how many records have embeddings, the file path,
  and counts by kind.
- **Retrieval check:** the broad search behind `memory_search`, with the
  combined, semantic, and keyword score of every hit. The query is the one you
  pass, or the most recent task.
- **Recent memories:** the newest 40 records.
- **Configuration:** the values in effect.

The badge is **Connected** when Ollama answers, the model embeds, and every
record has a vector. It is **Degraded** when memory still works but only
partly (keyword search only, or records that semantic search cannot find).
Each warning on the page names its fix.

The page is a snapshot. Run the command again to refresh it. The command also
answers with the verdict and warnings as JSON, so `davinci -p "/memory-page
--no-open"` works in scripts.
