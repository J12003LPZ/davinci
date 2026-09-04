# Davinci Closed Ecosystem Integration Architecture

This document describes the closed ecosystem integration contracts connecting **Graph**, **Token Governor**, **Vector Memory**, **Learning**, and **Security** in Davinci (`pi-rust`).

---

## 1. Producer / Consumer Matrix

Every subsystem boundary in the Davinci ecosystem operates under explicit, bounded contracts with zero model orchestration overhead:

| Producer Subsystem | Consumer Subsystem | Flow & Payload Contract | Bounded Budget / Invariant |
| :--- | :--- | :--- | :--- |
| **Vector Memory** | **Normal Interactive Turn** | Ephemeral similarity search on user/turn prompt | Top-k relevant memories, ephemeral injection |
| **Vector Memory + Learning** | **Graph Worker Context** | `build_context_packet` produces `<context source="davinci" untrusted="true">` | Strict cap: <= 2,500 aggregate tokens (<= 1,200 memory tok / 4 hits; <= 1,000 skill tok / 2 skills) |
| **Token Governor** | **Graph Worker Execution** | Compressible tool outputs (>100 B) compacted into digest (`governor://`) | `retrieve_output` preserved in worker allowlist; lossless byte-for-byte recovery on demand |
| **Graph Execution** | **Security Scanner** | File mutations evaluated via `assess_change_risk` | High risk (`ChangeRisk::High`) or `always` mode triggers `verify_changed_surface` before review |
| **Graph Verification** | **Learning System** | `VerificationBundle` derived deterministically from unit tests and security | Approval eligibility computed pure/deterministic; `record_skill_version_outcome` updates ledger |
| **Learning System** | **Future Graph Runs** | Verified procedural skills (`SKILL.md`) & high-confidence facts | Selected exact version `(name, version, content_hash)` injected into worker context |

---

## 2. Invariants

1. **Zero Coordinator Model Calls**:
   - Preparing context packets, deriving cache keys, taking resource snapshots, evaluating security gates, and recording learning outcomes are strictly local, deterministic computations.
   - Normal turns and graph runs execute **0 additional coordinator or preparation model calls**.
2. **Ephemeral Worker Isolation**:
   - Graph workers execute with `--no-session --no-extensions --no-skills`.
   - Automatic background memory injection in child processes is suppressed via `PI_GRAPH_SUPPRESS_MEMORY_INJECT=1`.
3. **Prompt Cache Affinity**:
   - Provider cache keys are decoupled from session IDs (`StreamOptions::cache_key`).
   - Derived cache keys (`derive_worker_cache_key`) preserve prompt prefix caching across worker retries and iterations while maintaining ephemeral worker isolation.
4. **Strict Context Budget Bounds**:
   - Combined context packets never exceed 2,500 estimated tokens.
   - At most 4 memory hits and at most 2 skills are injected per worker.
5. **Exact Provenance and Attribution**:
   - Tasks record the exact `(name, version, content_hash)` of every injected skill.
   - Outcome ledgers increment only when the executing version's hash matches the store record.

---

## 3. Fallbacks and Kill Switches

If any ecosystem subsystem needs to be bypassed or isolated during troubleshooting or minimal deployments:

| Capability | Kill Switch / Configuration | Default | Fallback Behavior |
| :--- | :--- | :--- | :--- |
| **Graph Context Packet** | `PI_GRAPH_DISABLE_CONTEXT=1` or `maxTokens = 0` | Enabled (2,500 tok) | Graph workers start with empty context packet |
| **Security Gate** | `GraphConfig::security_verification: "off"` | `"risk"` | Changed files bypass security scan; deterministic test commands still run |
| **Learning Background Review** | `PI_LEARNING_DISABLE_BACKGROUND=1` | Enabled | Background reviewer thread skips turn analysis; foreground remains unaffected |
| **Cache Key Decoupling** | Fallback when `cache_key == None` | `Some(key)` | Reverts to `session_id` provider prompt cache grouping |
| **Token Governor** | `TokenGovernorConfig::enabled: false` | Enabled | Tools stream full, uncompacted output directly |

---

## 4. Named Ecosystem Test Commands

The ecosystem integration is covered by offline, deterministic integration tests that require zero external network access or provider credentials:

```bash
# Run all closed-loop ecosystem integration tests:
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture

# Run invariant enforcement tests (token limits, hit caps, zero model calls):
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture

# Run integration telemetry tests:
cargo test -p davinci-coding-agent ecosystem_telemetry -- --nocapture
```

### Interpretation of Cache Evidence

- **Structural Cache Evidence (Offline & Tests)**:
  `derive_worker_cache_key` generates identical cache keys for compatible worker retries and changes deterministically when model, tools, prompt, or contract changes. This is verified offline in `ecosystem_loop_cache_affinity`.
- **Live Provider Cache Evidence (Online / Production)**:
  Real provider usage reports cache hits/writes via provider response headers (e.g. Anthropic `cache_read_input_tokens`, `cache_creation_input_tokens`). These metrics populate `EcosystemStats::cache_read_tokens` and `cache_write_tokens`, displayed compactly in `/status` and `/graph-status`.
