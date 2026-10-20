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
| Vector memory | Reports Ollama, model and record health. After your approval (`/setup approve start-ollama`, `/setup approve download-model`) it starts the installed local Ollama and downloads `embeddingModel`, then embeds records that have no vector (`/setup repair`). | Install Ollama when it is missing. Approve the start and the download. Fix `embeddingDimensions` when the model disagrees. |
| .gitignore | Adds `/.davinci/vector-memory/` and `/.davinci/graph/` to the project's `.gitignore` inside a Git work tree. | Nothing. |
| Project trust | Reports when project settings, hooks, skills or MCP servers are ignored because the project is not trusted. | Review them, then run `/setup trust`. Restart to load them. |
| Plugin hooks | Lists enabled plugins whose hooks wait for approval. | `/plugins approve <name>`. |
| Language servers | Detects Rust, TypeScript/JavaScript and Python projects and looks for their servers on `PATH` or in `node_modules/.bin`. | Install the server it names. Davinci never installs one. |
| Project instructions | Looks for the `AGENTS.md` the session loads: inside a git checkout, from its root down to the working directory (`CLAUDE.md` is not read). | `/init`. |

`/setup check` reports without changing anything. The model pull continues
after the command returns, so run `/setup` again to follow it. Trust is never
granted by `/setup` alone, because a trusted project can run its own hooks,
extensions and MCP servers.


## Optional Qdrant

Qdrant is off. Memory is searched from the local record file, which is the
authority for every record, and nothing contacts Qdrant or Docker unless you
choose it. In local mode a `/setup` run asks Docker nothing.

**What it is.** A second, rebuildable copy of the memory *vectors* (numbers
and an id, never memory text) that Qdrant answers nearest-neighbour queries
from. DaVinci resolves each answer against the local records, so a record that
was removed, replaced or is outside the scope cannot come back from a stale
remote point.

**Benefit.** Faster nearest-neighbour search on a very large store, and one
vector index other tools on this machine can share.

**Downsides.** It needs Qdrant running (usually in Docker), uses more memory
and disk, and is one more service that can be down. On a normal store the
local search is already fast. The speed-up has **not** been measured against a
real Qdrant yet (see the addendum in
`docs/superpowers/plans/2026-09-15-memory-projection-decision.md`), so it is
not the default anywhere.

**Turning it on and off.**

| Command | Effect |
| --- | --- |
| `/setup qdrant` | Explains the above and looks, read-only, at what is on this machine: does Qdrant answer, is Docker installed, is its engine running, does a Qdrant container exist, is it DaVinci's. |
| `/setup qdrant enable` / `disable` | Writes the choice to `setup-profile.json`. Disabling stops all Qdrant and Docker traffic at once; the remote collection is kept as it is. |
| `/setup qdrant own <container-id>` / `release` | Records (or forgets) the one container you say is DaVinci's. Needs 12 or more characters of the id from `docker ps --no-trunc`, and the container must carry the label `io.davinci.owner=davinci-memory` (the `docker run` command DaVinci prints sets it). Recording a different container, or releasing, withdraws the start approval. |
| `/setup approve start-qdrant` | Your yes to starting the recorded container with `docker start <id>`. |
| `/setup qdrant keep-running on\|off` | Adds `--restart unless-stopped` to the `docker run` command DaVinci prints. DaVinci never changes a restart policy itself. |

Environment: `PI_MEMORY_BACKEND=local` turns the remote off without editing a
file, whatever the profile says. `PI_MEMORY_BACKEND=qdrant` turns it on.

**Configuration** (`vector-memory.json`; the URL keeps its existing keys):

```json
{
  "qdrantUrl": "http://127.0.0.1:6333",
  "projection": {
    "backend": "qdrant",
    "apiKey": "optional, or PI_MEMORY_QDRANT_API_KEY",
    "allowRemote": false,
    "collectionPrefix": "davinci",
    "batchSize": 64,
    "offlineBackoffSeconds": 60
  }
}
```

The API key is sent as the `api-key` header and nowhere else: it is not
written back by DaVinci, not shown in status or logs, and not part of any
payload. A URL that is not on this machine is refused unless `allowRemote` is
true.

**Collections.** One collection per repository and embedding model, named
`davinci-<12 hex of the repo>-<8 hex of the model identity>`. The old global
`pi_memory_v1` is not used. A collection that already exists with another
vector size or distance is reported (`QDRANT_REFUSED`) and left untouched.
Every query and delete carries the repository id; `/memory-clear` and a
rebuild delete only this repository's points.

**When Qdrant is down.** Search falls back to the local index and says so:
`memory_search` ends with `[QDRANT_OFFLINE] remote vector search is
unavailable; local search was used.`, the memories injected into a turn
automatically start with the same marker on their first line, and status shows
the code. Nothing is
lost: records are written locally first. The remote is skipped for
`offlineBackoffSeconds` after a failure, so a stopped Qdrant costs one timeout.
`remoteProjectionLag` in the status counts records the remote does not hold
yet (plus retired ones it should drop). It is caught up in bounded, idempotent
batches by indexing, `/memory-reindex` and `/setup repair`; a lost
`projection-state.json` only makes the next pass send everything again.

**Docker.** Discovery runs `docker version` and `docker ps` with a five second
timeout and nothing else. DaVinci does not install Docker or Qdrant, does not
start Docker, does not change Docker Desktop settings, and touches no
container that is not the one you recorded. Ownership is the recorded id and
nothing else; the label `io.davinci.owner=davinci-memory` is only a check that
the recorded container is still the one DaVinci's command made. A labelled
container nobody recorded is never started, wherever `docker ps` lists it. With
nothing running it prints the `docker run` command for you to run yourself.

**Collection ownership.** When DaVinci creates a collection it writes one
marker point (no repo id, no text) into it. A collection with the derived name
but no marker was not made by DaVinci: it is reported (`QDRANT_REFUSED`), never
adopted, and nothing is written to it or deleted from it. Delete it yourself or
set `collectionPrefix` to another name.

**API key.** It is sent only over `https://`, or over `http://` to this
machine. A plain `http://` URL to another host with a key set is refused with
an error, and redirects are not followed, so the header never goes anywhere
else. `batchSize` is clamped to 512 points per request.

**Known limits.** `/setup approve start-qdrant` runs on the interactive loop and
can block it for up to about a minute while Docker answers (every call has a
timeout). `/setup qdrant` (show, enable, own) asks Docker even while the
backend is local, because you asked it to; `/setup check` and everything
automatic do not. An empty answer from a reachable Qdrant falls back to local
search without a label, because Qdrant is up; `remoteProjectionLag` shows the
gap.

**Rollback.** `/setup qdrant disable` (or `PI_MEMORY_BACKEND=local`). Local
search and every record are exactly as before; delete the remote
collection in Qdrant if you want it gone.

## Commands

| Command | What it does |
| --- | --- |
| `/setup [check [--json]\|repair\|approve <action>\|enable\|disable <feature>\|qdrant ...\|trust]` | Sets up vector memory and the other features above. See `docs/setup.md`. |
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
