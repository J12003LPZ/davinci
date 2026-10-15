# Session cache-key experiment (opt-in, default off)

The production default is unchanged: every compatible run shares one `prompt_cache_key`
(`ci-root-<16 hex>`, from `CacheIdentity::cache_key`). This adds an isolated experiment that
partitions that key per conversation. It exists to be measured, not to be enabled.

## Switch

Settings key `experimentalSessionCacheKey`, boolean. Only an explicit `true` turns it on; absent,
`null` and `false` leave the shared key. There is deliberately no environment variable, so a stray
shell variable cannot change a run, and the benchmark harness records the setting in
`effective_settings`.

## What it does

`request_cache_key` (`main.rs`) builds the shared key with `live_cache_key` as before. With the
switch on, `session_partitioned_cache_key(shared, conversation_id)` returns
`{shared}-s{first 16 hex of SHA-256(shared, conversation_id)}`.

* The shared key stays in the text and the digest, so a change to the model, system prompt, tool
  schema, permissions or context still changes the key exactly as before.
* `conversation_id` is the stored session's header id when there is a session (stable across
  retries, continuation and process restart), otherwise one random id per process. Every request,
  retry and later prompt in one conversation therefore sends one key.
* The raw id is never written out. The key is at most 42 characters of `[a-z0-9-]`, with no path,
  secret or transcript content.
* `PI_GRAPH_CACHE_KEY` cannot mask the experiment. With the switch off it wins exactly as before.
  With the switch on, a top-level process that has it set refuses the request with an error naming
  both settings (`select_cache_key`), so a run is never measured as something it was not. Graph
  workers (`PI_GRAPH_ROLE` set) keep the key their parent handed them. An empty variable counts as
  unset. An empty conversation id returns the shared key rather than a constant partition.
* The effective mode is `effectiveSettings.experimentalSessionCacheKey` in the benchmark campaign,
  plus the key shape on the wire: `ci-root-<16 hex>` (shared) or `ci-root-<16 hex>-s<16 hex>` (session).

Nothing else changes: transport, headers, retention, model routing and every other request field are
as in the default. The experiment does not follow the module rule that keys must not contain session
ids; that rule is why it is behind a switch and documented in `identity.rs`.

## Offline checks (no model calls)

| Check | Where |
|---|---|
| Stable per conversation, distinct across conversations, follows prefix changes, provider-safe, leaks nothing, empty id falls back | `runtime::cache::identity::tests` (4 new) |
| Default off for absent, null, false | `settings::tests::session_cache_key_experiment_is_off_unless_explicitly_enabled` |
| With the switch on, every serialized request equals the default run's request except `prompt_cache_key` (session and no-session) | `ws_wire_diagnosis_tests::exp_session_cache_key_changes_only_prompt_cache_key` |
| One key across all requests and prompts of a conversation; two conversations differ | `ws_wire_diagnosis_tests::exp_session_cache_key_is_stable_within_a_conversation_and_differs_between_them` |
| Default key keeps the `ci-root-` shape with no suffix | `ws_wire_diagnosis_tests::exp_session_cache_key_default_is_the_shared_key` |
| `PI_GRAPH_CACHE_KEY` is refused when it would mask the experiment, still wins when the switch is off, and still reaches workers | `ws_wire_diagnosis_tests::exp_env_override_cannot_mask_the_session_experiment` |
| Existing append-only, field-stability and determinism tests still pass | `ws1_*`, `ws4_*` |
| Default requests are byte-identical to `main@8119a17` | one-off comparison, below |

Byte-identity check (not a committed test, it needs two checkouts): the same scripted conversation
(`Script::Equivalence`, mode `off`, with and without a session) was serialized request by request
on `main@8119a17` and on this branch. After normalizing the process id in the temp path, all 84
request bodies (42 per configuration) are identical.

## Risks

See `docs/perf/harness-gap/cache-key-ab.md` on the benchmark branch: the main one is losing the
cross-conversation reuse of the static prefix (about 8.7k tokens per new conversation).

## Not decided here

Whether to enable it anywhere. That needs the A/B in `cache-key-ab.md` and the owner's approval.
