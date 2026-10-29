# 09 — Roadmap and acceptance criteria

Status: plan. Only Wave 0 is implemented (`10`). Every item is ordered so that the
measurement that justifies the next item exists before that item is written.

Rules that constrain every wave:

- regression test first, then the change;
- no new model call in CI, no live subscription consumption without explicit
  authorisation;
- preserve `openai-codex` subscription auth and the user's selected model/effort;
- no silent model switching, no paid API-key fallback;
- no commit/push/deploy without explicit authorisation;
- every acceptance criterion is a command and an expected value, not an adjective.

## Wave 0 — Unknown outcome for a lost MCP reply (done)

Gap G-01. See `10-IMPLEMENTATION-RECORD.md`.

**Acceptance (met):**

```
cargo test -p davinci-mcp                   → 52 lib + 15 integration tests pass
cargo test -p davinci-agent --lib           → 1863 pass
cargo clippy -p davinci-mcp -p davinci-agent --all-targets → no warnings
```

and every regression named in `02` §6 is green (`11` §3).

## Wave 1 — Make "done" mean something

Order inside the wave: G-06 (corpus) → G-07 (gate) → G-02 (worker claims). The corpus
comes first because otherwise neither of the other two can be evaluated on a valid
oracle.

### 1.1 G-06 — a corpus that can fail

**Change:** replace `core-200.json`'s empty verification commands with real commands per
scenario; replace the 21-byte `core-repo` fixture with repositories that contain the
files the requirements name; delete or repair the dead fixture directories.

**Acceptance:**

1. a loader-level test asserts that no mutating scenario resolves to a fallback command;
2. a test asserts every `repo_fixture` referenced by a scenario exists and contains
   every path named in a `file_changed`/`read_before_edit` requirement;
3. `cargo test -p davinci-evals behavior::` is green;
4. the corpus hash is recorded in the campaign directory.

**Do not:** pad scenarios with `git diff --check` to make the loader pass.

### 1.2 G-07 — the completion gate refuses, or records

**Change:** choose one:

- **(a) refusal**: after the reminder, a finish attempt with unverified mutations of the
  current generation is refused with a tool-level override; or
- **(b) terminal state**: the turn ends but the session records
  `completion: unverified` for the generation, and the host surfaces it as a state, not
  a transient notice.

**Acceptance:**

1. a test drives a mutation, a failed verification and a finish attempt; the session
   either refuses once more (a) or records `unverified` for exactly that generation (b);
2. a test proves a verified generation is never refused/flagged;
3. a test proves the override path (if (a)) is itself recorded;
4. no change to the number of provider requests for a text-only turn.

### 1.3 G-02 — verify a worker's claim before reporting success

**Change:** a worker result carries a verification receipt: the command it ran, its exit
status and the artifact/diff digest. `finish_agent` reports `Unverified` when the claim
has no receipt, and never reports `Succeeded` on exit 0 alone.

**Acceptance:**

1. fixture: worker says "tests pass" and exits 0 with no receipt → parent sees
   `Unverified`, not success;
2. fixture: worker with a passing receipt → `Succeeded`;
3. fixture: worker with a failing receipt → `Failed` with the command and exit code;
4. the worktree is preserved for `Unverified` exactly as for `Failed`;
5. `agent_status` shows the receipt fields.

## Wave 2 — Durability and leaks

### 2.1 G-03 — worktree teardown and reaping

**Change:** inspect every teardown result and record it; add a startup scan that lists
`wt-*` directories and `davinci/agent/*` branches it does not own, and refuses to reuse a
path it cannot account for.

**Acceptance:**

1. test: a forced `worktree remove` failure leaves a recorded `LeaseCleanupFailed` event
   and the lease still listed;
2. test: a directory planted at the deterministic path makes creation fail with a
   message naming the path (already true) and the scan lists it before creation;
3. test: after a simulated crash (drop without cleanup), the next start lists the orphan
   and does not delete it without an explicit flag;
4. no orphan deletion happens automatically.

### 2.2 G-05 — workflow state and notices

**Change:** persist `spec`, `launch` and status; use `record_worker_result` for workflow
completion notices so they rehydrate like subagent results.

**Acceptance:**

1. test: `workflow_status` returns the last known phases after a process restart;
2. test: `resume` works without the caller re-supplying the spec;
3. test: a completion notice survives a lost mailbox notification and appears once in
   `agent_status`;
4. test: resume cannot widen launch authority (existing `wor117` stays green).

## Wave 3 — Context quality

### 3.1 G-04 — constraints must survive the default mode

**Change:** either default to `Hybrid`/`Active`, or add a deterministic constraint check
to `Off`: extract user constraint sentences before compaction, require each to be
present (verbatim or as a cited paraphrase) in the replacement, and refuse the
compaction if one is missing.

**Acceptance:**

1. test: a constraint stated in the first user message survives five compactions in the
   **default** mode;
2. test: a summary that drops the constraint is rejected and the transcript is kept;
3. no provider call is added by the check itself.

### 3.2 G-08 — default tool surface

**Precondition:** a plane-B measurement of prefix bytes and tool count with `Full` vs
`lean` on the same session, offline. **Acceptance:** `lean` becomes the default only if
the measurement shows a smaller stable prefix with no missing tool on a recorded set of
representative prompts. Task success is not claimed by this change.

### 3.3 G-17 — lazy skill bodies

**Acceptance:** a test asserts that constructing the agent does not read `SKILL.md`
bodies; invoking a skill reads exactly one body.

## Wave 4 — Security and cleanup

### 4.1 G-09 — IPC auth

**Change:** refuse to serve with an empty token; constant-time comparison; peer
credential check on Unix; explicit "unsupported on Windows" error.

**Acceptance:**

1. test: empty token → server refuses to start;
2. test: wrong token → connection closed before any command;
3. test: correct token → one command succeeds;
4. test: comparison time does not depend on the matching prefix length (bounded
   statistical check, or a code-level wrapper with a constant-time primitive).

### 4.2 G-10 — cleanup on signal

**Acceptance:**

1. test: SIGTERM during a live job removes the socket file and terminates the job;
2. test: killing the parent does not leave a foreground child running (job object
   kill-on-close);
3. the existing stale-socket recovery test stays green.

### 4.3 G-13 — one policy revision on both routes

**Acceptance:** a test changes the policy between prepare and dispatch on the
compatibility route and asserts denial; a batch child and a codemode child are each
denied by a deny rule end to end.

## Wave 5 — Capacity and future work

| Item | Acceptance |
|---|---|
| G-11 admission | a configured budget blocks with a timeout and a diagnostic; an unconfigured budget refuses fan-out above a stated default instead of being unlimited |
| G-12 invocation id | every `tools/call` carries a stable invocation id; a fixture server records it and the same id is observed on a model-initiated retry of the *same* tool call |
| G-14 cancellation | fixture server commits after receiving `notifications/cancelled`; the report stays "unknown" and no second call is made |
| G-18 spend cap | workflow `max_cost_usd` either enforces a stop or the field is removed from the accepted spec |

## Hygiene (any time)

| Item | Acceptance |
|---|---|
| G-15 duplicate flag | `execution_authority` receives the real capability flag; a test distinguishes a builtin from an unregistered tool |
| G-16 ledger window | the 256-record bound is documented in the ledger doc comment and covered by a test that re-admits beyond the window |
| Doc fixes from `01` §4 | each divergence is either fixed in code or corrected in the doc, with the test that settles it |

## Sequencing rationale

Measurement precedes optimisation on purpose:

- Wave 1 makes a valid success measurement possible at all. Without it, any later
  "improvement" is unverifiable.
- Wave 2 stops silent loss. It is independent of model behaviour and testable offline.
- Wave 3 changes prompt shape and is the most likely to move plane B; it must not be
  credited with a plane-A effect.
- Wave 4 is security and cleanup; it has no task-success story and must not be sold as
  one.
- Wave 5 items are either preconditions (G-12) or capacity policy (G-11).

## Out of scope for this roadmap

- Any retry/reconnect layer before G-12 and a proven non-delivery signal.
- Copying Claude Code constants.
- A "parity score".
- Any claim of token or subscription savings.
