# Context VM and graph hardening

Baseline: fetched `origin/main` and HEAD both `1106b1be2bffa1fb7de6e8c0708cc59d1ca2e576`.
Continue the existing isolated audit worktree; preserve all previous audit fixes.
The initial workspace Clippy run with warnings denied passes with those fixes.

## Frozen acceptance criteria

1. One provider budget accounts for system text, serialized tools, output and
   reasoning reserves, and a safety margin. Mandatory overflow stops dispatch.
2. Active VM requests never emit orphan or empty-ID tool outputs. Live tool
   protocol messages retain pairing; historical evidence uses data context.
3. Fold events use structured, provenance-validated proposals with explicit
   lifecycle transitions. Rejected proposals cannot corrupt authoritative state.
4. Changes after a checkpoint do not serialize repeated full checkpoints into
   provider input. Old persisted cache data is migrated or safely rebuilt.
5. A prepared context image is reused within an unchanged turn/revision, with
   invalidation for messages, overlays/plans, tool/permission surfaces, budget,
   and checkpoint roots. Inspection is not a second compilation.
6. Authoritative history remains retrievable without two runtime copies of all
   historical text. Cache/body retention and cold/warm behavior are measurable.
7. Lookup misses, rebuild attempts/results and semantic retrieval metrics have
   distinct meanings. Adversarial evals exercise corrections, rejected hypotheses,
   failures followed by success, provenance and resident/pageable evidence.
8. Workspace facts preserve typed uncertainty and reuse an existing per-turn
   engineering snapshot.
9. Graph layout is measured at 25/100/500/1000 nodes. Any cache must preserve
   layout/navigation/hit-test results and invalidate on graph, folds and geometry.
10. Relevant tests/evals, workspace check/Clippy/formatting, release and installed
    executable verification pass. No live-provider or hardware claim without a
    real check.

## Ordered work

- Confirm baseline and inspect independent Context VM and graph/snapshot paths.
- Fix budget and wire protocol; add regression tests before implementation.
- Integrate folding/materialization, prepared images, authoritative retrieval and
  metrics; add lifecycle and cold/warm evals.
- Share snapshot computation and benchmark/cache graph work as measurements justify.
- Independent review, proportional verification, documentation and local delivery.

The main agent is the sole writer. Four read-only investigators cover independent
paths; a separate reviewer judges artifacts against the criteria above.
Performance measurements use deterministic fixtures and report their limits.

## Results

1. A shared ProviderContextBudget charges system text, schemas, reserves and
   margin. Both native and JavaScript dispatch receive its output cap; failed
   Context VM compilation stops dispatch. Budget and JavaScript-handler
   regressions pass.
2. Historical tool evidence is custom data. Only the complete current exchange
   retains protocol calls/results; empty, duplicate and mismatched IDs are
   rejected from that exchange. The provider-wire replay target passes 13 tests.
3. Runtime fold events use the configured summarizer, structured proposal parser
   and provenance validator. Typed lifecycle transitions retire resolved or
   superseded state. Fake-summarizer integration and adversarial fixtures pass;
   real model proposal quality is not measured.
4. Updates materialize one checkpoint instead of accumulating full-state deltas.
   Legacy caches remain readable and divergent branches rebuild.
5. Prepared images are shared by Arc across estimates, payloads and manifests.
   Revision inputs include message content, overlays/plans, provider budget,
   effective tool/permission surface and VM roots. The permission-mode regression
   passes. Hashing publicly mutable history still costs O(history bytes).
6. Bound historical bodies are retrieved from the authoritative session JSONL.
   The VM retains metadata instead of two old-body copies. Paired 100/300-turn
   fixtures measure zero retained historical body bytes in the VM; this is not
   total process memory.
7. Lookup, rebuild and semantic retrieval metrics are separate. Nine mixed-role
   eval scenarios cover corrections, rejected hypotheses, verification state,
   pageable evidence and provenance.
8. Git state uses typed facts/unknown values. EngineeringSnapshot shares
   available index/metadata with impact, build and verification consumers.
   Reuse rechecks authorization/freshness.
9. The graph fixture covers all four requested sizes. A 1,000-node simulated
   key-to-frame p95 of 10.177 ms does not justify a cache on its own. No layout
   cache or navigation refactor was added.
10. Workspace tests and the focused fixture correction, all-target check,
    warnings-denied Clippy, formatting, Node tests and release benchmarks are
    complete. The release build and local install passed; the installed binary
    matches the build SHA-256 and passes version/help smoke checks. The old
    executable has a distinct verified backup.

See [measurements and verification](../../context-vm-measurements.md) and
[Context VM contracts](../../context-vm.md) for evidence and limitations. No
commit, push, remote CI result or live-provider acceptance is claimed.
