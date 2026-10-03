# Lazy Semantic Navigation and Incremental Security Scan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish semantic code navigation as a lazy, permission-aware capability and make deterministic security scanning incremental without weakening report sealing.

**Architecture:** Extend the existing `NativeSemanticService` rather than replacing it, using workspace-keyed lazy sessions and deterministic text/compiler fallback. Extend the existing security scanner with file-level reusable deterministic results keyed by content/rules/config, while every completed scan still builds and seals a fresh run report.

**Tech Stack:** Rust 1.83, std process/filesystem, existing semantic/security modules, serde/sha2 already present in the workspace.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- No LSP process starts at application launch.
- Language servers are project-adjacent executables and remain subject to trust/permission policy.
- Semantic failure falls back; it must not make ordinary grep/read navigation unavailable.
- Security cache is file-level evidence only; final run manifests/reports are newly generated and sealed.
- No network is introduced into deterministic security scanning.

---

### Task 1: Define lazy semantic session identity and lifecycle

**Files:**
- Modify: `crates/davinci-coding-agent/src/semantic/mod.rs`
- Create focused file only if current module becomes unwieldy: `crates/davinci-coding-agent/src/semantic/session.rs`

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticSessionKey {
    pub canonical_root: PathBuf,
    pub language: String,
    pub config_digest: String,
}

pub enum SemanticSessionState {
    Starting,
    Ready,
    Unavailable { reason: String },
}
```

`NativeSemanticService` owns a bounded map of session keys to lazy session handles. No process spawn occurs in `Default`, `new`, or CLI startup.

- [ ] **Step 1: Add a constructor test proving no server is spawned**

Inject a fake spawner counter and assert service construction leaves count `0`.

- [ ] **Step 2: Run and confirm current stub cannot satisfy a ready-on-demand lifecycle**

```bash
cargo test -p davinci-coding-agent semantic_service_is_lazy_at_construction
```

- [ ] **Step 3: Implement key normalization and lazy session storage**

Canonicalize root; normalize language; hash only stable config inputs. Cap idle session count with deterministic oldest-use eviction if the service already tracks usage timestamps, otherwise cap by insertion order.

- [ ] **Step 4: Commit**

```bash
cargo test -p davinci-coding-agent semantic_service_is_lazy_at_construction
git commit -am "feat(semantic): add lazy workspace session lifecycle"
```

---

### Task 2: Implement on-demand server resolution and permission-aware launch

**Files:**
- Modify semantic service files
- Modify only existing host/trust hook plumbing required to authorize executable launch

**Interfaces:**

Add resolver:

```rust
pub fn resolve_language_server(language: &str, root: &Path) -> Option<LanguageServerSpec>;

pub struct LanguageServerSpec {
    pub program: String,
    pub args: Vec<String>,
    pub language: String,
}
```

Initial supported mappings should be limited to servers already expected by the product/docs or discoverable locally, e.g. Rust `rust-analyzer` and TypeScript `typescript-language-server --stdio`. Do not download/install servers.

- [ ] **Step 1: Add resolver tests**

Use fixture environment variables or injected executable lookup; no real server needed.

- [ ] **Step 2: Add permission-refusal test**

A denied project/server launch returns semantic unavailable and does not spawn.

- [ ] **Step 3: Implement launch gate through existing permission/trust path**

Do not silently use Always Approve. Non-interactive contexts without authority should fail closed and fall back.

- [ ] **Step 4: Run**

```bash
cargo test -p davinci-coding-agent semantic_server_resolution_is_local_only
cargo test -p davinci-coding-agent semantic_launch_respects_permission_gate
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(semantic): launch language servers on demand"
```

---

### Task 3: Implement definition/reference/diagnostic requests with fallback

**Files:**
- Modify: `crates/davinci-coding-agent/src/semantic/mod.rs`
- Modify semantic tool dispatch/host adapter if separate

**Interfaces:**

Existing `SemanticService` methods should attempt:

```text
healthy lazy server
  -> semantic result
server unavailable/timeout/protocol error
  -> deterministic existing grep/compiler/text fallback
```

Do not invent results when both paths fail; return an explicit unavailable/error result.

- [ ] **Step 1: Add fake-server success test**

Feed a canned LSP response for definition/reference and assert conversion to the current semantic result type.

- [ ] **Step 2: Add timeout fallback test**

Fake server times out; assert fallback runner is invoked once and its deterministic result returned.

- [ ] **Step 3: Implement bounded request timeout and unhealthy-session invalidation**

Repeated failed sessions should not incur one timeout per tool call. Add a short local backoff/unavailable state similar in spirit to vector-memory dense backoff.

- [ ] **Step 4: Run focused semantic tests**

```bash
cargo test -p davinci-coding-agent semantic_definition_uses_lazy_server
cargo test -p davinci-coding-agent semantic_timeout_falls_back_deterministically
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(semantic): add lazy navigation with fallback"
```

---

### Task 4: Add language-scoped semantic benchmark fixtures

**Files:**
- Modify: `crates/davinci-evals` or existing semantic test fixtures

**Interfaces:**

Offline fixture tasks for at least Rust and TypeScript:

```text
find definition
find references
identify diagnostic target
```

Compare:
- text-only fallback;
- semantic-enabled fake/local fixture path.

No external server installation in CI.

- [ ] **Step 1: Add deterministic fixtures**
- [ ] **Step 2: Assert semantic path improves or ties navigation precision without changing permissions**
- [ ] **Step 3: Run and commit**

```bash
cargo test -p davinci-evals semantic_navigation
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "test(semantic): add language-scoped navigation ablation"
```

---

### Task 5: Define security scan reuse key and file-level cache record

**Files:**
- Modify security scan controller/store/types under `crates/davinci-coding-agent/src/native_extensions/security_scan/`
- Prefer focused new file: `cache.rs` if no existing cache/store file owns this responsibility

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityFileCacheKey {
    pub content_sha256: String,
    pub ruleset_version: String,
    pub config_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileScan {
    pub key: SecurityFileCacheKey,
    pub findings: Vec<Finding>,
    pub candidates: Vec<Candidate>,
    pub coverage: FileCoverage,
}
```

Use existing finding/candidate/coverage types; do not duplicate them.

- [ ] **Step 1: Add stable-key test**

Same content/rules/config => identical key; any one change => different key.

- [ ] **Step 2: Run and confirm type/helper missing**

```bash
cargo test -p davinci-coding-agent security_file_cache_key_is_content_rules_and_config_sensitive
```

- [ ] **Step 3: Implement key from existing scanner version/config**

Ruleset version must be explicit/stable; if rules are compiled constants, hash their canonical identifiers/pattern versions rather than using build timestamp.

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(security): define deterministic file scan cache keys"
```

---

### Task 6: Reuse unchanged file scan evidence within a fresh scan run

**Files:**
- Modify security scan enumeration/controller/store

**Interfaces:**

At scan time:

```text
enumerate in-scope file
→ compute file hash/key
→ cache hit: clone deterministic file evidence into current run
→ cache miss: run current fixed rules and store file-level cache record
→ continue building current run's fresh aggregate artifacts
```

The cache may live under the existing scanner storage root; do not create repo-local mutable artifacts that bypass current immutable run sealing.

- [ ] **Step 1: Add cold/warm scan regression**

Scan fixture repo twice unchanged. Assert second run reports reused file count > 0 and final `scan-manifest.json` has a new run identity/seal.

- [ ] **Step 2: Add changed-file regression**

Modify one file; assert only that file misses reuse while unchanged files reuse.

- [ ] **Step 3: Run and confirm current scanner rescans all files**

```bash
cargo test -p davinci-coding-agent security_scan_reuses_unchanged_files_but_reseals_run
cargo test -p davinci-coding-agent security_scan_invalidates_changed_file_cache_entry
```

- [ ] **Step 4: Implement cache lookup/write fail-open**

If cache is unreadable/corrupt, scan file normally. Cache failure must never fail a healthy scan.

- [ ] **Step 5: Preserve redaction and lifecycle invariants**

Cached findings/candidates must already be redacted using the same deterministic representation persisted in normal artifacts. Never cache raw secret evidence.

- [ ] **Step 6: Run security lifecycle tests and commit**

```bash
cargo test -p davinci-coding-agent security_scan
cargo test -p davinci-coding-agent --test security_scan_lifecycle
git commit -am "feat(security): reuse unchanged file scan evidence"
```

---

### Task 7: Add security incremental ablation telemetry

**Files:**
- Modify security telemetry/status types
- Modify `davinci-evals` or security test harness

**Interfaces:**

Report:

```text
filesScannedCold
filesReused
filesRescanned
cacheReadErrors
cacheWriteErrors
```

- [ ] **Step 1: Add telemetry test for one cold + one warm + one changed-file run**
- [ ] **Step 2: Add deterministic ablation asserting identical findings/coverage between cold and warm runs**
- [ ] **Step 3: Run and commit subsystem boundary**

```bash
cargo test -p davinci-coding-agent security_scan_incremental_telemetry
cargo test -p davinci-evals security_scan_incremental
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "test(security): verify incremental scan equivalence"
```
