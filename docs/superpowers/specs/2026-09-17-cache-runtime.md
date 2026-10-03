# DaVinci Cache Runtime

## Reference and completion contract

Implement the supplied 55-section Cache Runtime plan and its 29 acceptance criteria.
Correct invalidation and request-time authorization outrank cache hits. Normal sessions
and Graph share mechanics. Immutable content may cross worktrees; mutable generations
and live resources may not. No subagents, force pushes, model API calls, or unrelated edits.

## Inspected baseline

Remote main, fetched and independently verified: a50fdc6dfb8fa6aad8829299e8aaac258bf15477.
The previous Repo Intelligence PR is not on this base. Do not import its unmerged files.
Language Intelligence IS present under coding-agent/src/semantic: bounded registries,
local LSP backend, exact document hashes, authorized launches, and text fallback.

Existing mechanisms:

- agent/runtime/cache.rs: universal prompt identity and reason-coded differences.
  Preserve its existing key bytes. Role changes currently change keys but lack a diff reason.
- coding-agent/native_extensions/ecosystem/cache_affinity.rs: legacy Graph key plus
  universal identity adapter; provider-reported counters are already separate.
- ai/cache.rs and stream.rs: provider-specific retention, OpenAI keys, Anthropic
  cache-control and Bedrock cache points. Preserve wire behavior and prefix ordering.
- ai/transport.rs: account/session socket decision bookkeeping. The dependency direction
  prevents ai from importing agent; do not introduce a cycle.
- native token governor: bounded read-output deduplication and explicit output recovery.
  These are context/recovery semantics, not authority to persist arbitrary tool outputs.
- security_scan: immutable review artifacts, private publication, confined reads and
  OS-held leases. Recovery artifacts are not generic deterministic knowledge.
- session storage: atomic checkpoint publication. No general CAS or weighted LRU exists.
- ordinary read tool: repeated prefix/window/totals reads; packages parse installed manifests.
- semantic/backend: registry mutex is held during process startup. Move expensive launch
  outside the registry lock with common single-flight; retain session ownership/shutdown.
- runtime events and resource ledger already report provider usage. Keep local statistics separate.

## Baseline measurements

Fresh offline locked debug build passed. Fifteen real --version process launches:
median 7.7508 ms (first launch 352.0845 ms, warm filesystem launches 7.4205–8.4705 ms).
This measures process startup only, not full session initialization. Benchmark host construction
and an ordinary deterministic read workflow separately before changing their implementation.

## Alternatives and decision

| Concern | Options | Decision |
|---|---|---|
| Persistence | filesystem CAS; SQLite; append-only log | CAS with bounded versioned JSON objects, no new database or recovery log |
| Memory | external LRU crate; bounded std maps with access sequence; FIFO | std maps with access sequence, weighted limits and deterministic least-recent eviction |
| Coordination | OS lease; create-new stale lock; database transactions | short nonblocking OS lease for publication/sweep, duplicate computation allowed across processes |

Existing private-file and OS-lock patterns fit CAS. Serialization and disk I/O happen
outside the memory mutex. No scan or directory creation at constructor time. A bounded
directory scan on disk writes/sweeps enforces the total disk budget without shared mutable
metadata. Persistent hits need only one object. Cache failures fall through to computation.

## Contract

Typed namespaces and canonical length-safe serialized key material include consumer
schema/algorithm, logical identity and sorted dependency tokens. The unchanged universal
prompt identity lives alongside these keys, not in a second prompt architecture.
Policies are explicit: no cache, RAM only, TTL/negative, immutable disk, workspace disk.
All lookups execute a host-supplied authorization callback; permission-dependent values
also key on authority inputs. Dependency versions are supplied freshly by consumers.
Invalidating a token removes only dependent memory entries; old immutable disk objects
become unreachable through changed versioned keys and are swept by budget.

Single-flight publishes one result/error to bounded waiters, releases map entries after
errors/panics/cancellation, and never holds registry locks during computation.
Live resources use the same single-flight primitive but are never serialized.
Provider usage has separate counters and no inference from local identity equality.

File freshness requires an authorized, confined exact reread/hash. RAM caches avoid
repeated decoding/derived work, not magically all filesystem I/O. Persistent consumers
must explicitly select safe deterministic artifacts, never raw sensitive file/tool output.
Workspace state owns canonical identity, exact fingerprints and generation; file-level
keys use individual hashes, aggregate keys use generation.

LSP result reuse is conservative: document symbols can use exact document versions and
server generation; workspace-sensitive references/diagnostics/definitions require broader
dependency proof and otherwise remain uncached. Live child health is checked before reuse.

## Ordered implementation

1. Capture baseline host/read metrics; add RED contracts for keys, LRU, invalidation,
   authority, concurrency and cancellation. Extract existing identity without changing keys.
2. Implement bounded memory, telemetry and single-flight. Validate focused agent tests.
3. Add confined versioned CAS, atomic publication, bounded sweep and process tests.
4. Integrate normal read-derived work and safe deterministic metadata; settings/status,
   shared runtime ownership and provider counters. Verify disabled/failure paths.
5. Integrate LSP lifecycle and conservative query reuse; preserve Graph compatibility,
   move common identity mechanics to agent ownership. Add Repo/AST adapter fixture tests.
6. Run required benchmarks: normal workflow, restart, worktrees, ten parallel callers,
   one-file edit, no Graph. Document actual latency, bytes and avoided work.
7. Audit all requirements; run fmt, workspace clippy, agent/ai/coding-agent suites;
   review diff, commit intended files, push feature branch, PR and green relevant CI.

## Verification boundaries

No provider hit or token-saving claim without provider usage. No production AST benchmark
when the parser is absent; use a clearly labeled deterministic adapter fixture. No test
outcome, shell output or model conclusion caching. Cache persistence is optional.

## Delivered architecture and integration

`davinci-agent/src/runtime/cache/` owns canonical typed keys, the existing universal
prompt identity, weighted memory LRU, immutable persistent objects, dependency revocation,
workspace snapshots, single-flight, confined fresh reads, and distinct telemetry.
Namespaces are prompt, file, ast, repo, query, lsp, package, git, build, and test.
The latter two provide namespaces only; test outcomes and broad build results are not cached.

The ordinary read tool rereads authorized bytes on every request. Exact byte equality
permits reuse of a previous SHA-256 digest; edits of equal length are detected. Text
windows and fresh snapshots stay in RAM. Only derived line/byte counts enter the CAS.
RuntimeHandle, ToolContext, and native extension hosts share ownership. Normal provider
requests use CacheIdentity; explicit Graph cache keys retain their original byte framing
through a compatibility adapter in the common module. Provider retention and wire-format
implementations are unchanged. Reported usage is recorded independently of local hits.

Local LSP backends share a bounded process-local owner per CacheRuntime. Ten callers
coordinate one launch; launch and shutdown occur outside the session registry mutex.
Discovery is cached for 250 ms, keyed by canonical workspace and resolver environment,
with a separate negative TTL. Canonical root validation remains fresh. Ready sessions
recheck current launch authority and child health. A changed command or restart creates
a new server generation. Only document-symbol responses are cached, for 250 ms and by
method, parameters, workspace, URI, exact content, document version, and server generation.
Diagnostics, references, and other workspace-dependent responses remain uncached.

WorkspaceSnapshot binds aggregates to canonical root plus exact declared input hashes.
Consumers must declare the complete dependency set for an aggregate. Immutable per-file
keys omit workspace identity, allowing compatible worktrees and restarted runtimes to
share objects. Revoked dependency tokens cannot revive through disk lookup or publish
in-flight results in the same runtime; callers must refresh their versions. Old immutable
objects remain eligible for bounded eviction. Package manifest/lockfile and Git object
tokens are exercised by consumer integration tests. The absent Repo Intelligence parser
has a tested API contract, not an invented production integration.

## Configuration and operation

Existing settings accept a `cache` object with `enabled`, `memoryEnabled`,
`persistentEnabled`, `maxEntries`, `memoryMaxBytes`, `persistentMaxBytes`, and
`maxObjectBytes`. Defaults: enabled, 2,048 memory entries, 64 MiB memory weight,
512 MiB persistent storage, and 8 MiB per payload. Constructor caps also limit extreme
settings. Memory weights include serialized size plus key/entry/type overhead; consumers
must keep custom deserialized allocations bounded. Single-flight has at most 256 active
keys and a ten-second runtime waiter timeout. Negative TTLs cap at five seconds.

Persistence lives under the host-selected agent state directory at `cache-runtime/v1`.
Construction creates no directories and performs no cache scan. The versioned JSON
envelope verifies identity, key, schema format, and payload checksum with bounded reads.
Atomic publication and short nonblocking OS leases protect duplicate writers and sweeps.
Unix operations use directory descriptors and no-follow opens; Windows pins directory
ancestors and rejects reparse points. Failed storage, corrupt objects, and occupied leases
fall back to computation. There is no model-controlled cache path or executable payload.

`/cache-status` reports memory, observed persistent usage, namespaces, single-flight,
resident LSP sessions, and provider counters. Disk totals are the last observed write/sweep
census, not a claim that an unscanned directory is empty. No destructive clear command
was added. The programmatic sweep only handles generated cache objects and orphan temps.

## Measurements

Windows, repository-pinned debug build, offline, no Graph or provider calls:

| Measurement | Baseline | Cache Runtime |
|---|---:|---:|
| 15 real `--version` launches, median | 7.7508 ms | 8.0157 ms |
| Native host construction, 30 samples, mean | 34.3688 ms | 33.43–35.06 ms across measurement runs |
| Ordinary 36,000-byte read, 100 calls, mean | 0.426635 ms | 0.396116 ms |
| Ordinary reads, source bytes actually reread | not instrumented | 100 reads / 3,600,000 bytes |
| Ordinary read cache | absent | 297 RAM hits, 3 entries, 172,842 estimated bytes |
| 10,000-file adapter fixture, cold | — | 768.2054 ms / 10,000 computations |
| Same fixture, warm | — | 207.7895 ms / zero additional computations |
| One file edited | — | 207.8585 ms / 1 computation / 9,999 reused |
| Adapter memory after edit | — | 10,001 entries / 4,089,297 estimated bytes |
| Persistent fixture publication | — | 17.8455 ms / 3,574 disk bytes |
| Fresh runtime persistent read | — | 4.4533 ms |
| 10 concurrent deterministic callers | — | 1 computation / 9 waiters reused |
| 10 concurrent local LSP callers | — | 1 process start / 9 reuses |

These are local measurements, not universal speed guarantees. `--version` exits before
full session initialization; host construction is reported separately. The ordinary-read
delta is small enough to be sensitive to host noise. A first implementation regressed to
1.19 ms/read; profiling found repeated SHA work, corrected by fresh-byte equality reuse.
The large fixture tests cache integration and incremental work counts, not a real AST
parser. Cross-worktree and cross-process tests use isolated fixtures. Live paid-provider
cache hit rates were not measured; no provider token savings are claimed.

## Validation and remaining boundaries

Regression coverage includes canonical/versioned keys, LRU, permission checks on hits,
same-length edits, disabled/unavailable storage, bounded reads, corruption and format
mismatch, duplicate process writers, orphan cleanup, dependency invalidation during
computation, workspace isolation, negative expiry, waiter cancellation/timeout, retries
after errors and panic, memory promotion, provider/local separation, and lazy status.
A real subprocess LSP fixture verifies concurrent startup, document updates, query reuse,
dead-child rejection, and restart generation isolation. Unix CI additionally exercises
symlink escape rejection.

The persistent store uses a bounded flat-directory census on writes and explicit sweeps.
Future high-volume AST ingestion may justify sharding or an index after measurement;
no such subsystem is needed by the current small persistent consumer. Live handles are
shared within a process, never serialized or shared through an implicit external daemon.
Host permissions remain the authority, and generic consumers must supply their request-time
authorization callbacks and complete dependency versions. There are no cached model
conclusions, arbitrary shell results, credentials, or test outcomes.

Final required local gates and CI results are recorded in the PR after execution.
