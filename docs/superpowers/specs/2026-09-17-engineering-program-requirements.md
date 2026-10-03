MISSION
=======

Evolve DaVinci from a capable coding harness into a high-performance, reliable, semantically aware software-engineering system.

Build the remaining missing intelligence, execution, reliability, and verification layers needed to make DaVinci:

- faster
- more accurate
- less wasteful with tokens
- less dependent on blind grep/file-reading
- better at TypeScript/JavaScript development
- better at frontend development
- safer when editing repositories
- better at deciding which tests/builds/checks matter
- better at understanding installed packages
- better at understanding Git history
- better at long-running development tasks
- better at verifying its own work
- better in normal single-agent interactive mode
- better in Graph mode
- measurable through repeatable evaluations

THIS MUST NOT BECOME A GRAPH-ONLY PROJECT.

Every major capability should benefit ordinary interactive DaVinci sessions.
Graph is an orchestrator and consumer of capabilities, not the owner of them.

==================================================
0. CRITICAL EXECUTION RULE: THIS IS A PROGRAM, NOT ONE GIANT PATCH
==================================================

Do NOT attempt to implement every subsystem simultaneously in one monolithic diff.

This request contains multiple architectural subsystems.

Treat it as a PROGRAM OF WORK composed of independently reviewable projects.

Before production implementation:

1. Fetch and inspect latest remote `main`.
2. Determine which prerequisite systems already exist.
3. Map the current architecture.
4. Produce an architecture/design document for the overall program.
5. Decompose the program into ordered subprojects.
6. Produce one implementation plan per subproject.
7. Implement subprojects sequentially.
8. Keep the repository working after every subproject.
9. Run tests and review before moving to the next layer.
10. Do not rewrite already-complete systems unless a demonstrated integration defect requires it.

The implementation should be able to stop after any completed subproject and leave DaVinci in a useful, production-quality state.

==================================================
1. INSPECT CURRENT MAIN BEFORE ASSUMING ANYTHING
==================================================

Before changing code:

- fetch latest remote `main`
- inspect recent commits
- inspect current workspace status
- preserve unrelated/pre-existing working-tree changes
- do not overwrite user changes
- use an isolated branch/worktree where appropriate
- never force-push unless explicitly authorized

Do not rely on old SHAs mentioned in previous conversations or plans.

At minimum inspect:

crates/davinci-agent/
crates/davinci-agent/src/runtime/
crates/davinci-coding-agent/
crates/davinci-coding-agent/src/native_extensions/
crates/davinci-coding-agent/src/native_extensions/mod.rs
crates/davinci-coding-agent/src/extension_host.rs
crates/davinci-tui/
crates/davinci-ai/
crates/davinci-evals/
docs/
.github/workflows/

Also inspect:

- RuntimeCapabilityRegistry
- native tool registration
- tool permission/classification
- Graph capability selection
- token governor
- output retention/retrieve_output
- settings/configuration
- event bus/hooks
- process execution
- file edit/write tools
- Git integration
- session/workspace management
- CI commands

==================================================
2. DETECT EXISTING FOUNDATION SYSTEMS
==================================================

Before implementing this program, explicitly determine whether these systems have already landed:

A. Language Intelligence / LSP
Expected focus:
- TypeScript
- JavaScript
- definition
- references
- hover
- symbols
- implementations
- type definition
- diagnostics

B. Semantic Repo Intelligence / AST Index
Expected focus:
- TS/JS AST
- repo map
- symbol search
- file dependencies
- related files
- code_query

C. General DaVinci Cache Runtime
Expected focus:
- memory cache
- persistent CAS
- single-flight
- dependency-aware invalidation
- shared immutable content cache
- workspace-specific mutable state

If one exists:

USE IT.

Do not recreate it.

Treat its public API as an architectural dependency.

If it exists but has integration gaps, fix only the demonstrated gaps necessary for the new subsystem.

If it does not exist, do NOT silently build all three prerequisites inside the first subproject.

Record the missing prerequisite and order the program accordingly.

==================================================
3. TARGET ARCHITECTURE
==================================================

The eventual architecture should resemble:

                    DaVinci

               Unified Intelligence
                      │
       ┌──────────────┼───────────────┐
       │              │               │
      Code         Execution       Knowledge
       │              │               │
      LSP        Process Manager      Memory
   Repo AST      Build Intelligence   Skills
   Git Intel     Test Intelligence    Learning
 Package Intel  Browser Verification  Docs
 Change Impact  Verification Planner
       │              │
       └───────┬──────┘
               │
       Transactional Editing
               │
         Cache Runtime
               │
         Capability System
               │
        Normal Agent / Graph

Graph sits on top of capabilities.

Normal single-agent sessions must have access to the same capabilities when authorized.

Do NOT create:

- a second tool registry
- a second capability registry
- a second permission system
- a second Graph runtime
- a second cache system
- a second output-retention system
- a second memory database
- a second skills database

Reuse existing DaVinci infrastructure.

==================================================
4. SUBPROJECTS TO BUILD
==================================================

Implement these remaining layers in dependency-aware order:

PROJECT 1
Test Impact Intelligence

PROJECT 2
Persistent Process Manager

PROJECT 3
Browser / Playwright Verification

PROJECT 4
Transactional Edit Engine

PROJECT 5
Package / Dependency Intelligence

PROJECT 6
Git Intelligence

PROJECT 7
Build Intelligence

PROJECT 8
Deterministic Hook / Policy Engine

PROJECT 9
Change Impact Engine

PROJECT 10
Verification Planner

PROJECT 11
Workspace Snapshot / Safe Sandbox Layer

PROJECT 12
Continuous Agent Evaluation Framework

You may adjust ordering after repository inspection if a real dependency demands it.

Explain any ordering change.

==================================================
5. CROSS-CUTTING PRINCIPLES
==================================================

Every subsystem must follow these principles.

A. Deterministic first

Do not add model calls for work that can be done deterministically.

Examples:

- test mapping
- dependency resolution
- build target discovery
- Git history lookup
- hook execution
- change impact collection
- verification command selection rules
- package metadata lookup

B. Evidence over inference

Return concrete evidence:

- paths
- symbols
- ranges
- dependencies
- commands
- exit codes
- commit SHAs
- browser console errors
- network failures
- diagnostics
- screenshots
- test results

Do not convert weak evidence into authoritative claims.

C. Bounded output

All tools must have sensible result limits.

Large results should integrate with existing token-governor/output-retention mechanisms.

D. Fail open when optional

Optional intelligence failures should not crash ordinary DaVinci coding.

Examples:

LSP unavailable
browser unavailable
repo cache corrupt
Git metadata unavailable

Return structured errors and allow fallback.

E. Permission checks happen at request time

A cached result, process, browser session, hook, or transaction must never bypass current authorization.

F. Shared infrastructure

Normal interactive mode and Graph should consume the same capability implementations.

==================================================
6. PROJECT 1 — TEST IMPACT INTELLIGENCE
==================================================

GOAL

Given a change or proposed change, identify the smallest meaningful set of tests that should run first.

This is NOT permission to skip final verification when broader verification is required.

Build native deterministic intelligence for:

- changed files
- changed symbols
- import/dependency relationships
- source-to-test pairing
- package/workspace boundaries
- historical test relationships where reliable
- test framework discovery
- test command discovery

Potential model-facing capabilities:

test_related
test_impacted
test_plan

Avoid adding a generic arbitrary test-query DSL.

Example:

changed:
src/auth/token.ts

result:
1. src/auth/token.test.ts
   reason: paired test

2. src/auth/session.test.ts
   reason: imports token.ts

3. tests/integration/login.test.ts
   reason: transitive impacted route

Each result must explain WHY it is considered impacted.

Do not return unexplained numeric confidence alone.

Support TypeScript/JavaScript first.

Recognize common patterns:

*.test.ts
*.test.tsx
*.spec.ts
*.spec.tsx
__tests__/
tests/
test/

Recognize frameworks where detectable:

Vitest
Jest
Node test runner
Playwright
other repository-defined scripts

Integrate with Semantic Repo Intelligence when available.

Do not duplicate AST parsing.

PERFORMANCE GOAL

A local edit should usually produce a targeted test plan without scanning every source file from scratch.

ACCEPTANCE

- deterministic results
- explained test relationships
- monorepo support
- package boundary awareness
- no full test suite required for every edit
- broader verification still available at completion

==================================================
7. PROJECT 2 — PERSISTENT PROCESS MANAGER
==================================================

GOAL

Stop repeatedly spawning expensive long-lived processes.

Build a native supervised process manager.

Possible tools:

process_start
process_status
process_output
process_write
process_stop
process_list

Capabilities should support processes such as:

pnpm dev
npm run dev
vite
next dev
tsc --watch
vitest --watch
local API servers
future Cargo/watch processes

A process record should include:

id
owner/session
workspace
command
argv
cwd
environment policy
PID/process-group identity
start time
state
exit status
output ring buffer
restart policy
port metadata when known

DO NOT launch through unsafe shell string concatenation.

Use argv/process APIs.

SECURITY

- workspace restrictions
- existing command policy
- process ownership
- safe environment handling
- no automatic access to secrets
- proper descendant cleanup
- Windows + Unix behavior

LIFECYCLE

Processes may outlive one tool call.

They must NOT become orphaned indefinitely.

Define ownership:

session-owned
workspace-owned
explicit persistent mode if justified

On shutdown:

stop or reconcile owned processes according to policy.

OUTPUT

Use bounded ring buffers.

Integrate with retrieve_output if output becomes large.

SINGLE-FLIGHT

If multiple callers ask for an equivalent server process, support controlled reuse where safe.

==================================================
8. PROJECT 3 — BROWSER / PLAYWRIGHT VERIFICATION
==================================================

GOAL

Allow DaVinci to verify frontend behavior rather than relying on source inspection alone.

Prefer Playwright or an equivalent maintained browser automation stack.

Possible tools:

browser_open
browser_snapshot
browser_click
browser_type
browser_select
browser_console
browser_network
browser_accessibility
browser_screenshot
browser_close

Do not expose an unrestricted low-level browser protocol directly to the model unless current architecture already provides a safe wrapper.

PRIMARY WORKFLOW

edit UI
→ LSP diagnostics
→ targeted tests
→ persistent dev server
→ browser verification
→ console/network/accessibility evidence

Browser verification should work in normal interactive sessions and Graph.

REQUIREMENTS

- attach to managed local dev servers
- deterministic selectors where possible
- accessibility-tree querying
- console error collection
- failed network request collection
- viewport control
- screenshot capture
- bounded artifact retention
- session cleanup

SECURITY

Do not allow arbitrary browser navigation to become unrestricted data exfiltration.

Respect project/network policy.

LOCAL development URLs should be first-class.

SCREENSHOTS

Store them as artifacts, not giant base64 tool output.

==================================================
9. PROJECT 4 — TRANSACTIONAL EDIT ENGINE
==================================================

GOAL

Make repository mutation safer and recoverable.

Every multi-file edit should be representable as a transaction.

Conceptual model:

EditTransaction {
    id,
    owner,
    workspace,
    base_revision,
    affected_files,
    before_hashes,
    proposed_changes,
    applied_hashes,
    state,
    verification_state
}

States may include:

draft
previewed
applied
verified
committed
rolled_back
conflicted

Integrate with current DaVinci write/edit tools.

Do not create a completely separate edit mechanism if existing tools can be wrapped.

Potential capabilities:

patch_preview
patch_apply
patch_status
patch_rollback

Avoid adding excessive model-visible tools if internal wrapping is enough.

MANDATORY SAFETY

Before applying:

- verify before-hash
- verify workspace authority
- detect stale file state

If another actor changed the file:

return conflict

do not overwrite.

ROLLBACK

Rollback must be safe.

Do not revert unrelated user changes.

Use exact transaction ownership and hashes.

PROVENANCE

Record:

agent/session
Graph node if applicable
files
before hash
after hash
timestamp/sequence
verification result

This should greatly improve crash recovery and multi-agent correctness.

==================================================
10. PROJECT 5 — PACKAGE / DEPENDENCY INTELLIGENCE
==================================================

GOAL

Make DaVinci reason about the ACTUAL installed dependency versions instead of generic remembered APIs.

TypeScript/JavaScript first.

Support:

package.json
package-lock.json
pnpm-lock.yaml
yarn.lock
workspace packages
node_modules metadata when installed

Potential tools:

package_info
package_exports
package_symbol
package_dependents
package_why

Example:

package_info("zod")

return:

installed version
workspace scope
manifest path
type declaration path
exports
dependencies
dependents

PACKAGE SYMBOL

Given:

package = "zod"
symbol = "z.object"

prefer inspecting installed types/source metadata over generic internet assumptions.

Do NOT recursively index all node_modules through the normal repo AST index.

Package intelligence should selectively inspect requested packages.

CACHE

Integrate with Cache Runtime.

Key on:

manifest hash
lockfile hash
installed package version
relevant package content hash

==================================================
11. PROJECT 6 — GIT INTELLIGENCE
==================================================

GOAL

Allow DaVinci to understand why code exists and how it evolved.

Potential tools:

git_symbol_history
git_related_commits
git_changed_symbols
git_branch_diff
git_blame_symbol
git_commit_context
git_conflict_explain

Do not merely wrap arbitrary `git` shell strings.

Normalize outputs.

Examples:

git_symbol_history(AuthService.login)

return:

introduced commit
important modifications
authors if useful
commit messages
file movement if detectable
related PR metadata if locally available

Use immutable Git object caching aggressively.

Do NOT invent reasons from commit text.

Separate:

facts:
commit changed these lines

from:

inference:
commit message suggests it fixed token refresh

==================================================
12. PROJECT 7 — BUILD INTELLIGENCE
==================================================

GOAL

Understand how the repository should be built instead of guessing commands.

TypeScript/JavaScript first.

Recognize:

npm
pnpm
yarn
bun if repository uses it

Build/task systems:

Turborepo
Nx
Vite
Next.js
tsconfig project references
package workspace scripts

Potential capabilities:

build_targets
build_dependencies
build_affected
build_command
workspace_packages

Example:

"What command builds packages affected by packages/ui?"

return:

tool: turbo
targets:
packages/ui
packages/web

suggested deterministic command:
pnpm turbo build --filter=...

Do not execute automatically unless authorized.

Do not replace project-native caches.

Understand and exploit:

Turborepo caching
Nx caching
tsbuildinfo
package-manager stores

==================================================
13. PROJECT 8 — DETERMINISTIC HOOK / POLICY ENGINE
==================================================

GOAL

Move deterministic rules out of prompts and into code/configuration.

Extend the existing DaVinci event system instead of creating a second event bus.

Potential events:

SessionStart
SessionEnd
BeforeTool
AfterTool
BeforeWrite
AfterWrite
BeforeProcessStart
AfterProcessExit
BeforeTest
AfterTest
BeforeCommit
AfterCommit
BeforeCompletion

Hooks should support deterministic actions such as:

AfterWrite *.ts
→ prettier/eslint

BeforeCommit
→ secret scan

AfterTestFailure
→ attach diagnostics

BeforeCompletion
→ require verification evidence

PROJECT CONFIGURATION

Support project-scoped configuration.

Do not execute untrusted repository hooks automatically before trust is established.

SECURITY

Hooks are code execution.

Respect project trust.

Prevent model-generated arbitrary hook injection without authorization.

TIMEOUTS

All hooks must be bounded.

Hook failure policy must be explicit:

warn
block
ignore

not ambiguous.

==================================================
14. PROJECT 9 — CHANGE IMPACT ENGINE
==================================================

GOAL

Before or after a proposed edit, determine likely blast radius.

Integrate:

- AST repo graph
- LSP references
- package graph
- test impact
- build graph
- Git history where useful

Potential tool:

impact_analyze

Input:

files
symbols
or transaction ID

Output sections:

Direct semantic impact
Structural impact
Tests
Packages
Build targets
Public API risk
Configuration impact
Potential browser flows

Each item must include evidence source:

LSP
AST
package
test-map
build
git

Do not claim certainty when analysis is incomplete.

Example:

Change:
AuthService.login

Impact:
HIGH confidence
- 12 LSP references
- 3 direct importing modules

Tests:
- login.test.ts
- session.test.ts

Packages:
- packages/web
- packages/api

Public API:
- exported from packages/auth/index.ts

==================================================
15. PROJECT 10 — VERIFICATION PLANNER
==================================================

GOAL

Choose the CHEAPEST VALID verification sequence for the actual change.

This should dramatically reduce over-testing while maintaining correctness.

Inputs:

- changed files
- changed symbols
- transaction metadata
- package graph
- test impact
- build graph
- browser relevance
- security risk
- repository CI configuration

Potential capability:

verification_plan

Example:

Changed:
packages/ui/src/Button.tsx

Plan:

1. TypeScript diagnostics
2. Button.test.tsx
3. package-level typecheck
4. package-level lint
5. browser interaction check
6. workspace build only if public API changed

Do NOT use an LLM call to decide this if deterministic evidence is sufficient.

RULES

Final verification authority remains real commands/evidence.

A verification plan is not itself verification.

SECURITY-SENSITIVE CHANGES

Auth
crypto
permissions
credentials
process execution
dependency manifests

should trigger stronger verification/security policy when appropriate.

CI parity matters.

==================================================
16. PROJECT 11 — WORKSPACE SNAPSHOT / SAFE SANDBOX
==================================================

GOAL

Allow cheap safe checkpoints around risky autonomous work.

Possible capabilities:

workspace_checkpoint
workspace_diff
workspace_restore

Prefer Git/worktree-native mechanisms when possible.

Do not create duplicate copies of massive repositories unnecessarily.

Snapshots should capture enough identity to safely answer:

"What changed since checkpoint X?"

and:

"Can transaction Y be rolled back without overwriting newer user edits?"

Snapshot must NOT include secrets outside normal repository scope.

WORKTREES

Integrate cleanly with Git worktrees.

Do not confuse:

workspace snapshot

with:

Git commit

They serve different purposes.

==================================================
17. PROJECT 12 — CONTINUOUS AGENT EVALUATION FRAMEWORK
==================================================

GOAL

Measure whether DaVinci actually gets better.

Do not optimize the harness only by intuition.

Extend or reuse `davinci-evals`.

Create reproducible coding-agent evaluations for:

- repo navigation
- symbol finding
- TypeScript debugging
- safe refactor
- dependency/API usage
- frontend bug fixing
- test selection
- browser verification
- Git-context reasoning
- multi-file transaction safety
- long-running autonomous task
- Graph vs normal single-agent behavior

METRICS

Track:

task success
verification success
incorrect completion claims
tool calls
full-file reads
bytes read
input tokens
output tokens
cache hits
wall-clock latency
tests executed
test runtime
process startups
LSP cold starts
browser verification success
rollback correctness
CI success

Compare:

baseline DaVinci
vs
new capability enabled

FEATURE FLAGS

Every major new subsystem should be independently disableable for evaluation.

Avoid evaluations that require live network providers unless explicitly separated.

Prefer deterministic offline fixtures for CI.

==================================================
18. UNIFIED CAPABILITY DISCOVERY
==================================================

Do not flood every model request with every new tool.

Integrate with the existing capability selection/toolbox architecture.

Advertise only tools relevant to:

- current task
- current mode
- permissions
- role
- workspace capabilities

Normal user session:

task asks frontend debugging
→ browser + LSP + repo + tests

task asks Git regression investigation
→ Git + repo + tests

task asks simple text edit
→ do not activate browser/process/package layers unnecessarily

Use deterministic capability classification where possible.

Do not add extra model calls solely for tool routing.

==================================================
19. NORMAL MODE MUST BE FIRST-CLASS
==================================================

For every subsystem, include tests proving it works WITHOUT Graph.

Required acceptance scenario:

ordinary interactive DaVinci session
→ uses intelligence capability
→ performs edit
→ verifies work

Graph-specific integrations are secondary adapters.

Do not require `/graph` to access:

LSP
repo intelligence
test impact
package intelligence
Git intelligence
browser verification
process manager
transactional edits

when normal tool policy authorizes them.

==================================================
20. GRAPH INTEGRATION
==================================================

Graph should consume the same tools.

Do not create:

graph_test_impact
graph_browser
graph_package_info
graph_process_start

Use the same native capabilities.

Role policy may limit them.

Suggested examples:

Researcher
- repo intelligence
- LSP
- Git
- package intelligence

Planner
- repo
- test impact
- build
- impact analysis

Writer
- transactional edits
- LSP
- package
- process manager when allowed

Reviewer
- impact analysis
- Git
- diagnostics
- browser evidence

Verifier
- test plan
- build intelligence
- browser verification

==================================================
21. CACHE RUNTIME INTEGRATION
==================================================

If Cache Runtime exists, use it.

Examples:

Test Impact:
cache dependency mapping

Package Intelligence:
cache parsed manifests/lockfiles

Git Intelligence:
cache immutable Git objects

Build Intelligence:
cache discovered targets/configs

Browser:
cache NO live page state to disk unless explicitly safe

Process Manager:
use resource/session reuse, not serialized process cache

Do not create subsystem-specific cache frameworks.

==================================================
22. OUTPUT / TOKEN EFFICIENCY
==================================================

Every new tool should be optimized for model consumption.

Bad:

return 5,000-line JSON object

Good:

12 impacted tests
showing top 10
reasons included

Use:

summary
bounded rows
stable ordering
provenance
remaining count

Large exact outputs should use existing output retention.

==================================================
23. TRANSACTION + VERIFICATION CLOSED LOOP
==================================================

Target normal workflow:

understand
    ↓
impact analysis
    ↓
begin transaction
    ↓
edit
    ↓
incremental diagnostics
    ↓
targeted test plan
    ↓
run targeted verification
    ↓
build/typecheck if required
    ↓
browser verification if applicable
    ↓
review change impact
    ↓
commit-ready evidence

No step should claim success without evidence.

==================================================
24. PERFORMANCE REQUIREMENTS
==================================================

These systems must improve DaVinci, not make it heavy.

Measure:

DaVinci startup time
memory usage
first-query latency
warm-query latency
tool activation cost
process count
browser startup
test runtime
index/cache reuse

Requirements:

- lazy startup
- no browser launch unless needed
- no LSP launch unless needed
- no repo full scan unless needed
- no persistent dev server unless needed
- shared expensive resources
- no duplicate Graph-worker processes for the same underlying service when safe

==================================================
25. CONCURRENCY
==================================================

DaVinci may have:

normal interactive agent
+
Graph workers
+
background learning
+
LSP
+
process manager
+
browser
+
cache

Avoid global lock contention.

Requirements:

- bounded locks
- no broad mutex over external process I/O
- no callbacks while holding unrelated global locks
- no duplicate process startup races
- no duplicate browser launch races
- proper cancellation
- clean shutdown

Add concurrency tests to relevant systems.

==================================================
26. WINDOWS + UNIX
==================================================

DaVinci must remain cross-platform.

Explicitly test or design for:

Windows
Linux
macOS

Particular risk areas:

process groups
child termination
signals
path normalization
shell quoting
lock files
browser executable discovery
Git
Node commands

Do not implement Unix-only process semantics and assume they work on Windows.

==================================================
27. SECURITY
==================================================

Maintain least privilege.

Mandatory invariants:

- tools obey current permission mode
- cached evidence cannot bypass permissions
- hooks require trust
- transactions stay inside workspace
- browser network access follows policy
- package inspection does not execute package lifecycle scripts
- Git inspection is read-only unless explicit mutation tool
- process manager cannot silently elevate shell capabilities
- verification commands remain bounded
- untrusted project config cannot silently execute before trust

==================================================
28. FAILURE MODEL
==================================================

Optional intelligence should fail open.

Examples:

Playwright unavailable
→ source/test verification still works

Git unavailable
→ code intelligence still works

package metadata missing
→ fallback to source/LSP

process server crashes
→ structured failure + bounded restart

Never let an optional subsystem crash the main interactive session.

Transactional edits are different:

if transaction integrity cannot be proven
→ fail closed on mutation

==================================================
29. SETTINGS
==================================================

Integrate into existing settings architecture.

Do not expose dozens of low-level knobs.

Each major subsystem should have:

enabled / disabled

plus only critical configuration.

Example conceptual shape:

{
  "intelligence": {
    "testImpact": true,
    "packages": true,
    "git": true,
    "build": true,
    "changeImpact": true
  },
  "execution": {
    "processManager": true,
    "browserVerification": true
  },
  "editing": {
    "transactions": true
  }
}

Do not copy this exact structure blindly.

Follow existing DaVinci conventions.

==================================================
30. OBSERVABILITY
==================================================

Add useful operational telemetry.

Possible status surfaces:

/process-status
/browser-status
/cache-status
/intelligence-status
/eval-status

Avoid one slash command for every tiny implementation detail.

Prefer one aggregate status where possible.

Track:

capability used
latency
cache hit
failure reason
fallback used
tests selected
tests avoided
process reuse
browser reuse
transaction rollback
verification outcome

==================================================
31. TESTING STRATEGY
==================================================

Use TDD.

Every subproject must include:

unit tests
integration tests
failure-path tests
security tests
concurrency tests where relevant
normal interactive mode tests
Graph integration tests where relevant

Tests should be offline and deterministic wherever possible.

Do not require package downloads or real external services in normal CI.

Use fake subprocesses/browser fixtures where necessary.

==================================================
32. EVALUATION-DRIVEN DEVELOPMENT
==================================================

Before each subsystem:

establish baseline behavior.

After each subsystem:

run relevant evals.

For example:

Test Impact:

baseline:
workspace suite = 1,200 tests

new:
6 impacted tests + final required package check

Measure:
runtime
correctness
missed failures

Browser:

baseline:
source-only frontend task

new:
source + browser

Measure:
UI bugs caught
console errors caught
task success

Package intelligence:

baseline:
model guesses API

new:
installed package evidence

Measure:
invalid API calls

Do not claim improvements without measurement.

==================================================
33. DO NOT DO THESE THINGS
==================================================

Do NOT:

- build everything in one giant diff
- make capabilities Graph-only
- duplicate LSP
- duplicate repo AST
- duplicate Cache Runtime
- create a second permissions system
- create a second tool registry
- add model calls for deterministic routing
- expose unrestricted arbitrary internal protocols to the model
- cache arbitrary shell results
- automatically execute untrusted project hooks
- make Browser a mandatory dependency for DaVinci startup
- launch long-lived processes unnecessarily
- overwrite files without stale-state checks
- declare verification success from LSP diagnostics alone
- trust package APIs from model memory when installed metadata exists
- use Git commit messages as unquestioned ground truth
- run the full test suite after every tiny edit when impact data supports a smaller safe sequence
- skip final required CI-equivalent verification merely because targeted tests passed
- perform unrelated code cleanup

==================================================
34. SUBPROJECT COMPLETION GATE
==================================================

A subproject is complete only when:

1. design approved
2. implementation plan exists
3. targeted red/green tests done
4. related package tests pass
5. fmt passes
6. clippy passes
7. integration tests pass
8. security invariants pass
9. normal non-Graph path proven
10. Graph integration proven if applicable
11. benchmarks/evals run
12. docs updated
13. diff reviewed
14. CI green

Do not proceed to the next subsystem with known failures in the current one.

==================================================
35. CI
==================================================

After each meaningful implementation milestone:

- push feature branch
- monitor GitHub Actions
- inspect exact failed job/log
- reproduce locally when possible
- fix root cause
- commit/push
- continue until relevant CI is green

Do not stop merely because local tests passed.

No force-push unless explicitly authorized.

==================================================
36. SUGGESTED PROGRAM ORDER
==================================================

After current prerequisites are confirmed, prefer:

Phase A — Reliable execution foundation
1. Test Impact Intelligence
2. Persistent Process Manager
3. Transactional Edit Engine

Phase B — Runtime verification
4. Browser / Playwright Verification
5. Build Intelligence

Phase C — Repository understanding
6. Package / Dependency Intelligence
7. Git Intelligence
8. Change Impact Engine

Phase D — Policy and completion
9. Deterministic Hook / Policy Engine
10. Verification Planner
11. Workspace Snapshot / Safe Sandbox

Phase E — Continuous improvement
12. Continuous Agent Evaluation Framework

You may adjust this order if real code dependencies justify it.

==================================================
37. GLOBAL ACCEPTANCE SCENARIO
==================================================

At the end of the entire program, prove a realistic non-Graph TypeScript/JavaScript task:

User:
"Fix the login button bug and make sure nothing else breaks."

DaVinci should be able to:

1. inspect repo map
2. use LSP to locate relevant symbols
3. inspect installed package APIs if relevant
4. use Git context if useful
5. calculate change impact
6. start/reuse dev server
7. create transactional edit
8. modify code
9. update LSP diagnostics
10. calculate impacted tests
11. run targeted tests
12. run necessary type/build checks
13. open browser
14. verify login flow
15. inspect console/network errors
16. verify transaction
17. produce evidence-backed completion

WITHOUT Graph.

Then prove the same capabilities can be selectively assigned to Graph workers.

==================================================
38. GLOBAL PERFORMANCE ACCEPTANCE
==================================================

The program should demonstrate measurable improvements in at least:

- fewer unnecessary full-file reads
- fewer unnecessary tests
- fewer repeated process startups
- fewer repeated package metadata reads
- lower warm-query latency
- increased cache reuse
- better frontend verification
- fewer stale-write conflicts
- fewer unsupported package API guesses
- better completion evidence

Do not require every metric to improve on every task.

Report tradeoffs honestly.

==================================================
39. FINAL DELIVERABLE
==================================================

At the end of the program, provide a comprehensive report including:

Architecture
- final subsystem diagram
- ownership boundaries
- capability routing

Subprojects
- files created
- files modified
- tools/capabilities added

Performance
- baseline metrics
- after metrics
- regressions if any

Quality
- eval results
- bugs caught by new verification layers
- false positive/negative findings

Reliability
- transaction behavior
- rollback behavior
- process cleanup
- browser cleanup
- cache correctness

Verification
- exact test commands
- fmt
- clippy
- package tests
- workspace tests
- CI status

Known limitations
- unsupported languages
- unsupported package managers
- browser limitations
- static-analysis limitations

Next recommended improvements

==================================================
40. GUIDING PRINCIPLE
==================================================

The final system should behave like this:

DaVinci should first obtain the cheapest reliable evidence available.

It should understand the repository before reading large amounts of source.

It should understand symbols before guessing.

It should inspect installed package APIs before hallucinating them.

It should understand impact before editing.

It should mutate transactionally.

It should run the smallest valid verification first.

It should use the browser when UI behavior matters.

It should inspect Git history when historical intent matters.

It should reuse expensive processes and deterministic computations.

It should only use Graph when orchestration genuinely helps.

It should never trade correctness, authorization, or evidence quality merely for speed.