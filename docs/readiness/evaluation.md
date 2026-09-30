# Readiness evaluation protocol

This protocol extends `scripts/bench`; it does not generate tasks or start model
calls automatically. The twelve public tasks remain smoke tests. No live baseline,
private task inventory, feature win, or competitor superiority is asserted by this
change. Campaigns require the owner's repository selection, human prompts and
explicit model budget. Private fixture imports remain outside the checkout.

## Freeze private tasks

Supply a JSON manifest with `schema_version: 1`,
`kind: "private-repository-suite"`, and `tasks`. Each task requires:

```json
{
  "id": "repo-change-001",
  "repository_id": "owner-reviewed-repository",
  "repository": "/absolute/path/to/authorized/cloud/checkout",
  "starter_commit": "FULL_40_CHARACTER_PARENT_SHA",
  "reference_commit": "FULL_40_CHARACTER_MERGED_COMMIT_SHA",
  "language": "rust",
  "split": "dev",
  "size_class": "large",
  "visible_tests": true,
  "requires_existing_test_changes": true,
  "test_paths": ["tests/existing_test.rs"],
  "grader_command": ["cargo", "test", "--locked", "--offline"],
  "regression_command": ["cargo", "test", "--locked", "--offline"],
  "human_prompt": {
    "text": "Human-written request from the PR description",
    "author": "owner",
    "source": "URL of the merged PR",
    "authored_from": "pr-description",
    "diff_used": false
  }
}
```

Use 3–5 actively maintained repositories, 40–60 dev tasks and at least 150
holdout tasks. The importer verifies first-parent provenance, source and changed
test membership, distinct revisions (including repository aliases), safe paths,
committed bytes and executable modes. Symlinks and submodules are refused rather
than followed. The human provenance fields are attestations; software cannot
prove how a human authored a prompt. Review them before importing.

```sh
python3 scripts/bench/bench.py private-import --private-manifest /authorized/tasks.json --destination /private/frozen-suite
python3 scripts/bench/bench.py validate --private-suite /private/frozen-suite --split dev
```

Validation runs the supplied grader on the starter and complete reference tree;
the starter must fail and the reference must pass. Reference regression checks
must also pass. Runs capture regression results before overlaying hidden tests,
including hidden test deletions. Allowed paths are precisely reference-touched
paths. Frozen bytes, executable modes, labels and provenance are rechecked before
each run. Explicit split selection is mandatory. Holdout is never a tuning set;
do not inspect holdout transcripts to redesign a failed candidate.
Prepared private baselines keep every exported committed file tracked, including
inputs that match repository or global ignore rules. Changes to tracked files
remain visible even in cache-named directories; generated untracked caches can
still be filtered from unrelated-edit reporting.

## Freeze campaigns before tuning

Use the existing `bench.py run`, `--variant`, `--reps 3`, `--order-seed`,
`--private-suite`, `--split`, `--price-table` and checkpoint build path. A checkpoint
must have schema-3 provenance for clean committed source, its exact binary hash
and a completed green full CI run. A campaign pins executable hashes, fixture
digest, labels, model, effort, settings, runner source and dated USD prices before
running. Never replace baseline bytes or append tuning runs to its directory.

The existing Python runner provides same OpenAI model/effort DaVinci vs Codex.
The manual `competitor-differential.yml` path provides the existing Claude Code
adapter; use the same exact Claude model and supported public controls and retain
the adapter's controlled-model evidence. Its public-pattern suite is smoke
evidence, not the required private holdout. Native lifecycle ownership or a
container alone does not establish an independent grader or protect credentials
from all tools. Such runs stay `diagnostic-only`; do not relabel them independent.
The current runner refuses unsupported process lifecycle platforms before launch.

## Arms and decisions

Every comparison uses the identical dev tasks, three repetitions and rotating
first-arm order (`campaign.schedule` / `readiness_protocol.prepare`). Measure:

| Decision | Required arms | Current choice until evidence exists |
| --- | --- | --- |
| Requirement review / Preview | Stable with review off; Stable with review on; Preview with review on | Stable unchanged; review has explicit on/off control |
| Completion review ceiling | Same arm identities; median uncached input per task | Proposed 25%; owner may set `--token-ceiling` |
| Tool surface | Full versus Lean + `tool_search`, separately OpenAI and Claude | Full remains default |
| Sandbox Auto | Backend default versus explicit disabled, including approvals/task, success and wall time | Containment is probed; native Windows makes no isolation claim; WSL2 is the sandboxed path |
| Learning / security watch | Interactive or RPC off/on, success, unrelated edits and security findings | Both off / opt-in; learned skill promotion requires approval |
| Vector memory injection | Existing injection off/on | No measured subtraction/promotion claimed |
| Learned skill injection | Existing injection off/on | No measured subtraction/promotion claimed |
| Token governor digests | Existing digests off/on | No measured subtraction/promotion claimed |
| Specialist repo/LSP/package/build/git tools | Existing tool selection off/on for each family | No measured subtraction/promotion claimed |
| Graph | Existing graph advertising/selection off/on | No measured subtraction/promotion claimed |
| Context VM | Dedicated long sessions that compact at least twice; existing compaction versus VM | Off; JEV stays off |

All result cells are **not run** until campaign directories, binary hashes and
commit identities exist. No public hidden example determines a new default.
Preview candidates are dropped or promoted only through the existing prompt
promotion process with evidence. Receipt-bound requirement answers (Task 2.4)
are conditional on observed invented checks. An Anthropic adapter (Phase 6)
is conditional on same-Claude-model traces showing a deficit. Neither condition
has been observed here.

## One report and promotion gate

`bench.py report` includes per-arm and small/large readiness metrics: composite
success, regression-free success, unrelated edits, uncached/cached/output tokens
and estimated USD per verified success, median/P90 wall time, model requests and
tool calls. Failed-attempt spend remains in each numerator. Missing usage,
regressions, prices or background receipts are unavailable, never zero. Cache
write tokens need a separate provider price when nonzero. Print background cost
is explicitly unmeasured; interactive/RPC background counters stay separate.
When provider observations exist, token totals come from unique transport-attempt
receipts, including failed attempts and prewarm/JEV calls. Replayed receipts are
counted once; conflicting, missing, malformed or unfinished receipts keep usage
and estimated cost unavailable. Final-message usage is a fallback only for
legacy streams without provider observations or retry markers. First/later cache
groups follow coding logical requests and include every retry in each group.

```sh
python3 scripts/bench/readiness_protocol.py \
  --baseline /campaign/baseline/results.jsonl --candidate /campaign/candidate/results.jsonl \
  --holdout-baseline /campaign/holdout-baseline/results.jsonl \
  --holdout-candidate /campaign/holdout-candidate/results.jsonl \
  --price-table /campaign/prices.json --target composite_success_rate --token-ceiling 0.25
```

The offline gate rejects mismatched identities/model/effort/labels/repetitions,
duplicate or absent pairs, dev/holdout overlap, size-class regressions, missing
required metrics in either dev or holdout, missing independent-grading evidence,
token ceiling violations, unrelated-edit
increases and wall-time growth above 15%. It reports paired task-cluster bootstrap
success intervals; repetitions are not independent tasks. An interval containing
zero is reported as **no measurable difference**. Existing `bench.py compare`
continues to enforce its additional provenance and latency gates; readiness
results do not override them. This JSON gate is a diagnostic decision artifact,
not verification of a operator-supplied independent-grading attestation.
Each task needs a nonblank repository ID, full reference SHA and boolean test
labels, held constant across repetitions and arms. Source revisions cannot be
relabelled as another task or reused across splits. The combined dev/holdout
inventory must stay within the suite's 3–5 repositories; labels alone do not
authenticate repository ownership or an attestation.

A production superiority claim additionally needs a green released tag, at least
150 private holdout tasks, three repetitions, the exact same model and effort,
success interval lower bound above zero, separate large-task and interactive
background results, and shipped/documented controls. Nothing in an offline
synthetic fixture satisfies that definition of done.
