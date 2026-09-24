# Phase 5: Agent Runtime Implementation Plan (`davinci-agent`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The agent loop cannot panic on user text, cannot be left "streaming" after an error, cannot run forever, cannot hang on a glob or a grandchild process, and an accepted plan no longer turns the agent read-only.

**Architecture:** Local fixes inside `crates/davinci-agent`. The fallback shell path uses `davinci_sys::process` (Task 1.3). Turn cleanup moves into one function called from `run_loop_inner` for every error.

**Tech Stack:** Rust 1.83.

Depends on Phase 1 (Task 1.3). Task 5.2 depends on decision D3 in `00-index.md`.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-agent/src/templates.rs:208-217` | 5.1 |
| `crates/davinci-agent/src/runtime/contract_executor.rs:25-50, 290-303` | 5.2 |
| `crates/davinci-agent/src/turn.rs:2940-2966, 3053-3087` | 5.2 |
| `crates/davinci-agent/src/planning.rs:46-86, 131-145` | 5.3 |
| `crates/davinci-agent/src/turn.rs:78-99` (`run_loop_inner`), `:245-262` | 5.4 |
| `crates/davinci-agent/src/tools.rs:3306-3331` | 5.5 |
| `crates/davinci-agent/src/turn.rs:1145-1157, 1394, 1598` | 5.6 |
| `crates/davinci-agent/src/subagent.rs:335-720` | 5.7 |
| `crates/davinci-agent/src/turn.rs:691-696` | 5.8 |
| `crates/davinci-agent/src/turn.rs:154-536`, `lib.rs` (settings field) | 5.9 |
| `crates/davinci-agent/src/turn.rs:3815-3831`, `batch.rs:248-258`, `shell_policy.rs` | 5.10 |
| `crates/davinci-agent/src/tools.rs:1985-2045` | 5.11 |
| `crates/davinci-agent/src/jobs.rs:397-403, 659-690` | 5.12 |
| `crates/davinci-agent/src/tool_ledger.rs:360-381, 861-880` | 5.13 |
| `crates/davinci-agent/src/runtime/rewind.rs:243-256, 420-435` | 5.14 |
| `crates/davinci-agent/src/runtime/tasks.rs:1837-1868` | 5.15 |
| `crates/davinci-agent/src/runtime/contracts.rs:140-152, 712-724` | 5.16 |
| `crates/davinci-agent/src/runtime/cache/workspace.rs:28-40` | 5.17 |

---

### Task 5.1: A slash command followed by a multi-byte space no longer panics

**Finding fixed:** davinci-agent 1 (checked in source). `expand_prompt_template` (`templates.rs:214-216`) slices `rest[index + 1..]` where `index` is a byte offset of a whitespace char. NBSP (U+00A0, 2 bytes), the IME full-width space (U+3000, 3 bytes) and U+2003 put `index + 1` inside the character, so `"/review\u{3000}foo"` panics before template lookup. RPC reaches this through `expand_user_text_with_metadata` (`davinci-coding-agent/src/rpc.rs:285`).

**Files:**
- Modify: `crates/davinci-agent/src/templates.rs:214-217`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn multibyte_whitespace_after_the_command_does_not_panic() {
        let templates = vec![PromptTemplate {
            name: "review".into(),
            content: "Review: $ARGUMENTS".into(),
            ..PromptTemplate::default()
        }];
        for text in ["/review\u{3000}foo", "/review\u{a0}foo", "/review\u{2003}foo"] {
            assert_eq!(expand_prompt_template(text, &templates), "Review: foo", "{text:?}");
        }
    }
```

Build `PromptTemplate` the way the existing template tests do (the fields above are illustrative; copy an existing test's constructor and change only the text).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib templates::tests::multibyte_whitespace`
Expected: FAIL with `byte index … is not a char boundary`.

- [ ] **Step 3: Implement**

```rust
    let rest = &text[1..];
    let (name, args_string) = match rest.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.to_string()),
        None => (rest, String::new()),
    };
```

Run `rg -n "find\(\|c: char\| c.is_whitespace\(\)\)|\[index \+ 1\.\.\]" crates` and fix any other site with the same byte-offset pattern the same way.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib templates`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/templates.rs
git commit -m "fix(templates): split slash commands on any whitespace without byte slicing"
```

---

### Task 5.2 (DECISION D3): An accepted plan no longer blocks built-in file edits; shell under a contract follows the chosen policy

**Finding fixed:** davinci-agent 2 (checked in source). `check_contract_dispatch_boundary` (`turn.rs:3053-3087`) builds a `ContractExecutor` with `ExecutorCapabilities::default()` (all false; `full_sandbox()` is `#[cfg(test)]`, `contract_executor.rs:40-50`). Every tool that declares `FileSystemWrite` fails `can_enforce_fs`, so after `/plan accept` every `write`, `edit`, `apply_patch` and `notebook_edit` returns `execution_contract_unenforceable`, and every `bash`/`powershell` fails `process_isolation` (`contract_executor.rs:296-303`). The test at `turn.rs:~4881` pins this for an in-scope `src/ok.rs`. The verification reminder (`turn.rs:393-414`) then asks for tests that can never run.

**Part A (bug fix, no decision needed):** the comment at `turn.rs:2922-2926` states the intent: "Native file tools are constrained by `check_call`." For the host's own file tools, the host performs the write itself after `check_call` has verified every target against the contract's writable scope, so filesystem enforcement *is* that path check. Those tools pass the dispatch boundary. Custom, MCP and extension tools stay refused (their effects are not host-verified).

**Part B (D3, recommended option "ask"):** shell commands under a hard contract stop failing closed on the missing sandbox and go to the normal permission policy instead (Ask mode asks; Auto auto-allows only routine commands). Every contract rule that can be checked without a sandbox still refuses: network default-deny, redirection outside artifact roots, command substitution. The alternative ("keep refusing") leaves Part A only; then the verification reminder must be suppressed while a contract is active, and Step 6 below is replaced by that.

**Files:**
- Modify: `crates/davinci-agent/src/runtime/contract_executor.rs`
- Modify: `crates/davinci-agent/src/turn.rs:2940-2966, 3053-3087, ~4881`

**Interfaces:**
- Produces: `ContractExecutor::allowing_unconfined_shell(self) -> Self`

- [ ] **Step 1: Change the pinned test and add the new ones**

Change the test near `turn.rs:4881` (search for `src/ok.rs` and `execution_contract_unenforceable`) so an in-scope `write` of `src/ok.rs` under an accepted contract **succeeds**. Add beside it:

```rust
    #[test]
    fn contract_still_refuses_out_of_scope_native_writes() {
        let (mut agent, dir) = agent_with_accepted_plan(&["src/"]);
        let result = run_single_tool(&mut agent, dir.path(), "write",
            serde_json::json!({"path": "docs/x.md", "content": "x"}));
        assert!(result.is_error);
        assert!(result.content.contains("scope"), "{}", result.content);
    }

    #[test]
    fn contract_refuses_custom_tools_without_a_verified_effect_profile() {
        let (mut agent, dir) = agent_with_accepted_plan(&["src/"]);
        let result = run_single_tool(&mut agent, dir.path(), "mcp__fs__write_file",
            serde_json::json!({"path": "src/x.rs"}));
        assert!(result.is_error);
    }

    #[test]
    fn contract_shell_goes_to_the_permission_policy_but_network_is_still_refused() {
        let (mut agent, dir) = agent_with_accepted_plan(&["src/"]);
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        let ok = run_single_tool(&mut agent, dir.path(), "bash",
            serde_json::json!({"command": "echo hi"}));
        assert!(!ok.is_error, "{}", ok.content);
        let net = run_single_tool(&mut agent, dir.path(), "bash",
            serde_json::json!({"command": "curl https://example.com"}));
        assert!(net.is_error);
    }
```

`agent_with_accepted_plan` and `run_single_tool` must wrap what the existing f05 tests (`turn.rs:4767`, `:4937`) already do to install a contract and dispatch one call; extract those setups into these two helpers.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib contract`
Expected: the changed test and the shell test FAIL.

- [ ] **Step 3: Implement Part A**

In `check_contract_dispatch_boundary`, after computing `effects`, return early for host-native file tools:

```rust
        // Host-native file tools write through the host after `check_call`
        // proved every target is inside the contract's writable scope; that
        // path check is the filesystem enforcement. Tools the host does not
        // implement still need a backend that can confine them.
        let host_native_file_tool = crate::tools::BUILTIN_TOOLS.contains(&name)
            && effects
                .iter()
                .all(|effect| matches!(effect, crate::runtime::DeclaredEffect::FileSystemWrite));
        if host_native_file_tool {
            return Ok(());
        }
```

Confirm `check_call` runs for these tools before dispatch (it is the `ScopeViolation` path at `turn.rs:~1690-1708`); if a native file tool reaches dispatch without `check_call`, route it through `check_call` here instead of returning `Ok(())`.

- [ ] **Step 4: Implement Part B**

`contract_executor.rs`: add a private field `allow_unconfined_shell: bool` (default `false`) and

```rust
    /// The ordinary host has no process sandbox. With this set, shell
    /// commands are not refused for that reason alone: the permission
    /// policy decides, and every statically checkable contract rule
    /// (network, redirection, substitution) still applies.
    pub fn allowing_unconfined_shell(mut self) -> Self {
        self.allow_unconfined_shell = true;
        self
    }
```

In `execute_shell`, change the isolation refusal to:

```rust
        if !self.allow_unconfined_shell
            && (!self.capabilities.can_enforce_process || !self.capabilities.process_isolation)
        {
            return Err(ExecutionError::ContractUnenforceable(
                "backend cannot enforce process isolation for contracted shell execution".into(),
            ));
        }
```

Move this check **after** the network, redirection and substitution checks if it is currently before them, so those rules still run when it is skipped.

`turn.rs:~2952-2958`: build the shell executor with `.allowing_unconfined_shell()`.

Update the comment at `turn.rs:2922-2926` to describe the new rule.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p davinci-agent`
Expected: PASS. The `full_sandbox` executor tests in `contract_executor.rs` are unchanged.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-agent
git commit -m "fix(contracts): let host file tools and permission-gated shell run under an accepted plan"
```

---

### Task 5.3: A plan that cannot compile to a contract is refused, and a stale contract never survives

**Finding fixed:** davinci-agent 3. `planning.rs:48, 62-66, 80-84` use `if let Ok(c) = compile_plan_contract(..)` and drop the error. When compilation fails and `return_to_plan` is false, the previous revision's `active_contract` stays installed (`planning.rs:131-143`). A step whose `files` contains `src/lib.rs:120`, an absolute path or `..` fails `normalize_relative_path` (the `:` reads as an alternate data stream), and the new revision runs under the old scope.

**Files:**
- Modify: `crates/davinci-agent/src/planning.rs:46-86, 131-145`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn accepting_a_plan_that_cannot_compile_is_refused_and_clears_nothing_silently() {
        let mut agent = planning_fixture_agent();
        agent.load_plan_steps(&[("s1", &["src/ok.rs"])]);
        agent.handle_plan_command("accept auto").unwrap();
        assert!(agent.active_contract().is_some());
        agent.handle_plan_command("").unwrap(); // back to Plan Mode
        agent.load_plan_steps(&[("s1", &["src/lib.rs:120"])]);
        let err = agent.handle_plan_command("accept auto").unwrap_err();
        assert!(err.contains("src/lib.rs:120"), "{err}");
        assert!(agent.active_contract().is_none());
    }
```

Use the planning test helpers that exist in `planning.rs` / `living_plan` tests to create a plan with steps and file lists; `load_plan_steps` above stands for that setup.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib accepting_a_plan_that_cannot_compile`
Expected: FAIL (accept succeeds).

- [ ] **Step 3: Implement**

Replace each `if let Ok(c) = compile_plan_contract(..) { compiled_contract = Some(c); }` with

```rust
                compiled_contract = Some(
                    crate::runtime::contracts::compile_plan_contract(&next, step_filter, None)
                        .map_err(|error| {
                            format!("The plan cannot be turned into an execution contract: {error}. Fix the step's file list (paths relative to the project, no line numbers) and accept again.")
                        })?,
                );
```

(with the right `step_filter` / `None` per call site). Because the `?` returns before `*living_plan = next`, a refused accept changes nothing. For the path where acceptance succeeds but produces no contract, set `active_contract = None` explicitly so an older contract cannot outlive its plan revision:

```rust
        *self.tool_context.active_contract.lock().unwrap_or_else(|e| e.into_inner()) = compiled_contract;
```

(replacing the `if let Some(contract) … else if return_to_plan …` block, since `compiled_contract` is now `Some` for every accepting path and `None` otherwise).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib planning`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/planning.rs
git commit -m "fix(plan): refuse plans that do not compile to a contract; never keep a stale contract"
```

---

### Task 5.4: Every failed turn resets streaming state and closes dangling tool calls

**Finding fixed:** davinci-agent 5. `persist_chat(..)?` / `inject_job_notices(..)?` (`turn.rs:174, 457, 480`) return `Err` without resetting state; `run_loop_inner` (`turn.rs:88-99`) resets only when `ensure_session_persistence()` fails afterwards. The mandatory-context early return (`turn.rs:251-260`) skips `emit_turn_end`. After such an error: `is_streaming` stays true (RPC treats every later prompt as steer/follow-up, `/plan` refuses, `!bash` output is buffered forever), and an assistant message with tool calls may have no tool results, which the next provider request rejects.

**Files:**
- Modify: `crates/davinci-agent/src/turn.rs:78-99, 251-260`

**Interfaces:**
- Produces: `fn fail_turn(&mut self, error: &str)` (private)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_turn_that_errors_midway_leaves_the_agent_idle_and_consistent() {
        let mut agent = fixture_agent_with_failing_persistence_after(1);
        let result = agent.run_prompt_with_fixture_tool_call("read", serde_json::json!({"path":"a"}));
        assert!(result.is_err());
        assert!(!agent.is_streaming);
        // Every assistant tool call has a matching tool result.
        let calls: Vec<String> = agent.messages.iter()
            .filter(|m| m.role == "assistant")
            .flat_map(|m| m.content.iter().filter_map(|c| match c {
                davinci_ai::MessageContent::ToolCall { id, .. } => Some(id.clone()),
                _ => None,
            }))
            .collect();
        for id in calls {
            assert!(agent.messages.iter().any(|m| m.role == "toolResult" && m.tool_call_id.as_deref() == Some(&id)), "{id}");
        }
    }
```

`session_persistence_tests.rs` (declared at `turn.rs:3833-3835`) already builds agents whose persistence fails; build this fixture there (e.g. make the session file read-only after the first append) and drive one fixture turn that returns a tool call.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib a_turn_that_errors_midway`
Expected: FAIL (`is_streaming` still true).

- [ ] **Step 3: Implement**

```rust
    /// Leave the agent idle after any failed turn: not streaming, buffered
    /// `!bash` output flushed, the runtime told the turn failed, and every
    /// assistant tool call answered so the next request is well-formed.
    fn fail_turn(&mut self, error: &str) {
        let answered: std::collections::HashSet<String> = self
            .messages
            .iter()
            .filter(|message| message.role == "toolResult")
            .filter_map(|message| message.tool_call_id.clone())
            .collect();
        let dangling: Vec<(String, String)> = self
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                davinci_ai::MessageContent::ToolCall { id, name, .. } if !answered.contains(id) => {
                    Some((id.clone(), name.clone()))
                }
                _ => None,
            })
            .collect();
        for (id, name) in dangling {
            // In memory only: if persistence failed, writing is what broke.
            self.messages.push(tool_result_message(
                &id,
                &name,
                &crate::ToolResult {
                    content: format!("The turn stopped before this tool result was recorded: {error}"),
                    is_error: true,
                    details: None,
                },
            ));
        }
        self.is_streaming = false;
        self.flush_pending_bash_messages();
        if let Some(runtime) = &self.runtime {
            runtime.mark_turn_failed();
            runtime.emit_turn_end(false);
        }
    }
```

`tool_result_message` is the builder at `turn.rs:3709`; match its real parameters.

`run_loop_inner` becomes:

```rust
        self.ensure_session_persistence()?;
        self.recover_pending_operation_publications()?;
        let result = self.run_loop_body(emit_prompt_messages, complete);
        let persistence = self.ensure_session_persistence();
        match (&result, persistence) {
            (_, Err(error)) => {
                self.fail_turn(&error);
                Err(error)
            }
            (Err(error), Ok(())) => {
                if self.is_streaming {
                    self.fail_turn(error);
                }
                result
            }
            (Ok(_), Ok(())) => result,
        }
```

In the mandatory-context early return (`turn.rs:251-260`) delete the manual `self.is_streaming = false; self.flush_pending_bash_messages();` lines; `run_loop_inner` now does it and also emits `turn_end`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/turn.rs crates/davinci-agent/src/session_persistence_tests.rs
git commit -m "fix(agent): reset state and answer dangling tool calls after any failed turn"
```

---

### Task 5.5: Glob matching runs in polynomial time

**Finding fixed:** davinci-agent 6. `match_glob_chars` (`tools.rs:3306-3331`) recurses on `*` with two branches and on `**` with three, with no memo. `*a*a*a*a*a*a*a*a*a*b` against a 40-character name of `a`s takes about C(40,9) ≈ 2.7e8 steps per file, and `find`/`grep` call it per file with no abort check.

**Behavior:** same semantics (in this matcher `*` also crosses `/`; keep that), memoized over `(pattern index, name index)`, so the cost is at most `|pattern| × |name|` cells.

**Files:**
- Modify: `crates/davinci-agent/src/tools.rs:3306-3331`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn pathological_glob_finishes_quickly() {
        let name = "a".repeat(40);
        let started = std::time::Instant::now();
        assert!(!match_glob_chars("*a*a*a*a*a*a*a*a*a*b", &name));
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn glob_semantics_are_unchanged() {
        for (pattern, name, expected) in [
            ("*.rs", "src/lib.rs", true),
            ("src/**/*.rs", "src/a/b/c.rs", true),
            ("src/**/*.rs", "src/c.rs", true),
            ("?.md", "a.md", true),
            ("?.md", "ab.md", false),
            ("**", "anything/at/all", true),
            ("a*b", "a/x/b", true),
        ] {
            assert_eq!(match_glob_chars(pattern, name), expected, "{pattern} {name}");
        }
    }
```

Run the second test against the **current** code first and adjust any row whose expected value differs from today's behavior (the task must not change semantics).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib pathological_glob`
Expected: FAIL (seconds, not milliseconds).

- [ ] **Step 3: Implement**

```rust
fn match_glob_chars(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // memo[i * (n.len() + 1) + j]: does p[i..] match n[j..]? Each cell is
    // computed once, so cost is O(|p| * |n|) whatever the number of stars.
    let mut memo = vec![None; (p.len() + 1) * (n.len() + 1)];
    fn rec(p: &[char], n: &[char], i: usize, j: usize, memo: &mut [Option<bool>]) -> bool {
        let key = i * (n.len() + 1) + j;
        if let Some(known) = memo[key] {
            return known;
        }
        let result = match (p.get(i), n.get(j)) {
            (None, None) => true,
            (Some('*'), _) if p.get(i + 1) == Some(&'*') => {
                let slash = p.get(i + 2) == Some(&'/');
                let rest = if slash { i + 3 } else { i + 2 };
                rec(p, n, rest, j, memo)
                    || (j < n.len() && rec(p, n, i, j + 1, memo))
                    || (slash && n.get(j) == Some(&'/') && rec(p, n, i + 3, j + 1, memo))
            }
            (Some('*'), _) => {
                rec(p, n, i + 1, j, memo) || (j < n.len() && rec(p, n, i, j + 1, memo))
            }
            (Some('?'), Some(_)) => rec(p, n, i + 1, j + 1, memo),
            (Some(a), Some(b)) if a == b => rec(p, n, i + 1, j + 1, memo),
            _ => false,
        };
        memo[key] = Some(result);
        result
    }
    rec(&p, &n, 0, 0, &mut memo)
}
```

`i` never exceeds `p.len()` (`i + 3` is used only when `p.get(i + 2)` exists), so every key is in range. Recursion depth is at most `|p| + |n|`, a few hundred for real paths.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib tools`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/tools.rs
git commit -m "fix(tools): memoize glob matching so crafted patterns cannot hang find and grep"
```

---

### Task 5.6: `agent` calls that can write in the shared checkout run serially

**Finding fixed:** davinci-agent 7. `agent` is `ParallelSafe` (`runtime/capabilities.rs:118-122, 757-761`) and `lane_for` says "read-only by construction" (`scheduler.rs:42-43`), but `subagent.rs:377-380, 423` hands mutation tools to workers in Edits / Auto / Always Approve. Two shared-cwd `agent` calls in one message, or `agent` beside `read`, run on up to 8 threads while workers write the same files.

**Files:**
- Modify: `crates/davinci-agent/src/turn.rs:1145-1157` (and its two callers at `:1394`, `:1598`)
- Modify: `crates/davinci-agent/src/scheduler.rs:42-43` (comment only)

**Interfaces:**
- Produces: `fn lane_for_call(&self, name: &str, class: ToolClass, args: &Value) -> ToolLane` (replaces `lane_for_tool` at both call sites)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn writable_shared_agent_calls_are_serial() {
        let mut agent = fixture_agent();
        agent.set_permission_mode(PermissionMode::Auto);
        let shared = serde_json::json!({"prompt": "edit things"});
        let worktree = serde_json::json!({"prompt": "edit things", "isolation": "worktree"});
        assert_eq!(agent.lane_for_call("agent", ToolClass::Other, &shared), ToolLane::Serial);
        assert_eq!(agent.lane_for_call("agent", ToolClass::Other, &worktree), ToolLane::Parallel);
        agent.set_permission_mode(PermissionMode::Ask);
        assert_eq!(agent.lane_for_call("agent", ToolClass::Other, &shared), ToolLane::Parallel);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib writable_shared_agent_calls`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
    /// The lane for one call. `agent` is parallel only when no task can
    /// write in the shared checkout: workers get mutation tools in Edits,
    /// Auto and Always Approve (subagent.rs), unless they run in their own
    /// worktree.
    fn lane_for_call(
        &self,
        name: &str,
        class: crate::permission::ToolClass,
        args: &Value,
    ) -> crate::scheduler::ToolLane {
        if name == "agent" && agent_call_may_write_shared(args, self.permission_mode()) {
            return crate::scheduler::ToolLane::Serial;
        }
        self.lane_for_tool(name, class)
    }
```

```rust
fn agent_call_may_write_shared(args: &Value, mode: PermissionMode) -> bool {
    if !matches!(mode, PermissionMode::Edits | PermissionMode::Auto | PermissionMode::AlwaysApprove) {
        return false;
    }
    let tasks: Vec<&Value> = match args.get("tasks").and_then(Value::as_array) {
        Some(tasks) => tasks.iter().collect(),
        None => vec![args],
    };
    tasks
        .iter()
        .any(|task| task.get("isolation").and_then(Value::as_str) != Some("worktree"))
}
```

Replace `self.lane_for_tool(name, class)` with `self.lane_for_call(name, class, args)` at `turn.rs:1394` and `:1598` (both have `args` in scope). Update the `scheduler.rs:42` comment to: `// Parallel by default; Agent::lane_for_call makes writable shared-checkout workers serial.`

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/turn.rs crates/davinci-agent/src/scheduler.rs
git commit -m "fix(agent): serialize subagent calls that can write the shared checkout"
```

---

### Task 5.7: Subagent launch rolls back on partial failure; mixed oneshot and background batches are refused

**Findings fixed:** davinci-agent 8 and 9.
- Workers are registered and set `Running` (`subagent.rs:495-496`) and leases created (`:446-449`) inside the loop; if a later spec's `create_lease`, `for_worker` (`:504`), durable admission (`:559`) or thread spawn (`:598-600`) fails, the function returns early and the earlier agents stay `Running` forever, their worktrees never released.
- If any task is `background` or `teammate`, every request, including `oneshot` ones, is spawned detached (`:567-644`); the oneshot answers never reach the model.

**Files:**
- Modify: `crates/davinci-agent/src/subagent.rs:335-720`

**Interfaces:**
- Produces: private `struct LaunchRollback<'a> { registry: Option<&'a AgentRegistry>, agents: Vec<AgentId>, leases: Vec<(WorktreeManager, WorktreeLease)>, committed: bool }` with `Drop`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn mixed_oneshot_and_background_batch_is_refused_before_launch() {
        let (runner, parent) = fixture_runner_and_parent();
        let input = serde_json::json!({"tasks": [
            {"prompt": "a", "mode": "oneshot"},
            {"prompt": "b", "mode": "background"}
        ]});
        let err = run_tool(&input, &[], Some(&runner), &parent).unwrap_err().to_string();
        assert!(err.contains("separate agent calls"), "{err}");
        assert_eq!(parent.runtime.as_ref().unwrap().registry.count_running(), 0);
    }

    #[test]
    fn a_launch_failure_leaves_no_running_agents_or_leases() {
        let (runner, parent) = fixture_runner_and_parent_failing_admission_on(2);
        let input = serde_json::json!({"tasks": [
            {"prompt": "a", "isolation": "worktree"},
            {"prompt": "b", "isolation": "worktree"}
        ]});
        assert!(run_tool(&input, &[], Some(&runner), &parent).is_err());
        let runtime = parent.runtime.as_ref().unwrap();
        assert_eq!(runtime.registry.count_running(), 0);
        assert_eq!(runtime.worktree_manager.as_ref().unwrap().active_leases(), 0);
    }
```

Build the fixtures from the existing subagent tests (search `run_tool(` in `subagent.rs` tests). `count_running` / `active_leases`: use the registry and manager query methods that exist; add a small `#[cfg(test)]` query if none does.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib subagent::tests::mixed subagent::tests::a_launch_failure`
Expected: FAIL.

- [ ] **Step 3: Implement the refusal**

Right after `specs` is built:

```rust
    let async_count = specs.iter().filter(|spec| spec.mode != AgentSpawnMode::Oneshot).count();
    if async_count > 0 && async_count < specs.len() {
        return Err(ToolError::Failed(
            "This batch mixes oneshot tasks with background/teammate tasks. Send them as separate agent calls: oneshot results are returned, background ones are only spawned.".into(),
        ));
    }
```

- [ ] **Step 4: Implement the rollback**

```rust
/// Undo a partially launched batch: agents registered as Running become
/// Failed, and leases created for them are released. `commit()` disarms it
/// once every worker is launched.
struct LaunchRollback<'a> {
    registry: Option<&'a crate::runtime::AgentRegistry>,
    agents: Vec<AgentId>,
    leases: Vec<(WorktreeManager, WorktreeLease)>,
    committed: bool,
}

impl LaunchRollback<'_> {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for LaunchRollback<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Some(registry) = self.registry {
            for id in &self.agents {
                let _ = registry.transition(*id, AgentState::Failed);
            }
        }
        for (manager, lease) in &self.leases {
            // Never started: nothing in the worktree worth keeping.
            let _ = manager.release_lease(lease, false);
        }
    }
}
```

Create it before the per-spec loop (`registry: parent.runtime.as_ref().map(|rt| &rt.registry)`), push each registered agent id right after `register_agent`, push `(mgr.clone(), lease.clone())` right after `create_lease` succeeds, and call `rollback.commit()` immediately before the first worker thread is spawned (async path) or before the first synchronous run (oneshot path). Use the real registry type and `release_lease` signature; if `WorktreeManager`/`WorktreeLease` are not `Clone`, store what `release_lease` needs.

Leases of workers that *ran and failed* are still kept (their worktree may hold useful partial work). That retention without a sweeper is deferred item X4.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p davinci-agent --lib subagent`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-agent/src/subagent.rs
git commit -m "fix(subagent): roll back partial launches and refuse mixed oneshot/background batches"
```

---

### Task 5.8: `maxRetries = N` gives N retries

**Finding fixed:** davinci-agent 10 (checked in source). `turn.rs:691-696` uses `attempts = retry_attempts.max(1)` as the total number of attempts, but `main.rs:825` sets `retry_attempts` from `settings.retry_max_retries()`. `maxRetries = 3` gives 2 retries while `AutoRetryStart.max_attempts` reports 3; `maxRetries = 1` gives none.

**Files:**
- Modify: `crates/davinci-agent/src/turn.rs:691-696`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn max_retries_counts_retries_not_attempts() {
        for (max_retries, expected_calls) in [(0, 1), (1, 2), (3, 4)] {
            let mut agent = fixture_agent();
            agent.auto_retry = true;
            agent.retry_attempts = max_retries;
            agent.retry_base_delay_ms = 0;
            let calls = std::cell::Cell::new(0);
            let _ = agent.run_with_complete(|_| {
                calls.set(calls.get() + 1);
                Err::<davinci_ai::AssistantMessage, _>("HTTP 503 Service Unavailable".to_string())
            });
            assert_eq!(calls.get(), expected_calls, "maxRetries={max_retries}");
        }
    }
```

`run_with_complete` stands for the test entry that drives the retry loop with a closure (`complete` in `run_loop_inner`); use the one the existing auto-retry tests use.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib max_retries_counts_retries`
Expected: FAIL (1, 1, 3 calls).

- [ ] **Step 3: Implement**

```rust
        let max_retries = if self.auto_retry { self.retry_attempts } else { 0 };
        // One first attempt plus `max_retries` retries.
        let attempts = max_retries.saturating_add(1);
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent`
Expected: PASS. Any test that asserted the old count changes to the new one.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/turn.rs
git commit -m "fix(agent): maxRetries counts retries, not total attempts"
```

---

### Task 5.9: A run stops after a configurable number of model turns

**Finding fixed:** davinci-agent 13. The loop (`turn.rs:154-536`) continues whenever `had_tools && !abort`; no max-turns setting exists outside evals. With no budget enforcement (Task 11.5 / D5) and no watchdog, a model stuck in a loop, including an unattended background worker, runs until context or credits run out.

**Behavior:** `Agent.max_model_turns: Option<u32>`. Default `Some(200)` for the main agent and `Some(60)` for workers (set where workers are built in `subagent.rs`). Setting `maxModelTurns` in settings overrides the main default; `0` means unlimited. On reaching the cap the loop ends the run with an assistant-visible notice: `Stopped after N model turns (maxModelTurns). Send a message to continue.`

**Files:**
- Modify: `crates/davinci-agent/src/lib.rs` (field + default)
- Modify: `crates/davinci-agent/src/turn.rs:154-265` (check at the top of each model turn, next to `self.stats.model_turns += 1`)
- Modify: `crates/davinci-agent/src/subagent.rs` (worker default)
- Modify: `crates/davinci-coding-agent/src/settings.rs` (`maxModelTurns`) and `main.rs:~825` (apply it)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn the_loop_stops_at_max_model_turns() {
        let mut agent = fixture_agent_that_always_calls_a_tool("read");
        agent.max_model_turns = Some(3);
        let events = agent.run_prompt("go").unwrap();
        assert_eq!(agent.stats.model_turns, 3);
        assert!(events.iter().any(|event| matches!(event,
            AgentEvent::Notice { text } if text.contains("maxModelTurns"))));
        assert!(!agent.is_streaming);
    }
```

Use the event variant the agent already uses for system notices (search `AgentEvent::` for a notice/status variant); if none fits, push an assistant `ChatMessage` with the notice text instead and assert on that.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib the_loop_stops_at_max_model_turns`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

At the top of each loop iteration, before `self.stats.model_turns += 1`:

```rust
            if let Some(cap) = self.max_model_turns.filter(|cap| *cap > 0) {
                if turns_this_run >= cap {
                    self.push_notice(&mut events, format!(
                        "Stopped after {cap} model turns (maxModelTurns). Send a message to continue."
                    ));
                    // Same exit as an abort: turn_end, AgentEnd, idle.
                    return Ok(self.finish_run(events, new_messages));
                }
            }
            turns_this_run += 1;
```

`turns_this_run` is a local counter initialised to 0 before `loop {`, so the cap is per run, not per session. `finish_run` is the abort branch's body (`turn.rs:155-169`) extracted into a method so both exits share it.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent` and `cargo test -p davinci-coding-agent --lib settings`
Expected: PASS.

- [ ] **Step 5: Document and commit**

Add `maxModelTurns` to the settings reference doc (wherever `retry.maxRetries` is documented; `rg -n "maxRetries" docs README.md`).

```bash
git add crates/davinci-agent crates/davinci-coding-agent docs README.md
git commit -m "feat(agent): cap model turns per run (maxModelTurns, workers default lower)"
```

---

### Task 5.10: Verification detection parses the command instead of searching substrings

**Finding fixed:** davinci-agent 16. `is_verification_command` (`turn.rs:3815-3831`) is a substring search: `echo cargo test` and `cargo test --no-run` count, `cargo test || true` counts as **passing**, and `npx vitest`, `jest`, `tsc`, `cargo nextest` are not recognised, so the "changed files but not verified" reminder fires after real tests.

**Files:**
- Modify: `crates/davinci-agent/src/shell_policy.rs` (add `verification_outcome`)
- Modify: `crates/davinci-agent/src/turn.rs:2380-2386, 3815-3831`, `batch.rs:248-258`

**Interfaces:**
- Produces: `pub fn verification_outcome(command: &str) -> Option<bool>`: `None` = not a verification command; `Some(true)` = verification whose exit status reflects the tests; `Some(false)` = verification whose exit status is masked (`||`, or piped into another command).

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn verification_commands_are_recognised_by_their_program() {
        for (command, expected) in [
            ("cargo test", Some(true)),
            ("cargo test --workspace -- --nocapture", Some(true)),
            ("cargo nextest run", Some(true)),
            ("npx vitest run", Some(true)),
            ("npx jest", Some(true)),
            ("tsc --noEmit", Some(true)),
            ("pytest -q", Some(true)),
            ("cargo test 2>&1 | tail -20", Some(false)),
            ("cargo test || true", Some(false)),
            ("echo cargo test", None),
            ("cargo test --no-run", None),
            ("ls", None),
        ] {
            assert_eq!(verification_outcome(command), expected, "{command}");
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib verification_commands_are_recognised`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`shell_policy.rs`:

```rust
/// Whether `command` runs a test/check program, and whether its exit status
/// can be trusted as the test result. Uses the same program patterns as
/// `is_test`, anchored at the start of a segment, so `echo cargo test` is
/// not a verification.
pub fn verification_outcome(command: &str) -> Option<bool> {
    let report = analyze_command(command);
    let test_position = report
        .segments
        .iter()
        .position(|segment| test_regex().is_match(segment) && !segment.contains("--no-run"))?;
    let status_masked = command.contains("||") || test_position + 1 != report.segments.len();
    Some(!status_masked)
}
```

Add `nextest` to `TEST_PATTERNS` (the cargo entry already lists `nextest`; confirm `cargo nextest run` matches, and add `r"(?i)^\s*cargo\s+nextest\b"` if not).

`turn.rs`: replace the body of `is_verification_command` with `crate::shell_policy::verification_outcome(cmd).is_some()`, and where the result is recorded (`turn.rs:~2385`, `batch.rs:~253`):

```rust
            if let Some(trustworthy) = crate::shell_policy::verification_outcome(command) {
                agent.record_verification_command(
                    command,
                    trustworthy && !pre_hook_error && !result.is_error,
                );
            }
```

A masked run counts as "ran, result unknown": recorded as not passed, so the reminder still asks for a clean run.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent
git commit -m "fix(agent): detect verification by program, not substring; masked exit status is not a pass"
```

---

### Task 5.11: The fallback shell path kills the whole process group and never waits forever on pipes

**Finding fixed (SUSPECTED, applies to embedders):** davinci-agent 17. When `foreground_supervisor` is `None` (library users; `main.rs:748` sets it for the CLI), `tools.rs:2011-2016` kills only the shell, and `join_shell_stream` then waits for EOF on pipes a grandchild still holds (`npm run dev &`, a watcher). Timeout or abort never returns; the same after a normal exit.

**Files:**
- Modify: `crates/davinci-agent/src/tools.rs` (the spawn site that feeds this function, and lines 1985-2045)

- [ ] **Step 1: Write the failing test (Unix)**

```rust
    #[cfg(unix)]
    #[test]
    fn fallback_shell_returns_when_a_grandchild_keeps_the_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let context = ToolContext::default(); // no foreground supervisor
        let started = std::time::Instant::now();
        let result = execute_tool_with(
            dir.path(),
            "bash",
            &serde_json::json!({"command": "sleep 30 & echo started", "timeout": 5}),
            &context,
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(8));
        assert!(result.unwrap().content.contains("started"));
    }
```

Use the real timeout argument name of the bash tool.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib fallback_shell_returns`
Expected: FAIL by hanging past the assertion (run with `timeout 60 cargo test ...` so the suite does not hang).

- [ ] **Step 3: Implement**

At the spawn site, call `davinci_sys::process::set_own_process_group(&mut command)` before `spawn()`. In the timeout/abort branch replace `let _ = child.kill();` with:

```rust
                davinci_sys::process::kill_tree(child.id());
                let _ = child.kill();
```

Make `join_shell_stream` bounded: the reader threads send their result over a channel; wait at most 500 ms after the child exits, then `kill_tree(child.id())` and return what was read so far with a note `(output truncated: a background process kept the pipe open)`. The simplest correct change is to replace this function's body with `davinci_sys::process::run_bounded` if the call site can pass the `Command` instead of a spawned `Child`; prefer that, and keep `read_shell_stream`'s byte cap as `RunLimits::output_cap`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib tools`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/tools.rs
git commit -m "fix(tools): kill the shell's process group and bound pipe joins in the fallback path"
```

---

### Task 5.12: Finished background jobs release their output

**Finding fixed:** davinci-agent 18. `JobBook.jobs` only grows (`jobs.rs:397-403`) and each job keeps up to 4 MiB of output (`OUTPUT_CAP`, `jobs.rs:24`).

**Behavior:** after `take_unannounced` marks jobs announced, keep full output for the 16 most recent finished-and-announced jobs; for older ones, drop the output buffer and keep a one-line summary (id, command, status, elapsed), which is all `jobs` listing needs. `job_output` on a pruned job returns `Output of job N was released; rerun the command to see it again.`

**Files:**
- Modify: `crates/davinci-agent/src/jobs.rs:397-403, 659-690` and the `job_output` path

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn old_announced_jobs_release_their_output() {
        let book = fixture_book();
        for i in 0..20 {
            finish_fixture_job(&book, &format!("echo {i}"), "x".repeat(1024));
        }
        book.lock().unwrap().take_unannounced();
        let book = book.lock().unwrap();
        let kept = book.jobs.iter().filter(|job| job.output_len() > 0).count();
        assert_eq!(kept, 16);
        assert!(book.output_of(1).unwrap_err().contains("released"));
    }
```

(Build jobs the way `a_finished_job_is_announced_once_with_its_tail` at `jobs.rs:1046` does; add `output_len`/`output_of` accessors if the existing ones differ.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib old_announced_jobs`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
const KEEP_FINISHED_OUTPUT: usize = 16;

impl JobBook {
    /// Free the output of announced, finished jobs beyond the newest few.
    fn release_old_output(&mut self) {
        let mut finished: Vec<usize> = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| job.announced && !job.status().is_running())
            .map(|(index, _)| index)
            .collect();
        if finished.len() <= KEEP_FINISHED_OUTPUT {
            return;
        }
        finished.truncate(finished.len() - KEEP_FINISHED_OUTPUT);
        for index in finished {
            self.jobs[index].release_output();
        }
    }
}
```

`Job::release_output` clears the shared output buffer (`shared` holds it; clear the `Vec` and `shrink_to_fit`) and sets a `released: bool`. Call `release_old_output()` at the end of `take_unannounced`. The output accessor returns the "released" error when `released` is set.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --lib jobs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/jobs.rs
git commit -m "fix(jobs): release output of old announced jobs"
```

---

### Task 5.13: The tool ledger stays bounded

**Finding:** davinci-agent 19. Every completion stores the full output (`tool_ledger.rs:869`) and `persist()` re-serialises the whole ledger as pretty JSON with an fsync on every `record_start` (`:369-381`, `:861`). I/O grows quadratically over a long session.

**Budget:** a session of 2,000 tool calls with 20 KB outputs each writes less than 50 MB of ledger bytes in total (today: about 2,000 × 20 MB average = tens of GB). Measure first with the test in Step 1.

**Files:**
- Modify: `crates/davinci-agent/src/tool_ledger.rs:360-381, 861-880`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn ledger_size_stays_bounded_over_a_long_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = ToolLedger::persistent(dir.path().join("ledger.json")).unwrap();
        let output = "y".repeat(20_000);
        for i in 0..2_000 {
            let id = format!("call-{i}");
            ledger.record_start(&id, "read", &serde_json::json!({"path": i}));
            ledger.record_completion(&id, &output, false);
        }
        let size = std::fs::metadata(dir.path().join("ledger.json")).unwrap().len();
        assert!(size < 8 * 1024 * 1024, "{size}");
    }
```

(use the real constructor for a persistent ledger.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --release --lib ledger_size_stays_bounded`
Expected: FAIL (file ~40 MB; also slow).

- [ ] **Step 3: Implement**

- `record_completion` stores at most `MAX_STORED_OUTPUT = 64 * 1024` bytes of `output` (cut on a char boundary, with `"\n[output truncated in ledger; digest covers the full text]"` appended). `result_digest` still covers the full output.
- Keep at most `MAX_TERMINAL_RECORDS = 256` completed records; evict the oldest completed ones (the pruning loop at `:360-367` already removes ids; call it after each completion with that limit). In-flight and reserved records are never evicted.
- `persist()` writes compact JSON (`to_vec`, not `to_vec_pretty`).

Replay of an evicted record falls back to "not in ledger", which the replay code must treat as "cannot replay, run again or ask" (check `BeginOutcome` handling for a missing record; it must not treat missing as success).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-agent --release --lib ledger_size_stays_bounded` then `cargo test -p davinci-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/tool_ledger.rs
git commit -m "perf(ledger): cap stored outputs and terminal records, write compact JSON"
```

---

### Task 5.14: Rewind treats a missing blob as a conflict, never as "delete the file"

**Finding fixed (latent):** davinci-agent 20. `rewind.rs:425-432` maps a failed `get_blob` for a present `before_blob` hash to `before_bytes = None`; `plan_file` (`:247-256`) then classifies "task created the file" as `inverse`, and apply deletes it (`:637`). `BlobStore` is in-memory only. The current caller validates first, so this is latent, but one caller change away from deleting user files.

**Files:**
- Modify: `crates/davinci-agent/src/runtime/rewind.rs:420-435`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_missing_before_blob_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "user data").unwrap();
        let effects = vec![fixture_effect("a.txt", Some("hash-not-in-store"), Some("after-hash"))];
        let store = BlobStore::default();
        let plan = plan_rewind(&effects, &store, dir.path());
        assert_eq!(plan.files[0].action, "conflict");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib a_missing_before_blob`
Expected: FAIL (`inverse`).

- [ ] **Step 3: Implement**

Before calling the classifier:

```rust
        let missing_before = first.before_blob.is_some() && before_bytes.is_none();
        let missing_after = last.after_blob.is_some() && after_bytes.is_none();
        if missing_before || missing_after {
            plans.push(FileRewindPlan::new(
                path.replace('\\', "/"),
                "conflict",
                None,
                false,
                Some("the recorded file content is no longer available".into()),
            ));
            continue;
        }
```

(match `FileRewindPlan::new`'s real argument meaning; the last argument above is the reason slot if one exists.)

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-agent --lib rewind` → PASS.

```bash
git add crates/davinci-agent/src/runtime/rewind.rs
git commit -m "fix(rewind): missing blobs are conflicts, never deletions"
```

---

### Task 5.15: Task rewind is all-or-nothing

**Finding fixed (SUSPECTED):** davinci-agent 21. `tasks.rs:1837-1868` sets dependents to `Blocked` and changes the task's attempt and result in place, then `initial_task_state(..)?` or `advance_revision(..)?` can fail and leave the registry half-mutated; line 1852 ignores an `advance_revision` error.

**Files:**
- Modify: `crates/davinci-agent/src/runtime/tasks.rs:1837-1868`

- [ ] **Step 1: Write the failing test**

Drive the rewind with a task whose `initial_task_state` fails (use whatever input the existing tests use to make it fail, e.g. a missing definition), then assert the registry is byte-for-byte equal to its state before the call (`assert_eq!(before, after)` on a clone taken first; add `PartialEq` derive in `#[cfg_attr(test, derive(PartialEq))]` if needed).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib task_rewind_is_atomic`
Expected: FAIL.

- [ ] **Step 3: Implement**

Clone the registry state the function mutates into `next`, perform every mutation on `next`, propagate every error with `?` (including line 1852's `advance_revision`), and assign `*self.state = next` only at the end. If the state is large, clone only the task map and the dependents it touches.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-agent --lib runtime::tasks` → PASS.

```bash
git add crates/davinci-agent/src/runtime/tasks.rs
git commit -m "fix(tasks): make task rewind atomic"
```

---

### Task 5.16: Contract scope checks are char-safe and case-insensitive where the filesystem is

**Finding fixed:** davinci-agent 22. `contracts.rs:147` slices `normalized[..scope.len()]`, which can split a UTF-8 character (reachable only with `case_insensitive = true`); `allows_path` always passes `false` (`:720`), so scope checks are case-sensitive on Windows and macOS, where `SRC/x.rs` and `src/x.rs` are the same file.

**Files:**
- Modify: `crates/davinci-agent/src/runtime/contracts.rs:140-152, 712-724`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn case_insensitive_prefix_check_does_not_split_characters() {
        let contract = contract_with_scopes(&["src/é/"], &[]);
        assert!(!contract.allows_path_case("src/\u{e9}x/a.rs", true).unwrap());
        assert!(contract.allows_path_case("SRC/é/a.rs", true).unwrap());
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn scope_matching_ignores_case_on_case_insensitive_filesystems() {
        let contract = contract_with_scopes(&["src/"], &[]);
        assert!(contract.allows_path("SRC/lib.rs").unwrap());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib contracts::tests::case_insensitive_prefix`
Expected: FAIL with a char-boundary panic.

- [ ] **Step 3: Implement**

```rust
                || (scope.ends_with('/')
                    && normalized
                        .get(..scope.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scope)))
```

and

```rust
    pub fn allows_path(&self, raw_path: &str) -> Result<bool, ContractError> {
        // NTFS and APFS (default) compare names case-insensitively.
        self.allows_path_case(raw_path, cfg!(any(windows, target_os = "macos")))
    }
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-agent --lib contracts` → PASS.

```bash
git add crates/davinci-agent/src/runtime/contracts.rs
git commit -m "fix(contracts): char-safe prefix check, case-insensitive scopes on Windows and macOS"
```

---

### Task 5.17: A corrupt cached workspace value cannot panic

**Finding fixed (latent):** davinci-agent 24. `cache/workspace.rs:35` does `u64::from_str_radix(&self.inputs_hash[..16], 16).expect("SHA-256 hex")` on a struct that derives `Deserialize`, so a corrupt cache file panics.

**Files:**
- Modify: `crates/davinci-agent/src/runtime/cache/workspace.rs:28-40`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn short_or_non_hex_hash_does_not_panic() {
        let mut snapshot = sample_workspace_state();
        snapshot.inputs_hash = "zz".into();
        let deps = snapshot.dependencies();
        assert!(deps.iter().any(|dep| matches!(dep, CacheDependency::WorkspaceGeneration { generation: 0, .. })));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-agent --lib short_or_non_hex_hash`
Expected: FAIL (panic).

- [ ] **Step 3: Implement**

```rust
        // A corrupt cache entry must invalidate, not crash: generation 0
        // never equals a live workspace's generation.
        let generation = self
            .inputs_hash
            .get(..16)
            .and_then(|prefix| u64::from_str_radix(prefix, 16).ok())
            .unwrap_or(0);
```

Confirm a live workspace can never have generation 0 (the first 16 hex digits of a SHA-256 being all zero has probability 2^-64; acceptable).

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-agent --lib cache` → PASS.

```bash
git add crates/davinci-agent/src/runtime/cache/workspace.rs
git commit -m "fix(cache): corrupt workspace hash invalidates instead of panicking"
```

---

## Phase 5 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-agent -p davinci-coding-agent
cargo test -p davinci-agent --release --lib ledger_size_stays_bounded
```

Then `00-index.md` → "Manual verification: agent loop".
