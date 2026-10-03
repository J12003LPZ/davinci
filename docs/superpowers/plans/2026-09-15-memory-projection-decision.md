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
