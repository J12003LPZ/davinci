# Memory vector projection decision — 2026-09-15

## Decision

Use the **local bounded retrieval/index as the production projection profile** for Phase 3. Qdrant remains a dormant compatibility surface and is not written on the normal indexing path.

## Break-even result

The inspected production path had local lexical+dense retrieval, while Qdrant was **write-only**: records were upserted but no Qdrant query participated in retrieval. Therefore there is no representative corpus size at which the existing remote projection can improve retrieval quality or latency; it only adds network and maintenance work. Claiming a measured remote break-even from that path would be misleading.

The local path is already bounded by `candidateLimit` and `maxIndexChunks`, preserves exact identifiers, and has labeled retrieval evaluation. This makes local the only projection with a demonstrated end-to-end retrieval benefit today.

## Reconsideration gate

Reconsider Qdrant only after a real namespace-filtered remote query path exists and the E4 gate from the engineering plan is measured on fixed corpora. Promotion requires **at least 30% better retrieval p95 at equal labeled quality**, with service-outage fallback, namespace isolation, tombstone exclusion, projection-lag accounting, collection/version validation, and model-change rebuild tests passing.

## Compatibility and rebuild policy

Persisted embeddings carry an identity containing model, dimensions, and the embedding-prefix revision. Legacy or mismatched embeddings are excluded from dense scoring. The authoritative local records remain available to lexical retrieval, and the local rebuild helper can regenerate bounded batches under the current identity. This permits safe rollback without deleting authoritative memory.

## Addendum 2026-10-19: optional Qdrant backend (Phase 5)

The reconsideration gate above is now measurable. A namespace-filtered remote
query path exists and is used when, and only when, the user chooses Qdrant
(`MemoryProjectionBackend::Qdrant`; `Local` is the default and makes no Qdrant
or Docker request). `crates/davinci-evals/src/behavior/qdrant_projection.rs`
runs the fixed, seeded gate: recall against exact local search, retrieval p95
for both, namespace isolation, retired-record exclusion, model-change rebuild
and outage recovery.

**Result so far (offline, in-process fake):** every functional gate passes;
recall@10 is 1.000 (the fake is exact); local p95 was 1.579 ms and the fake's
p95 5.407 ms on 1500 records of 64 dimensions. Those latencies describe a
loopback fake and are **not** evidence for or against the 30% p95 gate, so the
gate reports `Unverified` and the recommendation is **No-Go: keep Local**.

**Not measured:** p95 against a real Qdrant on a corpus large enough to matter.
Nothing here flips a default. To measure it, on a machine with Qdrant running:
`DAVINCI_QDRANT_LIVE_URL=http://127.0.0.1:6333 cargo test -p davinci-evals
qdrant_live_measurement -- --ignored --nocapture` (optional
`DAVINCI_QDRANT_LIVE_API_KEY`). It creates and removes only its own `eval-*`
namespaces and prints the receipt. A `Go` needs that live run to show at least
30% better p95 with every other gate passing, and then a separate decision to
change any default.
