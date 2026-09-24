# Vector memory

Vector memory keeps what was said in past turns of a project and brings the
relevant parts back before each prompt. It is a native Rust extension
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

Only `user` and `assistant` messages are indexed. Tool output is not.

## Commands

| Command | What it does |
| --- | --- |
| `/memory-page [--no-open] [query]` | Writes `.davinci/vector-memory/memory-page.html` beside the records and opens it. |
| `/memory-status` | Opens the memory sheet with record counts and configuration. |
| `/memory-search <query>` | Runs the same search used before each prompt. |
| `/memory-reindex` | Reloads the store, gives records that share an id their own id, and embeds up to 256 records that have no vector. Run it again for more. |
| `/memory-clear` | Deletes the project's records. |

## The memory page

`/memory-page` probes the configured Ollama server when it runs and shows:

- **Connection:** whether Ollama answers, whether the embedding model is
  installed, a test embedding with its dimensions and latency, and whether the
  retrieval check used vector similarity or only keywords.
- **Store:** record count, how many records have embeddings, the file path,
  and counts by kind.
- **Retrieval check:** the search that runs before each prompt, with the
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
